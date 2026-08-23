import { describe, it, expect } from "vitest";
import { resolveLlmConfig } from "../src/main.js";

describe("resolveLlmConfig", () => {
  it("auto-detects anthropic from ANTHROPIC_API_KEY", async () => {
    const cfg = await resolveLlmConfig(undefined, undefined, undefined, {
      ANTHROPIC_API_KEY: "sk-ant",
    });
    expect(cfg.provider).toBe("anthropic");
    expect(cfg.apiKey).toBe("sk-ant");
    expect(cfg.model).toBe("claude-sonnet-4-20250514");
  });

  it("auto-detects openai from OPENAI_API_KEY with OPENAI_BASE_URL available", async () => {
    const cfg = await resolveLlmConfig(undefined, undefined, undefined, {
      OPENAI_API_KEY: "sk-oai",
    });
    expect(cfg.provider).toBe("openai");
    expect(cfg.apiKey).toBe("sk-oai");
    expect(cfg.model).toBe("gpt-4o");
  });

  it("--provider overrides env auto-detection and picks the right key", async () => {
    const cfg = await resolveLlmConfig(
      "explicit-key",
      "openai",
      undefined,
      { ANTHROPIC_API_KEY: "sk-ant", OPENAI_API_KEY: "sk-oai" },
    );
    expect(cfg.provider).toBe("openai");
    expect(cfg.apiKey).toBe("explicit-key");
  });

  it("reads provider/apiKey/model from config.json when flags and env are unset", async () => {
    const cfg = await resolveLlmConfig(
      undefined,
      undefined,
      undefined,
      {},
      async () => ({ provider: "openai", apiKey: "cfg-key", model: "llama3" }),
    );
    expect(cfg.provider).toBe("openai");
    expect(cfg.apiKey).toBe("cfg-key");
    expect(cfg.model).toBe("llama3");
  });

  it("--model beats config.model", async () => {
    const cfg = await resolveLlmConfig(
      "k",
      undefined,
      "gpt-4o-mini",
      {},
      async () => ({ model: "from-config" }),
    );
    expect(cfg.model).toBe("gpt-4o-mini");
    expect(cfg.provider).toBe("anthropic"); // default
  });

  it("rejects unknown providers", async () => {
    await expect(
      resolveLlmConfig("k", "mistral", undefined),
    ).rejects.toThrow(/unknown provider/);
  });
});
