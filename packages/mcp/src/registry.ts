import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import type { ToolImpl } from "./tool-impl.js";
import { loadMcpConfig, type McpConfig, type McpServerConfig } from "./config.js";

/** Minimal interface of an MCP client used for routing (mockable in tests). */
export interface McpClientLike {
  listTools(): Promise<{
    tools: Array<{ name: string; description?: string; inputSchema?: Record<string, unknown> }>;
  }>;
  callTool(opts: {
    name: string;
    arguments?: Record<string, unknown> | undefined;
  }): Promise<{
    content?: Array<{ type: string; text?: string }> | undefined;
    isError?: boolean;
  }>;
  close(): Promise<void>;
}

export interface ConnectOptions {
  /** Spawn the real stdio transport. Override in tests with a fake client. */
  connect?: ((name: string, cfg: McpServerConfig) => Promise<McpClientLike>) | undefined;
  log?: ((line: string) => void) | undefined;
}

const MCP_TOOL_PREFIX = "mcp_";

/** Namespaced tool name exposed to the model: mcp_<server>_<tool>. */
export function mcpToolName(server: string, tool: string): string {
  return `${MCP_TOOL_PREFIX}${server}_${tool}`;
}

/**
 * Split `mcp_<server>_<tool>` back into its parts. When known server names
 * are supplied, the longest matching prefix wins so server underscores do not
 * get mistaken for the server/tool separator.
 */
export function parseMcpToolName(
  name: string,
  knownServers?: readonly string[],
): { server: string; tool: string } | undefined {
  if (!name.startsWith(MCP_TOOL_PREFIX)) return undefined;
  const rest = name.slice(MCP_TOOL_PREFIX.length);
  if (knownServers !== undefined) {
    const matches = knownServers
      .filter((server) => server.length > 0 && rest.startsWith(`${server}_`))
      .map((server) => ({
        server,
        tool: rest.slice(server.length + 1),
      }))
      .filter((candidate) => candidate.tool.length > 0)
      .sort((left, right) => right.server.length - left.server.length);
    if (matches[0]) return matches[0];
  }
  const idx = rest.indexOf("_");
  if (idx <= 0 || idx === rest.length - 1) return undefined;
  return { server: rest.slice(0, idx), tool: rest.slice(idx + 1) };
}

interface ConnectedServer {
  server: string;
  client: McpClientLike;
  /** Underlying tool names allowed for search/call; undefined means all discovered tools. */
  allowlist?: readonly string[];
}

function namespacedOrSlashName(name: string): string | undefined {
  if (name.startsWith(MCP_TOOL_PREFIX)) return name;
  const slash = name.indexOf("/");
  if (slash <= 0 || slash === name.length - 1) return undefined;
  return mcpToolName(name.slice(0, slash), name.slice(slash + 1));
}

/**
 * Registry of tools discovered from connected MCP servers, shaped so each
 * entry can be merged straight into the AgentSession ToolMap.
 */
export class McpToolRegistry {
  private readonly servers: ConnectedServer[] = [];
  private readonly byName = new Map<string, ToolImpl & { server: string; tool: string }>();

  /** All discovered MCP tool implementations keyed by their namespaced name. */
  get tools(): Record<string, ToolImpl> {
    const out: Record<string, ToolImpl> = {};
    for (const [name, impl] of this.byName) out[name] = impl;
    return out;
  }

  private allowedTool(server: string, tool: string): boolean {
    const allowlist = this.servers.find((entry) => entry.server === server)?.allowlist;
    return allowlist === undefined || allowlist.includes(tool);
  }

  /**
   * Two meta tools for the product agent: search discovered inbound MCP tools,
   * then call one by namespaced or `server/tool` name. Does not dump every
   * discovered schema into the model.
   */
  metaTools(): Record<string, ToolImpl> {
    const self = this;
    return {
      mcp_search: {
        description:
          "Search discovered inbound MCP tools by server name, tool name, or description. Empty query lists all allowed tools.",
        inputJsonSchema: {
          type: "object",
          properties: { query: { type: "string" } },
          additionalProperties: false,
        },
        async execute(input: unknown) {
          const query =
            typeof input === "object" && input !== null && !Array.isArray(input)
              && typeof (input as { query?: unknown }).query === "string"
              ? (input as { query: string }).query.trim().toLowerCase()
              : "";
          const matches: Array<{
            name: string;
            server: string;
            tool: string;
            description?: string;
          }> = [];
          for (const [name, impl] of self.byName) {
            if (!self.allowedTool(impl.server, impl.tool)) continue;
            const haystack = `${impl.server} ${impl.tool} ${name} ${impl.description ?? ""}`.toLowerCase();
            if (query && !haystack.includes(query)) continue;
            matches.push({
              name,
              server: impl.server,
              tool: impl.tool,
              ...(impl.description !== undefined ? { description: impl.description } : {}),
            });
          }
          return matches;
        },
      },
      mcp_call: {
        description:
          "Call a discovered inbound MCP tool. `name` is `mcp_<server>_<tool>` or `server/tool`.",
        inputJsonSchema: {
          type: "object",
          properties: {
            name: { type: "string", minLength: 1 },
            arguments: { type: "object" },
          },
          required: ["name"],
          additionalProperties: false,
        },
        async execute(input: unknown) {
          if (typeof input !== "object" || input === null || Array.isArray(input)) {
            throw new Error('mcp_call requires { "name": "<mcp_<server>_<tool>|server/tool>" }');
          }
          const name = (input as { name?: unknown }).name;
          if (typeof name !== "string" || name.length === 0) {
            throw new Error("mcp_call requires a non-empty name");
          }
          const args = (input as { arguments?: unknown }).arguments;
          const resolved = namespacedOrSlashName(name) ?? name;
          const parsed = parseMcpToolName(resolved, self.servers.map((server) => server.server));
          if (parsed && !self.allowedTool(parsed.server, parsed.tool)) {
            throw new Error(`MCP tool not allowed: ${name}`);
          }
          return self.call(name, args ?? {});
        },
      },
    };
  }

  /** Summary lines for `cut --mcp`: server → discovered tools. */
  describe(): Array<{ server: string; tools: Array<{ name: string; description?: string }> }> {
    return this.servers.map((s) => ({
      server: s.server,
      tools: [...this.byName.values()]
        .filter((t) => t.server === s.server)
        .map((t) => ({ name: t.tool, ...(t.description !== undefined ? { description: t.description } : {}) })),
    }));
  }

  /** Route a model tool call with the mcp_ prefix or `server/tool` to the right server. */
  async call(namespacedName: string, args: unknown): Promise<string> {
    const knownServers = this.servers.map((server) => server.server);
    const resolved = namespacedOrSlashName(namespacedName) ?? namespacedName;
    const parsed = parseMcpToolName(resolved, knownServers);
    if (!parsed) throw new Error(`Not an MCP tool: ${namespacedName}`);
    const entry = this.byName.get(resolved);
    if (!entry) throw new Error(`Unknown MCP tool: ${namespacedName}`);
    const client = this.servers.find((s) => s.server === parsed!.server)?.client;
    if (!client) throw new Error(`MCP server not connected: ${parsed.server}`);
    const result = await client.callTool({
      name: parsed.tool,
      arguments: (args ?? {}) as Record<string, unknown>,
    });
    const text = (result.content ?? [])
      .filter((c) => c.type === "text" && typeof c.text === "string")
      .map((c) => c.text as string)
      .join("\n");
    if (result.isError && !text) throw new Error(`MCP tool ${parsed.tool} failed`);
    return text;
  }

  async close(): Promise<void> {
    await Promise.allSettled(this.servers.map((s) => s.client.close()));
  }

  /** Internal: register concrete tool defs for a connected server. */
  register(
    server: string,
    client: McpClientLike,
    tools: Array<{ name: string; description?: string; inputSchema?: Record<string, unknown> }>,
    allowlist?: readonly string[],
  ): void {
    for (const [name, t] of [...this.byName]) if (t.server === server) this.byName.delete(name);
    const existing = this.servers.findIndex((s) => s.server === server);
    const connected: ConnectedServer = {
      server,
      client,
      ...(allowlist !== undefined ? { allowlist: [...allowlist] } : {}),
    };
    if (existing >= 0) this.servers[existing] = connected;
    else this.servers.push(connected);
    for (const t of tools) {
      const namespaced = mcpToolName(server, t.name);
      const self = this;
      this.byName.set(namespaced, {
        server,
        tool: t.name,
        ...(t.description !== undefined ? { description: t.description } : {}),
        inputJsonSchema:
          t.inputSchema && Object.keys(t.inputSchema).length > 0 ? t.inputSchema : { type: "object" },
        async execute(input: unknown): Promise<string> {
          return self.call(namespaced, input);
        },
      });
    }
  }
}

function defaultConnect(name: string, cfg: McpServerConfig): Promise<McpClientLike> {
  const transport = new StdioClientTransport({
    command: cfg.command,
    ...(cfg.args ? { args: cfg.args } : {}),
    ...(cfg.env ? { env: { ...cfg.env } } : {}),
  });
  const client = new Client({ name: "cut", version: "0.1.0" });
  void name;
  return client.connect(transport).then(() => client as unknown as McpClientLike);
}

/**
 * Connect to every configured MCP server and discover its tools.
 * Connection failures degrade gracefully: the server is skipped with a warning.
 */
export async function connectMcpServers(
  config: McpConfig,
  opts: ConnectOptions = {},
): Promise<McpToolRegistry> {
  const log = opts.log ?? (() => {});
  const connect = opts.connect ?? defaultConnect;
  const registry = new McpToolRegistry();
  await Promise.all(
    Object.entries(config.mcpServers).map(async ([name, cfg]) => {
      let client: McpClientLike | undefined;
      try {
        client = await connect(name, cfg);
        const res = await client.listTools();
        registry.register(name, client, res.tools, cfg.tools);
        log(`mcp: connected ${name} (${res.tools.length} tools)`);
      } catch (err) {
        // A client may have connected successfully before listTools failed;
        // release that transport instead of leaving a child process/socket
        // alive after this server is skipped.
        await client?.close().catch(() => undefined);
        log(
          `warning: mcp server '${name}' unavailable (${
            err instanceof Error ? err.message : String(err)
          }), skipping`,
        );
      }
    }),
  );
  return registry;
}

/**
 * Convenience: load config from disk and connect. Returns an empty registry
 * when no mcp.json exists anywhere.
 */
export async function initMcpTools(
opts: { home?: string; cwd?: string; read?: ((p: string) => Promise<string>) | undefined } &
  ConnectOptions = {},
): Promise<McpToolRegistry> {
const config = await loadMcpConfig({
  ...(opts.home !== undefined ? { home: opts.home } : {}),
  ...(opts.cwd !== undefined ? { cwd: opts.cwd } : {}),
  ...(opts.read !== undefined ? { read: opts.read } : {}),
});
return connectMcpServers(config, {
  connect: opts.connect,
  log: opts.log,
});
}
