# `@cut/vision`

This package is a TypeScript wrapper that calls the native `cut-engine` binary
for YOLO26 tracking. Inference runs in-process in `cut-engine` (ONNX Runtime +
ByteTrack). Python/Ultralytics trains or exports weights only; it does not run
inference.

## Model and labels

Ultralytics identifies the small YOLO26 detection checkpoint as `yolo26s.pt`.
The official checkpoint is COCO-trained, so it is not assumed to contain
`cam`, `chat`, or `content`. The adapter rejects the bare stock identifier (which
could trigger an implicit download) and requires a local custom-trained weights
path. Custom models emit class names `cam`, `chat`, and `content` (IDs `0`, `1`,
`2`). `labelMap` is optional; when omitted the adapter uses the identity map for
both class names and IDs.

```ts
import { createYolo26Tracker } from "@cut/vision";

const tracker = createYolo26Tracker({
  // A custom checkpoint trained with these application labels.
  weights: "/models/cut-yolo26s-cam-chat-content.onnx",
});

const result = await tracker.track({ source: "/video/input.mp4" });
```

Output classes are exactly `cam`, `chat`, and `content`. Unmapped model classes
are ignored by default; set `unknownLabelPolicy: "error"` to fail closed.
Each detection has `trackId`, `class`, `confidence`, a zero-based `frame`, and
second-based `time`. It also has `bbox: { x, y, width, height }` with every
coordinate normalized and clamped to `[0, 1]`. Detections sort by frame,
semantic class, track ID, and geometry for stable downstream analysis.
When the runner does not provide timestamps, `fps` (from the request/config)
is used. The deterministic fallback is 30 FPS.

## Native tracking

Default tracking is `cut-engine track --source … --weights …`. Weights must be
`.onnx`, or a `.pt` checkpoint that has a sibling `.onnx` produced by:

```text
yolo export model=/models/cut-yolo26s-cam-chat-content.pt format=onnx
```

Training still uses [`python/train_yolo26s.py`](./python/train_yolo26s.py); that
script is not on the inference path. For tests, inject a `runner`/`processRunner`
that returns JSONL on stdout so no model file is required.

## Tracking plans and crop ROIs

`buildTrackingPlan` converts detections into immutable, class-separated
segments and renderer-friendly ROI keyframes:

```ts
import { buildTrackingPlan, planCropForAspectRatio } from "@cut/vision";

const plan = buildTrackingPlan(result, {
  minConfidence: 0.5,
  maxGapSeconds: 0.5,
  smoothingFactor: 0.5,
});

const crop = planCropForAspectRatio({
  aspectRatio: 9 / 16,
  sourceAspectRatio: 16 / 9,
  cam: plan.keyframesByClass.cam[0],
  chat: plan.keyframesByClass.chat[0],
  content: plan.keyframesByClass.content[0],
  padding: 0.02,
});
```

Segments are grouped by semantic class and track ID, split after the configured
time gap, and smoothed with a deterministic exponential moving average. The
planner exposes `byClass.cam`, `byClass.chat`, `byClass.content`, and matching
keyframe collections so the ROIs remain separate. The crop helper expands the
selected union to the requested aspect ratio, shifts it into the normalized
frame, clamps it to `[0, 1]`, and reports whether each selected target was
actually included. If no target is available it returns the largest centered
crop at the requested aspect ratio. `sourceAspectRatio` is the source pixel
width/height. Normalized bbox width and height are not pixel aspect unless the
source is square. The field defaults to `1` for backwards compatibility. Real
video callers should pass the source ratio (or `sourceWidth` and `sourceHeight`).

## Research source

The model identifier and supported filenames come from the official
[Ultralytics YOLO26 model documentation](https://docs.ultralytics.com/models/yolo26).
The `model.track`/`persist`/`tracker` interface comes from the official
[Ultralytics tracking guide](https://docs.ultralytics.com/modes/track).

## Training layout

[`training/dataset.template.yaml`](./training/dataset.template.yaml) defines
the required class IDs (`0: cam`, `1: chat`, `2: content`). Validate or train
with [`python/train_yolo26s.py`](./python/train_yolo26s.py). The script checks
every train and validation label. It supports `--offline --base /path/to/local.pt`
and emits the resulting weights path plus an adapter config snippet. It never
installs packages or downloads data. The bare `yolo26s.pt` base may be
downloaded by Ultralytics unless offline mode is used. See
[`training/README.md`](./training/README.md) for dataset, licensing, privacy,
and command requirements.
