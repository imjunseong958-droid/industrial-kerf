//! A request anchored to what the editor has selected. The editor sends the
//! clip, range, or playhead the person asked about; the agent receives the
//! request with that scope stated, in sequence frames the edit language
//! reads directly.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FocusScope {
    /// The selected clip; the clips around it follow by ripple.
    Clip,
    /// A span of the sequence.
    Range,
    /// The whole sequence, centred on the clip or playhead.
    Sequence,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Focus {
    pub scope: FocusScope,
    #[serde(default)]
    pub clip: Option<String>,
    /// The span, in frames of the sequence.
    pub from_frame: i64,
    pub to_frame: i64,
}

/// The request as the agent receives it: the scope, then the person's words.
pub fn focused_request(text: &str, focus: &Focus) -> String {
    let span = format!("{}f–{}f", focus.from_frame, focus.to_frame);
    let scope = match (focus.scope, &focus.clip) {
        (FocusScope::Clip, Some(clip)) => format!(
            "Focus: clip {clip} ({span}). Change only this clip; the clips around it \
             follow by ripple."
        ),
        (FocusScope::Sequence, Some(clip)) => format!(
            "Focus: the whole sequence, centred on clip {clip} ({span}). Other clips may \
             change to fit it."
        ),
        (FocusScope::Sequence, None) => {
            format!("Focus: the whole sequence, centred on {span}.")
        }
        _ => format!("Focus: the range {span} of the sequence. Keep edits inside it."),
    };
    format!("{scope}\n\n{text}")
}
