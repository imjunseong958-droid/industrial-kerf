# `@cut/evals`

`@cut/evals` runs deterministic regression checks on Cut's agentic editing
workflows. It does not create an LLM client, access the network, invoke ffmpeg,
read media files, or load model weights. A scenario is JSON-shaped fixture
state, a recorded tool trace, a final v1/v2 result, and named invariants.

```ts
import {
  bundledScenarios,
  evaluateScenario,
  evaluateScenarios,
} from "@cut/evals";

const report = evaluateScenarios(bundledScenarios);
if (!report.passed) {
  for (const scenario of report.scenarios) {
    for (const failure of scenario.failures) {
      console.error(scenario.scenarioId, failure.invariant, failure.message);
    }
  }
}
```

The corpus includes ten workflows: fetched YouTube VTT captions without
Whisper, silence removal, a multi-track overlay, audio mixing, a v2
transition, v2 revision conflict plus undo, cam/chat/content tracking before a
crop, a chat-preservation warning, draft-before-full rendering, and unsafe
bash rejection. `TimelineSchema` and `ProjectSchema` validate final v1/v2
documents; the other checks inspect only the trace and plain JSON state.

## Adding a recorded agent trace

The live agent emits `tool_call_started` and `tool_call_finished` events. Save
those events as JSON alongside a fixture, or adapt another recorder's shape to
the compact form below:

```json
[
  {"tool":"timeline","input":{"include":"full"},"status":"ok"},
  {"tool":"edit","input":{"ops":[{"op":"trim","clipId":"c1","newOut":4}]},"status":"ok"}
]
```

`normalizeToolTrace` also accepts `name`/`toolName`, `id`, started/finished
events, and `{ "kind": "message", "text": "..." }` narration. Keep tool
outputs and rejection errors in the recording: revision-conflict, VTT,
tracking, and unsafe-shell checks use that evidence. A future adapter can load
the JSON and construct an `EvalScenario` without importing an LLM provider or
media runtime:

```ts
const scenario = parseScenario({
  id: "recorded-case",
  name: "Recorded case",
  description: "A checked-in trace",
  version: "v2",
  fixture: { version: "v2", initial: initialProject, target: { duration: 12 } },
  trace: recordedEvents,
  result: { project: finalProject, revision: 2 },
  expectations: {
    invariants: ["inspect-before-edit", "valid-timeline", "duration-preservation"],
  },
});
const evaluation = evaluateScenario(scenario);
```

Use `assertScenario` in a CI regression gate when a failing invariant should
fail the test immediately. The package-local checks can be run with
`npm --workspace @cut/evals test` and `npm --workspace @cut/evals run typecheck`.
