/**
 * Filtergraph compiler. TypeScript wrapper that calls the native cut-engine binary.
 */
import type { SubtitlesOverlay, Timeline } from "@cut/timeline";
import { CutEngineError, invokeCutEngineJson } from "@cut/engine";

export interface CaptionCue {
  start: number;
  end: number;
  text: string;
}

export interface CaptionStyle {
  fontSize?: number;
  fontColor?: string;
  outlineColor?: string;
  position?: "bottom" | "center" | "top" | string;
  marginV?: number;
  /** Supported when supplied by a newer timeline schema. */
  outlineWidth?: number;
  backgroundColor?: string;
  fontFamily?: string;
}

export interface CaptionsOverlayLike {
  id: string;
  type: "captions";
  cues: CaptionCue[];
  style?: CaptionStyle;
}

export interface CompiledFilterGraph {
  /** Full ffmpeg argv array (spawn-safe, no shell). */
  argv: string[];
  /** Human-readable rendering of the same command. */
  command: string;
  /** The filter_complex string itself. */
  filterComplex: string;
}

interface CompileV1Result extends CompiledFilterGraph {
  duration: number;
}

function compileV1Args(opts: { videoPostFilters?: string[] } = {}): string[] {
  const args = ["compile-v1"];
  for (const filter of opts.videoPostFilters ?? []) {
    args.push("--post-filter", filter);
  }
  return args;
}

function invokeCompileV1(
  timeline: Timeline,
  opts: { videoPostFilters?: string[] } = {},
): CompileV1Result {
  try {
    return invokeCutEngineJson<CompileV1Result>(compileV1Args(opts), {
      input: JSON.stringify(timeline),
    });
  } catch (error) {
    if (error instanceof CutEngineError) throw new Error(error.message);
    throw error;
  }
}

/** Escape a file path for use inside an ffmpeg filter argument. */
export function escapeFilterPath(p: string): string {
  return invokeCutEngineJson<string>(["overlay", "escape-path", "--path", p]);
}

/** Build the `subtitles=` filter string for one overlay entry. */
export function subtitlesFilter(o: SubtitlesOverlay): string {
  return invokeCutEngineJson<string>(["overlay", "subtitles"], {
    input: JSON.stringify({ path: o.path, style: o.style }),
  });
}

/** Escape text for drawtext's filter value. */
export function escapeDrawtextText(text: string): string {
  return invokeCutEngineJson<string>(["overlay", "escape-drawtext", "--text", text]);
}

/** Build one timed drawtext filter for a caption cue. */
export function captionsFilter(cue: CaptionCue, style?: CaptionStyle): string {
  return invokeCutEngineJson<string>(["overlay", "captions"], {
    input: JSON.stringify({ ...cue, style }),
  });
}

/** FFmpeg atempo filter chain for a clip speed multiplier. */
export function atempoFilters(speed: number): string[] {
  return invokeCutEngineJson<string[]>(["overlay", "atempo", "--speed", String(speed)]);
}

/** Total output duration of a timeline in seconds. */
export function timelineDuration(timeline: Timeline): number {
  try {
    return invokeCutEngineJson<{ duration: number }>(["duration-v1"], {
      input: JSON.stringify(timeline),
    }).duration;
  } catch (error) {
    if (error instanceof CutEngineError) throw new Error(error.message);
    throw error;
  }
}

export function compileFilterGraph(
  timeline: Timeline,
  opts: { videoPostFilters?: string[] } = {},
): CompiledFilterGraph {
  const { argv, command, filterComplex } = invokeCompileV1(timeline, opts);
  return { argv, command, filterComplex };
}
