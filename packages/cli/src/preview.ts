/**
 * Terminal preview display of the rendered output.
 *
 * Detection order:
 * 1. kitty terminal → `kitty +kitten icat <frame.png>`
 * 2. iTerm2 (TERM_PROGRAM=iTerm.app) → inline image escape sequence
 * 3. macOS darwin fallback → extract middle frame PNG and `open` it
 * 4. otherwise → just print the path
 */
import { spawn, execFile } from "node:child_process";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { extractFrames, info } from "@cut/media";

function isKitty(): boolean {
  return process.env["KITTY_WINDOW_ID"] != null || (process.env["TERM"] ?? "").includes("kitty");
}
function isITerm(): boolean {
  return process.env["TERM_PROGRAM"] === "iTerm.app";
}

/** Extract the middle frame of `video` to outDir and return its path. */
async function extractMiddleFrame(video: string, outDir: string): Promise<string | undefined> {
  try {
    const meta = await info(video);
    const duration = meta.duration ?? 10;
    const frames = await extractFrames(video, [duration / 2], outDir);
    return frames[0]?.path;
  } catch {
    return undefined;
  }
}

function runCapture(bin: string, args: string[]): Promise<boolean> {
  return new Promise((resolve) => {
    const child = spawn(bin, args, { stdio: "inherit" });
    child.on("error", () => resolve(false));
    child.on("close", (code) => resolve(code === 0));
  });
}

/** Best-effort inline image display; returns true when something was shown. */
export async function showFrame(video: string): Promise<"inline" | "opened" | "path"> {
  if (isKitty()) {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-frame-"));
    const frame = await extractMiddleFrame(video, dir);
    if (frame && (await runCapture("kitty", ["+kitten", "icat", frame]))) {
      return "inline";
    }
  }
  if (isITerm()) {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-frame-"));
    const frame = await extractMiddleFrame(video, dir);
    if (frame) {
      const { readFile } = await import("node:fs/promises");
      const b64 = (await readFile(frame)).toString("base64");
      process.stdout.write(`\x1b]1337;File=inline=1;size=${b64.length}:${b64}\x07\n`);
      return "inline";
    }
  }
  if (process.platform === "darwin") {
    const dir = await mkdtemp(path.join(tmpdir(), "cut-frame-"));
    const frame = await extractMiddleFrame(video, dir);
    if (frame) {
      try {
        await new Promise<void>((resolve, reject) =>
          execFile("open", [frame], (e) => (e ? reject(e) : resolve())),
        );
        console.log(`preview frame opened: ${frame}`);
        return "opened";
      } catch {
        /* fall through */
      }
    }
  }
  console.log(`preview frame available at render output: ${video}`);
  return "path";
}
