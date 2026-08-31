import { mkdir, readFile, rename, unlink, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import {
  ProjectSchema,
  TimelineSchema,
  migrateV1ToV2,
  parseTimelineProject,
  type Project,
  type Timeline,
  type VersionedTimeline,
} from "@cut/timeline";

export const PROJECT_FILE = "project.cut.json";

/** Absolute paths for the session directory's well-known files. */
export function projectPath(dir: string): string {
  return path.resolve(dir, PROJECT_FILE);
}
export function framesDir(dir: string): string {
  return path.join(path.resolve(dir), ".frames");
}

/** A parsed project retains the on-disk schema version for compatibility. */
export type ProjectValue = VersionedTimeline;

export interface LoadedProject {
  readonly project: ProjectValue;
  readonly version: 1 | 2;
  readonly migrated: boolean;
}

async function readProjectValue(dir: string): Promise<LoadedProject | undefined> {
  const target = projectPath(dir);
  let raw: string;
  try {
    raw = await readFile(target, "utf-8");
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw new Error(
      `failed to read project at ${target}: ${err instanceof Error ? err.message : String(err)}`,
      { cause: err },
    );
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw) as unknown;
  } catch (err) {
    throw new Error(
      `project at ${target} contains malformed JSON: ${err instanceof Error ? err.message : String(err)}`,
      { cause: err },
    );
  }

  try {
    const project = parseTimelineProject(parsed);
    return {
      project,
      version: project.version === 2 ? 2 : 1,
      migrated: false,
    };
  } catch (err) {
    throw new Error(
      `project at ${target} failed validation: ${err instanceof Error ? err.message : String(err)}`,
      { cause: err },
    );
  }
}

/**
 * Load either the legacy v1 timeline or the v2 project without silently
 * treating malformed files as a missing project.
 */
export async function loadProject(dir: string): Promise<ProjectValue | undefined> {
  return (await readProjectValue(dir))?.project;
}

/** Load a project together with its detected disk version. */
export async function loadProjectDetails(dir: string): Promise<LoadedProject | undefined> {
  return readProjectValue(dir);
}

/**
 * Load a project as the v2 IR. Legacy files are migrated in memory only; the
 * caller decides whether and when that migration should be persisted.
 */
export async function loadProjectV2(dir: string): Promise<Project | undefined> {
  const loaded = await readProjectValue(dir);
  if (!loaded) return undefined;
  return loaded.version === 2
    ? ProjectSchema.parse(loaded.project)
    : migrateV1ToV2(loaded.project);
}

export const loadV2Project = loadProjectV2;

/** Explicitly migrate and persist a legacy project as v2. */
export async function migrateProjectToV2(dir: string): Promise<Project | undefined> {
  const loaded = await readProjectValue(dir);
  if (!loaded) return undefined;
  const project = loaded.version === 2
    ? ProjectSchema.parse(loaded.project)
    : migrateV1ToV2(loaded.project);
  if (loaded.version === 1) await saveProjectV2(dir, project);
  return project;
}

export const migrateProject = migrateProjectToV2;

async function writeProjectAtomic(dir: string, value: unknown): Promise<void> {
  const target = projectPath(dir);
  const parent = path.dirname(target);
  await mkdir(parent, { recursive: true });
  const temp = path.join(
    parent,
    `.${path.basename(target)}.${process.pid}.${randomUUID()}.tmp`,
  );
  try {
    await writeFile(temp, JSON.stringify(value, null, 2), "utf-8");
    await rename(temp, target);
  } catch (err) {
    await unlink(temp).catch(() => {});
    throw err;
  }
}

/** Save either schema version after validating it, using an atomic rename. */
export async function saveProject(dir: string, project: ProjectValue): Promise<void> {
  const validated = parseTimelineProject(project);
  await writeProjectAtomic(dir, validated);
}

/** Save a v2 project explicitly (useful at migration and MCP boundaries). */
export async function saveProjectV2(dir: string, project: Project): Promise<void> {
  await writeProjectAtomic(dir, ProjectSchema.parse(project));
}

export const saveV2Project = saveProjectV2;

/**
 * Return the current end position for placing a newly appended source.
 *
 * Does not use media's render-duration helper: render
 * compilation correctly rejects an empty v2 project, while append operations
 * need an empty project to be a valid starting point and should place their
 * first clip at zero.
 */
export function projectPlacementDuration(project: Project): number {
  let end = typeof project.duration === "number" && Number.isFinite(project.duration) && project.duration >= 0
    ? project.duration
    : 0;
  const endAt = (startValue: unknown, durationValue: unknown): void => {
    const start = typeof startValue === "number" && Number.isFinite(startValue) && startValue >= 0
      ? startValue
      : 0;
    const duration = typeof durationValue === "number" && Number.isFinite(durationValue) && durationValue > 0
      ? durationValue
      : 0;
    end = Math.max(end, start + duration);
  };
  for (const track of project.tracks) {
    if (track.enabled === false) continue;
    for (const clip of track.clips) {
      const start = clip.timelineStart ?? 0;
      let duration = clip.duration;
      if (!(typeof duration === "number" && Number.isFinite(duration) && duration > 0)) {
        const sourceIn = clip.in;
        const sourceOut = clip.out;
        const speed = typeof clip.speed === "number" && Number.isFinite(clip.speed) && clip.speed > 0
          ? clip.speed
          : 1;
        if (
          typeof sourceIn === "number" && Number.isFinite(sourceIn) &&
          typeof sourceOut === "number" && Number.isFinite(sourceOut) &&
          sourceOut > sourceIn
        ) {
          duration = (sourceOut - sourceIn) / speed;
        }
      }
      endAt(start, duration);
    }
    for (const gap of track.gaps ?? []) {
      endAt(gap.start ?? 0, gap.duration);
    }
  }
  for (const gap of project.gaps) {
    endAt(gap.start ?? 0, gap.duration);
  }
  return end;
}

export async function saveTimeline(dir: string, timeline: Timeline): Promise<void> {
  // Validate before touching the existing file. TimelineSchema is the
  // persistence contract and its parsed value is what gets serialized.
  const validated = TimelineSchema.parse(timeline);
  await writeProjectAtomic(dir, validated);
}

export async function loadTimeline(dir: string): Promise<Timeline | undefined> {
  const loaded = await readProjectValue(dir);
  if (!loaded) return undefined;
  if (loaded.version !== 1) {
    throw new Error(
      `project at ${projectPath(dir)} is v2; use loadProject/loadProjectV2 at the v2 boundary`,
    );
  }
  return TimelineSchema.parse(loaded.project);
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
  const duration =
    info.duration !== undefined && Number.isFinite(info.duration) && info.duration > 0
      ? info.duration
      : 60;
  const source = {
    id: srcId,
    path: srcPath,
    duration,
    ...(info.width !== undefined && Number.isFinite(info.width) && info.width > 0
      ? { width: info.width }
      : {}),
    ...(info.height !== undefined && Number.isFinite(info.height) && info.height > 0
      ? { height: info.height }
      : {}),
    ...(info.hasAudio !== undefined ? { hasAudio: info.hasAudio } : {}),
    ...(subs ? { subs } : {}),
  };
  return {
    version: 1,
    fps: 30,
    sources: [source],
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
