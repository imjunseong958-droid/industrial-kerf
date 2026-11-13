//! A signal: a number over time that a recipe hands a model.
//!
//! A guide says where things are, and a prompt what they are. Neither says
//! when: the frame a ball lands on, how a light comes up, how much smoke
//! there is from one second to the next. A signal does, as a number from 0
//! to 1 for each frame, and the prompt says what it is of.
//!
//! It is a clip of the project, `gen=signal@1`, that draws nothing and
//! whose `value` is keyframed as any number is, so it is cut, moved,
//! proposed, and merged with the rest of its sequence:
//!
//! ```text
//! clip bounce gen=signal@1 at=0f dur=96f
//!   set value={0f:0 11f:0 12f:1 13f:0 35f:0 36f:1 37f:0}
//! ```
//!
//! A `generate` line names it, `signal=bounce`, and the request holds what
//! it comes to on each frame of its sequence, from the sequence's first:
//!
//! ```json
//! "signals": [{ "name": "bounce", "rate": 24.0, "values": [0.0, 0.0, 1.0] }]
//! ```
//!
//! Where the clip is not, before it and after it, and where it or its
//! track is turned off, the signal is 0. A model's program brings the values to the frames
//! the model makes; Kerf does not know how a model reads one.

use kerf_model::{Content, Project, Value, SIGNAL, SIGNAL_VALUE};
use sha2::{Digest, Sha256};

/// A signal as a model is handed it.
#[derive(Clone, Debug, PartialEq)]
pub struct Signal {
    /// The clip it is, by its id.
    pub name: String,
    /// The frames a second its values are at, which is its sequence's rate.
    pub rate: f64,
    /// What it is on each frame of its sequence, from the first: 0 to 1.
    pub values: Vec<f64>,
}

impl Signal {
    /// The signal clip `id` of `project` is. What is wrong is said as a
    /// `generate` line's `signal=` is read.
    pub fn read(project: &Project, id: &str) -> std::result::Result<Self, String> {
        let at = project
            .locate(id)
            .ok_or_else(|| format!("`signal={id}`: {}", project.no_clip(id)))?;
        let clip = project.clip_at(at);
        let params = match &clip.content {
            Content::Generator { fx, params, .. } if fx.as_str() == SIGNAL => params,
            _ => {
                return Err(format!(
                    "`signal={id}` names a clip that is not a signal: a signal is a `gen={SIGNAL}@1` clip, whose `{SIGNAL_VALUE}` is keyframed"
                ))
            }
        };
        let sequence = &project.sequences[at.sequence];
        let on = clip.enabled && sequence.tracks[at.track].enabled;
        let rate = sequence.rate;
        let value = params.get(SIGNAL_VALUE);
        let frames = rate.frames_round(sequence.duration()).max(0);
        let values = (0..frames)
            .map(|frame| {
                let time = rate.time_of(frame);
                if !on || time < clip.start || time >= clip.end() {
                    return 0.0;
                }
                let read = match value.map(|value| value.value_at(time - clip.start)) {
                    Some(Value::Number(number)) if number.is_finite() => number,
                    _ => 0.0,
                };
                // To four places, which is finer than a model follows and the same on any machine.
                (read.clamp(0.0, 1.0) * 1e4).round() / 1e4
            })
            .collect();
        Ok(Self {
            name: id.to_owned(),
            rate: rate.fps(),
            values,
        })
    }

    /// The value it holds on every frame, where it never changes: such a
    /// signal says nothing of when.
    pub fn steady(&self) -> Option<f64> {
        let first = *self.values.first()?;
        self.values
            .iter()
            .all(|value| *value == first)
            .then_some(first)
    }

    /// As a request holds it.
    pub(crate) fn asked(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "rate": self.rate,
            "values": self.values,
        })
    }

    /// A hash of what a model is handed of it: a take is held to that, and
    /// not to the rest of the sequence the clip is in.
    pub(crate) fn hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.asked().to_string());
        hex::encode(hasher.finalize())
    }
}
