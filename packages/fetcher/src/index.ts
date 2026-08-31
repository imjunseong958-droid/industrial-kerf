/**
 * @cut/fetcher. yt-dlp wrapper.
 *
 * Downloads videos (max height 1080, best audio) plus ko/en VTT subtitles.
 */
import { spawn } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { readdir } from "node:fs/promises";
import path from "node:path";

/** Quality policy format string: video ≤1080p + best audio. */
export const FORMAT_STRING = "bv*[height<=1080]+ba/b[height<=1080]";

export const OUTPUT_TEMPLATE = "%(title)s.%(ext)s";
export const SUB_LANGS = "ko.*,en.*";

/** Typed error thrown when yt-dlp is not installed on PATH. */
export class YtDlpMissingError extends Error {
  constructor(detail?: string) {
    super(
      `yt-dlp was not found on PATH.${detail ? ` (${detail})` : ""} Install it first: ` +
        `brew install yt-dlp OR pipx install yt-dlp`,
    );
    this.name = "YtDlpMissingError";
  }
}

/** Typed error thrown when the yt-dlp process fails or times out. */
export class YtDlpError extends Error {
  readonly exitCode: number | null;
  readonly stderrTail: string;
  constructor(message: string, exitCode: number | null, stderrTail: string) {
    super(message);
    this.name = "YtDlpError";
    this.exitCode = exitCode;
    this.stderrTail = stderrTail;
  }
}

export interface FetchOptions {
  /** Called with download progress percent (0-100) when parseable. */
  onProgress?: (percent: number) => void;
  /** Kill yt-dlp after this many milliseconds. Default: none. */
  timeoutMs?: number;
  /** Path to a Netscape-format cookie file passed as `--cookies` to yt-dlp. */
  cookies?: string;
  /** Extra args appended to the yt-dlp invocation. */
  extraArgs?: string[];
}

export interface FetchResult {
  path: string;
  subsPath?: string;
  title?: string;
}

/**
 * URL detector. True for YouTube-style URLs (youtube.com, youtu.be, /shorts/)
 * and for any other valid http(s) URL, since yt-dlp supports many sites.
 */
export function isYouTubeUrl(url: string): boolean {
  const trimmed = url.trim();
  if (trimmed.length === 0) return false;
  try {
    const u = new URL(trimmed);
    if (u.protocol !== "http:" && u.protocol !== "https:") return false;
    const host = u.hostname.replace(/^www\./, "").toLowerCase();
    return [
      "youtube.com",
      "m.youtube.com",
      "music.youtube.com",
      "youtube-nocookie.com",
      "youtu.be",
    ].includes(host)
      ? true
      : true; // any http(s) URL: yt-dlp supports many extractors
  } catch {
    // Not an absolute URL; still accept non-empty input (IDs, search terms).
    return true;
  }
}

/** Resolve `yt-dlp --version`; throws YtDlpMissingError when unavailable. */
export async function checkYtDlp(bin = "yt-dlp"): Promise<string> {
  try {
    const { stdout } = await run(bin, ["--version"], { timeoutMs: 10_000 });
    return stdout.trim();
  } catch (err) {
    throw new YtDlpMissingError(err instanceof Error ? err.message : String(err));
  }
}

const PERCENT_RE = /\[download\]\s+(\d+(?:\.\d+)?)%/;

/** Extract progress percent from a yt-dlp output line/chunk, or null. */
export function parseProgressPercent(text: string): number | null {
  const m = PERCENT_RE.exec(text);
  if (!m) return null;
  const pct = Number.parseFloat(m[1] ?? "");
  return Number.isFinite(pct) ? pct : null;
}

interface RunResult {
  stdout: string;
  stderr: string;
  code: number | null;
  signal: NodeJS.Signals | null;
  timedOut: boolean;
}

function run(
  bin: string,
  args: string[],
  opts: { timeoutMs?: number | undefined; cwd?: string | undefined; onData?: ((text: string) => void) | undefined },
): Promise<RunResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(bin, args, opts.cwd ? { cwd: opts.cwd } : {});
    let stdout = "";
    let stderr = "";
    let stdoutBuf = "";
    let timedOut = false;
    let timer: NodeJS.Timeout | undefined;

    child.on("error", (err) => {
      if (timer) clearTimeout(timer);
      reject(Object.assign(err, { code: (err as NodeJS.ErrnoException).code }));
    });

    if (opts.timeoutMs != null && opts.timeoutMs > 0) {
      timer = setTimeout(() => {
        timedOut = true;
        child.kill("SIGKILL");
      }, opts.timeoutMs);
      timer.unref?.();
    }

    child.stdout?.setEncoding("utf8");
    child.stderr?.setEncoding("utf8");

    child.stdout?.on("data", (chunk: string) => {
      stdout += chunk;
      if (!opts.onData) return;
      stdoutBuf += chunk;
      const lines = stdoutBuf.split(/\r\n|\r|\n/);
      stdoutBuf = lines.pop() ?? "";
      for (const line of lines) opts.onData(line);
    });
    child.stderr?.on("data", (chunk: string) => {
      stderr += chunk;
      if (opts.onData) opts.onData(chunk);
    });

    child.on("close", (code, signal) => {
      if (timer) clearTimeout(timer);
      resolve({ stdout, stderr, code, signal, timedOut });
    });
  });
}

/**
 * Pure helper building the yt-dlp argv. Exported for unit tests.
 * When `cookies` is set, `--cookies <path>` is inserted before extraArgs.
 */
export function buildYtDlpArgs(url: string, opts: FetchOptions = {}): string[] {
  return [
    "--format", FORMAT_STRING,
    "--write-auto-subs",
    "--write-subs",
    "--sub-langs", SUB_LANGS,
    "--convert-subs", "vtt",
    "--no-playlist",
    "--newline",
    "--output", OUTPUT_TEMPLATE,
    ...(opts.cookies ? ["--cookies", opts.cookies] : []),
    ...(opts.extraArgs ?? []),
    url,
  ];
}

/**
 * Download `url` into `outDir` at ≤1080p + best audio, with ko/en VTT subs.
 */
export async function fetchVideo(url: string, outDir: string, opts: FetchOptions = {}): Promise<FetchResult> {
  await checkYtDlp();

  const args = buildYtDlpArgs(url, opts);

  const res = await run("yt-dlp", args, {
    timeoutMs: opts.timeoutMs,
    cwd: outDir,
    onData: (text) => {
      if (!opts.onProgress) return;
      const pct = parseProgressPercent(text);
      if (pct != null) opts.onProgress(pct);
    },
  });

  if (res.timedOut || res.signal === "SIGKILL") {
    throw new YtDlpError(`yt-dlp timed out`, res.code, tail(res.stderr));
  }
  if (res.code !== 0) {
    throw new YtDlpError(`yt-dlp exited with code ${res.code}`, res.code, tail(res.stderr));
  }

  const title = parseTitle(res.stdout);

  // Find the newest video file produced in outDir.
  const files = await readdir(outDir);
  const videoExts = [".mkv", ".mp4", ".webm"];
  let videoPath: string | undefined;
  for (const f of files) {
    const ext = path.extname(f).toLowerCase();
    if (!videoExts.includes(ext)) continue;
    if (title != null && f.startsWith(title + ".")) {
      videoPath = f;
      break;
    }
  }
  if (!videoPath) {
    // Fall back to most recently created video-looking file.
    let bestMtime = -1;
    for (const f of files) {
      const ext = path.extname(f).toLowerCase();
      if (!videoExts.includes(ext)) continue;
      const st = await import("node:fs/promises").then((m) => m.stat(path.join(outDir, f)));
      if (st.mtimeMs > bestMtime) {
        bestMtime = st.mtimeMs;
        videoPath = f;
      }
    }
  }
  if (!videoPath) {
    throw new YtDlpError("yt-dlp finished but no video file found in outDir", res.code, tail(res.stderr));
  }

  const base = stripExt(videoPath);
  const relSubs = resolveSubsPath("", base, files);

  return {
    path: path.join(outDir, videoPath),
    ...(relSubs ? { subsPath: path.join(outDir, relSubs) } : {}),
    ...(title ? { title } : {}),
  };
}

function tail(s: string, lines = 10): string {
  return s
    .trim()
    .split(/\r?\n|\r/)
    .filter(Boolean)
    .slice(-lines)
    .join("\n");
}

function parseTitle(stdout: string): string | undefined {
  const merger = [...stdout.matchAll(/\[Merger\]\s+Merging formats into "(.+)"/g)].at(-1)?.[1];
  if (merger) return stripExt(merger.trim());
  const dest = [...stdout.matchAll(/\[download\] Destination:\s*(.+)/g)].at(-1)?.[1];
  if (dest) return stripExt(dest.trim());
  const existing = [...stdout.matchAll(/\[download\]\s+(.+?) has already been downloaded/g)].at(-1)?.[1];
  if (existing) return stripExt(existing.trim());
  return undefined;
}

function stripExt(f: string): string {
  const ext = path.extname(f);
  return ext ? f.slice(0, -ext.length) : f;
}

/**
 * Resolve subtitle file for a downloaded title. Prefers `.vtt` over `.srt`,
 * then ko before en before any other lang. Pure helper exported for tests.
 * When `dir` is empty, returns the relative filename.
 */
export function resolveSubsPath(dir: string, base: string, files: readonly string[]): string | undefined {
  const vtt = pickSub(files, base, ".vtt") ?? pickSub(files, base, ".srt");
  return vtt == null ? undefined : path.join(dir, vtt);
}

function pickSub(files: readonly string[], base: string, ext: ".vtt" | ".srt"): string | undefined {
  const prefix = `${base}.`;
  const candidates = files.filter((f) => f.startsWith(prefix) && f.endsWith(ext));
  if (candidates.length === 0) return undefined;
  const rank = (f: string): number => {
    const lang = f.slice(prefix.length, -ext.length);
    if (/^ko(\.|$)/i.test(lang)) return 0;
    if (/^en(\.|$)/i.test(lang)) return 1;
    return 2;
  };
  candidates.sort((a, b) => rank(a) - rank(b));
  return candidates[0];
}
