#!/usr/bin/env -S npx tsx
/**
 * cut — interactive CLI video editor.
 *
 * Usage:
 *   npx cut [video-or-url] [-k key] [--provider anthropic|openai] [--model m] [--dir d] [-p "instruction"]
 *
 * Providers: anthropic (ANTHROPIC_API_KEY) or any OpenAI-compatible API
 * (OPENAI_API_KEY, optional OPENAI_BASE_URL for OpenRouter/Ollama/LM Studio).
 * Defaults can live in ~/.cut/config.json as { provider, apiKey, model }.
 */
import { createInterface } from "node:readline/promises";
import { readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import process from "node:process";
import { AgentSession, DEFAULT_MODELS, type ProviderName } from "@cut/agent";
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
  provider?: string | undefined;
  model?: string | undefined;
  dir: string;
  cookies?: string | undefined;
  print?: string | undefined;
  listSkills?: boolean;
  listMcp?: boolean;
}

interface ResolvedLlmConfig {
  provider: ProviderName;
  apiKey: string | undefined;
  model: string | undefined;
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
      case "--provider":
        args.provider = argv[++i];
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

/** Read ~/.cut/config.json, tolerating a missing or malformed file. */
async function readCutConfig(): Promise<Record<string, unknown>> {
  try {
    const raw = await readFile(path.join(homedir(), ".cut", "config.json"), "utf-8");
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return {};
  }
}

/**
 * Resolve provider + API key + model, in priority order:
 *   provider: --provider > config.provider > env keys (anthropic wins if both set)
 *   key:      -k flag > provider-specific env > config.apiKey > interactive prompt
 *   model:    --model > config.model > provider default (handled by AgentSession)
 */
export async function resolveLlmConfig(
  explicitKey: string | undefined,
  explicitProvider: string | undefined,
  explicitModel: string | undefined,
  env: NodeJS.ProcessEnv = process.env,
  readConfig: () => Promise<Record<string, unknown>> = readCutConfig,
): Promise<ResolvedLlmConfig> {
  const cfg = await readConfig();
  const cfgProvider = typeof cfg["provider"] === "string" ? (cfg["provider"] as string) : undefined;
  const providerRaw =
    explicitProvider ?? cfgProvider ?? (env["ANTHROPIC_API_KEY"] ? "anthropic" : env["OPENAI_API_KEY"] ? "openai" : undefined);
  if (providerRaw !== undefined && providerRaw !== "anthropic" && providerRaw !== "openai") {
    throw new Error(`unknown provider "${providerRaw}" — expected anthropic or openai`);
  }
  const provider: ProviderName = providerRaw ?? "anthropic";
  const envKey = provider === "openai" ? env["OPENAI_API_KEY"] : env["ANTHROPIC_API_KEY"];
  const cfgKey = typeof cfg["apiKey"] === "string" && cfg["apiKey"] ? (cfg["apiKey"] as string) : undefined;
  let apiKey = explicitKey ?? envKey ?? cfgKey;
  if (!apiKey) {
    const rl = createInterface({ input: process.stdin, output: process.stdout });
    try {
      const key = (await rl.question(`${provider.toUpperCase()}_API_KEY: `)).trim();
      if (!key) throw new Error("no API key provided");
      apiKey = key;
    } finally {
      rl.close();
    }
  }
  const cfgModel = typeof cfg["model"] === "string" && cfg["model"] ? (cfg["model"] as string) : undefined;
  const model = explicitModel ?? cfgModel ?? DEFAULT_MODELS[provider];
  return { provider, apiKey, model };
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
  const llm = await resolveLlmConfig(args.key, args.provider, args.model);
  const baseUrl =
    llm.provider === "openai" ? process.env["OPENAI_BASE_URL"] || undefined : undefined;

  const session = new AgentSession({
    provider: llm.provider,
    ...(llm.apiKey ? { apiKey: llm.apiKey } : {}),
    ...(baseUrl ? { baseUrl } : {}),
    ...(llm.model ? { model: llm.model } : {}),
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
  console.log("\n" + formatBanner(args.dir, skills.length, `${llm.provider} · ${llm.model}`) + "\n");
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
