import type { Timeline } from "@cut/timeline";
import { timelineDuration } from "@cut/media";

const WIDTH = 60;

function fmt(t: number): string {
  return t.toFixed(1).padStart(4, "0");
}

/**
 * Render a text gantt of the first track's clips, scaled to ~60 chars.
 * Example: `v1 |00.0-03.2 ▓▓▓|05.8-12.0 ▓▓▓▓|`
 */
export function renderTimelineGantt(timeline: Timeline): string {
  const track = timeline.tracks[0];
  if (!track || track.clips.length === 0) return "(empty timeline)";
  const total = timelineDuration(timeline);
  const scale = total > 0 ? WIDTH / total : 1;

  const parts: string[] = [];
  let cursor = 0;
  for (const clip of track.clips) {
    const dur = clip.out - clip.in;
    const gap = Math.round((clip.in - cursor) * scale);
    const bar = "▓".repeat(Math.max(1, Math.round(dur * scale)));
    const seg = `${fmt(cursor)}-${fmt(cursor + dur)} ${bar}`;
    parts.push(gap > 0 ? `${"·".repeat(gap)}|${seg}|` : `|${seg}|`);
    cursor += dur;
  }
  return `v1 ${parts.join("")} (total ${total.toFixed(1)}s)`;
}

/** Simple ASCII progress bar for fetch/render. */
export function progressBar(ratio: number, width = 30): string {
  const clamped = Math.min(1, Math.max(0, ratio));
  const filled = Math.round(clamped * width);
  return `[${"█".repeat(filled)}${"░".repeat(width - filled)}] ${(clamped * 100).toFixed(0)}%`;
}
