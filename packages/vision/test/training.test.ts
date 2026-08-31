import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const script = fileURLToPath(new URL("../python/train_yolo26s.py", import.meta.url));

describe("YOLO26 training layout", () => {
  it("exposes a dependency-free argparse smoke path without training", () => {
    const result = spawnSync("python3", [script, "--help"], { encoding: "utf8" });
    expect(result.status).toBe(0);
    expect(result.stdout).toContain("--offline");
    expect(result.stdout).toContain("--validate-only");
    expect(result.stdout).toContain("--output");
  });
});
