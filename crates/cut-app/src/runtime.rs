use crate::config::AppConfig;
use cut_core::FrameIndex;
use cut_domain::{CropRegion, Effect, Keyframe, Transform, Transition, TrimEdge};
use cut_runtime::{RenderJob, RenderRange, RuntimeClient, RuntimeError, StateSnapshot};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

/// Typed nested clip properties emitted by the inspector.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipProperty {
    Crop(Option<CropRegion>),
    Transform(Option<Transform>),
    Opacity(f64),
    Effects(Vec<Effect>),
    Keyframes(Vec<Keyframe>),
    Transitions(Vec<Transition>),
}

/// The application-side session is a thin request adapter. It deliberately
/// stores a client, not a project snapshot; every mutation is sent to `cutd`
/// with the revision currently displayed by the view model.
#[derive(Clone, Debug)]
pub struct AppRuntime {
    client: RuntimeClient,
    actor: String,
}

impl AppRuntime {
    /// Connect to `cutd` and load the first authoritative snapshot.
    pub fn open(config: &AppConfig) -> Result<(Self, StateSnapshot), RuntimeError> {
        let client = RuntimeClient::connect_or_spawn(
            config.project_directory(),
            Some(config.cutd_binary()),
        )?;
        let runtime = Self {
            client,
            actor: config.actor().to_owned(),
        };
        let snapshot = runtime.refresh()?;
        Ok((runtime, snapshot))
    }

    pub fn refresh(&self) -> Result<StateSnapshot, RuntimeError> {
        self.client.get_state()
    }

    /// Apply one canonical wire command through the daemon.
    pub fn apply_command(
        &self,
        command: Value,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.client
            .apply_request(&self.actor, &operation_key("apply"), command, base_revision)
    }

    /// Add a track as the first native command surface.
    pub fn add_video_track(&self, base_revision: u64) -> Result<StateSnapshot, RuntimeError> {
        let number = NEXT_KEY.load(Ordering::Relaxed);
        let track_id = operation_key("track");
        self.apply_command(add_video_track_command(&track_id, number), base_revision)
    }

    /// Apply the native inspector's clip-volume edit through cutd.
    pub fn update_clip_volume(
        &self,
        clip_id: &str,
        volume: f64,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        if !volume.is_finite() {
            return Err(RuntimeError::new(
                "INVALID_ARGUMENT",
                "clip volume must be finite",
            ));
        }
        self.apply_command(
            update_clip_volume_command(clip_id, volume.clamp(0.0, 2.0)),
            base_revision,
        )
    }

    /// Update the timeline start and explicit duration as one undoable edit.
    pub fn update_clip_timing(
        &self,
        clip_id: &str,
        timeline_start: f64,
        duration: f64,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        if !timeline_start.is_finite() || timeline_start < 0.0 {
            return Err(RuntimeError::new(
                "INVALID_ARGUMENT",
                "clip timeline start must be finite and nonnegative",
            ));
        }
        if !duration.is_finite() || duration <= 0.0 {
            return Err(RuntimeError::new(
                "INVALID_ARGUMENT",
                "clip duration must be finite and positive",
            ));
        }
        self.apply_command(
            update_clip_timing_command(clip_id, timeline_start, duration),
            base_revision,
        )
    }

    /// Update a clip's source interval in one canonical command.
    pub fn update_clip_source_range(
        &self,
        clip_id: &str,
        source_in: f64,
        source_out: f64,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        if !source_in.is_finite()
            || source_in < 0.0
            || !source_out.is_finite()
            || source_out <= source_in
        {
            return Err(RuntimeError::new(
                "INVALID_ARGUMENT",
                "clip source range must be finite and positive",
            ));
        }
        self.apply_command(
            update_clip_source_range_command(clip_id, source_in, source_out),
            base_revision,
        )
    }

    /// Update a clip's speed in one canonical command.
    pub fn update_clip_speed(
        &self,
        clip_id: &str,
        speed: f64,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        if !speed.is_finite() || speed <= 0.0 {
            return Err(RuntimeError::new(
                "INVALID_ARGUMENT",
                "clip speed must be finite and positive",
            ));
        }
        self.apply_command(update_clip_speed_command(clip_id, speed), base_revision)
    }

    /// Set a typed nested clip property. The inspector chooses the property;
    /// this adapter only serializes its already-typed value for the wire.
    pub fn update_clip_property(
        &self,
        clip_id: &str,
        property: ClipProperty,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        let (name, value) = match property {
            ClipProperty::Crop(crop) => (
                "crop",
                serde_json::to_value(crop).map_err(serialization_error)?,
            ),
            ClipProperty::Transform(transform) => (
                "transform",
                serde_json::to_value(transform).map_err(serialization_error)?,
            ),
            ClipProperty::Opacity(opacity)
                if opacity.is_finite() && (0.0..=1.0).contains(&opacity) =>
            {
                ("opacity", Value::from(opacity))
            }
            ClipProperty::Opacity(_) => {
                return Err(RuntimeError::new(
                    "INVALID_ARGUMENT",
                    "clip opacity must be finite and between 0 and 1",
                ));
            }
            ClipProperty::Effects(effects) => (
                "effects",
                serde_json::to_value(effects).map_err(serialization_error)?,
            ),
            ClipProperty::Keyframes(keyframes) => (
                "keyframes",
                serde_json::to_value(keyframes).map_err(serialization_error)?,
            ),
            ClipProperty::Transitions(transitions) => (
                "transitions",
                serde_json::to_value(transitions).map_err(serialization_error)?,
            ),
        };
        self.apply_command(
            update_clip_property_command(clip_id, name, value),
            base_revision,
        )
    }

    /// Import one local media file through the atomic runtime import command.
    pub fn import_media(
        &self,
        path: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        let (snapshot, _, _) = self.client.import_media_request(
            &self.actor,
            &operation_key("import"),
            path,
            base_revision,
            None,
            None,
        )?;
        Ok(snapshot)
    }

    /// Place an existing clip on the project frame grid, optionally changing tracks.
    pub fn place_clip(
        &self,
        clip_id: &str,
        timeline_frame: FrameIndex,
        track_id: Option<&str>,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.apply_command(
            place_clip_command(clip_id, timeline_frame, track_id),
            base_revision,
        )
    }

    /// Trim one edge of an existing clip at a project frame.
    pub fn trim_clip(
        &self,
        clip_id: &str,
        edge: TrimEdge,
        timeline_frame: FrameIndex,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.apply_command(
            trim_clip_command(clip_id, edge, timeline_frame),
            base_revision,
        )
    }

    /// Split an existing clip and assign the right half a fresh stable ID.
    pub fn split_clip(
        &self,
        clip_id: &str,
        at_frame: FrameIndex,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        let new_clip_id = operation_key("split-clip");
        self.apply_command(
            split_clip_command(clip_id, at_frame, &new_clip_id),
            base_revision,
        )
    }

    /// Remove one clip; the daemon records its inverse for undo.
    pub fn remove_clip(
        &self,
        clip_id: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.apply_command(remove_clip_command(clip_id), base_revision)
    }

    pub fn undo(&self, base_revision: u64) -> Result<StateSnapshot, RuntimeError> {
        self.client
            .undo_request(&self.actor, &operation_key("undo"), base_revision)
    }

    pub fn redo(&self, base_revision: u64) -> Result<StateSnapshot, RuntimeError> {
        self.client
            .redo_request(&self.actor, &operation_key("redo"), base_revision)
    }

    pub fn accept_proposal(
        &self,
        proposal_id: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.client.accept_proposal(
            proposal_id,
            &self.actor,
            &operation_key("accept-proposal"),
            base_revision,
        )
    }

    pub fn reject_proposal(
        &self,
        proposal_id: &str,
        base_revision: u64,
    ) -> Result<StateSnapshot, RuntimeError> {
        self.client.reject_proposal_request(
            proposal_id,
            &self.actor,
            &operation_key("reject-proposal"),
            base_revision,
        )
    }

    pub fn endpoint_label(&self) -> String {
        match self.client.endpoint() {
            cut_runtime::EndpointAddress::Unix { path } => path.display().to_string(),
            cut_runtime::EndpointAddress::WindowsNamedPipe { name } => name.clone(),
        }
    }

    pub fn project_directory(&self) -> &Path {
        self.client.directory()
    }

    pub fn start_preview_render(
        &self,
        range: RenderRange,
    ) -> Result<(RenderJob, Value, std::path::PathBuf), RuntimeError> {
        self.client.start_render_range("preview", Some(range))
    }

    pub fn render_job(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
        self.client.render_job(job_id)
    }

    pub fn update_render_job(
        &self,
        job_id: &str,
        status: &str,
        error: Option<String>,
        output_url: Option<String>,
        progress: Option<f64>,
    ) -> Result<(), RuntimeError> {
        self.client
            .update_job_with_progress(job_id, status, error, output_url, progress)
    }

    pub fn cancel_render(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
        self.client.cancel_render(job_id)
    }
}

/// Build the canonical command used to add a native video track.
pub fn add_video_track_command(track_id: &str, number: u64) -> Value {
    json!({
        "type": "add_track",
        "track": {
            "id": track_id,
            "type": "video",
            "name": format!("Video {number}"),
            "clips": []
        }
    })
}

fn update_clip_volume_command(clip_id: &str, volume: f64) -> Value {
    update_clip_property_command(clip_id, "volume", json!(volume))
}

/// Build the typed inspector command for timeline timing.
pub fn update_clip_timing_command(clip_id: &str, timeline_start: f64, duration: f64) -> Value {
    json!({
        "type": "update_clip",
        "clipId": clip_id,
        "patch": {"timelineStart": timeline_start, "duration": duration}
    })
}

/// Build the typed inspector command for a source interval.
pub fn update_clip_source_range_command(clip_id: &str, source_in: f64, source_out: f64) -> Value {
    json!({
        "type": "update_clip",
        "clipId": clip_id,
        "patch": {"in": source_in, "out": source_out}
    })
}

/// Build the typed inspector command for playback speed.
pub fn update_clip_speed_command(clip_id: &str, speed: f64) -> Value {
    update_clip_property_command(clip_id, "speed", json!(speed))
}

/// Build one canonical update command for an inspector property.
fn update_clip_property_command(clip_id: &str, property: &str, value: Value) -> Value {
    let mut patch = serde_json::Map::new();
    patch.insert(property.to_owned(), value);
    json!({
        "type": "update_clip",
        "clipId": clip_id,
        "patch": patch
    })
}

fn serialization_error(error: serde_json::Error) -> RuntimeError {
    RuntimeError::new("SERIALIZATION_ERROR", error.to_string())
}

/// Build a semantic command that places an existing clip at a project frame.
pub fn place_clip_command(
    clip_id: &str,
    timeline_frame: FrameIndex,
    track_id: Option<&str>,
) -> Value {
    let mut command = json!({
        "type": "place_clip",
        "clipId": clip_id,
        "timelineFrame": timeline_frame,
    });
    if let Some(track_id) = track_id {
        command["trackId"] = json!(track_id);
    }
    command
}

/// Build a semantic command that trims one edge at a project frame.
pub fn trim_clip_command(clip_id: &str, edge: TrimEdge, timeline_frame: FrameIndex) -> Value {
    let edge = match edge {
        TrimEdge::Leading => "leading",
        TrimEdge::Trailing => "trailing",
    };
    json!({
        "type": "trim_clip",
        "clipId": clip_id,
        "edge": edge,
        "timelineFrame": timeline_frame,
    })
}

/// Build a semantic command that splits a clip and names its right half.
pub fn split_clip_command(clip_id: &str, at_frame: FrameIndex, new_clip_id: &str) -> Value {
    json!({
        "type": "split_clip",
        "clipId": clip_id,
        "atFrame": at_frame,
        "newClipId": new_clip_id,
    })
}

/// Build the canonical command used by timeline deletion.
pub fn remove_clip_command(clip_id: &str) -> Value {
    json!({"type": "remove_clip", "clipId": clip_id})
}

fn operation_key(kind: &str) -> String {
    let sequence = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "native-{kind}-{}-{sequence}-{timestamp}",
        std::process::id()
    )
}

/// Run the command surface without creating a window. This is used by CI and
/// the native smoke script to prove that the real `cut` binary reads, edits,
/// undoes, and redoes canonical state through `cutd`.
pub fn run_command_surface_smoke(config: &AppConfig) -> Result<Value, RuntimeError> {
    let (runtime, initial) = AppRuntime::open(config)?;
    let added = runtime.add_video_track(initial.revision)?;
    let undone = runtime.undo(added.revision)?;
    let redone = runtime.redo(undone.revision)?;
    Ok(json!({
        "initialRevision": initial.revision,
        "addedRevision": added.revision,
        "undoRevision": undone.revision,
        "redoRevision": redone.revision,
        "trackCount": redone.project.get("tracks").and_then(Value::as_array).map_or(0, Vec::len),
        "canUndo": redone.can_undo,
        "canRedo": redone.can_redo,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        add_video_track_command, place_clip_command, remove_clip_command, split_clip_command,
        trim_clip_command, update_clip_source_range_command, update_clip_speed_command,
        update_clip_timing_command, update_clip_volume_command,
    };
    use cut_domain::TrimEdge;

    #[test]
    fn first_command_surface_is_a_canonical_add_track_command() {
        let command = add_video_track_command("native-video-1", 1);
        assert_eq!(command["type"], "add_track");
        assert_eq!(command["track"]["type"], "video");
        assert_eq!(command["track"]["clips"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn inspector_volume_uses_the_canonical_update_clip_command() {
        let command = update_clip_volume_command("clip-1", 0.8);
        assert_eq!(command["type"], "update_clip");
        assert_eq!(command["clipId"], "clip-1");
        assert_eq!(command["patch"]["volume"], 0.8);
    }

    #[test]
    fn inspector_timing_source_range_and_speed_use_typed_fields() {
        assert_eq!(
            update_clip_timing_command("clip-1", 2.0, 4.0)["patch"],
            serde_json::json!({"timelineStart": 2.0, "duration": 4.0})
        );
        assert_eq!(
            update_clip_source_range_command("clip-1", 1.0, 5.0)["patch"],
            serde_json::json!({"in": 1.0, "out": 5.0})
        );
        assert_eq!(
            update_clip_speed_command("clip-1", 1.5)["patch"]["speed"],
            1.5
        );
    }

    #[test]
    fn timeline_commands_use_frame_grid_fields_and_typed_edges() {
        assert_eq!(
            place_clip_command("clip-1", 42, Some("video-2")),
            serde_json::json!({
                "type":"place_clip",
                "clipId":"clip-1",
                "timelineFrame":42,
                "trackId":"video-2"
            })
        );
        assert_eq!(
            trim_clip_command("clip-1", TrimEdge::Trailing, 90)["edge"],
            "trailing"
        );
        assert_eq!(
            split_clip_command("clip-1", 90, "clip-1-right")["atFrame"],
            90
        );
        assert_eq!(
            remove_clip_command("clip-1"),
            serde_json::json!({"type":"remove_clip", "clipId":"clip-1"})
        );
    }
}
