import type { Clip, Timeline, Track } from "./schema.js";

/** Deep-clone a timeline (structuredClone keeps it pure & simple). */
function clone(t: Timeline): Timeline {
  return structuredClone(t);
}

/** Find a clip by id across all tracks. Returns [track, index]. */
function findClip(t: Timeline, clipId: string): [Track, number] {
  for (const track of t.tracks) {
    const i = track.clips.findIndex((c) => c.id === clipId);
    if (i !== -1) return [track, i];
  }
  throw new Error(`clip not found: ${clipId}`);
}

// ---------------------------------------------------------------------------
// trim
// ---------------------------------------------------------------------------

function applyTrim(t: Timeline, clipId: string, newIn?: number, newOut?: number): void {
  const [track, i] = findClip(t, clipId);
  const clip = track.clips[i]!;
  const inN = newIn ?? clip.in;
  const outN = newOut ?? clip.out;
  if (!(outN > inN)) {
    throw new Error(`trim: invalid bounds in=${inN} out=${outN} (need out > in)`);
  }
  track.clips[i] = { ...clip, in: inN, out: outN };
}

// ---------------------------------------------------------------------------
// split — `at` is an absolute source time inside the clip's [in, out)
// ---------------------------------------------------------------------------

function applySplit(t: Timeline, clipId: string, at: number): void {
  const [track, i] = findClip(t, clipId);
  const clip = track.clips[i]!;
  if (!(at > clip.in && at < clip.out)) {
    throw new Error(
      `split: at=${at} must be strictly inside clip bounds [${clip.in}, ${clip.out}]`
    );
  }
  const right: Clip = {
    ...clip,
    id: `${clip.id}-b`,
    in: at,
  };
  track.clips[i] = { ...clip, out: at };
  track.clips.splice(i + 1, 0, right);
}

// ---------------------------------------------------------------------------
// remove_ranges — absolute ranges on the FIRST track's timeline; no gaps remain
// ---------------------------------------------------------------------------

function normalizeRanges(ranges: { start: number; end: number }[]): { start: number; end: number }[] {
  if (ranges.some((r) => !(r.end > r.start))) {
    throw new Error("remove_ranges: each range requires end > start");
  }
  const sorted = [...ranges].sort((a, b) => a.start - b.start);
  const merged: { start: number; end: number }[] = [];
  for (const r of sorted) {
    const last = merged[merged.length - 1];
    if (last && r.start <= last.end) {
      last.end = Math.max(last.end, r.end);
    } else {
      merged.push({ ...r });
    }
  }
  return merged;
}

function applyRemoveRanges(t: Timeline, ranges: { start: number; end: number }[]): void {
  const track = t.tracks[0];
  if (!track) return;
  const merged = normalizeRanges(ranges);

  // Walk clips along the track's timeline; keep the surviving source-time
  // portions, then re-lay them out back-to-back with no gaps.
  const kept: Clip[] = [];
  let cursor = 0; // absolute track-time position of the current clip's start
  for (const clip of track.clips) {
    const dur = clip.out - clip.in;
    const clipStart = cursor;
    const clipEnd = cursor + dur;
    cursor = clipEnd;

    // remaining source-time intervals after cutting out all merged ranges
    let segIn = clip.in;
    for (const r of merged) {
      const s = clipStart + (segIn - clip.in); // absolute time of segIn
      if (r.end <= s || r.start >= clipEnd) continue;
      const cutStart = Math.max(r.start, s);
      const cutEnd = Math.min(r.end, clipEnd);
      if (cutEnd > cutStart) {
        if (cutStart > s) kept.push({ ...clip, in: segIn, out: segIn + (cutStart - s) });
        segIn = clip.in + (cutEnd - clipStart);
      }
    }
    const s = clipStart + (segIn - clip.in);
    if (clipEnd > s) kept.push({ ...clip, in: segIn, out: segIn + (clipEnd - s) });
  }

  track.clips = kept;
}

// ---------------------------------------------------------------------------
// set_speed / set_volume
// ---------------------------------------------------------------------------

function applySetSpeed(t: Timeline, clipId: string, speed: number): void {
  const [track, i] = findClip(t, clipId);
  if (!(speed > 0)) throw new Error(`set_speed: speed must be positive, got ${speed}`);
  track.clips[i] = { ...track.clips[i]!, speed };
}

function applySetVolume(t: Timeline, clipId: string, volume: number): void {
  const [track, i] = findClip(t, clipId);
  if (!(volume >= 0)) throw new Error(`set_volume: volume must be >= 0, got ${volume}`);
  track.clips[i] = { ...track.clips[i]!, volume };
}

// ---------------------------------------------------------------------------
// add_subtitles / remove_overlay
// ---------------------------------------------------------------------------

function applyAddSubtitles(
  t: Timeline,
  path: string,
  style?: import("./schema.js").SubtitleStyle,
): void {
  t.overlays.push({
    id: `subs${t.overlays.length + 1}`,
    type: "subtitles",
    path,
    ...(style !== undefined ? { style } : {}),
  });
}

function applyRemoveOverlay(t: Timeline, id: string): void {
  const before = t.overlays.length;
  t.overlays = t.overlays.filter((o) => o.id !== id);
  if (t.overlays.length === before) throw new Error(`overlay not found: ${id}`);
}

// ---------------------------------------------------------------------------
// dispatcher
// ---------------------------------------------------------------------------

export function applyEdit(timeline: Timeline, op: import("./schema.js").EditOp): Timeline {
  const t = clone(timeline);
  switch (op.op) {
    case "trim":
      applyTrim(t, op.clipId, op.newIn, op.newOut);
      break;
    case "split":
      applySplit(t, op.clipId, op.at);
      break;
    case "remove_ranges":
      applyRemoveRanges(t, op.ranges);
      break;
    case "set_speed":
      applySetSpeed(t, op.clipId, op.speed);
      break;
    case "set_volume":
      applySetVolume(t, op.clipId, op.volume);
      break;
    case "add_subtitles":
      applyAddSubtitles(t, op.path, op.style);
      break;
    case "remove_overlay":
      applyRemoveOverlay(t, op.id);
      break;
    default: {
      const never: never = op;
      throw new Error(`unknown edit op: ${JSON.stringify(never)}`);
    }
  }
  return t;
}
