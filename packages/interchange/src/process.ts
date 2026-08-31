import { spawn } from "node:child_process";
import type { ProcessInvocation, ProcessResult, ProcessRunner, ProcessRunnerLike } from "./types.js";

function withOptional<T extends object, K extends keyof T>(target: T, key: K, value: T[K] | undefined): void {
  if (value !== undefined) target[key] = value;
}

/** Invoke a function runner or an object with a `run` method. */
export function invokeProcessRunner(runner: ProcessRunnerLike, invocation: ProcessInvocation): Promise<ProcessResult> {
  const callable = runner as ProcessRunner;
  if (typeof callable === "function") return Promise.resolve(callable(invocation));
  return Promise.resolve((runner as { run(invocation: ProcessInvocation): PromiseLike<ProcessResult> | ProcessResult }).run(invocation));
}

function abortError(): Error {
  const error = new Error("Process invocation aborted.");
  Object.assign(error, { name: "AbortError", code: "ABORT_ERR" });
  return error;
}

/** Default runner. It never installs OTIO or downloads an adapter. */
export function createNodeProcessRunner(): ProcessRunner {
  return (invocation) =>
    new Promise<ProcessResult>((resolve, reject) => {
      if (invocation.signal?.aborted) {
        reject(abortError());
        return;
      }

      const spawnOptions: Parameters<typeof spawn>[2] = {
        stdio: ["pipe", "pipe", "pipe"],
      };
      withOptional(spawnOptions, "cwd", invocation.cwd);
      if (invocation.env !== undefined) spawnOptions.env = { ...process.env, ...invocation.env };

      let child: ReturnType<typeof spawn>;
      try {
        child = spawn(invocation.command, [...invocation.args], spawnOptions);
      } catch (error) {
        reject(error);
        return;
      }

      let stdout = "";
      let stderr = "";
      let timeout: NodeJS.Timeout | undefined;
      let forceKill: NodeJS.Timeout | undefined;
      let settled = false;

      const finish = (callback: () => void): void => {
        if (settled) return;
        settled = true;
        if (timeout !== undefined) clearTimeout(timeout);
        if (forceKill !== undefined) clearTimeout(forceKill);
        invocation.signal?.removeEventListener("abort", onAbort);
        callback();
      };

      const onAbort = (): void => {
        if (settled) return;
        child.kill("SIGTERM");
        forceKill = setTimeout(() => {
          if (!settled) child.kill("SIGKILL");
        }, 250);
        forceKill.unref?.();
      };

      invocation.signal?.addEventListener("abort", onAbort, { once: true });
      if (invocation.timeoutMs !== undefined) {
        timeout = setTimeout(() => {
          if (!settled) child.kill("SIGTERM");
        }, invocation.timeoutMs);
        timeout.unref?.();
      }

      child.stdout?.setEncoding("utf8");
      child.stderr?.setEncoding("utf8");
      child.stdout?.on("data", (chunk: string | Buffer) => {
        stdout += chunk.toString();
      });
      child.stderr?.on("data", (chunk: string | Buffer) => {
        stderr += chunk.toString();
      });
      child.once("error", (error) => finish(() => reject(error)));
      child.once("close", (exitCode, signal) => finish(() => resolve({ stdout, stderr, exitCode, signal })));

      if (invocation.stdin !== undefined) {
        child.stdin?.end(invocation.stdin);
      } else {
        child.stdin?.end();
      }
    });
}

export function linkAbortSignal(signal: AbortSignal | undefined): {
  readonly controller: AbortController;
  readonly dispose: () => void;
} {
  const controller = new AbortController();
  if (signal === undefined) return { controller, dispose: () => undefined };
  const onAbort = (): void => controller.abort();
  if (signal.aborted) controller.abort();
  else signal.addEventListener("abort", onAbort, { once: true });
  return {
    controller,
    dispose: () => signal.removeEventListener("abort", onAbort),
  };
}
