import { describe, expect, it } from "vitest";
import {
  CutEngineError,
  CUT_ENGINE_REPO_ROOT,
  invokeCutEngineJson,
  resolveCutEngineBin,
  spawnCutEngineJson,
} from "../src/index.js";
import path from "node:path";

describe("cut-engine host", () => {
  it("resolves the debug profile without attempting a build", () => {
    expect(resolveCutEngineBin({ profile: "debug" })).toBe(
      path.join(CUT_ENGINE_REPO_ROOT, "target", "debug", process.platform === "win32" ? "cut-engine.exe" : "cut-engine"),
    );
  });

  it("reports a missing explicit binary with a build instruction", () => {
    let thrown: unknown;
    try {
      resolveCutEngineBin({ binary: path.join(CUT_ENGINE_REPO_ROOT, "target", "missing", "cut-engine") });
    } catch (error) {
      thrown = error;
    }
    expect(thrown).toBeInstanceOf(CutEngineError);
    expect(thrown).toMatchObject({ code: "BINARY_NOT_FOUND" });
    expect(String((thrown as Error).message)).toContain("does not build the Rust engine automatically");
  });

  it("uses one JSON stdout and stderr-progress contract for async commands", async () => {
    const ratios: number[] = [];
    const lines: string[] = [];
    const value = await spawnCutEngineJson<{ ok: boolean }>(
      [
        "-e",
        "process.stderr.write('{\"progress\":-1}\\nnoise\\n{\"progress\":0.25}\\n{\"progress\":2}\\n'); process.stdout.write('{\"ok\":true}');",
      ],
      {
        binary: process.execPath,
        onProgress: (ratio) => ratios.push(ratio),
        onStderrLine: (line) => lines.push(line),
      },
    );
    expect(value).toEqual({ ok: true });
    expect(ratios).toEqual([0, 0.25, 1, 1]);
    expect(lines).toEqual(["{\"progress\":-1}", "noise", "{\"progress\":0.25}", "{\"progress\":2}"]);
  });

  it("parses encoder listings in the native binary", () => {
    const listing = [
      " V....D h264_videotoolbox VideoToolbox H.264 Encoder",
      " V....D hevc_qsv Intel Quick Sync Video",
    ].join("\n");
    const rust = invokeCutEngineJson<{
      encoders: Array<{ encoder: string; available: boolean; family: string }>;
    }>(["encoders", "--text", listing]);
    expect(rust.encoders.find((entry) => entry.encoder === "h264_videotoolbox")?.available).toBe(true);
    expect(rust.encoders.find((entry) => entry.encoder === "hevc_qsv")?.available).toBe(true);
    const selected = invokeCutEngineJson<{ encoder: string; hardware: boolean }>(
      ["select-encoder", "--prefer-hardware"],
      { input: JSON.stringify(rust) },
    );
    expect(selected).toMatchObject({ encoder: "h264_videotoolbox", hardware: true });
  });

  it("reports a laptop resource budget", () => {
    const budget = invokeCutEngineJson<{
      logicalCpus: number;
      ffmpegThreads: number;
      onnxIntraThreads: number;
      trackingFps: number;
      softwarePreset: string;
    }>(["resources"]);
    expect(budget.logicalCpus).toBeGreaterThan(0);
    expect(budget.ffmpegThreads).toBeGreaterThan(0);
    expect(budget.onnxIntraThreads).toBeGreaterThan(0);
    expect(budget.trackingFps).toBeGreaterThan(0);
    expect(["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow"]).toContain(
      budget.softwarePreset,
    );
  });
});
