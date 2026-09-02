//! Deterministic, read-only checks for a v2 project.
//!
//! The checker deliberately reports findings instead of repairing the project.
//! It is used by agents before they ask for a render or propose an edit, so its
//! output must be stable, bounded, and independent of UI state.

use crate::capability::{capability, CapabilitySupport, EditCapability, CAPABILITY_MATRIX};
use crate::compile::{compile_filter_graph_v2, CompileV2Options};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Maximum number of findings returned by one check.
pub const MAX_FINDINGS: usize = 256;

/// Severity of a timeline check finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingSeverity {
    Error,
    Warning,
    Info,
}

/// A finite project-time interval attached to a finding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingTimeRange {
    pub start: f64,
    pub end: f64,
}

/// One stable, machine-readable timeline check result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineCheckFinding {
    pub code: String,
    pub severity: FindingSeverity,
    pub entity_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_range: Option<FindingTimeRange>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
}

/// Bounded result of checking a project without mutating canonical state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineCheckReport {
    pub findings: Vec<TimelineCheckFinding>,
    pub truncated: bool,
}

impl TimelineCheckReport {
    /// Whether the project has no error findings.
    pub fn is_valid(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|finding| finding.severity == FindingSeverity::Error)
    }
}

#[derive(Clone, Debug)]
struct SourceInfo {
    id: String,
    path: Option<String>,
    duration: Option<f64>,
    entity_path: String,
    available: bool,
}

#[derive(Clone, Debug)]
struct ClipInfo {
    id: Option<String>,
    track_type: String,
    entity_path: String,
    source_id: Option<String>,
    start: Option<f64>,
    duration: Option<f64>,
    source_in: Option<f64>,
    source_out: Option<f64>,
    speed: Option<f64>,
    opacity: Option<f64>,
    enabled: bool,
    timeline_end: Option<f64>,
}

#[derive(Clone, Debug)]
struct ProjectFacts {
    duration: f64,
    authored_duration: Option<f64>,
    sources: Vec<SourceInfo>,
    source_by_id: HashMap<String, usize>,
    clips: Vec<ClipInfo>,
    video_coverage: Vec<(f64, f64, String, String)>,
    used_capabilities: Vec<CapabilityUse>,
}

#[derive(Clone, Debug)]
struct CapabilityUse {
    capability: EditCapability,
    entity_path: String,
    entity_id: Option<String>,
    time_range: Option<FindingTimeRange>,
    detail: String,
}

/// Check a v2 project against source availability, timeline structure,
/// compiler support, and the live/offline capability contract.
pub fn check_project(project: &Value, media_root: Option<&Path>) -> TimelineCheckReport {
    let mut collector = FindingCollector::default();
    let Some(object) = project.as_object() else {
        collector.push(TimelineCheckFinding {
            code: "compiler_rejected_feature".into(),
            severity: FindingSeverity::Error,
            entity_path: "/".into(),
            entity_id: None,
            time_range: None,
            message: "timeline project must be a JSON object".into(),
            suggested_action: Some("provide_a_v2_project_object".into()),
        });
        return collector.finish();
    };

    let facts = collect_facts(object, media_root, &mut collector);
    report_source_findings(&facts, object, &mut collector);
    report_range_findings(&facts, &mut collector);
    report_never_visible_findings(&facts, &mut collector);
    report_black_gaps(&facts, &mut collector);
    report_capability_mismatches(&facts, &mut collector);
    // An empty timeline is a valid editing state even though it cannot be
    // rendered yet. Structural findings above still report malformed projects.
    if facts.duration > 0.0 {
        report_compiler_findings(project, &mut collector);
    }
    collector.finish()
}

#[derive(Default)]
struct FindingCollector {
    findings: Vec<TimelineCheckFinding>,
    truncated: bool,
}

impl FindingCollector {
    fn push(&mut self, finding: TimelineCheckFinding) {
        if self.findings.len() >= MAX_FINDINGS {
            self.truncated = true;
            return;
        }
        self.findings.push(finding);
    }

    fn finish(self) -> TimelineCheckReport {
        TimelineCheckReport {
            findings: self.findings,
            truncated: self.truncated,
        }
    }
}

fn collect_facts(
    project: &Map<String, Value>,
    media_root: Option<&Path>,
    collector: &mut FindingCollector,
) -> ProjectFacts {
    let mut sources = Vec::new();
    let mut source_by_id = HashMap::new();
    if let Some(source_values) = project.get("sources").and_then(Value::as_array) {
        for (index, value) in source_values.iter().enumerate() {
            let path = value
                .as_object()
                .and_then(|source| source.get("path"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let id = value
                .as_object()
                .and_then(|source| source.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("source-{index}"));
            let duration = value
                .as_object()
                .and_then(|source| source.get("duration"))
                .and_then(finite_nonnegative);
            let available = path
                .as_deref()
                .map(|path| resolve_source_path(media_root, path).is_file())
                .unwrap_or(false);
            source_by_id.insert(id.clone(), sources.len());
            sources.push(SourceInfo {
                id,
                path,
                duration,
                entity_path: format!("/sources/{index}"),
                available,
            });
        }
    } else {
        collector.push(TimelineCheckFinding {
            code: "compiler_rejected_feature".into(),
            severity: FindingSeverity::Error,
            entity_path: "/sources".into(),
            entity_id: None,
            time_range: None,
            message: "v2 project requires a sources array".into(),
            suggested_action: Some("add_sources_array".into()),
        });
    }

    let mut clips = Vec::new();
    let mut video_coverage = Vec::new();
    let mut used_capabilities = Vec::new();
    if let Some(tracks) = project.get("tracks").and_then(Value::as_array) {
        for (track_index, track_value) in tracks.iter().enumerate() {
            let Some(track) = track_value.as_object() else {
                collector.push(TimelineCheckFinding {
                    code: "compiler_rejected_feature".into(),
                    severity: FindingSeverity::Error,
                    entity_path: format!("/tracks/{track_index}"),
                    entity_id: None,
                    time_range: None,
                    message: "track must be an object".into(),
                    suggested_action: Some("replace_with_a_valid_track".into()),
                });
                continue;
            };
            let track_id = track.get("id").and_then(Value::as_str).map(str::to_owned);
            let track_type = track
                .get("type")
                .or_else(|| track.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or("video")
                .to_ascii_lowercase();
            let track_enabled = track.get("enabled").and_then(Value::as_bool) != Some(false);
            let track_path = format!("/tracks/{track_index}");

            record_track_capabilities(
                track,
                &track_path,
                track_id.as_deref(),
                &track_type,
                track_enabled,
                &mut used_capabilities,
            );

            let Some(raw_clips) = track.get("clips").and_then(Value::as_array) else {
                collector.push(TimelineCheckFinding {
                    code: "compiler_rejected_feature".into(),
                    severity: FindingSeverity::Error,
                    entity_path: format!("{track_path}/clips"),
                    entity_id: track_id.clone(),
                    time_range: None,
                    message: "v2 track requires a clips array".into(),
                    suggested_action: Some("add_clips_array".into()),
                });
                continue;
            };

            for (clip_index, clip_value) in raw_clips.iter().enumerate() {
                let clip_path = format!("{track_path}/clips/{clip_index}");
                let Some(clip) = clip_value.as_object() else {
                    collector.push(TimelineCheckFinding {
                        code: "compiler_rejected_feature".into(),
                        severity: FindingSeverity::Error,
                        entity_path: clip_path,
                        entity_id: track_id.clone(),
                        time_range: None,
                        message: "clip must be an object".into(),
                        suggested_action: Some("replace_with_a_valid_clip".into()),
                    });
                    continue;
                };
                let id = clip.get("id").and_then(Value::as_str).map(str::to_owned);
                let source_id = clip
                    .get("sourceId")
                    .or_else(|| clip.get("src"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let start = clip
                    .get("timelineStart")
                    .or_else(|| clip.get("start"))
                    .and_then(finite_nonnegative);
                let explicit_duration = clip.get("duration").and_then(finite_number);
                let source_in = clip
                    .get("in")
                    .or_else(|| clip.get("sourceIn"))
                    .and_then(finite_number)
                    .or(Some(0.0));
                let source_out = clip
                    .get("out")
                    .or_else(|| clip.get("sourceOut"))
                    .and_then(finite_number);
                let speed = clip.get("speed").and_then(finite_number);
                let opacity = clip.get("opacity").and_then(finite_number).or(Some(1.0));
                let enabled = clip.get("enabled").and_then(Value::as_bool) != Some(false);
                let source_duration = source_id
                    .as_deref()
                    .and_then(|source_id| source_by_id.get(source_id))
                    .and_then(|index| sources.get(*index))
                    .and_then(|source| source.duration);
                let effective_source_out = source_out.or(source_duration);
                let effective_duration = explicit_duration.or_else(|| {
                    let speed = speed.unwrap_or(1.0);
                    match (effective_source_out, source_in) {
                        (Some(source_out), Some(source_in))
                            if speed > 0.0 && source_out > source_in =>
                        {
                            Some((source_out - source_in) / speed)
                        }
                        _ => None,
                    }
                });
                let timeline_end = start
                    .zip(effective_duration)
                    .map(|(start, duration)| start + duration);
                let time_range = timeline_range(start, timeline_end);

                record_clip_capabilities(
                    clip,
                    &clip_path,
                    id.as_deref(),
                    time_range.clone(),
                    &track_type,
                    track_enabled && enabled,
                    &mut used_capabilities,
                );

                if track_enabled && enabled && track_type == "video" && opacity.unwrap_or(1.0) > 0.0
                {
                    if let Some(range) = time_range.as_ref() {
                        video_coverage.push((
                            range.start,
                            range.end,
                            clip_path.clone(),
                            id.clone().unwrap_or_default(),
                        ));
                    }
                }

                clips.push(ClipInfo {
                    id,
                    track_type: track_type.clone(),
                    entity_path: clip_path,
                    source_id,
                    start,
                    duration: effective_duration,
                    source_in,
                    source_out,
                    speed,
                    opacity,
                    enabled: track_enabled && enabled,
                    timeline_end,
                });
            }
        }
    } else {
        collector.push(TimelineCheckFinding {
            code: "compiler_rejected_feature".into(),
            severity: FindingSeverity::Error,
            entity_path: "/tracks".into(),
            entity_id: None,
            time_range: None,
            message: "v2 project requires a tracks array".into(),
            suggested_action: Some("add_tracks_array".into()),
        });
    }

    let duration = project_duration(project, &clips);
    record_project_capabilities(
        project,
        &sources,
        &clips,
        &video_coverage,
        &mut used_capabilities,
    );
    ProjectFacts {
        duration,
        authored_duration: project.get("duration").and_then(finite_nonnegative),
        sources,
        source_by_id,
        clips,
        video_coverage,
        used_capabilities,
    }
}

fn report_source_findings(
    facts: &ProjectFacts,
    _project: &Map<String, Value>,
    collector: &mut FindingCollector,
) {
    for source in &facts.sources {
        if source.path.is_none() || !source.available {
            let detail = source
                .path
                .as_deref()
                .map(|path| format!("source is missing or offline: {path}"))
                .unwrap_or_else(|| "source has no media path".into());
            collector.push(TimelineCheckFinding {
                code: "missing_source".into(),
                severity: FindingSeverity::Error,
                entity_path: source.entity_path.clone(),
                entity_id: Some(source.id.clone()),
                time_range: None,
                message: detail,
                suggested_action: Some("relink_or_import_source".into()),
            });
        }
    }
}

fn report_range_findings(facts: &ProjectFacts, collector: &mut FindingCollector) {
    for clip in &facts.clips {
        let Some(source_id) = clip.source_id.as_deref() else {
            if clip.track_type == "video" || clip.track_type == "audio" {
                collector.push(TimelineCheckFinding {
                    code: "missing_source".into(),
                    severity: FindingSeverity::Error,
                    entity_path: clip.entity_path.clone(),
                    entity_id: clip.id.clone(),
                    time_range: timeline_range(clip.start, clip.timeline_end),
                    message: "media clip has no sourceId".into(),
                    suggested_action: Some("attach_a_source".into()),
                });
            }
            continue;
        };
        let Some(source_index) = facts.source_by_id.get(source_id).copied() else {
            collector.push(TimelineCheckFinding {
                code: "missing_source".into(),
                severity: FindingSeverity::Error,
                entity_path: clip.entity_path.clone(),
                entity_id: clip.id.clone(),
                time_range: timeline_range(clip.start, clip.timeline_end),
                message: format!("clip references unknown source: {source_id}"),
                suggested_action: Some("attach_an_existing_source_or_import_source".into()),
            });
            continue;
        };
        let source = &facts.sources[source_index];
        let source_in = clip.source_in.unwrap_or(0.0);
        let source_out = clip.source_out;
        let invalid_range = !source_in.is_finite()
            || source_in < 0.0
            || source_out.is_some_and(|out| !out.is_finite() || out <= source_in)
            || clip
                .duration
                .is_some_and(|duration| !duration.is_finite() || duration <= 0.0)
            || clip
                .speed
                .is_some_and(|speed| !speed.is_finite() || speed <= 0.0)
            || clip.start.is_none();
        if invalid_range {
            collector.push(TimelineCheckFinding {
                code: "zero_or_negative_range".into(),
                severity: FindingSeverity::Error,
                entity_path: clip.entity_path.clone(),
                entity_id: clip.id.clone(),
                time_range: timeline_range(clip.start, clip.timeline_end),
                message: "clip has a zero, negative, or invalid source/timeline range".into(),
                suggested_action: Some("set_a_positive_source_range_and_timeline_duration".into()),
            });
        }
        if let Some(source_duration) = source.duration {
            let requested_source_out = source_out.or_else(|| {
                clip.duration
                    .filter(|duration| duration.is_finite() && *duration > 0.0)
                    .and_then(|duration| {
                        let speed = clip.speed.unwrap_or(1.0);
                        speed.is_finite().then_some(source_in + duration * speed)
                    })
            });
            let overflows = source_in > source_duration
                || requested_source_out.is_some_and(|out| out > source_duration);
            if overflows {
                let displayed_out = requested_source_out.unwrap_or(source_duration);
                collector.push(TimelineCheckFinding {
                    code: "source_range_overflow".into(),
                    severity: FindingSeverity::Error,
                    entity_path: clip.entity_path.clone(),
                    entity_id: clip.id.clone(),
                    time_range: timeline_range(clip.start, clip.timeline_end),
                    message: format!(
                        "clip source range [{source_in}, {displayed_out}] exceeds source duration {source_duration}"
                    ),
                    suggested_action: Some("trim_source_range_to_source_duration".into()),
                });
            }
        }
        if let (Some(start), Some(end)) = (clip.start, clip.timeline_end) {
            if !start.is_finite() || !end.is_finite() || end <= start {
                collector.push(TimelineCheckFinding {
                    code: "zero_or_negative_range".into(),
                    severity: FindingSeverity::Error,
                    entity_path: clip.entity_path.clone(),
                    entity_id: clip.id.clone(),
                    time_range: None,
                    message: "clip timeline range must have a positive duration".into(),
                    suggested_action: Some("set_a_positive_clip_duration".into()),
                });
            }
        }
    }
}

fn report_never_visible_findings(facts: &ProjectFacts, collector: &mut FindingCollector) {
    let visible_end = facts.authored_duration.unwrap_or(facts.duration);
    for clip in &facts.clips {
        let reason = if !clip.enabled {
            Some("clip or track is disabled")
        } else if clip.opacity == Some(0.0) {
            Some("clip opacity is zero")
        } else if clip
            .timeline_end
            .zip(Some(visible_end))
            .is_some_and(|(end, project_end)| {
                end <= 0.0 || clip.start.unwrap_or(0.0) >= project_end
            })
        {
            Some("clip lies outside the project time range")
        } else {
            None
        };
        if let Some(reason) = reason {
            collector.push(TimelineCheckFinding {
                code: "never_visible_clip".into(),
                severity: FindingSeverity::Warning,
                entity_path: clip.entity_path.clone(),
                entity_id: clip.id.clone(),
                time_range: timeline_range(clip.start, clip.timeline_end),
                message: reason.into(),
                suggested_action: Some("enable_clip_or_remove_unreachable_clip".into()),
            });
        }
    }
}

fn report_black_gaps(facts: &ProjectFacts, collector: &mut FindingCollector) {
    if facts.duration.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return;
    }
    let mut coverage = facts
        .video_coverage
        .iter()
        .filter_map(|(start, end, path, id)| {
            let start = start.max(0.0);
            let end = end.min(facts.duration);
            (end > start).then_some((start, end, path.clone(), id.clone()))
        })
        .collect::<Vec<_>>();
    coverage.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.total_cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });

    let mut cursor = 0.0;
    for (start, end, _, _) in coverage {
        if start > cursor + 1e-9 {
            push_black_gap(cursor, start, collector);
        }
        cursor = cursor.max(end);
    }
    if cursor < facts.duration - 1e-9 {
        push_black_gap(cursor, facts.duration, collector);
    }
}

fn push_black_gap(start: f64, end: f64, collector: &mut FindingCollector) {
    collector.push(TimelineCheckFinding {
        code: "black_canvas_gap".into(),
        severity: FindingSeverity::Warning,
        entity_path: "/tracks".into(),
        entity_id: None,
        time_range: Some(FindingTimeRange { start, end }),
        message: format!("video coverage leaves a black canvas gap from {start:.3}s to {end:.3}s"),
        suggested_action: Some("place_or_extend_a_visible_video_clip".into()),
    });
}

fn report_capability_mismatches(facts: &ProjectFacts, collector: &mut FindingCollector) {
    let mut seen = HashSet::new();
    for used in &facts.used_capabilities {
        let entry = capability(used.capability);
        if entry.live.is_exact()
            || !seen.insert((
                used.capability,
                used.entity_path.clone(),
                used.entity_id.clone(),
            ))
        {
            continue;
        }
        let live = support_name(entry.live);
        let offline = support_name(entry.offline);
        collector.push(TimelineCheckFinding {
            code: "live_offline_capability_mismatch".into(),
            severity: FindingSeverity::Warning,
            entity_path: used.entity_path.clone(),
            entity_id: used.entity_id.clone(),
            time_range: used.time_range.clone(),
            message: format!(
                "{} uses {}; native preview is {live}, offline export is {offline}. Live: {} Offline: {}",
                used.detail,
                capability_name(used.capability),
                entry.live_note,
                entry.offline_note
            ),
            suggested_action: Some("use_timeline_capture_for_an_exact_preview".into()),
        });
    }
}

fn report_compiler_findings(project: &Value, collector: &mut FindingCollector) {
    if let Err(error) = compile_filter_graph_v2(project, CompileV2Options::default()) {
        let feature = error.feature().unwrap_or("project");
        let (entity_path, entity_id) = compiler_entity(project, &error.to_string());
        collector.push(TimelineCheckFinding {
            code: "compiler_rejected_feature".into(),
            severity: FindingSeverity::Error,
            entity_path,
            entity_id,
            time_range: None,
            message: format!("offline compiler rejected {feature}: {error}"),
            suggested_action: Some(
                "remove_the_unsupported_feature_or_use_a_rendered_preview".into(),
            ),
        });
    }
}

fn record_track_capabilities(
    track: &Map<String, Value>,
    entity_path: &str,
    track_id: Option<&str>,
    track_type: &str,
    enabled: bool,
    uses: &mut Vec<CapabilityUse>,
) {
    if !enabled {
        return;
    }
    if track_type == "title" {
        uses.push(CapabilityUse {
            capability: EditCapability::Titles,
            entity_path: entity_path.into(),
            entity_id: track_id.map(str::to_owned),
            time_range: None,
            detail: format!("track {}", track_id.unwrap_or("<unnamed>")),
        });
    }
    if track
        .get("effects")
        .and_then(Value::as_array)
        .is_some_and(|effects| effects.iter().any(is_enabled_value))
    {
        uses.push(CapabilityUse {
            capability: EditCapability::Effects,
            entity_path: format!("{entity_path}/effects"),
            entity_id: track_id.map(str::to_owned),
            time_range: None,
            detail: format!("track {}", track_id.unwrap_or("<unnamed>")),
        });
    }
    if track
        .get("transitions")
        .and_then(Value::as_array)
        .is_some_and(|transitions| !transitions.is_empty())
    {
        uses.push(CapabilityUse {
            capability: EditCapability::Transitions,
            entity_path: format!("{entity_path}/transitions"),
            entity_id: track_id.map(str::to_owned),
            time_range: None,
            detail: format!("track {}", track_id.unwrap_or("<unnamed>")),
        });
    }
}

fn record_clip_capabilities(
    clip: &Map<String, Value>,
    entity_path: &str,
    clip_id: Option<&str>,
    time_range: Option<FindingTimeRange>,
    track_type: &str,
    active: bool,
    uses: &mut Vec<CapabilityUse>,
) {
    let detail = format!("clip {}", clip_id.unwrap_or("<unnamed>"));
    if clip
        .get("timelineStart")
        .or_else(|| clip.get("start"))
        .is_some()
    {
        uses.push(capability_use(
            EditCapability::Placement,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if clip.get("in").or_else(|| clip.get("sourceIn")).is_some()
        || clip.get("out").or_else(|| clip.get("sourceOut")).is_some()
    {
        uses.push(capability_use(
            EditCapability::SourceRange,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if clip
        .get("speed")
        .and_then(finite_number)
        .is_some_and(|speed| (speed - 1.0).abs() > 1e-9)
    {
        uses.push(capability_use(
            EditCapability::Speed,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if clip
        .get("volume")
        .and_then(finite_number)
        .is_some_and(|volume| (volume - 1.0).abs() > 1e-9)
    {
        uses.push(capability_use(
            EditCapability::Volume,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if clip.get("crop").is_some_and(|value| !value.is_null()) {
        uses.push(capability_use(
            EditCapability::Crop,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if clip.get("transform").is_some_and(|value| !value.is_null()) {
        uses.push(capability_use(
            EditCapability::Transform,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if clip
        .get("opacity")
        .and_then(finite_number)
        .is_some_and(|opacity| (opacity - 1.0).abs() > 1e-9)
    {
        uses.push(capability_use(
            EditCapability::Opacity,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if clip
        .get("effects")
        .and_then(Value::as_array)
        .is_some_and(|effects| effects.iter().any(is_enabled_value))
    {
        uses.push(capability_use(
            EditCapability::Effects,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if clip
        .get("keyframes")
        .and_then(Value::as_array)
        .is_some_and(|keyframes| !keyframes.is_empty())
    {
        uses.push(capability_use(
            EditCapability::Keyframes,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if clip
        .get("transitions")
        .and_then(Value::as_array)
        .is_some_and(|transitions| !transitions.is_empty())
        || clip.get("transition").is_some_and(|value| !value.is_null())
    {
        uses.push(capability_use(
            EditCapability::Transitions,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if active && track_type == "audio" {
        uses.push(capability_use(
            EditCapability::AudioMixing,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
    if track_type == "title" || clip.get("text").is_some() || clip.get("title").is_some() {
        uses.push(capability_use(
            EditCapability::Titles,
            entity_path,
            clip_id,
            time_range.clone(),
            &detail,
        ));
    }
}

fn record_project_capabilities(
    project: &Map<String, Value>,
    sources: &[SourceInfo],
    clips: &[ClipInfo],
    video_coverage: &[(f64, f64, String, String)],
    uses: &mut Vec<CapabilityUse>,
) {
    for (index, left) in video_coverage.iter().enumerate() {
        for right in video_coverage.iter().skip(index + 1) {
            if left.2 == right.2 {
                continue;
            }
            let start = left.0.max(right.0);
            let end = left.1.min(right.1);
            if end <= start {
                continue;
            }
            uses.push(capability_use(
                EditCapability::LayeredVideo,
                "/tracks",
                None,
                Some(FindingTimeRange { start, end }),
                "video timeline",
            ));
            break;
        }
    }

    let has_explicit_audio = clips
        .iter()
        .any(|clip| clip.enabled && clip.track_type == "audio");
    if !has_explicit_audio {
        for source in sources {
            if source_has_audio(project, source) {
                uses.push(capability_use(
                    EditCapability::AudioMixing,
                    &source.entity_path,
                    Some(&source.id),
                    None,
                    "embedded source audio",
                ));
            }
        }
    }

    if let Some(overlays) = project.get("overlays").and_then(Value::as_array) {
        for (index, overlay) in overlays.iter().enumerate() {
            let kind = overlay
                .as_object()
                .and_then(|object| object.get("type").or_else(|| object.get("kind")))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if kind != "captions" && kind != "subtitles" {
                uses.push(capability_use(
                    EditCapability::Overlays,
                    &format!("/overlays/{index}"),
                    overlay
                        .as_object()
                        .and_then(|object| object.get("id"))
                        .and_then(Value::as_str),
                    None,
                    "generic overlay",
                ));
            }
        }
    }
}

fn source_has_audio(project: &Map<String, Value>, source: &SourceInfo) -> bool {
    project
        .get("sources")
        .and_then(Value::as_array)
        .and_then(|sources| {
            sources.iter().find(|value| {
                value
                    .as_object()
                    .and_then(|object| object.get("id"))
                    .and_then(Value::as_str)
                    == Some(source.id.as_str())
            })
        })
        .and_then(|value| value.as_object())
        .and_then(|source| source.get("hasAudio"))
        .and_then(Value::as_bool)
        == Some(true)
}

fn capability_use(
    capability: EditCapability,
    entity_path: &str,
    entity_id: Option<&str>,
    time_range: Option<FindingTimeRange>,
    detail: &str,
) -> CapabilityUse {
    CapabilityUse {
        capability,
        entity_path: entity_path.into(),
        entity_id: entity_id.map(str::to_owned),
        time_range,
        detail: detail.into(),
    }
}

fn project_duration(project: &Map<String, Value>, clips: &[ClipInfo]) -> f64 {
    let mut duration = project
        .get("duration")
        .and_then(finite_nonnegative)
        .unwrap_or(0.0);
    for clip in clips {
        if clip.enabled {
            duration = duration.max(clip.timeline_end.unwrap_or(0.0));
        }
    }
    if let Some(gaps) = project.get("gaps").and_then(Value::as_array) {
        for gap in gaps {
            let Some(gap) = gap.as_object() else {
                continue;
            };
            let start = gap
                .get("start")
                .or_else(|| gap.get("timelineStart"))
                .and_then(finite_nonnegative)
                .unwrap_or(0.0);
            let length = gap
                .get("duration")
                .and_then(finite_nonnegative)
                .unwrap_or(0.0);
            duration = duration.max(start + length);
        }
    }
    if let Some(tracks) = project.get("tracks").and_then(Value::as_array) {
        for track in tracks {
            let Some(track) = track.as_object() else {
                continue;
            };
            if track.get("enabled").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            let Some(gaps) = track.get("gaps").and_then(Value::as_array) else {
                continue;
            };
            for gap in gaps {
                let Some(gap) = gap.as_object() else {
                    continue;
                };
                let start = gap
                    .get("start")
                    .or_else(|| gap.get("timelineStart"))
                    .and_then(finite_nonnegative)
                    .unwrap_or(0.0);
                let length = gap
                    .get("duration")
                    .and_then(finite_nonnegative)
                    .unwrap_or(0.0);
                duration = duration.max(start + length);
            }
        }
    }
    duration
}

fn timeline_range(start: Option<f64>, end: Option<f64>) -> Option<FindingTimeRange> {
    let (Some(start), Some(end)) = (start, end) else {
        return None;
    };
    (start.is_finite() && end.is_finite() && end >= start)
        .then_some(FindingTimeRange { start, end })
}

fn resolve_source_path(root: Option<&Path>, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.map(|root| root.join(path))
            .unwrap_or_else(|| path.to_path_buf())
    }
}

fn finite_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|value| value.is_finite())
}

fn finite_nonnegative(value: &Value) -> Option<f64> {
    finite_number(value).filter(|value| *value >= 0.0)
}

fn is_enabled_value(value: &Value) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("enabled"))
        .and_then(Value::as_bool)
        != Some(false)
}

fn support_name(support: CapabilitySupport) -> &'static str {
    match support {
        CapabilitySupport::Exact => "exact",
        CapabilitySupport::Limited => "limited",
        CapabilitySupport::Unsupported => "unsupported",
    }
}

fn capability_name(capability: EditCapability) -> &'static str {
    CAPABILITY_MATRIX
        .iter()
        .find(|entry| entry.capability == capability)
        .map(|entry| match entry.capability {
            EditCapability::Placement => "placement",
            EditCapability::SourceRange => "source range",
            EditCapability::Speed => "speed",
            EditCapability::Volume => "volume",
            EditCapability::Captions => "captions",
            EditCapability::Crop => "crop",
            EditCapability::Transform => "transform",
            EditCapability::Opacity => "opacity",
            EditCapability::LayeredVideo => "layered video",
            EditCapability::AudioMixing => "audio mixing",
            EditCapability::Effects => "effects",
            EditCapability::Keyframes => "keyframes",
            EditCapability::Transitions => "transitions",
            EditCapability::Overlays => "overlays",
            EditCapability::Titles => "titles",
        })
        .unwrap_or("capability")
}

fn compiler_entity(project: &Value, message: &str) -> (String, Option<String>) {
    if let Some(tracks) = project.get("tracks").and_then(Value::as_array) {
        for (track_index, track) in tracks.iter().enumerate() {
            let Some(track) = track.as_object() else {
                continue;
            };
            let track_id = track.get("id").and_then(Value::as_str);
            if let Some(clips) = track.get("clips").and_then(Value::as_array) {
                for (clip_index, clip) in clips.iter().enumerate() {
                    let clip_id = clip
                        .as_object()
                        .and_then(|clip| clip.get("id"))
                        .and_then(Value::as_str);
                    if clip_id.is_some_and(|id| message.contains(&format!("clip {id}"))) {
                        return (
                            format!("/tracks/{track_index}/clips/{clip_index}"),
                            clip_id.map(str::to_owned),
                        );
                    }
                }
            }
            if track_id.is_some_and(|id| message.contains(&format!("track {id}"))) {
                return (
                    format!("/tracks/{track_index}"),
                    track_id.map(str::to_owned),
                );
            }
        }
    }
    if let Some(overlays) = project.get("overlays").and_then(Value::as_array) {
        for (index, overlay) in overlays.iter().enumerate() {
            let id = overlay
                .as_object()
                .and_then(|overlay| overlay.get("id"))
                .and_then(Value::as_str);
            if id.is_some_and(|id| message.contains(id)) {
                return (format!("/overlays/{index}"), id.map(str::to_owned));
            }
        }
    }
    for marker in ["clip ", "track ", "overlay "] {
        if let Some(start) = message.find(marker) {
            let suffix = &message[start + marker.len()..];
            let id = suffix
                .split(|character: char| character.is_whitespace() || character == ':')
                .next()
                .unwrap_or_default();
            if !id.is_empty() {
                return (format!("/{marker}{id}"), Some(id.to_owned()));
            }
        }
    }
    ("/".into(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn empty_timeline_is_not_a_compiler_error() {
        let project = json!({
            "version": 2,
            "fps": 30,
            "sources": [],
            "tracks": [
                {"id": "video", "type": "video", "clips": []},
                {"id": "audio", "type": "audio", "clips": []}
            ]
        });

        let report = check_project(&project, None);
        assert!(report.is_valid());
        assert!(!report
            .findings
            .iter()
            .any(|finding| finding.code == "compiler_rejected_feature"));
    }

    #[test]
    fn reports_missing_source_and_black_gap_deterministically() {
        let root = tempdir().expect("temp directory");
        let project = json!({
            "version": 2,
            "fps": 30,
            "duration": 4,
            "sources": [{"id": "source-a", "path": "missing.mp4", "duration": 4}],
            "tracks": [{
                "id": "video-1",
                "type": "video",
                "clips": [{"id": "clip-a", "sourceId": "source-a", "in": 0, "out": 1, "timelineStart": 0}]
            }]
        });
        let report = check_project(&project, Some(root.path()));
        assert_eq!(report.findings[0].code, "missing_source");
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "black_canvas_gap"));
    }

    #[test]
    fn reports_compiler_rejection_and_live_offline_mismatch() {
        let project = json!({
            "version": 2,
            "fps": 30,
            "sources": [{"id": "source-a", "path": "/tmp/a.mp4", "duration": 4}],
            "tracks": [{
                "id": "video-1",
                "type": "video",
                "clips": [{
                    "id": "clip-a", "sourceId": "source-a", "in": 0, "out": 1,
                    "timelineStart": 0, "crop": {"x": 0, "y": 0, "width": 1, "height": 1},
                    "effects": [{"type": "blur"}]
                }]
            }]
        });
        let report = check_project(&project, None);
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "compiler_rejected_feature"));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "live_offline_capability_mismatch"));
    }

    #[test]
    fn reports_inferred_source_overflow_invalid_speed_and_track_gap_duration() {
        let root = tempdir().expect("temp directory");
        std::fs::write(root.path().join("source.mp4"), b"placeholder").expect("source file");
        let project = json!({
            "version": 2,
            "fps": 30,
            "sources": [{"id": "source-a", "path": "source.mp4", "duration": 10}],
            "tracks": [{
                "id": "video-1",
                "type": "video",
                "gaps": [{"start": 0, "duration": 20}],
                "clips": [{
                    "id": "clip-a", "sourceId": "source-a", "in": 8, "duration": 2,
                    "speed": 2, "timelineStart": 0
                }, {
                    "id": "clip-b", "sourceId": "source-a", "in": 0, "out": 1,
                    "speed": 0, "timelineStart": 20
                }]
            }]
        });
        let report = check_project(&project, Some(root.path()));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "source_range_overflow"));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "zero_or_negative_range"));
        assert!(report.findings.iter().any(|finding| {
            finding.code == "black_canvas_gap"
                && finding
                    .time_range
                    .as_ref()
                    .is_some_and(|range| (range.end - 20.0).abs() < 1e-9)
        }));

        let default_speed_project = json!({
            "version": 2,
            "fps": 30,
            "sources": [{"id": "source-a", "path": "source.mp4", "duration": 10}],
            "tracks": [{
                "id": "video-1",
                "type": "video",
                "clips": [{
                    "id": "clip-a", "sourceId": "source-a", "in": 0, "out": 10,
                    "timelineStart": 0
                }]
            }]
        });
        let default_speed_report = check_project(&default_speed_project, Some(root.path()));
        assert!(!default_speed_report
            .findings
            .iter()
            .any(|finding| finding.code == "black_canvas_gap"));
    }

    #[test]
    fn findings_are_bounded() {
        let clips = (0..400)
            .map(|index| json!({"id": format!("c{index}"), "sourceId": "missing", "in": 0, "out": 0, "timelineStart": 0}))
            .collect::<Vec<_>>();
        let project = json!({
            "version": 2,
            "fps": 30,
            "sources": [],
            "tracks": [{"id": "video", "type": "video", "clips": clips}]
        });
        let report = check_project(&project, None);
        assert_eq!(report.findings.len(), MAX_FINDINGS);
        assert!(report.truncated);
    }

    #[test]
    fn reports_each_never_visible_reason() {
        let project = json!({
            "version": 2,
            "duration": 4,
            "sources": [{"id": "source", "path": "/tmp/source.mp4", "duration": 10}],
            "tracks": [{
                "id": "video",
                "type": "video",
                "clips": [
                    {"id": "disabled", "sourceId": "source", "enabled": false, "duration": 1, "timelineStart": 0},
                    {"id": "transparent", "sourceId": "source", "opacity": 0, "duration": 1, "timelineStart": 1},
                    {"id": "outside", "sourceId": "source", "duration": 1, "timelineStart": 5}
                ]
            }]
        });
        let report = check_project(&project, None);
        let ids = report
            .findings
            .iter()
            .filter(|finding| finding.code == "never_visible_clip")
            .filter_map(|finding| finding.entity_id.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["disabled", "transparent", "outside"]);
    }
}
