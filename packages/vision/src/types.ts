/**
 * The three semantic classes emitted by the custom YOLO model and exposed by
 * the vision adapter (`0=cam`, `1=chat`, `2=content`).
 */
export const VISION_TARGETS = ["cam", "chat", "content"] as const;

export type VisionTarget = (typeof VISION_TARGETS)[number];

/** The official Ultralytics YOLO26 small detection checkpoint identifier. */
export const YOLO26_SMALL_MODEL = "yolo26s.pt" as const;

/** Official Ultralytics tracker config chosen for predictable IDs. */
export const DEFAULT_TRACKER = "bytetrack.yaml" as const;
export const DEFAULT_CONFIDENCE = 0.25;
export const DEFAULT_IOU = 0.7;
export const DEFAULT_TIMEOUT_MS = 120_000;
export const DEFAULT_PYTHON_EXECUTABLE = "python3";

export type UnknownLabelPolicy = "ignore" | "error";

/**
 * Configuration for a custom YOLO26-small model.
 *
 * The model emits class names `cam`, `chat`, and `content` (IDs `0`, `1`, `2`).
 * `labelMap` is optional; when omitted or empty the adapter uses the identity
 * map for both class names and IDs. When provided, keys must be those names or
 * IDs only. Not a custom-label remap layer.
 */
export interface Yolo26TrackingConfig {
  /** Local custom-trained model weights. Remote URLs and stock model IDs are rejected. */
  readonly weights: string;
  /** Optional identity map override for `cam`/`chat`/`content` and `0`/`1`/`2`. */
  readonly labelMap?: Readonly<Record<string, VisionTarget>>;
  /** Optional explicit assertion of the fixed semantic target set. */
  readonly targets?: readonly VisionTarget[];
  /** Python executable used for the bundled runner. */
  readonly pythonExecutable?: string;
  /** Override the bundled runner script path. */
  readonly runnerScript?: string;
  /** Working directory for the process. */
  readonly cwd?: string;
  /** Ultralytics tracker YAML, e.g. `bytetrack.yaml` or `botsort.yaml`. */
  readonly tracker?: string;
  /** Ultralytics confidence threshold. `conf` is accepted as an alias. */
  readonly confidence?: number;
  readonly conf?: number;
  /** Ultralytics IoU threshold. */
  readonly iou?: number;
  /** Optional source frame rate used to derive missing timestamps. */
  readonly fps?: number;
  /** Optional Ultralytics device selector such as `cpu` or `0`. */
  readonly device?: string;
  /** Process timeout. Set on the invocation and enforced by the adapter. */
  readonly timeoutMs?: number;
  /** How labels not present in `labelMap` should be handled. */
  readonly unknownLabelPolicy?: UnknownLabelPolicy;
}

/** A source plus per-run controls. */
export interface TrackRequest {
  readonly source: string;
  /** Overrides the config FPS for timestamp derivation. */
  readonly fps?: number;
  /** Fallback dimensions for raw runner output that omits frame dimensions. */
  readonly frameWidth?: number;
  readonly frameHeight?: number;
  readonly timeoutMs?: number;
  readonly signal?: AbortSignal;
  /** Optional upper bound passed to the runner. */
  readonly maxFrames?: number;
}

export interface TrackOptions {
  readonly fps?: number;
  readonly frameWidth?: number;
  readonly frameHeight?: number;
  readonly timeoutMs?: number;
  readonly signal?: AbortSignal;
  readonly maxFrames?: number;
}

export interface ProcessInvocation {
  readonly command: string;
  readonly args: readonly string[];
  readonly cwd?: string;
  readonly env?: NodeJS.ProcessEnv;
  readonly signal?: AbortSignal;
  readonly timeoutMs?: number;
}

export interface ProcessResult {
  readonly stdout: string;
  readonly stderr: string;
  readonly exitCode: number | null;
  readonly signal?: string | null;
}

/** Injectable process boundary. Tests can provide a deterministic fake. */
export type ProcessRunner = (invocation: ProcessInvocation) => PromiseLike<ProcessResult> | ProcessResult;

export interface ProcessRunnerObject {
  run(invocation: ProcessInvocation): PromiseLike<ProcessResult> | ProcessResult;
}

export type ProcessRunnerLike = ProcessRunner | ProcessRunnerObject;

export interface NormalizedBoundingBox {
  /** Left edge, normalized to [0, 1]. */
  readonly x: number;
  /** Top edge, normalized to [0, 1]. */
  readonly y: number;
  /** Width, normalized to [0, 1]. */
  readonly width: number;
  /** Height, normalized to [0, 1]. */
  readonly height: number;
}

/** One tracked semantic detection. */
export interface NormalizedDetection {
  readonly trackId: number | string;
  readonly class: VisionTarget;
  readonly confidence: number;
  /** Zero-based frame number. */
  readonly frame: number;
  /** Timestamp in seconds. */
  readonly time: number;
  readonly frameIndex: number;
  readonly timeSeconds: number;
  readonly bbox: NormalizedBoundingBox;
}

export interface NormalizedTrackingResult {
  readonly source: string;
  readonly model: string;
  readonly targets: typeof VISION_TARGETS;
  readonly detections: readonly NormalizedDetection[];
  /** Number of frames represented by the runner output (including empty ones). */
  readonly frameCount: number;
  /** Present when supplied by config/request or runner metadata. */
  readonly fps?: number;
}

export interface TrackerDependencies {
  readonly runner?: ProcessRunnerLike;
  readonly processRunner?: ProcessRunnerLike;
}

export interface RawTrackingDetection {
  readonly track_id?: unknown;
  readonly trackId?: unknown;
  readonly class_id?: unknown;
  readonly classId?: unknown;
  readonly class_name?: unknown;
  readonly className?: unknown;
  readonly label?: unknown;
  readonly class?: unknown;
  readonly confidence?: unknown;
  readonly conf?: unknown;
  readonly bbox?: unknown;
  readonly box?: unknown;
  readonly bbox_format?: unknown;
  readonly normalized?: unknown;
}

export interface RawTrackingFrame {
  readonly frame?: unknown;
  readonly frame_index?: unknown;
  readonly frameIndex?: unknown;
  readonly time?: unknown;
  readonly time_seconds?: unknown;
  readonly timeSeconds?: unknown;
  readonly fps?: unknown;
  readonly width?: unknown;
  readonly height?: unknown;
  readonly frame_width?: unknown;
  readonly frame_height?: unknown;
  readonly orig_shape?: unknown;
  readonly shape?: unknown;
  readonly detections?: unknown;
  readonly boxes?: unknown;
  readonly tracks?: unknown;
}
