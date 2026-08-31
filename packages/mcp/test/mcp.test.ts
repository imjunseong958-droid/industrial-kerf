import { describe, expect, it } from "vitest";
import { loadMcpConfig } from "../src/config.js";
import {
  connectMcpServers,
  initMcpTools,
  mcpToolName,
  parseMcpToolName,
  type McpClientLike,
} from "../src/registry.js";

function fakeClient(tools: Array<{ name: string; description?: string; inputSchema?: Record<string, unknown> }>, calls: string[] = []): McpClientLike {
  return {
    async listTools() {
      return { tools };
    },
    async callTool(opts) {
      calls.push(opts.name);
      return { content: [{ type: "text", text: `echo:${opts.name}:${JSON.stringify(opts.arguments ?? {})}` }] };
    },
    async close() {},
  };
}

describe("loadMcpConfig", () => {
  it("merges user then project config with project winning", async () => {
    const files: Record<string, string> = {
      "/home/u/.cut/mcp.json": JSON.stringify({
        mcpServers: {
          fs: { command: "npx", args: ["-y", "mcp-fs"] },
          shared: { command: "user-cmd" },
        },
      }),
      "/proj/.cut/mcp.json": JSON.stringify({
        mcpServers: { shared: { command: "project-cmd", env: { FOO: "1" } } },
      }),
    };
    const cfg = await loadMcpConfig({
      home: "/home/u",
      cwd: "/proj",
      read: async (p) => {
        if (!(p in files)) throw new Error("missing");
        return files[p]!;
      },
    });
    expect(Object.keys(cfg.mcpServers).sort()).toEqual(["fs", "shared"]);
    expect(cfg.mcpServers["shared"]!.command).toBe("project-cmd");
    expect(cfg.mcpServers["fs"]!.args).toEqual(["-y", "mcp-fs"]);
  });

  it("parses optional per-server tool allowlists", async () => {
    const cfg = await loadMcpConfig({
      home: "/home/u",
      cwd: "/proj",
      read: async (p) => {
        if (p !== "/proj/.cut/mcp.json") throw new Error("missing");
        return JSON.stringify({
          mcpServers: {
            hello: { command: "fake", tools: ["greet", 1, "search"] },
          },
        });
      },
    });
    expect(cfg.mcpServers["hello"]!.tools).toEqual(["greet", "search"]);
  });

  it("returns empty config when no files exist", async () => {
    const cfg = await loadMcpConfig({ home: "/none", cwd: "/none2", read: async () => { throw new Error("nope"); } });
    expect(cfg.mcpServers).toEqual({});
  });

  it("throws on malformed json (caller warns)", async () => {
    await expect(
      loadMcpConfig({ home: "/h", cwd: "/c", read: async () => "{not json" }),
    ).rejects.toThrow();
  });
});

describe("namespacing", () => {
  it("round-trips server/tool names", () => {
    const n = mcpToolName("fs", "read_file");
    expect(n).toBe("mcp_fs_read_file");
    expect(parseMcpToolName(n)).toEqual({ server: "fs", tool: "read_file" });
    const underscored = mcpToolName("team_alpha", "read_file");
    expect(parseMcpToolName(underscored, ["team", "team_alpha"]))
      .toEqual({ server: "team_alpha", tool: "read_file" });
  });
  it("rejects non-mcp names", () => {
    expect(parseMcpToolName("fetch")).toBeUndefined();
  });
});

describe("connectMcpServers", () => {
  const toolDefs = [
    { name: "greet", description: "Say hello", inputSchema: { type: "object", properties: { who: { type: "string" } }, required: ["who"] } },
  ];

  it("discovers tools with namespaced entries and routes calls", async () => {
    const calls: string[] = [];
    const registry = await connectMcpServers(
      { mcpServers: { hello: { command: "fake" } } },
      { connect: async () => fakeClient(toolDefs, calls) },
    );
    const tools = registry.tools;
    expect(Object.keys(tools)).toEqual(["mcp_hello_greet"]);
    expect(tools["mcp_hello_greet"]!.description).toBe("Say hello");
    expect(tools["mcp_hello_greet"]!.inputJsonSchema).toEqual(toolDefs[0]!.inputSchema);
    const out = await tools["mcp_hello_greet"]!.execute({ who: "world" });
    expect(out).toBe('echo:greet:{"who":"world"}');
    expect(calls).toEqual(["greet"]);
    await registry.close();
  });

  it("routes servers whose names contain underscores", async () => {
    const calls: string[] = [];
    const registry = await connectMcpServers(
      { mcpServers: { team_alpha: { command: "fake" } } },
      { connect: async () => fakeClient(toolDefs, calls) },
    );
    const out = await registry.tools["mcp_team_alpha_greet"]!.execute({ who: "world" });
    expect(out).toContain("echo:greet");
    expect(calls).toEqual(["greet"]);
    await registry.close();
  });

  it("skips failing servers with a warning", async () => {
    const lines: string[] = [];
    const registry = await connectMcpServers(
      { mcpServers: { bad: { command: "boom" }, good: { command: "ok" } } },
      {
        connect: async (name) => {
          if (name === "bad") throw new Error("spawn failed");
          return fakeClient(toolDefs);
        },
        log: (l) => lines.push(l),
      },
    );
    expect(Object.keys(registry.tools)).toEqual(["mcp_good_greet"]);
    expect(lines.some((l) => l.includes("'bad' unavailable"))).toBe(true);
    await registry.close();
  });

  it("closes a client when discovery fails after connection", async () => {
    let closed = 0;
    const registry = await connectMcpServers(
      { mcpServers: { broken: { command: "fake" } } },
      {
        connect: async () => ({
          async listTools() {
            throw new Error("list failed");
          },
          async callTool() {
            return { content: [] };
          },
          async close() {
            closed++;
          },
        }),
      },
    );
    expect(registry.tools).toEqual({});
    expect(closed).toBe(1);
    await registry.close();
  });

  it("routes unknown / non-mcp names to errors", async () => {
    const registry = await connectMcpServers(
      { mcpServers: { s: { command: "x" } } },
      { connect: async () => fakeClient(toolDefs) },
    );
    await expect(registry.call("mcp_s_nope", {})).rejects.toThrow(/Unknown MCP tool/);
    await expect(registry.call("fetch", {})).rejects.toThrow(/Not an MCP tool/);
  });

  it("describes servers for cut --mcp", async () => {
    const registry = await connectMcpServers(
      { mcpServers: { hello: { command: "x" } } },
      { connect: async () => fakeClient(toolDefs) },
    );
    expect(registry.describe()).toEqual([
      { server: "hello", tools: [{ name: "greet", description: "Say hello" }] },
    ]);
  });
});

describe("initMcpTools", () => {
  it("loads config from disk and connects", async () => {
    const registry = await initMcpTools({
      home: "/h",
      cwd: "/c",
      read: async (p) =>
        p === "/h/.cut/mcp.json"
          ? JSON.stringify({ mcpServers: { s1: { command: "a" } } })
          : JSON.stringify({ mcpServers: { s2: { command: "b" } } }),
      connect: async (name) =>
        fakeClient([{ name: `t_${name}`, description: `${name} tool`, inputSchema: { type: "object" } }]),
    });
    expect(Object.keys(registry.tools).sort()).toEqual(["mcp_s1_t_s1", "mcp_s2_t_s2"]);
    await registry.close();
  });
});

describe("inbound MCP meta tools", () => {
  const toolDefs = [
    { name: "greet", description: "Say hello", inputSchema: { type: "object", properties: { who: { type: "string" } } } },
    { name: "secret", description: "Hidden admin tool", inputSchema: { type: "object" } },
  ];

  it("exposes mcp_search and mcp_call while leaving registry.tools for listing", async () => {
    const calls: string[] = [];
    const registry = await connectMcpServers(
      { mcpServers: { hello: { command: "fake" } } },
      { connect: async () => fakeClient(toolDefs, calls) },
    );
    expect(Object.keys(registry.tools).sort()).toEqual(["mcp_hello_greet", "mcp_hello_secret"]);
    const meta = registry.metaTools();
    expect(Object.keys(meta).sort()).toEqual(["mcp_call", "mcp_search"]);
    const listed = await meta["mcp_search"]!.execute({});
    expect(listed).toEqual(expect.arrayContaining([
      expect.objectContaining({ name: "mcp_hello_greet", server: "hello", tool: "greet" }),
      expect.objectContaining({ name: "mcp_hello_secret", tool: "secret" }),
    ]));
    const filtered = await meta["mcp_search"]!.execute({ query: "hello" });
    expect(filtered).toEqual(expect.arrayContaining([
      expect.objectContaining({ tool: "greet" }),
    ]));
    const out = await meta["mcp_call"]!.execute({ name: "hello/greet", arguments: { who: "world" } });
    expect(out).toBe('echo:greet:{"who":"world"}');
    const namespaced = await meta["mcp_call"]!.execute({ name: "mcp_hello_greet", arguments: { who: "again" } });
    expect(namespaced).toContain("echo:greet");
    await registry.close();
  });

  it("allowlists searchable and callable meta tools without hiding listing names", async () => {
    const registry = await connectMcpServers(
      { mcpServers: { hello: { command: "fake", tools: ["greet"] } } },
      { connect: async () => fakeClient(toolDefs) },
    );
    expect(Object.keys(registry.tools).sort()).toEqual(["mcp_hello_greet", "mcp_hello_secret"]);
    expect(registry.describe()).toEqual([
      {
        server: "hello",
        tools: [
          { name: "greet", description: "Say hello" },
          { name: "secret", description: "Hidden admin tool" },
        ],
      },
    ]);
    const meta = registry.metaTools();
    await expect(meta["mcp_search"]!.execute({})).resolves.toEqual([
      expect.objectContaining({ tool: "greet", name: "mcp_hello_greet" }),
    ]);
    await expect(meta["mcp_call"]!.execute({ name: "mcp_hello_secret" })).rejects.toThrow(/not allowed/);
    await expect(meta["mcp_call"]!.execute({ name: "hello/greet" })).resolves.toContain("echo:greet");
    await registry.close();
  });
});
