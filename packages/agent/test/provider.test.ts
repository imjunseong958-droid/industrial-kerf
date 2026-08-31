import { describe, it, expect } from "vitest";
import {
  AgentSession,
  OpenAIProvider,
  OpenAIResponsesProvider,
  toOpenAIMessages,
  toOpenAIResponsesInput,
  type AgentEvent,
} from "../src/index.js";

/** Build a fake streaming response for chat.completions.create. */
function streamOf(chunks: Array<Record<string, unknown>>) {
  return (async function* () {
    for (const c of chunks) yield c;
  })();
}

function textDelta(text: string): Record<string, unknown> {
  return { choices: [{ index: 0, delta: { content: text }, finish_reason: null }] };
}

function makeMockCompletions(calls: Array<Record<string, unknown>>) {
  return {
    create: async (params: Record<string, unknown>) => {
      calls.push(params);
      if (calls.length === 1) {
        // Turn 1: some text + a tool call whose arguments arrive in fragments.
        return streamOf([
          textDelta("Applying the edit."),
          {
            choices: [
              {
                delta: {
                  tool_calls: [
                    {
                      index: 0,
                      id: "call_1",
                      function: { name: "edit", arguments: '{"op":{"op":"trim",' },
                    },
                  ],
                },
                finish_reason: null,
              },
            ],
          },
          {
            choices: [
              {
                delta: {
                  tool_calls: [
                    { index: 0, function: { arguments: '"target":"clip1","start":0,"end":5}}' } },
                  ],
                },
                finish_reason: null,
              },
            ],
          },
          {
            choices: [{ delta: {}, finish_reason: "tool_calls" }],
          },
        ]);
      }
      // Turn 2: final text-only answer.
      return streamOf([
        textDelta("Done trimming."),
        { choices: [{ delta: {}, finish_reason: "stop" }] },
      ]);
    },
  };
}

describe("toOpenAIMessages", () => {
  it("translates anthropic-format history to chat messages", () => {
    const out = toOpenAIMessages([
      { role: "user", content: "hi" },
      {
        role: "assistant",
        content: [
          { type: "text", text: "calling tool" },
          { type: "tool_use", id: "tu_1", name: "bash", input: { command: "ls" } },
        ],
      },
      {
        role: "user",
        content: [
          { type: "tool_result", tool_use_id: "tu_1", content: '{"ok":true}' },
        ],
      },
    ]);
    expect(out).toEqual([
      { role: "user", content: "hi" },
      {
        role: "assistant",
        content: "calling tool",
        tool_calls: [
          {
            id: "tu_1",
            type: "function",
            function: { name: "bash", arguments: '{"command":"ls"}' },
          },
        ],
      },
      { role: "tool", tool_call_id: "tu_1", content: '{"ok":true}' },
    ]);
  });

  it("keeps tool output text in the tool message and forwards images as linked user content", () => {
    const out = toOpenAIMessages([
      {
        role: "assistant",
        content: [{ type: "tool_use", id: "probe_1", name: "probe", input: {} }],
      },
      {
        role: "user",
        content: [
          {
            type: "tool_result",
            tool_use_id: "probe_1",
            content: [
              { type: "text", text: "Frame at 1.0s" },
              {
                type: "image",
                source: { type: "base64", media_type: "image/png", data: "aGVsbG8=" },
              },
            ],
          },
        ],
      },
    ]);
    expect(out).toEqual([
      {
        role: "assistant",
        content: null,
        tool_calls: [
          {
            id: "probe_1",
            type: "function",
            function: { name: "probe", arguments: "{}" },
          },
        ],
      },
      { role: "tool", tool_call_id: "probe_1", content: "Frame at 1.0s" },
      {
        role: "user",
        content: [
          {
            type: "text",
            text: "Visual output for tool call probe_1 (see the tool result immediately above).",
          },
          { type: "image_url", image_url: { url: "data:image/png;base64,aGVsbG8=" } },
        ],
      },
    ]);
  });
});

describe("toOpenAIResponsesInput", () => {
  it("translates assistant tool calls and function outputs", () => {
    expect(
      toOpenAIResponsesInput([
        { role: "user", content: "hi" },
        {
          role: "assistant",
          content: [
            { type: "text", text: "calling tool" },
            { type: "tool_use", id: "call_1", name: "bash", input: { command: "ls" } },
          ],
        },
        {
          role: "user",
          content: [
            { type: "tool_result", tool_use_id: "call_1", content: '{"ok":true}' },
          ],
        },
      ]),
    ).toEqual([
      { role: "user", content: "hi" },
      { role: "assistant", content: "calling tool" },
      { type: "function_call", call_id: "call_1", name: "bash", arguments: '{"command":"ls"}' },
      { type: "function_call_output", call_id: "call_1", output: '{"ok":true}' },
    ]);
  });

  it("forwards tool-result images as input_image content after function output", () => {
    expect(
      toOpenAIResponsesInput([
        {
          role: "assistant",
          content: [{ type: "tool_use", id: "call_1", name: "probe", input: {} }],
        },
        {
          role: "user",
          content: [
            {
              type: "tool_result",
              tool_use_id: "call_1",
              content: [
                { type: "text", text: "Frame at 1.0s" },
                {
                  type: "image",
                  source: { type: "base64", media_type: "image/jpeg", data: "aGVsbG8=" },
                },
              ],
            },
          ],
        },
      ]),
    ).toEqual([
      { type: "function_call", call_id: "call_1", name: "probe", arguments: "{}" },
      { type: "function_call_output", call_id: "call_1", output: "Frame at 1.0s" },
      {
        role: "user",
        content: [
          {
            type: "input_text",
            text: "Visual output for tool call call_1 (see the tool result immediately above).",
          },
          { type: "input_image", image_url: "data:image/jpeg;base64,aGVsbG8=" },
        ],
      },
    ]);
  });
});

describe("AgentSession with OpenAI provider (mock client)", () => {
  it("runs one tool-call turn then finishes, emitting the same event shapes", async () => {
    const calls: Array<Record<string, unknown>> = [];
    const executed: Array<{ name: string; input: unknown }> = [];
    const events: AgentEvent[] = [];

    const session = new AgentSession({
      provider: "openai",
      openaiClient: makeMockCompletions(calls) as never,
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

    // First request carries system prompt + tools in OpenAI format.
    expect(session.provider).toBe("openai");
    expect(session.model).toBe("gpt-4o");
    const req0 = calls[0]!;
    expect(req0["stream"]).toBe(true);
    const messages = req0["messages"] as Array<Record<string, unknown>>;
    expect(messages[0]).toEqual({ role: "system", content: session.systemPrompt });
    expect(messages[1]).toEqual({ role: "user", content: "trim clip1 to 5s" });
    const tools = req0["tools"] as Array<Record<string, unknown>>;
    expect(tools[0]!.type).toBe("function");
    expect((tools[0]!.function as Record<string, unknown>).name).toBe("edit");

    // Second request carries assistant tool_calls + role:'tool' result.
    const msgs1 = (calls[1]!["messages"] as Array<Record<string, unknown>>).slice(1);
    expect(msgs1).toHaveLength(3);
    expect(msgs1[1]!.role).toBe("assistant");
    expect(msgs1[2]).toMatchObject({ role: "tool", tool_call_id: "call_1" });
    expect(JSON.parse(String(msgs1[2]!.content))).toEqual({ clips: [] });

    // Same normalized events as the Anthropic loop.
    const types = events.map((e) => e.type);
    expect(types).toContain("text_delta");
    expect(types.filter((t) => t === "tool_call_started")).toHaveLength(1);
    expect(types.filter((t) => t === "tool_call_finished")).toHaveLength(1);
    expect(types).toContain("turn_started");
    expect(events.at(-1)!.type).toBe("done");

    // History stays in the canonical anthropic format.
    expect(session.messages).toHaveLength(4);
    expect(JSON.stringify(session.messages[1])).toContain('"tool_use"');
  });

  it("handles a non-streaming response fallback", async () => {
    const completions = {
      create: async () => ({
        choices: [
          {
            message: {
              content: "plain answer",
            },
            finish_reason: "stop",
          },
        ],
      }),
    };
    const provider = new OpenAIProvider(completions as never);
    let end: { stopReason: string; content: unknown[] } | undefined;
    for await (const chunk of provider.stream({
      model: "gpt-4o",
      system: "sys",
      tools: [],
      messages: [{ role: "user", content: "hi" }],
    })) {
      if (chunk.type === "message_end") end = chunk;
    }
    expect(end!.stopReason).toBe("end_turn");
    expect(end!.content).toEqual([{ type: "text", text: "plain answer" }]);
  });
});

describe("OpenAI Responses provider", () => {
  it("streams text and function calls with Luna reasoning settings", async () => {
    const calls: Array<Record<string, unknown>> = [];
    const responses = {
      create: async (params: Record<string, unknown>) => {
        calls.push(params);
        return streamOf([
          { type: "response.output_text.delta", delta: "Applying the edit." },
          {
            type: "response.output_item.added",
            item: { type: "function_call", id: "fc_1", call_id: "fc_1", name: "edit", arguments: "" },
          },
          {
            type: "response.function_call_arguments.delta",
            item_id: "fc_1",
            delta: '{"op":{"op":"trim"}}',
          },
          {
            type: "response.output_item.done",
            item: {
              type: "function_call",
              id: "fc_1",
              call_id: "fc_1",
              name: "edit",
              arguments: '{"op":{"op":"trim"}}',
            },
          },
          {
            type: "response.completed",
            response: {
              output: [
                {
                  type: "message",
                  content: [{ type: "output_text", text: "Applying the edit." }],
                },
                {
                  type: "function_call",
                  id: "fc_1",
                  call_id: "fc_1",
                  name: "edit",
                  arguments: '{"op":{"op":"trim"}}',
                },
              ],
            },
          },
        ]);
      },
    };
    const provider = new OpenAIResponsesProvider(responses as never, {
      reasoningEffort: "low",
    });
    let end: { stopReason: string; content: unknown[] } | undefined;
    const text: string[] = [];
    for await (const chunk of provider.stream({
      model: "gpt-5.6-luna",
      system: "sys",
      tools: [
        { name: "edit", description: "edit timeline", input_schema: { type: "object" } },
      ],
      messages: [{ role: "user", content: "trim" }],
    })) {
      if (chunk.type === "text_delta") text.push(chunk.text);
      if (chunk.type === "message_end") end = chunk;
    }
    expect(text.join("")).toBe("Applying the edit.");
    expect(end?.stopReason).toBe("tool_use");
    expect(end?.content).toEqual([
      { type: "text", text: "Applying the edit." },
      { type: "tool_use", id: "fc_1", name: "edit", input: { op: { op: "trim" } } },
    ]);
    expect(calls[0]).toMatchObject({
      model: "gpt-5.6-luna",
      instructions: "sys",
      reasoning: { effort: "low" },
      stream: true,
      tools: [{ type: "function", name: "edit" }],
    });
  });
});
