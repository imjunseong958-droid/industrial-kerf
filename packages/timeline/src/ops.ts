import {
  EditOpSchema,
  TimelineSchema,
  type CaptionCue,
  type Clip,
  type CropRegion,
  type EditOp,
  type SubtitleStyle,
  type Timeline,
  type Track,
} from "./schema.js";

/** Deep-clone a timeline (structuredClone keeps operations pure). */
function clone(timeline: Timeline): Timeline {
  return structuredClone(timeline);
}

function finiteNonnegative(value: number): boolean {
  return Number.isFinite(value) && value >= 0;
}

function finitePositive(value: number): boolean {
  return Number.isFinite(value) && value > 0;
}

function validateIndex(index: number, length: number, operation: string): void {
  if (!Number.isInteger(index) || index < 0 || index > length) {
    throw new Error(`${operation}: index ${index} must be an integer in [0, ${length}]`);
  }
}

function validateCrop(crop: CropRegion): void {
  if (
    !finiteNonnegative(crop.x) ||
    !finiteNonnegative(crop.y) ||
    !finitePositive(crop.width) ||
    !finitePositive(crop.height) ||
    crop.x > 1 ||
    crop.y > 1 ||
    crop.width > 1 ||
    crop.height > 1 ||
    crop.x + crop.width > 1 ||
    crop.y + crop.height > 1
  ) {
    throw new Error("crop must use normalized values within [0, 1]");
  }
}

/** Find a track by id, or the first track when no id was supplied. */
function findTrack(timeline: Timeline, trackId: string | undefined, operation: string): Track {
  if (trackId === undefined) {
    const track = timeline.tracks[0];
    if (!track) throw new Error(`${operation}: timeline has no tracks`);
    return track;
  }

  const matches = timeline.tracks.filter((track) => track.id === trackId);
  if (matches.length === 0) throw new Error(`${operation}: track not found: ${trackId}`);
  if (matches.length > 1) throw new Error(`${operation}: ambiguous track id: ${trackId}`);
  return matches[0]!;
}

/** Find a clip by id across all tracks. IDs are globally unique in valid timelines. */
function findClip(timeline: Timeline, clipId: string, operation = "edit"): [Track, number] {
  let found: [Track, number] | undefined;
  for (const track of timeline.tracks) {
    const index = track.clips.findIndex((clip) => clip.id === clipId);
    if (index === -1) continue;
    if (found) throw new Error(`${operation}: ambiguous clip id: ${clipId}`);
    found = [track, index];
  }
  if (!found) throw new Error(`${operation}: clip not found: ${clipId}`);
  return found;
}

function clipIds(timeline: Timeline): Set<string> {
  const ids = new Set<string>();
  for (const track of timeline.tracks) {
    for (const clip of track.clips) ids.add(clip.id);
  }
  return ids;
}

function overlayIds(timeline: Timeline): Set<string> {
  return new Set(
    timeline.overlays
      .map((overlay) => overlay.id)
      .filter((id): id is string => id !== undefined),
  );
}

/**
 * Pick the first available ID in a stable sequence. The caller owns `used`,
 * which lets one operation reserve multiple IDs without relying on array
 * length (removed entries and user-selected IDs are handled correctly).
 */
function nextId(used: Set<string>, base: string): string {
  if (!used.has(base)) {
    used.add(base);
    return base;
  }
  let n = 2;
  while (used.has(`${base}-${n}`)) n += 1;
  const id = `${base}-${n}`;
  used.add(id);
  return id;
}

function nextNumberedId(used: Set<string>, prefix: string): string {
  let n = 1;
  while (used.has(`${prefix}${n}`)) n += 1;
  const id = `${prefix}${n}`;
  used.add(id);
  return id;
}

// ---------------------------------------------------------------------------
// add_source / add_clip / remove_clip / move_clip / set_crop
// ---------------------------------------------------------------------------

function applyAddSource(timeline: Timeline, source: Timeline["sources"][number]): void {
  if (timeline.sources.some((candidate) => candidate.id === source.id)) {
    throw new Error(`add_source: source id already exists: ${source.id}`);
  }
  timeline.sources.push(structuredClone(source));
}

function applyAddClip(
  timeline: Timeline,
  input: Extract<EditOp, { op: "add_clip" }>,
): void {
  const track = findTrack(timeline, input.trackId, "add_clip");
  const index = input.index === undefined ? track.clips.length : input.index;
  validateIndex(index, track.clips.length, "add_clip");

  if (!timeline.sources.some((source) => source.id === input.clip.src)) {
    throw new Error(`add_clip: source not found: ${input.clip.src}`);
  }
  if (!finiteNonnegative(input.clip.in) || !finiteNonnegative(input.clip.out)) {
    throw new Error("add_clip: clip in/out must be finite and nonnegative");
  }
  if (!(input.clip.out > input.clip.in)) {
    throw new Error(`add_clip: clip out must be greater than in`);
  }
  if (input.clip.speed !== undefined && !finitePositive(input.clip.speed)) {
    throw new Error(`add_clip: speed must be positive, got ${input.clip.speed}`);
  }
  if (input.clip.volume !== undefined && !finiteNonnegative(input.clip.volume)) {
    throw new Error(`add_clip: volume must be nonnegative, got ${input.clip.volume}`);
  }
  if (input.clip.crop !== undefined) validateCrop(input.clip.crop);

  const ids = clipIds(timeline);
  if (input.clip.id !== undefined && ids.has(input.clip.id)) {
    throw new Error(`add_clip: clip id already exists: ${input.clip.id}`);
  }
  const id = input.clip.id ?? nextNumberedId(ids, "clip");
  const clip: Clip = {
    id,
    src: input.clip.src,
    in: input.clip.in,
    out: input.clip.out,
    ...(input.clip.speed !== undefined ? { speed: input.clip.speed } : {}),
    ...(input.clip.volume !== undefined ? { volume: input.clip.volume } : {}),
    ...(input.clip.crop !== undefined ? { crop: structuredClone(input.clip.crop) } : {}),
  };
  track.clips.splice(index, 0, clip);
}

function applyRemoveClip(timeline: Timeline, clipId: string): void {
  const [track, index] = findClip(timeline, clipId, "remove_clip");
  track.clips.splice(index, 1);
}

function applyMoveClip(
  timeline: Timeline,
  clipId: string,
  toIndex: number,
  trackId?: string,
): void {
  if (!Number.isInteger(toIndex) || toIndex < 0) {
    throw new Error(`move_clip: toIndex must be a nonnegative integer, got ${toIndex}`);
  }
  const [fromTrack, fromIndex] = findClip(timeline, clipId, "move_clip");
  const toTrack = findTrack(timeline, trackId, "move_clip");

  // Remove first so a same-track move's destination index is the final index
  // in the resulting array. This also makes moving to the current index a no-op.
  const [clip] = fromTrack.clips.splice(fromIndex, 1);
  if (!clip) throw new Error(`move_clip: clip not found: ${clipId}`);

  try {
    validateIndex(toIndex, toTrack.clips.length, "move_clip");
  } catch (error) {
    // Restore the clone's pre-operation state before propagating the error. It
    // is not observable by the caller, but keeps subsequent operations safe if
    // this helper is ever reused within a transaction.
    fromTrack.clips.splice(fromIndex, 0, clip);
    throw error;
  }
  toTrack.clips.splice(toIndex, 0, clip);
}

function applySetCrop(
  timeline: Timeline,
  clipId: string,
  crop: CropRegion | null,
): void {
  const [track, index] = findClip(timeline, clipId, "set_crop");
  if (crop === null) {
    const { crop: _crop, ...withoutCrop } = track.clips[index]!;
    track.clips[index] = withoutCrop;
    return;
  }
  validateCrop(crop);
  track.clips[index] = { ...track.clips[index]!, crop: structuredClone(crop) };
}

// ---------------------------------------------------------------------------
// trim / split
// ---------------------------------------------------------------------------

function applyTrim(timeline: Timeline, clipId: string, newIn?: number, newOut?: number): void {
  const [track, index] = findClip(timeline, clipId, "trim");
  const clip = track.clips[index]!;
  const inN = newIn === undefined ? clip.in : newIn;
  const outN = newOut === undefined ? clip.out : newOut;
  if (!finiteNonnegative(inN) || !finiteNonnegative(outN) || !(outN > inN)) {
    throw new Error(`trim: invalid bounds in=${inN} out=${outN} (need out > in >= 0)`);
  }
  track.clips[index] = { ...clip, in: inN, out: outN };
}

/** `at` is an absolute source time inside the clip's [in, out) interval. */
function applySplit(timeline: Timeline, clipId: string, at: number): void {
  const [track, index] = findClip(timeline, clipId, "split");
  const clip = track.clips[index]!;
  if (!finiteNonnegative(at) || !(at > clip.in && at < clip.out)) {
    throw new Error(
      `split: at=${at} must be strictly inside clip bounds [${clip.in}, ${clip.out}]`,
    );
  }

  const ids = clipIds(timeline);
  // Preserve the selected clip's ID for the left hand side, while reserving a
  // stable derived ID for the right hand side. If that ID already exists, the
  // suffix allocator keeps every segment addressable.
  const rightId = nextId(ids, `${clip.id}-b`);
  const right: Clip = { ...clip, id: rightId, in: at };
  track.clips[index] = { ...clip, out: at };
  track.clips.splice(index + 1, 0, right);
}

// ---------------------------------------------------------------------------
// remove_ranges
// ---------------------------------------------------------------------------

function normalizeRanges(
  ranges: { start: number; end: number }[],
): { start: number; end: number }[] {
  if (
    ranges.some(
      (range) =>
        !finiteNonnegative(range.start) ||
        !finiteNonnegative(range.end) ||
        !(range.end > range.start),
    )
  ) {
    throw new Error("remove_ranges: each range requires finite nonnegative end > start");
  }

  const sorted = [...ranges].sort((a, b) => a.start - b.start);
  const merged: { start: number; end: number }[] = [];
  for (const range of sorted) {
    const last = merged[merged.length - 1];
    if (last && range.start <= last.end) {
      last.end = Math.max(last.end, range.end);
    } else {
      merged.push({ ...range });
    }
  }
  return merged;
}

interface SourceSegment {
  in: number;
  out: number;
}

/**
 * Remove absolute output-timeline ranges from the first track. A clip's
 * output duration is `(out - in) / speed`, so each output cut is mapped back
 * through the playback rate before source intervals are emitted.
 */
function applyRemoveRanges(
  timeline: Timeline,
  ranges: { start: number; end: number }[],
): void {
  const merged = normalizeRanges(ranges);
  const track = timeline.tracks[0];
  if (!track) return;
  const ids = clipIds(timeline);
  const kept: Clip[] = [];
  let cursor = 0;

  for (const clip of track.clips) {
    const speed = clip.speed ?? 1;
    if (!finitePositive(speed)) {
      throw new Error(`remove_ranges: clip ${clip.id} has invalid speed ${speed}`);
    }

    const sourceDuration = clip.out - clip.in;
    const outputDuration = sourceDuration / speed;
    const clipStart = cursor;
    const clipEnd = clipStart + outputDuration;
    cursor = clipEnd;

    const segments: SourceSegment[] = [];
    let outputCursor = 0;
    for (const range of merged) {
      if (range.end <= clipStart) continue;
      if (range.start >= clipEnd) break;

      const cutStart = Math.max(0, range.start - clipStart);
      const cutEnd = Math.min(outputDuration, range.end - clipStart);
      if (!(cutEnd > cutStart)) continue;

      if (cutStart > outputCursor) {
        segments.push({
          in: clip.in + outputCursor * speed,
          out: clip.in + cutStart * speed,
        });
      }
      outputCursor = Math.max(outputCursor, cutEnd);
    }
    if (outputCursor < outputDuration) {
      segments.push({
        in: clip.in + outputCursor * speed,
        out: clip.out,
      });
    }

    if (segments.length === 0) continue;
    for (const [segmentIndex, segment] of segments.entries()) {
      // The first survivor keeps the original id. Every additional survivor
      // gets a deterministic id that cannot collide with any existing clip or
      // an earlier generated segment.
      const id =
        segmentIndex === 0
          ? clip.id
          : nextId(ids, `${clip.id}-part-${segmentIndex + 1}`);
      kept.push({ ...clip, id, in: segment.in, out: segment.out });
    }
  }

  track.clips = kept;
}

// ---------------------------------------------------------------------------
// set_speed / set_volume
// ---------------------------------------------------------------------------

function applySetSpeed(timeline: Timeline, clipId: string, speed: number): void {
  const [track, index] = findClip(timeline, clipId, "set_speed");
  if (!finitePositive(speed)) throw new Error(`set_speed: speed must be positive, got ${speed}`);
  track.clips[index] = { ...track.clips[index]!, speed };
}

function applySetVolume(timeline: Timeline, clipId: string, volume: number): void {
  const [track, index] = findClip(timeline, clipId, "set_volume");
  if (!finiteNonnegative(volume)) {
    throw new Error(`set_volume: volume must be finite and >= 0, got ${volume}`);
  }
  track.clips[index] = { ...track.clips[index]!, volume };
}

// ---------------------------------------------------------------------------
// overlays
// ---------------------------------------------------------------------------

function applyAddSubtitles(
  timeline: Timeline,
  path: string,
  style?: SubtitleStyle,
): void {
  const ids = overlayIds(timeline);
  const id = nextNumberedId(ids, "subs");
  timeline.overlays.push({
    id,
    type: "subtitles",
    path,
    ...(style !== undefined ? { style: structuredClone(style) } : {}),
  });
}

function applyAddCaptions(
  timeline: Timeline,
  cues: CaptionCue[],
  style?: SubtitleStyle,
): void {
  const ids = overlayIds(timeline);
  const id = nextNumberedId(ids, "captions");
  timeline.overlays.push({
    id,
    type: "captions",
    cues: structuredClone(cues),
    ...(style !== undefined ? { style: structuredClone(style) } : {}),
  });
}

function applyRemoveOverlay(timeline: Timeline, id: string): void {
  const matches = timeline.overlays
    .map((overlay, index) => (overlay.id === id ? index : -1))
    .filter((index) => index !== -1);
  if (matches.length === 0) throw new Error(`overlay not found: ${id}`);
  if (matches.length > 1) throw new Error(`ambiguous overlay id: ${id}`);
  timeline.overlays.splice(matches[0]!, 1);
}

// ---------------------------------------------------------------------------
// dispatcher
// ---------------------------------------------------------------------------

function applyOne(timeline: Timeline, op: EditOp): void {
  switch (op.op) {
    case "add_source":
      applyAddSource(timeline, op.source);
      break;
    case "add_clip":
      applyAddClip(timeline, op);
      break;
    case "remove_clip":
      applyRemoveClip(timeline, op.clipId);
      break;
    case "move_clip":
      applyMoveClip(timeline, op.clipId, op.toIndex, op.trackId);
      break;
    case "set_crop":
      applySetCrop(timeline, op.clipId, op.crop);
      break;
    case "add_captions":
      applyAddCaptions(timeline, op.cues, op.style);
      break;
    case "trim":
      applyTrim(timeline, op.clipId, op.newIn, op.newOut);
      break;
    case "split":
      applySplit(timeline, op.clipId, op.at);
      break;
    case "remove_ranges":
      applyRemoveRanges(timeline, op.ranges);
      break;
    case "set_speed":
      applySetSpeed(timeline, op.clipId, op.speed);
      break;
    case "set_volume":
      applySetVolume(timeline, op.clipId, op.volume);
      break;
    case "add_subtitles":
      applyAddSubtitles(timeline, op.path, op.style);
      break;
    case "remove_overlay":
      applyRemoveOverlay(timeline, op.id);
      break;
    default: {
      const never: never = op;
      throw new Error(`unknown edit op: ${JSON.stringify(never)}`);
    }
  }
}

/**
 * Apply a sequence of edits atomically. Operations mutate only a private deep
 * clone; the input is validated exactly once after all operations complete.
 */
export function applyEdits(timeline: Timeline, ops: readonly EditOp[]): Timeline {
  if (!Array.isArray(ops)) throw new Error("applyEdits: ops must be an array");
  const next = clone(timeline);
  for (const rawOp of ops) {
    const op = EditOpSchema.parse(rawOp);
    applyOne(next, op);
  }
  return TimelineSchema.parse(next);
}

/** Apply one edit using the same atomic path as a batch. */
export function applyEdit(timeline: Timeline, op: EditOp): Timeline {
  return applyEdits(timeline, [op]);
}
