#!/usr/bin/env python3
"""OpenTimelineIO bridge for the native interchange adapter.

This file intentionally has no installation or download behavior.  The
selected Python executable must already have ``opentimelineio`` and any
optional adapter plugin installed.  The Rust side speaks a small JSON protocol
over stdin/stdout so its tests can replace the process entirely.

Both directions exchange the timeline form defined in ``otio.rs``: the
program sequence of a Cut project with positions in frames of the timeline
rate and media times in frames of the media rate, timecode included. Cut ids
travel in ``metadata["cut"]["id"]`` so a round trip keeps them.
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Any, Iterable
from urllib.parse import unquote, urlparse

import opentimelineio as otio
from opentimelineio import opentime, schema


def _issue(code: str, message: str, severity: str = "warning", path: str | None = None) -> dict[str, Any]:
    result: dict[str, Any] = {"code": code, "message": message, "severity": severity}
    if path is not None:
        result["path"] = path
    return result


def _frames(time: Any, rate: float) -> int:
    return round(time.value_rescaled_to(rate))


def _rt(frames: int, rate: float) -> opentime.RationalTime:
    return opentime.RationalTime(frames, rate)


def _range(start: int, duration: int, rate: float) -> opentime.TimeRange:
    return opentime.TimeRange(_rt(start, rate), _rt(duration, rate))


def _cut_id(value: Any) -> str | None:
    cut = getattr(value, "metadata", {}).get("cut")
    if isinstance(cut, dict) and isinstance(cut.get("id"), str):
        return cut["id"]
    return None


def _tag(value: Any, item: dict[str, Any]) -> None:
    if item.get("id"):
        value.metadata["cut"] = {"id": item["id"]}


def _path_from_url(target: str) -> str:
    if not target.lower().startswith("file:"):
        return target
    parsed = urlparse(target)
    path = unquote(parsed.path)
    if parsed.netloc and parsed.netloc not in ("", "localhost"):
        path = f"//{parsed.netloc}{path}"
    return path or target


def _marker(marker: Any, rate: float, offset: int = 0) -> dict[str, Any]:
    marked = marker.marked_range
    result: dict[str, Any] = {
        "at": offset + _frames(marked.start_time, rate),
        "duration": _frames(marked.duration, rate),
    }
    if marker.name:
        result["name"] = str(marker.name)
    if marker.color:
        result["color"] = str(marker.color).lower()
    if _cut_id(marker):
        result["id"] = _cut_id(marker)
    return result


def otio_to_timeline(timeline: Any) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    issues: list[dict[str, Any]] = []
    start_time = timeline.global_start_time
    rate = float(start_time.rate) if start_time is not None else 24.0
    media: list[dict[str, Any]] = []
    media_index: dict[str, int] = {}
    tracks: list[dict[str, Any]] = []
    markers = [_marker(marker, rate) for marker in timeline.tracks.markers]

    def media_of(clip: Any, path: str) -> int:
        reference = clip.media_reference
        target = _path_from_url(str(getattr(reference, "target_url", "") or ""))
        if not target:
            issues.append(_issue("missing_media_reference", "a clip without a media file was imported as a placeholder", "loss", path))
            target = f"missing-{len(media) + 1}"
        if target in media_index:
            return media_index[target]
        available = getattr(reference, "available_range", None)
        media_rate = float((available or clip.source_range).start_time.rate)
        entry: dict[str, Any] = {"path": target, "rate": media_rate}
        if available is not None:
            entry["start"] = _frames(available.start_time, media_rate)
            entry["duration"] = _frames(available.duration, media_rate)
        else:
            entry["start"] = 0
            entry["duration"] = _frames(clip.source_range.end_time_exclusive(), media_rate)
        if _cut_id(reference):
            entry["id"] = _cut_id(reference)
        media_index[target] = len(media)
        media.append(entry)
        return media_index[target]

    for track_index, track in enumerate(timeline.tracks):
        path = f"tracks[{track_index}]"
        if not isinstance(track, schema.Track) or track.kind not in (schema.TrackKind.Video, schema.TrackKind.Audio):
            issues.append(_issue("track_not_imported", "only video and audio tracks are imported", "loss", path))
            continue
        items: list[dict[str, Any]] = []
        for child_index, child in enumerate(track):
            child_path = f"{path}.children[{child_index}]"
            if isinstance(child, schema.Clip):
                source = child.source_range or child.trimmed_range()
                index = media_of(child, child_path)
                item: dict[str, Any] = {
                    "type": "clip",
                    "media": index,
                    "sourceIn": _frames(source.start_time, media[index]["rate"]),
                    "start": _frames(child.range_in_parent().start_time, rate),
                    "duration": _frames(source.duration, rate),
                }
                if child.name:
                    item["name"] = str(child.name)
                if _cut_id(child):
                    item["id"] = _cut_id(child)
                if child.effects:
                    issues.append(_issue("effects_not_imported", "clip effects and time warps were not imported", "loss", child_path))
                for marker in child.markers:
                    offset = item["start"] - _frames(source.start_time, rate)
                    markers.append(_marker(marker, rate, offset))
                items.append(item)
            elif isinstance(child, schema.Transition):
                item = {
                    "type": "transition",
                    "before": _frames(child.in_offset, rate),
                    "after": _frames(child.out_offset, rate),
                }
                if _cut_id(child):
                    item["id"] = _cut_id(child)
                items.append(item)
            elif not isinstance(child, schema.Gap):
                issues.append(_issue("item_not_imported", f"{child.__class__.__name__} items are not imported", "loss", child_path))
        markers.extend(_marker(marker, rate) for marker in track.markers)
        entry = {
            "kind": "video" if track.kind == schema.TrackKind.Video else "audio",
            "enabled": bool(track.enabled),
            "items": items,
        }
        if track.name:
            entry["name"] = str(track.name)
        if _cut_id(track):
            entry["id"] = _cut_id(track)
        tracks.append(entry)

    result = {
        "name": str(timeline.name or ""),
        "rate": rate,
        "start": _frames(start_time, rate) if start_time is not None else 0,
        "media": media,
        "tracks": tracks,
        "markers": markers,
    }
    return result, issues


def _new_marker(value: dict[str, Any], rate: float) -> schema.Marker:
    marker = schema.Marker(name=value.get("name", ""), marked_range=_range(value["at"], value.get("duration", 0), rate))
    color = str(value.get("color", "")).upper()
    if color and hasattr(schema.MarkerColor, color):
        marker.color = getattr(schema.MarkerColor, color)
    _tag(marker, value)
    return marker


def timeline_to_otio(value: dict[str, Any]) -> Any:
    rate = float(value["rate"])
    timeline = schema.Timeline(name=value.get("name", ""))
    timeline.global_start_time = _rt(value.get("start", 0), rate)
    references = []
    for media in value.get("media", []):
        media_rate = float(media["rate"])
        reference = schema.ExternalReference(target_url=media["path"])
        reference.available_range = _range(media.get("start", 0), media.get("duration", 0), media_rate)
        _tag(reference, media)
        references.append((reference, media_rate))
    for track_value in value.get("tracks", []):
        kind = schema.TrackKind.Video if track_value["kind"] == "video" else schema.TrackKind.Audio
        track = schema.Track(name=track_value.get("name") or track_value.get("id", ""), kind=kind)
        track.enabled = track_value.get("enabled", True)
        _tag(track, track_value)
        cursor = 0
        for item in track_value.get("items", []):
            if item["type"] == "transition":
                transition = schema.Transition(
                    transition_type=schema.TransitionTypes.SMPTE_Dissolve,
                    in_offset=_rt(item["before"], rate),
                    out_offset=_rt(item["after"], rate),
                )
                _tag(transition, item)
                track.append(transition)
                continue
            if item["start"] > cursor:
                track.append(schema.Gap(source_range=_range(0, item["start"] - cursor, rate)))
            reference, media_rate = references[item["media"]]
            duration = round(_rt(item["duration"], rate).value_rescaled_to(media_rate))
            clip = schema.Clip(
                name=item.get("name") or item.get("id", ""),
                media_reference=reference.clone(),
                source_range=_range(item["sourceIn"], duration, media_rate),
            )
            _tag(clip, item)
            track.append(clip)
            cursor = item["start"] + item["duration"]
        timeline.tracks.append(track)
    for marker in value.get("markers", []):
        timeline.tracks.markers.append(_new_marker(marker, rate))
    return timeline


def _read(path: str, adapter: str | None) -> Any:
    if adapter:
        return otio.adapters.read_from_file(path, adapter_name=adapter)
    return otio.adapters.read_from_file(path)


def _emit(payload: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def _run(args: argparse.Namespace) -> int:
    if args.operation == "import":
        timeline, issues = otio_to_timeline(_read(args.input, args.adapter))
        _emit({"ok": True, "timeline": timeline, "report": {"issues": issues}})
        return 0

    payload = json.loads(sys.stdin.read())
    if not isinstance(payload, dict) or not isinstance(payload.get("timeline"), dict):
        raise ValueError("export input must be a JSON object containing timeline")
    otio.adapters.write_to_file(timeline_to_otio(payload["timeline"]), args.output, adapter_name=args.adapter)
    _emit({"ok": True, "outputPath": args.output, "report": {"issues": []}})
    return 0


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="cut OpenTimelineIO bridge")
    parser.add_argument("operation", choices=("import", "export"))
    parser.add_argument("--input")
    parser.add_argument("--output")
    parser.add_argument("--adapter")
    args = parser.parse_args(list(argv) if argv is not None else None)
    if args.operation == "import" and not args.input:
        parser.error("import requires --input")
    if args.operation == "export" and not args.output:
        parser.error("export requires --output")
    try:
        return _run(args)
    except Exception as error:  # pragma: no cover - exercised with an installed OTIO environment
        _emit({"ok": False, "error": str(error), "errorType": error.__class__.__name__})
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
