import { z } from "zod";
import {
  MarkerSchema,
  OverlayV2Schema,
  ProjectSchema,
  ProjectSourceSchema,
  TimelineClipSchema,
  TrackV2Schema,
  type Marker,
  type OverlayV2,
  type Project,
  type ProjectSource,
  type TimelineClip,
  type TrackV2,
} from "./v2.js";

const AddTrackCommandSchema: z.ZodTypeAny = z
  .object({ type: z.literal("add_track"), track: TrackV2Schema })
  .passthrough();
const RemoveTrackCommandSchema = z
  .object({ type: z.literal("remove_track"), trackId: z.string().min(1) })
  .passthrough();
const AddClipCommandSchema = z
  .object({
    type: z.literal("add_clip"),
    trackId: z.string().min(1),
    clip: TimelineClipSchema,
    index: z.number().int().nonnegative().optional(),
  })
  .passthrough();
const RemoveClipCommandSchema = z
  .object({
    type: z.literal("remove_clip"),
    clipId: z.string().min(1),
    trackId: z.string().min(1).optional(),
  })
  .passthrough();
const MoveClipCommandSchema = z
  .object({
    type: z.literal("move_clip"),
    clipId: z.string().min(1),
    trackId: z.string().min(1).optional(),
    toTrackId: z.string().min(1).optional(),
    index: z.number().int().nonnegative().optional(),
    toIndex: z.number().int().nonnegative().optional(),
  })
  .passthrough()
  .superRefine((command, ctx) => {
    if (command.index === undefined && command.toIndex === undefined) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["index"],
        message: "move_clip requires index or toIndex",
      });
    }
  });
const UpdateClipCommandSchema = z
  .object({
    type: z.literal("update_clip"),
    clipId: z.string().min(1),
    patch: z.record(z.unknown()),
  })
  .passthrough();
const SetClipCommandSchema = z
  .object({
    type: z.literal("set_clip"),
    clipId: z.string().min(1),
    clip: TimelineClipSchema,
  })
  .passthrough();
const AddMarkerCommandSchema = z
  .object({ type: z.literal("add_marker"), marker: MarkerSchema })
  .passthrough();
const RemoveMarkerCommandSchema = z
  .object({ type: z.literal("remove_marker"), markerId: z.string().min(1) })
  .passthrough();
const SetMetadataCommandSchema = z
  .object({ type: z.literal("set_metadata"), patch: z.record(z.unknown()) })
  .passthrough();
const AddSourceCommandSchema = z
  .object({ type: z.literal("add_source"), source: ProjectSourceSchema })
  .passthrough();
const AddOverlayCommandSchema = z
  .object({ type: z.literal("add_overlay"), overlay: OverlayV2Schema })
  .passthrough()
  .superRefine((command, ctx) => {
    if (!command.overlay.id) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ["overlay", "id"],
        message: "add_overlay requires overlay.id",
      });
    }
  });
const RemoveOverlayCommandSchema = z
  .object({ type: z.literal("remove_overlay"), overlayId: z.string().min(1) })
  .passthrough();

// A regular union is used rather than z.discriminatedUnion because
// MoveClipCommandSchema has a refinement effect.  Runtime behavior remains
// discriminated by `type`, while all branch schemas are still exact about
// their operation payloads.
const CommandWithoutBatchSchema: z.ZodTypeAny = z.union([
  AddTrackCommandSchema,
  RemoveTrackCommandSchema,
  AddClipCommandSchema,
  RemoveClipCommandSchema,
  MoveClipCommandSchema,
  UpdateClipCommandSchema,
  SetClipCommandSchema,
  AddMarkerCommandSchema,
  RemoveMarkerCommandSchema,
  SetMetadataCommandSchema,
  AddSourceCommandSchema,
  AddOverlayCommandSchema,
  RemoveOverlayCommandSchema,
]);

type CommandWithoutBatch =
  | { type: "add_track"; track: TrackV2 }
  | { type: "remove_track"; trackId: string }
  | { type: "add_clip"; trackId: string; clip: TimelineClip; index?: number }
  | { type: "remove_clip"; clipId: string; trackId?: string }
  | {
      type: "move_clip";
      clipId: string;
      trackId?: string;
      toTrackId?: string;
      index?: number;
      toIndex?: number;
    }
  | { type: "update_clip"; clipId: string; patch: Record<string, unknown> }
  | { type: "set_clip"; clipId: string; clip: TimelineClip }
  | { type: "add_marker"; marker: Marker }
  | { type: "remove_marker"; markerId: string }
  | { type: "set_metadata"; patch: Record<string, unknown> }
  | { type: "add_source"; source: ProjectSource }
  | { type: "add_overlay"; overlay: OverlayV2 }
  | { type: "remove_overlay"; overlayId: string };

const BatchCommandSchema: z.ZodTypeAny = z.object({
  type: z.literal("batch"),
  commands: z.lazy(() => z.array(ProjectCommandSchema)),
});

const CanonicalProjectCommandSchema: z.ZodTypeAny = z.union([
  CommandWithoutBatchSchema,
  BatchCommandSchema,
]);

/**
 * Accept the canonical `type` spelling and the `op` spelling commonly used by
 * v1 EditOp callers. The normalizer is small: command payloads
 * remain JSON-shaped and are checked by the command-specific schemas below.
 */
function normalizeCommand(value: unknown): unknown {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return value;
  const raw = value as Record<string, unknown>;
  const normalized: Record<string, unknown> = { ...raw };
  if (normalized.type === undefined && typeof normalized.command === "string") {
    normalized.type = normalized.command;
  }
  if (normalized.type === undefined && typeof normalized.kind === "string") {
    normalized.type = normalized.kind;
  }
  if (normalized.type === undefined && typeof normalized.op === "string") {
    normalized.type = normalized.op;
  }
  const commandAliases: Record<string, string> = {
    addClip: "add_clip",
    removeClip: "remove_clip",
    deleteClip: "remove_clip",
    insertClip: "add_clip",
    moveClip: "move_clip",
    updateClip: "update_clip",
    setClip: "set_clip",
    addTrack: "add_track",
    removeTrack: "remove_track",
    deleteTrack: "remove_track",
    insertTrack: "add_track",
    addMarker: "add_marker",
    removeMarker: "remove_marker",
    deleteMarker: "remove_marker",
    insertMarker: "add_marker",
    setMetadata: "set_metadata",
    addSource: "add_source",
    insertSource: "add_source",
    addOverlay: "add_overlay",
    insertOverlay: "add_overlay",
    removeOverlay: "remove_overlay",
    deleteOverlay: "remove_overlay",
  };
  if (typeof normalized.type === "string" && commandAliases[normalized.type] !== undefined) {
    normalized.type = commandAliases[normalized.type];
  }
  if (normalized.type === "update" || normalized.type === "set") {
    normalized.type = "update_clip";
    if (normalized.clipId === undefined && typeof normalized.id === "string") {
      normalized.clipId = normalized.id;
    }
    if (normalized.patch === undefined && normalized.changes !== undefined) {
      normalized.patch = normalized.changes;
    }
    if (normalized.patch === undefined && normalized.value !== undefined) {
      const property = typeof normalized.property === "string" ? normalized.property : undefined;
      normalized.patch = property === undefined ? normalized.value : { [property]: normalized.value };
    }
  }
  const clipPropertyCommands: Record<string, string> = {
    setOpacity: "opacity",
    set_opacity: "opacity",
    setTransform: "transform",
    set_transform: "transform",
    setBlend: "blend",
    set_blend: "blend",
    setEffects: "effects",
    set_effects: "effects",
    setKeyframes: "keyframes",
    set_keyframes: "keyframes",
    setTransitions: "transitions",
    set_transitions: "transitions",
    setTimelineStart: "timelineStart",
    set_timeline_start: "timelineStart",
  };
  if (typeof normalized.type === "string" && clipPropertyCommands[normalized.type] !== undefined) {
    const property = clipPropertyCommands[normalized.type]!;
    normalized.type = "update_clip";
    if (normalized.clipId === undefined && typeof normalized.id === "string") {
      normalized.clipId = normalized.id;
    }
    if (normalized.patch === undefined) {
      normalized.patch = {
        [property]: normalized[property] === undefined ? normalized.value : normalized[property],
      };
    }
  }
  if (normalized.type === "insert_clip") normalized.type = "add_clip";
  if (normalized.type === "delete_clip") normalized.type = "remove_clip";
  if (normalized.type === "insert_track") normalized.type = "add_track";
  if (normalized.type === "delete_track") normalized.type = "remove_track";
  if (normalized.type === "insert_marker") normalized.type = "add_marker";
  if (normalized.type === "delete_marker") normalized.type = "remove_marker";
  return normalized;
}

/** JSON-serialisable command union for v2 project edits. */
export const ProjectCommandSchema = z.preprocess(
  normalizeCommand,
  CanonicalProjectCommandSchema,
) as z.ZodType<ProjectCommand>;
export type ProjectCommand = CommandWithoutBatch | { type: "batch"; commands: ProjectCommand[] };

export const AddTrackCommand = AddTrackCommandSchema;
export const RemoveTrackCommand = RemoveTrackCommandSchema;
export const AddClipCommand = AddClipCommandSchema;
export const RemoveClipCommand = RemoveClipCommandSchema;
export const MoveClipCommand = MoveClipCommandSchema;
export const UpdateClipCommand = UpdateClipCommandSchema;
export const SetClipCommand = SetClipCommandSchema;
export const AddMarkerCommand = AddMarkerCommandSchema;
export const RemoveMarkerCommand = RemoveMarkerCommandSchema;
export const SetMetadataCommand = SetMetadataCommandSchema;
export const AddSourceCommand = AddSourceCommandSchema;
export const AddOverlayCommand = AddOverlayCommandSchema;
export const RemoveOverlayCommand = RemoveOverlayCommandSchema;
export const BatchCommand = BatchCommandSchema;

export interface HistoryEntry {
  readonly command: ProjectCommand;
  readonly before: Project;
  readonly after: Project;
}

export interface ProjectHistory {
  readonly project: Project;
  readonly revision: number;
  readonly past: readonly HistoryEntry[];
  readonly future: readonly HistoryEntry[];
  /** Aliases for present/undoStack terminology. */
  readonly present: Project;
  readonly undoStack: readonly HistoryEntry[];
  readonly redoStack: readonly HistoryEntry[];
}

export type CommandHistory = ProjectHistory;
export type ProjectHistoryState = ProjectHistory;

export interface RevisionOptions {
  expectedRevision?: number;
}

export class RevisionConflictError extends Error {
  readonly code = "REVISION_CONFLICT";
  readonly expectedRevision: number;
  readonly actualRevision: number;
  readonly expected: number;
  readonly actual: number;

  constructor(expectedRevision: number, actualRevision: number) {
    super(`revision conflict: expected ${expectedRevision}, actual ${actualRevision}`);
    this.name = "RevisionConflictError";
    this.expectedRevision = expectedRevision;
    this.actualRevision = actualRevision;
    this.expected = expectedRevision;
    this.actual = actualRevision;
  }
}

export const ConflictError = RevisionConflictError;

function makeHistory(
  project: Project,
  revision: number,
  past: readonly HistoryEntry[],
  future: readonly HistoryEntry[],
): ProjectHistory {
  const state = {
    project,
    revision,
    past: [...past],
    future: [...future],
  } as unknown as ProjectHistory;
  // Non-enumerable aliases keep serialized/debug output compact while making
  // both common history vocabularies available to TypeScript callers.
  Object.defineProperties(state, {
    present: { enumerable: false, get: () => state.project },
    undoStack: { enumerable: false, get: () => state.past },
    redoStack: { enumerable: false, get: () => state.future },
  });
  return state;
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

function expectedRevisionFrom(
  commandOrOptions: unknown,
  optionsOrRevision?: RevisionOptions | number,
): number | undefined {
  if (typeof optionsOrRevision === "number") return optionsOrRevision;
  if (optionsOrRevision?.expectedRevision !== undefined) return optionsOrRevision.expectedRevision;
  if (typeof commandOrOptions === "object" && commandOrOptions !== null) {
    const expected = (commandOrOptions as { expectedRevision?: unknown }).expectedRevision;
    if (typeof expected === "number") return expected;
  }
  return undefined;
}

function checkRevision(history: ProjectHistory, expectedRevision: number | undefined): void {
  if (expectedRevision === undefined) return;
  if (!Number.isInteger(expectedRevision) || expectedRevision < 0) {
    throw new Error(`expectedRevision must be a nonnegative integer, got ${expectedRevision}`);
  }
  if (history.revision !== expectedRevision) {
    throw new RevisionConflictError(expectedRevision, history.revision);
  }
}

function trackIndex(project: Project, trackId: string): number {
  const indexes = project.tracks
    .map((track, index) => (track.id === trackId ? index : -1))
    .filter((index) => index !== -1);
  if (indexes.length === 0) throw new Error(`track not found: ${trackId}`);
  if (indexes.length > 1) throw new Error(`ambiguous track id: ${trackId}`);
  return indexes[0]!;
}

function clipLocation(
  project: Project,
  clipId: string,
  trackId?: string,
): { trackIndex: number; clipIndex: number } {
  const matches: Array<{ trackIndex: number; clipIndex: number }> = [];
  project.tracks.forEach((track, trackIndex) => {
    if (trackId !== undefined && track.id !== trackId) return;
    track.clips.forEach((clip, clipIndex) => {
      if (clip.id === clipId) matches.push({ trackIndex, clipIndex });
    });
  });
  if (matches.length === 0) throw new Error(`clip not found: ${clipId}`);
  if (matches.length > 1) throw new Error(`ambiguous clip id: ${clipId}`);
  return matches[0]!;
}

function markerIndex(project: Project, markerId: string): number {
  const indexes = project.markers
    .map((marker, index) => (marker.id === markerId ? index : -1))
    .filter((index) => index !== -1);
  if (indexes.length === 0) throw new Error(`marker not found: ${markerId}`);
  if (indexes.length > 1) throw new Error(`ambiguous marker id: ${markerId}`);
  return indexes[0]!;
}

function applyOne(project: Project, command: ProjectCommand): void {
  switch (command.type) {
    case "add_track": {
      if (project.tracks.some((track) => track.id === command.track.id)) {
        throw new Error(`track already exists: ${command.track.id}`);
      }
      project.tracks.push(clone(command.track));
      return;
    }
    case "remove_track": {
      const index = trackIndex(project, command.trackId);
      project.tracks.splice(index, 1);
      return;
    }
    case "add_clip": {
      const index = trackIndex(project, command.trackId);
      const track = project.tracks[index]!;
      if (project.tracks.some((candidate) => candidate.clips.some((clip) => clip.id === command.clip.id))) {
        throw new Error(`clip already exists: ${command.clip.id}`);
      }
      const destination = command.index ?? track.clips.length;
      if (destination < 0 || destination > track.clips.length) {
        throw new Error(`add_clip index out of range: ${destination}`);
      }
      track.clips.splice(destination, 0, clone(command.clip));
      return;
    }
    case "remove_clip": {
      const location = clipLocation(project, command.clipId, command.trackId);
      project.tracks[location.trackIndex]!.clips.splice(location.clipIndex, 1);
      return;
    }
    case "move_clip": {
      const source = clipLocation(project, command.clipId);
      const sourceTrack = project.tracks[source.trackIndex]!;
      const [clip] = sourceTrack.clips.splice(source.clipIndex, 1);
      if (!clip) throw new Error(`clip not found: ${command.clipId}`);
      const targetTrackId = command.toTrackId ?? command.trackId;
      const targetIndex = targetTrackId === undefined ? source.trackIndex : trackIndex(project, targetTrackId);
      const targetTrack = project.tracks[targetIndex]!;
      const destination = command.toIndex ?? command.index;
      if (destination === undefined || destination > targetTrack.clips.length) {
        sourceTrack.clips.splice(source.clipIndex, 0, clip);
        throw new Error(`move_clip index out of range: ${destination}`);
      }
      targetTrack.clips.splice(destination, 0, clip);
      return;
    }
    case "update_clip": {
      const location = clipLocation(project, command.clipId);
      const current = project.tracks[location.trackIndex]!.clips[location.clipIndex]!;
      if ("id" in command.patch && command.patch.id !== current.id) {
        throw new Error("update_clip cannot change clip id");
      }
      const updated = TimelineClipSchema.parse({ ...current, ...clone(command.patch) });
      project.tracks[location.trackIndex]!.clips[location.clipIndex] = updated;
      return;
    }
    case "set_clip": {
      if (command.clip.id !== command.clipId) {
        throw new Error(`set_clip id mismatch: ${command.clip.id} !== ${command.clipId}`);
      }
      const location = clipLocation(project, command.clipId);
      project.tracks[location.trackIndex]!.clips[location.clipIndex] = clone(command.clip);
      return;
    }
    case "add_marker": {
      if (project.markers.some((marker) => marker.id === command.marker.id)) {
        throw new Error(`marker already exists: ${command.marker.id}`);
      }
      project.markers.push(clone(command.marker));
      return;
    }
    case "remove_marker": {
      project.markers.splice(markerIndex(project, command.markerId), 1);
      return;
    }
    case "set_metadata": {
      project.metadata = { ...(project.metadata ?? {}), ...clone(command.patch) };
      return;
    }
    case "add_source": {
      if (project.sources.some((source) => source.id === command.source.id)) {
        throw new Error(`source already exists: ${command.source.id}`);
      }
      project.sources.push(clone(command.source));
      return;
    }
    case "add_overlay": {
      const overlay = command.overlay;
      if (!overlay.id) throw new Error("add_overlay requires overlay.id");
      if (project.overlays.some((candidate) => candidate.id === overlay.id)) {
        throw new Error(`overlay already exists: ${overlay.id}`);
      }
      project.overlays.push(clone(overlay));
      return;
    }
    case "remove_overlay": {
      const matches = project.overlays
        .map((overlay, index) => (overlay.id === command.overlayId ? index : -1))
        .filter((index) => index !== -1);
      if (matches.length === 0) throw new Error(`overlay not found: ${command.overlayId}`);
      if (matches.length > 1) throw new Error(`ambiguous overlay id: ${command.overlayId}`);
      project.overlays.splice(matches[0]!, 1);
      return;
    }
    case "batch":
      for (const nested of command.commands) applyOne(project, nested);
      return;
    default: {
      const never: never = command;
      throw new Error(`unknown project command: ${JSON.stringify(never)}`);
    }
  }
}

function parseCommand(command: unknown): ProjectCommand {
  return ProjectCommandSchema.parse(command) as ProjectCommand;
}

function applyCommandsToProject(project: Project, commands: readonly unknown[]): Project {
  const parsed = commands.map(parseCommand);
  const next = clone(project);
  for (const command of parsed) applyOne(next, command);
  // One final parse is the transaction boundary.  A failed command or an
  // invalid final reference never escapes the private clone.
  return ProjectSchema.parse(next);
}

/** Start an immutable project history at revision zero. */
export function createProjectHistory(project: Project | unknown, revision = 0): ProjectHistory {
  if (!Number.isInteger(revision) || revision < 0) {
    throw new Error(`revision must be a nonnegative integer, got ${revision}`);
  }
  const validated = ProjectSchema.parse(project);
  return makeHistory(validated, revision, [], []);
}

export const createHistory = createProjectHistory;
export const createProjectCommandHistory = createProjectHistory;
export const beginProjectHistory = createProjectHistory;

/** Apply one command atomically and append one undo entry. */
export function applyProjectCommand(
  history: ProjectHistory,
  command: ProjectCommand | unknown,
  optionsOrRevision?: RevisionOptions | number,
): ProjectHistory {
  if (Array.isArray(command)) {
    return applyProjectCommands(history, command, optionsOrRevision);
  }
  checkRevision(history, expectedRevisionFrom(command, optionsOrRevision));
  const before = ProjectSchema.parse(history.project);
  const parsed = parseCommand(command);
  const after = applyCommandsToProject(before, [parsed]);
  const entry: HistoryEntry = { command: clone(parsed), before: clone(before), after: clone(after) };
  return makeHistory(after, history.revision + 1, [...history.past, entry], []);
}

export const applyCommand = applyProjectCommand;
export const applyProjectEdit = applyProjectCommand;

/** Apply a batch as one atomic, one-step history entry. */
export function applyProjectCommands(
  history: ProjectHistory,
  commands: readonly (ProjectCommand | unknown)[],
  optionsOrRevision?: RevisionOptions | number,
): ProjectHistory {
  checkRevision(history, expectedRevisionFrom(undefined, optionsOrRevision));
  if (!Array.isArray(commands)) throw new Error("commands must be an array");
  if (commands.length === 0) throw new Error("commands must not be empty");
  const before = ProjectSchema.parse(history.project);
  const parsed = commands.map(parseCommand);
  const after = applyCommandsToProject(before, parsed);
  const command: ProjectCommand = { type: "batch", commands: clone(parsed) };
  const entry: HistoryEntry = { command, before: clone(before), after: clone(after) };
  return makeHistory(after, history.revision + 1, [...history.past, entry], []);
}

export const applyCommands = applyProjectCommands;

/** Apply v2 commands without retaining history, useful at integration edges. */
export function applyProject(
  project: Project | unknown,
  commandOrCommands: ProjectCommand | readonly (ProjectCommand | unknown)[] | unknown,
): Project {
  const current = ProjectSchema.parse(project);
  if (Array.isArray(commandOrCommands)) {
    return applyCommandsToProject(current, commandOrCommands);
  }
  return applyCommandsToProject(current, [commandOrCommands]);
}

export const applyCommandToProject = applyProject;

/** Undo one entry.  Empty stacks are a pure no-op. */
export function undoProject(
  history: ProjectHistory,
  optionsOrRevision?: RevisionOptions | number,
): ProjectHistory {
  checkRevision(history, expectedRevisionFrom(undefined, optionsOrRevision));
  const entry = history.past[history.past.length - 1];
  if (!entry) return history;
  return makeHistory(
    clone(entry.before),
    history.revision + 1,
    history.past.slice(0, -1),
    [...history.future, entry],
  );
}

export const undo = undoProject;
export const undoProjectHistory = undoProject;

/** Redo one entry.  Empty stacks are a pure no-op. */
export function redoProject(
  history: ProjectHistory,
  optionsOrRevision?: RevisionOptions | number,
): ProjectHistory {
  checkRevision(history, expectedRevisionFrom(undefined, optionsOrRevision));
  const entry = history.future[history.future.length - 1];
  if (!entry) return history;
  return makeHistory(
    clone(entry.after),
    history.revision + 1,
    [...history.past, entry],
    history.future.slice(0, -1),
  );
}

export const redo = redoProject;
export const redoProjectHistory = redoProject;
export const canUndo = (history: ProjectHistory): boolean => history.past.length > 0;
export const canRedo = (history: ProjectHistory): boolean => history.future.length > 0;

/** Read-only helper for UIs and command adapters. */
export function currentProject(history: ProjectHistory): Project {
  return clone(history.project);
}
