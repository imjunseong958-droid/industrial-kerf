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
import { initMcpTools } from "@cut/mcp";
import { YtDlpMissingError } from "@cut/fetcher";
import { info } from "@cut/media";
import { loadSkills, BUNDLED_SKILLS_DIR, type Skill } from "@cut/skills";
import { dim, yellow } from "./ansi.js";
import { Spinner } from "./spinner.js";
import { formatToolLine, formatGanttBlock, formatBanner, GANTT_TOOLS } from "./ui.js";
import { buildCutTools } from "./tools.js";
import { initialTimeline, loadTimeline, saveTimeline } from "./state.js";
import { renderTimelineGantt } from "./gantt.js";

interface Args {
  media?: string | undefined;
  key?: string | undefined;
  model?: string | undefined;
  dir: string;
  cookies?: string | undefined;
  print?: string | undefined;
  listSkills?: boolean;
  listMcp?: boolean;
}

/** Skill search order for a session dir: bundled examples < user < project (last wins). */
export function skillDirsFor(dir: string): string[] {
  return [BUNDLED_SKILLS_DIR, path.join(homedir(), ".cut", "skills"), path.join(dir, ".cut", "skills")];
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
      case "--skills":
        args.listSkills = true;
        break;
      case "--mcp":
        args.listMcp = true;
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

  const skills = await loadSkills(skillDirsFor(args.dir));
  if (args.listSkills) {
    if (skills.length === 0) {
      console.log("no skills loaded");
    } else {
      for (const s of skills) console.log(`${s.name} — ${s.description}\n  ${path.join(s.dir, "SKILL.md")}`);
    }
    return;
  }

  // MCP extension: connect configured stdio servers, discover their tools.
  // Failures degrade to warnings; an empty/missing mcp.json is silent.
  const mcpRegistry = await initMcpTools({
    cwd: args.dir,
    ...(args.listMcp ? { log: (l: string) => console.log(l) } : {}),
  });
  if (args.listMcp) {
    const described = mcpRegistry.describe();
    if (described.length === 0) {
      console.log("no MCP tools discovered (no ~/.cut/mcp.json or ./.cut/mcp.json)");
    } else {
      for (const s of described) {
        console.log(`${s.server}:`);
        if (s.tools.length === 0) console.log("  (no tools)");
        for (const t of s.tools) {
          console.log(`  mcp_${s.server}_${t.name}${t.description ? ` — ${t.description}` : ""}`);
        }
      }
    }
    await mcpRegistry.close();
    return;
  }

  await initializeMedia(args.media, args.dir, args.cookies);

  const tools = buildCutTools({ dir: args.dir, ...(args.cookies ? { cookies: args.cookies } : {}) });
  const apiKey = await resolveApiKey(args.key);

  const session = new AgentSession({
    apiKey,
    ...(args.model ? { model: args.model } : {}),
    skills,
    tools: { ...tools, ...mcpRegistry.tools },
  });
  session.onEvent(printEvent as never);

  // Print mode: single-shot instruction.
  if (args.print !== undefined) {
    const text = await session.run(args.print);
    console.log(`\n${text}`);
    return;
  }

  // Interactive REPL.
  const rl = createInterface({
    input: process.stdin,
    output: process.stdout,
    prompt: "❯ ",
    historySize: 200,
  });
  console.log("\n" + formatBanner(args.dir, skills.length) + "\n");
  rl.prompt();

  // Ctrl-C: cancel the in-flight generation; Ctrl-D (EOF) closes the loop.
  let generating = false;
  rl.on("SIGINT", () => {
    if (generating) {
      session.abort();
      process.stdout.write(dim("\n  cancelling…\n"));
    } else {
      process.stdout.write("\n");
      rl.prompt();
    }
  });

  for await (const raw of rl) {
    const line = raw.trim();
    if (line === "/quit" || line === "/q" || line === "exit") break;
    if (line === "/timeline") {
      try {
        const tl = await loadTimeline(args.dir);
        if (!tl) throw new Error("none");
        console.log(formatGanttBlock(tl));
      } catch {
        console.log("no project timeline yet");
      }
      rl.prompt();
      continue;
    }
    if (line === "/save") {
      await session.save(path.join(args.dir, "conversation.json"));
      console.log("saved conversation.json");
      rl.prompt();
      continue;
    }
    if (line === "") {
      rl.prompt();
      continue;
    }

    generating = true;
    const spinner = new Spinner();
    spinner.start("thinking…");
    const startedCalls = new Map<string, { name: string; input: unknown; t: number }>();
    let sawText = false;
    let ganttDue = false;
    const off = session.onEvent((event) => {
      switch (event["type"]) {
        case "text_delta":
          if (!sawText) {
            spinner.stop();
            sawText = true;
          }
          process.stdout.write(String(event["text"]));
          break;
        case "tool_call_started":
          startedCalls.set(String(event["id"]), {
            name: String(event["name"]),
            input: event["input"],
            t: Date.now(),
          });
          spinner.update(`running ${String(event["name"])}…`);
          break;
        case "tool_call_finished": {
          const s = startedCalls.get(String(event["id"]));
          const ok = event["ok"] !== false;
          console.log(
            "\n" +
              formatToolLine({
                name: String(event["name"]),
                ...(s ? { input: s.input } : {}),
                ok,
                ...(s ? { durationMs: Date.now() - s.t } : {}),
                ...(ok ? {} : { error: String(event["error"] ?? "failed") }),
              }),
          );
          if (ok && GANTT_TOOLS.has(String(event["name"]))) ganttDue = true;
          break;
        }
        case "error":
          spinner.stop();
          console.log("\n" + yellow(`error: ${String(event["error"] ?? event)}`));
          break;
        default:
          break;
      }
    });
    try {
      await session.run(line);
    } finally {
      off();
      spinner.stop();
      generating = false;
    }
    if (ganttDue && !session.isAborted) {
      const tl = await loadTimeline(args.dir);
      if (tl) console.log("\n" + dim(formatGanttBlock(tl)));
    }
    console.log();
    rl.prompt();
  }
  console.log(dim("bye 👋"));
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
