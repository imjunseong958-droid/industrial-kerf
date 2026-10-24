//! Feature trajectories through a clip: Shi–Tomasi corners followed with pyramidal
//! Lucas–Kanade and a forward–backward check ([`crate::klt`]); lost features end their track and
//! new ones are detected where the frame has none, so tracks are as long as possible
//! ([`TrackAnalyzer`]). The Warp Stabilizer's Subspace Warp smooths them ([`crate::subspace`]).

pub mod features;
pub mod linalg;

use serde::{Deserialize, Serialize};

pub use features::{AnalyzeOpts, TrackAnalyzer};

/// One feature track: positions (layer pixels) on consecutive frames from `start`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track2D {
    pub id: u32,
    pub start: u32,
    pub pts: Vec<[f32; 2]>,
}

impl Track2D {
    pub fn end(&self) -> u32 {
        self.start + self.pts.len() as u32
    }
    /// Position on frame `k` when the track covers it.
    pub fn at(&self, k: u32) -> Option<[f64; 2]> {
        let i = k.checked_sub(self.start)? as usize;
        self.pts.get(i).map(|p| [p[0] as f64, p[1] as f64])
    }
}

/// The tracked features of a clip (step 1 of the analysis).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CameraTracks {
    pub version: u32,
    /// Layer size (pixels).
    pub size: [f64; 2],
    /// Layer time (seconds) of frame 0, and the frame duration.
    pub start: f64,
    pub frame_duration: f64,
    pub frames: u32,
    /// Analysis downsampling factor (image pixels per analysis pixel): the tracking noise scale.
    pub factor: f64,
    pub detailed: bool,
    pub tracks: Vec<Track2D>,
}

impl CameraTracks {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Option<CameraTracks> {
        if s.trim().is_empty() {
            return None;
        }
        serde_json::from_str(s).ok()
    }
    pub fn is_empty(&self) -> bool {
        self.frames == 0
    }
    pub fn track(&self, id: u32) -> Option<&Track2D> {
        self.tracks.binary_search_by_key(&id, |t| t.id).ok().map(|i| &self.tracks[i])
    }
}
