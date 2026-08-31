#!/usr/bin/env python3
"""OpenTimelineIO bridge for @cut/interchange.

This file intentionally has no installation or download behavior.  The
selected Python executable must already have ``opentimelineio`` and any
optional adapter plugin installed.  The TypeScript side speaks a small JSON
protocol over stdin/stdout so its tests can replace the process entirely.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import PurePosixPath
from typing import Any, Iterable
from urllib.parse import unquote, urlparse

import opentimelineio as otio
from opentimelineio import opentime, schema


NATIVE_ADAPTERS = {"otio_json", "otioz"}


def _jsonable(value: Any) -> Any:
    """Best-effort conversion of OTIO metadata into JSON values."""

    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    if isinstance(value, dict):
        return {str(key): _jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_jsonable(item) for item in value]
    if hasattr(value, "value"):
        try:
            return _jsonable(value.value)
        except Exception:
            pass
    return str(value)


def _metadata(value: Any) -> dict[str, Any]:
    raw = getattr(value, "metadata", None)
    result = _jsonable(raw if isinstance(raw, dict) else {})
    return result if isinstance(result, dict) else {}


def _seconds(value: Any) -> float:
    if value is None:
        return 0.0
    try:
        return float(value.to_seconds())
    except Exception:
        pass
    try:
        return float(value.value) / float(value.rate)
    except Exception:
        return 0.0


def _rate(value: Any, fallback: float = 24.0) -> float:
    try:
        rate = float(value.rate)
        return rate if rate > 0 else fallback
    except Exception:
        return fallback


def _duration(value: Any) -> float:
    try:
        return max(0.0, _seconds(value.duration()))
    except Exception:
        return 0.0


def _range(value: Any) -> dict[str, float] | None:
    if value is None:
        return None
    try:
        return {
            "start": _seconds(value.start_time),
            "duration": max(0.0, _seconds(value.duration)),
            "rate": _rate(value.start_time),
        }
    except Exception:
        return None


def _range_in_parent(value: Any) -> dict[str, float] | None:
    try:
        return _range(value.range_in_parent())
    except Exception:
        return None


def _target_url(media_reference: Any) -> str:
    target = getattr(media_reference, "target_url", None)
    return str(target) if target is not None else ""


def _path_from_url(target: str) -> str:
    if not target.lower().startswith("file:"):
        return target
    parsed = urlparse(target)
    path = unquote(parsed.path)
    if parsed.netloc and parsed.netloc not in ("", "localhost"):
        path = f"//{parsed.netloc}{path}"
    return path or target


def _name(value: Any, fallback: str) -> str:
    name = getattr(value, "name", None)
    return str(name) if name else fallback


def _class_name(value: Any) -> str:
    return value.__class__.__name__.lower()


def _item_kind(value: Any) -> str:
    if isinstance(value, schema.Clip):
        return "clip"
    if isinstance(value, schema.Gap):
        return "gap"
    if isinstance(value, schema.Transition):
        return "transition"
    if isinstance(value, schema.Track):
        return "track"
    if isinstance(value, schema.Stack):
        return "stack"
    return _class_name(value)


class _Ids:
    def __init__(self) -> None:
        self._counts: dict[str, int] = {}
        self._used: set[str] = set()

    def make(self, prefix: str, hint: Any = None) -> str:
        text = str(hint).strip() if hint is not None else ""
        if not text:
            self._counts[prefix] = self._counts.get(prefix, 0) + 1
            text = f"{prefix}-{self._counts[prefix]}"
        candidate = text
        suffix = 2
        while candidate in self._used:
            candidate = f"{text}-{suffix}"
            suffix += 1
        self._used.add(candidate)
        return candidate


def _metadata_id(value: Any) -> str | None:
    metadata = _metadata(value)
    candidate = metadata.get("id")
    return str(candidate) if isinstance(candidate, (str, int)) and str(candidate).strip() else None


def _marker_to_cut(marker: Any, marker_id: str, base: float = 0.0) -> dict[str, Any]:
    marked = getattr(marker, "marked_range", None)
    marked_range = _range(marked)
    start = base + (marked_range["start"] if marked_range else 0.0)
    duration = marked_range["duration"] if marked_range else 0.0
    result: dict[str, Any] = {
        "id": marker_id,
        "time": max(0.0, start),
        "label": str(getattr(marker, "name", "") or ""),
        "duration": duration,
        "metadata": _metadata(marker),
    }
    color = getattr(marker, "color", None)
    if color is not None:
        result["color"] = str(getattr(color, "value", color))
    return result


def _issue(
    code: str,
    message: str,
    severity: str = "warning",
    path: str | None = None,
    details: dict[str, Any] | None = None,
) -> dict[str, Any]:
    result: dict[str, Any] = {"code": code, "message": message, "severity": severity}
    if path is not None:
        result["path"] = path
    if details is not None:
        result["details"] = details
    return result


def _effect_values(value: Any, path: str, issues: list[dict[str, Any]]) -> list[dict[str, Any]]:
    effects = getattr(value, "effects", None)
    if not effects:
        return []
    result: list[dict[str, Any]] = []
    for index, effect in enumerate(effects):
        effect_type = _name(effect, _class_name(effect)) or "otio_effect"
        result.append({"id": f"{effect_type}-{index + 1}", "type": effect_type, "params": _metadata(effect)})
    issues.append(_issue("effects_not_supported", "OTIO effects are retained as descriptive cut effects only.", "loss", path))
    return result


def _clip_source_range(clip: Any) -> tuple[float, float, float]:
    source_range = getattr(clip, "source_range", None)
    parsed = _range(source_range)
    if parsed is None:
        duration = _duration(clip)
        return 0.0, duration, duration
    start = max(0.0, parsed["start"])
    duration = max(0.0, parsed["duration"])
    return start, start + duration, duration


def _item_start(item: Any, fallback: float) -> float:
    parsed = _range_in_parent(item)
    if parsed is not None and parsed["start"] >= -1e-9:
        return max(0.0, parsed["start"])
    return max(0.0, fallback)


def _track_kind(track: Any, issues: list[dict[str, Any]], path: str) -> str:
    kind = str(getattr(track, "kind", "video") or "video").lower()
    if kind in ("video", "audio"):
        return kind
    issues.append(_issue("track_kind_not_supported", f"OTIO track kind {kind} mapped to video.", "warning", path))
    return "video"


def timeline_to_project(timeline: Any) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    issues: list[dict[str, Any]] = []
    ids = _Ids()
    global_start = getattr(timeline, "global_start_time", None)
    fps = _rate(global_start, 24.0)
    if fps <= 0:
        fps = 24.0
    sources: list[dict[str, Any]] = []
    source_by_path: dict[str, str] = {}
    source_duration: dict[str, float] = {}
    tracks: list[dict[str, Any]] = []
    markers: list[dict[str, Any]] = []

    def ensure_source(media_reference: Any, needed_duration: float, path: str) -> str:
        target = _path_from_url(_target_url(media_reference)) if media_reference is not None else ""
        key = target or f"missing:{len(sources) + 1}"
        if key in source_by_path:
            source_id = source_by_path[key]
            source_duration[source_id] = max(source_duration.get(source_id, 0.0), needed_duration)
            return source_id
        source_id = ids.make("source", _metadata_id(media_reference))
        source_by_path[key] = source_id
        available = _range(getattr(media_reference, "available_range", None)) if media_reference is not None else None
        duration = max(needed_duration, available["duration"] if available else 0.0)
        source_duration[source_id] = duration
        entry: dict[str, Any] = {
            "id": source_id,
            "path": target,
            "metadata": _metadata(media_reference) if media_reference is not None else {},
        }
        if duration > 0:
            entry["duration"] = duration
        if media_reference is None or not target:
            issues.append(_issue("missing_media_reference", "Clip has no ExternalReference target URL; a placeholder source was created.", "loss", path))
        sources.append(entry)
        return source_id

    timeline_markers = getattr(timeline, "markers", None) or []
    for index, marker in enumerate(timeline_markers):
        markers.append(_marker_to_cut(marker, ids.make("marker", _metadata_id(marker) or f"timeline-marker-{index + 1}")))

    otio_tracks = getattr(timeline, "tracks", None)
    for track_index, otio_track in enumerate(otio_tracks or []):
        track_path = f"tracks[{track_index}]"
        track_type = _track_kind(otio_track, issues, track_path)
        track_id = ids.make("track", _metadata_id(otio_track) or _name(otio_track, f"track-{track_index + 1}"))
        track: dict[str, Any] = {
            "id": track_id,
            "type": track_type,
            "name": _name(otio_track, track_id),
            "clips": [],
            "gaps": [],
            "transitions": [],
            "markers": [],
        }
        cursor = 0.0
        children = getattr(otio_track, "children", None) or []
        for child_index, child in enumerate(children):
            child_path = f"{track_path}.children[{child_index}]"
            kind = _item_kind(child)
            start = _item_start(child, cursor)
            if kind == "clip":
                source_in, source_out, source_duration_value = _clip_source_range(child)
                output_duration = _duration(child) or max(0.0, source_out - source_in)
                if output_duration <= 0:
                    issues.append(_issue("zero_duration_clip", "Zero-duration OTIO clip omitted from cut v2.", "loss", child_path))
                    continue
                source_id = ensure_source(getattr(child, "media_reference", None), source_out, child_path)
                clip_id = ids.make("clip", _metadata_id(child) or _name(child, f"clip-{track_index + 1}-{child_index + 1}"))
                clip: dict[str, Any] = {
                    "id": clip_id,
                    "sourceId": source_id,
                    "in": source_in,
                    "out": source_out,
                    "timelineStart": start,
                    "duration": output_duration,
                }
                name = _name(child, "")
                if name:
                    clip["name"] = name
                source_span = max(0.0, source_out - source_in)
                if output_duration > 0 and abs(source_span - output_duration) > 1e-6:
                    clip["speed"] = source_span / output_duration
                    issues.append(_issue("rate_conversion", "OTIO clip source and parent durations differ; cut speed was inferred.", "warning", child_path))
                effects = _effect_values(child, child_path + ".effects", issues)
                if effects:
                    clip["effects"] = effects
                metadata = _metadata(child)
                if metadata:
                    clip["metadata"] = metadata
                track["clips"].append(clip)
                for marker_index, marker in enumerate(getattr(child, "markers", None) or []):
                    markers.append(_marker_to_cut(marker, ids.make("marker", _metadata_id(marker) or f"clip-marker-{track_index}-{child_index}-{marker_index}"), start))
                cursor = max(cursor, start + output_duration)
            elif kind == "gap":
                duration = _duration(child)
                if duration > 0:
                    track["gaps"].append({"id": ids.make("gap", f"gap-{track_index + 1}-{child_index + 1}"), "start": start, "duration": duration})
                    cursor = max(cursor, start + duration)
            elif kind == "transition":
                in_offset = _seconds(getattr(child, "in_offset", None))
                out_offset = _seconds(getattr(child, "out_offset", None))
                duration = in_offset + out_offset
                if duration <= 0:
                    issues.append(_issue("zero_duration_transition", "Zero-duration OTIO transition omitted.", "loss", child_path))
                    continue
                transition_type = str(getattr(child, "transition_type", "SMPTE_Dissolve") or "SMPTE_Dissolve")
                track["transitions"].append({
                    "id": ids.make("transition", _metadata_id(child) or f"transition-{track_index + 1}-{child_index + 1}"),
                    "type": transition_type,
                    "duration": duration,
                    "alignment": "center",
                    "start": start,
                    "params": {"inOffset": in_offset, "outOffset": out_offset, "metadata": _metadata(child)},
                })
                cursor = max(cursor, start)
            elif kind in ("track", "stack"):
                issues.append(_issue("nested_timeline_not_supported", "Nested OTIO tracks/stacks were not flattened into a cut track.", "loss", child_path))
            else:
                issues.append(_issue("unsupported_item", f"OTIO item {kind} was not mapped.", "loss", child_path))

        for marker_index, marker in enumerate(getattr(otio_track, "markers", None) or []):
            track["markers"].append(_marker_to_cut(marker, ids.make("marker", _metadata_id(marker) or f"track-marker-{track_index}-{marker_index}")))
            markers.append(track["markers"][-1])
        track_metadata = _metadata(otio_track)
        if track_metadata:
            track["metadata"] = track_metadata
        # Empty arrays are valid but keeping them makes the generated JSON
        # explicit for consumers inspecting the interchange report.
        tracks.append(track)

    for source in sources:
        duration = source_duration.get(source["id"], 0.0)
        if duration > 0:
            source["duration"] = duration

    project: dict[str, Any] = {
        "version": 2,
        "id": ids.make("project", _metadata_id(timeline) or _name(timeline, "project")),
        "name": _name(timeline, ""),
        "fps": fps,
        "sources": sources,
        "tracks": tracks,
        "overlays": [],
        "gaps": [],
        "transitions": [],
        "effects": [],
        "markers": markers,
        "metadata": _metadata(timeline),
    }
    total_duration = _duration(timeline)
    if total_duration > 0:
        project["duration"] = total_duration
    return project, issues


def _rt(seconds: float, rate: float) -> opentime.RationalTime:
    return opentime.RationalTime(float(seconds) * rate, rate)


def _tr(start: float, duration: float, rate: float) -> opentime.TimeRange:
    return opentime.TimeRange(_rt(max(0.0, start), rate), _rt(max(0.0, duration), rate))


def _string(value: Any, fallback: str = "") -> str:
    return value if isinstance(value, str) else fallback


def _transition_type(value: Any) -> Any:
    """Return the OTIO transition enum when the installed API exposes one."""

    raw = _string(value, "SMPTE_Dissolve")
    enum = getattr(schema, "TransitionTypes", None)
    if enum is None:
        return raw
    normalized = raw.replace(" ", "_").replace("-", "_").upper()
    if normalized in ("SMPTE_DISSOLVE", "SMPTE_DISSOLVE_TRANSITION"):
        return getattr(enum, "SMPTE_DISSOLVE", raw)
    if normalized in ("CUSTOM", "CUSTOM_TRANSITION"):
        return getattr(enum, "CUSTOM", raw)
    return getattr(enum, normalized, raw)


def _source_refs(project: dict[str, Any], rate: float) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for source in project.get("sources", []):
        if not isinstance(source, dict):
            continue
        source_id = _string(source.get("id"))
        if not source_id:
            continue
        path = _string(source.get("path"))
        reference = schema.ExternalReference(target_url=path)
        duration = source.get("duration")
        if isinstance(duration, (int, float)) and float(duration) > 0:
            reference.available_range = _tr(0.0, float(duration), rate)
        metadata = source.get("metadata")
        if isinstance(metadata, dict):
            reference.metadata.update(_jsonable(metadata))
        result[source_id] = reference
    return result


def _copy_metadata(target: Any, values: Any) -> None:
    if isinstance(values, dict):
        target.metadata.update(_jsonable(values))


def _new_marker(marker_value: dict[str, Any], rate: float) -> schema.Marker:
    marker = schema.Marker()
    marker.name = _string(marker_value.get("label"), _string(marker_value.get("id"), "marker"))
    start = marker_value.get("time", marker_value.get("start", 0.0))
    duration = marker_value.get("duration", 0.0)
    start_number = float(start) if isinstance(start, (int, float)) else 0.0
    duration_number = float(duration) if isinstance(duration, (int, float)) else 0.0
    marker.marked_range = _tr(start_number, max(0.0, duration_number), rate)
    _copy_metadata(marker, marker_value.get("metadata"))
    return marker


def project_to_timeline(project: dict[str, Any]) -> tuple[Any, list[dict[str, Any]]]:
    issues: list[dict[str, Any]] = []
    fps_value = project.get("fps", 24.0)
    fps = float(fps_value) if isinstance(fps_value, (int, float)) and float(fps_value) > 0 else 24.0
    timeline = schema.Timeline(name=_string(project.get("name"), _string(project.get("id"), "cut-project")))
    timeline.global_start_time = _rt(0.0, fps)
    _copy_metadata(timeline, project.get("metadata"))
    timeline.metadata.setdefault("cut_v2", {})
    if isinstance(timeline.metadata["cut_v2"], dict):
        timeline.metadata["cut_v2"]["version"] = 2
    refs = _source_refs(project, fps)

    track_values = project.get("tracks", [])
    track_objects: list[schema.Track] = []
    for track_index, track_value in enumerate(track_values if isinstance(track_values, list) else []):
        if not isinstance(track_value, dict):
            continue
        track_type = _string(track_value.get("type"), _string(track_value.get("kind"), "video"))
        if track_type not in ("video", "audio"):
            issues.append(_issue("track_type_not_supported", f"{track_type} track was not emitted as OTIO.", "loss", f"tracks[{track_index}]"))
            continue
        track = schema.Track(name=_string(track_value.get("name"), _string(track_value.get("id"), f"track-{track_index + 1}")), kind=track_type)
        _copy_metadata(track, track_value.get("metadata"))
        if track_value.get("enabled") is False or track_value.get("muted") is True or track_value.get("locked") is True:
            issues.append(_issue("track_state_not_supported", "Track enabled/muted/locked state was not emitted as OTIO metadata.", "loss", f"tracks[{track_index}]"))
        if isinstance(track_value.get("effects"), list) and track_value.get("effects"):
            issues.append(_issue("effects_not_supported", "Track effects were not emitted as OTIO effects.", "loss", f"tracks[{track_index}].effects"))
        events: list[tuple[float, str, dict[str, Any], str]] = []
        clips = track_value.get("clips", [])
        for clip_index, clip_value in enumerate(clips if isinstance(clips, list) else []):
            if isinstance(clip_value, dict):
                start_value = clip_value.get("timelineStart", clip_value.get("start", 0.0))
                start = float(start_value) if isinstance(start_value, (int, float)) else 0.0
                events.append((max(0.0, start), "clip", clip_value, f"tracks[{track_index}].clips[{clip_index}].timelineStart"))
        gaps = track_value.get("gaps", [])
        for gap_index, gap_value in enumerate(gaps if isinstance(gaps, list) else []):
            if isinstance(gap_value, dict):
                start_value = gap_value.get("start", gap_value.get("timelineStart", 0.0))
                duration_value = gap_value.get("duration", 0.0)
                if isinstance(start_value, (int, float)) and isinstance(duration_value, (int, float)) and float(duration_value) > 0:
                    events.append((max(0.0, float(start_value)), "gap", gap_value, f"tracks[{track_index}].gaps[{gap_index}].start"))
        transitions = track_value.get("transitions", [])
        for transition_index, transition_value in enumerate(transitions if isinstance(transitions, list) else []):
            if isinstance(transition_value, dict):
                start_value = transition_value.get("start", transition_value.get("timelineStart", 0.0))
                if isinstance(start_value, (int, float)):
                    events.append((max(0.0, float(start_value)), "transition", transition_value, f"tracks[{track_index}].transitions[{transition_index}].start"))
        events.sort(key=lambda value: (value[0], {"gap": 0, "transition": 1, "clip": 2}.get(value[1], 3)))
        cursor = 0.0

        def append_gap(duration: float) -> None:
            nonlocal cursor
            if duration > 1e-9:
                track.append(schema.Gap(source_range=_tr(0.0, duration, fps)))
                cursor += duration

        for start, kind, value, event_path in events:
            requested_start = start
            if start < cursor - 1e-9:
                issues.append(
                    _issue(
                        "placement_overlap",
                        f"Explicit placement at {requested_start} seconds overlaps the preceding OTIO track item; it was shifted to {cursor} seconds.",
                        "loss",
                        event_path,
                        {"requestedStart": requested_start, "effectiveStart": cursor},
                    )
                )
                start = cursor
            if start > cursor + 1e-9:
                append_gap(start - cursor)
            if kind == "clip":
                source_id = _string(value.get("sourceId"), _string(value.get("src"), _string(value.get("source"), "")))
                reference = refs.get(source_id)
                if reference is None:
                    issues.append(_issue("missing_source", "Cut clip source was not found; a MissingReference was emitted.", "loss"))
                    media_reference: Any = schema.MissingReference()
                else:
                    media_reference = schema.ExternalReference(target_url=reference.target_url)
                    if getattr(reference, "available_range", None) is not None:
                        media_reference.available_range = reference.available_range
                clip = schema.Clip(name=_string(value.get("name"), _string(value.get("id"), "clip")), media_reference=media_reference)
                source_in = value.get("in", value.get("sourceIn", 0.0))
                source_out = value.get("out", value.get("sourceOut"))
                source_in_number = float(source_in) if isinstance(source_in, (int, float)) else 0.0
                if not isinstance(source_out, (int, float)):
                    duration_value = value.get("duration", 0.0)
                    source_out = source_in_number + (float(duration_value) if isinstance(duration_value, (int, float)) else 0.0)
                source_out_number = float(source_out)
                if source_out_number <= source_in_number:
                    source_out_number = source_in_number + 1.0 / fps
                    issues.append(_issue("invalid_source_range", "Cut clip source range was adjusted to a positive duration.", "loss"))
                clip.source_range = _tr(source_in_number, source_out_number - source_in_number, fps)
                _copy_metadata(clip, value.get("metadata"))
                cut_metadata = clip.metadata.setdefault("cut_v2", {})
                if isinstance(cut_metadata, dict):
                    cut_metadata["id"] = _string(value.get("id"), "")
                    speed = value.get("speed")
                    if isinstance(speed, (int, float)) and float(speed) != 1.0:
                        cut_metadata["speed"] = float(speed)
                effects = value.get("effects")
                if isinstance(effects, list) and effects:
                    issues.append(_issue("effects_not_supported", "Cut effects were not emitted as OTIO effects.", "loss"))
                if isinstance(value.get("keyframes"), list) and value.get("keyframes"):
                    issues.append(_issue("keyframes_not_supported", "Cut keyframes were not emitted as OTIO effects.", "loss", f"tracks[{track_index}].clips[{clip_index}].keyframes"))
                if value.get("blend") is not None or value.get("blendMode") is not None:
                    issues.append(_issue("blend_not_supported", "Cut blend mode was not emitted as OTIO compositing.", "loss", f"tracks[{track_index}].clips[{clip_index}].blend"))
                clip_path = f"tracks[{track_index}].clips[{clip_index}]"
                if value.get("crop") is not None:
                    issues.append(_issue("crop_not_supported", "Cut crop was not emitted as an OTIO effect.", "loss", f"{clip_path}.crop"))
                if value.get("volume") is not None:
                    issues.append(_issue("volume_not_supported", "Cut volume was not emitted as OTIO metadata.", "loss", f"{clip_path}.volume"))
                if value.get("opacity") is not None:
                    issues.append(_issue("opacity_not_supported", "Cut opacity was not emitted as OTIO metadata.", "loss", f"{clip_path}.opacity"))
                if isinstance(value.get("speed"), (int, float)) and abs(float(value["speed"]) - 1.0) > 1e-9:
                    issues.append(_issue("speed_not_supported", "Cut playback speed was retained only in metadata; OTIO clip duration is unchanged.", "loss", f"{clip_path}.speed"))
                if value.get("transform") is not None:
                    issues.append(_issue("transform_not_supported", "Cut transform was not emitted as an OTIO effect.", "loss", f"{clip_path}.transform"))
                track.append(clip)
                duration = _duration(clip)
                explicit_duration = value.get("duration")
                if isinstance(explicit_duration, (int, float)) and abs(float(explicit_duration) - duration) > 1e-6:
                    issues.append(_issue("speed_not_supported", "Cut playback speed was retained in metadata; OTIO clip duration is unchanged.", "loss", f"tracks[{track_index}].clips[{clip_index}].speed"))
                cursor = max(cursor, start + duration)
            elif kind == "gap":
                duration = float(value.get("duration", 0.0)) if isinstance(value.get("duration"), (int, float)) else 0.0
                append_gap(duration)
                cursor = max(cursor, start + duration)
            elif kind == "transition":
                params = value.get("params") if isinstance(value.get("params"), dict) else {}
                in_value = params.get("inOffset", 0.0)
                out_value = params.get("outOffset", 0.0)
                duration = value.get("duration", 0.0)
                in_offset = float(in_value) if isinstance(in_value, (int, float)) else 0.0
                out_offset = float(out_value) if isinstance(out_value, (int, float)) else 0.0
                if in_offset + out_offset <= 0 and isinstance(duration, (int, float)):
                    in_offset = float(duration) / 2.0
                    out_offset = float(duration) - in_offset
                if in_offset + out_offset <= 0:
                    issues.append(_issue("zero_duration_transition", "Zero-duration transition was omitted.", "loss"))
                    continue
                transition = schema.Transition(
                    transition_type=_transition_type(value.get("type")),
                    in_offset=_rt(in_offset, fps),
                    out_offset=_rt(out_offset, fps),
                )
                _copy_metadata(transition, params.get("metadata"))
                track.append(transition)
                # OTIO transitions occupy the cut rather than adding full
                # sequential duration; keep cursor at the explicit position.
                cursor = max(cursor, start)
        for marker_index, marker_value in enumerate(track_value.get("markers", []) if isinstance(track_value.get("markers"), list) else []):
            if isinstance(marker_value, dict):
                track.markers.append(_new_marker(marker_value, fps))
                if marker_value.get("color") is not None:
                    issues.append(_issue("marker_color_not_supported", "Marker color was not written to the OTIO marker.", "loss", f"tracks[{track_index}].markers[{marker_index}].color"))
        track_objects.append(track)
        timeline.tracks.append(track)

    # Root markers are directly portable. Track-local markers were attached
    # above; project-level gaps/transitions have no owner in OTIO, so report
    # and retain them in metadata rather than silently dropping them.
    for marker_value in project.get("markers", []) if isinstance(project.get("markers"), list) else []:
        if isinstance(marker_value, dict):
            timeline.markers.append(_new_marker(marker_value, fps))
            if marker_value.get("color") is not None:
                marker_index = len(timeline.markers) - 1
                issues.append(_issue("marker_color_not_supported", "Marker color was not written to the OTIO marker.", "loss", f"markers[{marker_index}].color"))
    if isinstance(project.get("gaps"), list) and project.get("gaps"):
        issues.append(_issue("project_gaps_not_track_scoped", "Project-level gaps are not portable without a track owner.", "loss", "gaps"))
    if isinstance(project.get("transitions"), list) and project.get("transitions"):
        issues.append(_issue("project_transitions_not_track_scoped", "Project-level transitions are not portable without a track owner.", "loss", "transitions"))
    if project.get("width") is not None or project.get("height") is not None:
        issues.append(_issue("project_dimensions_not_supported", "Project dimensions were not emitted to OTIO.", "loss", "width"))
    for source_index, source_value in enumerate(project.get("sources", []) if isinstance(project.get("sources"), list) else []):
        if isinstance(source_value, dict) and any(source_value.get(field) is not None for field in ("width", "height", "hasAudio")):
            issues.append(_issue("source_properties_not_supported", "Source dimensions/audio flags were not emitted to OTIO.", "loss", f"sources[{source_index}]"))
    overlays = project.get("overlays")
    if isinstance(overlays, list) and overlays:
        issues.append(_issue("overlays_not_supported", "Project overlays were not emitted as OTIO composables.", "loss", "overlays"))
    return timeline, issues


def _read(path: str, adapter: str | None) -> Any:
    if adapter:
        return otio.adapters.read_from_file(path, adapter_name=adapter)
    return otio.adapters.read_from_file(path)


def _write(timeline: Any, path: str, adapter: str) -> None:
    otio.adapters.write_to_file(timeline, path, adapter_name=adapter)


def _emit(payload: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def _run(args: argparse.Namespace) -> int:
    if args.operation == "import":
        timeline = _read(args.input, args.adapter)
        project, issues = timeline_to_project(timeline)
        _emit({"ok": True, "project": project, "report": {"issues": issues}})
        return 0

    raw = sys.stdin.read()
    payload = json.loads(raw)
    project_value = payload.get("project") if isinstance(payload, dict) and "project" in payload else payload
    if not isinstance(project_value, dict):
        raise ValueError("export input must be a JSON object containing project")
    timeline, issues = project_to_timeline(project_value)
    _write(timeline, args.output, args.adapter)
    _emit({"ok": True, "outputPath": args.output, "report": {"issues": issues}})
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
