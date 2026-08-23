import { describe, it, expect } from "vitest";
import type { FetchResult } from "../src/index.js";

describe("fetcher", () => {
  it("placeholder test", () => {
    const f: FetchResult = { path: "" };
    expect(f.path).toBe("");
  });
});
