/**
 * Hardware encoder capability discovery and explicit selection.
 *
 * Detection is advisory: callers must opt into a selected
 * hardware encoder.  `selectVideoEncoder()` defaults to libx264, preserving
 * existing output characteristics when a host starts using this module.
 */
import { invokeCutEngineJson } from "@cut/engine";

export type HardwareEncoderFamily = "videotoolbox" | "nvenc" | "qsv" | "amf";
export type SoftwareEncoder = "libx264" | "libx265" | "mpeg4" | string;

export interface EncoderCapability {
  family: HardwareEncoderFamily;
  /** The codec name passed to ffmpeg's `-c:v`. */
  encoder: string;
  /** True only when the encoder was found in ffmpeg's encoder list. */
  available: boolean;
  /** Why it was unavailable, when detection could explain it. */
  reason?: string;
}

export interface EncoderCapabilities {
  ffmpegBin: string;
  encoders: EncoderCapability[];
  /** Raw output is useful to MCP hosts for diagnostics, but optional. */
  raw?: string;
}

export interface EncoderCommandResult {
  stdout?: string;
  stderr?: string;
  code?: number;
}

export type EncoderCommandRunner = (
  bin: string,
  args: string[],
) => EncoderCommandResult | string | Promise<EncoderCommandResult | string>;

export interface DetectEncoderOptions {
  ffmpegBin?: string;
  /** Inject command execution for deterministic tests/no-ffmpeg hosts. */
  runner?: EncoderCommandRunner;
}

export interface SelectEncoderOptions {
  /** `false` (the default) keeps software encoding unchanged. */
  preferHardware?: boolean;
  /** Preference order; defaults to VideoToolbox, NVENC, QSV, then AMF. */
  preference?: HardwareEncoderFamily[];
  /** h264 selects h264_* candidates; hevc selects hevc_* candidates. */
  codec?: "h264" | "hevc";
  softwareEncoder?: SoftwareEncoder;
}

export interface EncoderSelection {
  encoder: string;
  hardware: boolean;
  family?: HardwareEncoderFamily;
  /** Human-readable decision detail for UI/logging. */
  reason: string;
}

function normalizeCommandResult(value: EncoderCommandResult | string): { output: string; failed: boolean } {
  if (typeof value === "string") return { output: value, failed: false };
  return {
    output: `${value.stdout ?? ""}\n${value.stderr ?? ""}`,
    failed: value.code !== undefined && value.code !== 0,
  };
}

/** Parse a captured `ffmpeg -encoders` listing without running a process. */
export function parseEncoderCapabilities(
  output: string,
  ffmpegBin = "ffmpeg",
): EncoderCapabilities {
  const capabilities = invokeCutEngineJson<EncoderCapabilities>(["encoders", "--text", output]);
  if (ffmpegBin !== "ffmpeg") {
    capabilities.ffmpegBin = ffmpegBin;
  }
  return capabilities;
}

/** Detect VideoToolbox, NVENC, QSV, and AMF encoders from ffmpeg's encoder list. */
export async function detectEncoderCapabilities(
  options: DetectEncoderOptions = {},
): Promise<EncoderCapabilities> {
  if (!options.runner) {
    return invokeCutEngineJson<EncoderCapabilities>(["encoders", "--detect"]);
  }
  const ffmpegBin = options.ffmpegBin ?? "ffmpeg";
  let result: EncoderCommandResult | string;
  try {
    result = await options.runner(ffmpegBin, ["-hide_banner", "-encoders"]);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const capabilities = parseEncoderCapabilities("", ffmpegBin);
    capabilities.raw = message;
    capabilities.encoders = capabilities.encoders.map((entry) => ({
      ...entry,
      available: false,
      reason: `ffmpeg detection failed: ${message}`,
    }));
    return capabilities;
  }
  const normalized = normalizeCommandResult(result);
  const capabilities = parseEncoderCapabilities(normalized.output, ffmpegBin);
  if (normalized.failed) {
    capabilities.encoders = capabilities.encoders.map((entry) => ({
      ...entry,
      available: false,
      reason: "ffmpeg encoder listing failed",
    }));
  }
  return capabilities;
}

/**
 * Select an encoder explicitly. Hardware is considered only when
 * `preferHardware` is true, so merely detecting a GPU never changes output.
 */
export function selectVideoEncoder(
  capabilities: EncoderCapabilities | readonly EncoderCapability[],
  options: SelectEncoderOptions = {},
): EncoderSelection {
  const args = ["select-encoder"];
  if (options.preferHardware === true) args.push("--prefer-hardware");
  if (options.codec === "hevc") args.push("--codec", "hevc");
  if (options.softwareEncoder !== undefined) args.push("--software", options.softwareEncoder);
  const input = Array.isArray(capabilities)
    ? JSON.stringify(capabilities)
    : JSON.stringify(capabilities);
  return invokeCutEngineJson<EncoderSelection>(args, { input });
}

export const selectEncoder = selectVideoEncoder;
export const detectHardwareEncoders = detectEncoderCapabilities;
export const detectHardwareEncoderCapabilities = detectEncoderCapabilities;
export const chooseVideoEncoder = selectVideoEncoder;

/** Spawn-safe argument pair for an explicitly selected video encoder. */
export function encoderArgs(selection: EncoderSelection): string[] {
  return ["-c:v", selection.encoder];
}

export const videoEncoderArgs = encoderArgs;
