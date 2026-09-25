//! Declared parity between native playback and offline compilation.
//!
//! This is a product contract, not a feature flag system. Callers use it to
//! decide whether native playback is exact enough to present as a preview.

use serde::{Deserialize, Serialize};

/// A video-domain behavior whose preview/export parity matters to an editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditCapability {
    Placement,
    SourceRange,
    Speed,
    Volume,
    Captions,
    Crop,
    Transform,
    Opacity,
    LayeredVideo,
    AudioMixing,
    Effects,
    Keyframes,
    Transitions,
    Overlays,
    Titles,
}

/// How completely one evaluation path implements a capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    /// The current path implements the authored behavior used by Cut.
    Exact,
    /// A documented subset is implemented; unsupported inputs need fallback.
    Limited,
    /// The path does not evaluate the authored behavior.
    Unsupported,
}

impl CapabilitySupport {
    /// Whether this support level is safe to present as an exact preview.
    pub const fn is_exact(self) -> bool {
        matches!(self, Self::Exact)
    }
}

/// Support declared for native playback and the offline v2 compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityEntry {
    pub capability: EditCapability,
    pub live: CapabilitySupport,
    pub offline: CapabilitySupport,
    pub live_note: &'static str,
    pub offline_note: &'static str,
}

/// Typed live/offline contract for every editor-facing capability in Phase 0.
pub const CAPABILITY_MATRIX: &[CapabilityEntry] = &[
    entry(
        EditCapability::Placement,
        CapabilitySupport::Exact,
        CapabilitySupport::Exact,
        "timelineStart selects the active native clip",
        "clips are placed on the output timeline",
    ),
    entry(
        EditCapability::SourceRange,
        CapabilitySupport::Exact,
        CapabilitySupport::Exact,
        "in/out bound native decode",
        "trim filters preserve the authored source interval",
    ),
    entry(
        EditCapability::Speed,
        CapabilitySupport::Exact,
        CapabilitySupport::Exact,
        "source time and audio tempo use clip speed",
        "video timestamps and audio tempo use clip speed",
    ),
    entry(
        EditCapability::Volume,
        CapabilitySupport::Exact,
        CapabilitySupport::Exact,
        "the native mixer applies clip volume",
        "the audio graph applies clip volume",
    ),
    entry(
        EditCapability::Captions,
        CapabilitySupport::Limited,
        CapabilitySupport::Limited,
        "native playback exposes cue text and timing but not authored styling",
        "caption and subtitle timing compile with a supported styling subset",
    ),
    entry(
        EditCapability::Crop,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Exact,
        "crop is not represented in PlaybackTimeline",
        "clip crop compiles into the video graph",
    ),
    entry(
        EditCapability::Transform,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Limited,
        "transform is not represented in PlaybackTimeline",
        "2D position, scale, and rotation are supported; 3D and anchors are rejected",
    ),
    entry(
        EditCapability::Opacity,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Exact,
        "opacity is not represented in PlaybackTimeline",
        "clip opacity compiles into the video graph",
    ),
    entry(
        EditCapability::LayeredVideo,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Limited,
        "native playback selects one topmost active video clip",
        "multiple video tracks composite with normal/over blending only",
    ),
    entry(
        EditCapability::AudioMixing,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Exact,
        "the native worker decodes only the first active audible clip",
        "active audio and optional embedded audio are mixed",
    ),
    entry(
        EditCapability::Effects,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Limited,
        "effects are not represented in PlaybackTimeline",
        "the color effect is supported; other effects are rejected",
    ),
    entry(
        EditCapability::Keyframes,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Limited,
        "keyframes are not represented in PlaybackTimeline",
        "linear 2D transform and opacity keyframes are supported",
    ),
    entry(
        EditCapability::Transitions,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Limited,
        "transitions are not represented in PlaybackTimeline",
        "fade/crossfade/dissolve are supported for constrained overlapping clips",
    ),
    entry(
        EditCapability::Overlays,
        CapabilitySupport::Limited,
        CapabilitySupport::Limited,
        "caption cues and subtitle paths are projected; generic overlays are ignored",
        "caption and subtitle overlays compile; generic overlays are rejected",
    ),
    entry(
        EditCapability::Titles,
        CapabilitySupport::Limited,
        CapabilitySupport::Limited,
        "title clips become unstyled caption cues",
        "text titles compile with a supported 2D styling subset",
    ),
];

const fn entry(
    capability: EditCapability,
    live: CapabilitySupport,
    offline: CapabilitySupport,
    live_note: &'static str,
    offline_note: &'static str,
) -> CapabilityEntry {
    CapabilityEntry {
        capability,
        live,
        offline,
        live_note,
        offline_note,
    }
}

/// Return the declared support for one domain capability.
pub fn capability(capability: EditCapability) -> &'static CapabilityEntry {
    CAPABILITY_MATRIX
        .iter()
        .find(|entry| entry.capability == capability)
        .expect("every EditCapability has exactly one matrix entry")
}

#[cfg(test)]
mod tests {
    use super::{EditCapability, CAPABILITY_MATRIX};
    use std::collections::HashSet;

    #[test]
    fn matrix_covers_each_capability_once() {
        let capabilities = [
            EditCapability::Placement,
            EditCapability::SourceRange,
            EditCapability::Speed,
            EditCapability::Volume,
            EditCapability::Captions,
            EditCapability::Crop,
            EditCapability::Transform,
            EditCapability::Opacity,
            EditCapability::LayeredVideo,
            EditCapability::AudioMixing,
            EditCapability::Effects,
            EditCapability::Keyframes,
            EditCapability::Transitions,
            EditCapability::Overlays,
            EditCapability::Titles,
        ];
        let declared: HashSet<_> = CAPABILITY_MATRIX
            .iter()
            .map(|entry| entry.capability)
            .collect();
        assert_eq!(declared.len(), CAPABILITY_MATRIX.len());
        assert_eq!(declared, capabilities.into_iter().collect());
        assert!(CAPABILITY_MATRIX
            .iter()
            .all(|entry| { !entry.live_note.is_empty() && !entry.offline_note.is_empty() }));
    }
}
