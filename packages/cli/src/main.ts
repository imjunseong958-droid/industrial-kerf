#!/usr/bin/env -S npx tsx
/**
 * cut — interactive CLI video editor.
 *
 * Usage:
 *   npx cut [video-or-url] [-k key] [--model m] [--dir d] [-p "instruction"]
 */
import { createInterface } from "node:readline/promises";
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import process from "node:process";
import { AgentSession } from "@cut/agent";
import { YtDlpMissingError } from "@cut/fetcher";
import { info } from "@cut/media";
import { buildCutTools } from "./tools.js";
import { initialTimeline, loadTimeline, saveTimeline, projectPath } from "./state.js";
import { renderTimelineGantt } from "./gantt.js";

interface Args {
  media?: string | undefined;
  key?: string | undefined;
  model?: string | undefined;
  dir: string;
  cookies?: string | undefined;
  print?: string | undefined;
}

/** Default cookie file used when it exists on disk. */
export const DEFAULT_COOKIES_PATH = path.join(
  homedir(),
  "yolomodellayout",
  "youtube.com_cookies.txt",
);

/**
 * Resolve the cookie file for yt-dlp fetches, in priority order:
 * --cookies flag > CUT_COOKIES env > default user cookie file (if present).
 * Returns undefined when nothing is available; callers proceed without cookies.
 */
export function resolveCookiesPath(
  explicit: string | undefined,
  exists: (p: string) => boolean = existsSync,
  env: NodeJS.ProcessEnv = process.env,
): string | undefined {
  if (explicit) return explicit;
  const fromEnv = env["CUT_COOKIES"];
  if (fromEnv) return fromEnv;
  return exists(DEFAULT_COOKIES_PATH) ? DEFAULT_COOKIES_PATH : undefined;
}

function parseArgs(argv: string[]): Args {
  const args: Args = { dir: process.cwd() };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case "-k":
        args.key = argv[++i];
        break;
      case "--model":
        args.model = argv[++i];
        break;
      case "--dir":
        args.dir = path.resolve(argv[++i] ?? ".");
        break;
      case "--cookies":
        args.cookies = argv[++i];
        break;
      case "-p": {
        const rest = argv.slice(++i).join(" ");
        args.print = rest || undefined;
        i = argv.length;
        break;
      }
      default:
        if (!a!.startsWith("-") && args.media === undefined) args.media = a;
    }
  }
  return args;
}

async function resolveApiKey(explicit?: string): Promise<string> {
  if (explicit) return explicit;
  if (process.env["ANTHROPIC_API_KEY"]) return process.env["ANTHROPIC_API_KEY"];
  try {
    const cfg = JSON.parse(await readFile(path.join(homedir(), ".cut", "config.json"), "utf-8"));
    if (typeof cfg["apiKey"] === "string" && cfg["apiKey"]) return cfg["apiKey"] as string;
  } catch {
    /* no config */
  }
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  try {
    const key = (await rl.question("ANTHROPIC_API_KEY: ")).trim();
    if (!key) throw new Error("no API key provided");
    return key;
  } finally {
    rl.close();
  }
}

async function initializeMedia(media: string | undefined, dir: string, cookies?: string): Promise<void> {
  const tools = buildCutTools({ dir, ...(cookies ? { cookies } : {}) });
  if (media === undefined) return;
  if (/^https?:\/\//i.test(media)) {
    console.log(`fetching ${media} ...`);
    await tools.fetch.execute({ url: media });
    console.log("fetch complete.");
    return;
  }
  const src = path.isAbsolute(media) ? media : path.join(dir, media);
  console.log(`probing ${src} ...`);
  const meta = await info(src);
  const existing = await loadTimeline(dir);
  if (!existing) {
    const subs = src.replace(/\.[^.]+$/, "") + ".vtt";
    const tl = initialTimeline(
      "src1",
      src,
      meta,
      (await import("node:fs")).existsSync(subs) ? subs : undefined,
    );
    await saveTimeline(dir, tl);
  }
  console.log(renderTimelineGantt(existing ?? (await loadTimeline(dir))!));
}

function printEvent(event: Record<string, unknown>): void {
  switch (event["type"]) {
    case "text_delta":
      process.stdout.write(String(event["text"]));
      break;
    case "tool_call_started":
      console.log(
        `\n▸ tool ${String(event["name"])} ${JSON.stringify(event["input"]).slice(0, 120)}`,
      );
      break;
    case "tool_call_finished":
      if (event["ok"] === false) {
        console.log(`✗ tool ${String(event["name"])} failed: ${String(event["error"])}`);
      }
      break;
    case "error":
      console.error(`\nerror: ${String(event["error"] ?? event)}`);
      break;
    default:
      break;
  }
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  args.cookies = resolveCookiesPath(args.cookies);
  console.log(`cut — session dir: ${args.dir}`);

  await initializeMedia(args.media, args.dir, args.cookies);

  const tools = buildCutTools({ dir: args.dir, ...(args.cookies ? { cookies: args.cookies } : {}) });
  const apiKey = await resolveApiKey(args.key);

  const session = new AgentSession({
    apiKey,
    ...(args.model ? { model: args.model } : {}),
    tools,
  });
  session.onEvent(printEvent as never);

  // Print mode: single-shot instruction.
  if (args.print !== undefined) {
    const text = await session.run(args.print);
    console.log(`\n${text}`);
    return;
  }

  // Interactive REPL.
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  console.log("\nCommands: /timeline /save /quit — otherwise just describe your edit.\n");
  // Exit cleanly when stdin closes (EOF / Ctrl-D).
  let eof = false;
  rl.on("close", () => {
    eof = true;
    console.log();
  });
  while (!eof) {
    const line = (await rl.question("cut> ")).trim();
    if (line === "/quit" || line === "/q" || line === "exit") break;
    if (line === "/timeline") {
      try {
        const raw = JSON.parse(await readFile(projectPath(args.dir), "utf-8"));
        console.log(JSON.stringify(raw, null, 2));
      } catch {
        console.log("no project timeline yet");
      }
      continue;
    }
    if (line === "/save") {
      await session.save(path.join(args.dir, "conversation.json"));
      console.log("saved conversation.json");
      continue;
    }
    if (line === "") continue;
    console.log();
    await session.run(line);
    console.log("\n");
  }
  rl.close();
}

main().catch((err) => {
  if (err instanceof YtDlpMissingError) {
    console.error(err.message);
  } else {
    console.error(err instanceof Error ? err.stack : err);
  }
  process.exit(1);
});
