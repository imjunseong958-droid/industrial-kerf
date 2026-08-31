import Anthropic from "@anthropic-ai/sdk";
import type { MessageParam } from "@anthropic-ai/sdk/resources/messages";
import OpenAI from "openai";

/**
 * Multi-provider LLM abstraction.
 *
 * The session's internal message history is stored in Anthropic message
 * format (MessageParam). This is the serialization used for save/load.
 * Each provider translates that history into its own wire format and
 * normalizes responses back into content blocks + a stop reason, streamed
 * as ProviderChunks.
 */

export type ProviderName = "anthropic" | "openai";

/** Wire protocol used by an OpenAI-compatible endpoint. */
export type OpenAIApiMode = "chat" | "responses";

/** Reasoning effort accepted by the OpenAI Responses API. */
export type ReasoningEffort =
  | "none"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max";

export const DEFAULT_MODELS: Record<ProviderName, string> = {
  anthropic: "claude-sonnet-4-20250514",
  openai: "gpt-4o",
};

/** Default model for an OpenAI Responses API session. */
export const DEFAULT_RESPONSES_MODEL = "gpt-5.6-luna";

/** Normalized tool definition (Anthropic shape used internally). */
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

/** Minimal client matching what AgentSession used to consume directly. */
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
 * text block as a single delta. This preserves the existing mock-client
 * test client exactly.
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

/** Loose client over `openai.chat.completions` so tests can inject a fake. */
export interface OpenAICompletionsLike {
  create(
    params: Record<string, unknown>,
  ): Promise<AsyncIterable<Record<string, unknown>> | Record<string, unknown>>;
}

/** Loose client over `openai.responses` so tests can inject a fake. */
export interface OpenAIResponsesLike {
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

interface ToolResultVisuals {
  text: string;
  imageUrls: string[];
}

function jsonString(value: unknown): string {
  try {
    const result = JSON.stringify(value);
    return result === undefined ? "null" : result;
  } catch {
    return JSON.stringify({ error: "Tool result could not be serialized" });
  }
}

/** Turn an Anthropic base64 image block into an OpenAI data URL. */
function imageBlockToDataUrl(block: Record<string, unknown>): string | undefined {
  const source = block["source"];
  if (typeof source !== "object" || source === null) return undefined;
  const sourceRecord = source as Record<string, unknown>;
  const sourceType = String(sourceRecord["type"] ?? "");
  if (sourceType === "url" && typeof sourceRecord["url"] === "string") {
    return sourceRecord["url"];
  }
  if (sourceType !== "base64" || typeof sourceRecord["data"] !== "string") return undefined;
  const data = sourceRecord["data"];
  if (/^data:image\/(?:png|jpeg);base64,/i.test(data)) return data;
  const mediaType = sourceRecord["media_type"];
  if (mediaType !== "image/png" && mediaType !== "image/jpeg") return undefined;
  return `data:${mediaType};base64,${data}`;
}

function toolResultVisuals(block: Record<string, unknown>): ToolResultVisuals {
  const content = block["content"];
  if (typeof content === "string") return { text: content, imageUrls: [] };
  if (!Array.isArray(content)) {
    return { text: jsonString(content ?? null), imageUrls: [] };
  }

  const textParts: string[] = [];
  const imageUrls: string[] = [];
  for (const part of content as Array<Record<string, unknown>>) {
    if (part["type"] === "text") {
      textParts.push(String(part["text"] ?? ""));
    } else if (part["type"] === "image") {
      const url = imageBlockToDataUrl(part);
      if (url) imageUrls.push(url);
    }
  }
  const text = textParts.join("\n");
  return {
    text: text || (imageUrls.length ? "Tool result includes visual output." : jsonString(content)),
    imageUrls,
  };
}

function visualLinkText(toolUseId: string): string {
  return `Visual output for tool call ${toolUseId} (see the tool result immediately above).`;
}

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
      const visualMessages: OpenAIMessage[] = [];
      for (const b of m.content as unknown as Array<Record<string, unknown>>) {
        if (b.type === "tool_result") {
          const toolUseId = String(b.tool_use_id);
          const visuals = toolResultVisuals(b);
          out.push({
            role: "tool",
            tool_call_id: toolUseId,
            // Chat Completions tool messages are text-only on the broadest
            // compatible surface. Visual blocks are sent in a linked user
            // message immediately after the complete tool-output sequence.
            content: visuals.text,
          });
          if (visuals.imageUrls.length) {
            visualMessages.push({
              role: "user",
              content: [
                { type: "text", text: visualLinkText(toolUseId) },
                ...visuals.imageUrls.map((url) => ({
                  type: "image_url",
                  image_url: { url },
                })),
              ],
            });
          }
        }
      }
      out.push(...visualMessages);
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

/** Convert an Anthropic-format history into Responses API input items. */
export function toOpenAIResponsesInput(messages: MessageParam[]): OpenAIMessage[] {
  const out: OpenAIMessage[] = [];
  for (const m of messages) {
    const blocks = (Array.isArray(m.content)
      ? m.content
      : [{ type: "text", text: String(m.content ?? "") }]) as unknown as Array<
      Record<string, unknown>
    >;

    if (m.role === "user") {
      let text = "";
      for (const b of blocks) {
        if (b.type === "text") text += String(b.text ?? "");
      }
      if (text.trim()) out.push({ role: "user", content: text });
      const visualMessages: OpenAIMessage[] = [];
      for (const b of blocks) {
        if (b.type !== "tool_result") continue;
        const callId = String(b.tool_use_id);
        const visuals = toolResultVisuals(b);
        out.push({
          type: "function_call_output",
          call_id: callId,
          output: visuals.text,
        });
        if (visuals.imageUrls.length) {
          visualMessages.push({
            role: "user",
            content: [
              { type: "input_text", text: visualLinkText(callId) },
              ...visuals.imageUrls.map((image_url) => ({
                type: "input_image",
                image_url,
              })),
            ],
          });
        }
      }
      out.push(...visualMessages);
      continue;
    }

    let text = "";
    for (const b of blocks) {
      if (b.type === "text") text += String(b.text ?? "");
    }
    if (text.trim()) out.push({ role: "assistant", content: text });
    for (const b of blocks) {
      if (b.type !== "tool_use") continue;
      out.push({
        type: "function_call",
        call_id: String(b.id),
        name: String(b.name),
        arguments: JSON.stringify(b.input ?? {}),
      });
    }
  }
  return out;
}

/** Convert a normalized tool definition to Responses function-tool format. */
export function toOpenAIResponsesTool(tool: NormalizedTool): Record<string, unknown> {
  return {
    type: "function",
    name: tool.name,
    description: tool.description,
    parameters: tool.input_schema,
  };
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

interface ResponsesFunctionCall {
  id: string;
  callId: string;
  name: string;
  arguments: string;
}

function responseOutputText(response: Record<string, unknown>): string {
  const parts: string[] = [];
  const output = Array.isArray(response["output"])
    ? (response["output"] as Array<Record<string, unknown>>)
    : [];
  for (const item of output) {
    if (item["type"] !== "message") continue;
    const content = Array.isArray(item["content"])
      ? (item["content"] as Array<Record<string, unknown>>)
      : [];
    for (const part of content) {
      if (part["type"] === "output_text" && typeof part["text"] === "string") {
        parts.push(part["text"]);
      }
    }
  }
  if (parts.length > 0) return parts.join("");
  return typeof response["output_text"] === "string" ? String(response["output_text"]) : "";
}

function responseOutputToBlocks(response: Record<string, unknown>): unknown[] {
  const blocks: unknown[] = [];
  const text = responseOutputText(response);
  if (text) blocks.push({ type: "text", text });
  const output = Array.isArray(response["output"])
    ? (response["output"] as Array<Record<string, unknown>>)
    : [];
  for (const item of output) {
    if (item["type"] !== "function_call") continue;
    let input: unknown = {};
    try {
      input = JSON.parse(String(item["arguments"] ?? "{}"));
    } catch {
      input = {};
    }
    const callId = String(item["call_id"] ?? item["id"] ?? "");
    blocks.push({
      type: "tool_use",
      id: callId || String(item["id"] ?? `call_${item["name"] ?? "tool"}`),
      name: String(item["name"] ?? ""),
      input,
    });
  }
  return blocks;
}

function responseBlocksFromStream(
  text: string,
  calls: Map<string, ResponsesFunctionCall>,
): unknown[] {
  const blocks: unknown[] = [];
  if (text) blocks.push({ type: "text", text });
  for (const call of calls.values()) {
    let input: unknown = {};
    try {
      input = JSON.parse(call.arguments || "{}");
    } catch {
      input = {};
    }
    blocks.push({
      type: "tool_use",
      id: call.callId || call.id || `call_${call.name}`,
      name: call.name,
      input,
    });
  }
  return blocks;
}

function responseFunctionCallFromItem(item: Record<string, unknown>): ResponsesFunctionCall | undefined {
  if (item["type"] !== "function_call") return undefined;
  const id = String(item["id"] ?? item["call_id"] ?? "");
  return {
    id,
    callId: String(item["call_id"] ?? id),
    name: String(item["name"] ?? ""),
    arguments: String(item["arguments"] ?? ""),
  };
}

/**
 * OpenAI Responses API provider (OpenCode Zen and other compatible endpoints).
 * The Responses API has a different tool and streaming-event shape from
 * chat.completions, so it is selected explicitly with `apiMode: "responses"`.
 */
export class OpenAIResponsesProvider implements LlmProvider {
  readonly name = "openai" as const;
  readonly defaultModel = DEFAULT_RESPONSES_MODEL;
  private readonly responses: OpenAIResponsesLike;
  private readonly reasoningEffort: ReasoningEffort | undefined;

  constructor(
    responses?: OpenAIResponsesLike,
    opts?: { apiKey?: string; baseURL?: string; reasoningEffort?: ReasoningEffort },
  ) {
    this.responses =
      responses ??
      (new OpenAI({
        ...(opts?.apiKey ? { apiKey: opts.apiKey } : {}),
        ...(opts?.baseURL ? { baseURL: opts.baseURL } : {}),
      }).responses as unknown as OpenAIResponsesLike);
    this.reasoningEffort = opts?.reasoningEffort;
  }

  async *stream(req: ProviderRequest): AsyncIterable<ProviderChunk> {
    const params: Record<string, unknown> = {
      model: req.model,
      instructions: req.system,
      input: toOpenAIResponsesInput(req.messages),
      ...(req.tools.length
        ? { tools: req.tools.map(toOpenAIResponsesTool), tool_choice: "auto" }
        : {}),
      ...(this.reasoningEffort ? { reasoning: { effort: this.reasoningEffort } } : {}),
      stream: true,
    };
    const result = await this.responses.create(params);

    if (!(Symbol.asyncIterator in (result as object))) {
      const response = result as Record<string, unknown>;
      const content = responseOutputToBlocks(response);
      const text = content.find(
        (b): b is { type: "text"; text: string } =>
          typeof b === "object" && b !== null && (b as { type?: unknown }).type === "text",
      );
      if (text) yield { type: "text_delta", text: text.text };
      yield {
        type: "message_end",
        stopReason: content.some(
          (b) => typeof b === "object" && b !== null && (b as { type?: unknown }).type === "tool_use",
        )
          ? "tool_use"
          : "end_turn",
        content,
      };
      return;
    }

    let textSent = "";
    let finalResponse: Record<string, unknown> | undefined;
    const calls = new Map<string, ResponsesFunctionCall>();
    for await (const event of result as AsyncIterable<Record<string, unknown>>) {
      const type = String(event["type"] ?? "");
      if (type === "error") {
        throw new Error(String(event["message"] ?? "Responses API stream error"));
      }
      if (type === "response.output_text.delta") {
        const delta = String(event["delta"] ?? "");
        if (delta) {
          textSent += delta;
          yield { type: "text_delta", text: delta };
        }
      } else if (type === "response.output_item.added" || type === "response.output_item.done") {
        const item = event["item"];
        if (typeof item === "object" && item !== null) {
          const call = responseFunctionCallFromItem(item as Record<string, unknown>);
          if (call) calls.set(call.callId || call.id, call);
        }
      } else if (type === "response.function_call_arguments.delta") {
        const itemId = String(event["item_id"] ?? event["call_id"] ?? event["output_index"] ?? "0");
        const call =
          calls.get(itemId) ??
          ({ id: itemId, callId: String(event["call_id"] ?? itemId), name: String(event["name"] ?? ""), arguments: "" } satisfies ResponsesFunctionCall);
        call.arguments += String(event["delta"] ?? "");
        calls.set(itemId, call);
      } else if (type === "response.completed") {
        const response = event["response"];
        if (typeof response === "object" && response !== null) {
          finalResponse = response as Record<string, unknown>;
        }
      }
    }

    const content = finalResponse
      ? responseOutputToBlocks(finalResponse)
      : responseBlocksFromStream(textSent, calls);
    // A minimal compatible mock/server may only send response.completed. Keep
    // the event contract useful in that case without duplicating streamed text.
    if (!textSent) {
      const text = content.find(
        (b): b is { type: "text"; text: string } =>
          typeof b === "object" && b !== null && (b as { type?: unknown }).type === "text",
      );
      if (text) yield { type: "text_delta", text: text.text };
    }
    yield {
      type: "message_end",
      stopReason: content.some(
        (b) => typeof b === "object" && b !== null && (b as { type?: unknown }).type === "tool_use",
      )
        ? "tool_use"
        : "end_turn",
      content,
    };
  }
}
