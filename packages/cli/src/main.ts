#!/bin/sh
':' //; exec env PATH="$(dirname "$(realpath "$0")")/../node_modules/.bin:$(dirname "$(realpath "$0")")/../../../node_modules/.bin:$PATH" tsx "$0" "$@"
/**
 * lcut. Interactive CLI video editor.
 *
 * Usage:
 *   npx lcut [video-or-url] [-k key] [--provider anthropic|openai] [--model m] [--dir d] [-p "instruction"]
 *
 * Providers: anthropic (ANTHROPIC_API_KEY) or any OpenAI-compatible API
 * (OPENAI_API_KEY, optional OPENAI_BASE_URL for OpenRouter/Ollama/LM Studio).
 * Defaults can live in ~/.cut/config.json as { provider, apiKey, model }.
 */
import { createInterface } from "node:readline/promises";
import { readFile } from "node:fs/promises";
import { existsSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { spawn, type ChildProcess } from "node:child_process";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  AgentSession,
  DEFAULT_MODELS,
  DEFAULT_RESPONSES_MODEL,
  type OpenAIApiMode,
  type ProviderName,
  type ReasoningEffort,
} from "@cut/agent";
import { createCutMcpProjectHost, createCutMcpRouter, initMcpTools } from "@cut/mcp";
import { fetchVideo, YtDlpMissingError } from "@cut/fetcher";
import { info, timelineDurationV2 } from "@cut/media";
import { ProjectSchema, TimelineSchema } from "@cut/timeline";
import { loadSkills, BUNDLED_SKILLS_DIR, type Skill } from "@cut/skills";
import { dim, yellow } from "./ansi.js";
import { Spinner } from "./spinner.js";
import { formatToolLine, formatGanttBlock, formatBanner, GANTT_TOOLS } from "./ui.js";
import { buildCutTools } from "./tools.js";
import { loadProjectDetails } from "./state.js";
import { renderTimelineGantt } from "./gantt.js";

export interface Args {
  media?: string | undefined;
  key?: string | undefined;
  provider?: string | undefined;
  model?: string | undefined;
  dir: string;
  cookies?: string | undefined;
  print?: string | undefined;
  listSkills?: boolean;
  listMcp?: boolean;
  mcpTools?: boolean;
  terminal?: boolean;
  noOpen?: boolean;
  host?: string | undefined;
  port?: number | undefined;
  help?: boolean;
  version?: boolean;
}

/** Keep the binary's displayed version in sync with packages/cli/package.json. */
export const CUT_VERSION = "0.1.0";

export const HELP_TEXT = `lcut. Agentic video editor

Usage:
  lcut [video-or-url] [options]

GUI options:
  --no-open                do not open the GUI URL automatically
  --host <host>            GUI bind host
  --port <port>            GUI port (0 chooses an available port)

Terminal options:
  --terminal               use the terminal REPL instead of the GUI
  -p "<instruction>"        run one instruction and exit
  --skills                 list loaded skills and exit
  --mcp                    list configured MCP tools and exit
  --mcp-tools              attach inbound MCP to the terminal agent via mcp_search/mcp_call
                           (or set CUT_ENABLE_MCP=1)

Common options:
  -k, --key <key>          LLM API key
  --provider <name>        anthropic | openai
  --model <model>          model id
  --dir <path>             session directory
  --cookies <path>         yt-dlp cookie file
  -h, --help               show this help and exit
  -v, --version            show the version and exit`;

export interface ResolvedLlmConfig {
  provider: ProviderName;
  apiKey: string | undefined;
  model: string | undefined;
  apiMode?: OpenAIApiMode;
  baseUrl?: string;
  reasoningEffort?: ReasoningEffort;
}

/** Public boundary between the CLI and the local GUI server package. */
export interface CutAppStartOptions {
  dir: string;
  media?: string | undefined;
  cookies?: string | undefined;
  host?: string | undefined;
  port?: number | undefined;
  llm?: ResolvedLlmConfig | undefined;
}

/** Handle returned by @cut/app.startCutApp. */
export interface CutAppHandle {
  url: string;
  close(): void | Promise<void>;
}

export type CutAppStarter = (
  options: CutAppStartOptions,
) => CutAppHandle | Promise<CutAppHandle>;

async function startDefaultCutApp(options: CutAppStartOptions): Promise<CutAppHandle> {
  // Keep app loading lazy so --help/--version and terminal-only commands do
  // not require starting or even initializing the HTTP application.
  const { startCutApp } = await import("@cut/app");
  return (startCutApp as unknown as CutAppStarter)(options);
}

export interface GuiLaunchDeps {
  startApp?: CutAppStarter;
  openBrowser?: (url: string) => Promise<void> | void;
  waitForExit?: () => Promise<void>;
  log?: (message: string) => void;
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

function parsePort(value: string | undefined): number {
  if (value === undefined || value.trim() === "") throw new Error("--port requires a number");
  const port = Number(value);
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    throw new Error(`invalid port "${value}", expected an integer from 0 to 65535`);
  }
  return port;
}

/** Parse command-line arguments. GUI is the default unless a terminal-only flag is used. */
export function parseArgs(argv: string[]): Args {
  const args: Args = { dir: process.cwd() };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case "-h":
      case "--help":
        args.help = true;
        break;
      case "-v":
      case "--version":
        args.version = true;
        break;
      case "--terminal":
        args.terminal = true;
        break;
      case "--no-open":
        args.noOpen = true;
        break;
      case "--host":
        args.host = argv[++i];
        if (!args.host) throw new Error("--host requires a value");
        break;
      case "--port":
        args.port = parsePort(argv[++i]);
        break;
      case "-k":
      case "--key":
        args.key = argv[++i];
        if (!args.key) throw new Error(`${a} requires a value`);
        break;
      case "--provider":
        args.provider = argv[++i];
        if (!args.provider) throw new Error("--provider requires a value");
        break;
      case "--model":
        args.model = argv[++i];
        if (!args.model) throw new Error("--model requires a value");
        break;
      case "--dir":
        args.dir = path.resolve(argv[++i] ?? ".");
        break;
      case "--cookies":
        args.cookies = argv[++i];
        if (!args.cookies) throw new Error("--cookies requires a value");
        break;
      case "--skills":
        args.listSkills = true;
        break;
      case "--mcp":
        args.listMcp = true;
        break;
      case "--mcp-tools":
        args.mcpTools = true;
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
  // These commands have text-oriented output and must remain terminal-only.
  if (args.help || args.version || args.print !== undefined || args.listSkills || args.listMcp || args.mcpTools) {
    args.terminal = true;
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
 *   model:    --model > config.model > provider/API-mode default
 *   OpenAI mode/base URL/reasoning: config, with OPENAI_* env overrides for base URL/mode
 */
export async function resolveLlmConfig(
  explicitKey: string | undefined,
  explicitProvider: string | undefined,
  explicitModel: string | undefined,
  env: NodeJS.ProcessEnv = process.env,
  readConfig: () => Promise<Record<string, unknown>> = readCutConfig,
  promptForKey = true,
): Promise<ResolvedLlmConfig> {
  const cfg = await readConfig();
  const cfgProvider = typeof cfg["provider"] === "string" ? (cfg["provider"] as string) : undefined;
  const providerRaw =
    explicitProvider ?? cfgProvider ?? (env["ANTHROPIC_API_KEY"] ? "anthropic" : env["OPENAI_API_KEY"] ? "openai" : undefined);
  if (providerRaw !== undefined && providerRaw !== "anthropic" && providerRaw !== "openai") {
    throw new Error(`unknown provider "${providerRaw}", expected anthropic or openai`);
  }
  const provider: ProviderName = providerRaw ?? "anthropic";
  const envKey = provider === "openai" ? env["OPENAI_API_KEY"] : env["ANTHROPIC_API_KEY"];
  const cfgKey = typeof cfg["apiKey"] === "string" && cfg["apiKey"] ? (cfg["apiKey"] as string) : undefined;
  let apiKey = explicitKey ?? envKey ?? cfgKey;
  if (!apiKey && promptForKey) {
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
  const apiModeRaw =
    env["OPENAI_API_MODE"] ??
    (typeof cfg["apiMode"] === "string" ? (cfg["apiMode"] as string) : undefined);
  if (apiModeRaw !== undefined && apiModeRaw !== "chat" && apiModeRaw !== "responses") {
    throw new Error(`unknown OpenAI API mode "${apiModeRaw}", expected chat or responses`);
  }
  const apiMode: OpenAIApiMode = apiModeRaw === "responses" ? "responses" : "chat";
  const cfgBaseUrl =
    typeof cfg["baseUrl"] === "string" && cfg["baseUrl"]
      ? (cfg["baseUrl"] as string)
      : typeof cfg["baseURL"] === "string" && cfg["baseURL"]
        ? (cfg["baseURL"] as string)
        : undefined;
  const baseUrl = provider === "openai" ? env["OPENAI_BASE_URL"] ?? cfgBaseUrl : undefined;
  const cfgReasoning =
    typeof cfg["reasoningEffort"] === "string" ? (cfg["reasoningEffort"] as string) : undefined;
  const reasoningEffortValues: ReasoningEffort[] = [
    "none",
    "minimal",
    "low",
    "medium",
    "high",
    "xhigh",
    "max",
  ];
  if (cfgReasoning !== undefined && !reasoningEffortValues.includes(cfgReasoning as ReasoningEffort)) {
    throw new Error(`unknown reasoning effort "${cfgReasoning}"`);
  }
  const reasoningEffort =
    provider === "openai" && apiMode === "responses"
      ? (cfgReasoning as ReasoningEffort | undefined)
      : undefined;
  const model =
    explicitModel ??
    cfgModel ??
    (provider === "openai" && apiMode === "responses"
      ? DEFAULT_RESPONSES_MODEL
      : DEFAULT_MODELS[provider]);
  return {
    provider,
    apiKey,
    model,
    ...(provider === "openai"
      ? {
          apiMode,
          ...(baseUrl ? { baseUrl } : {}),
          ...(reasoningEffort ? { reasoningEffort } : {}),
        }
      : {}),
  };
}

/** Normalize local paths for duplicate-source checks without changing storage paths. */
export function sourcePathKey(dir: string, sourcePath: string): string {
  return path.normalize(path.isAbsolute(sourcePath) ? sourcePath : path.resolve(dir, sourcePath));
}

export async function initializeMedia(media: string | undefined, dir: string, cookies?: string): Promise<void> {
  const tools = buildCutTools({ dir, ...(cookies ? { cookies } : {}) });
  if (media === undefined) return;
  if (/^https?:\/\//i.test(media)) {
    console.log(`fetching ${media} ...`);
    await tools.fetch.execute({ url: media });
    console.log("fetch complete.");
    return;
  }
  const src = path.isAbsolute(media) ? media : path.join(dir, media);
  const loaded = await loadProjectDetails(dir);
  if (loaded?.project.sources.some((source) => sourcePathKey(dir, source.path) === sourcePathKey(dir, src))) {
    if (loaded.version === 2) {
      console.log(`v2 project · duration ${timelineDurationV2(ProjectSchema.parse(loaded.project)).toFixed(1)}s`);
    } else {
      console.log(renderTimelineGantt(TimelineSchema.parse(loaded.project)));
    }
    return;
  }
  console.log(`importing ${src} ...`);
  const host = createCutMcpProjectHost({
    dir,
    fetchVideo,
    info,
    ...(cookies ? { cookies } : {}),
  });
  const router = createCutMcpRouter(host.handlers);
  const result = await router.call("media.import", { path: src });
  await host.close();
  if (result.isError) {
    const structured = result.structuredContent as { error?: { message?: unknown } };
    throw new Error(
      structured.error?.message !== undefined
        ? String(structured.error.message)
        : result.content[0]?.text ?? "media.import failed",
    );
  }
  const after = await loadProjectDetails(dir);
  if (after?.version === 2) {
    console.log(`v2 project · duration ${timelineDurationV2(ProjectSchema.parse(after.project)).toFixed(1)}s`);
  }
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

type BrowserSpawn = (
  command: string,
  args: string[],
  options: { detached: boolean; stdio: "ignore" },
) => ChildProcess;

/** Return the platform-specific browser command without invoking a shell. */
export function browserCommandForPlatform(platform: NodeJS.Platform = process.platform): {
  command: string;
  args: (url: string) => string[];
} {
  if (platform === "darwin") return { command: "open", args: (url) => [url] };
  if (platform === "win32") return { command: "cmd", args: (url) => ["/c", "start", "", url] };
  if (platform === "linux" || platform === "freebsd" || platform === "openbsd" || platform === "sunos") {
    return { command: "xdg-open", args: (url) => [url] };
  }
  throw new Error(`automatic browser opening is not supported on ${platform}`);
}

/** Open a URL using the host OS browser, with URL passed as an argument (never shell-interpolated). */
export function openInBrowser(
  url: string,
  platform: NodeJS.Platform = process.platform,
  spawnBrowser: BrowserSpawn = spawn as BrowserSpawn,
): Promise<void> {
  const browser = browserCommandForPlatform(platform);
  return new Promise<void>((resolve, reject) => {
    let child: ChildProcess;
    try {
      child = spawnBrowser(browser.command, browser.args(url), {
        detached: true,
        stdio: "ignore",
      });
    } catch (error) {
      reject(error);
      return;
    }
    // Handle an asynchronous ENOENT without leaving an unhandled ChildProcess error.
    let settled = false;
    child.once("error", (error) => {
      if (!settled) {
        settled = true;
        reject(error);
      }
    });
    child.once("spawn", () => {
      if (!settled) {
        settled = true;
        resolve();
      }
    });
    child.unref();
  });
}

/** Wait until the user asks the GUI server to stop. */
export function waitForGuiSignals(): Promise<void> {
  return new Promise<void>((resolve) => {
    const finish = () => {
      process.removeListener("SIGINT", finish);
      process.removeListener("SIGTERM", finish);
      resolve();
    };
    process.once("SIGINT", finish);
    process.once("SIGTERM", finish);
  });
}

/** Start the local GUI server and keep it alive until SIGINT/SIGTERM. */
export async function runGui(args: Args, deps: GuiLaunchDeps = {}): Promise<void> {
  const log = deps.log ?? ((message: string) => console.log(message));
  const startApp = deps.startApp ?? startDefaultCutApp;

  // Resolving without prompting lets the GUI open immediately. The app's agent
  // endpoint is responsible for returning a useful missing-key error later.
  let llm: ResolvedLlmConfig | undefined;
  try {
    llm = await resolveLlmConfig(args.key, args.provider, args.model, process.env, readCutConfig, false);
  } catch (error) {
    log(`warning: could not resolve saved LLM configuration (${error instanceof Error ? error.message : String(error)})`);
  }

  // The app server imports local files. Preserve the terminal CLI's URL
  // behavior by fetching a remote source first, then let the app migrate the
  // resulting legacy project into its canonical v2 host.
  let media = args.media;
  if (media !== undefined && /^https?:\/\//i.test(media)) {
    await initializeMedia(media, args.dir, args.cookies);
    media = undefined;
  }

  const options: CutAppStartOptions = {
    dir: args.dir,
    ...(media !== undefined
      ? {
          // The app resolves local media independently from its project dir;
          // pass an absolute path so `lcut --dir ./project clip.mp4` behaves
          // the same as the terminal workflow. URLs remain untouched for the
          // app's remote-media handling.
          media: path.resolve(args.dir, media),
        }
      : {}),
    ...(args.cookies !== undefined ? { cookies: args.cookies } : {}),
    ...(args.host !== undefined ? { host: args.host } : {}),
    ...(args.port !== undefined ? { port: args.port } : {}),
    ...(llm ? { llm } : {}),
  };
  const app = await startApp(options);
  if (!app || typeof app.url !== "string" || typeof app.close !== "function") {
    throw new Error("@cut/app.startCutApp returned an invalid app handle (expected { url, close })");
  }

  let closed = false;
  const close = async () => {
    if (closed) return;
    closed = true;
    await app.close();
  };
  try {
    log(`lcut app listening at ${app.url}`);
    if (!args.noOpen) {
      try {
        await (deps.openBrowser ?? openInBrowser)(app.url);
      } catch (error) {
        log(`warning: could not open browser (${error instanceof Error ? error.message : String(error)})`);
        log(`open this URL manually: ${app.url}`);
      }
    }
    await (deps.waitForExit ?? waitForGuiSignals)();
  } finally {
    await close();
  }
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));

  if (args.help) {
    console.log(HELP_TEXT);
    return;
  }
  if (args.version) {
    console.log(CUT_VERSION);
    return;
  }

  args.cookies = resolveCookiesPath(args.cookies);

  if (!args.terminal) {
    await runGui(args);
    return;
  }

  console.log(`lcut, session dir: ${args.dir}`);

  const skills = await loadSkills(skillDirsFor(args.dir));
  if (args.listSkills) {
    if (skills.length === 0) {
      console.log("no skills loaded");
    } else {
      for (const s of skills) console.log(`${s.name} · ${s.description}\n  ${path.join(s.dir, "SKILL.md")}`);
    }
    return;
  }

  // MCP extension: connect configured stdio servers when listing or attaching.
  // Failures degrade to warnings; an empty/missing mcp.json is silent.
  const enableMcpTools = args.mcpTools === true || process.env["CUT_ENABLE_MCP"] === "1";
  const mcpRegistry =
    args.listMcp || enableMcpTools
      ? await initMcpTools({
          cwd: args.dir,
          ...(args.listMcp ? { log: (l: string) => console.log(l) } : {}),
        })
      : undefined;
  try {
    if (args.listMcp) {
      const described = mcpRegistry?.describe() ?? [];
      if (described.length === 0) {
        console.log("no MCP tools discovered (no ~/.cut/mcp.json or ./.cut/mcp.json)");
      } else {
        for (const s of described) {
          console.log(`${s.server}:`);
          if (s.tools.length === 0) console.log("  (no tools)");
          for (const t of s.tools) {
            console.log(`  mcp_${s.server}_${t.name}${t.description ? ` · ${t.description}` : ""}`);
          }
        }
      }
      return;
    }

    await initializeMedia(args.media, args.dir, args.cookies);

  const tools = buildCutTools({
    dir: args.dir,
    skills,
    ...(args.cookies ? { cookies: args.cookies } : {}),
  });
  const llm = await resolveLlmConfig(args.key, args.provider, args.model);
  const session = new AgentSession({
    provider: llm.provider,
    ...(llm.apiKey ? { apiKey: llm.apiKey } : {}),
    ...(llm.baseUrl ? { baseUrl: llm.baseUrl } : {}),
    ...(llm.apiMode ? { apiMode: llm.apiMode } : {}),
    ...(llm.reasoningEffort ? { reasoningEffort: llm.reasoningEffort } : {}),
    ...(llm.model ? { model: llm.model } : {}),
    skills,
    tools: enableMcpTools && mcpRegistry ? { ...tools, ...mcpRegistry.metaTools() } : tools,
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
        const loaded = await loadProjectDetails(args.dir);
        if (!loaded) throw new Error("none");
        if (loaded.version === 2) {
          const project = ProjectSchema.parse(loaded.project);
          console.log(`v2 project · ${project.tracks.length} tracks · ${timelineDurationV2(project).toFixed(1)}s`);
        } else {
          console.log(formatGanttBlock(TimelineSchema.parse(loaded.project)));
        }
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
      const loaded = await loadProjectDetails(args.dir);
      if (loaded?.version === 2) {
        const project = ProjectSchema.parse(loaded.project);
        console.log("\n" + dim(`v2 project · ${project.tracks.length} tracks · ${timelineDurationV2(project).toFixed(1)}s`));
      } else if (loaded) {
        console.log("\n" + dim(formatGanttBlock(TimelineSchema.parse(loaded.project))));
      }
    }
    console.log();
    rl.prompt();
  }
    console.log(dim("bye 👋"));
    rl.close();
  } finally {
    await mcpRegistry?.close();
  }
}

function isDirectInvocation(): boolean {
  const entry = process.argv[1];
  if (!entry) return false;
  try {
    const entryPath = realpathSync(path.resolve(entry));
    const modulePath = realpathSync(fileURLToPath(import.meta.url));
    return entryPath === modulePath || pathToFileURL(entryPath).href === import.meta.url;
  } catch {
    return false;
  }
}

if (isDirectInvocation()) {
  main().catch((err) => {
    if (err instanceof YtDlpMissingError) {
      console.error(err.message);
    } else {
      console.error(err instanceof Error ? err.stack : err);
    }
    process.exit(1);
  });
}
