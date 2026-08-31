import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { MessageParam } from "@anthropic-ai/sdk/resources/messages";
import {
  buildToolsParam,
  type MultimodalToolResult,
  type ToolMap,
  type ToolResultImage,
  type ToolResultImageMediaType,
} from "./schema-bridge.js";
import {
  AnthropicProvider,
  OpenAIProvider,
  OpenAIResponsesProvider,
  DEFAULT_MODELS,
  DEFAULT_RESPONSES_MODEL,
  type LlmProvider,
  type ProviderName,
  type OpenAIApiMode,
  type ReasoningEffort,
  type AnthropicClientLike,
  type OpenAICompletionsLike,
  type OpenAIResponsesLike,
} from "./provider.js";
import { composeSystemPrompt, type Skill } from "@cut/skills";

export const DEFAULT_MODEL = DEFAULT_MODELS.anthropic;
export { DEFAULT_MODELS, DEFAULT_RESPONSES_MODEL };
export type { ProviderName, OpenAIApiMode, ReasoningEffort };

export const DEFAULT_SYSTEM_PROMPT = `You are a video editing assistant for Cut.
The project file is project.cut.json (v2).
When present, fetch pulls URL media into the project. probe returns info, silence, scenes, or frames (frames return images). timeline inspects ids and positions before editing. project_edit (the only mutation tool: v2 commands, or legacy op/ops the host translates) writes changes. render with draft=true for a preview, then a full render.
Call skill_read with a skill name to load a playbook body. Use timeline_undo and timeline_redo when present.
If track_objects is present, use it only for cam (streamer camera overlay), chat (chat overlay), and content (main gameplay/stream canvas). Treat content as the primary framing region unless the user asks to keep cam or chat. Never invent weights.
bash is absent unless explicitly enabled.
Inspect the timeline first. Probe frames when a visual decision is needed. Use normalized 0..1 crops. Batch related edits. Draft-render to verify, then run a full render.
Do not call a tool named edit.`;

/** Maximum number of image payloads accepted from one tool result. */
export const MAX_TOOL_RESULT_IMAGES = 8;
/** Maximum decoded bytes accepted for one tool-result image. */
export const MAX_TOOL_RESULT_IMAGE_BYTES = 5 * 1024 * 1024;
/** Maximum decoded bytes accepted across one tool result's images. */
export const MAX_TOOL_RESULT_TOTAL_IMAGE_BYTES = 20 * 1024 * 1024;

export interface AgentEvent {
  type:
    | "text_delta"
    | "tool_call_started"
    | "tool_call_finished"
    | "turn_started"
    | "done"
    | "aborted"
    | "error";
  [key: string]: unknown;
}

export type AgentEventHandler = (event: AgentEvent) => void;

export interface AgentSessionOptions {
  apiKey?: string;
  /** LLM provider when no client is injected. Default: anthropic. */
  provider?: ProviderName;
  /** Base URL override for the OpenAI-compatible provider (OpenRouter, Ollama…). */
  baseUrl?: string;
  /** Injected Anthropic-like client. Overrides apiKey. Used by tests with a mock. */
  client?: AnthropicClientLike;
  /** Injected OpenAI chat.completions-like client. Overrides apiKey. */
  openaiClient?: OpenAICompletionsLike;
  /** Injected OpenAI Responses-like client. Overrides apiKey. */
  openaiResponsesClient?: OpenAIResponsesLike;
  /** OpenAI-compatible wire protocol. Defaults to chat completions. */
  apiMode?: OpenAIApiMode;
  /** Responses API reasoning effort, e.g. `low` for Luna. */
  reasoningEffort?: ReasoningEffort;
  model?: string;
  systemPrompt?: string;
  /** Skills appended to the system prompt as a compact index. */
  skills?: Skill[];
  tools?: ToolMap;
  maxTurns?: number;
}

interface ToolUseBlockLike {
  type: "tool_use";
  id: string;
  name: string;
  input: unknown;
}

interface CanonicalImageBlock {
  type: "image";
  source: {
    type: "base64";
    media_type: ToolResultImageMediaType;
    data: string;
  };
}

type CanonicalToolResultContent =
  | string
  | Array<{ type: "text"; text: string } | CanonicalImageBlock>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isMultimodalToolResult(value: unknown): value is MultimodalToolResult {
  return isRecord(value) && value.type === "multimodal_tool_result" && Array.isArray(value.images);
}

function stringifyJson(value: unknown): string {
  try {
    const result = JSON.stringify(value);
    // JSON.stringify returns undefined for values such as undefined, symbols,
    // and functions. Tool results must always remain a string on the wire.
    return result === undefined ? "null" : result;
  } catch {
    throw new Error("Tool returned non-serializable JSON metadata");
  }
}

function errorText(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  try {
    return String(error);
  } catch {
    return "Unknown tool error";
  }
}

function validateImageMediaType(value: unknown): ToolResultImageMediaType {
  if (value === "image/png" || value === "image/jpeg") return value;
  throw new Error("Tool result image mediaType must be image/png or image/jpeg");
}

function normalizeImage(
  image: ToolResultImage,
  index: number,
  totalBytes: { value: number },
): CanonicalImageBlock {
  if (!isRecord(image) || typeof image.data !== "string" || image.data.length === 0) {
    throw new Error(`Tool result image ${index + 1} must contain non-empty base64 data`);
  }

  let mediaType: ToolResultImageMediaType;
  let encoded = image.data;
  const dataUrl = /^data:(image\/(?:png|jpeg));base64,([\s\S]*)$/i.exec(image.data);
  if (dataUrl) {
    mediaType = validateImageMediaType(dataUrl[1]!.toLowerCase());
    encoded = dataUrl[2]!;
  } else {
    mediaType = validateImageMediaType(image.mediaType);
  }

  const compact = encoded.replace(/\s+/g, "");
  const maxEncodedLength = Math.ceil((MAX_TOOL_RESULT_IMAGE_BYTES * 4) / 3) + 4;
  if (compact.length === 0 || compact.length > maxEncodedLength) {
    throw new Error(
      `Tool result image ${index + 1} exceeds the ${MAX_TOOL_RESULT_IMAGE_BYTES} byte limit`,
    );
  }
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(compact) || compact.length % 4 === 1) {
    throw new Error(`Tool result image ${index + 1} is not valid base64`);
  }

  const decoded = Buffer.from(compact, "base64");
  const canonical = decoded.toString("base64").replace(/=+$/, "");
  const supplied = compact.replace(/=+$/, "");
  if (decoded.length === 0 || canonical !== supplied) {
    throw new Error(`Tool result image ${index + 1} is not valid base64`);
  }
  if (decoded.length > MAX_TOOL_RESULT_IMAGE_BYTES) {
    throw new Error(
      `Tool result image ${index + 1} exceeds the ${MAX_TOOL_RESULT_IMAGE_BYTES} byte limit`,
    );
  }
  totalBytes.value += decoded.length;
  if (totalBytes.value > MAX_TOOL_RESULT_TOTAL_IMAGE_BYTES) {
    throw new Error(
      `Tool result images exceed the ${MAX_TOOL_RESULT_TOTAL_IMAGE_BYTES} byte total limit`,
    );
  }

  return {
    type: "image",
    source: { type: "base64", media_type: mediaType, data: compact },
  };
}

function normalizeMultimodalToolResult(result: MultimodalToolResult): CanonicalToolResultContent {
  if (result.images.length === 0) {
    throw new Error("A multimodal tool result must contain at least one image");
  }
  if (result.images.length > MAX_TOOL_RESULT_IMAGES) {
    throw new Error(`A tool result may contain at most ${MAX_TOOL_RESULT_IMAGES} images`);
  }
  if (result.text !== undefined && typeof result.text !== "string") {
    throw new Error("Multimodal tool-result text must be a string");
  }

  const textParts: string[] = [];
  if (result.text) textParts.push(result.text);
  if (result.metadata !== undefined) {
    textParts.push(
      typeof result.metadata === "string" ? result.metadata : stringifyJson(result.metadata),
    );
  }
  if (textParts.length === 0) {
    textParts.push(`Tool returned ${result.images.length} image(s).`);
  }

  const totalBytes = { value: 0 };
  const imageBlocks = result.images.map((image, index) =>
    normalizeImage(image, index, totalBytes),
  );
  return [{ type: "text", text: textParts.join("\n") }, ...imageBlocks];
}

function normalizeToolResult(output: unknown): CanonicalToolResultContent {
  return isMultimodalToolResult(output) ? normalizeMultimodalToolResult(output) : stringifyJson(output);
}

function isToolUse(block: unknown): boolean {
  return (
    typeof block === "object" &&
    block !== null &&
    (block as { type?: unknown }).type === "tool_use"
  );
}

/**
 * One agentic turn loop over a pluggable LLM provider
 * ('anthropic' or any OpenAI-compatible endpoint) with tool use.
 */
export class AgentSession {
  readonly model: string;
  readonly provider: ProviderName;
  /** OpenAI wire protocol selected for this session, when applicable. */
  readonly apiMode: OpenAIApiMode | undefined;
  /** Responses API reasoning effort selected for this session, when applicable. */
  readonly reasoningEffort: ReasoningEffort | undefined;
  readonly systemPrompt: string;
  messages: MessageParam[] = [];
  private readonly tools: ToolMap;
  private readonly normalizedTools;
  private readonly maxTurns: number;
  private readonly llm: LlmProvider;
  private handlers = new Set<AgentEventHandler>();
  private aborted = false;

  /** Request cancellation of the current run(); takes effect between steps. */
  abort(): void {
    this.aborted = true;
  }

  get isAborted(): boolean {
    return this.aborted;
  }

  constructor(opts: AgentSessionOptions) {
    this.provider = opts.provider ?? "anthropic";
    this.apiMode = undefined;
    this.reasoningEffort = undefined;
    if (opts.client) {
      this.llm = new AnthropicProvider(opts.client);
      this.provider = "anthropic";
    } else if (opts.apiMode === "responses" || opts.openaiResponsesClient) {
      this.llm = new OpenAIResponsesProvider(opts.openaiResponsesClient, {
        ...(opts.apiKey ? { apiKey: opts.apiKey } : {}),
        ...(opts.baseUrl ? { baseURL: opts.baseUrl } : {}),
        ...(opts.reasoningEffort ? { reasoningEffort: opts.reasoningEffort } : {}),
      });
      this.provider = "openai";
      this.apiMode = "responses";
      this.reasoningEffort = opts.reasoningEffort;
    } else if (opts.openaiClient) {
      this.llm = new OpenAIProvider(opts.openaiClient);
      this.provider = "openai";
      this.apiMode = "chat";
    } else if (this.provider === "openai") {
      this.llm = new OpenAIProvider(undefined, {
        ...(opts.apiKey ? { apiKey: opts.apiKey } : {}),
        ...(opts.baseUrl ? { baseURL: opts.baseUrl } : {}),
      });
      const effectiveApiMode: OpenAIApiMode = opts.apiMode ?? "chat";
      this.apiMode = effectiveApiMode;
      // The Responses branch above handles `apiMode: "responses"`; reaching
      // this branch means the chat protocol is selected.
      this.reasoningEffort = undefined;
    } else {
      this.llm = new AnthropicProvider(undefined, opts.apiKey);
    }
    this.model = opts.model ?? this.llm.defaultModel;
    this.systemPrompt = composeSystemPrompt(opts.systemPrompt ?? DEFAULT_SYSTEM_PROMPT, opts.skills ?? []);
    this.tools = opts.tools ?? {};
    this.normalizedTools = buildToolsParam(this.tools);
    this.maxTurns = opts.maxTurns ?? 32;
  }

  onEvent(handler: AgentEventHandler): () => void {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  private emit(event: AgentEvent): void {
    for (const h of this.handlers) h(event);
  }

  /** Run the agent loop for a user message until a text-only response. */
  async run(userMessage: string): Promise<string> {
    this.messages.push({ role: "user", content: userMessage });
    this.aborted = false;
    let finalText = "";

    for (let turn = 0; turn < this.maxTurns; turn++) {
      if (this.aborted) {
        this.emit({ type: "aborted" });
        break;
      }
      // Stream the model response through the normalized provider event flow.
      let stopReason = "end_turn";
      let contentBlocks: unknown[] = [];
      for await (const chunk of this.llm.stream({
        model: this.model,
        system: this.systemPrompt,
        tools: this.normalizedTools as never,
        messages: [...this.messages],
      })) {
        if (chunk.type === "text_delta") {
          this.emit({ type: "text_delta", text: chunk.text });
          continue;
        }
        stopReason = chunk.stopReason;
        contentBlocks = chunk.content;
      }

      this.emit({ type: "turn_started", stopReason });
      this.messages.push({
        role: "assistant",
        content: contentBlocks as MessageParam["content"],
      });

      const textParts: string[] = [];
      for (const block of contentBlocks) {
        if (
          typeof block === "object" &&
          block !== null &&
          (block as { type?: unknown }).type === "text" &&
          typeof (block as { text?: unknown }).text === "string"
        ) {
          textParts.push((block as { text: string }).text);
        }
      }

      if (stopReason !== "tool_use") {
        finalText += textParts.join("");
        break;
      }

      // Execute each requested tool call and append results.
      const toolUses = contentBlocks.filter(isToolUse) as unknown as ToolUseBlockLike[];
      const results: MessageParam[] = [
        {
          role: "user",
          content: [] as never[],
        },
      ];
      const resultBlocks = results[0]!.content as Array<{
        type: "tool_result";
        tool_use_id: string;
        content: CanonicalToolResultContent;
      }>;
      for (const use of toolUses) {
        if (this.aborted) {
          this.emit({ type: "aborted" });
          this.emit({ type: "done", text: finalText });
          return finalText;
        }
        this.emit({ type: "tool_call_started", id: use.id, name: use.name, input: use.input });
        let outputContent: CanonicalToolResultContent;
        try {
          const impl = this.tools[use.name as keyof typeof this.tools];
          if (!impl) throw new Error(`Unknown tool: ${use.name}`);
          const output = await impl.execute(use.input as never);
          outputContent = normalizeToolResult(output);
          this.emit({ type: "tool_call_finished", id: use.id, name: use.name, ok: true, result: output });
        } catch (err) {
          const message = errorText(err);
          outputContent = stringifyJson({ error: message });
          this.emit({ type: "tool_call_finished", id: use.id, name: use.name, ok: false, error: message });
        }
        resultBlocks.push({
          type: "tool_result",
          tool_use_id: use.id,
          content: outputContent,
        });
      }
      this.messages.push(...results);
    }

    this.emit({ type: "done", text: finalText });
    return finalText;
  }

  /** Serialize message history to a JSON file. */
  async save(path: string): Promise<void> {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, JSON.stringify(this.messages, null, 2), "utf-8");
  }

  /** Restore message history from a JSON file written by save(). */
  static async load(
    path: string,
    opts: Omit<AgentSessionOptions, never> = {},
  ): Promise<AgentSession> {
    const raw = await readFile(path, "utf-8");
    const session = new AgentSession(opts);
    session.messages = JSON.parse(raw) as MessageParam[];
    return session;
  }
}
