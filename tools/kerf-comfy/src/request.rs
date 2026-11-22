//! What Kerf asks of a model: the request a `generate` step writes
//! (`kerf-engine/src/recipe/model.rs`).

// The names a request hands things under are the workflow's to know too, and are kept with it.
pub use kerf_workflow::names::{
    guide_named, names_a_file, names_a_signal, names_points, names_what_is_handed, reference,
    signal_named, signal_video_named, INSIDE, MOVES, MOVES_BOX, MOVES_VIDEO, OUTSIDE,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The version of request this program reads.
const REQUEST_VERSION: u32 = 1;

/// A request as its file says it. Every file in it is where it is on this
/// machine.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Request {
    #[serde(rename = "kerf-request")]
    version: u32,
    /// What the file should show, in words.
    #[serde(default)]
    pub prompt: String,
    /// A picture in motion to follow.
    #[serde(default)]
    pub guide: Option<PathBuf>,
    /// More pictures in motion to follow, each under what it is of: `depth`, `ids`.
    #[serde(default)]
    pub guides: BTreeMap<String, PathBuf>,
    /// The first and the last frame.
    #[serde(default)]
    pub first: Option<PathBuf>,
    #[serde(default)]
    pub last: Option<PathBuf>,
    /// Pictures of who and what is in it, and of how it should look.
    #[serde(default)]
    pub references: Vec<PathBuf>,
    #[serde(default)]
    pub seconds: Option<f64>,
    /// Where the shot starts in its sequences' time, in seconds: a part of a
    /// longer shot. Its signals and where its clips go are read from there.
    #[serde(default)]
    pub from: Option<f64>,
    #[serde(default)]
    pub size: Option<(u32, u32)>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Numbers over time that say when what the prompt asks for happens.
    #[serde(default)]
    pub signals: Vec<Signal>,
    /// Where clips of the guide go, frame by frame.
    #[serde(default)]
    pub moves: Vec<Moves>,
    /// A workflow of the project to run in place of the one this program was
    /// started with: the step's `workflow=`, a file a person keeps beside
    /// the shot's takes.
    #[serde(default)]
    pub workflow: Option<PathBuf>,
    /// Anything else the `generate` line said, as it was written.
    #[serde(default)]
    pub more: BTreeMap<String, String>,
    /// The file to write.
    pub out: PathBuf,
}

/// Where a clip goes, as a request holds it: for each frame of its sequence the middle of
/// its box across and down the frame and the box's width and height, from 0 to 1, or
/// nothing on a frame it does not show on.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Moves {
    /// The clip it is, by its id.
    pub name: String,
    /// The frames a second its places are at.
    pub rate: f64,
    /// How wide and tall its sequence is, which its places are shares of: none is the
    /// shape of whatever is made.
    #[serde(default)]
    pub frame: Option<(u32, u32)>,
    pub places: Vec<Option<[f64; 4]>>,
}

impl Moves {
    /// The middle of its box on each of `frames` frames at `fps` a second, in pixels of a
    /// picture `width` by `height`: on a frame it does not show on, where it is on the
    /// nearest frame it does. A sequence of another shape fills the picture as its guide
    /// does, what is over cut off evenly. None where it shows on no frame.
    pub fn points(&self, frames: u32, fps: f64, size: (u32, u32)) -> Option<Vec<[f64; 2]>> {
        self.points_at([0.0, 0.0], frames, fps, size)
    }

    /// A point of its box on each frame, as [`Moves::points`] gives its middle: `at` is how
    /// far from the middle toward the right and bottom edges, 1 being on them.
    pub fn points_at(
        &self,
        at: [f64; 2],
        frames: u32,
        fps: f64,
        (width, height): (u32, u32),
    ) -> Option<Vec<[f64; 2]>> {
        let shown: Vec<usize> = (0..self.places.len())
            .filter(|frame| self.places[*frame].is_some())
            .collect();
        if shown.is_empty() {
            return None;
        }
        let last = self.places.len() - 1;
        Some(
            (0..frames)
                .map(|frame| {
                    let own = ((f64::from(frame) * self.rate / fps).round() as usize).min(last);
                    let nearest = shown
                        .iter()
                        .min_by_key(|frame| frame.abs_diff(own))
                        .copied()
                        .unwrap_or(own);
                    let [x, y, w, h] = self.places[nearest].unwrap_or_default();
                    let (x, y) = (x + at[0] * w / 2.0, y + at[1] * h / 2.0);
                    let (wide, tall) = self
                        .frame
                        .map_or((f64::from(width), f64::from(height)), |(wide, tall)| {
                            (f64::from(wide), f64::from(tall))
                        });
                    let fills = (f64::from(width) / wide).max(f64::from(height) / tall);
                    let pixel = |share: f64, own: f64, side: u32| {
                        (((share - 0.5) * own * fills + f64::from(side) / 2.0) * 10.0).round()
                            / 10.0
                    };
                    [pixel(x, wide, width), pixel(y, tall, height)]
                })
                .collect(),
        )
    }
}

/// What a workflow's `{moves}` blank is filled with: each clip's points as a track, a list
/// of `{"x": …, "y": …}` a frame, the tracks in a list, as text a node reads coordinates
/// from.
pub fn tracks(moves: &[Moves], frames: u32, fps: f64, size: (u32, u32)) -> String {
    tracks_at(&[[0.0, 0.0]], moves, frames, fps, size)
}

/// What a workflow's `{inside}` or `{outside}` blank is filled with: the points a line gives
/// under `name` (`inside="0.3,0.6 0.3,0.7"`, each across and down the frame from 0 to 1) as a
/// list of `{"x": …, "y": …}` in whole pixels of a picture `width` by `height`, as text a
/// node reads coordinates from.
pub fn points(name: &str, given: &str, (width, height): (u32, u32)) -> Result<String, String> {
    let wrong = || {
        format!(
            "`{name}={given}` is not points: each is across and down the frame from 0 to 1, a comma between the two and a space between points, as `{name}=\"0.3,0.6 0.3,0.7\"`"
        )
    };
    let share = |text: &str| {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|share| (0.0..=1.0).contains(share))
    };
    let points = given
        .split_whitespace()
        .map(|point| {
            let (across, down) = point.split_once(',')?;
            let pixel = |share: f64, side: u32| (share * f64::from(side)).round().min(f64::from(side) - 1.0) as u32;
            Some(serde_json::json!({"x": pixel(share(across)?, width), "y": pixel(share(down)?, height)}))
        })
        .collect::<Option<Vec<_>>>()
        .filter(|points| !points.is_empty())
        .ok_or_else(wrong)?;
    Ok(serde_json::Value::from(points).to_string())
}

/// Where in a clip's box the tracks of a `{moves_box}` are: its middle, and four points
/// toward its corners, far enough in to be on what the box holds.
const IN_A_BOX: [[f64; 2]; 5] = [
    [0.0, 0.0],
    [-0.7, -0.7],
    [0.7, -0.7],
    [-0.7, 0.7],
    [0.7, 0.7],
];

/// What a workflow's `{moves_box}` blank is filled with: tracks as `{moves}` has them, five
/// a clip, its middle and four points toward its corners, so that how large the clip is on
/// each frame is said with where it is.
pub fn tracks_of_boxes(moves: &[Moves], frames: u32, fps: f64, size: (u32, u32)) -> String {
    tracks_at(&IN_A_BOX, moves, frames, fps, size)
}

fn tracks_at(at: &[[f64; 2]], moves: &[Moves], frames: u32, fps: f64, size: (u32, u32)) -> String {
    let tracks: Vec<Vec<serde_json::Value>> = moves
        .iter()
        .flat_map(|clip| {
            at.iter()
                .filter_map(move |at| clip.points_at(*at, frames, fps, size))
        })
        .map(|points| {
            points
                .into_iter()
                .map(|[x, y]| serde_json::json!({"x": x, "y": y}))
                .collect()
        })
        .collect();
    serde_json::Value::from(tracks).to_string()
}

impl Moves {
    /// Its box on each of `frames` frames at `fps` a second, in pixels of a picture `width`
    /// by `height`, left, top, right, bottom: none on a frame it does not show on. A
    /// sequence of another shape fills the picture as its guide does.
    pub fn boxes(
        &self,
        frames: u32,
        fps: f64,
        (width, height): (u32, u32),
    ) -> Vec<Option<[f64; 4]>> {
        let Some(last) = self.places.len().checked_sub(1) else {
            return vec![None; frames as usize];
        };
        let (wide, tall) = self
            .frame
            .map_or((f64::from(width), f64::from(height)), |(wide, tall)| {
                (f64::from(wide), f64::from(tall))
            });
        let fills = (f64::from(width) / wide).max(f64::from(height) / tall);
        (0..frames)
            .map(|frame| {
                let own = ((f64::from(frame) * self.rate / fps).round() as usize).min(last);
                let [x, y, w, h] = self.places[own]?;
                let across = |share: f64| (share - 0.5) * wide * fills + f64::from(width) / 2.0;
                let down = |share: f64| (share - 0.5) * tall * fills + f64::from(height) / 2.0;
                Some([
                    across(x - w / 2.0),
                    down(y - h / 2.0),
                    across(x + w / 2.0),
                    down(y + h / 2.0),
                ])
            })
            .collect()
    }
}

/// The frames of a `{moves_video}`: each `width` by `height` bytes of grey, white with every
/// clip's box on that frame outlined in black.
pub fn layout(moves: &[Moves], frames: u32, fps: f64, (width, height): (u32, u32)) -> Vec<Vec<u8>> {
    let boxes: Vec<Vec<Option<[f64; 4]>>> = moves
        .iter()
        .map(|clip| clip.boxes(frames, fps, (width, height)))
        .collect();
    // As thick as a model's own annotator draws a box: a hundredth of the frame's height.
    let line = (i64::from(height) / 100).max(2);
    let (wide, tall) = (i64::from(width), i64::from(height));
    (0..frames as usize)
        .map(|frame| {
            let mut picture = vec![255u8; (width * height) as usize];
            for [left, top, right, bottom] in boxes.iter().filter_map(|clip| clip[frame]) {
                let (left, top) = (left.round() as i64, top.round() as i64);
                let (right, bottom) = (right.round() as i64, bottom.round() as i64);
                for y in top.max(0)..bottom.min(tall) {
                    for x in left.max(0)..right.min(wide) {
                        let edge = x < left + line
                            || x >= right - line
                            || y < top + line
                            || y >= bottom - line;
                        if edge {
                            picture[(y * wide + x) as usize] = 0;
                        }
                    }
                }
            }
            picture
        })
        .collect()
}

/// A signal as a request holds it: a number from 0 to 1 for each frame of
/// the sequence its clip is in.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Signal {
    /// The clip it is, by its id.
    pub name: String,
    /// The frames a second its values are at.
    pub rate: f64,
    pub values: Vec<f64>,
}

impl Signal {
    /// What it is at `at`, counted in its own frames: on a line between
    /// the two around it, and past either end what it is there.
    fn at(&self, at: f64) -> f64 {
        let Some(last) = self.values.len().checked_sub(1) else {
            return 0.0;
        };
        let at = at.clamp(0.0, last as f64);
        let before = at.floor() as usize;
        let after = (before + 1).min(last);
        let along = at - before as f64;
        self.values[before] * (1.0 - along) + self.values[after] * along
    }

    /// What it is on each of `frames` frames at `fps` a second, the first
    /// at its own first. Where the model makes fewer frames than it has,
    /// each is the mean of it over that frame's time, so a peak one of its
    /// own frames long is still there, lower and as long as one of the
    /// model's.
    pub fn over(&self, frames: u32, fps: f64) -> Vec<f64> {
        /// How many places a longer frame is read at.
        const READS: u32 = 8;
        let span = self.rate / fps;
        (0..frames)
            .map(|frame| {
                let middle = f64::from(frame) * span;
                let value = if span > 1.0 {
                    (0..READS)
                        .map(|read| {
                            let across = (f64::from(read) + 0.5) / f64::from(READS) - 0.5;
                            self.at(middle + across * span)
                        })
                        .sum::<f64>()
                        / f64::from(READS)
                } else {
                    self.at(middle)
                };
                (value.clamp(0.0, 1.0) * 1e4).round() / 1e4
            })
            .collect()
    }

    /// [`Signal::over`] as a node takes a list of numbers: `0,0.25,0.5`.
    pub fn text(&self, frames: u32, fps: f64) -> String {
        self.over(frames, fps)
            .iter()
            .map(f64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

impl Request {
    /// Read the request `file` holds.
    pub fn read(file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file)
            .map_err(|error| format!("cannot read the request {}: {error}", file.display()))?;
        Self::parse(&text).map_err(|error| format!("the request {}: {error}", file.display()))
    }

    /// Read a request's text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut request: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
        // A part of a longer shot: what its sequences say before it starts is not its own.
        if let Some(from) = request.from.filter(|from| from.is_finite() && *from > 0.0) {
            let skipped =
                |rate: f64, frames: usize| ((from * rate).round().max(0.0) as usize).min(frames);
            for signal in &mut request.signals {
                // One past its end is what it ended on, so its last value is kept.
                let skip = skipped(signal.rate, signal.values.len().saturating_sub(1));
                signal.values.drain(..skip);
            }
            for moves in &mut request.moves {
                let skip = skipped(moves.rate, moves.places.len().saturating_sub(1));
                moves.places.drain(..skip);
            }
        }
        if request.version != REQUEST_VERSION {
            return Err(format!(
                "it is `kerf-request` {}, and this program reads {REQUEST_VERSION}",
                request.version
            ));
        }
        Ok(request)
    }

    /// The file the request names as `name`: `guide`, `first`, `last`, `ref`
    /// for the first reference, or `ref2` and on for the ones after it.
    pub fn file(&self, name: &str) -> Option<&Path> {
        match name {
            "guide" => self.guide.as_deref(),
            "first" => self.first.as_deref(),
            "last" => self.last.as_deref(),
            other => match guide_named(other) {
                Some(of) => self.guides.get(of).map(PathBuf::as_path),
                None => reference(other)
                    .and_then(|index| self.references.get(index).map(PathBuf::as_path)),
            },
        }
    }

    /// The names of the files it gives, as [`Request::file`] takes them.
    pub fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = ["guide", "first", "last"]
            .into_iter()
            .filter(|name| self.file(name).is_some())
            .map(str::to_owned)
            .collect();
        names.extend(self.guides.keys().map(|of| format!("guide_{of}")));
        names.extend((0..self.references.len()).map(|index| match index {
            0 => "ref".to_owned(),
            index => format!("ref{}", index + 1),
        }));
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request as a `generate` step writes it.
    const SHOWN: &str = r#"{
      "kerf-request": 1,
      "model": "dawn",
      "prompt": "a fishing boat leaves the harbour at dawn, fog",
      "guide": "/project/shots/open.takes/003.work/guide.mp4",
      "first": "/project/refs/boat.png",
      "last": null,
      "references": ["/project/refs/sea.png", "/project/refs/fog.png"],
      "seconds": 4.0,
      "size": null,
      "seed": 7,
      "signals": [{ "name": "swell", "rate": 24.0, "values": [0.0, 0.0, 1.0, 0.0, 0.5, 0.5, 0.5] }],
      "more": { "strength": "0.6" },
      "out": "/project/shots/open.takes/.making-003.mp4"
    }"#;

    #[test]
    fn a_request_of_another_version_is_refused() {
        let newer = Request::parse(&SHOWN.replace("\"kerf-request\": 1", "\"kerf-request\": 2"))
            .unwrap_err();
        assert!(newer.contains("this program reads 1"), "{newer}");
        assert!(Request::parse("{}").is_err());
    }

    #[test]
    fn only_a_requests_own_names_are_files() {
        for name in ["guide", "first", "last", "ref", "ref2", "ref12"] {
            assert!(names_a_file(name), "{name}");
        }
        for name in ["prompt", "seed", "ref1", "ref0", "refs", "reference"] {
            assert!(!names_a_file(name), "{name}");
        }
    }
}
