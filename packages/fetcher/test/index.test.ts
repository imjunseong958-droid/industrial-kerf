import { describe, it, expect, vi } from "vitest";
import {
  FORMAT_STRING,
  OUTPUT_TEMPLATE,
  SUB_LANGS,
  YtDlpMissingError,
  isYouTubeUrl,
  parseProgressPercent,
  resolveSubsPath,
  fetchVideo,
  checkYtDlp,
} from "../src/index.js";
import { mkdtempSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";

describe("format string building", () => {
  it("caps video at 1080p and never above", () => {
    expect(FORMAT_STRING).toBe("bv*[height<=1080]+ba/b[height<=1080]");
    expect(FORMAT_STRING).not.toMatch(/height<=(?!1080)\d+/);
    expect(FORMAT_STRING).toMatch(/\bbv\*/);
    expect(FORMAT_STRING).toMatch(/\+ba\b/);
  });

  it("uses the documented output template and sub langs", () => {
    expect(OUTPUT_TEMPLATE).toBe("%(title)s.%(ext)s");
    expect(SUB_LANGS).toContain("ko.*");
    expect(SUB_LANGS).toContain("en.*");
  });
});

describe("isYouTubeUrl", () => {
  it.each([
    "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
    "https://youtu.be/dQw4w9WgXcQ",
    "https://www.youtube.com/shorts/abcdefghijk",
    "https://m.youtube.com/watch?v=x",
  ])("accepts YouTube URL %s", (url) => {
    expect(isYouTubeUrl(url)).toBe(true);
  });

  it("accepts any other http(s) URL, yt-dlp supports many sites", () => {
    expect(isYouTubeUrl("https://vimeo.com/76979871")).toBe(true);
  });

  it("rejects empty strings", () => {
    expect(isYouTubeUrl("")).toBe(false);
    expect(isYouTubeUrl("   ")).toBe(false);
  });
});

describe("parseProgressPercent", () => {
  it("parses yt-dlp progress lines", () => {
    expect(parseProgressPercent("[download]  42.3% of   10.00MiB")).toBe(42.3);
    expect(parseProgressPercent("[download] 100% of   10.00MiB")).toBe(100);
    expect(parseProgressPercent("[download] Destination: x.mp4")).toBeNull();
    expect(parseProgressPercent("garbage")).toBeNull();
  });
});

describe("resolveSubsPath", () => {
  const dir = "/out";

  it("prefers ko vtt over en vtt", () => {
    const got = resolveSubsPath(dir, "video", ["video.en.vtt", "video.ko.vtt"]);
    expect(got).toBe(path.join(dir, "video.ko.vtt"));
  });

  it("falls back to en when no ko", () => {
    expect(resolveSubsPath(dir, "video", ["video.en.vtt"])).toBe(path.join(dir, "video.en.vtt"));
  });

  it("matches lang tags like ko-KR / en-US", () => {
    expect(resolveSubsPath(dir, "v", ["v.ko-KR.vtt"])).toBe(path.join(dir, "v.ko-KR.vtt"));
    expect(resolveSubsPath(dir, "v", ["v.en-US.vtt"])).toBe(path.join(dir, "v.en-US.vtt"));
  });

  it("falls back to srt when no vtt", () => {
    expect(resolveSubsPath(dir, "v", ["v.en.srt"])).toBe(path.join(dir, "v.en.srt"));
  });

  it("returns undefined when nothing matches", () => {
    expect(resolveSubsPath(dir, "v", ["other.ko.vtt", "v.mp4"])).toBeUndefined();
  });

  it("resolves against a real temp dir fixture", () => {
    const tmp = mkdtempSync(path.join(tmpdir(), "fetcher-test-"));
    writeFileSync(path.join(tmp, "My Video.ko.vtt"), "WEBVTT");
    writeFileSync(path.join(tmp, "My Video.mp4"), "");
    expect(resolveSubsPath(tmp, "My Video", ["My Video.ko.vtt", "My Video.mp4"])).toBe(
      path.join(tmp, "My Video.ko.vtt"),
    );
  });
});

describe("fetchVideo without yt-dlp on PATH", () => {
  it("throws a typed error with install instructions", async () => {
    // Point PATH lookups at an empty dir by using checkYtDlp's error path via
    // fetchVideo with a nonexistent bin through extraArgs won't work; instead
    // simulate the missing-binary path directly.
    const orig = process.env.PATH;
    try {
      process.env.PATH = "";
      await expect(checkYtDlp()).rejects.toBeInstanceOf(YtDlpMissingError);
      await expect(checkYtDlp()).rejects.toThrow(/brew install yt-dlp/);
      await expect(fetchVideo("https://example.com/v", tmpdir())).rejects.toBeInstanceOf(YtDlpMissingError);
    } finally {
      process.env.PATH = orig;
    }
  }, 20_000);
});

describe("integration (skipped unless yt-dlp + network)", () => {
  const hasYtDlp = (() => {
    try {
      execFileSync("yt-dlp", ["--version"], { stdio: "pipe" });
      return true;
    } catch {
      return false;
    }
  })();

  it.skipIf(!hasYtDlp || !process.env.CUT_ALLOW_NETWORK)("downloads a small public clip", async () => {
    const outDir = mkdtempSync(path.join(tmpdir(), "fetcher-int-"));
    const onProgress = vi.fn();
    const result = await fetchVideo("https://upload.wikimedia.org/wikipedia/commons/video?skip=1", outDir, {
      timeoutMs: 120_000,
      onProgress,
    });
    expect(result.path).toBeTruthy();
    expect(onProgress).toHaveBeenCalled();
  }, 150_000);
});
