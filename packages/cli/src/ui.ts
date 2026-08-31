/**
 * Pure formatting helpers for the cut REPL: tool-call lines, the boxed
 * timeline gantt, and the welcome banner. All functions are side-effect
 * free so they can be unit-tested without a terminal.
 */
import { dim, red, green, bold, cyan, stripAnsi } from "./ansi.js";
import { renderTimelineGantt } from "./gantt.js";
import type { Timeline } from "@cut/timeline";

/** Tool names whose success should trigger a gantt block after the turn. */
export const GANTT_TOOLS = new Set(["project_edit", "render"]);

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/**
 * Compact summary of a tool input, e.g. `{action:silence}` or `{url:https://…}`.
 * Keys are truncated so long inputs never blow up the line.
 */
export function summarizeInput(input: unknown, maxLen = 60): string {
  if (input === undefined || input === null) return "{}";
  let body: string;
  try {
    body = JSON.stringify(input) ?? String(input);
  } catch {
    body = String(input);
  }
  if (body.length > maxLen) body = `${body.slice(0, maxLen - 1)}…`;
  return body;
}

export interface ToolLineParts {
  name: string;
  input?: unknown;
  ok?: boolean;
  durationMs?: number;
  error?: string;
}

/**
 * One-line tool call rendering:
 *   `⏺ probe {action:silence} ... ✓ 340ms`   (dim; green check on success)
 *   `⏺ probe {url:…} ... ✗ boom`             (red on error)
 */
export function formatToolLine(parts: ToolLineParts): string {
  const head = `⏺ ${parts.name} ${summarizeInput(parts.input)} ... `;
  if (parts.ok === false) {
    return red(`${head}✗ ${parts.error ?? "failed"}`);
  }
  const tail = parts.durationMs !== undefined ? `✓ ${formatDuration(parts.durationMs)}` : "…";
  return dim(head) + (parts.ok === true ? green(tail) : dim(tail));
}

/** Wrap text in a box-drawing block with a title label. */
export function boxBlock(title: string, contentLines: string[]): string[] {
  const contentWidth = Math.max(
    title.length + 3,
    ...contentLines.map((l) => stripAnsi(l).length),
  );
  const bodyWidth = contentWidth + 2; // one space of padding per side
  const top = `┌─ ${title} ${"─".repeat(bodyWidth - title.length - 3)}┐`;
  const bottom = `└${"─".repeat(bodyWidth)}┘`;
  const rows = contentLines.map((l) => {
    const pad = " ".repeat(Math.max(0, bodyWidth - stripAnsi(l).length - 2));
    return `│ ${l}${pad} │`;
  });
  return [top, ...rows, bottom];
}

/** Boxed gantt for the current timeline state. */
export function formatGanttBlock(timeline: Timeline): string {
  return boxBlock("timeline", [renderTimelineGantt(timeline)]).join("\n");
}

/** Bilingual welcome banner listing slash commands and example prompts. */
export function formatBanner(dir: string, skillCount = 0, llmInfo?: string): string {
  return [
    bold(cyan("cut")) + dim(`. Agentic video editor · session dir: ${dir}`) +
      (llmInfo ? dim(` · ${llmInfo}`) : "") +
      (skillCount > 0 ? dim(` · ${skillCount} skill${skillCount === 1 ? "" : "s"}`) : ""),
    "",
    bold("Commands") + dim("  /timeline  /save  /quit"),
    bold("예시 프롬프트 · Example prompts") + ":",
    dim('  ❯ "앞부분 5초 잘라내고 자막 넣어줘". Trim the first 5s and add subtitles'),
    dim('  ❯ "make a 15s highlight reel from this video"'),
    dim('  ❯ "silence the audio between 00:10 and 00:20"'),
  ].join("\n");
}
