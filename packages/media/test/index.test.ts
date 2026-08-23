import { describe, it, expect } from "vitest";
import type { ProbeInfo } from "../src/index.js";

describe("media", () => {
  it("placeholder test", () => {
    const p: ProbeInfo = {};
    expect(p).toBeDefined();
  });
});
