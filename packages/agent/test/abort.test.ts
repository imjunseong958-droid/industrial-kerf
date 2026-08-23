import { describe, it, expect, vi } from "vitest";
import { AgentSession } from "../src/session.js";
import type { AgentEvent } from "../src/session.js";

const toolUseResponse = {
  stop_reason: "tool_use",
  content: [{ type: "tool_use", id: "t1", name: "noop", input: {} }],
};

describe("AgentSession.abort", () => {
  it("stops between turns once abort() is called mid-run", async () => {
    let calls = 0;
    const session = new AgentSession({
      client: {
        create: vi.fn(async () => {
          calls += 1;
          return calls === 1 ? toolUseResponse : { stop_reason: "end_turn", content: [] };
        }),
      } as never,
      tools: {
        noop: { description: "no-op", inputSchema: {}, execute: vi.fn(async () => ({})) },
      } as never,
    });
    const events: AgentEvent[] = [];
    session.onEvent((e) => events.push(e));
    // Cancel as soon as the first tool call starts.
    session.onEvent((e) => {
      if (e.type === "tool_call_started") session.abort();
    });

    const text = await session.run("hi");
    expect(calls).toBe(1); // never went back to the API
    expect(text).toBe("");
    expect(events.map((e) => e.type)).toContain("aborted");
    expect(events[events.length - 1]!.type).toBe("done");
  });

  it("resets the flag at the start of each run()", async () => {
    const client = { create: vi.fn(async () => ({ stop_reason: "end_turn", content: [{ type: "text", text: "ok" }] })) };
    const session = new AgentSession({ client: client as never });
    session.abort();
    expect(session.isAborted).toBe(true);
    const text = await session.run("hello");
    expect(text).toBe("ok");
    expect(session.isAborted).toBe(false);
    expect(client.create).toHaveBeenCalledTimes(1);
  });
});
