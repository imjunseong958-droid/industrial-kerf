import { z } from "zod";

/** JSON-like values are sufficient for traces and make fixtures replayable. */
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

export type WorkflowVersion = "v1" | "v2";

/** Names of deterministic checks supplied by this package. */
export const InvariantNames = [
  "inspect-before-edit",
  "atomic-command-batch",
  "no-arbitrary-model-weights",
  "no-bash",
  "draft-before-full",
  "valid-timeline",
  "duration-preservation",
  "aspect-preservation",
  "target-preservation",
  "fetched-youtube-vtt",
  "no-whisper",
  "revision-conflict",
  "undo",
  "tracking-before-crop",
  "chat-preservation-warning",
] as const;

export type InvariantName = (typeof InvariantNames)[number];

export type TraceStatus = "ok" | "rejected" | "error";

/** A normalized tool call. Inputs and outputs intentionally remain JSON-shaped unknowns. */
export interface ToolCallTrace {
  readonly kind?: "tool_call";
  readonly tool: string;
  readonly input?: unknown;
  readonly output?: unknown;
  readonly status?: TraceStatus;
  readonly error?: string;
  readonly id?: string;
  readonly step?: number;
  readonly [key: string]: unknown;
}

/** Assistant/user narration can be kept in a trace for warning checks. */
export interface TraceMessage {
  readonly kind: "message";
  readonly text: string;
  readonly step?: number;
  readonly [key: string]: unknown;
}

export type ToolTraceEntry = ToolCallTrace | TraceMessage;
export type ToolTrace = readonly ToolTraceEntry[];

/** Output dimensions and identity constraints expected from a final result. */
export interface TargetExpectation {
  readonly duration?: number;
  readonly width?: number;
  readonly height?: number;
  readonly aspectRatio?: number;
  readonly aspect?: number;
  readonly sourceIds?: readonly string[];
  readonly requiredSourceIds?: readonly string[];
  readonly clipIds?: readonly string[];
  readonly requiredClipIds?: readonly string[];
  readonly trackIds?: readonly string[];
  readonly requiredTrackIds?: readonly string[];
  readonly overlayKinds?: readonly string[];
  readonly requiredOverlayKinds?: readonly string[];
  readonly text?: readonly string[];
  readonly requiredText?: readonly string[];
  readonly metadata?: Record<string, unknown>;
  readonly undoRestoresInitial?: boolean;
  /** When dimensions come from a particular source, use this source id. */
  readonly aspectSourceId?: string;
}

export interface FixtureAsset {
  readonly path: string;
  readonly kind?: "video" | "audio" | "vtt" | "image" | "other";
  readonly duration?: number;
  readonly width?: number;
  readonly height?: number;
  readonly hasAudio?: boolean;
  readonly [key: string]: unknown;
}

/** Initial state plus metadata needed to evaluate a workflow deterministically. */
export interface ScenarioFixture {
  readonly version: WorkflowVersion;
  readonly initial: unknown;
  readonly assets?: Record<string, FixtureAsset>;
  readonly target?: TargetExpectation;
  readonly [key: string]: unknown;
}

export interface ScenarioExpectations {
  readonly invariants: readonly InvariantName[];
  readonly target?: TargetExpectation;
  /** Require an actual multi-operation transaction, not several one-op calls. */
  readonly atomicBatch?: boolean;
  /** Require at least one draft and one full render when checking draft order. */
  readonly requireDraftAndFull?: boolean;
  /** A warning phrase, when supplied, must occur before the crop operation. */
  readonly warningIncludes?: readonly string[];
  readonly [key: string]: unknown;
}

export interface EvalScenario {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly version: WorkflowVersion;
  readonly fixture: ScenarioFixture;
  readonly trace: ToolTrace;
  /** The final result may be a v1 timeline or a v2 project envelope. */
  readonly result: unknown;
  readonly expectations: ScenarioExpectations;
  readonly [key: string]: unknown;
}

export type EvaluationScenario = EvalScenario;
export type Scenario = EvalScenario;
export type Fixture = ScenarioFixture;

export interface EvaluationCheck {
  readonly invariant: InvariantName;
  readonly passed: boolean;
  readonly message: string;
  readonly phase: "trace" | "final" | "scenario";
  readonly evidence?: readonly string[];
}

export interface EvaluationResult {
  readonly passed: boolean;
  readonly checks: readonly EvaluationCheck[];
  readonly failures: readonly EvaluationCheck[];
  readonly scenarioId?: string;
}

export interface ScenarioEvaluation extends EvaluationResult {
  readonly scenarioId: string;
  readonly name: string;
}

export interface EvaluationReport {
  readonly passed: boolean;
  readonly total: number;
  readonly passedCount: number;
  readonly failedCount: number;
  readonly scenarios: readonly ScenarioEvaluation[];
}

// ---------------------------------------------------------------------------
// Runtime schemas
// ---------------------------------------------------------------------------

const TargetExpectationSchema = z
  .object({
    duration: z.number().finite().optional(),
    width: z.number().finite().positive().optional(),
    height: z.number().finite().positive().optional(),
    aspectRatio: z.number().finite().positive().optional(),
    aspect: z.number().finite().positive().optional(),
    sourceIds: z.array(z.string().min(1)).optional(),
    requiredSourceIds: z.array(z.string().min(1)).optional(),
    clipIds: z.array(z.string().min(1)).optional(),
    requiredClipIds: z.array(z.string().min(1)).optional(),
    trackIds: z.array(z.string().min(1)).optional(),
    requiredTrackIds: z.array(z.string().min(1)).optional(),
    overlayKinds: z.array(z.string().min(1)).optional(),
    requiredOverlayKinds: z.array(z.string().min(1)).optional(),
    text: z.array(z.string().min(1)).optional(),
    requiredText: z.array(z.string().min(1)).optional(),
    metadata: z.record(z.unknown()).optional(),
    undoRestoresInitial: z.boolean().optional(),
    aspectSourceId: z.string().min(1).optional(),
  })
  .passthrough();

export const TargetSchema = TargetExpectationSchema;

const FixtureAssetSchema = z
  .object({
    path: z.string().min(1),
    kind: z.enum(["video", "audio", "vtt", "image", "other"]).optional(),
    duration: z.number().finite().nonnegative().optional(),
    width: z.number().finite().positive().optional(),
    height: z.number().finite().positive().optional(),
    hasAudio: z.boolean().optional(),
  })
  .passthrough();

export const FixtureSchema = z
  .object({
    version: z.enum(["v1", "v2"]),
    initial: z.unknown(),
    assets: z.record(FixtureAssetSchema).optional(),
    target: TargetExpectationSchema.optional(),
  })
  .passthrough();

const TraceEntrySchema = z
  .object({
    kind: z.enum(["tool_call", "message"]).optional(),
    tool: z.string().min(1).optional(),
    name: z.string().min(1).optional(),
    input: z.unknown().optional(),
    output: z.unknown().optional(),
    status: z.enum(["ok", "rejected", "error"]).optional(),
    error: z.string().optional(),
    text: z.string().optional(),
    id: z.string().optional(),
    step: z.number().int().nonnegative().optional(),
  })
  .passthrough()
  .refine(
    (entry) =>
      typeof entry.tool === "string" ||
      typeof entry.name === "string" ||
      entry.kind === "message" ||
      typeof entry.text === "string",
    "trace entry must identify a tool or contain a message",
  );

export const ToolTraceEntrySchema = TraceEntrySchema;
export const ToolTraceSchema = z.array(TraceEntrySchema);

const InvariantNameSchema = z.enum(InvariantNames);

const ScenarioExpectationsSchema = z
  .object({
    invariants: z.array(InvariantNameSchema),
    target: TargetExpectationSchema.optional(),
    atomicBatch: z.boolean().optional(),
    requireDraftAndFull: z.boolean().optional(),
    warningIncludes: z.array(z.string().min(1)).optional(),
  })
  .passthrough();

export const ScenarioSchema = z
  .object({
    id: z.string().min(1),
    name: z.string().min(1),
    description: z.string().min(1),
    version: z.enum(["v1", "v2"]),
    fixture: FixtureSchema,
    trace: ToolTraceSchema,
    result: z.unknown(),
    expectations: ScenarioExpectationsSchema,
  })
  .passthrough();

export function parseFixture(value: unknown): ScenarioFixture {
  return FixtureSchema.parse(value) as ScenarioFixture;
}

export function parseScenario(value: unknown): EvalScenario {
  return ScenarioSchema.parse(value) as EvalScenario;
}

export function isScenario(value: unknown): value is EvalScenario {
  return ScenarioSchema.safeParse(value).success;
}
