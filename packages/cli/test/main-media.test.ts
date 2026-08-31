import { describe, expect, it } from "vitest";
import { sourcePathKey } from "../src/main.js";

describe("local media initialization", () => {
  it("compares relative and absolute source paths without changing stored paths", () => {
    expect(sourcePathKey("/work/project", "clips/video.mp4")).toBe(
      sourcePathKey("/work/project", "/work/project/clips/video.mp4"),
    );
  });
});
