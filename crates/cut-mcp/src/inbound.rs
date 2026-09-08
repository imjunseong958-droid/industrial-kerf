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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MCP_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MCP_SEARCH_PAGE_SIZE: usize = 50;

fn request_timeout() -> Duration {
    env::var("CUT_MCP_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(MCP_REQUEST_TIMEOUT)
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
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
    input_schema: Option<Value>,
}

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    timeout: Duration,
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

/// Whether optional inbound MCPs are enabled for an agent session. Both UI
/// and CLI use this helper so the environment opt-in has one contract.
pub fn enabled(explicit: bool) -> bool {
    explicit || env::var("CUT_ENABLE_MCP").ok().as_deref() == Some("1")
}

/// Extend an agent's native tool map with the configured inbound MCP tools.
/// The UI calls this from its Rust bridge; Qt itself remains presentation-only.
pub fn add_agent_tools(
    tools: &mut ToolMap,
    directory: &Path,
    explicit: bool,
    log: impl FnMut(String),
) -> Result<(), String> {
    if !enabled(explicit) {
        return Ok(());
    }
    let registry = InboundMcpRegistry::connect(directory, log)?;
    tools.extend(registry.meta_tools());
    Ok(())
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
                        input_schema: tool.input_schema.clone(),
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
                    "properties": {"query": {"type": "string"}, "offset": {"type":"integer", "minimum":0}, "limit": {"type":"integer", "minimum":1, "maximum": MCP_SEARCH_PAGE_SIZE}},
                    "additionalProperties": false
                }),
                move |input| search_registry.search(input),
            ),
        );
        tools.insert(
            "mcp_call".into(),
            AgentTool::cancellable(
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
                move |input, cancelled| call_registry.call_from_input(input, cancelled),
            ),
        );
        let describe_registry = self.clone();
        tools.insert(
            "mcp_describe".into(),
            AgentTool::new(
                "mcp_describe",
                "Get the exact input schema for one discovered inbound MCP tool. Use mcp_search first.",
                json!({"type":"object","properties":{"name":{"type":"string","minLength":1}},"required":["name"],"additionalProperties":false}),
                move |input| describe_registry.describe_tool(input),
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
        let offset = input.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(MCP_SEARCH_PAGE_SIZE as u64)
            .clamp(1, MCP_SEARCH_PAGE_SIZE as u64) as usize;
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
                    "description": tool
                        .description
                        .as_ref()
                        .map(|value| bounded_description(value)),
                }));
            }
        }
        let total = matches.len();
        let next_offset = offset.saturating_add(limit);
        Ok(
            json!({"items": matches.into_iter().skip(offset).take(limit).collect::<Vec<_>>(), "offset": offset, "limit": limit, "total": total, "nextOffset": (next_offset < total).then_some(next_offset)}),
        )
    }

    fn describe_tool(&self, input: Value) -> Result<Value, String> {
        let name = input
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "mcp_describe requires a non-empty name".to_owned())?;
        let (server_name, tool_name) = parse_tool_name(name, self.servers.keys())
            .ok_or_else(|| format!("unknown MCP tool: {name}"))?;
        let server = self
            .servers
            .get(&server_name)
            .ok_or_else(|| format!("MCP server not connected: {server_name}"))?;
        if !server.allows(&tool_name) {
            return Err(format!("MCP tool not allowed: {name}"));
        }
        let tool = server
            .tools
            .iter()
            .find(|t| t.name == tool_name)
            .ok_or_else(|| format!("unknown MCP tool: {name}"))?;
        Ok(
            json!({"name": format!("mcp_{server_name}_{}", tool.name), "server": server_name, "tool": tool.name, "description": tool.description, "inputSchema": tool.input_schema.clone().unwrap_or_else(|| json!({"type":"object"}))}),
        )
    }

    fn call_from_input(&self, input: Value, cancelled: &AtomicBool) -> Result<Value, String> {
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
        self.call(name, arguments, cancelled)
    }

    fn call(
        &self,
        presented_name: &str,
        arguments: Value,
        cancelled: &AtomicBool,
    ) -> Result<Value, String> {
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
        let response = process.request_with_cancel(
            "tools/call",
            json!({"name": tool.name, "arguments": arguments}),
            Some(cancelled),
        )?;
        parse_tool_response(response)
    }
}

impl ConnectedServer {
    fn connect(config: McpServerConfig) -> Result<(Self, usize), String> {
        Self::connect_with_timeout(config, request_timeout())
    }

    fn connect_with_timeout(
        config: McpServerConfig,
        timeout: Duration,
    ) -> Result<(Self, usize), String> {
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
        set_nonblocking_stdin(&stdin)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "MCP child stdout was not piped".to_owned())?;
        set_nonblocking(&stdout)?;
        let mut process = McpProcess {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            timeout,
        };
        let handshake = (|| {
            let started = Instant::now();
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
            let mut cursor = None;
            let mut all_tools = Vec::new();
            let mut seen_cursors = std::collections::BTreeSet::new();
            let mut catalog_bytes = 0usize;
            loop {
                if started.elapsed() >= timeout {
                    return Err("MCP catalog discovery timed out".into());
                }
                let params = cursor
                    .as_ref()
                    .map_or_else(|| json!({}), |value| json!({"cursor": value}));
                let listing = process.request("tools/list", params)?;
                catalog_bytes = catalog_bytes.saturating_add(listing.to_string().len());
                if catalog_bytes > 16 * 1024 * 1024 {
                    return Err("MCP catalog exceeds the discovery size limit".into());
                }
                if listing.get("error").is_some() {
                    return Err(format!("tools/list failed: {listing}"));
                }
                if let Some(tools) = listing
                    .get("result")
                    .and_then(|result| result.get("tools"))
                    .and_then(Value::as_array)
                {
                    all_tools.extend(tools.iter().cloned());
                }
                cursor = listing
                    .get("result")
                    .and_then(|result| result.get("nextCursor"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if cursor.is_none() {
                    break;
                }
                if !seen_cursors.insert(cursor.clone()) || seen_cursors.len() > 128 {
                    return Err("MCP catalog returned a repeated cursor or too many pages".into());
                }
            }
            Ok(all_tools)
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
                    input_schema: tool.get("inputSchema").cloned(),
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
        self.request_with_cancel(method, params, None)
    }

    fn request_with_cancel(
        &mut self,
        method: &str,
        params: Value,
        cancelled: Option<&AtomicBool>,
    ) -> Result<Value, String> {
        let started = Instant::now();
        let id = self.next_id;
        self.next_id += 1;
        self.write_message_with_cancel(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }),
            cancelled,
        )?;
        let mut line = Vec::new();
        loop {
            self.check_cancelled(cancelled)?;
            if started.elapsed() >= self.timeout {
                // A timed out child may still be computing and holding resources.
                // Terminate it so the next call cannot inherit a stuck transport.
                let _ = self.child.kill();
                return Err("MCP request timed out".to_owned());
            }
            match self.stdout.fill_buf() {
                Ok([]) => return Err("MCP server closed stdout".to_owned()),
                Ok(buffer) => {
                    let newline = buffer.iter().position(|byte| *byte == b'\n');
                    let count = newline.map_or(buffer.len(), |index| index + 1);
                    if line.len().saturating_add(count) > MCP_MAX_MESSAGE_BYTES {
                        let _ = self.child.kill();
                        return Err("MCP response exceeds size limit".into());
                    }
                    line.extend_from_slice(&buffer[..count]);
                    self.stdout.consume(count);
                    if newline.is_some() {
                        let response: Value = match serde_json::from_slice(&line) {
                            Ok(response) => response,
                            Err(error) => {
                                let _ = self.child.kill();
                                return Err(format!("invalid MCP JSON response: {error}"));
                            }
                        };
                        if response.get("id") == Some(&json!(id)) {
                            return Ok(response);
                        }
                        line.clear();
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(format!("MCP read failed: {error}")),
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
        self.write_message_with_cancel(message, None)
    }

    fn check_cancelled(&mut self, cancelled: Option<&AtomicBool>) -> Result<(), String> {
        if cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            let _ = self.child.kill();
            return Err("MCP call cancelled. Its completion may be unknown; verify external state before retrying a mutation.".into());
        }
        Ok(())
    }

    fn write_message_with_cancel(
        &mut self,
        message: Value,
        cancelled: Option<&AtomicBool>,
    ) -> Result<(), String> {
        let encoded = serde_json::to_vec(&message).map_err(|error| error.to_string())?;
        if encoded.len() > MCP_MAX_MESSAGE_BYTES {
            return Err("MCP request exceeds size limit".to_owned());
        }
        let mut bytes = encoded;
        bytes.push(b'\n');
        let mut remaining = bytes.as_slice();
        let started = Instant::now();
        while !remaining.is_empty() {
            self.check_cancelled(cancelled)?;
            if started.elapsed() >= self.timeout {
                let _ = self.child.kill();
                return Err("MCP request timed out while writing".to_owned());
            }
            match self.stdin.write(remaining) {
                Ok(0) => return Err("MCP stdin closed".to_owned()),
                Ok(written) => {
                    remaining = &remaining[written..];
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(format!("MCP write failed: {error}")),
            }
        }
        self.stdin
            .flush()
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
    let images = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("image"))
        .filter_map(|block| {
            Some(json!({"data": block.get("data")?.clone(), "mediaType": block.get("mimeType").or_else(|| block.get("mediaType"))?.clone()}))
        })
        .collect::<Vec<_>>();
    if !images.is_empty() && !is_error {
        let mut envelope =
            json!({"type":"multimodal_tool_result", "text": text.join("\n"), "images": images});
        if let Some(metadata) = result.get("structuredContent") {
            envelope["metadata"] = metadata.clone();
        }
        return Ok(envelope);
    }
    if !text.is_empty() && !is_error && result.get("structuredContent").is_none() {
        Ok(Value::String(text.join("\n")))
    } else {
        Ok(result)
    }
}

fn bounded_description(value: &str) -> String {
    value.chars().take(500).collect()
}

#[cfg(unix)]
fn set_nonblocking(stdout: &ChildStdout) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    let fd = stdout.as_raw_fd();
    // SAFETY: fcntl only changes flags on this owned pipe descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err("MCP stdout flags unavailable".to_owned());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("MCP stdout could not be made nonblocking".to_owned());
    }
    Ok(())
}

#[cfg(unix)]
fn set_nonblocking_stdin(stdin: &ChildStdin) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    let fd = stdin.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("MCP stdin could not be made nonblocking".to_owned());
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_nonblocking(_stdout: &ChildStdout) -> Result<(), String> {
    Ok(())
}

#[cfg(not(unix))]
fn set_nonblocking_stdin(_stdin: &ChildStdin) -> Result<(), String> {
    Ok(())
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
    use super::{
        parse_tool_name, parse_tool_response, ConnectedServer, InboundMcpRegistry, McpServerConfig,
    };
    use super::{AtomicBool, Duration, Instant, Ordering, MCP_MAX_MESSAGE_BYTES};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Arc;

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

    #[test]
    fn tool_response_keeps_error_and_multimodal_envelopes() {
        let error = parse_tool_response(json!({
            "result": {"isError": true, "content": [{"type":"text", "text":"failed"}]}
        }))
        .unwrap();
        assert_eq!(error["isError"], true);
        assert_eq!(error["content"][0]["text"], "failed");

        let multimodal = parse_tool_response(json!({
            "result": {"content": [{"type":"text", "text":"caption"}, {"type":"image", "data":"abc", "mimeType":"image/png"}]}
        }))
        .unwrap();
        assert_eq!(multimodal["type"], "multimodal_tool_result");
        assert_eq!(multimodal["text"], "caption");
        assert_eq!(multimodal["images"][0]["mediaType"], "image/png");
    }

    fn registry() -> InboundMcpRegistry {
        let script = r#"import sys,json,time
for line in sys.stdin:
 try:
  q=json.loads(line); i=q.get('id'); m=q.get('method')
  if m=='initialize': r={'protocolVersion':'2024-11-05'}
  elif m=='tools/list': r={'tools':[{'name':'lookup','description':'find records','inputSchema':{'type':'object','properties':{'q':{'type':'string'}},'required':['q']}},{'name':'other','description':'other','inputSchema':{'type':'object'}}]}
  elif m=='tools/call':
   if q.get('params',{}).get('arguments',{}).get('stall'): time.sleep(5)
   r={'content':[{'type':'text','text':'ok'}]}
  else: continue
  print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
 except Exception: pass
"#;
        let config = McpServerConfig {
            command: "python3".into(),
            args: vec!["-u".into(), "-c".into(), script.into()],
            env: BTreeMap::new(),
            tools: Some(vec!["lookup".into()]),
        };
        let (server, _) = ConnectedServer::connect(config).expect("mock MCP connects");
        let mut servers = BTreeMap::new();
        servers.insert("mock".into(), server);
        InboundMcpRegistry {
            servers: Arc::new(servers),
        }
    }

    #[test]
    fn search_paginates_and_describe_returns_exact_schema_with_allowlist() {
        let registry = registry();
        let page = registry.search(json!({"limit": 1})).unwrap();
        assert_eq!(page["total"], 1);
        assert_eq!(page["items"][0]["tool"], "lookup");
        let exhausted = registry.search(json!({"offset": usize::MAX})).unwrap();
        assert_eq!(exhausted["items"], json!([]));
        assert!(exhausted["nextOffset"].is_null());
        let schema = registry
            .describe_tool(json!({"name":"mcp_mock_lookup"}))
            .unwrap();
        assert_eq!(schema["inputSchema"]["required"][0], "q");
        assert!(registry
            .describe_tool(json!({"name":"mcp_mock_other"}))
            .is_err());
    }

    #[test]
    fn stalled_mcp_request_times_out() {
        let script = "import sys,time; time.sleep(1)";
        let config = McpServerConfig {
            command: "python3".into(),
            args: vec!["-u".into(), "-c".into(), script.into()],
            env: BTreeMap::new(),
            tools: None,
        };
        // This child never completes initialize; use a short test-only timeout.
        let result =
            ConnectedServer::connect_with_timeout(config, std::time::Duration::from_millis(50));
        assert!(result.is_err());
    }

    #[test]
    fn cancellation_interrupts_a_running_external_tool() {
        let registry = registry();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let interrupt = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            flag.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let error = registry
            .call("mcp_mock_lookup", json!({"stall":true}), &cancelled)
            .unwrap_err();
        interrupt.join().unwrap();
        assert!(error.contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn unterminated_oversized_response_is_rejected_before_timeout() {
        let config = McpServerConfig {command:"python3".into(),args:vec!["-u".into(),"-c".into(),format!("import sys; sys.stdin.readline(); sys.stdout.write('x'*{}); sys.stdout.flush()",MCP_MAX_MESSAGE_BYTES+1)],env:BTreeMap::new(),tools:None};
        let error = match ConnectedServer::connect_with_timeout(config, Duration::from_secs(5)) {
            Ok(_) => panic!("oversized response accepted"),
            Err(error) => error,
        };
        assert!(error.contains("size limit"), "{error}");
    }
}
