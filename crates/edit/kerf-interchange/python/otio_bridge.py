#!/usr/bin/env python3
"""OpenTimelineIO bridge for the native interchange adapter.

This file intentionally has no installation or download behavior.  The
selected Python executable must already have ``opentimelineio`` and any
optional adapter plugin installed.  The Rust side speaks a small JSON protocol
over stdin/stdout so its tests can replace the process entirely.

Both directions exchange the timeline form defined in ``otio.rs``: one
sequence of a Kerf project with positions in frames of the timeline rate and
media times in frames of the media rate, timecode included. Kerf ids travel in
``metadata["kerf"]["id"]`` so a round trip keeps them, and the sequence's own
name and frame size in the timeline's ``metadata["kerf"]``.

An ``.otio`` file does not come here: ``otio_json.rs`` writes and reads it in
Rust. The two hold the same things, so a file means the same whichever wrote
or read it; a change to one is a change to the other.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections.abc import Mapping
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


def _kerf(value: Any) -> Mapping[str, Any]:
    """``metadata["kerf"]`` of an object: a dict set here, or the dictionary
    OpenTimelineIO makes of one it read from a file, which is not a dict."""
    kerf = getattr(value, "metadata", {}).get("kerf")
    return kerf if isinstance(kerf, Mapping) else {}


def _kerf_id(value: Any) -> str | None:
    identifier = _kerf(value).get("id")
    return identifier if isinstance(identifier, str) else None


def _tag(value: Any, item: dict[str, Any]) -> None:
    if item.get("id"):
        value.metadata["kerf"] = {"id": item["id"]}


def _path_from_url(target: str) -> str:
    if not target.lower().startswith("file:"):
        return target
    parsed = urlparse(target)
    path = unquote(parsed.path)
    # ``/C:/footage/a.mov`` is ``C:/footage/a.mov``.
    if re.match(r"^/[A-Za-z]:", path):
        path = path[1:]
    if parsed.netloc and parsed.netloc.lower() not in ("", "localhost"):
        path = f"//{parsed.netloc}{path}"
    return path or target


def _file_name(path: str) -> str:
    return re.split(r"[/\\]", path)[-1]


def _file_stem(path: str) -> str:
    name = _file_name(path)
    dot = name.rfind(".")
    return name[:dot] if dot > 0 else name


def _first_rate(item: Any) -> float | None:
    """The first rate anything in ``item`` counts at."""
    source = getattr(item, "source_range", None)
    if source is not None:
        return float(source.duration.rate)
    if isinstance(item, schema.Clip):
        available = getattr(item.media_reference, "available_range", None)
        return float(available.duration.rate) if available is not None else None
    for child in item if isinstance(item, (schema.Stack, schema.Track)) else ():
        rate = _first_rate(child)
        if rate is not None:
            return rate
    return None


def _marker(marker: Any, rate: float, offset: int = 0) -> dict[str, Any]:
    marked = marker.marked_range
    result: dict[str, Any] = {
        "at": offset + _frames(marked.start_time, rate),
        "duration": _frames(marked.duration, rate),
    }
    if marker.name:
        result["name"] = str(marker.name)
    if marker.color and _kerf(marker).get("uncolored") is not True:
        result["color"] = str(marker.color).lower()
    if marker.comment:
        result["note"] = str(marker.comment)
    if _kerf_id(marker):
        result["id"] = _kerf_id(marker)
    return result


def otio_to_timeline(timeline: Any) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    issues: list[dict[str, Any]] = []
    start_time = timeline.global_start_time
    # A timeline counts at its start time's rate; one with no start time, at
    # the rate of the first thing on it.
    rate = float(start_time.rate) if start_time is not None else (_first_rate(timeline.tracks) or 24.0)
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
        if _kerf_id(reference):
            entry["id"] = _kerf_id(reference)
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
                if child.source_range is None and getattr(child.media_reference, "available_range", None) is None:
                    issues.append(_issue("item_not_imported", "a clip with neither a source range nor media of a known length was not imported", "loss", child_path))
                    continue
                source = child.source_range or child.trimmed_range()
                index = media_of(child, child_path)
                item: dict[str, Any] = {
                    "type": "clip",
                    "media": index,
                    "sourceIn": _frames(source.start_time, media[index]["rate"]),
                    "start": _frames(child.range_in_parent().start_time, rate),
                    "duration": _frames(source.duration, rate),
                }
                # A clip named after itself or its file has no name of its own.
                path_of_media = media[index]["path"]
                if child.name and str(child.name) not in (_kerf_id(child), _file_name(path_of_media), _file_stem(path_of_media)):
                    item["name"] = str(child.name)
                if _kerf_id(child):
                    item["id"] = _kerf_id(child)
                if not child.enabled:
                    item["enabled"] = False
                other_effects = False
                for effect in child.effects:
                    scalar = effect.time_scalar if type(effect) is schema.LinearTimeWarp else None
                    if scalar is not None and scalar > 0:
                        if scalar != 1:
                            item["speed"] = float(scalar)
                        else:
                            item.pop("speed", None)
                    elif scalar is not None and scalar < 0:
                        issues.append(_issue("reverse_not_imported", "a reversed clip was imported playing forward", "loss", child_path))
                        if scalar != -1:
                            item["speed"] = float(-scalar)
                        else:
                            item.pop("speed", None)
                    else:
                        other_effects = True
                if other_effects:
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
                if _kerf_id(child):
                    item["id"] = _kerf_id(child)
                items.append(item)
            elif not isinstance(child, schema.Gap):
                issues.append(_issue("item_not_imported", f"{child.__class__.__name__} items are not imported", "loss", child_path))
        markers.extend(_marker(marker, rate) for marker in track.markers)
        entry = {
            "kind": "video" if track.kind == schema.TrackKind.Video else "audio",
            "enabled": bool(track.enabled),
            "items": items,
        }
        if track.name and str(track.name) != _kerf_id(track):
            entry["name"] = str(track.name)
        if _kerf_id(track):
            entry["id"] = _kerf_id(track)
        tracks.append(entry)

    result = {
        "name": str(timeline.name or ""),
        "rate": rate,
        "start": _frames(start_time, rate) if start_time is not None else 0,
        "media": media,
        "tracks": tracks,
        "markers": markers,
    }
    kerf = timeline.metadata.get("kerf")
    if isinstance(kerf, Mapping):
        sequence: dict[str, Any] = {}
        for field in ("id", "name"):
            if isinstance(kerf.get(field), str) and kerf[field]:
                sequence[field] = kerf[field]
        for field in ("width", "height"):
            if isinstance(kerf.get(field), int) and not isinstance(kerf[field], bool) and 0 <= kerf[field] < 2**32:
                sequence[field] = kerf[field]
        if kerf.get("dropFrame") is True:
            sequence["dropFrame"] = True
        result["sequence"] = sequence
    return result, issues


def _new_marker(value: dict[str, Any], rate: float) -> schema.Marker:
    marker = schema.Marker(name=value.get("name", ""), marked_range=_range(value["at"], value.get("duration", 0), rate))
    color = str(value.get("color") or "").upper()
    _tag(marker, value)
    if color and hasattr(schema.MarkerColor, color):
        marker.color = getattr(schema.MarkerColor, color)
    else:
        # OpenTimelineIO has no marker without a color.
        marker.color = schema.MarkerColor.RED
        marker.metadata["kerf"] = {**_kerf(marker), "uncolored": True}
    if value.get("note"):
        marker.comment = value["note"]
    return marker


def timeline_to_otio(value: dict[str, Any]) -> Any:
    rate = float(value["rate"])
    timeline = schema.Timeline(name=value.get("name", ""))
    timeline.global_start_time = _rt(value.get("start", 0), rate)
    if isinstance(value.get("sequence"), dict):
        timeline.metadata["kerf"] = dict(value["sequence"])
    references = []
    for media in value.get("media", []):
        media_rate = float(media["rate"])
        reference = schema.ExternalReference(target_url=media["path"])
        if media.get("duration") is not None:
            reference.available_range = _range(media.get("start", 0), media["duration"], media_rate)
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
            # How long it plays, counted in its media's frames: a part of one
            # where the two rates differ, so that the cut stays where the
            # sequence has it.
            duration = item["duration"] if media_rate == rate else item["duration"] * media_rate / rate
            clip = schema.Clip(
                name=item.get("name") or _file_name(str(reference.target_url)),
                media_reference=reference.clone(),
                source_range=_range(item["sourceIn"], duration, media_rate),
            )
            if item.get("speed") is not None:
                clip.effects.append(schema.LinearTimeWarp(time_scalar=float(item["speed"])))
            clip.enabled = item.get("enabled", True)
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
    parser = argparse.ArgumentParser(description="Kerf OpenTimelineIO bridge")
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
