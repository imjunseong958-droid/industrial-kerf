use crate::agent_rail::AgentRailViewModel;
use crate::config::AppConfig;
use crate::inspector::InspectorViewModel;
use crate::media_bin::MediaBinViewModel;
use crate::settings::SettingsViewModel;
use crate::timeline::TimelineViewModel;
use crate::viewer::ViewerViewModel;
use cut_runtime::StateSnapshot;
use serde_json::Value;

/// Projection of authoritative daemon state into the six native editor
/// surfaces. No canonical project JSON is retained here.
#[derive(Clone, Debug, PartialEq)]
pub struct AppViewModel {
    pub project_name: String,
    pub revision: u64,
    pub duration: f64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub projection_status: String,
    pub viewer: ViewerViewModel,
    pub timeline: TimelineViewModel,
    pub inspector: InspectorViewModel,
    pub agent_rail: AgentRailViewModel,
    pub media_bin: MediaBinViewModel,
    pub settings: SettingsViewModel,
}

impl AppViewModel {
    pub fn from_snapshot(snapshot: &StateSnapshot, config: &AppConfig, endpoint: String) -> Self {
        let project_name = snapshot
            .project
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("Untitled project")
            .to_owned();
        Self {
            project_name,
            revision: snapshot.revision,
            duration: snapshot.duration,
            can_undo: snapshot.can_undo,
            can_redo: snapshot.can_redo,
            projection_status: snapshot.projection_status.clone(),
            viewer: ViewerViewModel::new(snapshot.duration),
            timeline: TimelineViewModel::from_project(&snapshot.project),
            inspector: InspectorViewModel,
            agent_rail: AgentRailViewModel::from_snapshot(snapshot),
            media_bin: MediaBinViewModel::from_snapshot(snapshot),
            settings: SettingsViewModel::new(config.project_directory(), endpoint, config.actor()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppViewModel;
    use crate::config::AppConfig;
    use cut_runtime::StateSnapshot;
    use serde_json::json;

    #[test]
    fn projection_reads_only_the_snapshot_surface() {
        let snapshot = StateSnapshot {
            project: json!({
                "name": "Native project",
                "tracks": [{"id":"video-1","type":"video","clips":[]}],
                "sources": []
            }),
            revision: 7,
            can_undo: true,
            can_redo: false,
            duration: 12.5,
            sources: vec![],
            pending_proposal: None,
            source_version: 2,
            projection_status: "current".into(),
        };
        let config = AppConfig::for_project_directory("/tmp/project", "/tmp/cutd");
        let model = AppViewModel::from_snapshot(&snapshot, &config, "/tmp/cutd.sock".into());
        assert_eq!(model.project_name, "Native project");
        assert_eq!(model.revision, 7);
        assert_eq!(model.timeline.tracks.len(), 1);
        assert_eq!(model.settings.actor, "ui");
    }
}
