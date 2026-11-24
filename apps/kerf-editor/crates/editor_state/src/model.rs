//! What the bridge says, as the editor reads and shows it. The types are
//! the bridge's own (kerf-runtime's timeline view, kerf-bridge's events),
//! under the editor's names; what the editor adds to them (how a clip, a
//! track, a media or a proposal is named and shown) is here, as an
//! extension trait for each. The window takes its snapshot from the bridge's
//! `state` event, already read; a snapshot or a timeline given as JSON (as a
//! test writes one) reads with [`SnapshotShow::from_value`] and
//! [`TimelineShow::from_value`], leniently: a field left out is at its
//! default, a snapshot's field of the wrong type too, and a timeline that
//! does not read is empty.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

pub use kerf_bridge::event::{
    Author, Conversation, ConversationAsk as Ask, ConversationItem as Turn, ConversationSummary,
    Interp, ProjectionStatus, ProposalEntry as Proposal, ProposalFreshness, ProposalStatus,
    ProposalVersion, QueuedRender as RenderJob, RenderCodec, RenderQuality, RenderStatus, Snapshot,
    TimelineView as Timeline, ViewClip as Clip, ViewEffect, ViewKey, ViewMarker as Marker,
    ViewParam, ViewSequence as SequenceInfo, ViewSource as Source, ViewTake as Take,
    ViewTrack as Track, ViewTrackKind as TrackKind, ViewTransition as Transition,
};

/// How the editor reads and shows a timeline.
pub trait TimelineShow {
    /// Reads a snapshot's timeline, its media named and each clip of media
    /// that has no name of its own given its media's ([`TimelineShow::name_clips`]);
    /// one that does not read (any field of the wrong type) is empty.
    fn from_value(value: &Value) -> Self;

    /// Names the media ([`crate::names::source_names`]) and gives each clip
    /// of media with a file its media's name to go by when it has none of
    /// its own: [`ClipShow::label`] then says it on the timeline, in a
    /// proposal's change rows and wherever else a clip is named. The clip's
    /// own `name` is left as the view has it.
    fn name_clips(&mut self);

    /// The readable name of media `id` ([`crate::names::source_names`]);
    /// the id itself for media the timeline does not have.
    fn media_name(&self, id: &str) -> String;

    /// A track's badge as its head and the Media band say it: "V1", "A2",
    /// counting picture tracks (captions among them) and sound tracks apart,
    /// in the timeline's order. The id itself for a track it does not have.
    fn track_badge(&self, id: &str) -> String;

    /// The badges of the tracks each media's clips are on, by media id, in
    /// the timeline's order ("V1", "A1"): where it is used.
    fn used_on(&self) -> HashMap<String, Vec<String>>;

    /// Width over height of the picture; 16:9 when the sequence says none.
    fn aspect(&self) -> f32;

    /// The picture the program plays into, in pixels: the sequence's own
    /// size; with none, the largest of its media's pictures; with none of
    /// those, 1920 × 1080. kerfd sends a size with every sequence (a
    /// document must say `size=`), so the fallbacks only answer a project
    /// with no sequence, which has no clips to place.
    fn canvas(&self) -> (u32, u32);
}

impl TimelineShow for Timeline {
    fn from_value(value: &Value) -> Self {
        let mut timeline: Self = serde_json::from_value(value.clone()).unwrap_or_default();
        timeline.name_clips();
        timeline
    }

    fn name_clips(&mut self) {
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

    fn media_name(&self, id: &str) -> String {
        if let Some(name) = self.media_names.get(id) {
            return name.clone();
        }
        crate::names::source_names(&self.sources)
            .remove(id)
            .unwrap_or_else(|| id.to_owned())
    }

    fn track_badge(&self, id: &str) -> String {
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

    fn used_on(&self) -> HashMap<String, Vec<String>> {
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

    fn aspect(&self) -> f32 {
        let (width, height) = (self.width.unwrap_or(0), self.height.unwrap_or(0));
        if width > 0 && height > 0 {
            width as f32 / height as f32
        } else {
            16.0 / 9.0
        }
    }

    fn canvas(&self) -> (u32, u32) {
        let (width, height) = (self.width.unwrap_or(0), self.height.unwrap_or(0));
        if width > 0 && height > 0 {
            return (width.max(2), height.max(2));
        }
        self.sources
            .iter()
            .filter_map(|source| Some((source.width?, source.height?)))
            .filter(|(width, height)| *width > 0 && *height > 0)
            .max_by_key(|(width, height)| u64::from(*width) * u64::from(*height))
            .map(|(width, height)| (width.max(2), height.max(2)))
            .unwrap_or((1920, 1080))
    }
}

/// How long a source edit places media that has no length of its own (a
/// still), in seconds.
pub const STILL_SECONDS: f64 = 5.0;

/// How the editor names a track.
pub trait TrackShow {
    /// What the track is called: its name, else its id.
    fn label(&self) -> &str;

    /// A track of room tone: its role holds "room", or its name says
    /// "room tone". Its waveforms are drawn darker.
    fn is_room_tone(&self) -> bool;
}

impl TrackShow for Track {
    fn label(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.id)
    }

    fn is_room_tone(&self) -> bool {
        let role = self.role.as_deref().unwrap_or_default().to_lowercase();
        let name = self.name.as_deref().unwrap_or_default().to_lowercase();
        role.contains("room") || name.contains("room tone")
    }
}

/// How the editor names and shows a clip.
pub trait ClipShow {
    /// What the clip is called: its name, its media's name, its text,
    /// else its id.
    fn label(&self) -> &str;

    /// Property `key`'s one value (`gain` in dB, `opacity`…); null when it
    /// has none or has keyframes.
    fn prop(&self, key: &str) -> &Value;

    /// Its gain in dB (`props.gain`), 0 when it has none.
    fn gain_db(&self) -> f64;

    /// Its gain as keys, in time order, and whether they are keys: a
    /// keyed gain's own, else one key at its start holding its level.
    fn gain_keys(&self) -> (Vec<GainKey>, bool);

    /// What its name bar says after its name: for a generated picture, its
    /// text when that is not its name, its blend when not normal and its
    /// opacity under 100%; for any other clip its speed when not 100%.
    fn secondary(&self) -> String;
}

impl ClipShow for Clip {
    fn label(&self) -> &str {
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

    fn prop(&self, key: &str) -> &Value {
        self.props
            .get(key)
            .and_then(ViewParam::value)
            .unwrap_or(&Value::Null)
    }

    fn gain_db(&self) -> f64 {
        self.prop("gain").as_f64().unwrap_or(0.0)
    }

    fn gain_keys(&self) -> (Vec<GainKey>, bool) {
        let mut keys: Vec<GainKey> = self
            .props
            .get("gain")
            .map(ViewParam::keys)
            .unwrap_or_default()
            .iter()
            .map(|key| GainKey {
                at: key.at,
                frame: key.frame,
                value: key.value.as_f64().unwrap_or(0.0),
                interp: key.interp,
            })
            .collect();
        keys.sort_by_key(|key| key.frame);
        if keys.is_empty() {
            return (
                vec![GainKey {
                    at: 0.0,
                    frame: 0,
                    value: self.gain_db(),
                    interp: Interp::Linear,
                }],
                false,
            );
        }
        (keys, true)
    }

    fn secondary(&self) -> String {
        if self.kind == "generator" {
            let mut parts = Vec::new();
            let text = self
                .prop("text")
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
            if let Some(blend) = self
                .prop("blend")
                .as_str()
                .filter(|blend| !blend.is_empty() && *blend != "normal")
            {
                let mut letters = blend.chars();
                let first = letters.next().map(|c| c.to_uppercase().collect::<String>());
                parts.push(format!("{}{}", first.unwrap_or_default(), letters.as_str()));
            }
            if let Some(opacity) = self.prop("opacity").as_f64().filter(|o| *o < 0.9995) {
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
}

/// One key of a clip's gain: when (seconds and frames from its start),
/// the level in dB, and how it eases to the next.
#[derive(Clone, Debug, PartialEq)]
pub struct GainKey {
    pub at: f64,
    pub frame: i64,
    pub value: f64,
    pub interp: Interp,
}

/// How the editor names a media and places it.
pub trait SourceShow {
    /// The file's name without its folder.
    fn file_name(&self) -> &str;

    /// How long Insert, Overwrite, a double-click or Return place it for,
    /// in seconds: the whole of it; a still, which has no length of its
    /// own, for [`STILL_SECONDS`].
    fn placed_seconds(&self) -> f64;
}

impl SourceShow for Source {
    fn file_name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }

    fn placed_seconds(&self) -> f64 {
        if self.duration.is_finite() && self.duration > 0.0 {
            self.duration
        } else {
            STILL_SECONDS
        }
    }
}

/// How the editor says what a proposal does and who made it.
pub trait ProposalShow {
    /// What it does, in a line: the script's first line that says
    /// something (not blank, not a comment, not `use <sequence>`).
    fn summary(&self) -> String;

    /// Who staged it, as the runtime names them.
    fn author_name(&self) -> String;
}

impl ProposalShow for Proposal {
    fn summary(&self) -> String {
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

    fn author_name(&self) -> String {
        self.author.name.clone()
    }
}

/// How the editor reads a snapshot, and names what its timelines show.
pub trait SnapshotShow {
    /// Reads an editor snapshot (the `state` event's `snapshot`) a field at
    /// a time: one of the wrong type is at its default, a timeline that
    /// does not read is empty, and a proposal that does not read is left
    /// out. Its timelines are named ([`SnapshotShow::name_clips`]).
    fn from_value(value: &Value) -> Self;

    /// Names the media of each of its timelines, the program's and its
    /// proposals' ([`TimelineShow::name_clips`]).
    fn name_clips(&mut self);
}

impl SnapshotShow for Snapshot {
    fn from_value(value: &Value) -> Self {
        let mut fields = value.as_object().cloned().unwrap_or_default();
        // Two things this reader takes that the bridge's type refuses: the
        // texts among import warnings that hold something else, and the
        // proposal's timeline under its old name, which counts where the
        // new one holds no timeline (the type takes the two names as one
        // field said twice).
        if let Some(Value::Array(warnings)) = fields.get_mut("importWarnings") {
            warnings.retain(Value::is_string);
        }
        if let Some(old) = fields.remove("pendingTimeline")
            && old.is_object()
            && !fields.get("proposalTimeline").is_some_and(Value::is_object)
        {
            fields.insert("proposalTimeline".into(), old);
        }
        // Each field as the bridge's reader reads it (its proposals and
        // timelines leniently already), kept only while the snapshot still
        // reads with it: one that does not is left at its default.
        let mut taken = Map::new();
        for (name, field) in fields {
            taken.insert(name.clone(), field);
            if serde_json::from_value::<Self>(Value::Object(taken.clone())).is_err() {
                taken.remove(&name);
            }
        }
        let mut snapshot: Self = serde_json::from_value(Value::Object(taken)).unwrap_or_default();
        snapshot.name_clips();
        snapshot
    }

    fn name_clips(&mut self) {
        self.timeline.name_clips();
        if let Some(timeline) = self.proposal_timeline.as_mut() {
            timeline.name_clips();
        }
        for timeline in self.proposal_timelines.values_mut() {
            timeline.name_clips();
        }
    }
}

/// What the sidebar calls a conversation.
pub trait SummaryShow {
    /// Its title, else "New conversation".
    fn label(&self) -> &str;
}

impl SummaryShow for ConversationSummary {
    fn label(&self) -> &str {
        if self.title.trim().is_empty() {
            "New conversation"
        } else {
            &self.title
        }
    }
}

/// Who said a turn of a conversation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Speaker {
    #[default]
    You,
    Kerf,
}

impl Speaker {
    /// As a conversation keeps it: `you` or `kerf`.
    pub fn wire(self) -> &'static str {
        match self {
            Speaker::You => "you",
            Speaker::Kerf => "kerf",
        }
    }
}

/// Who said a turn, as the editor tells them apart.
pub trait TurnShow {
    /// Kerf for a `kerf` turn, else the person.
    fn speaker(&self) -> Speaker;

    /// Whether the editor shows it: a turn of the person (`you`, or one
    /// that does not say) or of Kerf (`kerf`); one of anyone else is left out.
    fn is_shown(&self) -> bool;
}

impl TurnShow for Turn {
    fn speaker(&self) -> Speaker {
        if self.kind == "kerf" {
            Speaker::Kerf
        } else {
            Speaker::You
        }
    }

    fn is_shown(&self) -> bool {
        matches!(self.kind.as_str(), "" | "you" | "kerf")
    }
}

/// Where a render job of the queue stands, for its row.
pub trait RenderJobShow {
    /// Waiting its turn or rendering.
    fn is_active(&self) -> bool;
}

impl RenderJobShow for RenderJob {
    fn is_active(&self) -> bool {
        self.status.is_under_way()
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

/// The clips `proposed` adds, takes out or changes against `current`, in
/// the proposal's track order with the clips it takes out after.
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
            if change.leaves_ghost()
                && let Some((start, end)) = change.old
            {
                let track = change
                    .old_track
                    .clone()
                    .unwrap_or_else(|| change.track.clone());
                band.ghosts.push((track, start, end));
            }
        }
        ranged.then_some(band)
    }
}

/// A time Kerf names, in seconds of the program:
/// a number of seconds, "4.5s" or "4.5", "12f" in frames at `fps`, or a
/// timecode "HH:MM:SS:FF" (or "MM:SS:FF") less the sequence's `start`.
/// None for anything else, or a time before the program.
pub fn focus_seconds(value: &Value, fps: f64, start: f64) -> Option<f64> {
    match value {
        Value::Number(number) => number
            .as_f64()
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0),
        Value::String(text) => focus_time(text, fps, start),
        _ => None,
    }
}

/// A time Kerf names as written ([`focus_seconds`]): "4.5s" or "4.5",
/// "12f", or a timecode.
pub fn focus_time(text: &str, fps: f64, start: f64) -> Option<f64> {
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    let text = text.trim();
    let seconds = if let Some(frames) = text.strip_suffix('f') {
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
    };
    (seconds.is_finite() && seconds >= 0.0).then_some(seconds)
}

/// Keys for a number drawn by hand, a value 0..1 at each frame: the first
/// frame, the last, and between them only the frames where the line from
/// the last key kept would pass a frame between by more than `within`.
/// Each `{at: frame, value, interp: linear}`, values to three places.
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
/// Overwrite): of each kind the source has, one track, never a locked one.
/// When the person has chosen targets it is the first of that kind they
/// targeted, and a kind with none targeted is left out; else it is the
/// first unlocked track of that kind.
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
