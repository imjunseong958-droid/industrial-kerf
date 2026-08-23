import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { MessageParam } from "@anthropic-ai/sdk/resources/messages";
import { buildToolsParam, type ToolMap } from "./schema-bridge.js";
import {
  AnthropicProvider,
  OpenAIProvider,
  DEFAULT_MODELS,
  type LlmProvider,
  type ProviderName,
  type AnthropicClientLike,
  type OpenAICompletionsLike,
} from "./provider.js";
import { composeSystemPrompt, type Skill } from "@cut/skills";

export const DEFAULT_MODEL = DEFAULT_MODELS.anthropic;
export { DEFAULT_MODELS };
export type { ProviderName };

export const DEFAULT_SYSTEM_PROMPT = `You are a video editing assistant.
Available tools: fetch (get media), probe (inspect media), edit (modify timeline), render (output video), bash (shell).
The edit tool supports add_subtitles ({ path, style?: { fontSize, fontColor, outlineColor, position: 'bottom'|'center'|'top', marginV } }) to burn in a subtitle file and remove_overlay ({ id }); subtitles are rendered into the output at render time.
Timeline JSON is the single source of truth. Prefer draft renders for preview.`;

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
  /** Injected Anthropic-like client — overrides apiKey. Used by tests with a mock. */
  client?: AnthropicClientLike;
  /** Injected OpenAI chat.completions-like seam — overrides apiKey. */
  openaiClient?: OpenAICompletionsLike;
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
    if (opts.client) {
      this.llm = new AnthropicProvider(opts.client);
      this.provider = "anthropic";
    } else if (opts.openaiClient) {
      this.llm = new OpenAIProvider(opts.openaiClient);
      this.provider = "openai";
    } else if (this.provider === "openai") {
      this.llm = new OpenAIProvider(undefined, {
        ...(opts.apiKey ? { apiKey: opts.apiKey } : {}),
        ...(opts.baseUrl ? { baseURL: opts.baseUrl } : {}),
      });
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
        content: string;
      }>;
      for (const use of toolUses) {
        if (this.aborted) {
          this.emit({ type: "aborted" });
          this.emit({ type: "done", text: finalText });
          return finalText;
        }
        this.emit({ type: "tool_call_started", id: use.id, name: use.name, input: use.input });
        let outputText: string;
        try {
          const impl = this.tools[use.name as keyof typeof this.tools];
          if (!impl) throw new Error(`Unknown tool: ${use.name}`);
          const output = await impl.execute(use.input as never);
          outputText = JSON.stringify(output);
          this.emit({ type: "tool_call_finished", id: use.id, name: use.name, ok: true, result: output });
        } catch (err) {
          outputText = JSON.stringify({
            error: err instanceof Error ? err.message : String(err),
          });
          this.emit({ type: "tool_call_finished", id: use.id, name: use.name, ok: false, error: String(err) });
        }
        resultBlocks.push({
          type: "tool_result",
          tool_use_id: use.id,
          content: outputText,
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
