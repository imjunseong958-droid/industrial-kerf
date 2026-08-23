/** Tiny zero-dependency ANSI styling helpers for the cut CLI. */

const esc = (code: string, text: string): string => `\x1b[${code}m${text}\x1b[0m`;

export const BOLD = "1";
export const DIM = "2";
export const RED = "31";
export const GREEN = "32";
export const YELLOW = "33";
export const BLUE = "34";
export const MAGENTA = "35";
export const CYAN = "36";

function style(code: string): (text: string) => string {
  return (text: string) => esc(code, text);
}

export const bold = style(BOLD);
export const dim = style(DIM);
export const red = style(RED);
export const green = style(GREEN);
export const yellow = style(YELLOW);
export const blue = style(BLUE);
export const magenta = style(MAGENTA);
export const cyan = style(CYAN);

/** True when stdout will mangle escape codes (not a TTY or color explicitly off). */
export function colorEnabled(stream: { isTTY?: boolean } = process.stdout, env = process.env): boolean {
  if (env["NO_COLOR"] !== undefined && env["NO_COLOR"] !== "") return false;
  if (env["FORCE_COLOR"] === "0") return false;
  return stream.isTTY === true;
}

/** Remove all SGR escape sequences from a string. */
export function stripAnsi(text: string): string {
  // eslint-disable-next-line no-control-regex
  return text.replace(/\x1b\[[0-9;]*m/g, "");
}
