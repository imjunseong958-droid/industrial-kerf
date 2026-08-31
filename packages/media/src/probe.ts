/**
 * @cut/media. Probing utilities via the cut-engine host.
 */
import { invokeCutEngineJson } from "@cut/engine";

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

/**
 * Probe a media file with ffprobe.
 * Returns normalized duration/size/fps/codec info.
 */
export async function info(src: string): Promise<ProbeInfo> {
  return invokeCutEngineJson<ProbeInfo>(["probe", "info", src]);
}

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
  return invokeCutEngineJson<SilenceRange[]>([
    "probe",
    "silence",
    src,
    `--threshold-db=${thresholdDb}`,
    `--min-duration=${minDurationSec}`,
  ]);
}

/**
 * Parse silencedetect stderr output into ranges.
 * Exported for unit testing; handles missing trailing silence_end (EOF).
 */
export function parseSilencedetect(stderr: string): SilenceRange[] {
  return invokeCutEngineJson<SilenceRange[]>(["probe", "silence", "_", "--parse", stderr]);
}

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
  return invokeCutEngineJson<SceneRange[]>([
    "probe",
    "scenes",
    src,
    "--threshold",
    String(threshold),
  ]);
}

/**
 * Parse scene-cut timestamps from `lavfi.scene_score` metadata print output.
 * Consecutive cut times become [{start: t[i], end: t[i+1]}]; the last range
 * has no end (runs to EOF).
 */
export function parseSceneTimes(stderr: string): SceneRange[] {
  return invokeCutEngineJson<SceneRange[]>(["probe", "scenes", "_", "--parse", stderr]);
}

/**
 * Extract single frames at the given timestamps (seconds) as PNGs into outDir.
 * Files are named frame_<index>.png in the order given. By default wide
 * frames are downscaled to at most 1280px wide (never upscaled), keeping the
 * PNG format so visual callers do not receive unexpectedly huge 4K images.
 */
export interface ExtractFramesOptions {
  /** Maximum output width. Defaults to 1280; smaller sources are not enlarged. */
  maxWidth?: number;
}

export async function extractFrames(
  src: string,
  times: number[],
  outDir: string,
  opts: ExtractFramesOptions = {},
): Promise<ExtractedFrame[]> {
  const maxWidth = opts.maxWidth ?? 1280;
  if (!Number.isFinite(maxWidth) || !(maxWidth > 0)) {
    throw new Error(`extractFrames maxWidth must be finite and positive, got ${maxWidth}`);
  }
  const width = Math.max(1, Math.floor(maxWidth));
  return invokeCutEngineJson<ExtractedFrame[]>([
    "probe",
    "frames",
    src,
    "--times",
    times.join(","),
    "--out-dir",
    outDir,
    "--max-width",
    String(width),
  ]);
}
