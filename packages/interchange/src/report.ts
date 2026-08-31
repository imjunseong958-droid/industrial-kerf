import type { Project } from "@cut/timeline";
import { clipSourceId, clipTimelineDuration, clipTimelineStart } from "@cut/timeline";
import { createReport, issue } from "./validation.js";
import type { InterchangeDirection, InterchangeIssue, InterchangeReport } from "./types.js";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function nonEmptyArray(value: unknown): value is readonly unknown[] {
  return Array.isArray(value) && value.length > 0;
}

function pathForTrackClip(trackIndex: number, clipIndex: number, field?: string): string {
  return `tracks[${trackIndex}].clips[${clipIndex}]${field === undefined ? "" : `.${field}`}`;
}

function unknownField(value: unknown, field: string): unknown {
  return isRecord(value) ? value[field] : undefined;
}

function inspectTrackPlacement(
  track: Project["tracks"][number],
  trackIndex: number,
  issues: InterchangeIssue[],
): void {
  const events: Array<{ start: number; end: number; path: string }> = [];
  track.clips.forEach((clip, clipIndex) => {
    const duration = clipTimelineDuration(clip);
    if (duration === undefined || duration <= 0) return;
    events.push({
      start: clipTimelineStart(clip),
      end: clipTimelineStart(clip) + duration,
      path: pathForTrackClip(trackIndex, clipIndex, "timelineStart"),
    });
  });
  for (const [gapIndex, gap] of (track.gaps ?? []).entries()) {
    events.push({ start: gap.start ?? 0, end: (gap.start ?? 0) + gap.duration, path: `tracks[${trackIndex}].gaps[${gapIndex}].start` });
  }
  for (const [transitionIndex, transition] of (track.transitions ?? []).entries()) {
    const start = unknownField(transition, "start");
    if (typeof start === "number" && Number.isFinite(start)) {
      events.push({ start, end: start + transition.duration, path: `tracks[${trackIndex}].transitions[${transitionIndex}].start` });
    }
  }
  events.sort((left, right) => left.start - right.start || left.end - right.end || left.path.localeCompare(right.path));
  let cursor = 0;
  let priorPath: string | undefined;
  for (const event of events) {
    if (event.start < cursor - 1e-9) {
      issues.push(
        issue(
          "placement_overlap",
          `Explicit placement at ${event.start} seconds overlaps the preceding track item; OTIO sequencing will shift it to ${cursor} seconds.`,
          "loss",
          event.path,
          { requestedStart: event.start, effectiveStart: cursor, precedingPath: priorPath },
        ),
      );
    }
    if (event.end > cursor) {
      cursor = event.end;
      priorPath = event.path;
    }
  }
}

/**
 * Report v2 features that have no portable OTIO representation. The fields
 * remain in the cut project passed to the bridge, but the report makes any
 * adapter loss explicit to the caller.
 */
export function inspectProjectLosses(project: Project, direction: InterchangeDirection): InterchangeReport {
  const issues: InterchangeIssue[] = [];

  project.tracks.forEach((track, trackIndex) => {
    const trackType = track.type;
    inspectTrackPlacement(track, trackIndex, issues);
    if (trackType === "title" || trackType === "adjustment") {
      issues.push(
        issue(
          "track_type_not_supported",
          `${trackType} tracks do not have a portable OpenTimelineIO equivalent.`,
          "loss",
          `tracks[${trackIndex}]`,
        ),
      );
    }
    if (nonEmptyArray(track.effects)) {
      issues.push(
        issue(
          "effects_not_supported",
          "Track effects are not representable by the portable OTIO mapping.",
          "loss",
          `tracks[${trackIndex}].effects`,
        ),
      );
    }
    if (track.enabled === false || track.muted === true || track.locked === true) {
      issues.push(
        issue(
          "track_state_not_supported",
          "Track enabled/muted/locked state is not represented in portable OTIO output.",
          "loss",
          `tracks[${trackIndex}]`,
        ),
      );
    }
    if (nonEmptyArray(track.markers)) {
      // Markers are mapped, so this is intentionally not a loss.
    }
    if (nonEmptyArray(track.transitions)) {
      for (const [transitionIndex, transition] of track.transitions.entries()) {
        if (typeof transition.type === "string" && transition.type.trim().length === 0) {
          issues.push(
            issue(
              "transition_type_invalid",
              "A transition has no type and cannot be emitted portably.",
              "warning",
              `tracks[${trackIndex}].transitions[${transitionIndex}]`,
            ),
          );
        }
      }
    }

    track.clips.forEach((clip, clipIndex) => {
      const clipPath = pathForTrackClip(trackIndex, clipIndex);
      if (nonEmptyArray(clip.effects)) {
        issues.push(
          issue(
            "effects_not_supported",
            "Clip effects are not representable by the portable OTIO mapping.",
            "loss",
            `${clipPath}.effects`,
          ),
        );
      }
      if (nonEmptyArray(clip.keyframes)) {
        issues.push(
          issue(
            "keyframes_not_supported",
            "Clip keyframes are not representable by the portable OTIO mapping.",
            "loss",
            `${clipPath}.keyframes`,
          ),
        );
      }
      const blend = clip.blend ?? clip.blendMode;
      if (typeof blend === "string" && blend.length > 0) {
        issues.push(
          issue(
            "blend_not_supported",
            `Clip blend mode ${blend} is not portable through the OTIO mapping.`,
            "loss",
            `${clipPath}.blend`,
          ),
        );
      }
      const rawClip = clip as unknown as Record<string, unknown>;
      if (rawClip.crop !== undefined) {
        issues.push(
          issue(
            "crop_not_supported",
            "Clip crop is retained in cut v2 but is not emitted as an OTIO effect.",
            "loss",
            `${clipPath}.crop`,
          ),
        );
      }
      if (clip.volume !== undefined) {
        issues.push(issue("volume_not_supported", "Clip volume is not represented in portable OTIO output.", "loss", `${clipPath}.volume`));
      }
      if (clip.opacity !== undefined) {
        issues.push(issue("opacity_not_supported", "Clip opacity is not represented in portable OTIO output.", "loss", `${clipPath}.opacity`));
      }
      if (clip.speed !== undefined && Math.abs(clip.speed - 1) > 1e-9) {
        issues.push(issue("speed_not_supported", "Clip playback speed is retained in metadata but OTIO clip duration is unchanged.", "loss", `${clipPath}.speed`));
      }
      if (clip.transform !== undefined) {
        issues.push(
          issue(
            "transform_not_supported",
            "Clip transforms are retained in cut v2 but are not emitted as OTIO effects.",
            "loss",
            `${clipPath}.transform`,
          ),
        );
      }
      if (clip.transitions !== undefined && clip.transitions.length > 0) {
        issues.push(
          issue(
            "clip_transitions_not_supported",
            "Clip-local transitions are retained in cut v2 but exported transitions are track-level.",
            "warning",
            `${clipPath}.transitions`,
          ),
        );
      }
      if (clipSourceId(clip) === undefined && trackType !== "title" && trackType !== "adjustment") {
        issues.push(
          issue(
            "missing_source",
            "A media clip has no source reference and cannot be emitted as an OTIO ExternalReference.",
            "loss",
            `${clipPath}.sourceId`,
          ),
        );
      }
      // A source-less clip with a duration is a valid title/adjustment leaf;
      // title text is still not understood by OTIO.
      if (trackType === "title" && (clip.text !== undefined || clip.title !== undefined)) {
        issues.push(
          issue(
            "title_not_supported",
            "Title text has no portable OTIO representation.",
            "loss",
            `${clipPath}.text`,
          ),
        );
      }
      // Explicit starts are the supported placement mechanism; this access is
      // intentional so custom clip aliases are normalized before inspection.
      void clipTimelineStart(clip);
    });
  });

  if (nonEmptyArray(project.effects)) {
    issues.push(issue("effects_not_supported", "Project effects are not portable through OTIO.", "loss", "effects"));
  }
  if (project.width !== undefined || project.height !== undefined) {
    issues.push(issue("project_dimensions_not_supported", "Project dimensions are not represented in portable OTIO output.", "loss", "width"));
  }
  if (nonEmptyArray(project.gaps)) {
    issues.push(issue("project_gaps_not_track_scoped", "Project-level gaps have no OTIO track owner and will not be emitted.", "loss", "gaps"));
  }
  if (nonEmptyArray(project.transitions)) {
    issues.push(issue("project_transitions_not_track_scoped", "Project-level transitions have no OTIO track owner and will not be emitted.", "loss", "transitions"));
  }
  if (nonEmptyArray(project.overlays)) {
    issues.push(issue("overlays_not_supported", "Project overlays have no portable OTIO equivalent.", "loss", "overlays"));
  }
  project.sources.forEach((source, sourceIndex) => {
    if (source.width !== undefined || source.height !== undefined || source.hasAudio !== undefined) {
      issues.push(issue("source_properties_not_supported", "Source dimensions/audio flags are not represented in portable OTIO output.", "loss", `sources[${sourceIndex}]`));
    }
  });
  const inspectMarker = (marker: { color?: string | undefined }, path: string): void => {
    if (direction !== "export") return;
    if (marker.color !== undefined) {
      issues.push(issue("marker_color_not_supported", "Marker color is not written by the OTIO export mapping.", "loss", `${path}.color`));
    }
  };
  project.markers.forEach((marker, markerIndex) => inspectMarker(marker, `markers[${markerIndex}]`));
  project.tracks.forEach((track, trackIndex) => (track.markers ?? []).forEach((marker, markerIndex) => inspectMarker(marker, `tracks[${trackIndex}].markers[${markerIndex}]`)));
  if (isRecord(project.metadata) && isRecord(project.metadata.cut) && project.metadata.cut.keyframes !== undefined) {
    issues.push(issue("keyframes_not_supported", "Metadata keyframes cannot be represented portably in OTIO.", "loss", "metadata.cut.keyframes"));
  }

  return createReport(direction, undefined, issues);
}

export function mergeReports(
  direction: InterchangeDirection,
  adapter: string | undefined,
  ...reports: readonly (InterchangeReport | undefined)[]
): InterchangeReport {
  const issues: InterchangeIssue[] = [];
  for (const report of reports) {
    if (report !== undefined) issues.push(...report.issues);
  }
  return createReport(direction, adapter, issues);
}

/** Convert a bridge-provided report-like value into a validated report. */
export function normalizeBridgeReport(
  value: unknown,
  direction: InterchangeDirection,
  adapter: string | undefined,
): InterchangeReport {
  if (!isRecord(value)) return createReport(direction, adapter);
  const issues: InterchangeIssue[] = [];
  const add = (entry: unknown, fallbackSeverity: "warning" | "loss"): void => {
    if (typeof entry === "string") {
      issues.push(issue("adapter_limitation", entry, fallbackSeverity));
      return;
    }
    if (!isRecord(entry) || typeof entry.message !== "string") return;
    const severity = entry.severity === "loss" || entry.severity === "warning" ? entry.severity : fallbackSeverity;
    const code = typeof entry.code === "string" && entry.code.length > 0 ? entry.code : "adapter_limitation";
    const path = typeof entry.path === "string" ? entry.path : undefined;
    const details = isRecord(entry.details) ? entry.details : undefined;
    issues.push(issue(code, entry.message, severity, path, details));
  };
  if (Array.isArray(value.issues)) for (const entry of value.issues) add(entry, "warning");
  if (Array.isArray(value.warnings)) for (const entry of value.warnings) add(entry, "warning");
  if (Array.isArray(value.losses)) for (const entry of value.losses) add(entry, "loss");
  if (Array.isArray(value.omissions)) for (const entry of value.omissions) add(entry, "loss");
  return createReport(direction, adapter, issues);
}
