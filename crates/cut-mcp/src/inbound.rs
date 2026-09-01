//! Optional inbound MCP stdio clients used by the terminal agent.
//!
//! `cut-mcp` is the project server that external clients connect to. This
//! registry is the inverse boundary: it discovers user-configured MCP
//! servers, keeps their tool catalog behind `mcp_search`, and routes
//! `mcp_call` without exposing every external schema to the model at once.

use cut_agent::{AgentTool, ToolMap};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Deserialize, Default)]
struct McpConfigFile {
    #[serde(rename = "mcpServers", default)]
    servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Clone, Debug, Deserialize)]
struct McpServerConfig {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    tools: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboundMcpToolDescription {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboundMcpServerDescription {
    pub server: String,
    pub tools: Vec<InboundMcpToolDescription>,
}

#[derive(Clone)]
struct ConnectedServer {
    config: McpServerConfig,
    tools: Vec<InboundTool>,
    process: Arc<Mutex<McpProcess>>,
}

#[derive(Clone)]
struct InboundTool {
    name: String,
    description: Option<String>,
}

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Discovered user-configured MCP servers and their synchronized stdio
/// transports.
#[derive(Clone)]
pub struct InboundMcpRegistry {
    servers: Arc<BTreeMap<String, ConnectedServer>>,
}

impl InboundMcpRegistry {
    /// Load `~/.cut/mcp.json` and `<project>/.cut/mcp.json`, then connect to
    /// every valid server. Missing or unavailable servers are skipped with a
    /// warning so the local Cut workflow remains usable without optional MCPs.
    pub fn connect(directory: &Path, mut log: impl FnMut(String)) -> Result<Self, String> {
        let configs = load_configs(directory)?;
        let mut servers = BTreeMap::new();
        for (name, config) in configs {
            match ConnectedServer::connect(config) {
                Ok((server, tool_count)) => {
                    log(format!("mcp: connected {name} ({tool_count} tools)"));
                    servers.insert(name, server);
                }
                Err(error) => {
                    log(format!(
                        "warning: mcp server '{name}' unavailable ({error}), skipping"
                    ));
                }
            }
        }
        Ok(Self {
            servers: Arc::new(servers),
        })
    }

    /// Return the discovered catalog for `lcut --mcp`.
    pub fn describe(&self) -> Vec<InboundMcpServerDescription> {
        self.servers
            .iter()
            .map(|(server, connected)| InboundMcpServerDescription {
                server: server.clone(),
                tools: connected
                    .tools
                    .iter()
                    .map(|tool| InboundMcpToolDescription {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                    })
                    .collect(),
            })
            .collect()
    }

    /// Build the two meta-tools used by the terminal agent.
    pub fn meta_tools(&self) -> ToolMap {
        let search_registry = self.clone();
        let call_registry = self.clone();
        let mut tools = ToolMap::new();
        tools.insert(
            "mcp_search".into(),
            AgentTool::new(
                "mcp_search",
                "Search discovered inbound MCP tools by server, tool name, or description. An empty query lists all allowed tools.",
                json!({
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "additionalProperties": false
                }),
                move |input| search_registry.search(input),
            ),
        );
        tools.insert(
            "mcp_call".into(),
            AgentTool::new(
                "mcp_call",
                "Call a discovered inbound MCP tool by mcp_<server>_<tool> or server/tool.",
                json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "minLength": 1},
                        "arguments": {"type": "object"}
                    },
                    "required": ["name"],
                    "additionalProperties": false
                }),
                move |input| call_registry.call_from_input(input),
            ),
        );
        tools
    }

    fn search(&self, input: Value) -> Result<Value, String> {
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let mut matches = Vec::new();
        for (server_name, server) in self.servers.iter() {
            for tool in &server.tools {
                if !server.allows(&tool.name) {
                    continue;
                }
                let qualified = format!("mcp_{server_name}_{}", tool.name);
                let haystack = format!(
                    "{server_name} {} {qualified} {}",
                    tool.name,
                    tool.description.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                if !query.is_empty() && !haystack.contains(&query) {
                    continue;
                }
                matches.push(json!({
                    "name": qualified,
                    "server": server_name,
                    "tool": tool.name,
                    "description": tool.description,
                }));
            }
        }
        Ok(Value::Array(matches))
    }

    fn call_from_input(&self, input: Value) -> Result<Value, String> {
        let object = input
            .as_object()
            .ok_or_else(|| "mcp_call requires an object input".to_owned())?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "mcp_call requires a non-empty name".to_owned())?;
        let arguments = object
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        self.call(name, arguments)
    }

    fn call(&self, presented_name: &str, arguments: Value) -> Result<Value, String> {
        let (server_name, tool_name) = parse_tool_name(presented_name, self.servers.keys())
            .ok_or_else(|| format!("unknown MCP tool: {presented_name}"))?;
        let server = self
            .servers
            .get(&server_name)
            .ok_or_else(|| format!("MCP server not connected: {server_name}"))?;
        if !server.allows(&tool_name) {
            return Err(format!("MCP tool not allowed: {presented_name}"));
        }
        let tool = server
            .tools
            .iter()
            .find(|tool| tool.name == tool_name)
            .ok_or_else(|| format!("unknown MCP tool: {presented_name}"))?;
        let mut process = server
            .process
            .lock()
            .map_err(|_| "MCP process lock poisoned".to_owned())?;
        let response = process.request(
            "tools/call",
            json!({"name": tool.name, "arguments": arguments}),
        )?;
        parse_tool_response(response)
    }
}

impl ConnectedServer {
    fn connect(config: McpServerConfig) -> Result<(Self, usize), String> {
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to spawn {}: {error}", config.command))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "MCP child stdin was not piped".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "MCP child stdout was not piped".to_owned())?;
        let mut process = McpProcess {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        let handshake = (|| {
            let initialized = process.request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "cut", "version": env!("CARGO_PKG_VERSION")}
                }),
            )?;
            if initialized.get("error").is_some() {
                return Err(format!("initialize failed: {initialized}"));
            }
            process.notify("notifications/initialized", json!({}))?;
            let listing = process.request("tools/list", json!({}))?;
            if listing.get("error").is_some() {
                return Err(format!("tools/list failed: {listing}"));
            }
            Ok(listing
                .get("result")
                .and_then(|result| result.get("tools"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default())
        })();
        let raw_tools = match handshake {
            Ok(tools) => tools,
            Err(error) => {
                let _ = process.child.kill();
                let _ = process.child.wait();
                return Err(error);
            }
        };
        let tools = raw_tools
            .into_iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?.to_owned();
                Some(InboundTool {
                    name,
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect::<Vec<_>>();
        let count = tools.len();
        Ok((
            Self {
                config,
                tools,
                process: Arc::new(Mutex::new(process)),
            },
            count,
        ))
    }

    fn allows(&self, tool: &str) -> bool {
        self.config
            .tools
            .as_ref()
            .is_none_or(|allowlist| allowlist.iter().any(|allowed| allowed == tool))
    }
}

impl McpProcess {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.write_message(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        let mut line = String::new();
        loop {
            line.clear();
            let read = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("MCP read failed: {error}"))?;
            if read == 0 {
                return Err("MCP server closed stdout".to_owned());
            }
            let response: Value = serde_json::from_str(line.trim())
                .map_err(|error| format!("invalid MCP JSON response: {error}"))?;
            if response.get("id") == Some(&json!(id)) {
                return Ok(response);
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write_message(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    fn write_message(&mut self, message: Value) -> Result<(), String> {
        let encoded = serde_json::to_vec(&message).map_err(|error| error.to_string())?;
        self.stdin
            .write_all(&encoded)
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| format!("MCP write failed: {error}"))
    }
}

fn parse_tool_response(response: Value) -> Result<Value, String> {
    if let Some(error) = response.get("error") {
        return Err(format!("MCP call failed: {error}"));
    }
    let result = response.get("result").cloned().unwrap_or(Value::Null);
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if is_error && text.is_empty() {
        return Err("MCP tool returned an error".to_owned());
    }
    if !text.is_empty() {
        Ok(Value::String(text.join("\n")))
    } else {
        Ok(result)
    }
}

fn parse_tool_name<'a, I>(name: &str, servers: I) -> Option<(String, String)>
where
    I: IntoIterator<Item = &'a String>,
{
    if let Some(rest) = name.strip_prefix("mcp_") {
        return servers
            .into_iter()
            .filter_map(|server| {
                let prefix = format!("{server}_");
                rest.strip_prefix(&prefix)
                    .and_then(|tool| (!tool.is_empty()).then(|| (server.clone(), tool.to_owned())))
            })
            .max_by_key(|(server, _)| server.len());
    }
    let (server, tool) = name.split_once('/')?;
    (!server.is_empty() && !tool.is_empty()).then(|| (server.to_owned(), tool.to_owned()))
}

fn load_configs(directory: &Path) -> Result<BTreeMap<String, McpServerConfig>, String> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let mut merged = BTreeMap::new();
    let mut paths = Vec::new();
    if let Some(home) = home {
        paths.push(home.join(".cut/mcp.json"));
    }
    paths.push(directory.join(".cut/mcp.json"));
    for path in paths {
        let raw = match fs::read(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("read {}: {error}", path.display())),
        };
        let config: McpConfigFile = serde_json::from_slice(&raw)
            .map_err(|error| format!("parse {}: {error}", path.display()))?;
        merged.extend(config.servers);
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::parse_tool_name;

    #[test]
    fn namespaced_parser_prefers_the_longest_server_prefix() {
        let servers = ["team".to_owned(), "team_tools".to_owned()];
        assert_eq!(
            parse_tool_name("mcp_team_tools_lookup", servers.iter()),
            Some(("team_tools".into(), "lookup".into()))
        );
        assert_eq!(
            parse_tool_name("team/lookup", servers.iter()),
            Some(("team".into(), "lookup".into()))
        );
    }
}
