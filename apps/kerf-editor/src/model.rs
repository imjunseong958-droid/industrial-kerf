//! What the bridge says, as typed data: the timeline view an editor snapshot
//! carries, conversations kept beside the project, proposals, render jobs.
//! Read leniently: a field the bridge leaves out is at its default, so a
//! newer kerfd never breaks the window.

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

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

    /// A track of room tone: its role holds "room", or its name says
    /// "room tone". Its waveforms are drawn darker (Qt's `isRoomTone`).
    pub fn is_room_tone(&self) -> bool {
        let role = self.role.as_deref().unwrap_or_default().to_lowercase();
        let name = self.name.as_deref().unwrap_or_default().to_lowercase();
        role.contains("room") || name.contains("room tone")
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
    /// A signal generator's number over the clip, 0..1, as kerfd samples
    /// it (at most a few hundred values, spread evenly across the clip).
    pub signal: Vec<f64>,
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

    /// Its gain as keys, in time order, and whether they are keys: a
    /// keyed gain's own (`props.gain` an array of `{at, frame, value,
    /// interp}`, `at` in seconds from the clip's start, `frame` counted
    /// from its first frame), else one key at its start holding its level.
    pub fn gain_keys(&self, fps: f64) -> (Vec<GainKey>, bool) {
        let fps = if fps.is_finite() && fps > 0.0 {
            fps
        } else {
            30.0
        };
        let gain = &self.props["gain"];
        let mut keys: Vec<GainKey> = gain
            .as_array()
            .map(|keys| {
                keys.iter()
                    .filter(|key| key.is_object())
                    .map(|key| {
                        let frame = key["frame"].as_f64().map(|frame| frame.round() as i64);
                        let at = key["at"]
                            .as_f64()
                            .unwrap_or_else(|| frame.unwrap_or(0) as f64 / fps);
                        GainKey {
                            at,
                            frame: frame.unwrap_or_else(|| (at * fps).round() as i64),
                            value: key["value"].as_f64().unwrap_or(0.0),
                            interp: key["interp"].as_str().unwrap_or("linear").to_owned(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        keys.sort_by_key(|key| key.frame);
        if keys.is_empty() {
            return (
                vec![GainKey {
                    at: 0.0,
                    frame: 0,
                    value: gain.as_f64().unwrap_or(0.0),
                    interp: "linear".into(),
                }],
                false,
            );
        }
        (keys, true)
    }

    /// What its name bar says after its name (Qt's `clipSecondary`): for
    /// a generated picture, its text when that is not its name, its blend
    /// when not normal and its opacity under 100%; for any other clip its
    /// speed when not 100%.
    pub fn secondary(&self) -> String {
        if self.kind == "generator" {
            let mut parts = Vec::new();
            let text = self.props["text"]
                .as_str()
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let name = self
                .label()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !text.is_empty() && text != name {
                parts.push(text);
            }
            if let Some(blend) = self.props["blend"]
                .as_str()
                .filter(|blend| !blend.is_empty() && *blend != "normal")
            {
                let mut letters = blend.chars();
                let first = letters.next().map(|c| c.to_uppercase().collect::<String>());
                parts.push(format!("{}{}", first.unwrap_or_default(), letters.as_str()));
            }
            if let Some(opacity) = self.props["opacity"].as_f64().filter(|o| *o < 0.9995) {
                parts.push(format!("{}%", (opacity * 100.0).round() as i64));
            }
            return parts.join(" · ");
        }
        let speed = self.speed.unwrap_or(1.0);
        if (speed - 1.0).abs() > 0.0005 && speed.is_finite() {
            return format!("{}%", (speed.abs() * 100.0).round() as i64);
        }
        String::new()
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

/// One key of a clip's gain: when (seconds and frames from its start),
/// the level in dB, and how it eases to the next.
#[derive(Clone, Debug, PartialEq)]
pub struct GainKey {
    pub at: f64,
    pub frame: i64,
    pub value: f64,
    pub interp: String,
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
    /// Every version of it, oldest first, the current one among them.
    pub versions: Vec<ProposalVersion>,
}

/// One version of a proposal, as its versions menu lists it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ProposalVersion {
    pub version: u32,
    pub proposal_id: String,
    pub base_revision: u64,
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
    /// Every open proposal's timeline by its id, and its earlier versions'
    /// made on this revision (`proposalTimelines`).
    pub proposal_timelines: HashMap<String, Timeline>,
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
        let proposal_timelines = value["proposalTimelines"]
            .as_object()
            .map(|timelines| {
                timelines
                    .iter()
                    .filter(|(_, timeline)| timeline.is_object())
                    .map(|(id, timeline)| (id.clone(), Timeline::from_value(timeline)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            document: value["document"].as_str().unwrap_or_default().to_owned(),
            timeline: Timeline::from_value(&value["timeline"]),
            revision: value["revision"].as_u64().unwrap_or(0),
            can_undo: value["canUndo"].as_bool().unwrap_or(false),
            can_redo: value["canRedo"].as_bool().unwrap_or(false),
            duration: value["duration"].as_f64().unwrap_or(0.0),
            proposals,
            proposal_timeline,
            proposal_timelines,
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

    /// The timeline `proposal` would leave, as the timeline shows it: the
    /// version `shown` names when it is one of its versions and kerfd drew
    /// it (the pinned card's shown version), else its own; the pending
    /// proposal's `proposalTimeline` when nothing else has one.
    pub fn timeline_of(&self, proposal: &Proposal, shown: Option<&str>) -> Option<&Timeline> {
        let version = shown.filter(|shown| {
            proposal.proposal_id == *shown
                || proposal
                    .versions
                    .iter()
                    .any(|version| version.proposal_id == *shown)
        });
        version
            .and_then(|id| self.proposal_timelines.get(id))
            .or_else(|| self.proposal_timelines.get(&proposal.proposal_id))
            .or_else(|| {
                self.pending()
                    .filter(|pending| pending.proposal_id == proposal.proposal_id)
                    .and(self.proposal_timeline.as_ref())
            })
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

/// What a proposal does to one clip of the program: where it was and
/// where it would be, in seconds, on which track.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipChange {
    pub clip: String,
    /// The track it is on in the proposal (its old one when removed).
    pub track: String,
    /// The track it was on, when it was in the program.
    pub old_track: Option<String>,
    pub old: Option<(f64, f64)>,
    pub new: Option<(f64, f64)>,
    pub moved: bool,
    pub trimmed: bool,
    pub retracked: bool,
}

impl ClipChange {
    pub fn added(&self) -> bool {
        self.old.is_none()
    }

    pub fn removed(&self) -> bool {
        self.new.is_none()
    }

    /// Where the change reaches, from its earliest edge to its latest.
    pub fn span(&self) -> (f64, f64) {
        let edges = [self.old, self.new];
        let mut spans = edges.iter().flatten();
        let first = spans.next().copied().unwrap_or((0.0, 0.0));
        spans.fold(first, |(low, high), (start, end)| {
            (low.min(*start), high.max(*end))
        })
    }

    /// Whether the clip it changes stands somewhere else, or no longer
    /// stands, in the program: where it is now shows as a ghost while the
    /// proposal plays.
    pub fn leaves_ghost(&self) -> bool {
        !self.added() && (self.moved || self.trimmed || self.removed() || self.retracked)
    }
}

/// The clips `proposed` adds, takes out or changes against `current`
/// (Qt's `ProposalModel::diffTimelines`), in the proposal's track order
/// with the clips it takes out after.
pub fn proposal_changes(current: &Timeline, proposed: &Timeline) -> Vec<ClipChange> {
    let mut changes = Vec::new();
    for track in &proposed.tracks {
        for clip in &track.clips {
            let new = Some((clip.timeline_start, clip.end()));
            match current.clip(&clip.id) {
                None => changes.push(ClipChange {
                    clip: clip.id.clone(),
                    track: track.id.clone(),
                    old_track: None,
                    old: None,
                    new,
                    moved: false,
                    trimmed: false,
                    retracked: false,
                }),
                Some((was_on, was)) => {
                    let moved = was.start_frame != clip.start_frame;
                    let trimmed = was.duration_frames != clip.duration_frames
                        || was.source_in != clip.source_in
                        || was.source_out != clip.source_out;
                    let retracked = was_on.id != track.id;
                    let other = was.speed != clip.speed
                        || was.reverse != clip.reverse
                        || was.enabled != clip.enabled
                        || was.source_id != clip.source_id
                        || was.fx != clip.fx
                        || was.name != clip.name
                        || was.text != clip.text
                        || was.effects != clip.effects
                        || was.props != clip.props;
                    if moved || trimmed || retracked || other {
                        changes.push(ClipChange {
                            clip: clip.id.clone(),
                            track: track.id.clone(),
                            old_track: Some(was_on.id.clone()),
                            old: Some((was.timeline_start, was.end())),
                            new,
                            moved,
                            trimmed,
                            retracked,
                        });
                    }
                }
            }
        }
    }
    for track in &current.tracks {
        for clip in &track.clips {
            if proposed.clip(&clip.id).is_none() {
                changes.push(ClipChange {
                    clip: clip.id.clone(),
                    track: track.id.clone(),
                    old_track: Some(track.id.clone()),
                    old: Some((clip.timeline_start, clip.end())),
                    new: None,
                    moved: false,
                    trimmed: false,
                    retracked: false,
                });
            }
        }
    }
    changes
}

/// An open proposal as the timeline shows it over the program: a band on
/// the ruler from its earliest change to its latest, a veil over the
/// tracks it changes, its clips outlined, and the ghosts a preview of it
/// leaves where the clips it moves, trims or takes out are now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProposalBand {
    pub id: String,
    pub author: String,
    pub start: f64,
    pub end: f64,
    pub tracks: Vec<String>,
    pub clips: Vec<String>,
    /// Track, start and end of where each clip it changes is now.
    pub ghosts: Vec<(String, f64, f64)>,
}

impl ProposalBand {
    /// The band of proposal `id` by `author` that would turn `current`
    /// into `proposed`; None when it changes no clip.
    pub fn of(id: &str, author: &str, current: &Timeline, proposed: &Timeline) -> Option<Self> {
        let changes = proposal_changes(current, proposed);
        let mut band = ProposalBand {
            id: id.to_owned(),
            author: author.to_owned(),
            ..Default::default()
        };
        let mut ranged = false;
        for change in &changes {
            let (start, end) = change.span();
            if ranged {
                band.start = band.start.min(start);
                band.end = band.end.max(end);
            } else {
                (band.start, band.end) = (start, end);
                ranged = true;
            }
            for track in std::iter::once(&change.track).chain(change.old_track.as_ref()) {
                if !band.tracks.contains(track) {
                    band.tracks.push(track.clone());
                }
            }
            band.clips.push(change.clip.clone());
            if change.leaves_ghost() {
                if let Some((start, end)) = change.old {
                    let track = change
                        .old_track
                        .clone()
                        .unwrap_or_else(|| change.track.clone());
                    band.ghosts.push((track, start, end));
                }
            }
        }
        ranged.then_some(band)
    }
}

/// A time Kerf names, in seconds of the program (Qt's `reel::parseTime`):
/// a number of seconds, "4.5s" or "4.5", "12f" in frames at `fps`, or a
/// timecode "HH:MM:SS:FF" (or "MM:SS:FF") less the sequence's `start`.
/// None for anything else, or a time before the program.
pub fn focus_seconds(value: &Value, fps: f64, start: f64) -> Option<f64> {
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    let seconds = match value {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => {
            let text = text.trim();
            if let Some(frames) = text.strip_suffix('f') {
                frames.trim().parse::<f64>().ok()? / fps
            } else if let Some(seconds) = text.strip_suffix('s') {
                seconds.trim().parse::<f64>().ok()?
            } else if text.contains(':') {
                let parts: Vec<&str> = text.split([':', ';']).collect();
                if parts.len() < 3 || parts.len() > 4 {
                    return None;
                }
                let numbers: Vec<f64> = parts
                    .iter()
                    .map(|part| part.trim().parse::<f64>().ok())
                    .collect::<Option<_>>()?;
                let (hours, minutes, secs, frames) = match numbers.as_slice() {
                    [h, m, s, f] => (*h, *m, *s, *f),
                    [m, s, f] => (0.0, *m, *s, *f),
                    _ => return None,
                };
                hours * 3600.0 + minutes * 60.0 + secs + frames / fps - start.max(0.0)
            } else {
                text.parse::<f64>().ok()?
            }
        }
        _ => return None,
    };
    (seconds.is_finite() && seconds >= 0.0).then_some(seconds)
}

/// Keys for a number drawn by hand, a value 0..1 at each frame (Qt's
/// `signalcurve::keys`): the first frame, the last, and between them only
/// the frames where the line from the last key kept would pass a frame
/// between by more than `within`. Each `{at: frame, value, interp: linear}`,
/// values to three places.
pub fn signal_keys(drawn: &BTreeMap<i64, f64>, within: f64) -> Vec<Value> {
    let frames: Vec<i64> = drawn.keys().copied().collect();
    let Some(&first) = frames.first() else {
        return Vec::new();
    };
    let value = |frame: i64| drawn.get(&frame).copied().unwrap_or(0.0);
    let key = |frame: i64| {
        json!({
            "at": frame,
            "value": (value(frame) * 1000.0).round() / 1000.0,
            "interp": "linear",
        })
    };
    let mut keys = vec![key(first)];
    let mut kept = 0;
    for next in 2..frames.len() {
        let (a, b) = (value(frames[kept]), value(frames[next]));
        let span = (frames[next] - frames[kept]) as f64;
        let misses = (kept + 1..next).any(|between| {
            let on = a + (b - a) * (frames[between] - frames[kept]) as f64 / span;
            (on - value(frames[between])).abs() > within
        });
        if misses {
            kept = next - 1;
            keys.push(key(frames[kept]));
        }
    }
    if frames.len() > 1 {
        keys.push(key(*frames.last().unwrap_or(&first)));
    }
    keys
}

/// Where a source's picture and sound land when it is put in (Insert,
/// Overwrite; Qt's `sourceTargets`): of each kind the source has, one
/// track, never a locked one. When the person has chosen targets it is the
/// first of that kind they targeted, and a kind with none targeted is left
/// out; else it is the first unlocked track of that kind.
pub fn source_targets(
    timeline: &Timeline,
    source: &Source,
    chosen: Option<&[String]>,
) -> Vec<String> {
    let kinds = [
        (TrackKind::Video, source.width.is_some()),
        (TrackKind::Audio, source.has_audio),
    ];
    let mut targets = Vec::new();
    for (kind, has) in kinds {
        if !has {
            continue;
        }
        let first = timeline.tracks.iter().find(|track| {
            track.kind == kind
                && !track.locked
                && chosen.is_none_or(|chosen| chosen.contains(&track.id))
        });
        targets.extend(first.map(|track| track.id.clone()));
    }
    targets
}

/// The tracks targeted before the person chooses any: the first unlocked
/// picture track and the first unlocked sound track.
pub fn default_targets(timeline: &Timeline) -> Vec<String> {
    [TrackKind::Video, TrackKind::Audio]
        .into_iter()
        .filter_map(|kind| {
            timeline
                .tracks
                .iter()
                .find(|track| track.kind == kind && !track.locked)
                .map(|track| track.id.clone())
        })
        .collect()
}

/// `targets` with track `id`'s target chip pressed. A source's picture
/// and sound each go on one track, so targeting a track takes its kind's
/// target from whichever track of that kind had it; pressing a targeted
/// track lets it go, and nothing of its kind is put in.
pub fn toggle_target(timeline: &Timeline, targets: &[String], id: &str) -> Vec<String> {
    if targets.iter().any(|known| known == id) {
        return targets
            .iter()
            .filter(|known| *known != id)
            .cloned()
            .collect();
    }
    let kind_of = |track: &str| {
        timeline
            .tracks
            .iter()
            .find(|known| known.id == track)
            .map(|known| known.kind)
    };
    let kind = kind_of(id);
    let mut toggled: Vec<String> = targets
        .iter()
        .filter(|known| kind.is_none() || kind_of(known) != kind)
        .cloned()
        .collect();
    toggled.push(id.to_owned());
    toggled
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
