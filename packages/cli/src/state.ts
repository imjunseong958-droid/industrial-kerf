import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { TimelineSchema, type Timeline } from "@cut/timeline";

export const PROJECT_FILE = "project.cut.json";

/** Absolute paths for the session directory's well-known files. */
export function projectPath(dir: string): string {
  return path.resolve(dir, PROJECT_FILE);
}
export function framesDir(dir: string): string {
  return path.join(path.resolve(dir), ".frames");
}

export async function saveTimeline(dir: string, timeline: Timeline): Promise<void> {
  await mkdir(dir, { recursive: true });
  await writeFile(projectPath(dir), JSON.stringify(timeline, null, 2), "utf-8");
}

export async function loadTimeline(dir: string): Promise<Timeline | undefined> {
  try {
    const raw = await readFile(projectPath(dir), "utf-8");
    return TimelineSchema.parse(JSON.parse(raw));
  } catch {
    return undefined;
  }
}

export interface SourceMediaInfo {
  duration?: number;
  width?: number;
  height?: number;
  hasAudio?: boolean;
}

/** Create an initial single-clip timeline covering the whole source. */
export function initialTimeline(
  srcId: string,
  srcPath: string,
  info: SourceMediaInfo,
  subs?: string,
): Timeline {
  const duration = info.duration ?? 60;
  return {
    version: 1,
    fps: 30,
    sources: [
      subs
        ? { id: srcId, path: srcPath, subs }
        : { id: srcId, path: srcPath },
    ],
    tracks: [
      {
        id: "v1",
        clips: [
          { id: "c1", src: srcId, in: 0, out: duration },
        ],
      },
    ],
    overlays: [],
  };
}
