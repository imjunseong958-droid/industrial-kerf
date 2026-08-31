import path from "node:path";
import type { ProbeInfo } from "@cut/media";
import {
  ProjectSchema,
  clipTimelineDuration,
  clipTimelineStart,
  emptyProject,
  type Project,
} from "@cut/timeline";

export interface SourceMediaInfo {
  duration?: number;
  width?: number;
  height?: number;
  hasAudio?: boolean;
}

function nextId(existing: Iterable<string>, prefix: string): string {
  const used = new Set(existing);
  let n = 1;
  while (used.has(`${prefix}${n}`)) n += 1;
  return `${prefix}${n}`;
}

export function projectPlacementDuration(project: Project): number {
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

export function findExistingSource(
  project: Project,
  dir: string,
  resolvedPath: string,
): { sourceId: string; clipId: string } | undefined {
  const existing = project.sources.find((source) => {
    const sourcePath = path.isAbsolute(source.path)
      ? path.resolve(source.path)
      : path.resolve(dir, source.path);
    return sourcePath === resolvedPath;
  });
  if (!existing) return undefined;
  const existingClip = project.tracks
    .flatMap((track) => track.clips)
    .find((clip) => clip.sourceId === existing.id);
  return {
    sourceId: existing.id,
    clipId: existingClip?.id ?? "",
  };
}

export function appendMediaSource(
  project: Project,
  src: string,
  meta: SourceMediaInfo,
  subs?: string,
): { project: Project; sourceId: string; clipId: string } {
  const duration = meta.duration !== undefined && meta.duration > 0 ? meta.duration : 60;
  const sourceId = nextId(project.sources.map((source) => source.id), "src");
  const clipId = nextId(
    project.tracks.flatMap((track) => track.clips.map((clip) => clip.id)),
    "c",
  );
  const source = {
    id: sourceId,
    path: src,
    duration,
    ...(subs ? { subs } : {}),
    ...(meta.width !== undefined && Number.isFinite(meta.width) && meta.width > 0
      ? { width: meta.width }
      : {}),
    ...(meta.height !== undefined && Number.isFinite(meta.height) && meta.height > 0
      ? { height: meta.height }
      : {}),
    ...(meta.hasAudio !== undefined ? { hasAudio: meta.hasAudio } : {}),
  };
  const tracks = structuredClone(project.tracks);
  let track = tracks.find((candidate) => candidate.type === "video") ?? tracks[0];
  if (!track) {
    track = { id: "video-1", type: "video", clips: [] };
    tracks.push(track);
  }
  track.clips.push({
    id: clipId,
    sourceId,
    in: 0,
    out: duration,
    timelineStart: projectPlacementDuration(project),
  });
  const updated = ProjectSchema.parse({
    ...structuredClone(project),
    sources: [...project.sources, source],
    tracks,
  });
  return { project: updated, sourceId, clipId };
}

export function probeToSourceMedia(meta: ProbeInfo): SourceMediaInfo {
  return {
    ...(meta.duration !== undefined ? { duration: meta.duration } : {}),
    ...(meta.width !== undefined ? { width: meta.width } : {}),
    ...(meta.height !== undefined ? { height: meta.height } : {}),
    ...(meta.hasAudio !== undefined ? { hasAudio: meta.hasAudio } : {}),
  };
}

export { emptyProject };
