import { EventEmitter } from "node:events";
import type { ChildProcess } from "node:child_process";
import { describe, expect, it } from "vitest";
import {
  browserCommandForPlatform,
  CUT_VERSION,
  HELP_TEXT,
  openInBrowser,
  parseArgs,
  runGui,
  type CutAppHandle,
} from "../src/main.js";

describe("GUI launcher argument routing", () => {
  it("uses GUI mode by default and parses host/port/browser flags", () => {
    expect(parseArgs(["clip.mp4", "--host", "127.0.0.1", "--port", "4312", "--no-open"])).toMatchObject({
      media: "clip.mp4",
      host: "127.0.0.1",
      port: 4312,
      noOpen: true,
    });
    expect(parseArgs([]).terminal).toBeUndefined();
  });

  it("keeps print, skills, and MCP commands in terminal mode", () => {
    expect(parseArgs(["--terminal"]).terminal).toBe(true);
    expect(parseArgs(["-p", "trim the silence"]).terminal).toBe(true);
    expect(parseArgs(["--skills"]).terminal).toBe(true);
    expect(parseArgs(["--mcp"]).terminal).toBe(true);
    expect(parseArgs(["--mcp-tools"])).toMatchObject({ mcpTools: true, terminal: true });
    expect(HELP_TEXT).toContain("--mcp-tools");
  });

  it("routes help and version to immediate non-GUI exits", () => {
    expect(parseArgs(["--help"])).toMatchObject({ help: true, terminal: true });
    expect(parseArgs(["-h", "video.mp4"])).toMatchObject({ help: true, media: "video.mp4", terminal: true });
    expect(parseArgs(["--version"])).toMatchObject({ version: true, terminal: true });
    expect(parseArgs(["-v"])).toMatchObject({ version: true, terminal: true });
    expect(HELP_TEXT).toContain("lcut [video-or-url]");
    expect(CUT_VERSION).toBe("0.1.0");
  });

  it("rejects invalid ports", () => {
    expect(() => parseArgs(["--port", "70000"])).toThrow(/invalid port/);
    expect(() => parseArgs(["--port"])).toThrow(/requires a number/);
  });
});

describe("GUI launcher lifecycle", () => {
  it("starts the app, forwards the resolved non-interactive config, and closes it", async () => {
    let received: unknown;
    let closed = 0;
    const app: CutAppHandle = {
      url: "http://127.0.0.1:4312",
      close: async () => {
        closed++;
      },
    };

    await runGui(
      { dir: "/tmp/cut-launcher-test", media: "clip.mp4", noOpen: true },
      {
        startApp: async (options) => {
          received = options;
          return app;
        },
        waitForExit: async () => undefined,
      },
    );

    expect(received).toMatchObject({
      dir: "/tmp/cut-launcher-test",
      media: "/tmp/cut-launcher-test/clip.mp4",
      llm: { provider: expect.stringMatching(/^(anthropic|openai)$/), model: expect.any(String) },
    });
    expect(closed).toBe(1);
  });

  it("opens the URL using an argument-safe platform command", async () => {
    const calls: { command: string; args: string[] }[] = [];
    const child = new EventEmitter() as ChildProcess;
    child.unref = () => child;
    const spawnBrowser = (command: string, args: string[]) => {
      calls.push({ command, args });
      queueMicrotask(() => child.emit("spawn"));
      return child;
    };

    await openInBrowser("http://127.0.0.1:4312/?x=a&y=b", "linux", spawnBrowser);

    expect(calls).toEqual([{
      command: "xdg-open",
      args: ["http://127.0.0.1:4312/?x=a&y=b"],
    }]);
    expect(browserCommandForPlatform("darwin").args("https://example.test")).toEqual([
      "https://example.test",
    ]);
  });
});
