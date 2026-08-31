import { describe, expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtemp, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {
  ProxyCache,
  createProxyCacheKey,
  proxyFfmpegArgs,
  type ProxyRunner,
} from "../src/proxy.js";

let ffmpegOnPath = false;
try {
  execFileSync("ffmpeg", ["-version"], { stdio: "ignore" });
  ffmpegOnPath = true;
} catch {
  /* integration test is skipped when ffmpeg is unavailable */
}

describe("ProxyCache", () => {
  it("normalizes requested dimensions to positive even values", () => {
    const args = proxyFfmpegArgs("input.mp4", "output.mp4", {
      width: 321,
      height: 241,
    });
    expect(args).toContain("scale=320:240");
    expect(() => proxyFfmpegArgs("input.mp4", "output.mp4", { width: Number.NaN, height: 240 })).toThrow(/width.*finite positive/);
  });

  it("uses deterministic stat/settings keys and atomically promotes injected output", async () => {
    const projectDir = await mkdtemp(path.join(os.tmpdir(), "cut-proxy-"));
    try {
      const source = path.join(projectDir, "source.mov");
      await writeFile(source, "source");
      const runner: ProxyRunner = async ({ output }, context) => {
        context.report({ ratio: 0.5, phase: "render" });
        await writeFile(output, "proxy");
      };
      const cache = new ProxyCache({ projectDir, runner });
      const settingsA = { width: 320, format: "mp4", nested: { b: 2, a: 1 } };
      const settingsB = { nested: { a: 1, b: 2 }, format: "mp4", width: 320 };
      const first = await cache.getOrCreate(source, settingsA);
      const second = await cache.getOrCreate(source, settingsB);
      expect(first.path).toBe(second.path);
      expect(first.fromCache).toBe(false);
      expect(second.fromCache).toBe(true);
      expect(await readFile(first.path, "utf8")).toBe("proxy");
      const files = await readdir(cache.cacheDir);
      expect(files).toEqual([path.basename(first.path)]);
    } finally {
      await rm(projectDir, { recursive: true, force: true });
    }
  });

  it("changes the key when source stat changes", async () => {
    const sourceStat = { size: 3, mtimeMs: 10 };
    const a = createProxyCacheKey("/tmp/source.mov", sourceStat, { width: 320 });
    const b = createProxyCacheKey("/tmp/source.mov", { ...sourceStat, size: 4 }, { width: 320 });
    const c = createProxyCacheKey("/tmp/source.mov", sourceStat, { width: 640 });
    expect(a).not.toBe(b);
    expect(a).not.toBe(c);
  });

  it("propagates cancellation and leaves no partial cache entry", async () => {
    const projectDir = await mkdtemp(path.join(os.tmpdir(), "cut-proxy-cancel-"));
    try {
      const source = path.join(projectDir, "source.mov");
      await writeFile(source, "source");
      let observedAbort = false;
      let started = false;
      const runner: ProxyRunner = async ({ output }, context) => {
        started = true;
        await new Promise<void>((resolve) => {
          if (context.signal.aborted) {
            observedAbort = true;
            resolve();
            return;
          }
          context.signal.addEventListener("abort", () => {
            observedAbort = true;
            resolve();
          }, { once: true });
        });
        await writeFile(output, "partial");
      };
      const cache = new ProxyCache({ projectDir, runner });
      const controller = new AbortController();
      const pending = cache.getOrCreate(source, { width: 160 }, { signal: controller.signal });
      const deadline = Date.now() + 2_000;
      while (!started && Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
      controller.abort();
      await expect(pending).rejects.toMatchObject({ code: "ABORT_ERR" });
      expect(observedAbort).toBe(true);
      await new Promise((resolve) => setTimeout(resolve, 10));
      expect(await readdir(cache.cacheDir)).toEqual([]);
    } finally {
      await rm(projectDir, { recursive: true, force: true });
    }
  });

  it.skipIf(!ffmpegOnPath)("renders the default proxy temp file with an inferable muxer", { timeout: 120_000 }, async () => {
    const projectDir = await mkdtemp(path.join(os.tmpdir(), "cut-proxy-ffmpeg-"));
    try {
      const source = path.join(projectDir, "source.mp4");
      execFileSync("ffmpeg", [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=0.5:size=64x48:rate=10",
        "-an",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        source,
      ]);
      const cache = new ProxyCache({ projectDir });
      const artifact = await cache.getOrCreate(source, { format: "mp4", width: 64, height: 48 });
      expect(path.extname(artifact.path)).toBe(".mp4");
      expect((await stat(artifact.path)).size).toBeGreaterThan(0);
    } finally {
      await rm(projectDir, { recursive: true, force: true });
    }
  });
});
