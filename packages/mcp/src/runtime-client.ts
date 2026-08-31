import { createInterface, type Interface } from "node:readline";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import type { ProjectCommand, Project } from "@cut/timeline";

export const CUT_RUNTIME_PROTOCOL_VERSION = 1 as const;

export class RuntimeClientError extends Error {
  readonly code?: string;
  readonly details?: unknown;
  constructor(message: string, code?: string, details?: unknown) {
    super(message);
    this.name = "RuntimeClientError";
    this.code = code;
    this.details = details;
    const record = details && typeof details === "object" ? details as Record<string, unknown> : undefined;
    if (record?.expected_revision !== undefined) Object.defineProperty(this, "expectedRevision", { value: record.expected_revision });
    if (record?.actual_revision !== undefined) Object.defineProperty(this, "actualRevision", { value: record.actual_revision });
  }
}

export interface RuntimeState {
  project: Project;
  revision: number;
  canUndo: boolean;
  canRedo: boolean;
  duration: number;
  sources: Project["sources"];
}

export interface RuntimeClientOptions {
  readonly binary: string;
  readonly dir: string;
}

export function resolveCutRuntimeBinary(): string {
  const configured = process.env.CUT_RUNTIME_BIN;
  if (configured) {
    if (existsSync(configured)) return configured;
    throw new Error(`CUT_RUNTIME_BIN points to a missing cut-runtime binary: ${configured}`);
  }
  // Resolve from the package location as well as cwd. CLI/MCP callers often
  // run from a project directory, so cwd is not a reliable repository root.
  const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
  const repositoryRoot = path.resolve(packageRoot, "..");
  const cwd = process.cwd();
  const roots = [repositoryRoot, cwd].filter((root, index, all) => all.indexOf(root) === index);
  const candidates = roots.flatMap((root) => [
    path.join(root, "packages/desktop/src-tauri/target/debug/cut-runtime"),
    path.join(root, "packages/desktop/src-tauri/target/release/cut-runtime"),
    path.join(root, "target/debug/cut-runtime"),
    path.join(root, "target/release/cut-runtime"),
  ]);
  const existing = candidates.find((candidate) => {
    try { return existsSync(candidate); } catch { return false; }
  });
  if (existing) return existing;
  throw new Error(
    `Cut runtime is unavailable. Build packages/desktop/src-tauri/bin/cut-runtime `
    + `or set CUT_RUNTIME_BIN to an executable path (searched ${candidates.join(", ")}).`,
  );
}

export function createCutRuntimeClient(dir: string): CutRuntimeClient {
  return new CutRuntimeClient({ binary: resolveCutRuntimeBinary(), dir });
}

type RuntimeResponse = {
  protocolVersion: number;
  id: string;
  ok: boolean;
  state?: RuntimeState;
  error?: { code?: string; message?: string; [key: string]: unknown };
};

type Pending = { resolve: (response: RuntimeResponse) => void; reject: (error: Error) => void };

/** Persistent JSONL client for the Rust runtime daemon. */
export interface RuntimeClient {
  getState(): Promise<RuntimeState>;
  apply(actor: string, baseRevision: number, idempotencyKey: string, command: ProjectCommand): Promise<RuntimeState>;
  undo(actor: string, baseRevision: number, idempotencyKey: string): Promise<RuntimeState>;
  redo(actor: string, baseRevision: number, idempotencyKey: string): Promise<RuntimeState>;
  importMedia(actor: string, baseRevision: number, idempotencyKey: string, path: string, options?: { metadata?: Record<string, unknown>; overlay?: Record<string, unknown> }): Promise<{ state: RuntimeState; sourceId: string; clipId: string }>;
  close(): void;
}

export class CutRuntimeClient implements RuntimeClient {
  private child: ChildProcessWithoutNullStreams | undefined;
  private lines: Interface | undefined;
  private sequence = 0;
  private readonly pending = new Map<string, Pending>();

  constructor(private readonly options: RuntimeClientOptions) {}

  private ensureProcess(): ChildProcessWithoutNullStreams {
    if (this.child) return this.child;
    const child = spawn(this.options.binary, ["--dir", this.options.dir], { stdio: ["pipe", "pipe", "pipe"] });
    this.child = child;
    this.lines = createInterface({ input: child.stdout });
    this.lines.on("line", (line) => this.handleLine(line));
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += String(chunk);
      if (stderr.length > 8_192) stderr = stderr.slice(-8_192);
    });
    const fail = (error: Error) => {
      for (const pending of this.pending.values()) pending.reject(error);
      this.pending.clear();
      this.child = undefined;
      this.lines?.close();
      this.lines = undefined;
    };
    child.once("error", fail);
    child.once("exit", (code, signal) => {
      const suffix = stderr.trim() ? `: ${stderr.trim()}` : "";
      fail(new Error(`cut-runtime exited (${code ?? signal ?? "unknown"})${suffix}`));
    });
    return child;
  }

  private handleLine(line: string): void {
    let response: RuntimeResponse;
    try { response = JSON.parse(line) as RuntimeResponse; } catch { return; }
    if (response.protocolVersion !== CUT_RUNTIME_PROTOCOL_VERSION || typeof response.id !== "string") return;
    const pending = this.pending.get(response.id);
    if (!pending) return;
    this.pending.delete(response.id);
    pending.resolve(response);
  }

  private request(operation: string, payload: Record<string, unknown> = {}): Promise<RuntimeResponse> {
    const child = this.ensureProcess();
    const id = `request-${this.sequence++}-${randomUUID()}`;
    return new Promise<RuntimeResponse>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      child.stdin.write(`${JSON.stringify({ protocolVersion: CUT_RUNTIME_PROTOCOL_VERSION, id, operation, ...payload })}\n`, (error) => {
        if (error) { this.pending.delete(id); reject(error); }
      });
    }).then((response) => {
      if (!response.ok) throw new RuntimeClientError(response.error?.message ?? response.error?.code ?? "runtime request failed", response.error?.code, response.error);
      return response;
    });
  }

  async getState(): Promise<RuntimeState> {
    const response = await this.request("get_state");
    if (!response.state) throw new Error("runtime returned no state");
    return response.state;
  }

  async apply(actor: string, baseRevision: number, idempotencyKey: string, command: ProjectCommand): Promise<RuntimeState> {
    const response = await this.request("apply", { actor, baseRevision, idempotencyKey, command });
    if (!response.state) throw new Error("runtime returned no state");
    return response.state;
  }

  async undo(actor: string, baseRevision: number, idempotencyKey: string): Promise<RuntimeState> {
    const response = await this.request("undo", { actor, baseRevision, idempotencyKey });
    if (!response.state) throw new Error("runtime returned no state");
    return response.state;
  }

  async redo(actor: string, baseRevision: number, idempotencyKey: string): Promise<RuntimeState> {
    const response = await this.request("redo", { actor, baseRevision, idempotencyKey });
    if (!response.state) throw new Error("runtime returned no state");
    return response.state;
  }

  async importMedia(actor: string, baseRevision: number, idempotencyKey: string, path: string, options: { metadata?: Record<string, unknown>; overlay?: Record<string, unknown> } = {}): Promise<{ state: RuntimeState; sourceId: string; clipId: string }> {
    const response = await this.request("import", {
      actor,
      baseRevision,
      idempotencyKey,
      path,
      ...(options.metadata ? { metadata: options.metadata } : {}),
      ...(options.overlay ? { overlay: options.overlay } : {}),
    });
    if (!response.state) throw new Error("runtime returned no state");
    const sourceId = (response as RuntimeResponse & { sourceId?: unknown }).sourceId;
    const clipId = (response as RuntimeResponse & { clipId?: unknown }).clipId;
    if (typeof sourceId !== "string" || typeof clipId !== "string") throw new Error("runtime returned no import identity");
    return { state: response.state, sourceId, clipId };
  }

  close(): void {
    this.lines?.close();
    this.lines = undefined;
    this.child?.kill();
    this.child = undefined;
  }
}
