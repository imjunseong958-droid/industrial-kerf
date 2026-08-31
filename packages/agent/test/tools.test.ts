import { describe, it, expect } from "vitest";
import {
  COLOR_EFFECT_JSON_SCHEMA,
  COLOR_GRADE_PARAMS_JSON_SCHEMA,
  COLOR_GRADE_INPUT_JSON_SCHEMA,
  EDIT_INPUT_JSON_SCHEMA,
  PROJECT_EDIT_INPUT_JSON_SCHEMA,
  TOOL_NAMES,
  createColorGradeCommand,
  cutToolsSchemas,
} from "../src/index.js";

describe("agent", () => {
  it("exposes the built-in tool contracts", () => {
    expect([...TOOL_NAMES]).toEqual(["fetch", "probe", "timeline", "project_edit", "render"]);
  });

  it("describes single and atomic batch edits precisely", () => {
    const schema = EDIT_INPUT_JSON_SCHEMA as {
      properties: Record<string, Record<string, unknown>>;
      oneOf: Array<Record<string, unknown>>;
    };
    expect(schema.properties.op?.oneOf).toBeDefined();
    expect(schema.properties.ops?.minItems).toBe(1);
    expect(schema.oneOf).toHaveLength(2);
    expect(JSON.stringify(schema)).toMatch(/0\.1-second/);
    expect(JSON.stringify(schema)).toMatch(/normalized/);
    expect(JSON.stringify(schema)).toMatch(/inline caption/);
    expect(JSON.stringify(schema)).toMatch(/minItems/);
    expect(JSON.stringify(schema)).toMatch(/minLength/);
    expect(JSON.stringify(schema)).toMatch(/timed zoom/);
  });

  it("documents visually inspectable frame probes and the eight-image cap", () => {
    const probe = cutToolsSchemas().find((schema) => schema.name === "probe")!;
    const input = probe.input_schema as unknown as {
      properties: {
        opts?: { properties?: { frames?: { maximum?: number; description?: string } } };
      };
    };
    expect(input.properties.opts?.properties?.frames?.maximum).toBe(8);
    expect(probe.description).toMatch(/visually inspectable/);
  });

  it("only includes opt-in bash when a concrete registry exposes it", () => {
    const defaults = cutToolsSchemas();
    expect(defaults.some((schema) => schema.name === "bash")).toBe(false);
    expect(defaults.some((schema) => schema.name === "edit")).toBe(false);
    expect(defaults.map((schema) => schema.name)).toEqual([
      "fetch",
      "probe",
      "timeline",
      "project_edit",
      "render",
    ]);
    expect(cutToolsSchemas({
      fetch: {} as never,
      probe: {} as never,
      timeline: {} as never,
      project_edit: {} as never,
      render: {} as never,
      bash: {} as never,
    }).some((schema) => schema.name === "bash")).toBe(true);
    expect(cutToolsSchemas({
      fetch: {} as never,
      probe: {} as never,
      timeline: {} as never,
      project_edit: {} as never,
      render: {} as never,
      edit: {} as never,
    }).some((schema) => schema.name === "edit")).toBe(true);
  });

  it("documents color grading through the canonical project_edit update path", () => {
    const projectEdit = cutToolsSchemas().find((schema) => schema.name === "project_edit")!;
    expect(JSON.stringify(projectEdit.input_schema)).toMatch(/update_clip/);
    expect(JSON.stringify(projectEdit.input_schema)).toMatch(/effects/);
    expect(JSON.stringify(projectEdit.input_schema)).toMatch(/type.*color/);
    expect(COLOR_EFFECT_JSON_SCHEMA.required).toEqual(["type", "params"]);
    expect(COLOR_GRADE_INPUT_JSON_SCHEMA.required).toEqual(["clipId", "params"]);
  });

  it("matches the closed v2 color parameter contract and identity defaults", () => {
    const properties = COLOR_GRADE_PARAMS_JSON_SCHEMA.properties as Record<string, Record<string, unknown>>;
    expect(Object.keys(properties)).toEqual([
      "exposure",
      "contrast",
      "brightness",
      "saturation",
      "temperature",
      "tint",
      "gamma",
      "highlights",
      "shadows",
      "vibrance",
      "mix",
    ]);
    expect(COLOR_GRADE_PARAMS_JSON_SCHEMA.additionalProperties).toBe(false);
    expect(properties.exposure).toMatchObject({ minimum: -3, maximum: 3, default: 0 });
    expect(properties.contrast).toMatchObject({ minimum: 0, maximum: 4, default: 1 });
    expect(properties.brightness).toMatchObject({ minimum: -1, maximum: 1, default: 0 });
    expect(properties.saturation).toMatchObject({ minimum: 0, maximum: 4, default: 1 });
    expect(properties.temperature).toMatchObject({ minimum: -1000, maximum: 1000, default: 0 });
    expect(properties.tint).toMatchObject({ minimum: -1, maximum: 1, default: 0 });
    expect(properties.gamma).toMatchObject({ minimum: 0.1, maximum: 10, default: 1 });
    expect(properties.highlights).toMatchObject({ minimum: -1, maximum: 1, default: 0 });
    expect(properties.shadows).toMatchObject({ minimum: -1, maximum: 1, default: 0 });
    expect(properties.vibrance).toMatchObject({ minimum: -1, maximum: 1, default: 0 });
    expect(properties.mix).toMatchObject({ minimum: 0, maximum: 1, default: 1 });
    expect(properties).not.toHaveProperty("hue");
    expect(properties).not.toHaveProperty("lut");
  });

  it("merges a color grade while preserving unrelated effects and input immutability", () => {
    const effects = [
      { type: "blur", params: { radius: 3 }, enabled: true },
      { type: "color", id: "grade-1", params: { exposure: 0.1, contrast: 0.2 } },
      { type: "sharpen", params: { amount: 0.4 } },
    ];
    const before = structuredClone(effects);
    const command = createColorGradeCommand({
      clipId: "clip-1",
      effects,
      params: { exposure: 0.35, saturation: 0.15, brightness: -0.1 },
    });

    expect(command).toEqual({
      type: "update_clip",
      clipId: "clip-1",
      patch: {
        effects: [
          { type: "blur", params: { radius: 3 }, enabled: true },
          {
            type: "color",
            id: "grade-1",
            params: { exposure: 0.35, contrast: 0.2, saturation: 0.15, brightness: -0.1 },
          },
          { type: "sharpen", params: { amount: 0.4 } },
        ],
      },
    });
    expect(effects).toEqual(before);
  });

  it("adds one color effect when none exists and collapses duplicate color effects", () => {
    const added = createColorGradeCommand({
      clipId: "clip-2",
      params: { temperature: 120 },
      existingEffects: [{ type: "blur", params: { radius: 2 } }],
    });
    expect((added.patch as { effects: unknown[] }).effects).toEqual([
      { type: "blur", params: { radius: 2 } },
      { type: "color", params: { temperature: 120 } },
    ]);

    const duplicate = createColorGradeCommand({
      clipId: "clip-3",
      params: { contrast: 0.1 },
      effects: [
        { type: "color", params: { exposure: 0.2 } },
        { type: "grain", params: { amount: 0.3 } },
        { type: "color", params: { saturation: 0.4 } },
      ],
    });
    expect((duplicate.patch as { effects: unknown[] }).effects).toEqual([
      { type: "color", params: { exposure: 0.2, contrast: 0.1 } },
      { type: "grain", params: { amount: 0.3 } },
    ]);
  });
});
