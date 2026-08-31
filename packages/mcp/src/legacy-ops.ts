import {
  ProjectSchema,
  ProjectSourceSchema,
  clipSourceId,
  clipTimelineDuration,
  clipTimelineStart,
  type Project,
  type ProjectCommand,
  type TimelineClip,
} from "@cut/timeline";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function nextNumberedId(used: Set<string>, prefix: string): string {
  let n = 1;
  while (used.has(`${prefix}${n}`)) n += 1;
  return `${prefix}${n}`;
}

function clipIds(project: Project): Set<string> {
  return new Set(project.tracks.flatMap((track) => track.clips.map((clip) => clip.id)));
}

function overlayIds(project: Project): Set<string> {
  return new Set(
    project.overlays
      .map((overlay) => overlay.id)
      .filter((id): id is string => id !== undefined),
  );
}

function findClip(
  project: Project,
  clipId: string,
): { clip: TimelineClip; trackId: string } {
  let found: { clip: TimelineClip; trackId: string } | undefined;
  for (const track of project.tracks) {
    const clip = track.clips.find((candidate) => candidate.id === clipId);
    if (!clip) continue;
    if (found) throw new Error(`clip not found or ambiguous: ${clipId}`);
    found = { clip, trackId: track.id };
  }
  if (!found) throw new Error(`clip not found or ambiguous: ${clipId}`);
  return found;
}

function normalizeRanges(
  ranges: { start: number; end: number }[],
): { start: number; end: number }[] {
  if (
    ranges.some(
      (range) =>
        !Number.isFinite(range.start) ||
        !Number.isFinite(range.end) ||
        range.start < 0 ||
        !(range.end > range.start),
    )
  ) {
    throw new Error("remove_ranges: each range requires finite nonnegative end > start");
  }
  const sorted = [...ranges].sort((left, right) => left.start - right.start);
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

function applyRemoveRangesV2(
  project: Project,
  ranges: { start: number; end: number }[],
): Project {
  const merged = normalizeRanges(ranges);
  const next = structuredClone(project);
  const track =
    next.tracks.find((candidate) => candidate.type === "video") ??
    next.tracks[0];
  if (!track) return ProjectSchema.parse(next);

  const ids = clipIds(next);
  const kept: TimelineClip[] = [];
  const sorted = [...track.clips].sort(
    (left, right) => clipTimelineStart(left) - clipTimelineStart(right),
  );

  for (const clip of sorted) {
    const speed = clip.speed ?? 1;
    if (!Number.isFinite(speed) || speed <= 0) {
      throw new Error(`remove_ranges: clip ${clip.id} has invalid speed ${speed}`);
    }
    const clipIn = clip.in ?? 0;
    const clipOut = clip.out ?? clipIn;
    const sourceDuration = clipOut - clipIn;
    const outputDuration = sourceDuration / speed;
    const clipStart = clipTimelineStart(clip);
    const clipEnd = clipStart + outputDuration;

    const segments: Array<{ in: number; out: number }> = [];
    let outputCursor = 0;
    for (const range of merged) {
      if (range.end <= clipStart) continue;
      if (range.start >= clipEnd) break;

      const cutStart = Math.max(0, range.start - clipStart);
      const cutEnd = Math.min(outputDuration, range.end - clipStart);
      if (!(cutEnd > cutStart)) continue;

      if (cutStart > outputCursor) {
        segments.push({
          in: clipIn + outputCursor * speed,
          out: clipIn + cutStart * speed,
        });
      }
      outputCursor = Math.max(outputCursor, cutEnd);
    }
    if (outputCursor < outputDuration) {
      segments.push({
        in: clipIn + outputCursor * speed,
        out: clipOut,
      });
    }

    if (segments.length === 0) continue;
    for (const [segmentIndex, segment] of segments.entries()) {
      const id =
        segmentIndex === 0
          ? clip.id
          : (() => {
              let candidate = `${clip.id}-part-${segmentIndex + 1}`;
              if (!ids.has(candidate)) {
                ids.add(candidate);
                return candidate;
              }
              let n = 2;
              while (ids.has(`${clip.id}-part-${segmentIndex + 1}-${n}`)) n += 1;
              candidate = `${clip.id}-part-${segmentIndex + 1}-${n}`;
              ids.add(candidate);
              return candidate;
            })();
      kept.push({
        ...clip,
        id,
        in: segment.in,
        out: segment.out,
        timelineStart:
          segmentIndex === 0
            ? clipTimelineStart(clip)
            : clipTimelineStart(clip) + (segment.in - clipIn) / speed,
      });
    }
  }

  track.clips = kept;
  return ProjectSchema.parse(next);
}

/** Lower a legacy whole-project transformation into semantic clip commands.
 * The old reducer never crosses the runtime boundary; only this adapter may
 * inspect the legacy result, and it emits a normal atomic batch. */
function projectClipDiff(before: Project, after: Project): ProjectCommand {
  const commands: ProjectCommand[] = [];
  for (const beforeTrack of before.tracks) {
    const afterTrack = after.tracks.find((track) => track.id === beforeTrack.id);
    if (!afterTrack) throw new Error(`legacy transformation cannot remove track ${beforeTrack.id}`);
    const changed = JSON.stringify(beforeTrack.clips) !== JSON.stringify(afterTrack.clips);
    if (!changed) continue;
    for (const clip of beforeTrack.clips) commands.push({ type: "remove_clip", clipId: clip.id, trackId: beforeTrack.id });
    afterTrack.clips.forEach((clip, index) => {
      commands.push({ type: "add_clip", trackId: beforeTrack.id, clip, index });
    });
  }
  if (commands.length === 0) throw new Error("legacy transformation produced no semantic clip changes");
  return { type: "batch", commands };
}

export function commandFromLegacyOp(project: Project, op: Record<string, unknown>): ProjectCommand {
  const name = op.op;
  if (typeof name !== "string") throw new Error("legacy edit operation requires an op discriminator");
  const clipId = typeof op.clipId === "string" ? op.clipId : undefined;
  const findClipById = (): TimelineClip => findClip(project, clipId!).clip;

  switch (name) {
    case "trim": {
      const clip = findClipById();
      const patch: Record<string, unknown> = {};
      if (typeof op.newIn === "number") patch.in = op.newIn;
      if (typeof op.newOut === "number") patch.out = op.newOut;
      if (Object.keys(patch).length === 0) throw new Error("trim requires newIn or newOut");
      return { type: "update_clip", clipId: clip.id, patch };
    }
    case "set_speed":
      findClipById();
      return { type: "update_clip", clipId: clipId!, patch: { speed: op.speed } };
    case "set_volume":
      findClipById();
      return { type: "update_clip", clipId: clipId!, patch: { volume: op.volume } };
    case "set_crop":
      findClipById();
      return { type: "update_clip", clipId: clipId!, patch: { crop: op.crop } };
    case "remove_clip":
      findClipById();
      return { type: "remove_clip", clipId: clipId! };
    case "move_clip": {
      findClipById();
      const index = op.toIndex;
      if (typeof index !== "number" || !Number.isInteger(index) || index < 0) {
        throw new Error("move_clip requires a nonnegative integer toIndex");
      }
      return {
        type: "move_clip",
        clipId: clipId!,
        ...(typeof op.trackId === "string" ? { toTrackId: op.trackId } : {}),
        toIndex: index,
      };
    }
    case "add_clip": {
      const raw = op.clip;
      if (!isRecord(raw) || typeof raw.src !== "string" || typeof raw.in !== "number" || typeof raw.out !== "number") {
        throw new Error("add_clip requires a src/in/out clip payload");
      }
      const trackId = typeof op.trackId === "string" ? op.trackId : project.tracks[0]?.id;
      if (!trackId) throw new Error("add_clip requires a destination track");
      const usedClipIds = clipIds(project);
      let generatedId = "clip1";
      let generatedNumber = 1;
      while (usedClipIds.has(generatedId)) {
        generatedNumber++;
        generatedId = `clip${generatedNumber}`;
      }
      const clip = {
        id: typeof raw.id === "string" ? raw.id : generatedId,
        sourceId: raw.src,
        in: raw.in,
        out: raw.out,
        timelineStart:
          typeof raw.timelineStart === "number"
            ? raw.timelineStart
            : projectDuration(project),
        ...(typeof raw.speed === "number" ? { speed: raw.speed } : {}),
        ...(typeof raw.volume === "number" ? { volume: raw.volume } : {}),
      };
      return {
        type: "add_clip",
        trackId,
        clip,
        ...(typeof op.index === "number" ? { index: op.index } : {}),
      };
    }
    case "split": {
      const { clip, trackId } = findClip(project, clipId!);
      const at = op.at;
      if (typeof at !== "number" || !Number.isFinite(at)) {
        throw new Error("split requires a finite at time");
      }
      const clipIn = clip.in ?? 0;
      const clipOut = clip.out ?? clipIn;
      if (!(at > clipIn && at < clipOut)) {
        throw new Error(`split: at=${at} must be strictly inside clip bounds [${clipIn}, ${clipOut}]`);
      }
      const speed = clip.speed ?? 1;
      const sourceId = clipSourceId(clip);
      if (!sourceId) throw new Error("split requires a source-backed clip");
      const ids = clipIds(project);
      let rightId = `${clip.id}-b`;
      if (ids.has(rightId)) {
        let n = 2;
        while (ids.has(`${clip.id}-b-${n}`)) n += 1;
        rightId = `${clip.id}-b-${n}`;
      }
      return {
        type: "batch",
        commands: [
          { type: "update_clip", clipId: clip.id, patch: { out: at } },
          {
            type: "add_clip",
            trackId,
            clip: {
              id: rightId,
              sourceId,
              in: at,
              out: clipOut,
              timelineStart: clipTimelineStart(clip) + (at - clipIn) / speed,
              ...(clip.speed !== undefined ? { speed: clip.speed } : {}),
              ...(clip.volume !== undefined ? { volume: clip.volume } : {}),
            },
          },
        ],
      };
    }
    case "remove_ranges": {
      const ranges = op.ranges;
      if (!Array.isArray(ranges)) throw new Error("remove_ranges requires ranges");
      return projectClipDiff(project, applyRemoveRangesV2(
        project,
        ranges as Array<{ start: number; end: number }>,
      ));
    }
    case "add_source": {
      const source = op.source;
      if (!isRecord(source) || typeof source.id !== "string" || typeof source.path !== "string") {
        throw new Error("add_source requires a source with id and path");
      }
      return { type: "add_source", source: ProjectSourceSchema.parse(source) };
    }
    case "add_subtitles": {
      const subtitlePath = op.path;
      if (typeof subtitlePath !== "string") throw new Error("add_subtitles requires path");
      const id = nextNumberedId(overlayIds(project), "subs");
      return {
        type: "add_overlay",
        overlay: {
          id,
          type: "subtitles",
          path: subtitlePath,
          ...(isRecord(op.style) ? { style: op.style } : {}),
        },
      };
    }
    case "add_captions": {
      const cues = op.cues;
      if (!Array.isArray(cues)) throw new Error("add_captions requires cues");
      const id = nextNumberedId(overlayIds(project), "captions");
      return {
        type: "add_overlay",
        overlay: {
          id,
          type: "captions",
          cues,
          ...(isRecord(op.style) ? { style: op.style } : {}),
        },
      };
    }
    case "remove_overlay": {
      const overlayId = op.id;
      if (typeof overlayId !== "string") throw new Error("remove_overlay requires id");
      return { type: "remove_overlay", overlayId };
    }
    default:
      throw new Error(`legacy operation ${name} is not supported by the v2 MCP host; send a canonical command`);
  }
}

function projectDuration(project: Project): number {
  let end = project.duration ?? 0;
  for (const track of project.tracks) {
    for (const clip of track.clips) {
      const duration = clipTimelineDuration(clip);
      if (duration === undefined) continue;
      end = Math.max(end, clipTimelineStart(clip) + duration);
    }
  }
  for (const gap of project.gaps) {
    end = Math.max(end, (gap.start ?? 0) + gap.duration);
  }
  return end;
}
