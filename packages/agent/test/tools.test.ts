import { describe, it, expect } from "vitest";
import { TOOL_NAMES } from "../src/index.js";

describe("agent", () => {
  it("exposes the 5 tool contracts", () => {
    expect([...TOOL_NAMES]).toEqual(["fetch", "probe", "edit", "render", "bash"]);
  });
});
