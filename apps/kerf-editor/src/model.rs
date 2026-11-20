//! What the bridge says, as typed data: the timeline view an editor snapshot
//! carries, conversations kept beside the project, proposals, render jobs.
//! Read leniently: a field the bridge leaves out is at its default, so a
//! newer kerfd never breaks the window.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// The timeline view of a snapshot (kerf-runtime's `timeline_json`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Timeline {
    pub name: String,
    pub sequence: Option<String>,
    pub fps: f64,
    pub rate: String,
    pub drop_frame: bool,
    pub width: u32,
    pub height: u32,
    pub start: f64,
    pub duration: f64,
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,
    pub sources: Vec<Source>,
    pub sequences: Vec<SequenceInfo>,
    /// Each media's readable name, by id ([`crate::names::source_names`]),
    /// as [`Timeline::name_clips`] last made them.
    #[serde(skip)]
    pub media_names: HashMap<String, String>,
}

impl Timeline {
    /// Reads a snapshot's timeline, its media named and each clip of media
    /// that has no name of its own given its media's ([`Timeline::name_clips`]).
    pub fn from_value(value: &Value) -> Self {
        let mut timeline: Self = serde_json::from_value(value.clone()).unwrap_or_default();
        timeline.name_clips();
        timeline
    }

    /// Names the media ([`crate::names::source_names`]) and gives each clip
    /// of media with a file its media's name to go by when it has none of
    /// its own (Qt's `withClipNames`): [`Clip::label`] then says it on the
    /// timeline, in a proposal's change rows and wherever else a clip is
    /// named. The clip's own `name` is left as the document has it.
    pub fn name_clips(&mut self) {
        self.media_names = crate::names::source_names(&self.sources);
        let named: HashMap<&str, &str> = self
            .sources
            .iter()
            .filter(|source| !source.path.is_empty())
            .filter_map(|source| {
                self.media_names
                    .get(&source.id)
                    .map(|name| (source.id.as_str(), name.as_str()))
            })
            .collect();
        for track in &mut self.tracks {
            for clip in &mut track.clips {
                clip.media_name = clip
                    .source_id
                    .as_deref()
                    .and_then(|source| named.get(source))
                    .map(|name| (*name).to_owned());
            }
        }
    }

    /// The readable name of media `id` ([`crate::names::source_names`]);
    /// the id itself for media the timeline does not have.
    pub fn media_name(&self, id: &str) -> String {
        if let Some(name) = self.media_names.get(id) {
            return name.clone();
        }
        crate::names::source_names(&self.sources)
            .remove(id)
            .unwrap_or_else(|| id.to_owned())
    }

    /// A track's badge as its head and the Media band say it: "V1", "A2",
    /// counting picture tracks (captions among them) and sound tracks apart,
    /// in the timeline's order. The id itself for a track it does not have.
    pub fn track_badge(&self, id: &str) -> String {
        let (mut picture, mut sound) = (0, 0);
        for track in &self.tracks {
            let audio = track.kind == TrackKind::Audio;
            let number = if audio {
                sound += 1;
                sound
            } else {
                picture += 1;
                picture
            };
            if track.id == id {
                return format!("{}{number}", if audio { 'A' } else { 'V' });
            }
        }
        id.to_owned()
    }

    /// The badges of the tracks each media's clips are on, by media id, in
    /// the timeline's order ("V1", "A1"): where it is used.
    pub fn used_on(&self) -> HashMap<String, Vec<String>> {
        let mut used: HashMap<String, Vec<String>> = HashMap::new();
        for track in &self.tracks {
            let badge = self.track_badge(&track.id);
            for clip in &track.clips {
                let Some(source) = clip.source_id.as_deref().filter(|id| !id.is_empty()) else {
                    continue;
                };
                let on = used.entry(source.to_owned()).or_default();
                if !on.contains(&badge) {
                    on.push(badge.clone());
                }
            }
        }
        used
    }

    /// Frames a second, never zero.
    pub fn rate(&self) -> f64 {
        if self.fps.is_finite() && self.fps > 0.0 {
            self.fps
        } else {
            30.0
        }
    }

    pub fn clip(&self, id: &str) -> Option<(&Track, &Clip)> {
        self.tracks.iter().find_map(|track| {
            track
                .clips
                .iter()
                .find(|clip| clip.id == id)
                .map(|clip| (track, clip))
        })
    }

    pub fn source(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|source| source.id == id)
    }

    pub fn has_clips(&self) -> bool {
        self.tracks.iter().any(|track| !track.clips.is_empty())
    }

    /// Width over height of the picture; 16:9 when the sequence says none.
    pub fn aspect(&self) -> f32 {
        if self.width > 0 && self.height > 0 {
            self.width as f32 / self.height as f32
        } else {
            16.0 / 9.0
        }
    }

    /// The whole frame nearest `seconds`.
    pub fn frame_at(&self, seconds: f64) -> i64 {
        (seconds * self.rate()).round() as i64
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    #[default]
    Video,
    Audio,
    Subtitle,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: TrackKind,
    pub name: Option<String>,
    pub enabled: bool,
    pub locked: bool,
    pub muted: bool,
    /// Its role, or its kind's default when none is assigned.
    pub role: Option<String>,
    /// Whether the role is the track's own, not its kind's default.
    pub role_assigned: bool,
    pub sync_locked: bool,
    pub clips: Vec<Clip>,
    pub transitions: Vec<Transition>,
}

impl Track {
    /// What the track is called: its name, else its id.
    pub fn label(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.id)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub name: Option<String>,
    pub timeline_start: f64,
    pub duration: f64,
    pub start_frame: i64,
    pub duration_frames: i64,
    pub enabled: bool,
    pub kind: String,
    pub source_id: Option<String>,
    pub sequence_id: Option<String>,
    #[serde(rename = "in")]
    pub source_in: Option<f64>,
    #[serde(rename = "out")]
    pub source_out: Option<f64>,
    pub speed: Option<f64>,
    /// It plays its source backwards (the speed's size is in `speed`).
    pub reverse: bool,
    pub link: Option<String>,
    pub fx: Option<String>,
    pub text: Option<String>,
    pub effects: Vec<Value>,
    /// Its properties as the snapshot carries them (`gain` in dB, `opacity`…).
    pub props: Value,
    /// Its media's readable name, for a clip of media with a file: what it
    /// goes by when it has no name of its own ([`Timeline::name_clips`]).
    #[serde(skip)]
    pub media_name: Option<String>,
}

impl Clip {
    pub fn end(&self) -> f64 {
        self.timeline_start + self.duration
    }

    pub fn end_frame(&self) -> i64 {
        self.start_frame + self.duration_frames
    }

    /// What the clip is called: its name, its media's name, its text,
    /// else its id.
    pub fn label(&self) -> &str {
        [
            self.name.as_deref(),
            self.media_name.as_deref(),
            self.text.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|name| !name.is_empty())
        .unwrap_or(&self.id)
    }

    /// Its gain in dB (`props.gain`), 0 when it has none.
    pub fn gain_db(&self) -> f64 {
        self.props["gain"].as_f64().unwrap_or(0.0)
    }

    /// The second of its source shown `offset` seconds into the clip: from
    /// its in point towards its out (which allow for its speed), or back
    /// from its out point when it plays in reverse. None for a clip with no
    /// source range (a title, a generator).
    pub fn source_at(&self, offset: f64) -> Option<f64> {
        let source_in = self.source_in?;
        let speed = self.speed.unwrap_or(1.0);
        let out = self
            .source_out
            .unwrap_or(source_in + self.duration * speed.abs());
        let rate = if self.duration > 0.0 {
            (out - source_in) / self.duration
        } else {
            speed.abs()
        };
        Some(if self.reverse || speed < 0.0 {
            out - offset * rate
        } else {
            source_in + offset * rate
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Transition {
    pub id: String,
    pub from: String,
    pub to: String,
    pub fx: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Marker {
    pub id: String,
    pub time: f64,
    pub frame: i64,
    pub duration: f64,
    pub label: Option<String>,
    pub color: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub path: String,
    pub duration: f64,
    pub fps: f64,
    pub has_audio: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub recipe: Option<String>,
    pub steps: Option<u32>,
    pub takes: Vec<Take>,
    /// The project's cast and styles its steps name.
    pub shows: Vec<String>,
}

impl Source {
    /// The file's name without its folder.
    pub fn file_name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }

    /// Made from steps: in the document, or in the recipe file it names.
    pub fn is_made(&self) -> bool {
        self.steps.is_some_and(|steps| steps > 0) || self.recipe.is_some()
    }

    /// Made by steps written under its line in the document.
    pub fn steps_in_document(&self) -> bool {
        self.recipe.as_deref().is_none_or(str::is_empty)
            && self.steps.is_some_and(|steps| steps > 0)
    }

    /// Its steps are still in the recipe file it names: carried into the
    /// document before it is made.
    pub fn uncarried(&self) -> bool {
        self.recipe
            .as_deref()
            .is_some_and(|recipe| !recipe.is_empty())
            && !self.steps.is_some_and(|steps| steps > 0)
    }

    /// It has a picture (a video or a still), not only sound.
    pub fn is_picture(&self) -> bool {
        self.width.is_some()
    }

    /// The take it is now, by number, when it has takes.
    pub fn chosen_take(&self) -> Option<u32> {
        self.takes
            .iter()
            .find(|take| take.chosen)
            .map(|take| take.number)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Take {
    pub number: u32,
    pub path: String,
    pub chosen: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SequenceInfo {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
}

/// One proposal the runtime keeps (newest first in a snapshot).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Proposal {
    pub proposal_id: String,
    pub status: String,
    pub freshness: String,
    pub base_revision: u64,
    pub script: String,
    pub document_diff: String,
    pub version: u32,
    pub staged: Option<String>,
    pub author: Value,
}

impl Proposal {
    pub fn is_pending(&self) -> bool {
        self.status == "pending"
    }

    pub fn is_stale(&self) -> bool {
        self.freshness == "stale"
    }

    /// Lines the diff adds and removes, without its headers.
    pub fn diff_counts(&self) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;
        for line in self.document_diff.lines() {
            if line.starts_with("+++") || line.starts_with("---") {
                continue;
            }
            if line.starts_with('+') {
                added += 1;
            } else if line.starts_with('-') {
                removed += 1;
            }
        }
        (added, removed)
    }

    /// What it does, in a line: the script's first line that says
    /// something (not blank, not a comment, not `use <sequence>`).
    pub fn summary(&self) -> String {
        self.script
            .lines()
            .map(str::trim)
            .find(|line| {
                !line.is_empty()
                    && !line.starts_with('#')
                    && !line.starts_with("//")
                    && !line.starts_with("use ")
            })
            .map(str::to_owned)
            .unwrap_or_else(|| "Proposal".to_owned())
    }

    /// Who staged it, as the runtime names them.
    pub fn author_name(&self) -> String {
        match &self.author {
            Value::String(name) => name.clone(),
            Value::Object(map) => map
                .get("name")
                .or_else(|| map.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or("Kerf")
                .to_owned(),
            _ => "Kerf".to_owned(),
        }
    }
}

/// An editor snapshot: the `state` event's `snapshot`.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub document: String,
    pub timeline: Timeline,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub duration: f64,
    pub proposals: Vec<Proposal>,
    /// The proposal's timeline, when one is pending and the bridge drew it.
    pub proposal_timeline: Option<Timeline>,
    pub import_warnings: Vec<String>,
    /// `current`, or `diverged` when project.kerf changed on disk since the
    /// editor last wrote it: kerfd then refuses edits until one is chosen.
    pub projection_status: String,
}

impl Snapshot {
    pub fn from_value(value: &Value) -> Self {
        let proposals = value["proposals"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| serde_json::from_value(item.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        let proposal_timeline = ["proposalTimeline", "pendingTimeline"]
            .iter()
            .map(|key| &value[*key])
            .find(|timeline| timeline.is_object())
            .map(Timeline::from_value);
        Self {
            document: value["document"].as_str().unwrap_or_default().to_owned(),
            timeline: Timeline::from_value(&value["timeline"]),
            revision: value["revision"].as_u64().unwrap_or(0),
            can_undo: value["canUndo"].as_bool().unwrap_or(false),
            can_redo: value["canRedo"].as_bool().unwrap_or(false),
            duration: value["duration"].as_f64().unwrap_or(0.0),
            proposals,
            proposal_timeline,
            projection_status: value["projectionStatus"]
                .as_str()
                .unwrap_or("current")
                .to_owned(),
            import_warnings: value["importWarnings"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The newest open proposal.
    pub fn pending(&self) -> Option<&Proposal> {
        self.proposals.iter().find(|proposal| proposal.is_pending())
    }
}

/// A conversation as the sidebar lists it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub created: String,
    pub updated: String,
    pub archived: bool,
    pub count: u32,
}

impl ConversationSummary {
    pub fn label(&self) -> &str {
        if self.title.trim().is_empty() {
            "New conversation"
        } else {
            &self.title
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speaker {
    #[default]
    You,
    Kerf,
}

/// One thing said in a conversation.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Turn {
    pub id: String,
    pub kind: Speaker,
    pub text: String,
    pub at: String,
    /// Where the playhead was when it was said, in seconds.
    pub timecode: Option<f64>,
    pub steps: Vec<String>,
    pub seconds: Option<f64>,
    /// The turn failed; its text says why.
    pub error: bool,
    pub stopped: bool,
    pub proposal: Option<String>,
    /// The makings Kerf asked the person to run.
    pub asks: Vec<Ask>,
}

/// A making Kerf asked for and a person has to say yes to: the media, how
/// many takes (one unless said), the first of them, and the plan as asked.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Ask {
    pub media: String,
    pub rolls: Option<u32>,
    pub take: Option<u32>,
    pub text: String,
}

/// A conversation kept beside the project, with all it holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created: String,
    pub updated: String,
    pub archived: bool,
    pub items: Vec<Turn>,
}

impl Conversation {
    /// Reads a `conversation` event. A turn that does not read is left out
    /// rather than losing the conversation; None without an id.
    pub fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
        let id = text("id");
        if id.is_empty() {
            return None;
        }
        Some(Self {
            id,
            title: text("title"),
            created: text("created"),
            updated: text("updated"),
            archived: value["archived"].as_bool().unwrap_or(false),
            items: value["items"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| serde_json::from_value(item.clone()).ok())
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// A render job as the queue lists it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RenderJob {
    pub job_id: String,
    pub name: String,
    pub quality: String,
    pub codec: String,
    pub status: String,
    pub progress: f64,
    pub output_path: String,
    pub error: Option<String>,
}

impl RenderJob {
    pub fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "running")
    }
}

/// The span of the program a proposal changes, padded by `pad` seconds
/// either side and held to `most` seconds: where `proposed` has a clip that
/// `current` lacks, lacks one it has, or has one placed or cut otherwise.
/// None when nothing differs.
pub fn changed_span(
    current: &Timeline,
    proposed: &Timeline,
    pad: f64,
    most: f64,
) -> Option<(f64, f64)> {
    fn clips(timeline: &Timeline) -> Vec<(&str, &Clip)> {
        timeline
            .tracks
            .iter()
            .flat_map(|track| {
                track
                    .clips
                    .iter()
                    .map(move |clip| (track.id.as_str(), clip))
            })
            .collect()
    }
    let before = clips(current);
    let after = clips(proposed);
    let same = |(track, clip): &(&str, &Clip), others: &[(&str, &Clip)]| {
        others.iter().any(|(other_track, other)| {
            other.id == clip.id
                && other_track == track
                && other.start_frame == clip.start_frame
                && other.duration_frames == clip.duration_frames
                && other.source_in == clip.source_in
                && other.enabled == clip.enabled
        })
    };
    let mut span: Option<(f64, f64)> = None;
    let changed = after
        .iter()
        .filter(|entry| !same(entry, &before))
        .chain(before.iter().filter(|entry| !same(entry, &after)));
    for (_, clip) in changed {
        let (start, end) = (clip.timeline_start, clip.end());
        span = Some(match span {
            Some((low, high)) => (low.min(start), high.max(end)),
            None => (start, end),
        });
    }
    let (start, end) = span?;
    let length = proposed.duration.max(current.duration).max(end);
    let start = (start - pad).max(0.0);
    let end = (end + pad).min(length.max(start));
    Some((start, end.min(start + most).max(start)))
}

/// `h:mm:ss:ff` at `fps`, the way the editor writes a time.
pub fn timecode(seconds: f64, fps: f64) -> String {
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    let whole = fps.round().max(1.0) as i64;
    let total = (seconds.max(0.0) * fps).round() as i64;
    let frames = total % whole;
    let secs = total / whole;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        frames
    )
}

/// `m:ss`, or `h:mm:ss` past an hour: a length as a person says it.
pub fn length(seconds: f64) -> String {
    let secs = seconds.max(0.0).round() as i64;
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}
