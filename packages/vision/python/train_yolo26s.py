#!/usr/bin/env python3
"""Validate and train a three-class custom YOLO26-small checkpoint.

This script runs in an external Python environment. It never installs Python packages,
downloads a dataset, or downloads weights itself. Ultralytics may download the
bare official ``yolo26s.pt`` base when --offline is not used; pass --offline
with a local base checkpoint to prevent that behavior.
"""

from __future__ import annotations

import argparse
import ast
import json
import math
import re
import shutil
import sys
from pathlib import Path
from typing import Any


OFFICIAL_BASE = "yolo26s.pt"
EXPECTED_NAMES = {0: "cam", 1: "chat", 2: "content"}
REMOTE_RE = re.compile(r"^(?:https?|ftp)://", re.IGNORECASE)


class TrainingConfigError(ValueError):
    """Raised for an invalid dataset/base/training option."""


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Validate a cam/chat/content YOLO dataset and train YOLO26-small.",
        epilog=(
            "No packages are installed by this command. The bare yolo26s.pt base "
            "may be downloaded by Ultralytics; --offline requires a local --base."
        ),
    )
    parser.add_argument("--data", type=Path, required=True, help="Dataset YAML (use training/dataset.template.yaml as a template)")
    parser.add_argument("--output", type=Path, help="Caller-chosen destination for the resulting .pt weights")
    parser.add_argument("--base", default=OFFICIAL_BASE, help=f"Base checkpoint (default: {OFFICIAL_BASE})")
    parser.add_argument("--offline", action="store_true", help="Require --base to be an existing local checkpoint")
    parser.add_argument("--validate-only", action="store_true", help="Validate YAML and labels, then emit a summary without training")
    parser.add_argument("--epochs", type=int, default=100)
    parser.add_argument("--imgsz", type=int, default=640)
    parser.add_argument("--batch", type=float, default=None)
    parser.add_argument("--workers", type=int, default=None)
    parser.add_argument("--device", default=None)
    parser.add_argument("--project", type=Path, default=None, help="Ultralytics run directory (defaults beside --output)")
    parser.add_argument("--name", default=None, help="Ultralytics run name (defaults to output stem)")
    parser.add_argument(
        "--export-format",
        choices=("onnx", "torchscript", "openvino", "engine", "coreml", "tflite"),
        default=None,
        help="Optionally export the copied weights after training",
    )
    return parser


def _without_comment(line: str) -> str:
    # Dataset paths here are plain scalars. This parser stays dependency-free
    # so validation-only use does not require PyYAML.
    return line.split("#", 1)[0].rstrip()


def _scalar(value: str) -> Any:
    value = value.strip()
    if not value:
        return None
    if (value.startswith("\"") and value.endswith("\"")) or (value.startswith("'") and value.endswith("'")):
        return value[1:-1]
    if value.startswith("[") and value.endswith("]"):
        try:
            return ast.literal_eval(value)
        except (ValueError, SyntaxError) as exc:
            # YAML permits unquoted plain strings where Python literals do not.
            return [item.strip().strip("\"'") for item in value[1:-1].split(",") if item.strip()]
    return value


def parse_dataset_yaml(path: Path) -> dict[str, Any]:
    """Parse the small scalar/mapping subset used by Ultralytics dataset YAML."""
    if not path.is_file():
        raise TrainingConfigError(f"dataset YAML does not exist: {path}")

    values: dict[str, Any] = {}
    names: dict[int, str] = {}
    names_indent: int | None = None
    names_list: list[str] = []
    in_names = False
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = _without_comment(raw_line)
        if not line.strip():
            continue
        indent = len(line) - len(line.lstrip())
        stripped = line.strip()
        if in_names:
            mapping = re.fullmatch(r"(\d+)\s*:\s*(.+)", stripped)
            list_item = re.fullmatch(r"-\s*(.+)", stripped)
            if mapping:
                names[int(mapping.group(1))] = str(_scalar(mapping.group(2)))
                continue
            if list_item:
                names_list.append(str(_scalar(list_item.group(1))))
                continue
            if indent > (names_indent or 0):
                raise TrainingConfigError("names must be a numeric mapping or a list")
            in_names = False

        match = re.fullmatch(r"([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)", stripped)
        if not match:
            raise TrainingConfigError(f"unsupported dataset YAML line: {raw_line}")
        key, raw_value = match.groups()
        if key == "names":
            in_names = True
            names_indent = indent
            if raw_value.strip():
                parsed = _scalar(raw_value)
                if not isinstance(parsed, (list, tuple)):
                    raise TrainingConfigError("names must be a list or numeric mapping")
                names_list.extend(str(item) for item in parsed)
        else:
            values[key] = _scalar(raw_value)

    if names_list:
        names.update(dict(enumerate(names_list)))
    values["names"] = names
    return values


def _resolve_dataset_root(data_yaml: Path, values: dict[str, Any]) -> Path:
    raw_root = values.get("path")
    if raw_root is None:
        return data_yaml.parent.resolve()
    root = Path(str(raw_root)).expanduser()
    return (root if root.is_absolute() else data_yaml.parent / root).resolve()


def _resolve_split(root: Path, raw_split: Any, field: str) -> Path:
    if not isinstance(raw_split, str) or not raw_split.strip():
        raise TrainingConfigError(f"dataset YAML {field} must be a path string")
    split = Path(raw_split).expanduser()
    return (split if split.is_absolute() else root / split).resolve()


def _label_dir_for_images(image_dir: Path, root: Path) -> Path:
    parts = list(image_dir.parts)
    if "images" in parts:
        parts[parts.index("images")] = "labels"
        return Path(*parts)
    candidate = root / "labels" / image_dir.name
    return candidate if candidate.exists() else image_dir


def _labels_for_split(split: Path, root: Path) -> list[Path]:
    if split.is_file():
        labels: list[Path] = []
        for raw in split.read_text(encoding="utf-8").splitlines():
            image_value = raw.split("#", 1)[0].strip()
            if not image_value:
                continue
            image = Path(image_value).expanduser()
            if not image.is_absolute():
                image = split.parent / image
            parts = list(image.parts)
            if "images" in parts:
                parts[parts.index("images")] = "labels"
                image = Path(*parts)
            labels.append(image.with_suffix(".txt"))
        return sorted(path for path in labels if path.is_file())

    if not split.is_dir():
        raise TrainingConfigError(f"dataset split does not exist: {split}")
    label_dir = _label_dir_for_images(split, root)
    if label_dir.is_dir():
        return sorted(path for path in label_dir.rglob("*.txt") if path.is_file())
    # This fallback supports datasets that place labels beside split folders.
    return sorted(path for path in split.rglob("*.txt") if path.is_file())


def _validate_label_file(path: Path) -> int:
    count = 0
    for line_number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw_line.split("#", 1)[0].strip()
        if not line:
            continue
        fields = line.split()
        if len(fields) < 5:
            raise TrainingConfigError(f"{path}:{line_number} must have class + 4 bbox values")
        try:
            class_id = int(fields[0])
        except ValueError as exc:
            raise TrainingConfigError(f"{path}:{line_number} class ID must be an integer") from exc
        if class_id not in EXPECTED_NAMES:
            raise TrainingConfigError(f"{path}:{line_number} uses class {class_id}; only 0 (cam), 1 (chat), and 2 (content) are allowed")
        try:
            coordinates = [float(value) for value in fields[1:5]]
        except ValueError as exc:
            raise TrainingConfigError(f"{path}:{line_number} bbox values must be numbers") from exc
        if not all(math.isfinite(value) and 0 <= value <= 1 for value in coordinates):
            raise TrainingConfigError(f"{path}:{line_number} bbox values must be normalized to 0..1")
        count += 1
    return count


def validate_dataset(data_yaml: Path) -> dict[str, Any]:
    """Validate exact class mapping, split paths, and every discovered label ID."""
    data_yaml = data_yaml.expanduser().resolve()
    values = parse_dataset_yaml(data_yaml)
    names = values.get("names")
    if names != EXPECTED_NAMES:
        raise TrainingConfigError(f"dataset names must be exactly {{0: 'cam', 1: 'chat', 2: 'content'}}; got {names!r}")
    root = _resolve_dataset_root(data_yaml, values)
    train = _resolve_split(root, values.get("train"), "train")
    val = _resolve_split(root, values.get("val"), "val")
    labels = sorted(set(_labels_for_split(train, root) + _labels_for_split(val, root)))
    if not labels:
        raise TrainingConfigError("no YOLO label TXT files were found in train/val splits")
    object_count = sum(_validate_label_file(path) for path in labels)
    if object_count == 0:
        raise TrainingConfigError("dataset contains no labeled cam/chat/content objects")
    return {
        "yaml": str(data_yaml),
        "root": str(root),
        "train": str(train),
        "val": str(val),
        "labelFiles": len(labels),
        "objects": object_count,
        "names": {str(key): value for key, value in EXPECTED_NAMES.items()},
    }


def validate_base(base: str, offline: bool) -> None:
    """Reject implicit/remote bases in offline mode and warn for official base."""
    if REMOTE_RE.match(base):
        raise TrainingConfigError("--base must be a local path or yolo26s.pt; remote URLs are not accepted")
    if base == OFFICIAL_BASE:
        if offline:
            raise TrainingConfigError("--offline requires --base to be an existing local checkpoint, not yolo26s.pt")
        print(
            "warning: Ultralytics may download the official yolo26s.pt base; use --offline --base /path/to/base.pt to prevent downloads",
            file=sys.stderr,
        )
        return
    base_path = Path(base).expanduser()
    if not base_path.is_file():
        raise TrainingConfigError(f"local base checkpoint does not exist: {base_path}")


def render_config_snippet(weights: Path) -> dict[str, Any]:
    """Return the exact TypeScript adapter config emitted after training."""
    return {
        "weights": str(weights.expanduser().resolve()),
        "labelMap": {"cam": "cam", "chat": "chat", "content": "content"},
        "targets": ["cam", "chat", "content"],
    }


def _train(args: argparse.Namespace, dataset: dict[str, Any]) -> dict[str, Any]:
    validate_base(args.base, args.offline)
    if args.epochs <= 0 or args.imgsz <= 0:
        raise TrainingConfigError("--epochs and --imgsz must be greater than zero")
    if args.batch is not None and args.batch <= 0:
        raise TrainingConfigError("--batch must be greater than zero")
    if args.workers is not None and args.workers < 0:
        raise TrainingConfigError("--workers must be non-negative")
    if args.output is None:
        raise TrainingConfigError("--output is required unless --validate-only is used")
    output = args.output.expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    project = (args.project.expanduser().resolve() if args.project is not None else output.parent / "runs")
    name = args.name or output.stem

    try:
        from ultralytics import YOLO
    except Exception as exc:  # pragma: no cover - external environment only
        raise TrainingConfigError(
            "ultralytics is not installed in the selected Python environment; install it externally (no auto-install is performed)"
        ) from exc

    model = YOLO(args.base)
    train_kwargs: dict[str, Any] = {
        "data": dataset["yaml"],
        "epochs": args.epochs,
        "imgsz": args.imgsz,
        "project": str(project),
        "name": name,
    }
    for key, value in (("batch", args.batch), ("workers", args.workers), ("device", args.device)):
        if value is not None:
            train_kwargs[key] = value
    model.train(**train_kwargs)

    best = project / name / "weights" / "best.pt"
    if not best.is_file():
        raise TrainingConfigError(f"Ultralytics training completed but best.pt was not found at {best}")
    shutil.copy2(best, output)

    exported: str | None = None
    if args.export_format is not None:
        exported_value = YOLO(str(output)).export(format=args.export_format)
        exported = str(exported_value) if exported_value is not None else None
    return {
        "dataset": dataset,
        "weightsPath": str(output),
        "config": render_config_snippet(output),
        "exportFormat": args.export_format,
        "exportedPath": exported,
    }


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        dataset = validate_dataset(args.data)
        if args.validate_only:
            print(json.dumps({"dataset": dataset, "config": render_config_snippet(Path("custom.pt"))}, indent=2, sort_keys=True))
            return 0
        print(json.dumps(_train(args, dataset), indent=2, sort_keys=True))
        return 0
    except (OSError, TrainingConfigError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
