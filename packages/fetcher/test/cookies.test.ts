import { describe, it, expect } from "vitest";
import { buildYtDlpArgs, FORMAT_STRING, OUTPUT_TEMPLATE, SUB_LANGS } from "../src/index.js";

const URL = "https://youtu.be/dQw4w9WgXcQ";

describe("buildYtDlpArgs", () => {
  it("builds the base argv without cookies", () => {
    const args = buildYtDlpArgs(URL);
    expect(args.at(-1)).toBe(URL);
    expect(args).not.toContain("--cookies");
    const i = args.indexOf("--format");
    expect(args[i + 1]).toBe(FORMAT_STRING);
    expect(args).toContain("--no-playlist");
    expect(args[args.indexOf("--output") + 1]).toBe(OUTPUT_TEMPLATE);
    expect(args[args.indexOf("--sub-langs") + 1]).toBe(SUB_LANGS);
  });

  it("inserts --cookies <path> when opts.cookies is set", () => {
    const args = buildYtDlpArgs(URL, { cookies: "/tmp/cookies.txt" });
    const i = args.indexOf("--cookies");
    expect(i).toBeGreaterThan(-1);
    expect(args[i + 1]).toBe("/tmp/cookies.txt");
    // still ends with the url
    expect(args.at(-1)).toBe(URL);
  });

  it("does not emit --cookies for an empty-string cookie path", () => {
    expect(buildYtDlpArgs(URL, { cookies: "" })).not.toContain("--cookies");
  });

  it("places cookies before extraArgs and keeps extra args before the url", () => {
    const args = buildYtDlpArgs(URL, {
      cookies: "/c.txt",
      extraArgs: ["--rate-limit", "1M"],
    });
    expect(args.indexOf("--cookies")).toBeLessThan(args.indexOf("--rate-limit"));
    expect(args).toEqual(
      expect.arrayContaining(["--cookies", "/c.txt", "--rate-limit", "1M"]),
    );
    expect(args.filter((a) => a === URL)).toHaveLength(1);
  });
});
