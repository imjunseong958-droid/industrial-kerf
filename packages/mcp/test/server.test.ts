import { describe, expect, it, vi } from "vitest";
import path from "node:path";
import {
  CUT_MCP_CORE_TOOL_NAMES,
  CUT_MCP_MVP_TOOL_NAMES,
  CUT_MCP_TOOL_NAMES,
  CUT_MCP_TOOL_SCHEMAS,
  createCutMcpRouter,
  createCutMcpServer,
  namespaceCutMcpToolName,
  parseCutMcpToolName,
} from "../src/server.js";
import { parseCutMcpArgs, parseCutMcpDir } from "../src/stdio.js";

describe("cut MCP capability router", () => {
  it("registers dotted capabilities under an optional namespace", async () => {
    const inspect = vi.fn(async (input: unknown) => ({ input, project: "demo" }));
    const router = createCutMcpRouter(
      { project: { inspect } },
      { namespace: "cut" },
    );

    expect(router.toolNames).toEqual(["cut.project.inspect"]);
    expect(router.listTools()[0]).toMatchObject({
      name: "cut.project.inspect",
      inputSchema: { type: "object", additionalProperties: false },
    });
    expect(namespaceCutMcpToolName("cut", "project.inspect")).toBe(
      "cut.project.inspect",
    );
    expect(parseCutMcpToolName("cut.project.inspect", "cut")).toBe(
      "project.inspect",
    );

    const result = await router.call("cut.project.inspect", { include: "full" });
    expect(result.isError).toBeUndefined();
    expect(result.structuredContent).toMatchObject({ ok: true });
    expect((result.structuredContent.result as { project: string }).project).toBe(
      "demo",
    );
    expect(inspect).toHaveBeenCalledWith(
      { include: "full" },
      expect.objectContaining({
        toolName: "cut.project.inspect",
        capability: "project.inspect",
      }),
    );
  });

  it("validates arguments before invoking an injected handler", async () => {
    const apply = vi.fn(async (input: unknown) => input);
    const router = createCutMcpRouter({ timeline: { apply } });

    const invalid = await router.call("timeline.apply", {
      op: { op: "not-a-supported-operation" },
    });
    expect(invalid.isError).toBe(true);
    expect(invalid.structuredContent).toMatchObject({
      ok: false,
      error: { code: "INVALID_ARGUMENTS" },
    });
    expect(apply).not.toHaveBeenCalled();

    const extra = await router.call("timeline.apply", {
      op: { op: "trim" },
      unexpected: true,
    });
    expect(extra.isError).toBe(true);
    expect(apply).not.toHaveBeenCalled();

    const valid = await router.call("timeline.apply", {
      op: { op: "trim", clipId: "clip-1", newOut: 2 },
    });
    expect(valid.isError).toBeUndefined();
    expect(apply).toHaveBeenCalledTimes(1);
  });

  it("turns unknown tools and handler failures into structured errors", async () => {
    const router = createCutMcpRouter({
      project: {
        inspect: async () => {
          throw new Error("state unavailable");
        },
      },
    });

    const unknown = await router.call("bash", { command: "echo unsafe" });
    expect(unknown.isError).toBe(true);
    expect(unknown.structuredContent).toMatchObject({
      ok: false,
      error: { code: "UNKNOWN_TOOL" },
    });

    const failed = await router.call("project.inspect", {});
    expect(failed.isError).toBe(true);
    expect(failed.structuredContent).toMatchObject({
      ok: false,
      error: { code: "HANDLER_FAILED", message: "state unavailable" },
    });
    expect(JSON.parse(failed.content[0]!.text)).toMatchObject({
      ok: false,
      error: { code: "HANDLER_FAILED" },
    });
  });

  it("advertises only safe capability names, never an unrestricted bash tool", () => {
    const router = createCutMcpRouter(
      {},
      { includeUnavailable: true, namespace: "cut" },
    );
    const names = router.listTools().map((tool) => tool.name);
    expect(names).toEqual(CUT_MCP_TOOL_NAMES.map((name) => `cut.${name}`));
    expect(names.some((name) => name === "bash" || name.endsWith(".bash"))).toBe(
      false,
    );
    expect(CUT_MCP_TOOL_SCHEMAS["timeline.apply"]).toMatchObject({
      type: "object",
      additionalProperties: false,
    });
  });

  it("returns a structured unavailable error for unimplemented advertised capabilities", async () => {
    const router = createCutMcpRouter({}, { includeUnavailable: true });
    const result = await router.call("timeline.query", {});
    expect(result).toMatchObject({ isError: true });
    expect(result.structuredContent).toMatchObject({
      ok: false,
      error: { code: "CAPABILITY_UNAVAILABLE" },
    });
  });

  it("creates a transport-neutral SDK server with JSON-schema tool registration", () => {
    const server = createCutMcpServer(
      { project: { inspect: async () => ({ ok: true }) } },
      { name: "cut-test", version: "1.2.3" },
    );
    expect(server.listTools()).toHaveLength(1);
    expect(server.listTools()[0]!.inputSchema.type).toBe("object");
    expect(server.server).toBeDefined();
  });

  it("filters to the core editing loop when profile is core", async () => {
    const apply = vi.fn(async () => ({ ok: true }));
    const inspect = vi.fn(async () => ({ ok: true }));
    const cancel = vi.fn(async () => ({ ok: true }));
    const router = createCutMcpRouter(
      {
        project: { inspect },
        timeline: { apply, query: async () => ({}) },
        render: { cancel },
      },
      { profile: "core" },
    );
    expect(router.hasTool("timeline.apply")).toBe(true);
    expect(router.hasTool("project.inspect")).toBe(false);
    expect(router.hasTool("render.cancel")).toBe(false);
    const applied = await router.call("timeline.apply", { op: { op: "trim", clipId: "c1" } });
    expect(applied.isError).toBeUndefined();
    const missing = await router.call("project.inspect", {});
    expect(missing.isError).toBe(true);
    expect(missing.structuredContent).toMatchObject({
      ok: false,
      error: { code: "UNKNOWN_TOOL" },
    });
    expect(inspect).not.toHaveBeenCalled();
    expect(cancel).not.toHaveBeenCalled();
  });

  it("keeps MVP names aliased to core and lists core tools when includeUnavailable", () => {
    expect(CUT_MCP_MVP_TOOL_NAMES).toEqual(CUT_MCP_CORE_TOOL_NAMES);
    const router = createCutMcpRouter({}, { profile: "core", includeUnavailable: true });
    expect(router.toolNames).toEqual(
      CUT_MCP_TOOL_NAMES.filter((name) =>
        (CUT_MCP_CORE_TOOL_NAMES as readonly string[]).includes(name),
      ),
    );
    expect(router.toolNames).toContain("skills.read");
    expect(router.toolNames).not.toContain("project.inspect");
    expect(router.toolNames).not.toContain("render.cancel");
  });

  it("does not filter createCutMcpRouter by default so inspect remains callable", () => {
    const router = createCutMcpRouter({
      project: { inspect: async () => ({ ok: true }) },
    });
    expect(router.hasTool("project.inspect")).toBe(true);
  });

  it("keeps the timeline.apply JSON schema under a 7700-byte budget", () => {
    // Tripwire only: 6658 bytes today, 115% ≈ 7657, rounded to 7700.
    const size = JSON.stringify(CUT_MCP_TOOL_SCHEMAS["timeline.apply"]).length;
    expect(size).toBeLessThan(7700);
  });
});

describe("cut-mcp argv parsing", () => {
  it("defaults to the core profile and resolves --dir", () => {
    const parsed = parseCutMcpArgs(["--dir", "/tmp/cut-project"], {});
    expect(parsed.profile).toBe("core");
    expect(parsed.dir).toBe(path.resolve("/tmp/cut-project"));
    expect(parseCutMcpDir(["--dir", "/tmp/cut-project"], {})).toBe(path.resolve("/tmp/cut-project"));
  });

  it("selects full from --full or CUT_MCP_PROFILE", () => {
    expect(parseCutMcpArgs(["--full"], {}).profile).toBe("full");
    expect(parseCutMcpArgs([], { CUT_MCP_PROFILE: "full" }).profile).toBe("full");
    expect(parseCutMcpArgs(["--full"], { CUT_MCP_PROFILE: "core" }).profile).toBe("full");
  });

  it("documents --dir and --full in usage errors", () => {
    expect(() => parseCutMcpArgs(["--help"], {})).toThrow(/--dir/);
    expect(() => parseCutMcpArgs(["--help"], {})).toThrow(/--full/);
    expect(() => parseCutMcpArgs(["--unknown"], {})).toThrow(/unknown cut-mcp option/);
  });
});
