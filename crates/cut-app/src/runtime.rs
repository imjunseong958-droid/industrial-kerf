use crate::config::AppConfig;
use cut_runtime::{RuntimeClient, RuntimeError, StateSnapshot};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

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
}

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
    json!({
        "type": "update_clip",
        "clipId": clip_id,
        "patch": {"volume": volume}
    })
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
    use super::{add_video_track_command, update_clip_volume_command};

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
}
