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

  it("cutToolsSchemas covers the 5 registry contracts", () => {
    const schemas = cutToolsSchemas();
    expect(schemas.map((s) => s.name)).toEqual([
      "fetch",
      "probe",
      "edit",
      "render",
      "bash",
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
    expect(session.systemPrompt).toContain("video editing assistant");
    expect(session.model).toBe("claude-sonnet-4-20250514");
  });
});
