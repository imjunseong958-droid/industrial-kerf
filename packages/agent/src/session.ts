import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import Anthropic from "@anthropic-ai/sdk";
import type { MessageParam } from "@anthropic-ai/sdk/resources/messages";
import { buildToolsParam, type ToolMap } from "./schema-bridge.js";

export const DEFAULT_MODEL = "claude-sonnet-4-20250514";

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
    | "error";
  [key: string]: unknown;
}

export type AgentEventHandler = (event: AgentEvent) => void;

export interface AgentSessionOptions {
  apiKey?: string;
  /** Injected client — overrides apiKey. Used by tests with a mock. */
  client?: Pick<typeof Anthropic.prototype.messages, "create">;
  model?: string;
  systemPrompt?: string;
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

function isTextBlock(block: unknown): block is { type: "text"; text: string } {
  return (
    typeof block === "object" &&
    block !== null &&
    (block as { type?: unknown }).type === "text"
  );
}

/** One agentic turn loop over the Anthropic messages API with tool use. */
export class AgentSession {
  readonly model: string;
  readonly systemPrompt: string;
  messages: MessageParam[] = [];
  private readonly tools: ToolMap;
  private readonly anthropicTools;
  private readonly maxTurns: number;
  private readonly client: { create: (typeof Anthropic.prototype.messages)["create"] };
  private handlers = new Set<AgentEventHandler>();

  constructor(opts: AgentSessionOptions) {
    this.client =
      opts.client ?? new Anthropic({ apiKey: opts.apiKey ?? process.env["ANTHROPIC_API_KEY"] }).messages;
    this.model = opts.model ?? DEFAULT_MODEL;
    this.systemPrompt = opts.systemPrompt ?? DEFAULT_SYSTEM_PROMPT;
    this.tools = opts.tools ?? {};
    this.anthropicTools = buildToolsParam(this.tools);
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
    let finalText = "";

    for (let turn = 0; turn < this.maxTurns; turn++) {
      const response = await this.client.create({
        model: this.model,
        max_tokens: 8192,
        system: this.systemPrompt,
        tools: this.anthropicTools,
        messages: [...this.messages],
      });

      this.messages.push({
        role: "assistant",
        // Serialize SDK blocks defensively (mocks / plain objects lack toJSON).
        content: response.content.map((b: unknown) =>
          typeof (b as { toJSON?: unknown }).toJSON === "function"
            ? (b as { toJSON: () => unknown }).toJSON()
            : b,
        ) as MessageParam["content"],
      });
      this.emit({ type: "turn_started", stopReason: response.stop_reason });

      // Emit text deltas.
      let textParts: string[] = [];
      for (const block of response.content) {
        if (isTextBlock(block)) {
          textParts.push(block.text);
          this.emit({ type: "text_delta", text: block.text });
        }
      }

      if (response.stop_reason !== "tool_use") {
        finalText += textParts.join("");
        break;
      }

      // Execute each requested tool call and append results.
      const toolUses = response.content.filter(isToolUse) as unknown as ToolUseBlockLike[];
      const results: MessageParam[] = [
        {
          role: "user",
          content: [] as Anthropic.ToolResultBlockParam[],
        },
      ];
      const resultBlocks = results[0]!.content as Anthropic.ToolResultBlockParam[];
      for (const use of toolUses) {
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
      textParts = [];
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
