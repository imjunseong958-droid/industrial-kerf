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
