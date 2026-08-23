import { describe, it, expect } from "vitest";
import { homedir } from "node:os";
import path from "node:path";
import { DEFAULT_COOKIES_PATH, resolveCookiesPath } from "../src/main.js";

describe("resolveCookiesPath", () => {
  const def = DEFAULT_COOKIES_PATH;

  it("default path is ~/yolomodellayout/youtube.com_cookies.txt", () => {
    expect(def).toBe(path.join(homedir(), "yolomodellayout", "youtube.com_cookies.txt"));
  });

  it("explicit flag wins over everything", () => {
    const got = resolveCookiesPath("/x/a.txt", () => true, { CUT_COOKIES: "/x/b.txt" } as NodeJS.ProcessEnv);
    expect(got).toBe("/x/a.txt");
  });

  it("CUT_COOKIES env wins over default file", () => {
    const got = resolveCookiesPath(undefined, () => true, { CUT_COOKIES: "/x/env.txt" } as NodeJS.ProcessEnv);
    expect(got).toBe("/x/env.txt");
  });

  it("falls back to the default file when it exists", () => {
    const got = resolveCookiesPath(undefined, () => true, {} as NodeJS.ProcessEnv);
    expect(got).toBe(def);
  });

  it("returns undefined (silent no-cookies) when default file is missing", () => {
    const got = resolveCookiesPath(undefined, () => false, {} as NodeJS.ProcessEnv);
    expect(got).toBeUndefined();
  });
});
