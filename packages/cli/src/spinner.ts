/**
 * Braille-dot spinner shown while awaiting an LLM response.
 * The animation itself is trivially testable via Spinner.frame(); the
 * interval-driven rendering is skipped entirely when stdout is not a TTY.
 */
import { dim } from "./ansi.js";

export const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export function spinnerFrame(index: number): string {
  const n = SPINNER_FRAMES.length;
  return SPINNER_FRAMES[((index % n) + n) % n]!;
}

export interface SpinnerOptions {
  stream?: NodeJS.WriteStream;
  intervalMs?: number;
}

export class Spinner {
  private index = 0;
  private label = "";
  private timer: ReturnType<typeof setInterval> | undefined;
  private readonly stream: NodeJS.WriteStream;
  private readonly intervalMs: number;

  constructor(opts: SpinnerOptions = {}) {
    this.stream = opts.stream ?? process.stdout;
    this.intervalMs = opts.intervalMs ?? 80;
  }

  get running(): boolean {
    return this.timer !== undefined;
  }

  /** Current rendered frame text (frame + label). Pure — safe for tests. */
  render(): string {
    return dim(`${spinnerFrame(this.index)} ${this.label}`);
  }

  start(label = "thinking…"): void {
    this.stop();
    this.label = label;
    if (!this.stream.isTTY) return; // piped / non-interactive: stay silent
    this.timer = setInterval(() => {
      this.index += 1;
      this.paint();
    }, this.intervalMs);
    this.paint();
  }

  update(label: string): void {
    this.label = label;
    if (this.running) this.paint();
  }

  /** Clear the spinner line. Safe to call repeatedly. */
  stop(): void {
    if (this.timer !== undefined) {
      clearInterval(this.timer);
      this.timer = undefined;
      this.stream.write("\r\x1b[2K");
    }
  }

  private paint(): void {
    this.stream.write(`\r\x1b[2K${this.render()}`);
  }
}
