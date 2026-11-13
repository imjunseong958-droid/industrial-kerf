//! What Kerf asks of a model: the request a recipe's `generate` writes
//! (`docs/shots.md`, `kerf-engine/src/recipe/model.rs`).

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
    pub fn points(&self, frames: u32, fps: f64, (width, height): (u32, u32)) -> Option<Vec<[f64; 2]>> {
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
                    let [x, y, ..] = self.places[nearest].unwrap_or_default();
                    let (wide, tall) = self
                        .frame
                        .map_or((f64::from(width), f64::from(height)), |(wide, tall)| {
                            (f64::from(wide), f64::from(tall))
                        });
                    let fills = (f64::from(width) / wide).max(f64::from(height) / tall);
                    let pixel = |share: f64, own: f64, side: u32| {
                        (((share - 0.5) * own * fills + f64::from(side) / 2.0) * 10.0).round() / 10.0
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
    let tracks: Vec<Vec<serde_json::Value>> = moves
        .iter()
        .filter_map(|clip| clip.points(frames, fps, size))
        .map(|points| {
            points
                .into_iter()
                .map(|[x, y]| serde_json::json!({"x": x, "y": y}))
                .collect()
        })
        .collect();
    serde_json::Value::from(tracks).to_string()
}

/// The name a workflow reads where clips go under.
pub const MOVES: &str = "moves";

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
        let request: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
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
            other => {
                reference(other).and_then(|index| self.references.get(index).map(PathBuf::as_path))
            }
        }
    }

    /// The names of the files it gives, as [`Request::file`] takes them.
    pub fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = ["guide", "first", "last"]
            .into_iter()
            .filter(|name| self.file(name).is_some())
            .map(str::to_owned)
            .collect();
        names.extend((0..self.references.len()).map(|index| match index {
            0 => "ref".to_owned(),
            index => format!("ref{}", index + 1),
        }));
        names
    }
}

/// Which reference `name` is, counting from 0: `ref` is the first, `ref2`
/// the second.
fn reference(name: &str) -> Option<usize> {
    match name.strip_prefix("ref")? {
        "" => Some(0),
        number => number
            .parse::<usize>()
            .ok()
            .filter(|number| *number >= 2)
            .map(|number| number - 1),
    }
}

/// Whether `name` is one a request gives a file under.
pub fn names_a_file(name: &str) -> bool {
    matches!(name, "guide" | "first" | "last") || reference(name).is_some()
}

/// The name a request's signal has, counting from 0: `signal` is the
/// first, `signal2` the second, as its references are named.
pub fn signal_named(index: usize) -> String {
    match index {
        0 => "signal".to_owned(),
        index => format!("signal{}", index + 1),
    }
}

/// Whether `name` is one a request gives a signal under.
pub fn names_a_signal(name: &str) -> bool {
    match name.strip_prefix("signal") {
        Some("") => true,
        Some(number) => number.parse::<usize>().is_ok_and(|number| number >= 2),
        None => false,
    }
}

/// Whether `name` is one a request hands something under, a file or a
/// signal: a blank of that name is filled only by a request that has it.
pub fn names_what_is_handed(name: &str) -> bool {
    names_a_file(name) || names_a_signal(name) || name == MOVES
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request `docs/shots.md` shows.
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
    fn a_request_is_read_as_kerf_writes_it() {
        let request = Request::parse(SHOWN).unwrap();
        assert_eq!(
            request.prompt,
            "a fishing boat leaves the harbour at dawn, fog"
        );
        assert_eq!(
            (request.seconds, request.size, request.seed),
            (Some(4.0), None, Some(7))
        );
        assert_eq!(
            request.more.get("strength").map(String::as_str),
            Some("0.6")
        );
        assert_eq!(
            request.out,
            Path::new("/project/shots/open.takes/.making-003.mp4")
        );
        assert_eq!(request.files(), ["guide", "first", "ref", "ref2"]);
        assert_eq!(
            request.file("ref2"),
            Some(Path::new("/project/refs/fog.png"))
        );
        assert_eq!(request.file("last"), None);
        assert_eq!(request.file("ref3"), None);
        let sized =
            Request::parse(&SHOWN.replace("\"size\": null", "\"size\": [1280, 720]")).unwrap();
        assert_eq!(sized.size, Some((1280, 720)));
    }

    #[test]
    fn a_signal_is_brought_to_the_frames_the_model_makes() {
        let request = Request::parse(SHOWN).unwrap();
        let swell = &request.signals[0];
        assert_eq!(swell.name, "swell");
        // At its own rate it is what it was, and past its end what it ended on.
        assert_eq!(
            swell.over(9, 24.0),
            [0.0, 0.0, 1.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.5]
        );
        // At twice the rate, each frame between two of its own is half way.
        assert_eq!(swell.over(6, 48.0), [0.0, 0.0, 0.0, 0.5, 1.0, 0.5]);
        // At a slower one its peak, one of its own frames long, is still there: lower, and
        // spread over the frames of the model's it falls in.
        let slower = swell.over(5, 16.0);
        assert_eq!(slower[0], 0.0);
        assert!(slower[1] > 0.4 && slower[1] < 1.0, "{slower:?}");
        assert!(slower[2] > 0.0 && slower[2] < slower[1], "{slower:?}");
        assert_eq!(slower[4], 0.5);
        assert_eq!(swell.text(3, 24.0), "0,0,1");
        // Held for two of the model's frames, three of its own here, a peak reaches one of
        // them whole, wherever it starts.
        for start in 4..8 {
            let mut values = vec![0.0; 20];
            values[start..=start + 3].fill(1.0);
            let held = Signal {
                name: "held".to_owned(),
                rate: 24.0,
                values,
            };
            assert!(held.over(13, 16.0).contains(&1.0), "from frame {start}");
        }
        // A request from before signals has none.
        let before = Request::parse(
            &SHOWN
                .lines()
                .filter(|line| !line.contains("signals"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert!(before.signals.is_empty());
        for name in ["signal", "signal2", "signal12"] {
            assert!(names_a_signal(name) && names_what_is_handed(name), "{name}");
        }
        for name in ["signal1", "signal0", "signals", "sign"] {
            assert!(!names_a_signal(name), "{name}");
        }
        assert_eq!([signal_named(0), signal_named(1)], ["signal", "signal2"]);
    }

    #[test]
    fn where_a_clip_goes_is_a_track_of_points_in_the_models_pixels() {
        // Four frames at 8 a second, the clip gone on the last, for a model that makes
        // eight at 16 on a picture 200 by 100.
        let ball: Moves = serde_json::from_str(
            r#"{"name": "ball", "rate": 8.0, "places": [[0.25, 0.5, 0.1, 0.1], [0.5, 0.5, 0.1, 0.1], [0.75, 0.4, 0.1, 0.1], null]}"#,
        )
        .unwrap();
        let points = ball.points(8, 16.0, (200, 100)).unwrap();
        assert_eq!(points.len(), 8);
        assert_eq!(points[0], [50.0, 50.0]);
        assert_eq!(points[2], [100.0, 50.0]);
        // Where it is gone it stays where it was last.
        assert_eq!(points[7], [150.0, 40.0]);
        let text = tracks(std::slice::from_ref(&ball), 2, 8.0, (200, 100));
        assert_eq!(text, r#"[[{"x":50.0,"y":50.0},{"x":100.0,"y":50.0}]]"#);
        // From a sequence wider than the picture, the sides are cut off: a quarter across
        // a frame twice as wide as it is tall is the left edge of a square.
        let wide = Moves { frame: Some((400, 200)), ..ball.clone() };
        assert_eq!(wide.points(1, 8.0, (100, 100)).unwrap()[0], [0.0, 50.0]);
        // One that shows on no frame is no track.
        let none = Moves { places: vec![None; 3], ..ball };
        assert_eq!(tracks(&[none], 2, 8.0, (200, 100)), "[]");
        assert!(names_what_is_handed(MOVES));
    }

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
