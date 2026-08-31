import { describe, expect, it } from "vitest";
import { AgentSession } from "../src/session.js";
import type { Skill } from "@cut/skills";

const skills: Skill[] = [
  { name: "shorts", description: "9:16 vertical recipes.", body: "ignored", dir: "/x/shorts" },
];

describe("AgentSession skills", () => {
  it("appends the skills index to the system prompt", () => {
    const session = new AgentSession({ client: {} as never, skills });
    expect(session.systemPrompt).toContain("Available skills:");
    expect(session.systemPrompt).toContain("- shorts: 9:16 vertical recipes.");
    expect(session.systemPrompt).toContain("skill_read");
    expect(session.systemPrompt).not.toContain("<dir>/SKILL.md");
  });

  it("leaves the base prompt untouched without skills", () => {
    const session = new AgentSession({ client: {} as never });
    expect(session.systemPrompt).not.toContain("Available skills:");
  });

  it("does not inject skill bodies (compact index only)", () => {
    const session = new AgentSession({ client: {} as never, skills });
    expect(session.systemPrompt).not.toContain("ignored");
  });
});
