/**
 * @cut/media — probing utilities (ffprobe/ffmpeg wrappers).
 *
 * All functions spawn binaries with array args (no shell interpolation)
 * and parse their JSON/text output into normalized typed results.
 */
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import path from "node:path";

export interface ProbeInfo {
  duration?: number;
  width?: number;
  height?: number;
  fps?: number | undefined;
  /** Whether an audio stream exists. Undefined only for placeholder/legacy use. */
  hasAudio?: boolean;
  videoCodec?: string;
  audioCodec?: string;
}

export interface SilenceRange {
  start: number;
  /** End of the silence range. May be undefined when silence runs to EOF. */
  end?: number;
}

export interface SceneRange {
  start: number;
  end?: number;
}

export interface ExtractedFrame {
  time: number;
  path: string;
}

/** Spawn a binary, resolving on success; reject with captured stderr/stdout on failure. */
function run(
  bin: string,
  args: string[],
  opts: { captureStderr?: boolean } = {},
): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(bin, args, { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d: Buffer) => (stdout += d.toString()));
    child.stderr.on("data", (d: Buffer) => (stderr += d.toString()));
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0 || opts.captureStderr) {
        resolve({ code: code ?? -1, stdout, stderr });
      } else {
        reject(
          new Error(
            `${bin} exited with code ${code}\nargs: ${args.join(" ")}\nstderr: ${stderr.slice(-2000)}`,
          ),
        );
      }
    });
  });
}

// ---------------------------------------------------------------------------
// info()
// ---------------------------------------------------------------------------

interface FfprobeStream {
  codec_type?: string;
  codec_name?: string;
  width?: number;
  height?: number;
  avg_frame_rate?: string;
  r_frame_rate?: string;
}

interface FfprobeOutput {
  format?: { duration?: string };
  streams?: FfprobeStream[];
}

function parseRate(rate: string | undefined): number | undefined {
  if (!rate) return undefined;
  const [num, den] = rate.split("/");
  const n = Number(num);
  const d = Number(den);
  if (!Number.isFinite(n) || !Number.isFinite(d) || d === 0) return undefined;
  return n / d;
}

/**
 * Probe a media file with ffprobe.
 * Returns normalized duration/size/fps/codec info.
 */
export async function info(src: string): Promise<ProbeInfo> {
  const { stdout } = await run("ffprobe", [
    "-v",
    "quiet",
    "-print_format",
    "json",
    "-show_format",
    "-show_streams",
    src,
  ]);
  const parsed = JSON.parse(stdout) as FfprobeOutput;
  const streams = parsed.streams ?? [];
  const video = streams.find((s) => s.codec_type === "video");
  const audio = streams.find((s) => s.codec_type === "audio");

  const result: ProbeInfo = {
    hasAudio: audio !== undefined,
  };
  if (parsed.format?.duration !== undefined) {
    result.duration = Number(parsed.format.duration);
  }
  if (video?.width !== undefined) result.width = video.width;
  if (video?.height !== undefined) result.height = video.height;
  result.fps =
    parseRate(video?.avg_frame_rate) ?? parseRate(video?.r_frame_rate);
  if (video?.codec_name) result.videoCodec = video.codec_name;
  if (audio?.codec_name) result.audioCodec = audio.codec_name;
  return result;
}

// ---------------------------------------------------------------------------
// silenceDetect()
// ---------------------------------------------------------------------------

export interface SilenceDetectOptions {
  /** Loudness threshold in dB below which audio counts as silent. Default -35. */
  thresholdDb?: number;
  /** Minimum duration in seconds for a range to be reported. Default 0.5. */
  minDurationSec?: number;
}

/**
 * Detect silent ranges using ffmpeg's silencedetect filter.
 * Parses `silence_start` / `silence_end` lines from stderr.
 */
export async function silenceDetect(
  src: string,
  opts: SilenceDetectOptions = {},
): Promise<SilenceRange[]> {
  const thresholdDb = opts.thresholdDb ?? -35;
  const minDurationSec = opts.minDurationSec ?? 0.5;

  // Run to completion even though there is no output stream; ffmpeg exits
  // non-zero ("Output file is empty") unless we map a dummy output.
  const { stderr } = await run(
    "ffmpeg",
    [
      "-hide_banner",
      "-nostats",
      "-i",
      src,
      "-af",
      `silencedetect=noise=${thresholdDb}dB:d=${minDurationSec}`,
      "-f",
      "null",
      "-",
    ],
    { captureStderr: true },
  );
  return parseSilencedetect(stderr);
}

/**
 * Parse silencedetect stderr output into ranges.
 * Exported for unit testing; handles missing trailing silence_end (EOF).
 */
export function parseSilencedetect(stderr: string): SilenceRange[] {
  const ranges: SilenceRange[] = [];
  let openStart: number | undefined;
  for (const m of stderr.matchAll(
    /silence_(start|end):\s*(-?\d+(?:\.\d+)?)/g,
  )) {
    const t = Number(m[2]);
    if (m[1] === "start") {
      openStart = t;
    } else if (openStart !== undefined) {
      ranges.push({ start: openStart, end: t });
      openStart = undefined;
    }
  }
  if (openStart !== undefined) ranges.push({ start: openStart });
  return ranges;
}

// ---------------------------------------------------------------------------
// detectScenes()
// ---------------------------------------------------------------------------

export interface SceneDetectOptions {
  /**
   * Scene-change score threshold (0-1). Higher = fewer cuts detected.
   * Default 0.4.
   */
  threshold?: number;
}

/**
 * Detect scene changes using ffmpeg's select='gt(scene,T)' filter,
 * writing each cut's pts_time from stderr metadata lines.
 */
export async function detectScenes(
  src: string,
  opts: SceneDetectOptions = {},
): Promise<SceneRange[]> {
  const threshold = opts.threshold ?? 0.4;
  const { stderr } = await run(
    "ffmpeg",
    [
      "-hide_banner",
      "-nostats",
      "-i",
      src,
      "-vf",
      `select='gt(scene,${threshold})',metadata=print`,
      "-an",
      "-f",
      "null",
      "-",
    ],
    { captureStderr: true },
  );
  return parseSceneTimes(stderr);
}

/**
 * Parse scene-cut timestamps from `lavfi.scene_score` metadata print output.
 * Consecutive cut times become [{start: t[i], end: t[i+1]}]; the last range
 * has no end (runs to EOF).
 */
export function parseSceneTimes(stderr: string): SceneRange[] {
  const cuts: number[] = [];
  let pendingTime: number | undefined;
  for (const line of stderr.split("\n")) {
    const timeMatch = line.match(/pts_time:(-?\d+(?:\.\d+)?)/);
    if (timeMatch) {
      pendingTime = Number(timeMatch[1]);
      continue;
    }
    const scoreMatch = line.match(/lavfi\.scene_score=(-?\d+(?:\.\d+)?)/);
    if (scoreMatch && pendingTime !== undefined) {
      cuts.push(pendingTime);
      pendingTime = undefined;
    }
  }
  return cuts.map((t, i) => ({
    start: t,
    ...(i + 1 < cuts.length ? { end: cuts[i + 1]! } : {}),
  }));
}

// ---------------------------------------------------------------------------
// extractFrames()
// ---------------------------------------------------------------------------

/**
 * Extract single frames at the given timestamps (seconds) as PNGs into outDir.
 * Files are named frame_<index>.png in the order given.
 */
export async function extractFrames(
  src: string,
  times: number[],
  outDir: string,
): Promise<ExtractedFrame[]> {
  await mkdir(outDir, { recursive: true });
  const results: ExtractedFrame[] = [];
  for (let i = 0; i < times.length; i++) {
    const t = times[i]!;
    const outPath = path.join(outDir, `frame_${i}.png`);
    await run("ffmpeg", [
      "-hide_banner",
      "-loglevel",
      "error",
      "-ss",
      String(t),
      "-i",
      src,
      "-frames:v",
      "1",
      "-y",
      outPath,
    ]);
    results.push({ time: t, path: outPath });
  }
  return results;
}
