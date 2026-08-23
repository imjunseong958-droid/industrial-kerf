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

/** Split `mcp_<server>_<tool>` back into its parts (server names may contain _). */
export function parseMcpToolName(
  name: string,
): { server: string; tool: string } | undefined {
  if (!name.startsWith(MCP_TOOL_PREFIX)) return undefined;
  const rest = name.slice(MCP_TOOL_PREFIX.length);
  const idx = rest.indexOf("_");
  if (idx <= 0 || idx === rest.length - 1) return undefined;
  return { server: rest.slice(0, idx), tool: rest.slice(idx + 1) };
}

interface ConnectedServer {
  server: string;
  client: McpClientLike;
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

  /** Summary lines for `cut --mcp`: server → discovered tools. */
  describe(): Array<{ server: string; tools: Array<{ name: string; description?: string }> }> {
    return this.servers.map((s) => ({
      server: s.server,
      tools: [...this.byName.values()]
        .filter((t) => t.server === s.server)
        .map((t) => ({ name: t.tool, ...(t.description !== undefined ? { description: t.description } : {}) })),
    }));
  }

  /** Route a model tool call with the mcp_ prefix to the right server. */
  async call(namespacedName: string, args: unknown): Promise<string> {
    const parsed = parseMcpToolName(namespacedName);
    if (!parsed) throw new Error(`Not an MCP tool: ${namespacedName}`);
    const entry = this.byName.get(namespacedName);
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
  register(server: string, client: McpClientLike, tools: Array<{ name: string; description?: string; inputSchema?: Record<string, unknown> }>): void {
    for (const [name, t] of [...this.byName]) if (t.server === server) this.byName.delete(name);
    const existing = this.servers.findIndex((s) => s.server === server);
    if (existing >= 0) this.servers[existing]!.client = client;
    else this.servers.push({ server, client });
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
      try {
        const client = await connect(name, cfg);
        const res = await client.listTools();
        registry.register(name, client, res.tools);
        log(`mcp: connected ${name} (${res.tools.length} tools)`);
      } catch (err) {
        log(
          `warning: mcp server '${name}' unavailable (${
            err instanceof Error ? err.message : String(err)
          }) — skipping`,
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
