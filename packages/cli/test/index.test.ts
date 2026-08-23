import { describe, it, expect } from "vitest";
import { NAME } from "../src/index.js";

describe("cli", () => {
  it("is named cut", () => {
    expect(NAME).toBe("cut");
  });
});
