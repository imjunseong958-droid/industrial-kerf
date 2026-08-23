import Anthropic from "@anthropic-ai/sdk";
import type { MessageParam } from "@anthropic-ai/sdk/resources/messages";
import OpenAI from "openai";

/**
 * Multi-provider LLM abstraction.
 *
 * The session's internal message history is stored in Anthropic message
 * format (MessageParam) — it is the canonical serialization for save/load.
 * Each provider translates that history into its own wire format and
 * normalizes responses back into content blocks + a stop reason, streamed
 * as ProviderChunks.
 */

export type ProviderName = "anthropic" | "openai";

export const DEFAULT_MODELS: Record<ProviderName, string> = {
  anthropic: "claude-sonnet-4-20250514",
  openai: "gpt-4o",
};

/** Normalized tool definition (Anthropic shape — the internal currency). */
export type NormalizedTool = {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
};

export interface ProviderRequest {
  model: string;
  system: string;
  tools: NormalizedTool[];
  messages: MessageParam[];
}

export type ProviderChunk =
  | { type: "text_delta"; text: string }
  | {
      type: "message_end";
      /** 'tool_use' or 'end_turn' (provider stop reasons normalized). */
      stopReason: string;
      /** Assistant content blocks in Anthropic format (text / tool_use). */
      content: unknown[];
    };

export interface LlmProvider {
  readonly name: ProviderName;
  readonly defaultModel: string;
  stream(req: ProviderRequest): AsyncIterable<ProviderChunk>;
}

/* ------------------------------------------------------------------ *
 * Anthropic provider
 * ------------------------------------------------------------------ */

/** Minimal seam matching what AgentSession used to consume directly. */
export interface AnthropicClientLike {
  create: (typeof Anthropic.prototype.messages)["create"];
}

function serializeBlocks(content: readonly unknown[]): unknown[] {
  return content.map((b: unknown) =>
    typeof (b as { toJSON?: unknown }).toJSON === "function"
      ? (b as { toJSON: () => unknown }).toJSON()
      : b,
  );
}

/**
 * Anthropic provider. Uses the non-streaming messages API and replays each
 * text block as a single delta — this preserves the existing mock-client
 * test seam exactly.
 */
export class AnthropicProvider implements LlmProvider {
  readonly name = "anthropic" as const;
  readonly defaultModel = DEFAULT_MODELS.anthropic;
  private readonly client: AnthropicClientLike;

  constructor(client?: AnthropicClientLike, apiKey?: string) {
    this.client =
      client ??
      new Anthropic({
        ...(apiKey ? { apiKey } : {}),
      }).messages;
  }

  async *stream(req: ProviderRequest): AsyncIterable<ProviderChunk> {
    const response = await this.client.create({
      model: req.model,
      max_tokens: 8192,
      system: req.system,
      tools: req.tools as unknown as Anthropic.Tool[],
      messages: [...req.messages],
    });
    for (const block of response.content) {
      if (
        typeof block === "object" &&
        block !== null &&
        (block as { type?: unknown }).type === "text"
      ) {
        yield { type: "text_delta", text: (block as { text: string }).text };
      }
    }
    yield {
      type: "message_end",
      stopReason: response.stop_reason ?? "end_turn",
      content: serializeBlocks(response.content),
    };
  }
}

/* ------------------------------------------------------------------ *
 * OpenAI-compatible provider
 * ------------------------------------------------------------------ */

/** Loose seam over `openai.chat.completions` so tests can inject a fake. */
export interface OpenAICompletionsLike {
  create(
    params: Record<string, unknown>,
  ): Promise<AsyncIterable<Record<string, unknown>> | Record<string, unknown>>;
}

interface OpenAIToolCall {
  id: string;
  name: string;
  arguments: string;
}

type OpenAIMessage = Record<string, unknown>;

/** Convert an Anthropic tool definition to OpenAI function-tool format. */
export function toOpenAITool(tool: NormalizedTool): Record<string, unknown> {
  return {
    type: "function",
    function: {
      name: tool.name,
      description: tool.description,
      parameters: tool.input_schema,
    },
  };
}

/** Translate the internal Anthropic-format history into chat messages. */
export function toOpenAIMessages(messages: MessageParam[]): OpenAIMessage[] {
  const out: OpenAIMessage[] = [];
  for (const m of messages) {
    if (m.role === "user") {
      if (typeof m.content === "string") {
        out.push({ role: "user", content: m.content });
        continue;
      }
      let text = "";
      for (const b of m.content as unknown as Array<Record<string, unknown>>) {
        if (b.type === "text") text += String(b.text ?? "");
      }
      if (text.trim()) out.push({ role: "user", content: text });
      for (const b of m.content as unknown as Array<Record<string, unknown>>) {
        if (b.type === "tool_result") {
          out.push({
            role: "tool",
            tool_call_id: String(b.tool_use_id),
            content:
              typeof b.content === "string"
                ? b.content
                : JSON.stringify(b.content ?? null),
          });
        }
      }
    } else {
      const blocks = (Array.isArray(m.content)
        ? m.content
        : [{ type: "text", text: String(m.content ?? "") }]) as unknown as Array<
        Record<string, unknown>
      >;
      let text = "";
      const toolCalls: OpenAIToolCall[] = [];
      for (const b of blocks) {
        if (b.type === "text") text += String(b.text ?? "");
        else if (b.type === "tool_use") {
          toolCalls.push({
            id: String(b.id),
            name: String(b.name),
            arguments: JSON.stringify(b.input ?? {}),
          });
        }
      }
      out.push({
        role: "assistant",
        content: text || null,
        ...(toolCalls.length
          ? {
              tool_calls: toolCalls.map((tc) => ({
                id: tc.id,
                type: "function",
                function: { name: tc.name, arguments: tc.arguments },
              })),
            }
          : {}),
      });
    }
  }
  return out;
}

function normalizeStopReason(finishReason: string | null | undefined): string {
  if (finishReason === "tool_calls" || finishReason === "function_call") {
    return "tool_use";
  }
  return "end_turn";
}

/** Convert an assistant chat message back into Anthropic content blocks. */
function assistantToBlocks(message: Record<string, unknown>): unknown[] {
  const blocks: unknown[] = [];
  const text = message["content"];
  if (typeof text === "string" && text) {
    blocks.push({ type: "text", text });
  } else if (Array.isArray(text)) {
    for (const part of text as Array<Record<string, unknown>>) {
      if (part.type === "text" && typeof part.text === "string") {
        blocks.push({ type: "text", text: part.text });
      }
    }
  }
  for (const tc of (message["tool_calls"] ?? []) as Array<Record<string, unknown>>) {
    const fn = (tc["function"] ?? {}) as Record<string, unknown>;
    let input: unknown = {};
    try {
      input = JSON.parse(String(fn.arguments ?? "{}"));
    } catch {
      input = {};
    }
    blocks.push({
      type: "tool_use",
      id: String(tc["id"]),
      name: String(fn.name),
      input,
    });
  }
  return blocks;
}

function extractAssistantMessage(response: Record<string, unknown>): Record<string, unknown> {
  const choices = response["choices"] as Array<Record<string, unknown>> | undefined;
  return ((choices?.[0] ?? {})["message"] ?? {}) as Record<string, unknown>;
}

/**
 * OpenAI-compatible provider (OpenAI, OpenRouter, Ollama/LM Studio, …).
 * Streams chat.completions chunks; tool calls are accumulated from
 * argument deltas and emitted in the final message_end chunk.
 */
export class OpenAIProvider implements LlmProvider {
  readonly name = "openai" as const;
  readonly defaultModel = DEFAULT_MODELS.openai;
  private readonly completions: OpenAICompletionsLike;

  constructor(completions?: OpenAICompletionsLike, opts?: { apiKey?: string; baseURL?: string }) {
    this.completions =
      completions ??
      (new OpenAI({
        ...(opts?.apiKey ? { apiKey: opts.apiKey } : {}),
        ...(opts?.baseURL ? { baseURL: opts.baseURL } : {}),
      }).chat.completions as unknown as OpenAICompletionsLike);
  }

  async *stream(req: ProviderRequest): AsyncIterable<ProviderChunk> {
    const params: Record<string, unknown> = {
      model: req.model,
      messages: [
        { role: "system", content: req.system },
        ...toOpenAIMessages(req.messages),
      ],
      ...(req.tools.length
        ? { tools: req.tools.map(toOpenAITool), tool_choice: "auto" }
        : {}),
      stream: true,
    };
    const result = await this.completions.create(params);

    if (!(Symbol.asyncIterator in (result as object))) {
      // Non-streaming fallback (some compatible servers ignore stream:true).
      const message = extractAssistantMessage(result as Record<string, unknown>);
      const text = message["content"];
      if (typeof text === "string" && text) {
        yield { type: "text_delta", text };
      }
      const finish = ((result as Record<string, unknown>)["choices"] as
        | Array<Record<string, unknown>>
        | undefined)?.[0]?.["finish_reason"];
      yield {
        type: "message_end",
        stopReason: normalizeStopReason(finish as string | null),
        content: assistantToBlocks(message),
      };
      return;
    }

    let textSent = "";
    const toolAcc = new Map<number, OpenAIToolCall>();
    let finishReason: string | null = null;

    for await (const chunk of result as AsyncIterable<Record<string, unknown>>) {
      const choice = (chunk["choices"] as Array<Record<string, unknown>> | undefined)?.[0];
      if (!choice) continue;
      const delta = (choice["delta"] ?? {}) as Record<string, unknown>;
      if (typeof delta["content"] === "string" && delta["content"]) {
        textSent += delta["content"];
        yield { type: "text_delta", text: delta["content"] };
      }
      for (const tc of (delta["tool_calls"] ?? []) as Array<Record<string, unknown>>) {
        const index = Number(tc["index"] ?? 0);
        const fn = (tc["function"] ?? {}) as Record<string, unknown>;
        const acc =
          toolAcc.get(index) ??
          { id: "", name: "", arguments: "" };
        if (typeof tc["id"] === "string") acc.id = tc["id"];
        if (typeof fn.name === "string") acc.name += fn.name;
        if (typeof fn.arguments === "string") acc.arguments += fn.arguments;
        toolAcc.set(index, acc);
      }
      if (choice["finish_reason"]) finishReason = String(choice["finish_reason"]);
    }

    const content: unknown[] = [];
    if (textSent) content.push({ type: "text", text: textSent });
    for (const [, tc] of [...toolAcc.entries()].sort((a, b) => a[0] - b[0])) {
      let input: unknown = {};
      try {
        input = JSON.parse(tc.arguments || "{}");
      } catch {
        input = {};
      }
      content.push({ type: "tool_use", id: tc.id || `call_${tc.name}`, name: tc.name, input });
    }
    yield {
      type: "message_end",
      stopReason: normalizeStopReason(finishReason),
      content,
    };
  }
}
