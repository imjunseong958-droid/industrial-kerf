import { describe, it, expect } from "vitest";
import type Anthropic from "@anthropic-ai/sdk";
import type { MessageCreateParamsNonStreaming, Message } from "@anthropic-ai/sdk/resources/messages";
import {
  AgentSession,
  buildToolsParam,
  cutToolsSchemas,
  type AgentEvent,
} from "../src/index.js";

function makeResponse(partial: Record<string, unknown>): Message {
  return {
    id: "msg_1",
    type: "message",
    role: "assistant",
    model: "claude-sonnet-4-20250514",
    content: [],
    stop_reason: "end_turn",
    stop_sequence: null,
    usage: { input_tokens: 1, output_tokens: 1 },
    ...partial,
  } as unknown as Message;
}

describe("tool schemas", () => {
  it("builds Anthropic tool params only for provided tools", () => {
    const tools = buildToolsParam({ edit: { execute: async () => ({}) as never } });
    expect(tools.map((t) => t.name)).toEqual(["edit"]);
    expect(tools[0]!.input_schema.type).toBe("object");
  });

  it("falls back to a stable description for extension tools without one", () => {
    const tools = buildToolsParam({
      mcp_external_no_description: { execute: async () => ({}) as never },
    });
    expect(tools[0]).toMatchObject({
      name: "mcp_external_no_description",
      description: "Call the mcp_external_no_description tool.",
    });
  });

  it("cutToolsSchemas covers the built-in registry contracts", () => {
    const schemas = cutToolsSchemas();
    expect(schemas.map((s) => s.name)).toEqual([
      "fetch",
      "probe",
      "timeline",
      "project_edit",
      "render",
    ]);
  });
});

describe("AgentSession loop", () => {
  it("calls edit then finishes with a text-only response", async () => {
    const calls: Array<Record<string, unknown>> = [];
    const mockClient = {
      create: async (params: MessageCreateParamsNonStreaming) => {
        calls.push(params as unknown as Record<string, unknown>);
        if (calls.length === 1) {
          return makeResponse({
            content: [
              { type: "text", text: "Applying the edit." },
              {
                type: "tool_use",
                id: "tu_1",
                name: "edit",
                input: { op: { op: "trim", target: "clip1", start: 0, end: 5 } },
              },
            ],
            stop_reason: "tool_use",
          });
        }
        return makeResponse({
          content: [{ type: "text", text: "Done trimming." }],
          stop_reason: "end_turn",
        });
      },
    };

    const executed: Array<{ name: string; input: unknown }> = [];
    const events: AgentEvent[] = [];
    const session = new AgentSession({
      client: mockClient as unknown as Pick<typeof Anthropic.prototype.messages, "create">,
      tools: {
        edit: {
          execute: async (input: unknown) => {
            executed.push({ name: "edit", input });
            return { clips: [] };
          },
        },
      },
    });
    session.onEvent((e) => events.push(e));

    const final = await session.run("trim clip1 to 5s");
    expect(final).toBe("Done trimming.");
    expect(executed).toEqual([
      { name: "edit", input: { op: { op: "trim", target: "clip1", start: 0, end: 5 } } },
    ]);

    // Second request carries assistant tool_use + user tool_result messages.
    const second = calls[1]!;
    const msgs = second["messages"] as Array<{ role: string; content: unknown }>;
    expect(msgs).toHaveLength(3);
    expect(msgs[1]!.role).toBe("assistant");
    expect(msgs[2]!.role).toBe("user");
    const result = (msgs[2]!.content as Array<Record<string, unknown>>)[0]!;
    expect(result["type"]).toBe("tool_result");
    expect(result["tool_use_id"]).toBe("tu_1");
    expect(JSON.parse(result["content"] as string)).toEqual({ clips: [] });

    // Events fired in order.
    const types = events.map((e) => e.type);
    expect(types).toContain("text_delta");
    expect(types.filter((t) => t === "tool_call_started")).toHaveLength(1);
    expect(types.filter((t) => t === "tool_call_finished")).toHaveLength(1);
    expect(events.at(-1)!.type).toBe("done");

    // History is serializable.
    expect(session.messages).toHaveLength(4);
  });

  it("normalizes a multimodal tool result into native Anthropic text and image blocks", async () => {
    let callCount = 0;
    const calls: MessageCreateParamsNonStreaming[] = [];
    const mockClient = {
      create: async (params: MessageCreateParamsNonStreaming) => {
        calls.push(params);
        callCount++;
        if (callCount === 1) {
          return makeResponse({
            content: [
              { type: "tool_use", id: "probe_1", name: "probe", input: { action: "frames" } },
            ],
            stop_reason: "tool_use",
          });
        }
        return makeResponse({ content: [{ type: "text", text: "I can see the frame." }] });
      },
    };

    const session = new AgentSession({
      client: mockClient as unknown as Pick<typeof Anthropic.prototype.messages, "create">,
      tools: {
        probe: {
          execute: async () => ({
            type: "multimodal_tool_result" as const,
            text: "Frame at 1.0s",
            metadata: { timestamp: 1 },
            images: [{ data: "aGVsbG8=", mediaType: "image/png" as const }],
          }),
        },
      },
    });

    await session.run("inspect the frame");
    const resultMessage = calls[1]!.messages.at(-1)!;
    const result = (resultMessage.content as unknown as Array<Record<string, unknown>>)[0]!;
    expect(result).toMatchObject({ type: "tool_result", tool_use_id: "probe_1" });
    expect(result.content).toEqual([
      { type: "text", text: 'Frame at 1.0s\n{"timestamp":1}' },
      {
        type: "image",
        source: { type: "base64", media_type: "image/png", data: "aGVsbG8=" },
      },
    ]);
    expect(JSON.parse(JSON.stringify(session.messages))).toEqual(session.messages);
  });

  it("turns invalid or oversized multimodal output into a safe error result", async () => {
    let callCount = 0;
    const mockClient = {
      create: async () => {
        callCount++;
        if (callCount === 1) {
          return makeResponse({
            content: [{ type: "tool_use", id: "probe_bad", name: "probe", input: {} }],
            stop_reason: "tool_use",
          });
        }
        return makeResponse({ content: [{ type: "text", text: "recovered" }] });
      },
    };
    const session = new AgentSession({
      client: mockClient as unknown as Pick<typeof Anthropic.prototype.messages, "create">,
      tools: {
        probe: {
          execute: async () => ({
            type: "multimodal_tool_result" as const,
            images: [{ data: "not base64", mediaType: "image/png" as const }],
          }),
        },
      },
    });

    await session.run("inspect");
    const last = session.messages.at(-2)!;
    const content = last.content as unknown as Array<Record<string, unknown>>;
    expect(content[0]!.content).toBe('{"error":"Tool result image 1 is not valid base64"}');
  });

  it("reports tool errors as error results and continues", async () => {
    let callCount = 0;
    const mockClient = {
      create: async () => {
        callCount++;
        if (callCount === 1) {
          return makeResponse({
            content: [
              {
                type: "tool_use",
                id: "tu_2",
                name: "bash",
                input: { command: "exit 1" },
              },
            ] as unknown as Message["content"],
            stop_reason: "tool_use",
          });
        }
        return makeResponse({
          content: [{ type: "text", text: "recovered" }],
        });
      },
    };

    const session = new AgentSession({
      client: mockClient as unknown as Pick<typeof Anthropic.prototype.messages, "create">,
      tools: {
        bash: {
          execute: async () => {
            throw new Error("boom");
          },
        },
      },
    });
    const finished: AgentEvent[] = [];
    session.onEvent((e) => {
      if (e.type === "tool_call_finished") finished.push(e);
    });
    const text = await session.run("do a thing");
    expect(text).toBe("recovered");
    expect(finished[0]).toMatchObject({ ok: false });
    expect(session.messages.at(-2)!.content[0]).toMatchObject({
      type: "tool_result",
      tool_use_id: "tu_2",
    });
  });

  it("save/load round-trips message history", async () => {
    const path = new URL("./tmp-session.json", import.meta.url).pathname;
    const session = new AgentSession({
      client: { create: async () => makeResponse({}) } as never,
    });
    session.messages.push(
      { role: "user", content: "hi" },
      { role: "assistant", content: [{ type: "text", text: "hello" }] },
    );
    await session.save(path);
    const loaded = await AgentSession.load(path);
    expect(loaded.messages).toEqual(session.messages);

    const { unlink } = await import("node:fs/promises");
    await unlink(path).catch(() => {});
  });

  it("uses default system prompt when none given", () => {
    const session = new AgentSession({
      client: { create: async () => makeResponse({}) } as never,
    });
    expect(session.systemPrompt).toContain("video editing assistant for Cut");
    expect(session.systemPrompt).toContain("project.cut.json");
    expect(session.systemPrompt).toContain("project_edit (the only mutation tool");
    expect(session.systemPrompt).toContain("normalized 0..1 crops");
    expect(session.systemPrompt).toContain("Do not call a tool named edit");
    expect(session.model).toBe("claude-sonnet-4-20250514");
  });
});
