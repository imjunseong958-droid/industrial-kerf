import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type CSSProperties,
  type FormEvent,
  type MouseEvent,
  type ReactNode,
} from "react";
import {
  clipSourceId,
  clipTimelineDuration,
  clipTimelineStart,
  type TimelineClip,
} from "@cut/timeline";
import {
  ApiError,
  type AgentEvent,
  type AgentConfig,
  type AgentModelOption,
  type JsonObject,
  type PendingProposal,
  type ProposalChange,
  type Project,
  type Source,
  type StatePayload,
  acceptProposal,
  getRender,
  getAgentConfig,
  importMedia,
  isDesktopRuntime,
  loadState,
  mediaUrl,
  outputUrl,
  pickMediaFile,
  redo as redoRemote,
  resolveAssetUrl,
  resetAgentSession,
  rejectProposal,
  runAgent,
  sendCommand,
  startRender,
  stateFromResponse,
  updateAgentConfig,
  undo as undoRemote,
} from "./api";
import "./styles.css";

type IconName =
  | "arrowDown"
  | "arrowUp"
  | "chevronDown"
  | "chevronLeft"
  | "chevronRight"
  | "clock"
  | "close"
  | "compare"
  | "download"
  | "film"
  | "folder"
  | "gear"
  | "magic"
  | "menu"
  | "mic"
  | "pause"
  | "play"
  | "plus"
  | "redo"
  | "rotate"
  | "scissors"
  | "search"
  | "send"
  | "settings"
  | "sliders"
  | "sparkle"
  | "split"
  | "undo"
  | "upload"
  | "volume"
  | "waveform"
  | "x";

function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  const common = {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.8,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
    "aria-hidden": true,
  };

  const paths: Record<IconName, ReactNode> = {
    arrowDown: <path d="m6 9 6 6 6-6" />,
    arrowUp: <path d="m18 15-6-6-6 6" />,
    chevronDown: <path d="m6 9 6 6 6-6" />,
    chevronLeft: <path d="m15 18-6-6 6-6" />,
    chevronRight: <path d="m9 18 6-6-6-6" />,
    clock: (
      <>
        <circle cx="12" cy="12" r="8.5" />
        <path d="M12 7v5l3.5 2" />
      </>
    ),
    close: <path d="m7 7 10 10M17 7 7 17" />,
    compare: (
      <>
        <path d="M5 6h7M5 10h7M5 14h5M5 18h5" />
        <path d="m16 8 3 3-3 3M19 11h-7" />
      </>
    ),
    download: (
      <>
        <path d="M12 4v11" />
        <path d="m8 11 4 4 4-4M5 20h14" />
      </>
    ),
    film: (
      <>
        <rect x="4" y="5" width="16" height="14" rx="2" />
        <path d="M8 5v14M16 5v14M4 9h4M16 9h4M4 15h4M16 15h4" />
      </>
    ),
    folder: (
      <>
        <path d="M3.8 7.5a2 2 0 0 1 2-2h4l1.8 2h6.6a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5.8a2 2 0 0 1-2-2z" />
      </>
    ),
    gear: (
      <>
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1-1.7 1.7-.1-.1a1.6 1.6 0 0 0-1.8-.3 1.6 1.6 0 0 0-1 1.5v.2h-2.4v-.2a1.6 1.6 0 0 0-1-1.5 1.6 1.6 0 0 0-1.8.3l-.1.1-1.7-1.7.1-.1a1.6 1.6 0 0 0 .3-1.8 1.6 1.6 0 0 0-1.5-1H7v-2.4h.2a1.6 1.6 0 0 0 1.5-1 1.6 1.6 0 0 0-.3-1.8l-.1-.1L10 6.9l.1.1a1.6 1.6 0 0 0 1.8.3 1.6 1.6 0 0 0 1-1.5v-.2h2.4v.2a1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.8-.3l.1-.1 1.7 1.7-.1.1a1.6 1.6 0 0 0-.3 1.8 1.6 1.6 0 0 0 1.5 1h.2v2.4H21a1.6 1.6 0 0 0-1.6 1.1Z" />
      </>
    ),
    magic: (
      <>
        <path d="m15.5 4.5 4 4" />
        <path d="m4.5 15.5 11-11 4 4-11 11z" />
        <path d="m6 5 .7 1.8L8.5 7 6.7 7.7 6 9.5l-.7-1.8L3.5 7l1.8-.7zM18 16l.8 2.2L21 19l-2.2.8L18 22l-.8-2.2L15 19l2.2-.8z" />
      </>
    ),
    menu: <path d="M4 7h16M4 12h16M4 17h16" />,
    mic: (
      <>
        <rect x="9" y="3.5" width="6" height="11" rx="3" />
        <path d="M5.5 11.5a6.5 6.5 0 0 0 13 0M12 18v3M9 21h6" />
      </>
    ),
    pause: (
      <>
        <path d="M9 5v14M15 5v14" />
      </>
    ),
    play: <path d="m9 5 10 7-10 7z" fill="currentColor" stroke="none" />,
    plus: <path d="M12 5v14M5 12h14" />,
    redo: (
      <>
        <path d="M9 7h8l-3-3M17 7l-3 3" />
        <path d="M17 7a6.5 6.5 0 1 1-5.1 10.5" />
      </>
    ),
    rotate: (
      <>
        <path d="M4 12a8 8 0 1 0 2.3-5.7" />
        <path d="M4 5v5h5" />
      </>
    ),
    scissors: (
      <>
        <circle cx="6" cy="7" r="2" />
        <circle cx="6" cy="17" r="2" />
        <path d="m8 8 10 8M8 16 18 8" />
      </>
    ),
    search: (
      <>
        <circle cx="10.5" cy="10.5" r="5.7" />
        <path d="m15 15 4.5 4.5" />
      </>
    ),
    send: (
      <>
        <path d="m4 4 16 8-16 8 3-8z" />
        <path d="M7 12h13" />
      </>
    ),
    settings: (
      <>
        <path d="M4 6h10M18 6h2M4 12h3M11 12h9M4 18h8M16 18h4" />
        <circle cx="16" cy="6" r="2" />
        <circle cx="9" cy="12" r="2" />
        <circle cx="14" cy="18" r="2" />
      </>
    ),
    sliders: (
      <>
        <path d="M4 6h16M4 12h16M4 18h16" />
        <circle cx="9" cy="6" r="2" fill="var(--panel)" />
        <circle cx="15" cy="12" r="2" fill="var(--panel)" />
        <circle cx="8" cy="18" r="2" fill="var(--panel)" />
      </>
    ),
    sparkle: (
      <>
        <path d="m12 3 1.5 5.5L19 10l-5.5 1.5L12 17l-1.5-5.5L5 10l5.5-1.5z" />
        <path d="m19 16 .6 2.4L22 19l-2.4.6L19 22l-.6-2.4L16 19l2.4-.6z" />
      </>
    ),
    split: (
      <>
        <path d="M8 5v5M16 14v5M5 8h6M13 16h6" />
        <path d="m8 10 3 2-3 2M16 14l-3-2 3-2" />
      </>
    ),
    undo: (
      <>
        <path d="M9 7H5l3-3M5 7a7 7 0 1 1 1.3 8.2" />
      </>
    ),
    upload: (
      <>
        <path d="M12 15V4" />
        <path d="m8 8 4-4 4 4M5 20h14" />
      </>
    ),
    volume: (
      <>
        <path d="M5 10v4h3l4 3V7l-4 3zM16 9.5a4 4 0 0 1 0 5M18.5 7a7 7 0 0 1 0 10" />
      </>
    ),
    waveform: (
      <>
        <path d="M3 12h2l1.5-5 2.5 10 2.2-13 2.3 16 2-8 1.5 4H21" />
      </>
    ),
    x: <path d="m7 7 10 10M17 7 7 17" />,
  };

  return <svg {...common}>{paths[name]}</svg>;
}

/**
 * Serializable color effect params. Keep this shape aligned with
 * ColorEffectParamsSchema in packages/timeline/src/v2.ts so a future host can
 * pass this object directly to a color effect command.
 */
type ColorState = {
  exposure: number;
  contrast: number;
  brightness: number;
  highlights: number;
  shadows: number;
  temperature: number;
  tint: number;
  saturation: number;
  gamma: number;
  vibrance: number;
  mix: number;
};

type ColorKey = keyof ColorState;

type MotionKey = "x" | "y" | "scale" | "rotation" | "opacity";
type CropKey = "x" | "y" | "width" | "height";

type MotionState = Record<MotionKey, number>;
type CropState = Record<CropKey, number>;

const DEFAULT_MOTION: MotionState = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };
const DEFAULT_CROP: CropState = { x: 0, y: 0, width: 1, height: 1 };

type Change = {
  id: string;
  title: string;
  detail: string;
  range: string;
  tone: "violet" | "blue" | "amber" | "green";
  icon: IconName;
};

type ChatMessage = {
  id: string;
  role: "user" | "assistant" | "error";
  text: string;
};

export type ActiveOverlay = "project-menu" | "settings" | "viewer-options" | "audio-controls" | null;

export type ViewerAudioState = {
  volume: number;
  muted: boolean;
  lastVolume: number;
};

export type ViewerOptions = {
  showAfter: boolean;
  captionsVisible: boolean;
};

const DEFAULT_COLOR: ColorState = {
  exposure: 0,
  contrast: 1,
  brightness: 0,
  highlights: 0,
  shadows: 0,
  temperature: 0,
  tint: 0,
  saturation: 1,
  gamma: 1,
  vibrance: 0,
  mix: 1,
};

const NATURAL_COLOR: ColorState = {
  ...DEFAULT_COLOR,
  exposure: 0.18,
  contrast: 1.08,
  brightness: 0.02,
  highlights: -0.12,
  shadows: 0.2,
  temperature: 8,
  saturation: 1.08,
  vibrance: 0.12,
};

const COLOR_CONTROLS: Array<{
  key: ColorKey;
  label: string;
  min: number;
  max: number;
  step: number;
  format: (value: number) => string;
}> = [
  { key: "exposure", label: "Exposure", min: -3, max: 3, step: 0.01, format: (v) => `${v >= 0 ? "+" : ""}${v.toFixed(2)}` },
  { key: "contrast", label: "Contrast", min: 0, max: 4, step: 0.01, format: (v) => `${Math.round(v * 100)}%` },
  { key: "brightness", label: "Brightness", min: -1, max: 1, step: 0.01, format: (v) => `${v >= 0 ? "+" : ""}${v.toFixed(2)}` },
  { key: "highlights", label: "Highlights", min: -1, max: 1, step: 0.01, format: (v) => `${v >= 0 ? "+" : ""}${v.toFixed(2)}` },
  { key: "shadows", label: "Shadows", min: -1, max: 1, step: 0.01, format: (v) => `${v >= 0 ? "+" : ""}${v.toFixed(2)}` },
  { key: "temperature", label: "Temperature", min: -100, max: 100, step: 1, format: (v) => `${v >= 0 ? "+" : ""}${Math.round(v)}` },
  { key: "tint", label: "Tint", min: -1, max: 1, step: 0.01, format: (v) => `${v >= 0 ? "+" : ""}${v.toFixed(2)}` },
  { key: "saturation", label: "Saturation", min: 0, max: 4, step: 0.01, format: (v) => `${Math.round(v * 100)}%` },
  { key: "gamma", label: "Gamma", min: 0.1, max: 10, step: 0.01, format: (v) => v.toFixed(2) },
  { key: "vibrance", label: "Vibrance", min: -1, max: 1, step: 0.01, format: (v) => `${v >= 0 ? "+" : ""}${v.toFixed(2)}` },
  { key: "mix", label: "Mix", min: 0, max: 1, step: 0.01, format: (v) => `${Math.round(v * 100)}%` },
];

const COLOR_PRESETS: Array<{ name: string; description: string; color: ColorState }> = [
  { name: "Natural", description: "Balanced", color: NATURAL_COLOR },
  { name: "Warm", description: "Golden hour", color: { ...NATURAL_COLOR, temperature: 38, tint: 0.04, saturation: 1.14 } },
  { name: "Clean", description: "Crisp studio", color: { ...NATURAL_COLOR, exposure: 0.25, contrast: 1.16, brightness: 0.05, temperature: -3, saturation: 1.02, gamma: 1.04 } },
  { name: "Cinematic", description: "Soft contrast", color: { ...NATURAL_COLOR, exposure: 0.02, contrast: 1.22, highlights: -0.34, shadows: -0.08, saturation: 0.92, vibrance: -0.05, gamma: 1.05 } },
];

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

/** Toggle one of the small, local UI overlays without allowing two at once. */
export function toggleOverlay(active: ActiveOverlay, next: Exclude<ActiveOverlay, null>): ActiveOverlay {
  return active === next ? null : next;
}

/** Keep viewer playback audio independent from serialized clip volume. */
export function setViewerAudioVolume(current: ViewerAudioState, value: number): ViewerAudioState {
  const volume = clamp(Number.isFinite(value) ? value : current.volume, 0, 1);
  return {
    volume,
    muted: volume === 0,
    lastVolume: volume > 0 ? volume : current.lastVolume,
  };
}

export function toggleViewerAudio(current: ViewerAudioState): ViewerAudioState {
  if (current.muted || current.volume === 0) {
    const volume = clamp(current.lastVolume || 1, 0.01, 1);
    return { volume, muted: false, lastVolume: volume };
  }
  return { volume: current.volume, muted: true, lastVolume: current.volume };
}

export function updateViewerOptions(current: ViewerOptions, patch: Partial<ViewerOptions>): ViewerOptions {
  return { ...current, ...patch };
}

export function fullscreenAction(isFullscreen: boolean, hasFullscreenElement: boolean): "enter" | "exit" {
  return isFullscreen || hasFullscreenElement ? "exit" : "enter";
}

function formatTime(seconds: number) {
  const safeSeconds = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(safeSeconds / 60);
  const remainder = safeSeconds % 60;
  return `${String(minutes).padStart(2, "0")}:${String(remainder).padStart(2, "0")}`;
}

function formatDuration(seconds: number) {
  if (!Number.isFinite(seconds) || seconds <= 0) return "00:00";
  return formatTime(seconds);
}

type RenderState = {
  quality: "preview" | "full";
  jobId?: string;
  status: string;
  outputUrl?: string;
  error?: string;
};

type VideoClip = TimelineClip & {
  trackId: string;
  trackName: string;
  trackType: string;
  sourceMeta?: Source;
  start: number;
  clipDuration: number;
};

function numberValue(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function clipLength(clip: TimelineClip, source?: Source): number {
  const duration = clipTimelineDuration(clip);
  if (duration !== undefined && duration > 0) return duration;
  if (typeof clip.in === "number" && typeof clip.out === "number" && clip.out > clip.in) {
    return (clip.out - clip.in) / Math.max(0.001, numberValue(clip.speed, 1));
  }
  if (source?.duration !== undefined) return Math.max(0, source.duration - numberValue(clip.in));
  return 0;
}

function videoClips(project: Project | undefined, sources: Source[]): VideoClip[] {
  if (!project || !Array.isArray(project.tracks)) return [];
  const sourceById = new Map(sources.map((source) => [source.id, source]));
  const clips: VideoClip[] = [];
  project.tracks.forEach((track) => {
    const trackType = String(track.type ?? "video").toLowerCase();
    if (trackType !== "video") return;
    (Array.isArray(track.clips) ? track.clips : []).forEach((clip) => {
      const source = sourceById.get(clipSourceId(clip) ?? "");
      clips.push({
        ...clip,
        trackId: track.id,
        trackName: track.name ?? track.id,
        trackType,
        sourceMeta: source,
        start: clipTimelineStart(clip),
        clipDuration: clipLength(clip, source),
      });
    });
  });
  return clips.sort((a, b) => a.start - b.start);
}

type CaptionCue = {
  start: number;
  end: number;
  text: string;
};

type SubtitleInfo = {
  available: boolean;
  label?: string;
  path?: string;
  cues: CaptionCue[];
};

function isSubtitleTrack(track: { type?: string; kind?: string; name?: string }): boolean {
  const value = String(track.type ?? track.name ?? "").toLowerCase();
  return value.includes("subtitle") || value.includes("caption") || value === "text" || value.includes("transcript");
}

function parseCue(value: unknown): CaptionCue | undefined {
  if (!value || typeof value !== "object") return undefined;
  const candidate = value as Record<string, unknown>;
  const start = numberValue(candidate.start ?? candidate.startTime ?? candidate.from, NaN);
  const end = numberValue(candidate.end ?? candidate.endTime ?? candidate.to, NaN);
  const rawText = candidate.text ?? candidate.caption ?? candidate.content ?? candidate.value;
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start || typeof rawText !== "string" || !rawText.trim()) return undefined;
  return { start, end, text: rawText.trim() };
}

function subtitleValues(value: unknown): { path?: string; label?: string; cues: CaptionCue[] } {
  if (typeof value === "string") return { path: value, label: value, cues: [] };
  if (!value || typeof value !== "object") return { cues: [] };
  const object = value as Record<string, unknown>;
  const pathValue = object.path ?? object.url ?? object.file ?? object.src ?? object.subtitlePath ?? object.subsPath;
  const labelValue = object.label ?? object.language ?? object.lang ?? object.name ?? object.title;
  const rawCues = object.cues ?? object.captions ?? object.entries ?? object.items;
  const cues = Array.isArray(rawCues) ? rawCues.map(parseCue).filter((cue): cue is CaptionCue => Boolean(cue)) : [];
  return {
    path: typeof pathValue === "string" ? pathValue : undefined,
    label: typeof labelValue === "string" ? labelValue : undefined,
    cues,
  };
}

/** Normalize the subtitle metadata shapes returned by imported sources. */
export function subtitleInfoForSource(source: Source | undefined, extra?: unknown): SubtitleInfo {
  if (!source && extra === undefined) return { available: false, cues: [] };
  const values: unknown[] = [
    extra,
    source?.subtitle,
    source?.subtitles,
    source?.subs,
    source?.captions,
    source?.subtitlePath,
    source?.subtitlesPath,
    source?.subsPath,
    source?.metadata?.subtitle,
    source?.metadata?.subtitles,
    source?.metadata?.captions,
  ];
  const merged = values.reduce<{ path?: string; label?: string; cues: CaptionCue[] }>((result, value) => {
    const next = subtitleValues(value);
    return {
      path: result.path ?? next.path,
      label: result.label ?? next.label,
      cues: [...result.cues, ...next.cues],
    };
  }, { cues: [] } as { path?: string; label?: string; cues: CaptionCue[] });
  const available = values.some((value) => value !== undefined && value !== null && value !== false)
    || merged.cues.length > 0;
  return { ...merged, available };
}

function subtitleCuesFromTrack(track: { clips?: TimelineClip[]; [key: string]: unknown }): CaptionCue[] {
  const trackCues = Array.isArray(track.cues) ? track.cues : Array.isArray(track.captions) ? track.captions : [];
  const cues = trackCues.map(parseCue).filter((cue): cue is CaptionCue => Boolean(cue));
  for (const clip of Array.isArray(track.clips) ? track.clips : []) {
    const clipStartTime = clipTimelineStart(clip);
    const rawCues = Array.isArray(clip.cues) ? clip.cues : clip.cue ? [clip.cue] : [];
    if (rawCues.length) {
      rawCues.forEach((rawCue) => {
        const cue = parseCue(rawCue);
        if (cue) cues.push({ ...cue, start: cue.start + clipStartTime, end: cue.end + clipStartTime });
      });
      continue;
    }
    const text = clip.text ?? clip.caption ?? clip.subtitle;
    if (typeof text === "string" && text.trim()) {
      const start = clipStartTime;
      const end = start + clipLength(clip);
      if (end > start) cues.push({ start, end, text: text.trim() });
    }
  }
  return cues;
}

/** Return the caption cue active at the viewer's current timeline time. */
export function captionTextForTime(project: Project | undefined, currentTime: number): string | undefined {
  if (!project) return undefined;
  for (const track of Array.isArray(project.tracks) ? project.tracks : []) {
    if (!isSubtitleTrack(track)) continue;
    const cue = subtitleCuesFromTrack(track).find((candidate) => currentTime >= candidate.start && currentTime < candidate.end);
    if (cue) return cue.text;
  }
  const overlays = Array.isArray(project.overlays) ? project.overlays : [];
  for (const overlay of overlays) {
    if (!overlay || typeof overlay !== "object") continue;
    const object = overlay as Record<string, unknown>;
    const type = String(object.type ?? object.kind ?? "").toLowerCase();
    if (!type.includes("caption") && !type.includes("subtitle") && type !== "text") continue;
    const cues = Array.isArray(object.cues) ? object.cues : Array.isArray(object.entries) ? object.entries : [];
    const cue = cues.map(parseCue).filter((candidate): candidate is CaptionCue => Boolean(candidate)).find((candidate) => currentTime >= candidate.start && currentTime < candidate.end);
    if (cue) return cue.text;
  }
  return undefined;
}

function projectTimelineDuration(state: StatePayload | null, clips: VideoClip[]): number {
  const reported = state?.duration ?? state?.project.duration ?? 0;
  const clipEnd = clips.reduce((end, clip) => Math.max(end, clip.start + clip.clipDuration), 0);
  return Math.max(numberValue(reported), clipEnd);
}

function colorParamsFromClip(clip: TimelineClip | undefined): ColorState {
  const effects = Array.isArray(clip?.effects) ? clip.effects : [];
  const effect = effects.find((candidate) => String(candidate.type ?? "").toLowerCase() === "color");
  const params = effect && typeof effect.params === "object" && effect.params !== null
    ? effect.params as Record<string, unknown>
    : {};
  return {
    exposure: numberValue(params.exposure),
    contrast: numberValue(params.contrast, 1),
    brightness: numberValue(params.brightness),
    highlights: numberValue(params.highlights),
    shadows: numberValue(params.shadows),
    temperature: numberValue(params.temperature),
    tint: numberValue(params.tint),
    saturation: numberValue(params.saturation, 1),
    gamma: numberValue(params.gamma, 1),
    vibrance: numberValue(params.vibrance),
    mix: numberValue(params.mix, 1),
  };
}

function transformFromClip(clip: TimelineClip | undefined): MotionState {
  const transform = clip?.transform && typeof clip.transform === "object"
    ? clip.transform as Record<string, unknown>
    : {};
  const position = transform.position && typeof transform.position === "object"
    ? transform.position as Record<string, unknown>
    : undefined;
  return {
    x: numberValue(transform.x ?? position?.x),
    y: numberValue(transform.y ?? position?.y),
    scale: numberValue(transform.scale, 1),
    rotation: numberValue(transform.rotation),
    opacity: numberValue(clip?.opacity, 1),
  };
}

function cropFromClip(clip: TimelineClip | undefined): CropState {
  const crop = clip?.crop && typeof clip.crop === "object" ? clip.crop as Record<string, unknown> : {};
  return {
    x: numberValue(crop.x),
    y: numberValue(crop.y),
    width: numberValue(crop.width, 1),
    height: numberValue(crop.height, 1),
  };
}

function preserveTransform(clip: TimelineClip, motion: Partial<MotionState>): JsonObject {
  const current = clip.transform && typeof clip.transform === "object"
    ? clip.transform as JsonObject
    : {};
  const transform = { ...current } as JsonObject;
  (Object.keys(motion) as MotionKey[]).forEach((key) => {
    if (key !== "opacity") transform[key] = motion[key] as number;
  });
  return transform;
}

function modelOptionLabel(option: AgentModelOption): string {
  return option.label || option.id;
}

function MotionInspector({
  motion,
  crop,
  motionChanged,
  cropChanged,
  selectedClip,
  state,
  blocked = false,
  onChange,
  onCropChange,
  onReset,
}: {
  motion: MotionState;
  crop: CropState;
  motionChanged: boolean;
  cropChanged: boolean;
  selectedClip?: TimelineClip;
  state: StatePayload | null;
  blocked?: boolean;
  onChange: (key: MotionKey, value: number) => void;
  onCropChange: (key: CropKey, value: number) => void;
  onReset: () => void;
}) {
  const motionControls: Array<{ key: MotionKey; label: string; min: number; max: number; step: number; format: (value: number) => string }> = [
    { key: "x", label: "Position X", min: -1, max: 1, step: 0.01, format: (value) => `${value >= 0 ? "+" : ""}${value.toFixed(2)}` },
    { key: "y", label: "Position Y", min: -1, max: 1, step: 0.01, format: (value) => `${value >= 0 ? "+" : ""}${value.toFixed(2)}` },
    { key: "scale", label: "Scale", min: 0.1, max: 4, step: 0.01, format: (value) => `${Math.round(value * 100)}%` },
    { key: "rotation", label: "Rotation", min: -180, max: 180, step: 1, format: (value) => `${value >= 0 ? "+" : ""}${Math.round(value)}°` },
    { key: "opacity", label: "Opacity", min: 0, max: 1, step: 0.01, format: (value) => `${Math.round(value * 100)}%` },
  ];
  const cropControls: Array<{ key: CropKey; label: string; min: number; max: number; step: number; format: (value: number) => string }> = [
    { key: "x", label: "Crop X", min: 0, max: 0.95, step: 0.01, format: (value) => `${Math.round(value * 100)}%` },
    { key: "y", label: "Crop Y", min: 0, max: 0.95, step: 0.01, format: (value) => `${Math.round(value * 100)}%` },
    { key: "width", label: "Crop width", min: 0.05, max: 1, step: 0.01, format: (value) => `${Math.round(value * 100)}%` },
    { key: "height", label: "Crop height", min: 0.05, max: 1, step: 0.01, format: (value) => `${Math.round(value * 100)}%` },
  ];
  const disabled = !selectedClip || !state || blocked;
  return <div className="inspector-content adjustment-content">
    <div className="inspector-section adjustment-overview">
      <div className="section-heading-row"><div><span className="section-label">Motion & framing</span><strong>{selectedClip?.id ?? "No video clip"}</strong></div><button className="text-button" onClick={onReset} disabled={disabled || (!motionChanged && !cropChanged)}>Reset</button></div>
      <p className="adjustment-help">Tune the selected clip directly. Changes preview on the canvas and save automatically.</p>
    </div>
    <div className="inspector-section sliders-section">
      <div className="section-heading-row"><span className="section-label">Transform</span><span className="range-hint">auto-saved</span></div>
      <div className="color-sliders">
        {motionControls.map((control) => <label className="slider-row" key={control.key}>
          <span className="slider-label"><span>{control.label}</span><output>{control.format(motion[control.key])}</output></span>
          <input type="range" min={control.min} max={control.max} step={control.step} value={motion[control.key]} onChange={(event) => onChange(control.key, Number(event.target.value))} aria-label={control.label} disabled={disabled} />
        </label>)}
      </div>
    </div>
    <div className="inspector-section sliders-section">
      <div className="section-heading-row"><span className="section-label">Crop</span><span className="range-hint">normalized 0-100%</span></div>
      <div className="color-sliders">
        {cropControls.map((control) => <label className="slider-row" key={control.key}>
          <span className="slider-label"><span>{control.label}</span><output>{control.format(crop[control.key])}</output></span>
          <input type="range" min={control.min} max={control.max} step={control.step} value={crop[control.key]} onChange={(event) => onCropChange(control.key, Number(event.target.value))} aria-label={control.label} disabled={disabled} />
        </label>)}
      </div>
    </div>
  </div>;
}

function AudioInspector({
  volume,
  muted,
  selectedClip,
  state,
  blocked = false,
  onChange,
  onToggleMute,
}: {
  volume: number;
  muted: boolean;
  selectedClip?: TimelineClip;
  state: StatePayload | null;
  blocked?: boolean;
  onChange: (value: number) => void;
  onToggleMute: () => void;
}) {
  const disabled = !selectedClip || !state || blocked;
  return <div className="inspector-content adjustment-content">
    <div className="inspector-section adjustment-overview">
      <div className="section-heading-row"><div><span className="section-label">Clip audio</span><strong>{selectedClip?.id ?? "No video clip"}</strong></div><button type="button" className={`mute-button ${muted ? "active" : ""}`} onClick={onToggleMute} disabled={disabled}><Icon name={muted ? "close" : "volume"} size={13} /> {muted ? "Muted" : "Mute"}</button></div>
      <p className="adjustment-help">Set a linear clip volume. Mute keeps the last level so it can be restored with one click.</p>
    </div>
    <div className="inspector-section sliders-section">
      <div className="section-heading-row"><span className="section-label">Volume</span><span className="range-hint">0-200%</span></div>
      <div className="color-sliders">
        <label className="slider-row"><span className="slider-label"><span>Volume</span><output>{Math.round(volume * 100)}%</output></span><input type="range" min={0} max={2} step={0.01} value={volume} onChange={(event) => onChange(Number(event.target.value))} aria-label="Volume" disabled={disabled} /></label>
      </div>
    </div>
  </div>;
}

function preserveColorEffects(clip: TimelineClip, color: ColorState): NonNullable<TimelineClip["effects"]> {
  const effects = Array.isArray(clip.effects) ? [...clip.effects] : [];
  const nextEffect = { type: "color", enabled: true, params: color };
  let replaced = false;
  const next = effects.map((effect) => {
    if (!replaced && String(effect.type ?? "").toLowerCase() === "color") {
      replaced = true;
      return { ...effect, type: "color", enabled: true, params: color };
    }
    return effect;
  });
  if (!replaced) next.push(nextEffect);
  return next;
}

function eventObject(event: AgentEvent): Record<string, unknown> {
  return typeof event === "object" && event !== null ? event : { text: event };
}

function eventKind(event: AgentEvent): string {
  const value = eventObject(event);
  return String(value.type ?? value.kind ?? value.event ?? value.tool ?? "agent").toLowerCase();
}

function eventText(event: AgentEvent): string {
  const value = eventObject(event);
  for (const key of ["summary", "message", "text", "detail", "output", "result"]) {
    const candidate = value[key];
    if (typeof candidate === "string" && candidate.trim()) return candidate.trim();
    if (candidate && typeof candidate === "object") {
      try { return JSON.stringify(candidate); } catch { /* ignore circular tool output */ }
    }
  }
  return JSON.stringify(value);
}

function eventToChange(event: AgentEvent, index: number): Change {
  const value = eventObject(event);
  const kind = eventKind(event);
  const isColor = kind.includes("color") || kind.includes("grade") || eventText(event).toLowerCase().includes("color");
  const isAudio = kind.includes("audio") || kind.includes("music") || eventText(event).toLowerCase().includes("music");
  const range = typeof value.range === "string"
    ? value.range
    : typeof value.start === "number" || typeof value.end === "number"
      ? `${formatTime(numberValue(value.start))}-${formatTime(numberValue(value.end, numberValue(value.start)))} `
      : "Project update";
  const title = typeof value.title === "string"
    ? value.title
    : typeof value.name === "string"
      ? value.name
    : typeof value.tool === "string"
      ? value.tool
      : kind === "agent" ? "Agent update" : kind.replace(/[_-]/g, " ");
  return {
    id: `event-${index}-${title}`,
    title: title.charAt(0).toUpperCase() + title.slice(1),
    detail: eventText(event),
    range,
    tone: isColor ? "violet" : isAudio ? "amber" : kind.includes("error") ? "amber" : "blue",
    icon: isColor ? "sliders" : isAudio ? "waveform" : kind.includes("edit") || kind.includes("command") ? "scissors" : "magic",
  };
}

const NON_CHANGE_EVENT_KINDS = new Set([
  "text_delta",
  "turn_started",
  "done",
  "result",
  "summary",
  "message",
  "assistant",
  "user",
  "thinking",
  "progress",
  "heartbeat",
  "aborted",
  "tool_call_started",
]);

const READ_ONLY_TOOL_NAMES = new Set([
  "probe",
  "timeline",
  "timeline_query",
  "project.inspect",
  "project_inspect",
  "render_status",
  "render_result",
  "media.probe",
  "project.validate",
  "timeline.diff",
  "vision.track_objects",
  "track_objects",
  "render.status",
  "render.result",
  "skill_read",
  "skills.read",
]);

function isMeaningfulChangeEvent(event: AgentEvent): boolean {
  const value = eventObject(event);
  const kind = eventKind(event);
  if (NON_CHANGE_EVENT_KINDS.has(kind)) return false;
  if (kind !== "tool_call_finished") return true;
  // A failed read-only call is still useful feedback; successful inspections
  // are not project changes and should not become change cards.
  if (value.ok === false) return true;
  const name = String(value.name ?? value.tool ?? "").toLowerCase();
  return !READ_ONLY_TOOL_NAMES.has(name);
}

/** Convert only meaningful agent events into a compact, deduplicated change list. */
export function changesFromEvents(events: AgentEvent[]): Change[] {
  const changes: Change[] = [];
  const seen = new Set<string>();
  events.forEach((event, index) => {
    if (!isMeaningfulChangeEvent(event)) return;
    const change = eventToChange(event, index);
    const signature = `${change.title}\u0000${change.range}\u0000${change.detail}`;
    if (seen.has(signature)) return;
    seen.add(signature);
    changes.push(change);
  });
  return changes;
}

function responseSummary(response: { text?: string }, hasChanges: boolean): string {
  const text = response.text?.trim();
  if (text) return text;
  return hasChanges
    ? "I completed the requested edit. The change details are listed below."
    : "I completed the request without a project change.";
}

function proposalValueLabel(value: unknown): string {
  if (value === undefined) return "—";
  if (typeof value === "string") return value || "empty";
  try {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) return "—";
    return encoded.length > 92 ? `${encoded.slice(0, 89)}…` : encoded;
  } catch {
    return String(value);
  }
}

function proposalChangeTitle(change: ProposalChange): string {
  const segments = change.path.replace(/^\$\.?/, "").split(/[.\[]/).filter(Boolean);
  const target = segments.at(-1)?.replace(/\]$/, "") || "project";
  return `${change.kind === "added" ? "Add" : change.kind === "removed" ? "Remove" : "Update"} ${target}`;
}

function proposalChangeDetail(change: ProposalChange): string {
  if (change.kind === "added") return proposalValueLabel(change.after);
  if (change.kind === "removed") return proposalValueLabel(change.before);
  return `${proposalValueLabel(change.before)} → ${proposalValueLabel(change.after)}`;
}

function proposalChangeTone(change: ProposalChange): Change["tone"] {
  if (change.kind === "added") return "green";
  if (change.kind === "removed") return "amber";
  return "violet";
}

function proposalChangeIcon(change: ProposalChange): IconName {
  if (change.kind === "added") return "plus";
  if (change.kind === "removed") return "x";
  return "compare";
}

function App() {
  const [state, setState] = useState<StatePayload | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [viewerOptions, setViewerOptions] = useState<ViewerOptions>({ showAfter: true, captionsVisible: true });
  const [activeOverlay, setActiveOverlay] = useState<ActiveOverlay>(null);
  const [viewerAudio, setViewerAudio] = useState<ViewerAudioState>({ volume: 1, muted: false, lastVolume: 1 });
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [activeRightPanel, setActiveRightPanel] = useState<"chat" | "adjust">("chat");
  const [activeInspectorTab, setActiveInspectorTab] = useState("Color");
  const [color, setColor] = useState<ColorState>(DEFAULT_COLOR);
  const colorRef = useRef<ColorState>(DEFAULT_COLOR);
  const [motion, setMotion] = useState<MotionState>(DEFAULT_MOTION);
  const motionRef = useRef<MotionState>(DEFAULT_MOTION);
  const [crop, setCrop] = useState<CropState>(DEFAULT_CROP);
  const cropRef = useRef<CropState>(DEFAULT_CROP);
  const [agentConfig, setAgentConfig] = useState<AgentConfig | null>(null);
  const [agentConfigBusy, setAgentConfigBusy] = useState(false);
  const [agentConfigError, setAgentConfigError] = useState<string | null>(null);
  const [selectedModel, setSelectedModel] = useState("");
  const stateRef = useRef<StatePayload | null>(null);
  const pendingClipPatches = useRef<Map<string, JsonObject>>(new Map());
  const clipPatchTimer = useRef<number | undefined>(undefined);
  const clipPatchBusy = useRef(false);
  const lastVolumeRef = useRef(1);
  const viewerStageRef = useRef<HTMLDivElement | null>(null);
  const [timelineExpanded, setTimelineExpanded] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [agentBusy, setAgentBusy] = useState(false);
  const [proposalBusy, setProposalBusy] = useState(false);
  const [chatResetBusy, setChatResetBusy] = useState(false);
  const [agentError, setAgentError] = useState<string | null>(null);
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([]);
  const [changes, setChanges] = useState<Change[]>([]);
  const [selectedChange, setSelectedChange] = useState<string | null>(null);
  const [selectedClipId, setSelectedClipId] = useState<string | null>(null);
  const [selectedSourceId, setSelectedSourceId] = useState<string | null>(null);
  const [importedSubtitleMeta, setImportedSubtitleMeta] = useState<Record<string, unknown>>({});
  const [mediaBinOpen, setMediaBinOpen] = useState(false);
  const [mediaImportBusy, setMediaImportBusy] = useState(false);
  const [mediaImportError, setMediaImportError] = useState<string | null>(null);
  const [mediaUrlInput, setMediaUrlInput] = useState("");
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | undefined>(undefined);
  const [exportUrl, setExportUrl] = useState<string | undefined>(undefined);
  const [render, setRender] = useState<RenderState | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const chatThreadRef = useRef<HTMLDivElement | null>(null);
  const promptInputRef = useRef<HTMLTextAreaElement | null>(null);
  const chatGeneration = useRef(0);
  const chatMessageSequence = useRef(0);

  const clips = useMemo(() => videoClips(state?.project, state?.sources ?? []), [state]);
  const activeDuration = projectTimelineDuration(state, clips);
  const progress = clamp((currentTime / activeDuration) * 100, 0, 100);
  const selectedClip = clips.find((clip) => clip.id === selectedClipId) ?? clips[0];
  const pendingProposal: Extract<PendingProposal, { status: "pending" }> | undefined = state?.pendingProposal?.status === "pending"
    ? state.pendingProposal
    : undefined;
  const editsBlocked = Boolean(pendingProposal || proposalBusy);
  stateRef.current = state;
  const sourceById = useMemo(() => new Map((state?.sources ?? []).map((item) => [item.id, item])), [state?.sources]);
  const source = sourceById.get(selectedSourceId ?? "") ?? selectedClip?.sourceMeta ?? state?.sources[0];
  const { showAfter, captionsVisible } = viewerOptions;
  const viewerSourceUrl = source ? mediaUrl(source.id, source.path) : undefined;
  const viewerUrl = !pendingProposal && showAfter && previewUrl ? previewUrl : viewerSourceUrl;
  const activeCaption = captionTextForTime(state?.project, currentTime);
  const projectName = state?.project.name ?? state?.project.id ?? "Untitled project";
  const colorFilter = useMemo(() => {
    // Apply mix around the backend identities before approximating the grade
    // with CSS filters. The serialized values stay backend-compatible even
    // though CSS cannot reproduce every colorbalance/gamma operation exactly.
    const mix = color.mix;
    const exposure = color.exposure * mix;
    const contrastValue = 1 + (color.contrast - 1) * mix;
    const brightnessValue = color.brightness * mix;
    const saturationValue = 1 + (color.saturation - 1) * mix;
    const gammaValue = 1 + (color.gamma - 1) * mix;
    const temperature = color.temperature * mix;
    const tint = color.tint * mix;
    const highlights = color.highlights * mix;
    const shadows = color.shadows * mix;
    const vibrance = color.vibrance * mix;
    const brightness = clamp(
      Math.pow(2, exposure * 0.16) * (1 + brightnessValue * 0.35 + shadows * 0.08 - highlights * 0.03) * Math.pow(gammaValue, -0.18),
      0.35,
      1.9,
    );
    const contrast = clamp(contrastValue + highlights * -0.08, 0.2, 2.8);
    const saturation = clamp(saturationValue + vibrance * 0.24, 0, 4.2);
    const sepia = clamp(Math.max(0, temperature) / 1000 * 0.35, 0, 0.35);
    const hue = tint * 18 - Math.max(0, -temperature) * 0.015;
    return `brightness(${brightness.toFixed(3)}) contrast(${contrast.toFixed(3)}) saturate(${saturation.toFixed(3)}) sepia(${sepia.toFixed(3)}) hue-rotate(${hue.toFixed(2)}deg)`;
  }, [color]);

  const colorChanged = useMemo(() => JSON.stringify(color) !== JSON.stringify(colorParamsFromClip(selectedClip)), [color, selectedClip]);
  const motionChanged = useMemo(() => JSON.stringify(motion) !== JSON.stringify(transformFromClip(selectedClip)), [motion, selectedClip]);
  const cropChanged = useMemo(() => JSON.stringify(crop) !== JSON.stringify(cropFromClip(selectedClip)), [crop, selectedClip]);
  const audioVolume = numberValue(selectedClip?.volume, 1);
  const audioMuted = selectedClip?.muted === true || audioVolume === 0;
  const agentModels = agentConfig?.models ?? [];
  const hasModelPicker = Boolean(agentConfig && (agentModels.length > 0 || agentConfig.model || agentConfig.provider));

  const refreshState = async () => {
    try {
      const next = await loadState();
      setState(next);
      setLoadError(null);
      return next;
    } catch (error) {
      const message = error instanceof Error ? error.message : "Unable to load project state";
      setLoadError(message);
      throw error;
    }
  };

  useEffect(() => {
    void refreshState().finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    let active = true;
    void getAgentConfig().then((config) => {
      if (!active) return;
      setAgentConfig(config);
      setSelectedModel(config.model ?? config.models?.[0]?.id ?? "");
      setAgentConfigError(null);
    }).catch((error) => {
      // Older hosts do not expose model configuration. Keep chat fully usable
      // and hide the picker instead of presenting an inert control.
      if (!active) return;
      setAgentConfigError(error instanceof Error ? error.message : "Agent model configuration unavailable");
      setAgentConfig(null);
    });
    return () => { active = false; };
  }, []);

  useEffect(() => {
    const first = clips[0];
    if (!first) {
      setSelectedClipId(null);
      setColor(DEFAULT_COLOR);
      colorRef.current = DEFAULT_COLOR;
      setMotion(DEFAULT_MOTION);
      motionRef.current = DEFAULT_MOTION;
      setCrop(DEFAULT_CROP);
      cropRef.current = DEFAULT_CROP;
      return;
    }
    if (!selectedClipId || !clips.some((clip) => clip.id === selectedClipId)) setSelectedClipId(first.id);
  }, [clips, selectedClipId]);

  useEffect(() => {
    const sources = state?.sources ?? [];
    if (!sources.length) {
      setSelectedSourceId(null);
      return;
    }
    if (selectedSourceId && sources.some((candidate) => candidate.id === selectedSourceId)) return;
    const clipSource = clips.find((clip) => clip.id === selectedClipId)?.sourceMeta?.id;
    setSelectedSourceId(clipSource ?? sources[0].id);
  }, [state?.sources, clips, selectedClipId, selectedSourceId]);

  useEffect(() => {
    const next = colorParamsFromClip(selectedClip);
    setColor(next);
    colorRef.current = next;
    const nextMotion = transformFromClip(selectedClip);
    setMotion(nextMotion);
    motionRef.current = nextMotion;
    const nextCrop = cropFromClip(selectedClip);
    setCrop(nextCrop);
    cropRef.current = nextCrop;
    const volume = numberValue(selectedClip?.volume, 1);
    if (volume > 0) lastVolumeRef.current = volume;
  }, [selectedClip?.id, state?.revision]);

  useEffect(() => () => {
    if (clipPatchTimer.current !== undefined) window.clearTimeout(clipPatchTimer.current);
  }, []);

  useEffect(() => {
    setCurrentTime((time) => clamp(time, 0, activeDuration));
  }, [activeDuration]);

  useEffect(() => {
    const handleFullscreenChange = () => {
      setIsFullscreen(document.fullscreenElement === viewerStageRef.current);
    };
    document.addEventListener("fullscreenchange", handleFullscreenChange);
    return () => document.removeEventListener("fullscreenchange", handleFullscreenChange);
  }, []);

  useEffect(() => {
    if (!isFullscreen) return;
    const timer = window.setInterval(() => {
      if (document.fullscreenElement !== viewerStageRef.current) setIsFullscreen(false);
    }, 250);
    return () => window.clearInterval(timer);
  }, [isFullscreen]);

  useEffect(() => {
    const media = videoRef.current;
    if (!media) return;
    media.volume = viewerAudio.volume;
    media.muted = viewerAudio.muted;
  }, [viewerAudio, viewerUrl]);

  useEffect(() => {
    if (!activeOverlay) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setActiveOverlay(null);
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [activeOverlay]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 3200);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    const thread = chatThreadRef.current;
    if (thread) thread.scrollTop = thread.scrollHeight;
  }, [chatMessages.length, changes.length, agentBusy]);

  useEffect(() => {
    const textarea = promptInputRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    textarea.style.height = `${Math.min(Math.max(textarea.scrollHeight, 42), 128)}px`;
  }, [prompt]);

  const togglePlayback = () => {
    const media = videoRef.current;
    if (media && viewerUrl) {
      if (media.paused) {
        void media.play();
      } else {
        media.pause();
      }
    } else setToast("Load a project with a video source to preview it");
  };

  const seekTo = (value: number) => {
    const nextTime = clamp(value, 0, activeDuration);
    setCurrentTime(nextTime);
    if (videoRef.current && viewerUrl) videoRef.current.currentTime = nextTime;
  };

  const handleMediaPlay = () => setPlaying(true);
  const handleMediaPause = () => setPlaying(false);

  const updateColor = (key: ColorKey, value: number) => {
    if (editsBlocked) return;
    const next = { ...colorRef.current, [key]: value };
    colorRef.current = next;
    setColor(next);
    if (selectedClip) scheduleClipPatch(selectedClip.id, { effects: preserveColorEffects(selectedClip, next) });
  };

  const applyPreset = (preset: (typeof COLOR_PRESETS)[number]) => {
    if (editsBlocked) return;
    colorRef.current = { ...preset.color };
    setColor({ ...preset.color });
    if (selectedClip) scheduleClipPatch(selectedClip.id, { effects: preserveColorEffects(selectedClip, preset.color) });
  };

  const resetColor = () => {
    if (!colorChanged || editsBlocked) return;
    colorRef.current = DEFAULT_COLOR;
    setColor(DEFAULT_COLOR);
    if (selectedClip) scheduleClipPatch(selectedClip.id, { effects: preserveColorEffects(selectedClip, DEFAULT_COLOR) });
  };

  function scheduleClipPatch(clipId: string, patch: JsonObject) {
    if (editsBlocked) return;
    const pending = pendingClipPatches.current;
    pending.set(clipId, { ...(pending.get(clipId) ?? {}), ...patch });
    if (clipPatchTimer.current !== undefined) window.clearTimeout(clipPatchTimer.current);
    clipPatchTimer.current = window.setTimeout(() => {
      clipPatchTimer.current = undefined;
      void commitPendingClipPatch();
    }, 400);
  }

  async function commitPendingClipPatch() {
    if (editsBlocked) {
      pendingClipPatches.current.clear();
      return;
    }
    if (clipPatchBusy.current) return;
    const pending = pendingClipPatches.current.entries().next().value as [string, JsonObject] | undefined;
    if (!pending) return;
    pendingClipPatches.current.delete(pending[0]);
    const currentState = stateRef.current;
    if (!currentState) return;
    clipPatchBusy.current = true;
    try {
      const response = await sendCommand(
        { type: "update_clip", clipId: pending[0], patch: pending[1] },
        currentState.revision,
      );
      const next = stateFromResponse(response) ?? await loadState();
      stateRef.current = next;
      setState(next);
      setPreviewUrl(undefined);
      setToast("Adjustment saved");
    } catch (error) {
      if (error instanceof ApiError && error.isConflict) {
        await refreshState().catch(() => undefined);
        setToast("This adjustment conflicted with another edit. The latest project is loaded.");
      } else {
        setToast(error instanceof Error ? error.message : "Unable to save adjustment");
      }
    } finally {
      clipPatchBusy.current = false;
      if (pendingClipPatches.current.size > 0) {
        if (clipPatchTimer.current !== undefined) window.clearTimeout(clipPatchTimer.current);
        clipPatchTimer.current = window.setTimeout(() => {
          clipPatchTimer.current = undefined;
          void commitPendingClipPatch();
        }, 120);
      }
    }
  }

  const updateMotion = (key: MotionKey, value: number) => {
    if (editsBlocked) return;
    const next = { ...motionRef.current, [key]: value };
    motionRef.current = next;
    setMotion(next);
    if (!selectedClip) return;
    const patch = key === "opacity"
      ? { opacity: value }
      : { transform: preserveTransform(selectedClip, { ...motionRef.current, [key]: value }) };
    scheduleClipPatch(selectedClip.id, patch);
  };

  const updateCrop = (key: CropKey, value: number) => {
    if (editsBlocked) return;
    const next = { ...cropRef.current, [key]: value };
    const safe = {
      ...next,
      x: clamp(next.x, 0, Math.max(0, 1 - next.width)),
      y: clamp(next.y, 0, Math.max(0, 1 - next.height)),
      width: clamp(next.width, 0.05, Math.max(0.05, 1 - next.x)),
      height: clamp(next.height, 0.05, Math.max(0.05, 1 - next.y)),
    };
    cropRef.current = safe;
    setCrop(safe);
    if (selectedClip) scheduleClipPatch(selectedClip.id, { crop: safe });
  };

  const updateVolume = (value: number) => {
    if (editsBlocked) return;
    const safe = clamp(value, 0, 2);
    if (safe > 0) lastVolumeRef.current = safe;
    if (selectedClip) scheduleClipPatch(selectedClip.id, { volume: safe, muted: safe === 0 });
  };

  const toggleMute = () => {
    if (!selectedClip || editsBlocked) return;
    if (audioMuted) {
      const value = Math.max(0.01, lastVolumeRef.current || 1);
      scheduleClipPatch(selectedClip.id, { volume: value, muted: false });
    } else {
      lastVolumeRef.current = Math.max(0.01, audioVolume);
      scheduleClipPatch(selectedClip.id, { volume: 0, muted: true });
    }
  }

  const undo = async () => {
    if (!state?.canUndo || editsBlocked) return;
    try {
      const response = await undoRemote(state.revision);
      setState(stateFromResponse(response) ?? await loadState());
      setPreviewUrl(undefined);
      setToast("Undid the last project change");
    } catch (error) {
      setToast(error instanceof Error ? error.message : "Unable to undo project change");
    }
  };

  const redo = async () => {
    if (!state?.canRedo || editsBlocked) return;
    try {
      const response = await redoRemote(state.revision);
      setState(stateFromResponse(response) ?? await loadState());
      setPreviewUrl(undefined);
      setToast("Redid the project change");
    } catch (error) {
      setToast(error instanceof Error ? error.message : "Unable to redo project change");
    }
  };

  const handleCompare = () => {
    setViewerOptions((current) => updateViewerOptions(current, { showAfter: !current.showAfter }));
    setToast(showAfter ? "Showing original preview" : "Showing edited preview");
  };

  const openOverlay = (overlay: Exclude<ActiveOverlay, null>) => {
    setActiveOverlay((current) => toggleOverlay(current, overlay));
  };

  const closeOverlay = () => setActiveOverlay(null);

  const refreshFromMenu = async () => {
    closeOverlay();
    setLoading(true);
    try {
      await refreshState();
      setToast("Project state refreshed");
    } catch (error) {
      setToast(error instanceof Error ? error.message : "Unable to refresh project state");
    } finally {
      setLoading(false);
    }
  };

  const updateViewerAudio = (value: number) => {
    setViewerAudio((current) => {
      const next = setViewerAudioVolume(current, value);
      if (videoRef.current) {
        videoRef.current.volume = next.volume;
        videoRef.current.muted = next.muted;
      }
      return next;
    });
  };

  const toggleViewerMute = () => {
    setViewerAudio((current) => {
      const next = toggleViewerAudio(current);
      if (videoRef.current) {
        videoRef.current.volume = next.volume;
        videoRef.current.muted = next.muted;
      }
      return next;
    });
  };

  const toggleFullscreen = async () => {
    const stage = viewerStageRef.current;
    if (!stage) return;
    try {
      const fullscreenElement = document.fullscreenElement;
      if (fullscreenAction(fullscreenElement === viewerStageRef.current, Boolean(fullscreenElement)) === "exit") {
        await document.exitFullscreen();
      } else if (stage.requestFullscreen) {
        await stage.requestFullscreen();
      } else {
        setToast("Fullscreen is not supported by this browser");
      }
    } catch (error) {
      setToast(error instanceof Error ? error.message : "Unable to change fullscreen mode");
    } finally {
      // The promise may resolve without the browser actually entering
      // fullscreen (for example when a permission request is denied). Read
      // the DOM again so the button never stays pressed on stale local state.
      setIsFullscreen(document.fullscreenElement === viewerStageRef.current);
      // Some embedded webviews resolve the request and then immediately leave
      // fullscreen without delivering a second fullscreenchange event. Re-read
      // the authoritative DOM state after that transition has settled.
      window.setTimeout(() => {
        setIsFullscreen(document.fullscreenElement === viewerStageRef.current);
      }, 750);
    }
  };

  const clearChat = async () => {
    if (agentBusy || chatResetBusy) return;
    // Incrementing the generation makes an in-flight response harmless to the
    // cleared conversation. The host request is intentionally not cancelled:
    // clearing chat is a UI-only action and must never undo or delete project
    // work that was already submitted.
    setAgentError(null);
    setChatResetBusy(true);
    try {
      await resetAgentSession();
      chatGeneration.current += 1;
      setChatMessages([]);
      setChanges([]);
      setSelectedChange(null);
      setPrompt("");
      setAgentBusy(false);
      setToast("Chat cleared. Project edits are unchanged.");
    } catch (error) {
      const message = error instanceof Error ? error.message : "Unable to clear chat";
      setAgentError(message);
      setToast(message);
    } finally {
      setChatResetBusy(false);
    }
  };

  const handleModelChange = async (event: ChangeEvent<HTMLSelectElement>) => {
    const model = event.target.value;
    if (!model || agentBusy || agentConfigBusy || !agentConfig) return;
    const previous = selectedModel;
    const option = agentConfig.models?.find((candidate) => candidate.id === model);
    setSelectedModel(model);
    setAgentConfigBusy(true);
    setAgentConfigError(null);
    try {
      const next = await updateAgentConfig({
        model,
        ...(option?.provider ?? agentConfig.provider ? { provider: option?.provider ?? agentConfig.provider } : {}),
      });
      setAgentConfig((current) => ({ ...(current ?? {}), ...next, model: next.model ?? model }));
      setSelectedModel(next.model ?? model);
      setToast(`Using ${next.model ?? model}`);
    } catch (error) {
      setSelectedModel(previous);
      const message = error instanceof Error ? error.message : "Unable to change agent model";
      setAgentConfigError(message);
      setToast(message);
    } finally {
      setAgentConfigBusy(false);
    }
  };

  const handleAgentSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const request = prompt.trim();
    if (!request || agentBusy || chatResetBusy || pendingProposal) return;
    const generation = chatGeneration.current;
    const messageId = `chat-${Date.now()}-${chatMessageSequence.current++}`;
    setChatMessages((messages) => [...messages, { id: messageId, role: "user", text: request }]);
    setPrompt("");
    setAgentError(null);
    setAgentBusy(true);
    setToast("Agent is working…");
    try {
      const response = await runAgent(request, state?.revision ?? 0, messageId);
      const eventChanges = Array.isArray(response.events) ? changesFromEvents(response.events) : [];
      const returnedState = stateFromResponse(response.state);
      const returnedProposal = response.proposal ?? returnedState?.pendingProposal;
      const hasStructuredProposal = returnedProposal?.status === "pending";
      const currentState = stateRef.current;
      if (returnedState && currentState && returnedState.revision === currentState.revision) {
        // The agent response is a proposal envelope.  Keep the project object
        // already displayed by the host and only adopt its lifecycle metadata;
        // canonical state changes happen after explicit acceptance.
        const nextState = { ...currentState, ...(returnedProposal ? { pendingProposal: returnedProposal } : {}) };
        stateRef.current = nextState;
        setState(nextState);
      } else if (returnedState && !currentState) {
        stateRef.current = returnedState;
        setState(returnedState);
      } else {
        await refreshState();
      }
      setPreviewUrl(undefined);
      if (generation !== chatGeneration.current) return;
      setChanges(eventChanges);
      setSelectedChange(eventChanges[0]?.id ?? null);
      setChatMessages((messages) => [
        ...messages,
        {
          id: `chat-${Date.now()}-${chatMessageSequence.current++}`,
          role: "assistant",
          text: responseSummary(response, eventChanges.length > 0),
        },
      ]);
      setToast(
        hasStructuredProposal
          ? "Proposal ready for review"
          : eventChanges.length
            ? "Agent completed the requested edit"
            : "Agent completed with no change summary",
      );
    } catch (error) {
      if (generation !== chatGeneration.current) return;
      const message = error instanceof Error ? error.message : "Agent request failed";
      setAgentError(message);
      setChatMessages((messages) => [
        ...messages,
        { id: `chat-${Date.now()}-${chatMessageSequence.current++}`, role: "error", text: message },
      ]);
      setToast(message);
    } finally {
      if (generation === chatGeneration.current) setAgentBusy(false);
    }
  };

  const handleAcceptProposal = async () => {
    const proposal = pendingProposal;
    const current = stateRef.current;
    if (!proposal || !current || proposalBusy) return;
    setProposalBusy(true);
    setAgentError(null);
    setToast("Applying proposal…");
    try {
      const response = await acceptProposal(proposal.proposalId, proposal.baseRevision);
      const next = stateFromResponse(response) ?? await loadState();
      stateRef.current = next;
      setState(next);
      setChanges([]);
      setSelectedChange(null);
      setPreviewUrl(undefined);
      setToast("Proposal applied as one project change");
    } catch (error) {
      if (error instanceof ApiError && error.isConflict) {
        await refreshState().catch(() => undefined);
        setToast("This proposal is stale. The latest project is loaded; ask the agent again.");
      } else {
        setToast(error instanceof Error ? error.message : "Unable to apply proposal");
      }
    } finally {
      setProposalBusy(false);
    }
  };

  const handleRejectProposal = async () => {
    const proposal = pendingProposal;
    if (!proposal || proposalBusy) return;
    setProposalBusy(true);
    try {
      const response = await rejectProposal(proposal.proposalId, proposal.baseRevision);
      const next = stateFromResponse(response) ?? await loadState();
      stateRef.current = next;
      setState(next);
      setChanges([]);
      setSelectedChange(null);
      setToast("Proposal rejected. The project is unchanged.");
    } catch (error) {
      setToast(error instanceof Error ? error.message : "Unable to reject proposal");
    } finally {
      setProposalBusy(false);
    }
  };

  const toggleChange = (id: string) => {
    setSelectedChange(id);
  };

  const applyDirectEdit = async (command: JsonObject, successMessage: string) => {
    if (editsBlocked) return;
    const current = stateRef.current;
    if (!current) return;
    try {
      const response = await sendCommand(command, current.revision);
      const next = stateFromResponse(response) ?? await loadState();
      stateRef.current = next;
      setState(next);
      setPreviewUrl(undefined);
      setToast(successMessage);
    } catch (error) {
      if (error instanceof ApiError && error.isConflict) {
        await refreshState().catch(() => undefined);
        setToast("This edit conflicted with another change. The latest project is loaded.");
      } else {
        setToast(error instanceof Error ? error.message : "Unable to apply edit");
      }
    }
  };

  const deleteSelectedClip = () => {
    if (!selectedClip) return;
    void applyDirectEdit(
      { type: "remove_clip", clipId: selectedClip.id, trackId: selectedClip.trackId },
      "Selected clip deleted",
    );
  };

  const canSplitSelectedClip = Boolean(
    selectedClip
    && currentTime > selectedClip.start + 0.01
    && currentTime < selectedClip.start + selectedClip.clipDuration - 0.01,
  );

  const splitSelectedClip = () => {
    if (!selectedClip || !canSplitSelectedClip || !state) return;
    const track = state.project.tracks.find((candidate) => candidate.id === selectedClip.trackId);
    const clip = track?.clips.find((candidate) => candidate.id === selectedClip.id);
    if (!track || !clip) return;
    const clipIn = numberValue(clip.in, 0);
    const clipOut = numberValue(clip.out, clipIn);
    const speed = Math.max(0.001, numberValue(clip.speed, 1));
    const splitSourceTime = clipIn + (currentTime - selectedClip.start) * speed;
    if (!(splitSourceTime > clipIn && splitSourceTime < clipOut)) return;
    const clipIds = new Set(state.project.tracks.flatMap((candidate) => candidate.clips.map((item) => item.id)));
    let rightId = `${clip.id}-b`;
    let suffix = 2;
    while (clipIds.has(rightId)) rightId = `${clip.id}-b-${suffix++}`;
    const rightClip = {
      ...clip,
      id: rightId,
      timelineStart: currentTime,
      in: splitSourceTime,
      out: clipOut,
    };
    void applyDirectEdit(
      {
        type: "batch",
        commands: [
          { type: "update_clip", clipId: clip.id, patch: { out: splitSourceTime } },
          { type: "add_clip", trackId: track.id, index: track.clips.indexOf(clip) + 1, clip: rightClip },
        ],
      },
      "Selected clip split at the playhead",
    );
  };

  const handleTimelineClick = (event: MouseEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const nextProgress = clamp((event.clientX - rect.left) / rect.width, 0, 1);
    seekTo(nextProgress * activeDuration);
  };

  const pollRender = async (jobId: string, quality: "preview" | "full") => {
    for (;;) {
      const status = await getRender(jobId);
      const normalizedStatus = String(status.status ?? status.state ?? "queued").toLowerCase();
      const done = ["completed", "complete", "done", "succeeded", "success"].includes(normalizedStatus);
      const failed = ["failed", "error", "cancelled", "canceled"].includes(normalizedStatus);
      if (done) {
        const url = resolveAssetUrl(status.outputUrl, jobId, status.outputPath) ?? outputUrl(jobId);
        setRender({ quality, jobId, status: "completed", outputUrl: url });
        if (quality === "preview") setPreviewUrl(url);
        else setExportUrl(url);
        setToast(quality === "preview" ? "Preview render is ready" : "Export is ready to download");
        return;
      }
      if (failed) throw new Error(status.error ?? "Render failed");
      setRender({ quality, jobId, status: normalizedStatus });
      await new Promise((resolve) => window.setTimeout(resolve, 700));
    }
  };

  const handleRender = async (quality: "preview" | "full") => {
    if (pendingProposal) {
      setToast("Rendered output reflects the last accepted state. Accept the proposal before rendering it.");
      return;
    }
    if (render && ["queued", "running", "processing"].includes(render.status)) return;
    setRender({ quality, status: "queued" });
    setToast(quality === "preview" ? "Rendering preview…" : "Rendering full export…");
    try {
      const job = await startRender(quality);
      setRender({ quality, jobId: job.jobId, status: String(job.status ?? "queued") });
      await pollRender(job.jobId, quality);
    } catch (error) {
      setRender({ quality, status: "failed", error: error instanceof Error ? error.message : "Render failed" });
      setToast(error instanceof Error ? error.message : "Render failed");
    }
  };

  const openMediaBin = () => {
    setMediaImportError(null);
    setMediaBinOpen(true);
  };

  const completeMediaImport = async (input: Parameters<typeof importMedia>[0]) => {
    if (mediaImportBusy || editsBlocked) return;
    setMediaImportBusy(true);
    setMediaImportError(null);
    try {
      const response = await importMedia(input, state?.revision ?? 0, `media-${Date.now()}-${Math.random().toString(36).slice(2)}`);
      const previousIds = new Set((state?.sources ?? []).map((candidate) => candidate.id));
      const nextState = stateFromResponse(response.state ?? response) ?? await loadState();
      setState(nextState);
      setPreviewUrl(undefined);
      const importedId = response.sourceId
        ?? response.source?.id
        ?? nextState.sources.find((candidate) => !previousIds.has(candidate.id))?.id
        ?? nextState.sources.at(-1)?.id;
      if (importedId) {
        setSelectedSourceId(importedId);
        const subtitleMetadata = response.subtitle ?? response.subtitles ?? response.captions ?? response.source?.subtitle ?? response.source?.subtitles;
        if (subtitleMetadata !== undefined) {
          setImportedSubtitleMeta((current) => ({ ...current, [importedId]: subtitleMetadata }));
        }
      }
      setMediaUrlInput("");
      if (fileInputRef.current) fileInputRef.current.value = "";
      setToast(importedId ? "Media imported and selected" : "Media imported");
      setMediaBinOpen(false);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Unable to import media";
      setMediaImportError(message);
      setToast(message);
    } finally {
      setMediaImportBusy(false);
    }
  };

  const chooseMediaFile = async () => {
    if (editsBlocked) return;
    if (isDesktopRuntime()) {
      try {
        const path = await pickMediaFile();
        if (path) await completeMediaImport(path);
      } catch (error) {
        setMediaImportError(error instanceof Error ? error.message : "Unable to choose media");
      }
      return;
    }
    fileInputRef.current?.click();
  };

  const handleMediaFile = (event: ChangeEvent<HTMLInputElement>) => {
    if (editsBlocked) return;
    const file = event.target.files?.[0];
    if (file) void completeMediaImport({ file });
  };

  const handleMediaUrlSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!editsBlocked && mediaUrlInput.trim()) void completeMediaImport({ url: mediaUrlInput });
  };

  const appStyle = { "--viewer-filter": showAfter && !previewUrl ? colorFilter : "none" } as CSSProperties;
  const viewerMediaStyle: CSSProperties = showAfter && !previewUrl
    ? {
      filter: "var(--viewer-filter)",
      opacity: motion.opacity,
      width: `${100 / Math.max(0.05, crop.width)}%`,
      height: `${100 / Math.max(0.05, crop.height)}%`,
      left: `${-(crop.x / Math.max(0.05, crop.width)) * 100}%`,
      top: `${-(crop.y / Math.max(0.05, crop.height)) * 100}%`,
      transform: `translate(${motion.x * 100}%, ${motion.y * 100}%) scale(${motion.scale}) rotate(${motion.rotation}deg)`,
      transformOrigin: "center center",
    }
    : { filter: "none", opacity: 1 };
  const baseTimelineTracks = (state?.project.tracks ?? []).filter((track) => {
    const type = String(track.type ?? "video").toLowerCase();
    return timelineExpanded || type === "video" || isSubtitleTrack(track);
  });
  const subtitleOverlays = Array.isArray(state?.project.overlays)
    ? state.project.overlays.filter((overlay) => {
      if (!overlay || typeof overlay !== "object") return false;
      const type = String((overlay as Record<string, unknown>).type ?? (overlay as Record<string, unknown>).kind ?? "").toLowerCase();
      return type.includes("subtitle") || type.includes("caption") || type === "text";
    })
    : [];
  const overlayTracks = subtitleOverlays.length && !baseTimelineTracks.some((track) => isSubtitleTrack(track))
    ? [{
      id: "captions-overlay",
      type: "captions",
      name: "Captions",
      clips: subtitleOverlays.flatMap((overlay, overlayIndex) => {
        const object = overlay as Record<string, unknown>;
        const cues = Array.isArray(object.cues) ? object.cues : Array.isArray(object.entries) ? object.entries : [];
        return cues.map((cue, cueIndex) => {
          const parsed = parseCue(cue);
          return parsed ? { id: `caption-${overlayIndex}-${cueIndex}`, timelineStart: parsed.start, duration: parsed.end - parsed.start, text: parsed.text } : undefined;
        }).filter((clip): clip is NonNullable<typeof clip> => Boolean(clip));
      }),
    } as unknown as (typeof baseTimelineTracks)[number]]
    : [];
  const timelineTracks = [...baseTimelineTracks, ...overlayTracks];

  return (
    <div className="app-shell" style={appStyle}>
      <header className="topbar">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            <span />
            <span />
          </div>
          <div>
            <div className="brand-name">cut<span>·</span></div>
            <div className="brand-subtitle">viewer & verifier</div>
          </div>
        </div>

        <div className="project-context">
          <div className="project-icon"><Icon name="film" size={15} /></div>
          <div>
            <div className="project-name">{projectName}</div>
            <div className="project-meta"><span className={`status-dot ${loadError ? "error" : ""}`} /> {loadError ? "Host unavailable" : loading ? "Loading project" : "Connected"} <span className="meta-divider">·</span> {formatDuration(activeDuration)} <span className="meta-divider">·</span> r{state?.revision ?? 0}</div>
          </div>
          <button className="icon-button subtle" aria-label="Project menu" title="Project menu" onClick={() => openOverlay("project-menu")} aria-expanded={activeOverlay === "project-menu"}><Icon name="chevronDown" size={14} /></button>
        </div>

        <div className="top-actions">
          <button className="icon-button" onClick={() => void undo()} disabled={!state?.canUndo || agentBusy || editsBlocked} aria-label="Undo" title="Undo"><Icon name="undo" /></button>
          <button className="icon-button" onClick={() => void redo()} disabled={!state?.canRedo || agentBusy || editsBlocked} aria-label="Redo" title="Redo"><Icon name="redo" /></button>
          <div className="action-divider" />
          <button className={`toolbar-button ${showAfter ? "" : "active"}`} onClick={handleCompare}><Icon name="compare" size={15} /> <span>Compare</span></button>
          <button className="toolbar-button" onClick={() => void handleRender("preview")} disabled={!state || Boolean(pendingProposal) || Boolean(render && ["queued", "running", "processing"].includes(render.status))}><Icon name="play" size={13} /> <span>{render?.quality === "preview" && render.status !== "completed" ? "Rendering" : "Preview"}</span></button>
          <button className="export-button" onClick={() => void handleRender("full")} disabled={!state || Boolean(pendingProposal) || Boolean(render && ["queued", "running", "processing"].includes(render.status))}><Icon name="download" size={15} /> <span>Export</span></button>
          {exportUrl && <a className="export-link" href={exportUrl} target="_blank" rel="noreferrer" download><Icon name="download" size={13} /> Download</a>}
          <button className="icon-button profile-button" aria-label="Settings" title="Settings" onClick={() => openOverlay("settings")} aria-expanded={activeOverlay === "settings"}><Icon name="settings" size={17} /></button>
        </div>
      </header>

      <main className="workspace">
        {activeRightPanel === "chat" && <aside className="agent-panel panel-surface" aria-label="Cut agent">
          <div className="right-rail-switcher" role="tablist" aria-label="Right rail">
            <button role="tab" aria-selected="true" className="active" onClick={() => setActiveRightPanel("chat")}><Icon name="sparkle" size={13} /> Chat</button>
            <button role="tab" aria-selected="false" onClick={() => setActiveRightPanel("adjust")}><Icon name="sliders" size={13} /> Adjust</button>
          </div>
          <div className="agent-sidebar-toolbar">
            <div className="agent-sidebar-title">
              <span className="agent-sidebar-icon"><Icon name="sparkle" size={13} /></span>
              <strong>Agent</strong>
              <span className={`activity-status ${agentBusy || proposalBusy ? "working" : agentError || loadError ? "error" : ""}`}><span />{agentBusy ? "Working" : pendingProposal ? "Review" : agentError || loadError ? "Error" : "Ready"}</span>
            </div>
            <div className="agent-sidebar-actions">
              <button
                type="button"
                className="icon-button subtle"
                aria-label="New chat"
                title="New chat"
                onClick={() => void clearChat()}
                disabled={chatResetBusy || agentBusy}
              ><Icon name="plus" size={15} /></button>
              <button
                type="button"
                className="clear-chat-button"
                aria-label="Clear chat"
                onClick={() => void clearChat()}
                disabled={chatResetBusy || agentBusy || (!chatMessages.length && !changes.length && !prompt && !agentError)}
                title="Clear this chat. Project files and agent edits are kept."
              >
                {chatResetBusy ? <span className="button-spinner" /> : <Icon name="rotate" size={11} />} <span>{chatResetBusy ? "Clearing…" : "Clear chat"}</span>
              </button>
            </div>
          </div>

          <div className="agent-session-caption">
            <span>Verifying</span>
            <strong title={projectName}>{projectName}</strong>
          </div>

          {hasModelPicker && <div className="agent-model-row">
            <label htmlFor="agent-model">Model</label>
            <select id="agent-model" value={selectedModel} onChange={(event) => void handleModelChange(event)} disabled={agentBusy || agentConfigBusy}>
              {agentModels.length ? agentModels.map((option) => <option key={option.id} value={option.id}>{modelOptionLabel(option)}</option>) : selectedModel && <option value={selectedModel}>{selectedModel}</option>}
            </select>
            {agentConfigBusy && <span className="button-spinner" aria-label="Updating model" />}
          </div>}

          {selectedClip && <div className="chat-context-chips" aria-label="Current editing context">
            <span className="context-chip"><Icon name="scissors" size={10} /> Clip {selectedClip.id}</span>
            {source && <span className="context-chip"><Icon name="film" size={10} /> {source.title ?? source.name ?? source.id}</span>}
          </div>}

          <div className="agent-activity" ref={chatThreadRef}>
            <div className="chat-thread" role="log" aria-live="polite" aria-busy={agentBusy} aria-label="Chat messages">
              {chatMessages.length ? chatMessages.map((message) => (
                <div className={`chat-message ${message.role}`} key={message.id}>
                  <span className="chat-message-avatar">{message.role === "user" ? "You" : message.role === "error" ? "!" : "✦"}</span>
                  <div className="chat-bubble">
                    <span className="chat-message-role">{message.role === "user" ? "You" : message.role === "error" ? "Error" : "Cut agent"}</span>
                    <span className="chat-message-copy">{message.text}</span>
                  </div>
                </div>
              )) : (
                <div className="chat-empty">Instruct the agent to edit the project; use the canvas to verify playback and before/after. Clearing chat does not undo project changes.</div>
              )}
              {agentBusy && (
                <div className="chat-message assistant pending" aria-label="Agent is working">
                  <span className="chat-message-avatar">✦</span>
                  <div className="chat-bubble"><span className="chat-message-role">Cut agent</span><span className="chat-message-copy typing-indicator"><i /><i /><i /></span></div>
                </div>
              )}
            </div>
            {pendingProposal ? <section className="proposal-review" aria-label="Pending agent proposal">
              <div className="activity-subheading"><span className="section-label">Review proposal</span><span>{pendingProposal.diff.length} change{pendingProposal.diff.length === 1 ? "" : "s"} · r{pendingProposal.baseRevision}</span></div>
              <p className="proposal-intro">The project is unchanged. Review these structured edits before applying them.</p>
              <div className="proposal-change-list">
                {pendingProposal.diff.length ? pendingProposal.diff.map((change, index) => (
                  <div className="proposal-change-card" key={`${change.path}-${index}`}>
                    <span className={`change-icon ${proposalChangeTone(change)}`}><Icon name={proposalChangeIcon(change)} size={14} /></span>
                    <span className="change-copy">
                      <span className="change-title">{proposalChangeTitle(change)}</span>
                      <span className="change-detail">{proposalChangeDetail(change)}</span>
                      <span className="change-range"><Icon name="clock" size={10} /> {change.path}</span>
                    </span>
                  </div>
                )) : <div className="proposal-empty">The agent returned a valid batch with no observable value changes.</div>}
              </div>
              {pendingProposal.diffTruncated && <p className="proposal-truncated">Showing the first 128 changes. The full batch is still validated by the runtime.</p>}
              <div className="proposal-actions">
                <button type="button" className="proposal-reject-button" onClick={() => void handleRejectProposal()} disabled={proposalBusy} aria-label="Reject proposal"><Icon name="x" size={13} /> Reject</button>
                <button type="button" className="proposal-accept-button" onClick={() => void handleAcceptProposal()} disabled={proposalBusy} aria-label="Accept proposal">{proposalBusy ? <span className="button-spinner" /> : <Icon name="plus" size={13} />} {proposalBusy ? "Applying…" : "Accept proposal"}</button>
              </div>
            </section> : changes.length > 0 && <>
              <div className="activity-subheading"><span className="section-label">Changes</span><span>{changes.length} update{changes.length === 1 ? "" : "s"}</span></div>
              <div className="change-list">
                {changes.map((change) => {
                  const isSelected = selectedChange === change.id;
                  return (
                    <button
                      className={`change-card ${isSelected ? "selected" : ""}`}
                      key={change.id}
                      onClick={() => toggleChange(change.id)}
                      aria-pressed={isSelected}
                    >
                      <span className={`change-icon ${change.tone}`}><Icon name={change.icon} size={15} /></span>
                      <span className="change-copy">
                        <span className="change-title">{change.title}</span>
                        <span className="change-detail">{change.detail}</span>
                        <span className="change-range"><Icon name="clock" size={11} /> {change.range}</span>
                      </span>
                      <Icon name="chevronRight" size={14} />
                    </button>
                  );
                })}
              </div>
            </>}
          </div>

          <form className="agent-composer" onSubmit={handleAgentSubmit}>
            <label className="visually-hidden" htmlFor="agent-prompt">Message the Cut agent</label>
            <textarea
              ref={promptInputRef}
              id="agent-prompt"
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.nativeEvent.isComposing && !event.shiftKey && !agentBusy && !chatResetBusy && !pendingProposal && prompt.trim() && state) {
                  event.preventDefault();
                  event.currentTarget.form?.requestSubmit();
                }
              }}
              placeholder={pendingProposal ? "Review the pending proposal before sending another request…" : "Tell the agent what to change in the project…"}
              rows={1}
              disabled={agentBusy || chatResetBusy || Boolean(pendingProposal)}
            />
            <div className="composer-footer">
              <div className="composer-footer-tools">
                <span className="composer-hint">Enter to send <span aria-hidden="true">·</span> Shift+Enter for newline</span>
              </div>
              <button className="send-button" type="submit" disabled={!prompt.trim() || agentBusy || Boolean(pendingProposal) || !state} aria-label="Send request to Cut agent" title="Send request to Cut agent">
                {agentBusy ? <span className="button-spinner" /> : <Icon name="send" size={14} />}
              </button>
            </div>
          </form>
        </aside>}

        <section className="viewer-column">
          <div className="viewer-toolbar">
            <div className="viewer-toolbar-left">
              <button className="back-button" aria-label="Back to project actions" title="Back to project actions" onClick={() => openOverlay("project-menu")} aria-expanded={activeOverlay === "project-menu"}><Icon name="chevronLeft" size={17} /></button>
              <span className="toolbar-path">My projects <span>/</span> <strong>{projectName}</strong></span>
            </div>
            <div className="viewer-toolbar-right">
              <span className="canvas-pill"><span className="canvas-dot" /> Canvas {state?.project.width && state.project.height ? `${state.project.width}×${state.project.height}` : "project"}</span>
              <button className="icon-button" aria-label="Viewer options" title="Viewer options" onClick={() => openOverlay("viewer-options")} aria-expanded={activeOverlay === "viewer-options"}><Icon name="sliders" size={16} /></button>
            </div>
          </div>

          <div className="viewer-card panel-surface">
            <div className={`viewer-stage ${isFullscreen ? "fullscreen-active" : ""}`} ref={viewerStageRef} onDoubleClick={togglePlayback}>
              {viewerUrl ? (
                <video
                  ref={videoRef}
                  key={viewerUrl}
                  className="video-element"
                  src={viewerUrl}
                  style={viewerMediaStyle}
                  muted={viewerAudio.muted}
                  onTimeUpdate={() => setCurrentTime(videoRef.current?.currentTime ?? 0)}
                  onPlay={handleMediaPlay}
                  onPause={handleMediaPause}
                  onEnded={() => setPlaying(false)}
                  aria-label={!pendingProposal && showAfter && previewUrl ? "Rendered project preview" : showAfter ? "Live approximation of project source" : "Project source preview"}
                  playsInline
                  controls={false}
                />
              ) : (
                <div className="empty-media" aria-label="No video source">
                  <span className="empty-inspector-icon"><Icon name={loading ? "clock" : "film"} size={22} /></span>
                  <strong>{loading ? "Loading project…" : loadError ? "Connect to the Cut host" : "No video source in this project"}</strong>
                  <span>{loadError ?? "Ask the agent to add media or open a project with a video track."}</span>
                  {!loading && !loadError && <button className="empty-media-import" onClick={openMediaBin}><Icon name="folder" size={13} /> Add media</button>}
                  {loadError && <button className="empty-media-retry" onClick={() => { setLoading(true); void refreshState().finally(() => setLoading(false)); }}>Retry connection</button>}
                </div>
              )}

              {captionsVisible && activeCaption && <div className="caption-overlay" role="status" aria-live="polite">{activeCaption}</div>}

              <div className="viewer-top-overlay">
                <span className="preview-badge"><span /> {!pendingProposal && showAfter && previewUrl ? "Rendered output" : showAfter ? pendingProposal ? "Live approximation · proposal pending" : "Live approximation" : "Original source"}</span>
                <span className="viewer-top-meta"><span className="viewer-resolution">{state?.project.width && state.project.height ? `${state.project.width} × ${state.project.height}` : "PROJECT"}</span>{source && subtitleInfoForSource(source, importedSubtitleMeta[source.id]).available && <span className="subtitle-badge"><Icon name="compare" size={10} /> CC</span>}</span>
              </div>

              <div className="viewer-controls">
                <button className="control-button" onClick={togglePlayback} aria-label={playing ? "Pause" : "Play"} title={playing ? "Pause" : "Play"}><Icon name={playing ? "pause" : "play"} size={16} /></button>
                <button className="control-button secondary" onClick={() => seekTo(currentTime - 5)} aria-label="Back five seconds" title="Back five seconds"><Icon name="chevronLeft" size={15} /></button>
                <button className="control-button secondary" onClick={() => seekTo(currentTime + 5)} aria-label="Forward five seconds" title="Forward five seconds"><Icon name="chevronRight" size={15} /></button>
                <span className="control-time"><strong>{formatTime(currentTime)}</strong><span> / {formatDuration(activeDuration)}</span></span>
                <div className="control-spacer" />
                <button className={`compare-pill ${showAfter ? "active" : ""}`} onClick={handleCompare}><span className="compare-dot" /> {showAfter ? "After" : "Before"}</button>
                <button className={`control-button secondary ${activeOverlay === "audio-controls" ? "active" : ""}`} onClick={() => openOverlay("audio-controls")} aria-label="Audio controls" title="Audio controls" aria-expanded={activeOverlay === "audio-controls"}><Icon name={viewerAudio.muted ? "close" : "volume"} size={16} /></button>
                <button className={`control-button secondary ${isFullscreen ? "active" : ""}`} onClick={() => void toggleFullscreen()} aria-label={isFullscreen ? "Exit fullscreen" : "Fullscreen"} title={isFullscreen ? "Exit fullscreen" : "Fullscreen"} aria-pressed={isFullscreen}><Icon name="arrowUp" size={15} /></button>
              </div>
            </div>
            <div className="viewer-footer">
              <div className="media-state"><span className="state-icon"><Icon name="film" size={12} /></span><span><strong>{source?.id ?? "No source"}</strong> · {!pendingProposal && previewUrl && showAfter ? "Rendered output" : showAfter ? "Source media · live approximation" : "Source media"}{source && subtitleInfoForSource(source, importedSubtitleMeta[source.id]).available ? " · subtitles ready" : ""}</span></div>
              <button className="load-media-button" onClick={openMediaBin}><Icon name="folder" size={14} /> Manage media</button>
            </div>
          </div>

          <div className={`timeline-panel panel-surface ${timelineExpanded ? "expanded" : ""}`}>
            <div className="timeline-header">
              <div className="timeline-title"><Icon name="waveform" size={15} /><strong>Timeline</strong><span className="timeline-mode">{pendingProposal ? "proposal pending · read-only" : "editable · seek & select"}</span></div>
              <div className="timeline-actions"><span className="edit-count"><span /> {timelineTracks.length} tracks · r{state?.revision ?? 0}</span><button type="button" className="timeline-correction-button" onClick={deleteSelectedClip} disabled={!selectedClip || editsBlocked} aria-label="Delete selected clip" title={pendingProposal ? "Accept or reject the proposal first" : "Delete selected clip"}><Icon name="x" size={12} /> Delete</button><button type="button" className="timeline-correction-button" onClick={splitSelectedClip} disabled={!canSplitSelectedClip || editsBlocked} aria-label="Split selected clip at playhead" title={pendingProposal ? "Accept or reject the proposal first" : "Split selected clip at playhead"}><Icon name="split" size={12} /> Split</button><button type="button" className="expand-button" onClick={() => setTimelineExpanded((value) => !value)}><Icon name={timelineExpanded ? "arrowDown" : "arrowUp"} size={13} /> {timelineExpanded ? "Collapse" : "Expand"}</button></div>
            </div>
            <div className="timeline-body">
              <div className="timeline-ruler" aria-hidden="true">
                {[0, 25, 50, 75, 100].map((tick) => <span key={tick} style={{ left: `${tick}%` }}>{formatTime((tick / 100) * activeDuration)}</span>)}
              </div>
              <div className="timeline-track-stack">
                {timelineTracks.length ? timelineTracks.map((track, trackIndex) => {
                  const trackType = String(track.type ?? "video").toLowerCase();
                  const trackClips = (Array.isArray(track.clips) ? track.clips : []).map((clip) => {
                    const clipSource = sourceById.get(clipSourceId(clip) ?? "");
                    return { clip, start: clipTimelineStart(clip), duration: clipLength(clip, clipSource) };
                  });
                  const subtitleTrack = isSubtitleTrack(track);
                  const tone = trackType === "video" ? "violet" : trackType === "audio" ? "amber" : subtitleTrack ? "green" : "blue";
                  return (
                    <div className="timeline-row" key={track.id}>
                      <div className="track-label"><Icon name={trackType === "audio" ? "mic" : trackType === "video" ? "film" : subtitleTrack ? "compare" : "sliders"} size={13} /><span>{track.name ?? track.id}</span></div>
                      <div className={`timeline-track ${trackType === "audio" ? "audio-track" : subtitleTrack ? "subtitle-track" : "video-track"}`} onClick={handleTimelineClick} role="slider" aria-label={`${track.name ?? track.id} read-only timeline map`} aria-valuemin={0} aria-valuemax={activeDuration} aria-valuenow={currentTime} tabIndex={trackIndex === 0 ? 0 : -1}>
                        {trackClips.map(({ clip, start, duration }) => <span className={`timeline-clip ${tone} ${trackType === "video" && selectedClipId === clip.id ? "selected" : ""}`} key={clip.id} onClick={(event) => { event.stopPropagation(); if (trackType === "video") { setSelectedClipId(clip.id); const id = clipSourceId(clip); if (id) setSelectedSourceId(id); } }} style={{ left: `${activeDuration ? (start / activeDuration) * 100 : 0}%`, width: `${activeDuration ? Math.max(0.5, (duration / activeDuration) * 100) : 0}%` }} title={clip.text ?? clip.id}><span>{clip.text ?? clip.id}</span></span>)}
                        {trackIndex === 0 && <span className="playhead" style={{ left: `${progress}%` }}><span /></span>}
                      </div>
                    </div>
                  );
                }) : <div className="empty-timeline">No timeline tracks to map. Edits go through the agent or API.</div>}
              </div>
              <input className="timeline-scrubber" type="range" min={0} max={Math.max(activeDuration, 0.01)} step={0.01} value={Math.min(currentTime, Math.max(activeDuration, 0.01))} onChange={(event) => seekTo(Number(event.target.value))} aria-label="Seek read-only timeline map" disabled={!activeDuration} />
            </div>
          </div>
        </section>

        {activeRightPanel === "adjust" && <aside className="inspector-panel panel-surface">
          <div className="right-rail-switcher" role="tablist" aria-label="Right rail">
            <button role="tab" aria-selected="false" onClick={() => setActiveRightPanel("chat")}><Icon name="sparkle" size={13} /> Chat</button>
            <button role="tab" aria-selected="true" className="active" onClick={() => setActiveRightPanel("adjust")}><Icon name="sliders" size={13} /> Adjust</button>
          </div>
          <div className="inspector-heading">
            <div><div className="eyebrow">Inspector</div><h2>Verify the grade.</h2></div>
            <button className="icon-button subtle" aria-label="Close inspector" title="Close inspector" onClick={() => setActiveRightPanel("chat")}><Icon name="close" size={15} /></button>
          </div>
          <div className="inspector-tabs" role="tablist" aria-label="Inspector sections">
            {[{ label: "Color", icon: "sparkle" as IconName }, { label: "Motion", icon: "sliders" as IconName }, { label: "Audio", icon: "volume" as IconName }, { label: "Notes", icon: "clock" as IconName }].map((tab) => (
              <button key={tab.label} role="tab" aria-selected={activeInspectorTab === tab.label} className={activeInspectorTab === tab.label ? "active" : ""} onClick={() => setActiveInspectorTab(tab.label)}><Icon name={tab.icon} size={13} /><span>{tab.label}</span></button>
            ))}
          </div>

          {activeInspectorTab === "Color" ? (
            <div className="inspector-content">
              <div className="inspector-section color-overview">
                <div className="section-heading-row"><div><span className="section-label">Color pass · {selectedClip?.id ?? "No video clip"}</span><strong>{colorChanged ? "Custom adjustment" : "Natural baseline"}</strong></div><button className={`before-after-toggle ${showAfter ? "active" : ""}`} onClick={handleCompare}><span className="toggle-knob" /> {showAfter ? "After" : "Before"}</button></div>
                <div className="before-after-preview"><div className="mini-preview original" /><div className={`mini-preview edited ${showAfter ? "visible" : ""}`} style={{ filter: colorFilter }} /><span className="preview-split" /><span className="mini-label left">BEFORE</span><span className="mini-label right">AFTER</span></div><span className="preview-truth-label">{pendingProposal ? "Live approximation · proposal not applied" : previewUrl ? "Rendered output" : "Live approximation · source media"}</span>
              </div>

              <div className="inspector-section">
                <div className="section-heading-row"><span className="section-label">Quick looks</span><button className="text-button" onClick={resetColor} disabled={!colorChanged || editsBlocked}>Reset</button></div>
                <div className="preset-grid">
                  {COLOR_PRESETS.map((preset) => <button key={preset.name} className={`preset-card ${JSON.stringify(color) === JSON.stringify(preset.color) ? "selected" : ""}`} onClick={() => applyPreset(preset)} disabled={editsBlocked}><span className={`preset-swatch ${preset.name.toLowerCase()}`} /><span><strong>{preset.name}</strong><small>{preset.description}</small></span></button>)}
                </div>
              </div>

              <div className="inspector-section sliders-section">
                <div className="section-heading-row"><span className="section-label">Fine tune</span><span className="range-hint">Selected clip · auto-saved</span></div>
                <div className="color-sliders">
                  {COLOR_CONTROLS.map((control) => (
                    <label className="slider-row" key={control.key}>
                      <span className="slider-label"><span>{control.label}</span><output>{control.format(color[control.key])}</output></span>
                      <input type="range" min={control.min} max={control.max} step={control.step} value={color[control.key]} onChange={(event) => updateColor(control.key, Number(event.target.value))} aria-label={control.label} disabled={!selectedClip || !state || editsBlocked} />
                    </label>
                  ))}
                </div>
              </div>

              <div className="color-scope-card"><div className="scope-header"><span><Icon name="waveform" size={13} /> Scope preview</span><span>Histogram</span></div><div className="scope-graph"><span className="scope-line line-one" /><span className="scope-line line-two" /><span className="scope-line line-three" /><i /><i /><i /><i /></div></div>
            </div>
          ) : activeInspectorTab === "Motion" ? (
            <MotionInspector motion={motion} crop={crop} motionChanged={motionChanged} cropChanged={cropChanged} selectedClip={selectedClip} state={state} blocked={editsBlocked} onChange={updateMotion} onCropChange={updateCrop} onReset={() => {
              (Object.keys(DEFAULT_MOTION) as MotionKey[]).forEach((key) => updateMotion(key, DEFAULT_MOTION[key]));
              (Object.keys(DEFAULT_CROP) as CropKey[]).forEach((key) => updateCrop(key, DEFAULT_CROP[key]));
            }} />
          ) : activeInspectorTab === "Audio" ? (
            <AudioInspector volume={audioVolume} muted={audioMuted} selectedClip={selectedClip} state={state} blocked={editsBlocked} onChange={updateVolume} onToggleMute={toggleMute} />
          ) : (
            <div className="empty-inspector"><span className="empty-inspector-icon"><Icon name="clock" size={20} /></span><strong>Notes stay in the chat context.</strong><p>Select Chat to describe an edit, or choose another Adjust tab to tune the selected clip.</p><button onClick={() => setActiveRightPanel("chat")}>Open Chat</button></div>
          )}
        </aside>}
      </main>

      {activeOverlay && <div
        className="ui-overlay-backdrop"
        role="presentation"
        onMouseDown={(event) => { if (event.target === event.currentTarget) closeOverlay(); }}
      >
        {activeOverlay === "project-menu" && <section className="ui-overlay-panel project-menu-panel" role="menu" aria-label="Project actions">
          <div className="ui-overlay-heading"><div><span className="eyebrow">Project</span><h2>{projectName}</h2></div><button type="button" className="icon-button subtle" aria-label="Close project menu" title="Close project menu" onClick={closeOverlay}><Icon name="close" size={15} /></button></div>
          <div className="project-menu-actions">
            <button type="button" role="menuitem" onClick={() => { closeOverlay(); openMediaBin(); }}><Icon name="folder" size={15} /><span><strong>Manage media</strong><small>Import and select project sources</small></span><Icon name="chevronRight" size={13} /></button>
            <button type="button" role="menuitem" onClick={() => void refreshFromMenu()} disabled={loading}><Icon name="rotate" size={15} /><span><strong>{loading ? "Refreshing…" : "Refresh state"}</strong><small>Reload the latest project from the host</small></span><Icon name="chevronRight" size={13} /></button>
            <button type="button" role="menuitem" onClick={() => { closeOverlay(); void handleRender("preview"); }} disabled={!state || Boolean(pendingProposal) || Boolean(render && ["queued", "running", "processing"].includes(render.status))}><Icon name="play" size={15} /><span><strong>Preview</strong><small>{pendingProposal ? "Accept the proposal before rendering" : "Render a quick review of this project"}</small></span><Icon name="chevronRight" size={13} /></button>
            <button type="button" role="menuitem" onClick={() => { closeOverlay(); void handleRender("full"); }} disabled={!state || Boolean(pendingProposal) || Boolean(render && ["queued", "running", "processing"].includes(render.status))}><Icon name="download" size={15} /><span><strong>Export</strong><small>{pendingProposal ? "Accept the proposal before rendering" : "Render the full project for download"}</small></span><Icon name="chevronRight" size={13} /></button>
          </div>
        </section>}

        {activeOverlay === "settings" && <section className="ui-overlay-panel settings-panel" role="dialog" aria-modal="true" aria-labelledby="settings-title">
          <div className="ui-overlay-heading"><div><span className="eyebrow"><Icon name="settings" size={11} /> Preferences</span><h2 id="settings-title">Settings</h2></div><button type="button" className="icon-button subtle" aria-label="Close settings" title="Close settings" onClick={closeOverlay}><Icon name="close" size={15} /></button></div>
          <div className="settings-section">
            <span className="section-label">Agent configuration</span>
            <p className="settings-help">Choose the model used by Chat. Changes are saved through the active Cut host.</p>
            {agentConfig ? <>
              <div className="settings-row"><span>Provider</span><strong>{agentConfig.provider ?? "Default"}</strong></div>
              {hasModelPicker ? <label className="settings-model-row" htmlFor="settings-agent-model"><span>Model</span><select id="settings-agent-model" value={selectedModel} onChange={(event) => void handleModelChange(event)} disabled={agentBusy || agentConfigBusy}>{agentModels.length ? agentModels.map((option) => <option key={option.id} value={option.id}>{modelOptionLabel(option)}</option>) : selectedModel && <option value={selectedModel}>{selectedModel}</option>}</select>{agentConfigBusy && <span className="button-spinner" aria-label="Updating model" />}</label> : <div className="settings-row"><span>Model</span><strong>{agentConfig.model ?? "Host default"}</strong></div>}
              {agentConfigError && <div className="settings-error" role="alert">{agentConfigError}</div>}
            </> : <div className="settings-empty" role={agentConfigError ? "alert" : undefined}>{agentConfigError ?? "Loading agent configuration…"}</div>}
          </div>
        </section>}

        {activeOverlay === "viewer-options" && <section className="ui-overlay-panel viewer-options-panel" role="dialog" aria-modal="true" aria-labelledby="viewer-options-title">
          <div className="ui-overlay-heading"><div><span className="eyebrow"><Icon name="sliders" size={11} /> Canvas</span><h2 id="viewer-options-title">Viewer options</h2></div><button type="button" className="icon-button subtle" aria-label="Close viewer options" title="Close viewer options" onClick={closeOverlay}><Icon name="close" size={15} /></button></div>
          <div className="viewer-option-group"><span className="section-label">Preview</span><div className="viewer-option-buttons" role="group" aria-label="Preview version"><button type="button" className={!showAfter ? "selected" : ""} aria-pressed={!showAfter} onClick={() => setViewerOptions((current) => updateViewerOptions(current, { showAfter: false }))}>Before</button><button type="button" className={showAfter ? "selected" : ""} aria-pressed={showAfter} onClick={() => setViewerOptions((current) => updateViewerOptions(current, { showAfter: true }))}>After</button></div></div>
          <div className="viewer-option-group"><span className="section-label">Overlays</span><label className="viewer-option-check"><input type="checkbox" checked={captionsVisible} onChange={(event) => setViewerOptions((current) => updateViewerOptions(current, { captionsVisible: event.target.checked }))} /> <span>Show captions</span></label><small>Uses captions already present in the project.</small></div>
        </section>}

        {activeOverlay === "audio-controls" && <section className="ui-overlay-panel audio-controls-panel" role="dialog" aria-modal="true" aria-labelledby="audio-controls-title">
          <div className="ui-overlay-heading"><div><span className="eyebrow"><Icon name="volume" size={11} /> Playback</span><h2 id="audio-controls-title">Audio controls</h2></div><button type="button" className="icon-button subtle" aria-label="Close audio controls" title="Close audio controls" onClick={closeOverlay}><Icon name="close" size={15} /></button></div>
          <div className="audio-control-row"><button type="button" className={`mute-button ${viewerAudio.muted ? "active" : ""}`} onClick={toggleViewerMute}><Icon name={viewerAudio.muted ? "close" : "volume"} size={13} /> {viewerAudio.muted ? "Unmute" : "Mute"}</button><output>{Math.round(viewerAudio.volume * 100)}%</output></div>
          <label className="viewer-volume-slider"><span className="section-label">Playback volume</span><input type="range" min={0} max={1} step={0.01} value={viewerAudio.volume} onChange={(event) => updateViewerAudio(Number(event.target.value))} aria-label="Playback volume" /></label>
          <p className="audio-controls-help">Playback only · clip volume and project state are unchanged.</p>
        </section>}
      </div>}

      {mediaBinOpen && <div className="media-modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget && !mediaImportBusy) setMediaBinOpen(false); }}>
        <section className="media-modal panel-surface" role="dialog" aria-modal="true" aria-labelledby="media-modal-title">
          <div className="media-modal-header">
            <div><span className="eyebrow"><Icon name="folder" size={11} /> Media bin</span><h2 id="media-modal-title">Manage media</h2><p>Import a local video or paste a YouTube link. Imported sources stay separate from your timeline selection.</p></div>
            <button className="icon-button subtle" aria-label="Close media manager" title="Close media manager" onClick={() => setMediaBinOpen(false)} disabled={mediaImportBusy}><Icon name="close" size={15} /></button>
          </div>

          <div className="media-import-options">
            <div className="media-import-card">
              <span className="media-import-icon"><Icon name="upload" size={17} /></span>
              <div><strong>Upload a file</strong><small>MP4, MOV, WebM and other supported formats</small></div>
              <input ref={fileInputRef} className="visually-hidden" type="file" accept="video/*,.mkv,.webm" onChange={handleMediaFile} disabled={mediaImportBusy} />
              <button type="button" className="media-primary-button" onClick={() => void chooseMediaFile()} disabled={mediaImportBusy || editsBlocked}>{mediaImportBusy ? <span className="button-spinner" /> : <Icon name="folder" size={13} />} Choose file</button>
            </div>
            <form className="media-import-card url-import-card" onSubmit={handleMediaUrlSubmit}>
              <span className="media-import-icon youtube-mark">▶</span>
              <div><strong>Import from YouTube</strong><small>Video and available ko/en subtitles are fetched together</small></div>
              <div className="media-url-row"><label className="visually-hidden" htmlFor="media-url">YouTube video URL</label><input id="media-url" type="url" value={mediaUrlInput} onChange={(event) => setMediaUrlInput(event.target.value)} placeholder="https://youtube.com/watch?v=…" disabled={mediaImportBusy || editsBlocked} /><button type="submit" className="media-primary-button" disabled={mediaImportBusy || editsBlocked || !mediaUrlInput.trim()}>{mediaImportBusy ? <span className="button-spinner" /> : <Icon name="download" size={13} />} Import</button></div>
            </form>
          </div>

          {mediaImportError && <div className="media-import-error" role="alert"><Icon name="x" size={13} /> {mediaImportError}</div>}

          <div className="media-bin-heading"><span className="section-label">Sources</span><span>{state?.sources.length ?? 0} imported</span></div>
          <div className="media-source-list" role="listbox" aria-label="Imported media sources">
            {(state?.sources ?? []).length ? (state?.sources ?? []).map((candidate) => {
              const subtitles = subtitleInfoForSource(candidate, importedSubtitleMeta[candidate.id]);
              const selected = candidate.id === source?.id;
              return <button type="button" role="option" aria-selected={selected} className={`media-source-item ${selected ? "selected" : ""}`} key={candidate.id} onClick={() => { setSelectedSourceId(candidate.id); setPreviewUrl(undefined); }}>
                <span className="media-source-thumb"><Icon name="film" size={16} /></span>
                <span className="media-source-copy"><strong>{candidate.title ?? candidate.name ?? candidate.id}</strong><small>{candidate.path ?? candidate.url ?? candidate.id}</small><span className="media-source-meta">{candidate.duration ? formatDuration(candidate.duration) : "Duration pending"}{subtitles.available ? <em><Icon name="compare" size={10} /> {subtitles.label ? `${subtitles.label} subtitles` : "Subtitles available"}</em> : <em className="muted">No subtitles</em>}</span></span>
                {selected && <span className="media-source-check" aria-hidden="true">✓</span>}
              </button>;
            }) : <div className="media-bin-empty"><Icon name="film" size={19} /><strong>No media yet</strong><span>Choose a file or paste a YouTube URL above to start the project.</span></div>}
          </div>
        </section>
      </div>}

      {toast && <div className="toast" role="status" aria-live="polite"><span className="toast-icon"><Icon name="sparkle" size={13} /></span>{toast}</div>}
    </div>
  );
}

export default App;
