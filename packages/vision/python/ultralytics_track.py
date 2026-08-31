#!/usr/bin/env python3
"""Small JSONL bridge for an externally installed Ultralytics package.

This script intentionally does not install packages or download weights. The
TypeScript adapter supplies a local custom checkpoint path and invokes this
script with the Python executable chosen by the caller.
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Any


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run Ultralytics YOLO26 tracking and emit JSONL.")
    parser.add_argument("--model", required=True, help="Local custom-trained YOLO weights path")
    parser.add_argument("--source", required=True, help="Video/image source accepted by Ultralytics")
    parser.add_argument("--tracker", default="bytetrack.yaml")
    parser.add_argument("--conf", type=float, default=0.25)
    parser.add_argument("--iou", type=float, default=0.7)
    parser.add_argument("--fps", type=float, default=None)
    parser.add_argument("--device", default=None)
    parser.add_argument("--max-frames", type=int, default=None)
    return parser.parse_args()


def class_name(names: Any, class_id: int) -> str | None:
    if isinstance(names, dict):
        value = names.get(class_id, names.get(str(class_id)))
    elif isinstance(names, (list, tuple)) and 0 <= class_id < len(names):
        value = names[class_id]
    else:
        value = None
    return str(value) if value is not None else None


def to_list(value: Any) -> list[Any]:
    if value is None:
        return []
    if hasattr(value, "detach"):
        value = value.detach()
    if hasattr(value, "cpu"):
        value = value.cpu()
    if hasattr(value, "tolist"):
        value = value.tolist()
    return value


def main() -> int:
    args = parse_args()
    try:
        from ultralytics import YOLO
    except Exception as exc:  # pragma: no cover - exercised only in external environments
        print(f"Unable to import ultralytics: {exc}", file=sys.stderr)
        return 2

    try:
        model = YOLO(args.model)
        track_kwargs: dict[str, Any] = {
            "source": args.source,
            "stream": True,
            "persist": True,
            "tracker": args.tracker,
            "conf": args.conf,
            "iou": args.iou,
        }
        if args.device is not None:
            track_kwargs["device"] = args.device
        results = model.track(**track_kwargs)

        for frame_index, result in enumerate(results):
            if args.max_frames is not None and frame_index >= args.max_frames:
                break
            height, width = (int(result.orig_shape[0]), int(result.orig_shape[1]))
            boxes = result.boxes
            xyxy = to_list(getattr(boxes, "xyxy", None))
            classes = to_list(getattr(boxes, "cls", None))
            confidences = to_list(getattr(boxes, "conf", None))
            ids = to_list(getattr(boxes, "id", None))
            detections: list[dict[str, Any]] = []
            for index, coordinates in enumerate(xyxy):
                # Tracking IDs are required by the TypeScript contract. A
                # non-tracked result is surfaced as null and rejected there.
                track_id = None
                if ids is not None and index < len(ids):
                    track_id = int(ids[index])
                class_id = int(classes[index]) if index < len(classes) else None
                confidence = float(confidences[index]) if index < len(confidences) else None
                item: dict[str, Any] = {
                    "track_id": track_id,
                    "class_id": class_id,
                    "class_name": class_name(result.names, class_id) if class_id is not None else None,
                    "confidence": confidence,
                    "bbox": [float(coordinate) for coordinate in coordinates],
                }
                detections.append(item)
            payload: dict[str, Any] = {
                "frame_index": frame_index,
                "time_seconds": (frame_index / args.fps) if args.fps else None,
                "width": width,
                "height": height,
                "fps": args.fps,
                "detections": detections,
            }
            # Compact + sorted output keeps the process contract deterministic.
            print(json.dumps(payload, separators=(",", ":"), sort_keys=True), flush=True)
    except Exception as exc:  # pragma: no cover - exercised only in external environments
        print(f"Ultralytics tracking failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
