import { describe, it, expect, vi } from "vitest";
import { stripAnsi, dim, red, green, bold } from "../src/ansi.js";
import { spinnerFrame, SPINNER_FRAMES, Spinner } from "../src/spinner.js";
import {
  formatDuration,
  summarizeInput,
  formatToolLine,
  boxBlock,
  formatGanttBlock,
  formatBanner,
  GANTT_TOOLS,
} from "../src/ui.js";
import { initialTimeline } from "../src/state.js";
import type { Timeline } from "@cut/timeline";

describe("ansi helpers", () => {
  it("wraps text in SGR codes", () => {
    expect(stripAnsi(bold("hi"))).toBe("hi");
    expect(bold("hi")).toContain("\x1b[1m");
    expect(dim("x")).toContain("\x1b[2m");
    expect(red("x")).toContain("\x1b[31m");
    expect(green("x")).toContain("\x1b[32m");
  });

  it("stripAnsi removes all escape sequences", () => {
    expect(stripAnsi("\x1b[1;32mmixed\x1b[0m plain")).toBe("mixed plain");
    expect(stripAnsi("clean")).toBe("clean");
  });
});

describe("spinner", () => {
  it("cycles braille frames with wraparound", () => {
    expect(spinnerFrame(0)).toBe(SPINNER_FRAMES[0]);
    expect(spinnerFrame(1)).toBe(SPINNER_FRAMES[1]);
    expect(spinnerFrame(SPINNER_FRAMES.length)).toBe(SPINNER_FRAMES[0]);
    expect(spinnerFrame(-1)).toBe(SPINNER_FRAMES[SPINNER_FRAMES.length - 1]);
  });

  it("stays silent on non-TTY streams", () => {
    const stream = { write: vi.fn(), isTTY: false } as never as NodeJS.WriteStream;
    const s = new Spinner({ stream });
    s.start();
    expect(s.running).toBe(false);
    expect(stream.write).not.toHaveBeenCalled();
  });

  it("renders frame + label and clears on stop", () => {
    const writes: string[] = [];
    const stream = { write: (s: string) => (writes.push(s), true), isTTY: true } as never as NodeJS.WriteStream;
    const s = new Spinner({ stream, intervalMs: 5 });
    s.start("thinking…");
    expect(s.running).toBe(true);
    expect(stripAnsi(s.render())).toContain("thinking…");
    s.stop();
    expect(s.running).toBe(false);
    expect(writes.at(-1)).toBe("\r\x1b[2K"); // line cleared
  });
});

describe("tool call line formatting", () => {
  it("formats a successful call with duration", () => {
    const line = formatToolLine({ name: "probe", input: { action: "silence" }, ok: true, durationMs: 340 });
    expect(stripAnsi(line)).toBe("⏺ probe {\"action\":\"silence\"} ... ✓ 340ms");
    expect(line).toContain("\x1b[2m"); // dim
    expect(line).toContain("\x1b[32m"); // green check
  });

  it("formats a failed call in red with the error", () => {
    const line = formatToolLine({ name: "render", input: {}, ok: false, error: "ffmpeg missing" });
    expect(stripAnsi(line)).toContain("✗ ffmpeg missing");
    expect(line).toContain("\x1b[31m"); // red
    expect(line).not.toContain("✓");
  });

  it("truncates long inputs", () => {
    expect(summarizeInput({ url: "x".repeat(200) }).length).toBeLessThanOrEqual(61);
    expect(summarizeInput(undefined)).toBe("{}");
  });

  it("formatDuration picks ms vs seconds", () => {
    expect(formatDuration(340)).toBe("340ms");
    expect(formatDuration(1500)).toBe("1.5s");
  });
});

describe("gantt block + banner", () => {
  const tl = (duration = 10): Timeline => initialTimeline("src1", "video.mp4", { duration });

  it("wraps the gantt in a box-drawing block", () => {
    const block = formatGanttBlock(tl());
    const lines = block.split("\n");
    expect(lines.length).toBe(3);
    expect(lines[0]).toMatch(/^┌─ timeline ─+┐$/);
    expect(lines[1]!.startsWith("│ v1 ")).toBe(true);
    expect(lines[1]!.endsWith("│")).toBe(true);
    expect(lines[2]).toMatch(/^└─+┘$/);
    // padding keeps every row the same visible width
    expect(stripAnsi(lines[1]!).length).toBe(stripAnsi(lines[0]!).length);
  });

  it("banner lists commands and bilingual examples", () => {
    const banner = formatBanner("/tmp/x");
    expect(banner).toContain("/timeline");
    expect(banner).toContain("/save");
    expect(banner).toContain("/quit");
    expect(banner).toContain("예시 프롬프트");
    expect(banner).toContain("Example prompts");
  });

  it("gantt tools include canonical project edits and render", () => {
    expect([...GANTT_TOOLS].sort()).toEqual(["project_edit", "render"]);
  });

  it("boxBlock pads rows to equal width", () => {
    const lines = boxBlock("t", ["ab", "a"]);
    const widths = lines.map((l) => stripAnsi(l).length);
    expect(new Set(widths).size).toBe(1);
  });
});
