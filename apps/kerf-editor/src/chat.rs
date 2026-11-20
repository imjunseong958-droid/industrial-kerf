//! The conversation with Kerf: what you asked, what Kerf did and said, the
//! cut it proposes, and the composer.
//!
//! Your request and Kerf's answer fade in where they are; the list keeps to
//! the latest unless you scrolled away. While Kerf works, under your request
//! "Working" has a band of light passing over it once each `WORK_SHIMMER`
//! (still under Reduce Motion), with the time so far and the steps it took;
//! done, it folds to "Worked for …", which unfolds its steps, its chevron
//! turning over `FADE`.
//!
//! What Kerf writes reads as written: bold, code, and the timecodes it names
//! as links that move the playhead there. Under a finished answer, Copy; under
//! a failed or stopped one, Ask again. A request written while Kerf works
//! waits over the composer ("Next: …") and goes when the turn ends.
//!
//! Proposal cards say what each proposal does and the clips it changes, with
//! its versions; the open one nearest the playhead is pinned.

use crate::commands::{RevealPicture, RunCommand};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::model::{self, Proposal, Speaker, Timeline, Turn};
use crate::motion::{self, Tween};
use crate::prefs::{Permissions, Provider};
use crate::session::{BridgeEvent, Session};
use crate::store::{Change, Connection, StepState};
use crate::theme::{self, layout, motion as scene, text};
use crate::ui::{self, icon_button};
use gpui::{
    anchored, deferred, div, linear_color_stop, linear_gradient, point, prelude::*, px, radians,
    AnimationExt, AnyElement, App, ClipboardItem, Context, Corner, Edges, ElementId, Entity,
    FocusHandle, Focusable, FontWeight, InteractiveText, Rgba, ScrollHandle, SharedString,
    StyledText, Subscription, TextRun, Transformation, Window,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// The most the composer's text grows to before it scrolls, in points.
pub const COMPOSER_MOST: f32 = 110.0;

// ---------------------------------------------------------------- what Kerf wrote

/// A piece of what Kerf wrote, as it reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Plain(String),
    /// Between `**` marks.
    Bold(String),
    /// Between backticks.
    Code(String),
    /// A timecode, `HH:MM:SS:FF` or `HH:MM:SS;FF`: a link to that moment.
    Timecode(String),
}

/// What Kerf wrote, cut into what reads plain, bold, as code, and the
/// timecodes it names. Marks with no partner stay as written; nothing
/// inside code is read as a mark or a timecode.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let Some(length) = rest[open + 1..].find('`') else {
            break;
        };
        bold_and_timecodes(&rest[..open], &mut out);
        let code = &rest[open + 1..open + 1 + length];
        if !code.is_empty() {
            out.push(Span::Code(code.to_owned()));
        }
        rest = &rest[open + 1 + length + 1..];
    }
    bold_and_timecodes(rest, &mut out);
    // Plain pieces side by side read as one.
    let mut merged: Vec<Span> = Vec::with_capacity(out.len());
    for span in out {
        match (merged.last_mut(), span) {
            (Some(Span::Plain(last)), Span::Plain(next)) => last.push_str(&next),
            (_, span) => merged.push(span),
        }
    }
    merged
}

fn bold_and_timecodes(text: &str, out: &mut Vec<Span>) {
    let parts: Vec<&str> = text.split("**").collect();
    // An odd number of parts has every mark paired; else the last is alone.
    let paired = parts.len() % 2 == 1;
    for (index, part) in parts.iter().enumerate() {
        let bold = index % 2 == 1;
        if bold && !paired && index == parts.len() - 1 {
            timecodes(&format!("**{part}"), false, out);
        } else {
            timecodes(part, bold, out);
        }
    }
}

fn timecodes(text: &str, bold: bool, out: &mut Vec<Span>) {
    let words = |words: &str| {
        if bold {
            Span::Bold(words.to_owned())
        } else {
            Span::Plain(words.to_owned())
        }
    };
    let bytes = text.as_bytes();
    let mut from = 0;
    let mut at = 0;
    while at + 11 <= bytes.len() {
        if is_timecode(&bytes[at..at + 11])
            && (at == 0 || !matches!(bytes[at - 1], b'0'..=b'9' | b':' | b';'))
            && bytes.get(at + 11).is_none_or(|next| !next.is_ascii_digit())
        {
            if from < at {
                out.push(words(&text[from..at]));
            }
            out.push(Span::Timecode(text[at..at + 11].to_owned()));
            at += 11;
            from = at;
        } else {
            at += 1;
        }
    }
    if from < text.len() {
        out.push(words(&text[from..]));
    }
}

/// `DD:DD:DD:DD`, the last separator `:` or `;`.
fn is_timecode(bytes: &[u8]) -> bool {
    bytes.len() == 11
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            2 | 5 => *byte == b':',
            8 => *byte == b':' || *byte == b';',
            _ => byte.is_ascii_digit(),
        })
}

/// The seconds a timecode Kerf wrote names at `fps`: whole frames of the
/// rounded rate, as [`model::timecode`] writes them. None for one that
/// does not read.
pub fn timecode_seconds(timecode: &str, fps: f64) -> Option<f64> {
    if !is_timecode(timecode.as_bytes()) {
        return None;
    }
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    let part = |range: std::ops::Range<usize>| timecode[range].parse::<i64>().ok();
    let (hours, minutes, seconds, frames) = (part(0..2)?, part(3..5)?, part(6..8)?, part(9..11)?);
    let whole = hours * 3600 + minutes * 60 + seconds;
    let total = whole * (fps.round().max(1.0) as i64) + frames;
    Some(total as f64 / fps)
}

/// A request written while Kerf works, joined to the one already waiting.
pub fn join_queued(waiting: Option<&str>, text: &str) -> String {
    let text = text.trim();
    match waiting.map(str::trim).filter(|waiting| !waiting.is_empty()) {
        Some(waiting) if !text.is_empty() => format!("{waiting}\n\n{text}"),
        Some(waiting) => waiting.to_owned(),
        None => text.to_owned(),
    }
}

/// "Worked for 1m 12s · 6 steps": what a finished turn folds to.
pub fn worked_headline(seconds: Option<f64>, steps: usize) -> String {
    let seconds = seconds.unwrap_or(0.0).max(0.0).round() as u64;
    let length = if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    };
    match steps {
        0 => format!("Worked for {length}"),
        1 => format!("Worked for {length} · 1 step"),
        many => format!("Worked for {length} · {many} steps"),
    }
}

/// The request a Kerf turn answered: the last of yours before it.
pub fn request_before(turns: &[Turn], index: usize) -> Option<&str> {
    turns[..index.min(turns.len())]
        .iter()
        .rev()
        .find(|turn| turn.kind == Speaker::You)
        .map(|turn| turn.text.as_str())
        .filter(|text| !text.trim().is_empty())
}

/// What the permission chip says, and its line in the chip's menu.
pub fn permission_words(permissions: Permissions) -> (&'static str, &'static str) {
    match permissions {
        Permissions::Ask => ("Ask before edits", "Each edit waits for you"),
        Permissions::AcceptEdits => ("Accept edits", "Edits apply as Kerf makes them"),
        Permissions::Auto => ("Auto", "Edits apply, and Kerf makes its shots"),
    }
}

/// The models the chip offers for `provider`: Claude Code's own aliases
/// ("default" leaves the choice to it), else the one typed in Settings, or
/// the service's default.
pub fn model_choices(provider: Provider, configured: &str) -> Vec<String> {
    match provider {
        Provider::ClaudeCode => ["default", "opus", "sonnet", "haiku"]
            .map(str::to_owned)
            .to_vec(),
        _ if configured.trim().is_empty() => vec!["default".to_owned()],
        _ => vec![configured.trim().to_owned()],
    }
}

/// A model as the chip names it: an alias capitalized, else as written.
pub fn model_label(model: &str) -> String {
    match model {
        "default" | "opus" | "sonnet" | "haiku" => {
            let mut chars = model.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
        "" => "Default".to_owned(),
        other => other.to_owned(),
    }
}

// ---------------------------------------------------------------- proposals

/// One version of a proposal, as the snapshot lists them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Version {
    pub version: u32,
    pub proposal_id: String,
    pub script: String,
    pub document_diff: String,
}

/// What a snapshot says of its proposals beyond [`Proposal`]: each one's
/// versions, what it replaced, the revision it was accepted in, and the
/// timelines of the open ones (and their earlier versions on this revision).
#[derive(Clone, Debug, Default)]
pub struct ProposalBook {
    /// By the proposal's current id: every version, oldest first, the
    /// current one included.
    pub versions: HashMap<String, Vec<Version>>,
    /// By the proposal's current id: the id this version replaced.
    pub supersedes: HashMap<String, String>,
    pub applied: HashMap<String, u64>,
    /// By any version's id.
    pub timelines: HashMap<String, Timeline>,
}

impl ProposalBook {
    /// Reads a `state` event's snapshot.
    pub fn read(snapshot: &Value) -> Self {
        let mut book = Self::default();
        for entry in snapshot["proposals"].as_array().into_iter().flatten() {
            let Some(id) = entry["proposalId"].as_str().filter(|id| !id.is_empty()) else {
                continue;
            };
            let versions: Vec<Version> = entry["versions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|version| {
                    let proposal_id = version["proposalId"].as_str()?.to_owned();
                    Some(Version {
                        version: version["version"].as_u64().unwrap_or(1) as u32,
                        proposal_id,
                        script: version["script"].as_str().unwrap_or_default().to_owned(),
                        document_diff: version["documentDiff"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    })
                })
                .collect();
            if !versions.is_empty() {
                book.versions.insert(id.to_owned(), versions);
            }
            if let Some(replaced) = entry["supersedes"].as_str().filter(|id| !id.is_empty()) {
                book.supersedes.insert(id.to_owned(), replaced.to_owned());
            }
            if let Some(revision) = entry["appliedRevision"].as_u64() {
                book.applied.insert(id.to_owned(), revision);
            }
        }
        if let Some(timelines) = snapshot["proposalTimelines"].as_object() {
            for (id, timeline) in timelines {
                if timeline.is_object() {
                    book.timelines
                        .insert(id.clone(), Timeline::from_value(timeline));
                }
            }
        }
        book
    }

    /// Every version of `proposal`, oldest first; the current one alone when
    /// the snapshot listed none.
    pub fn versions_of(&self, proposal: &Proposal) -> Vec<Version> {
        self.versions
            .get(&proposal.proposal_id)
            .cloned()
            .unwrap_or_else(|| {
                vec![Version {
                    version: proposal.version.max(1),
                    proposal_id: proposal.proposal_id.clone(),
                    script: proposal.script.clone(),
                    document_diff: proposal.document_diff.clone(),
                }]
            })
    }

    /// Whether `id` is `proposal` by any of the ids it has gone by.
    pub fn is(&self, proposal: &Proposal, id: &str) -> bool {
        proposal.proposal_id == id
            || self
                .supersedes
                .get(&proposal.proposal_id)
                .map(String::as_str)
                == Some(id)
            || self
                .versions
                .get(&proposal.proposal_id)
                .is_some_and(|versions| versions.iter().any(|version| version.proposal_id == id))
    }

    /// The proposal among `proposals` that `id` names, by any of its ids.
    pub fn find<'a>(&self, proposals: &'a [Proposal], id: &str) -> Option<&'a Proposal> {
        proposals
            .iter()
            .find(|proposal| proposal.proposal_id == id)
            .or_else(|| proposals.iter().find(|proposal| self.is(proposal, id)))
    }

    /// The timeline of the version `id` of `proposal`, else of its newest
    /// version that has one.
    pub fn timeline_of(&self, proposal: &Proposal, id: &str) -> Option<&Timeline> {
        self.timelines.get(id).or_else(|| {
            self.versions_of(proposal)
                .iter()
                .rev()
                .find_map(|version| self.timelines.get(&version.proposal_id))
        })
    }
}

/// A clip a proposal changes, from its timeline against the program's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipChange {
    pub clip: String,
    /// What the clip is called.
    pub name: String,
    /// The track it ends up on (or was on, removed), as it is called.
    pub track: String,
    pub added: bool,
    pub removed: bool,
    /// Its start changed.
    pub moved: bool,
    /// Its length changed.
    pub trimmed: bool,
    pub effects: bool,
    /// Its text, generator or source in changed.
    pub properties: bool,
    pub retracked: bool,
    pub toggled: bool,
    pub retimed: bool,
    pub relinked: bool,
    pub old_start: f64,
    pub old_end: f64,
    pub new_start: f64,
    pub new_end: f64,
}

impl ClipChange {
    /// Where on the program it lies, before and after.
    pub fn span(&self) -> (f64, f64) {
        if self.added {
            (self.new_start, self.new_end)
        } else if self.removed {
            (self.old_start, self.old_end)
        } else {
            (
                self.old_start.min(self.new_start),
                self.old_end.max(self.new_end),
            )
        }
    }

    /// Its row on a card: the clip and its track, where it goes, and by
    /// how much ("Logo · V2", "→ 00:00:26:12", "+17f").
    pub fn row(&self, fps: f64) -> (String, String, String) {
        let name = if self.track.is_empty() {
            self.name.clone()
        } else {
            format!("{} · {}", self.name, self.track)
        };
        let frames = |seconds: f64| (seconds * fps).round() as i64;
        let (target, shift) = if self.removed {
            ("removed".to_owned(), String::new())
        } else if self.added {
            (
                format!("→ {}", model::timecode(self.new_start, fps)),
                "new".to_owned(),
            )
        } else if self.moved {
            (
                format!("→ {}", model::timecode(self.new_start, fps)),
                signed_frames(frames(self.new_start - self.old_start)),
            )
        } else if self.trimmed {
            (
                format!("→ {}", model::timecode(self.new_end, fps)),
                signed_frames(frames(
                    (self.new_end - self.new_start) - (self.old_end - self.old_start),
                )),
            )
        } else {
            let what: Vec<&str> = [
                (self.effects, "effects"),
                (self.properties, "properties"),
                (self.toggled, "enabled"),
                (self.retimed, "speed"),
                (self.relinked, "link"),
                (self.retracked, "track"),
            ]
            .into_iter()
            .filter_map(|(changed, word)| changed.then_some(word))
            .collect();
            (what.join(", "), String::new())
        };
        (name, target, shift)
    }
}

/// Frames as a shift: "+17f", "−3f", "0f".
pub fn signed_frames(frames: i64) -> String {
    match frames {
        0 => "0f".to_owned(),
        frames if frames < 0 => format!("−{}f", -frames),
        frames => format!("+{frames}f"),
    }
}

/// The clips `proposed` changes against `program`, by clip id: those it
/// adds, in its order, those it changes, then those it removes.
pub fn clip_changes(program: &Timeline, proposed: &Timeline) -> Vec<ClipChange> {
    const NEAR: f64 = 1e-4;
    type Placed<'a> = (&'a model::Track, &'a model::Clip);
    fn placed(timeline: &Timeline) -> Vec<Placed<'_>> {
        timeline
            .tracks
            .iter()
            .flat_map(|track| track.clips.iter().map(move |clip| (track, clip)))
            .collect()
    }
    let before = placed(program);
    let after = placed(proposed);
    let mut changes = Vec::new();
    for (track, clip) in &after {
        let mut change = ClipChange {
            clip: clip.id.clone(),
            name: clip.label().to_owned(),
            track: track.label().to_owned(),
            new_start: clip.timeline_start,
            new_end: clip.end(),
            ..ClipChange::default()
        };
        let Some((was_track, was)) = before.iter().find(|(_, was)| was.id == clip.id) else {
            change.added = true;
            changes.push(change);
            continue;
        };
        change.old_start = was.timeline_start;
        change.old_end = was.end();
        change.moved = (clip.timeline_start - was.timeline_start).abs() > NEAR;
        change.trimmed = (clip.duration - was.duration).abs() > NEAR;
        change.effects = clip.effects != was.effects;
        change.properties = clip.text != was.text
            || clip.fx != was.fx
            || clip.name != was.name
            || clip.source_id != was.source_id
            || clip
                .source_in
                .zip(was.source_in)
                .is_some_and(|(now, then)| (now - then).abs() > NEAR);
        change.retracked = track.id != was_track.id;
        change.toggled = clip.enabled != was.enabled;
        change.retimed = clip.speed != was.speed;
        change.relinked = clip.link != was.link;
        if change.moved
            || change.trimmed
            || change.effects
            || change.properties
            || change.retracked
            || change.toggled
            || change.retimed
            || change.relinked
        {
            changes.push(change);
        }
    }
    for (track, clip) in &before {
        if after.iter().any(|(_, now)| now.id == clip.id) {
            continue;
        }
        changes.push(ClipChange {
            clip: clip.id.clone(),
            name: clip.label().to_owned(),
            track: track.label().to_owned(),
            removed: true,
            old_start: clip.timeline_start,
            old_end: clip.end(),
            ..ClipChange::default()
        });
    }
    changes
}

/// The stretch of the program `changes` lie in; None for none.
pub fn changes_span(changes: &[ClipChange]) -> Option<(f64, f64)> {
    changes
        .iter()
        .map(ClipChange::span)
        .reduce(|(low, high), (start, end)| (low.min(start), high.max(end)))
}

/// Seconds from `playhead` to `span`: none inside it, endless without one.
pub fn distance(span: Option<(f64, f64)>, playhead: f64) -> f64 {
    match span {
        Some((start, _)) if playhead < start => start - playhead,
        Some((_, end)) if playhead > end => playhead - end,
        Some(_) => 0.0,
        None => f64::INFINITY,
    }
}

/// The open proposals' ids, the nearest to the playhead first (the pinned
/// one), then in the snapshot's order. `fallback` is the timeline the
/// snapshot drew for its first open proposal.
pub fn pin_order(
    proposals: &[Proposal],
    book: &ProposalBook,
    program: &Timeline,
    fallback: Option<&Timeline>,
    playhead: f64,
) -> Vec<String> {
    let first_open = proposals
        .iter()
        .find(|proposal| proposal.is_pending())
        .map(|proposal| proposal.proposal_id.clone());
    let mut open: Vec<(f64, String)> = proposals
        .iter()
        .filter(|proposal| proposal.is_pending())
        .map(|proposal| {
            let timeline = book
                .timeline_of(proposal, &proposal.proposal_id)
                .or(fallback
                    .filter(|_| first_open.as_deref() == Some(proposal.proposal_id.as_str())));
            let span = timeline.and_then(|timeline| changes_span(&clip_changes(program, timeline)));
            (distance(span, playhead), proposal.proposal_id.clone())
        })
        .collect();
    // Stable: equally near ones keep the snapshot's order.
    open.sort_by(|a, b| a.0.total_cmp(&b.0));
    open.into_iter().map(|(_, id)| id).collect()
}

/// What a script does, in a line: its first line that says something,
/// else "Proposal vN".
pub fn script_summary(script: &str, version: u32) -> String {
    script
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("//")
                && !line.starts_with("use ")
        })
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Proposal v{}", version.max(1)))
}

// ---------------------------------------------------------------- the view

/// Which of the conversation's menus is open.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Menu {
    Permissions,
    Model,
    /// A card's versions, by the proposal's id.
    Versions(String),
}

pub struct ChatView {
    session: Entity<Session>,
    composer: Entity<TextInput>,
    makings: Entity<crate::making::Makings>,
    scroll: ScrollHandle,
    follow: bool,
    /// Proposals whose script shows, by proposal id.
    scripts: HashSet<String>,
    /// What folds (a turn's steps, a card's changes), each with its
    /// chevron's turn: 1 open.
    folds: HashMap<String, Tween>,
    /// The version a card shows where it is not the newest, by proposal id.
    shown: HashMap<String, String>,
    book: ProposalBook,
    /// A request written while Kerf worked, to go when the turn ends.
    queued: Option<String>,
    /// Stop was asked for and the turn has not ended yet.
    stopping: bool,
    /// The answer just copied, which says so for a moment.
    copied: Option<String>,
    menu: Option<Menu>,
    /// The menu a press outside it closed, and when: that press on its own
    /// chip does not open it again.
    menu_closed: Option<(Menu, Instant)>,
    placeholder: SharedString,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

const PLACEHOLDER: &str = "Ask Kerf to cut, trim, find, or make…";

impl ChatView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextInput::new(cx, PLACEHOLDER)
                .multiline()
                .max_height(px(COMPOSER_MOST))
        });
        let subscriptions = vec![
            cx.observe(&session, |this, _, cx| {
                this.refresh_placeholder(cx);
                cx.notify()
            }),
            cx.subscribe(&session, |this, _, change: &Change, cx| {
                if *change == Change::TurnEnded {
                    this.turn_ended(cx);
                }
                if matches!(
                    change,
                    Change::Conversation(_) | Change::Work | Change::TurnEnded
                ) && this.near_bottom()
                {
                    this.follow = true;
                    cx.notify();
                }
            }),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                if event.0["type"] == "state" {
                    this.book = ProposalBook::read(&event.0["snapshot"]);
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.send(window, cx),
                    // Esc stops Kerf while it works; else leaves the box.
                    InputEvent::Cancel => {
                        if this.session.read(cx).state.work.busy() {
                            this.stop(cx);
                        } else {
                            window.blur();
                        }
                    }
                    InputEvent::Changed => cx.notify(),
                },
            ),
        ];
        let makings = cx.new(|cx| crate::making::Makings::new(session.clone(), window, cx));
        Self {
            session,
            composer,
            makings,
            scroll: ScrollHandle::new(),
            follow: true,
            scripts: HashSet::new(),
            folds: HashMap::new(),
            shown: HashMap::new(),
            book: ProposalBook::default(),
            queued: None,
            stopping: false,
            copied: None,
            menu: None,
            menu_closed: None,
            placeholder: PLACEHOLDER.into(),
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    pub fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.read(cx).focus(window);
    }

    /// The request waiting for Kerf to finish, if one is.
    pub fn queued(&self) -> Option<&str> {
        self.queued.as_deref()
    }

    /// The version id the pinned proposal's card shows: what Accept and
    /// Reject from the keyboard should act on.
    pub fn pinned_proposal(&self, cx: &App) -> Option<String> {
        let session = self.session.read(cx);
        let snapshot = &session.state.snapshot;
        let pinned = pin_order(
            &snapshot.proposals,
            &self.book,
            session.timeline(),
            snapshot.proposal_timeline.as_ref(),
            session.state.position,
        )
        .into_iter()
        .next()?;
        Some(self.shown.get(&pinned).cloned().unwrap_or(pinned))
    }

    fn near_bottom(&self) -> bool {
        let offset = -f32::from(self.scroll.offset().y);
        let most = f32::from(self.scroll.max_offset().height);
        most - offset < 48.0
    }

    /// Why the composer cannot ask now, in its own words; None when it can.
    fn cannot_ask(&self, cx: &App) -> Option<&'static str> {
        let session = self.session.read(cx);
        if !session.has_project() {
            Some("Open a folder to ask Kerf")
        } else if matches!(session.state.connection, Connection::Failed(_)) {
            Some("Connect to ask Kerf")
        } else {
            None
        }
    }

    fn refresh_placeholder(&mut self, cx: &mut Context<Self>) {
        let placeholder: SharedString = self.cannot_ask(cx).unwrap_or(PLACEHOLDER).into();
        if placeholder != self.placeholder {
            self.placeholder = placeholder.clone();
            self.composer
                .update(cx, |composer, cx| composer.set_placeholder(placeholder, cx));
        }
    }

    /// Sends what is written; while Kerf works, it waits for the turn to end,
    /// joining what already waits.
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).text().trim().to_owned();
        if text.is_empty() || self.cannot_ask(cx).is_some() {
            return;
        }
        if self.session.read(cx).state.work.busy() {
            self.queued = Some(join_queued(self.queued.as_deref(), &text));
            self.composer.update(cx, |composer, cx| composer.clear(cx));
            self.focus_composer(window, cx);
            cx.notify();
            return;
        }
        if self.ask(&text, cx) {
            self.composer.update(cx, |composer, cx| composer.clear(cx));
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn ask(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.ask(text);
            cx.notify();
            sent
        });
        if sent {
            self.follow = true;
        }
        sent
    }

    /// Sends `text` again: at once, or after the turn under way.
    fn ask_again(&mut self, text: String, cx: &mut Context<Self>) {
        if self.session.read(cx).state.work.busy() {
            self.queued = Some(join_queued(self.queued.as_deref(), &text));
        } else {
            self.ask(&text, cx);
        }
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.stopping = true;
        self.session.update(cx, |session, _| session.stop());
        cx.notify();
    }

    /// The turn ended: what waited goes now, unless the turn was stopped or
    /// failed, when it goes back in the composer to send or change.
    fn turn_ended(&mut self, cx: &mut Context<Self>) {
        let stopped = std::mem::take(&mut self.stopping);
        let Some(text) = self.queued.take() else {
            cx.notify();
            return;
        };
        let troubled = {
            let session = self.session.read(cx);
            stopped
                || matches!(session.state.work.state.as_str(), "aborted" | "failed")
                || session.state.connection != Connection::Connected
        };
        if !troubled && self.ask(&text, cx) {
            cx.notify();
            return;
        }
        let written = self.composer.read(cx).text().to_owned();
        let back = join_queued(Some(&text), &written);
        self.composer
            .update(cx, |composer, cx| composer.set_text(back, cx));
        cx.notify();
    }

    /// A suggestion fills the composer, to be changed before it is sent.
    fn fill(&mut self, words: String, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.set_text(words, cx));
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn is_open(&self, key: &str) -> bool {
        self.folds
            .get(key)
            .is_some_and(|tween| tween.target() > 0.5)
    }

    fn toggle_fold(&mut self, key: &str, cx: &mut Context<Self>) {
        let open = self.is_open(key);
        self.folds
            .entry(key.to_owned())
            .or_insert_with(|| Tween::at(0.0))
            .go(if open { 0.0 } else { 1.0 }, scene::FADE);
        cx.notify();
    }

    /// A fold's chevron, down when closed and turned over when open.
    fn chevron(&self, key: &str, color: Rgba, window: &Window) -> gpui::Svg {
        let turn = self
            .folds
            .get(key)
            .map_or(0.0, |tween| tween.sample(window));
        Icon::ChevronDown
            .view(12.0, color)
            .with_transformation(Transformation::rotate(radians(std::f32::consts::PI * turn)))
    }

    fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        let closed_by_this_press = self.menu_closed.take().is_some_and(|(closed, at)| {
            closed == menu && at.elapsed() < Duration::from_millis(500)
        });
        if self.menu.as_ref() == Some(&menu) {
            self.menu = None;
        } else if !closed_by_this_press {
            self.menu = Some(menu);
        }
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() {
            self.menu_closed = Some((menu, Instant::now()));
        }
        cx.notify();
    }

    fn copy(&mut self, turn: String, words: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(words.trim().to_owned()));
        self.copied = Some(turn.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(scene::DONE_HOLD).await;
            let _ = this.update(cx, |this, cx| {
                if this.copied.as_deref() == Some(turn.as_str()) {
                    this.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    // ------------------------------------------------------------ turns

    fn turn(
        &self,
        index: usize,
        turns: &[Turn],
        proposals: &[Proposal],
        pinned: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let turn = &turns[index];
        let id = SharedString::from(format!("turn-{}", turn.id));
        let body = match turn.kind {
            Speaker::You => self.you(&turn.text, turn.timecode, &turn.id, cx),
            Speaker::Kerf => {
                let mut column = div().flex().flex_col().gap(px(10.0));
                if !turn.steps.is_empty() || turn.seconds.is_some() {
                    column = column.child(self.worked(turn, window, cx));
                }
                let troubled = turn.error || turn.stopped;
                if turn.error {
                    column = column.child(ui::label(
                        "Couldn't finish",
                        text::META_STRONG,
                        theme::neg(),
                    ));
                } else if turn.stopped {
                    column = column.child(ui::label("Stopped", text::META_STRONG, theme::ink3()));
                }
                if !turn.text.trim().is_empty() {
                    let ink = if troubled {
                        theme::ink2()
                    } else {
                        theme::ink()
                    };
                    column = column.child(self.prose(&turn.text, &turn.id, ink, cx));
                }
                if let Some(proposal) = turn
                    .proposal
                    .as_deref()
                    .and_then(|id| self.book.find(proposals, id))
                {
                    let pinned = pinned == Some(proposal.proposal_id.as_str());
                    column = column.child(self.proposal_card(proposal, pinned, window, cx));
                }
                let again = troubled
                    .then(|| request_before(turns, index))
                    .flatten()
                    .map(str::to_owned);
                let copy = (!turn.text.trim().is_empty()).then(|| turn.text.clone());
                if again.is_some() || copy.is_some() {
                    column = column.child(self.turn_actions(&turn.id, copy, again, cx));
                }
                column
            }
        };
        body.with_animation(id, motion::animation(scene::APPEAR), |turn, t| {
            turn.opacity(t).mt(px(scene::APPEAR_RISE * (1.0 - t)))
        })
        .into_any_element()
    }

    /// Under an answer: Copy, and Ask again where it failed or was stopped.
    fn turn_actions(
        &self,
        turn: &str,
        copy: Option<String>,
        again: Option<String>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let copied = self.copied.as_deref() == Some(turn);
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .when_some(copy, |row, words| {
                let turn = turn.to_owned();
                row.child(
                    link(
                        ElementId::Name(format!("copy-{turn}").into()),
                        if copied { "Copied" } else { "Copy" },
                        copied,
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.copy(turn.clone(), words.clone(), cx)
                        }),
                    ),
                )
            })
            .when_some(again, |row, request| {
                row.child(
                    link(
                        ElementId::Name(format!("again-{turn}").into()),
                        "Ask again",
                        false,
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.ask_again(request.clone(), cx)),
                    ),
                )
            })
    }

    /// Your request, set off on the right, with the playhead time it was
    /// asked at over it, which goes there.
    fn you(
        &self,
        words: &str,
        timecode: Option<f64>,
        key: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let fps = self.session.read(cx).timeline().rate();
        let session = self.session.clone();
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(theme::space::S4))
            .when_some(timecode.filter(|at| at.is_finite()), |column, at| {
                column.child(
                    theme::type_style(div(), text::MONO_SMALL)
                        .id(ElementId::Name(format!("stamp-{key}").into()))
                        .flex()
                        .items_center()
                        .h(px(20.0))
                        .px(px(theme::space::S6))
                        .rounded(px(theme::radius::SMALL))
                        .bg(theme::raised())
                        .text_color(theme::accent_ink())
                        .cursor_pointer()
                        .hover(|chip| chip.bg(theme::hover2()))
                        .child(model::timecode(at, fps))
                        .on_click(move |_, window, cx| {
                            session.update(cx, |session, _| session.seek(at, false));
                            window.dispatch_action(Box::new(RevealPicture), cx);
                        }),
                )
            })
            .child(
                theme::reading(div())
                    .max_w(gpui::relative(0.8))
                    .px(px(14.0))
                    .py(px(9.0))
                    .rounded(px(theme::radius::PANEL))
                    .bg(theme::raised())
                    .text_color(theme::ink())
                    .child(words.to_owned()),
            )
    }

    /// "Worked for …", which unfolds the steps the turn took.
    fn worked(&self, turn: &Turn, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let key = format!("steps-{}", turn.id);
        let open = self.is_open(&key);
        let headline = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(Icon::Clock.view(12.0, theme::ink3()))
            .child(ui::label(
                worked_headline(turn.seconds, turn.steps.len()),
                text::META,
                theme::ink3(),
            ));
        let mut column = div().flex().flex_col().gap(px(theme::space::S8));
        if turn.steps.is_empty() {
            return column.child(headline);
        }
        let toggle = key.clone();
        column = column.child(
            headline
                .id(ElementId::Name(key.clone().into()))
                .cursor_pointer()
                .child(self.chevron(&key, theme::ink3(), window))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_fold(&toggle, cx))),
        );
        if open {
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .pl(px(10.0))
                    .border_l_1()
                    .border_color(theme::line2())
                    .children(turn.steps.iter().map(|step| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(Icon::Check.view(12.0, theme::ink3()))
                            .child(ui::line(step.clone(), text::CONTROL, theme::ink2()))
                    }))
                    .with_animation(
                        ElementId::Name(format!("{key}-open").into()),
                        motion::animation(scene::FADE),
                        |steps, t| steps.opacity(t),
                    ),
            );
        }
        column
    }

    /// What Kerf said, set for reading: paragraphs (their line breaks kept),
    /// lists, headings and code, with bold, code and timecode links inline.
    fn prose(&self, words: &str, key: &str, ink: Rgba, cx: &App) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(8.0));
        let mut code: Option<Vec<String>> = None;
        let mut paragraph: Vec<String> = Vec::new();
        let mut pieces = 0usize;
        let flush = |paragraph: &mut Vec<String>, column: gpui::Div, pieces: &mut usize| {
            if paragraph.is_empty() {
                return column;
            }
            let text = paragraph.join("\n");
            paragraph.clear();
            *pieces += 1;
            column.child(self.rich(format!("{key}-{pieces}"), &text, ink, cx))
        };
        for line in words.lines() {
            if line.trim_start().starts_with("```") {
                match code.take() {
                    Some(lines) => column = column.child(code_block(lines)),
                    None => {
                        column = flush(&mut paragraph, column, &mut pieces);
                        code = Some(Vec::new());
                    }
                }
                continue;
            }
            if let Some(lines) = code.as_mut() {
                lines.push(line.to_owned());
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                column = flush(&mut paragraph, column, &mut pieces);
            } else if let Some(item) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
            {
                column = flush(&mut paragraph, column, &mut pieces);
                pieces += 1;
                column = column.child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(theme::reading(div()).text_color(theme::ink3()).child("•"))
                        .child(div().flex_1().min_w_0().child(self.rich(
                            format!("{key}-{pieces}"),
                            item,
                            ink,
                            cx,
                        ))),
                );
            } else if let Some(heading) = trimmed.strip_prefix('#') {
                column = flush(&mut paragraph, column, &mut pieces);
                let words: String = spans(heading.trim_start_matches('#').trim())
                    .into_iter()
                    .map(|span| match span {
                        Span::Plain(words)
                        | Span::Bold(words)
                        | Span::Code(words)
                        | Span::Timecode(words) => words,
                    })
                    .collect();
                column = column.child(ui::label(words, text::TITLE, theme::ink()));
            } else {
                paragraph.push(trimmed.to_owned());
            }
        }
        if let Some(lines) = code {
            column = column.child(code_block(lines));
        }
        flush(&mut paragraph, column, &mut pieces)
    }

    /// A paragraph of what Kerf wrote: bold in the medium face, code in mono,
    /// and each timecode a mono link in accent ink that moves the playhead
    /// there and brings the picture in.
    fn rich(&self, id: String, words: &str, ink: Rgba, cx: &App) -> AnyElement {
        let session = self.session.read(cx);
        let fps = session.timeline().rate();
        let plain = theme::prose_font(FontWeight::NORMAL);
        let strong = theme::prose_font(FontWeight::MEDIUM);
        let mono = theme::mono_font(FontWeight::NORMAL);
        let mut shown = String::new();
        let mut runs = Vec::new();
        let mut links = Vec::new();
        let mut moments = Vec::new();
        for span in spans(words) {
            let (words, font, color, ground) = match &span {
                Span::Plain(words) => (words, plain.clone(), ink, None),
                Span::Bold(words) => (words, strong.clone(), ink, None),
                Span::Code(words) => (words, mono.clone(), theme::ink2(), Some(theme::well())),
                Span::Timecode(words) => (words, mono.clone(), theme::accent_ink(), None),
            };
            if words.is_empty() {
                continue;
            }
            let start = shown.len();
            shown.push_str(words);
            if let Span::Timecode(timecode) = &span {
                if let Some(seconds) = timecode_seconds(timecode, fps) {
                    links.push(start..shown.len());
                    moments.push(seconds);
                }
            }
            runs.push(TextRun {
                len: words.len(),
                font,
                color: color.into(),
                background_color: ground.map(Into::into),
                underline: None,
                strikethrough: None,
            });
        }
        let styled = StyledText::new(shown).with_runs(runs);
        let paragraph = theme::reading(div()).text_color(ink);
        if links.is_empty() {
            return paragraph.child(styled).into_any_element();
        }
        // The paragraph takes the press, down and up on one frame, and finds
        // the link under it in the words as they were laid out; the words
        // themselves only show the hand over a link.
        let laid_out = styled.layout().clone();
        let session = self.session.clone();
        let ranges = links.clone();
        paragraph
            .id(ElementId::Name(format!("{id}-press").into()))
            .child(
                InteractiveText::new(ElementId::Name(id.into()), styled)
                    .on_click(links, |_, _, _| {}),
            )
            .on_click(move |event, window, cx| {
                let Ok(index) = laid_out.index_for_position(event.position()) else {
                    return;
                };
                let Some(at) = ranges
                    .iter()
                    .position(|range| range.contains(&index))
                    .and_then(|link| moments.get(link).copied())
                else {
                    return;
                };
                session.update(cx, |session, cx| {
                    session.seek(at, false);
                    cx.notify();
                });
                window.dispatch_action(Box::new(RevealPicture), cx);
            })
            .into_any_element()
    }

    // ------------------------------------------------------------ proposals

    fn proposal_card(
        &self,
        proposal: &Proposal,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = proposal.proposal_id.clone();
        let session = self.session.read(cx);
        let fps = session.timeline().rate();
        let connected = session.state.connection == Connection::Connected;
        if !proposal.is_pending() {
            // Settled, it keeps to one line.
            let accepted = proposal.status == "accepted";
            let words = match (accepted, self.book.applied.get(&id)) {
                (true, Some(revision)) => format!("Accepted in rev {revision}"),
                (true, None) => "Accepted".to_owned(),
                (false, _) => "Rejected".to_owned(),
            };
            let (icon, color) = if accepted {
                (Icon::Check, theme::accent_ink())
            } else {
                (Icon::Close, theme::ink2())
            };
            return div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(icon.view(12.0, color))
                .child(ui::line(
                    script_summary(&proposal.script, proposal.version),
                    text::CONTROL,
                    theme::ink2(),
                ))
                .child(ui::label("·", text::CONTROL, theme::ink3()))
                .child(ui::label(words, text::CONTROL, color))
                .into_any_element();
        }
        let versions = self.book.versions_of(proposal);
        let shown_id = self
            .shown
            .get(&id)
            .filter(|shown| {
                versions
                    .iter()
                    .any(|version| &version.proposal_id == *shown)
            })
            .cloned()
            .unwrap_or_else(|| id.clone());
        let version = versions
            .iter()
            .find(|version| version.proposal_id == shown_id)
            .cloned()
            .unwrap_or_default();
        let stale = proposal.is_stale();
        // Its own timeline, else the one the snapshot drew for the first open one.
        let timeline = self.book.timelines.get(&shown_id).or_else(|| {
            (session
                .state
                .snapshot
                .pending()
                .map(|first| first.proposal_id.as_str())
                == Some(shown_id.as_str()))
            .then_some(session.state.snapshot.proposal_timeline.as_ref())
            .flatten()
        });
        let changes = timeline
            .map(|timeline| clip_changes(session.timeline(), timeline))
            .unwrap_or_default();
        let rows: Vec<(String, String, String)> = if changes.is_empty() {
            // Without its timeline, the script's lines say what it does.
            version
                .script
                .lines()
                .map(str::trim)
                .filter(|line| {
                    !line.is_empty() && !line.starts_with('#') && !line.starts_with("use ")
                })
                .map(|line| (line.to_owned(), String::new(), String::new()))
                .collect()
        } else {
            changes.iter().map(|change| change.row(fps)).collect()
        };
        // The changed lines of the document, else the script.
        let mut script: Vec<String> = version
            .document_diff
            .lines()
            .filter(|line| !line.starts_with("+++") && !line.starts_with("---"))
            .filter(|line| line.starts_with('+') || line.starts_with('-'))
            .map(str::to_owned)
            .collect();
        if script.is_empty() {
            script = version
                .script
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_owned)
                .collect();
        }
        let preview_shown = session.state.preview.proposal.as_deref() == Some(shown_id.as_str())
            && !matches!(session.state.preview.status.as_str(), "exited" | "stopped");
        // Only what it changes, with a little either side; the bridge renders
        // at most two minutes of a preview.
        let watch = timeline
            .filter(|_| !stale)
            .and_then(|proposed| model::changed_span(session.timeline(), proposed, 1.0, 120.0));
        let changes_key = format!("changes-{id}");
        let changes_open = self.is_open(&changes_key);
        let script_open = self.scripts.contains(&id);
        let count = rows.len();

        let mut card = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(14.0))
            .rounded(px(theme::radius::CARD))
            .bg(theme::raised())
            .border_1()
            .border_color(if stale {
                theme::line()
            } else {
                theme::with_alpha(theme::pos(), 0.45)
            });
        if pinned {
            card = card
                .child(
                    div()
                        .absolute()
                        .left(px(-1.0))
                        .top(px(12.0))
                        .bottom(px(12.0))
                        .w(px(2.0))
                        .rounded_full()
                        .bg(theme::accent()),
                )
                .child(ui::label("Pinned proposal", text::META, theme::ink3()));
        }
        card = card.child(
            div()
                .flex()
                .items_start()
                .gap(px(8.0))
                .child(div().pt(px(2.0)).child(Icon::Film.view(16.0, theme::pos())))
                .child(
                    ui::label(
                        script_summary(&version.script, version.version),
                        text::BODY_NAME,
                        theme::ink(),
                    )
                    .flex_1()
                    .min_w_0(),
                )
                .when(stale, |row| {
                    row.child(ui::chip("Out of date", theme::warn()))
                }),
        );
        // N changes, Script, and the versions.
        let toggle_changes = changes_key.clone();
        let toggle_script = id.clone();
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .when(count > 0, |row| {
                    row.child(
                        div()
                            .id(ElementId::Name(format!("changes-toggle-{id}").into()))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .cursor_pointer()
                            .child(ui::label(
                                if count == 1 {
                                    "1 change".to_owned()
                                } else {
                                    format!("{count} changes")
                                },
                                text::CONTROL,
                                theme::accent_ink(),
                            ))
                            .child(self.chevron(&changes_key, theme::accent_ink(), window))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_fold(&toggle_changes, cx)
                            })),
                    )
                })
                .when(!script.is_empty(), |row| {
                    row.child(
                        link(
                            ElementId::Name(format!("script-{id}").into()),
                            "Script",
                            script_open,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.scripts.remove(&toggle_script) {
                                this.scripts.insert(toggle_script.clone());
                            }
                            cx.notify();
                        })),
                    )
                })
                .child(div().flex_1())
                .when(versions.len() > 1, |row| {
                    row.child(self.versions_button(&id, &shown_id, &versions, fps, cx))
                }),
        );
        if changes_open && count > 0 {
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .children(rows.into_iter().map(|(name, target, shift)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::space::S12))
                            .h(px(24.0))
                            .child(
                                ui::line(name, text::CONTROL, theme::ink())
                                    .flex_1()
                                    .min_w_0(),
                            )
                            .when(!target.is_empty(), |row| {
                                row.child(
                                    ui::line(target, text::TIMECODE, theme::ink2()).flex_none(),
                                )
                            })
                            .child(div().w(px(40.0)).flex().justify_end().child(ui::line(
                                shift,
                                text::MONO_SMALL,
                                theme::ink3(),
                            )))
                    }))
                    .with_animation(
                        ElementId::Name(format!("{changes_key}-open").into()),
                        motion::animation(scene::FADE),
                        |rows, t| rows.opacity(t),
                    ),
            );
        }
        if script_open && !script.is_empty() {
            card = card.child(diff(&script, &id));
        }
        // What to do with it.
        let accept = shown_id.clone();
        let reject = id.clone();
        let preview = shown_id.clone();
        let forward = id.clone();
        card = card.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .when(!stale, |row| {
                    let button =
                        ui::button(ElementId::Name(format!("accept-{id}").into()), "Accept")
                            .small()
                            .icon(Icon::Check)
                            .disabled(!connected)
                            .on_click({
                                let session = self.session.clone();
                                move |_, _, cx| {
                                    session.update(cx, |session, _| session.accept(&accept))
                                }
                            });
                    row.child(if pinned { button.primary() } else { button })
                })
                .when(stale, |row| {
                    let button = ui::button(
                        ElementId::Name(format!("forward-{id}").into()),
                        "Bring forward",
                    )
                    .small()
                    .disabled(!connected)
                    .on_click({
                        let session = self.session.clone();
                        move |_, _, cx| session.update(cx, |session, _| session.forward(&forward))
                    });
                    row.child(if pinned { button.primary() } else { button })
                })
                .when_some(watch, |row, (start, end)| {
                    let session = self.session.clone();
                    let proposal = shown_id.clone();
                    row.child(
                        ui::button(
                            ElementId::Name(format!("watch-{id}").into()),
                            "Watch the change",
                        )
                        .small()
                        .icon(Icon::Eye)
                        .disabled(!connected)
                        .on_click(move |_, window, cx| {
                            session.update(cx, |session, _| {
                                session.send(serde_json::json!({
                                    "type": "preview", "proposalId": proposal,
                                    "start": start, "duration": end - start,
                                }));
                                session.seek(start, false);
                            });
                            window.dispatch_action(Box::new(RevealPicture), cx);
                        }),
                    )
                })
                .child(
                    ui::button(
                        ElementId::Name(format!("preview-{id}").into()),
                        if preview_shown {
                            "Previewing"
                        } else {
                            "Preview"
                        },
                    )
                    .small()
                    .icon(if preview_shown {
                        Icon::Check
                    } else {
                        Icon::Play
                    })
                    .disabled(!connected)
                    .on_click({
                        let session = self.session.clone();
                        move |_, window, cx| {
                            // On, it shows what the proposal makes; off, the program again.
                            session.update(cx, |session, _| {
                                if preview_shown {
                                    session.send(serde_json::json!({"type": "preview_exit"}));
                                } else {
                                    session.preview(Some(&preview));
                                }
                            });
                            if !preview_shown {
                                window.dispatch_action(Box::new(RevealPicture), cx);
                            }
                        }
                    }),
                )
                .child(
                    ui::button(ElementId::Name(format!("reject-{id}").into()), "Reject")
                        .small()
                        .ghost()
                        .disabled(!connected)
                        .on_click({
                            let session = self.session.clone();
                            move |_, _, cx| session.update(cx, |session, _| session.reject(&reject))
                        }),
                ),
        );
        card.into_any_element()
    }

    /// "v2 ⌄": the versions of a proposal, each with where its first move
    /// lands; choosing one shows it and previews it.
    fn versions_button(
        &self,
        id: &str,
        shown_id: &str,
        versions: &[Version],
        fps: f64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let menu = Menu::Versions(id.to_owned());
        let open = self.menu.as_ref() == Some(&menu);
        let shown = versions
            .iter()
            .find(|version| version.proposal_id == shown_id)
            .map_or(1, |version| version.version);
        let program = self.session.read(cx).timeline().clone();
        let toggle = menu.clone();
        let button = theme::type_style(div(), text::MONO_SMALL)
            .id(ElementId::Name(format!("versions-{id}").into()))
            .flex()
            .items_center()
            .gap(px(2.0))
            .h(px(20.0))
            .pl(px(6.0))
            .pr(px(2.0))
            .rounded(px(theme::radius::SMALL))
            .text_color(if open { theme::ink() } else { theme::ink3() })
            .cursor_pointer()
            .hover(|button| button.bg(theme::hover2()).text_color(theme::ink()))
            .child(format!("v{shown}"))
            .child(Icon::ChevronDown.view(14.0, theme::ink3()))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_menu(toggle.clone(), cx)));
        let mut wrapper = div().relative().flex_none().child(button);
        if open {
            let items = versions.iter().map(|version| {
                let label = self
                    .book
                    .timelines
                    .get(&version.proposal_id)
                    .and_then(|timeline| {
                        clip_changes(&program, timeline)
                            .into_iter()
                            .find(|change| change.moved)
                    })
                    .map(|change| {
                        let shift = ((change.new_start - change.old_start) * fps).round() as i64;
                        let shift = signed_frames(shift);
                        format!(
                            "v{} {} · {} fr",
                            version.version,
                            model::timecode(change.new_start, fps),
                            shift.trim_end_matches('f')
                        )
                    })
                    .unwrap_or_else(|| format!("v{}", version.version));
                let proposal = id.to_owned();
                let chosen = version.proposal_id.clone();
                menu_item(
                    ElementId::Name(format!("version-{}", version.proposal_id).into()),
                    label,
                    None,
                    version.proposal_id == shown_id,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if chosen == proposal {
                        this.shown.remove(&proposal);
                    } else {
                        this.shown.insert(proposal.clone(), chosen.clone());
                    }
                    this.menu = None;
                    let chosen = chosen.clone();
                    this.session
                        .update(cx, |session, _| session.preview(Some(&chosen)));
                    window.dispatch_action(Box::new(RevealPicture), cx);
                    cx.notify();
                }))
            });
            let body = menu_frame(px(260.0), cx).children(items);
            wrapper = wrapper.child(
                div().absolute().top_full().right_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopRight)
                            .offset(point(px(0.0), px(theme::space::S4)))
                            .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                            .child(fade_in(body, format!("versions-menu-{id}"))),
                    )
                    .with_priority(1),
                ),
            );
        }
        wrapper.into_any_element()
    }

    // ------------------------------------------------------------ while Kerf works

    /// What Kerf is doing now, under your request.
    fn working(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        let work = &session.state.work;
        let started = work.started?;
        let elapsed = started.elapsed().as_secs();
        let clock = format!("{}:{:02}", elapsed / 60, elapsed % 60);
        // The clock goes on once a second; the light 30 times a second while it shows.
        if motion::reduced() {
            let next = scene::CLOCK_FRAME;
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(next).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        } else {
            window.request_animation_frame();
        }
        let phase = motion::phase(started, scene::WORK_SHIMMER);
        let status = if self.stopping {
            "Stopping…".to_owned()
        } else {
            work.status.clone()
        };
        let mut label =
            ui::label(SharedString::from(status), text::BODY_NAME, theme::ink2()).relative();
        if let Some(at) = motion::shimmer_at(phase, 1.0).filter(|_| !motion::reduced()) {
            label = label.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(gpui::relative(at - 0.2))
                    .w(gpui::relative(0.4))
                    .bg(linear_gradient(
                        90.0,
                        linear_color_stop(theme::with_alpha(theme::ink(), 0.0), 0.0),
                        linear_color_stop(theme::with_alpha(theme::ink(), 0.28), 0.5),
                    )),
            );
        }
        let steps = work
            .steps
            .iter()
            .rev()
            .take(4)
            .rev()
            .cloned()
            .collect::<Vec<_>>();
        let answer = work.answer.clone();
        let mut column = div().flex().flex_col().gap(px(8.0)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(label)
                .child(ui::label(clock, text::TIMECODE, theme::ink3())),
        );
        if !steps.is_empty() {
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .pl(px(10.0))
                    .border_l_1()
                    .border_color(theme::line2())
                    .children(steps.into_iter().map(|step| {
                        let (icon, color) = match step.state {
                            StepState::Running => (Icon::Dot, theme::activity::kerf()),
                            StepState::Done => (Icon::Check, theme::ink3()),
                            StepState::Failed => (Icon::Close, theme::neg()),
                        };
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(icon.view(12.0, color))
                            .child(ui::line(step.text, text::CONTROL, theme::ink2()))
                    })),
            );
        }
        if !answer.trim().is_empty() {
            column = column.child(self.prose(&answer, "live", theme::ink(), cx));
        }
        Some(column.into_any_element())
    }

    fn empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let has_cut = session.timeline().has_clips();
        let suggestions: &[&str] = if has_cut {
            &[
                "Tighten the cut",
                "Find the best take of each line",
                "Add captions",
            ]
        } else {
            &[
                "Make a first cut from my footage",
                "Find the strongest moments",
                "Cut a 30-second short",
            ]
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(18.0))
            .pt(px(120.0))
            .child(ui::label("What are we making?", text::DISPLAY, theme::ink()))
            .child(ui::label(
                "Kerf cuts in the project with you. Ask, and look at what it proposes before it is yours.",
                text::BODY,
                theme::ink3(),
            ))
            .child(
                div().flex().flex_wrap().justify_center().gap(px(8.0)).children(suggestions.iter().enumerate().map(
                    |(index, words)| {
                        let words = (*words).to_owned();
                        ui::button(ElementId::Name(format!("suggest-{index}").into()), words.clone())
                            .on_click(cx.listener(move |this, _, window, cx| this.fill(words.clone(), window, cx)))
                    },
                )),
            )
            .with_animation("empty-rise", motion::animation(scene::RISE), |empty, t| {
                empty.opacity(t).mt(px(scene::PAGE_RISE * (1.0 - t)))
            })
            .into_any_element()
    }

    // ------------------------------------------------------------ the composer

    /// A chip in the composer's row that opens its menu over it.
    fn menu_chip(
        &self,
        menu: Menu,
        id: &'static str,
        words: String,
        tip: String,
        body: impl FnOnce(&Self, &Context<Self>) -> AnyElement,
        cx: &Context<Self>,
    ) -> AnyElement {
        let open = self.menu.as_ref() == Some(&menu);
        let toggle = menu.clone();
        let chip = ui::chip(words, if open { theme::ink() } else { theme::ink3() })
            .id(id)
            .pr(px(4.0))
            .cursor_pointer()
            .hover(|chip| chip.bg(theme::hover2()).text_color(theme::ink2()))
            .child(Icon::ChevronDown.view(12.0, theme::ink3()))
            .when(!open, |chip| {
                chip.tooltip(move |_, cx| cx.new(|_| ui::Tooltip(tip.clone().into())).into())
            })
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_menu(toggle.clone(), cx)));
        let mut wrapper = div().relative().flex_none().child(chip);
        if open {
            let body = body(self, cx);
            wrapper = wrapper.child(
                // Over the chip, its left edge on the chip's, over everything.
                div().absolute().bottom_full().left_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::BottomLeft)
                            .offset(point(px(0.0), px(-theme::space::S4)))
                            .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                            .child(fade_in(body, format!("{id}-menu"))),
                    )
                    .with_priority(1),
                ),
            );
        }
        wrapper.into_any_element()
    }

    /// "What Kerf does without asking": the three modes, each with its line.
    fn permissions_menu(&self, cx: &Context<Self>) -> AnyElement {
        let current = self.session.read(cx).prefs.permissions;
        menu_frame(px(280.0), cx)
            .child(menu_header("What Kerf does without asking"))
            .children(Permissions::ALL.iter().map(|permissions| {
                let permissions = *permissions;
                let (label, line) = permission_words(permissions);
                menu_item(
                    ElementId::Name(format!("permissions-{}", permissions.wire()).into()),
                    label.to_owned(),
                    Some(line),
                    permissions == current,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.menu = None;
                    this.session.update(cx, |session, cx| {
                        session.prefs.permissions = permissions;
                        session.save_prefs();
                        cx.notify();
                    });
                    cx.notify();
                }))
            }))
            .into_any_element()
    }

    /// The models that can be chosen, Agent settings…, and whether a request
    /// carries the playhead time.
    fn model_menu(&self, cx: &Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let provider = session.prefs.provider;
        let configured = session.prefs.model().to_owned();
        let choices = model_choices(provider, &configured);
        let current = if configured.is_empty() {
            "default".to_owned()
        } else {
            configured
        };
        let stamped = session.prefs.send_playhead;
        menu_frame(px(240.0), cx)
            .child(menu_header(provider.label()))
            .children(choices.into_iter().map(|model| {
                let chosen = model == current;
                // Only Claude Code's aliases are chosen here; a key's model
                // is the one typed in Settings.
                menu_item(
                    ElementId::Name(format!("model-{model}").into()),
                    model_label(&model),
                    None,
                    chosen,
                )
            }))
            .child(div().my(px(4.0)).child(ui::hairline(true)))
            .child(
                menu_item("agent-settings", "Agent settings…".to_owned(), None, false).on_click(
                    cx.listener(|this, _, window, cx| {
                        this.menu = None;
                        window.dispatch_action(Box::new(RunCommand::new("app.preferences")), cx);
                        cx.notify();
                    }),
                ),
            )
            .child(
                menu_item(
                    "send-playhead",
                    "Send playhead time".to_owned(),
                    Some("A request says where the playhead is"),
                    stamped,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.session.update(cx, |session, cx| {
                        session.prefs.send_playhead = !session.prefs.send_playhead;
                        session.save_prefs();
                        cx.notify();
                    });
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    fn composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let busy = session.state.work.busy();
        let empty = self.composer.read(cx).text().trim().is_empty();
        let refused = self.cannot_ask(cx).is_some();
        let provider = session.prefs.provider;
        let model = session.prefs.model().to_owned();
        let permissions = session.prefs.permissions;
        let context = session.state.context;
        let next = session.state.next_steps.clone();
        let focus = session
            .selection
            .first()
            .and_then(|id| session.timeline().clip(id))
            .map(|(_, clip)| clip.label().to_owned());
        let model_words = if provider == Provider::ClaudeCode || model.is_empty() {
            provider.label().to_owned()
        } else {
            format!("{} · {}", provider.label(), model)
        };
        let model_tip = format!("Answers come from {model_words}");
        let (mode_words, mode_line) = permission_words(permissions);
        let mode_tip = format!("{mode_words} · {mode_line}");
        let queued = self.queued.clone();
        let stopping = self.stopping;
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .when(!next.is_empty() && !busy, |column| {
                column.child(div().flex().flex_wrap().gap(px(6.0)).children(
                    next.into_iter().enumerate().map(|(index, step)| {
                        ui::button(
                            ElementId::Name(format!("next-{index}").into()),
                            step.clone(),
                        )
                        .small()
                        .icon(Icon::Sparkle)
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.fill(step.clone(), window, cx),
                        ))
                    }),
                ))
            })
            // What waits for Kerf to finish: "Next: …" and Cancel, over the box.
            .when_some(queued, |column, queued| {
                let line = queued.split_whitespace().collect::<Vec<_>>().join(" ");
                column.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .pl(px(4.0))
                        .child(Icon::Clock.view(12.0, theme::ink3()))
                        .child(
                            ui::line(format!("Next: {line}"), text::META, theme::ink2())
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(
                            ui::button("queued-cancel", "Cancel")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.queued = None;
                                    cx.notify();
                                })),
                        )
                        .with_animation("queued", motion::animation(scene::APPEAR), |row, t| {
                            row.opacity(t)
                        }),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .rounded(px(theme::radius::CARD + 4.0))
                    .bg(theme::raised())
                    .border_1()
                    .border_color(theme::line2())
                    .when_some(focus, |box_, clip| {
                        box_.child(ui::chip(format!("@{clip}"), theme::accent_ink()))
                    })
                    .child(self.composer.clone())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(self.menu_chip(
                                Menu::Model,
                                "model-chip",
                                model_words,
                                model_tip,
                                |this, cx| this.model_menu(cx),
                                cx,
                            ))
                            .child(self.menu_chip(
                                Menu::Permissions,
                                "permissions-chip",
                                mode_words.to_owned(),
                                mode_tip,
                                |this, cx| this.permissions_menu(cx),
                                cx,
                            ))
                            .when_some(context, |row, (used, limit)| {
                                let share = if limit > 0.0 {
                                    (used / limit).clamp(0.0, 1.0)
                                } else {
                                    0.0
                                };
                                let color = if share > 0.85 {
                                    theme::warn()
                                } else {
                                    theme::ink3()
                                };
                                row.child(ui::label(
                                    format!("{:.0}% context", share * 100.0),
                                    text::META,
                                    color,
                                ))
                            })
                            .child(div().flex_1())
                            .child(if busy {
                                icon_button("stop", Icon::Stop)
                                    .tooltip(if stopping { "Stopping…" } else { "Stop Kerf" })
                                    .color(theme::ink())
                                    .disabled(stopping)
                                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                                    .into_any_element()
                            } else {
                                let live = !empty && !refused;
                                div()
                                    .id("send")
                                    .size(px(28.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(if live { theme::accent() } else { theme::on() })
                                    .when(live, |send| {
                                        send.cursor_pointer()
                                            .hover(|send| send.bg(theme::accent_hover()))
                                    })
                                    .child(Icon::Send.view(
                                        16.0,
                                        if live {
                                            theme::accent_text()
                                        } else {
                                            theme::dis()
                                        },
                                    ))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.send(window, cx)),
                                    )
                                    .into_any_element()
                            }),
                    ),
            )
            .into_any_element()
    }
}

/// Words that do something when pressed: a fold, Copy, Ask again. `on` is
/// held in ink.
fn link(id: ElementId, words: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    ui::label(
        words,
        text::CONTROL,
        if on { theme::ink() } else { theme::ink3() },
    )
    .id(id)
    .cursor_pointer()
    .hover(|link| link.text_color(theme::ink()))
}

/// A menu's box: a step up from what it opens over; a press outside closes
/// it at once.
fn menu_frame(width: gpui::Pixels, cx: &Context<ChatView>) -> gpui::Div {
    div()
        .w(width)
        .p(px(4.0))
        .flex()
        .flex_col()
        .rounded(px(theme::radius::CARD))
        .bg(theme::popup())
        .border_1()
        .border_color(theme::line2())
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_menu(cx)))
}

/// What opens from a press, fading in where it is over `FADE`.
fn fade_in(body: impl IntoElement, id: String) -> impl IntoElement {
    div().child(body).with_animation(
        ElementId::Name(id.into()),
        motion::animation(scene::FADE),
        |menu, t| menu.opacity(t),
    )
}

fn menu_header(words: &'static str) -> gpui::Div {
    ui::label(words, text::META, theme::ink3())
        .px(px(10.0))
        .pt(px(6.0))
        .pb(px(4.0))
}

/// A row of a menu: a check where it is the one chosen, its words, and a
/// line under them when it has one.
fn menu_item(
    id: impl Into<ElementId>,
    words: String,
    line: Option<&'static str>,
    checked: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .flex()
        .items_start()
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(5.0))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .hover(|item| item.bg(theme::popup_hover()))
        .child(div().pt(px(3.0)).child(if checked {
            Icon::Check.view(14.0, theme::ink()).into_any_element()
        } else {
            crate::icons::spacer(14.0).into_any_element()
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .child(ui::label(words, text::MENU_ITEM, theme::ink()))
                .when_some(line, |column, line| {
                    column.child(ui::label(line, text::META, theme::ink3()))
                }),
        )
}

fn code_block(lines: Vec<String>) -> gpui::Div {
    theme::type_style(div(), text::TIMECODE)
        .p(px(10.0))
        .rounded(px(theme::radius::CONTROL))
        .bg(theme::well())
        .text_color(theme::ink2())
        .child(lines.join("\n"))
}

/// A proposal's changed lines, its additions sage and removals brick.
fn diff(lines: &[String], id: &str) -> impl IntoElement {
    div()
        .id(ElementId::Name(format!("diff-{id}").into()))
        .max_h(px(260.0))
        .overflow_y_scroll()
        .p(px(8.0))
        .rounded(px(theme::radius::CONTROL))
        .bg(theme::well())
        .children(lines.iter().take(400).map(|line| {
            let color = if line.starts_with('+') {
                theme::pos()
            } else if line.starts_with('-') {
                theme::neg()
            } else if line.starts_with("@@") {
                theme::accent_ink()
            } else {
                theme::ink2()
            };
            ui::line(line.clone(), text::MONO_SMALL, color)
        }))
}

impl Focusable for ChatView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let turns = session
            .state
            .open_conversation
            .as_ref()
            .map(|c| c.items.clone())
            .unwrap_or_default();
        let proposals = session.state.snapshot.proposals.clone();
        let request = session.state.work.request.clone();
        let busy = session.state.work.busy();
        let showing_this = session.conversation.is_none()
            || session.state.work.conversation == session.conversation;
        let pins = pin_order(
            &proposals,
            &self.book,
            session.timeline(),
            session.state.snapshot.proposal_timeline.as_ref(),
            session.state.position,
        );
        let pinned = pins.first().cloned();
        // An open proposal no turn shows yet shows at the end, the pinned first.
        let attached: HashSet<String> = turns
            .iter()
            .filter_map(|turn| turn.proposal.as_deref())
            .filter_map(|id| self.book.find(&proposals, id))
            .map(|proposal| proposal.proposal_id.clone())
            .collect();
        let loose: Vec<Proposal> = pins
            .iter()
            .filter(|id| !attached.contains(*id))
            .filter_map(|id| {
                proposals
                    .iter()
                    .find(|proposal| &proposal.proposal_id == id)
            })
            .cloned()
            .collect();
        let nothing = turns.is_empty() && request.is_none() && !busy && loose.is_empty();

        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(22.0))
            .pt(px(24.0))
            .pb(px(24.0));
        if nothing {
            list = list.child(self.empty(cx));
        }
        for index in 0..turns.len() {
            list = list.child(self.turn(index, &turns, &proposals, pinned.as_deref(), window, cx));
        }
        if let Some(request) = request.filter(|_| showing_this) {
            list = list.child(self.you(&request, None, "live", cx).with_animation(
                "request",
                motion::animation(scene::APPEAR),
                |turn, t| turn.opacity(t),
            ));
        }
        if busy && showing_this {
            if let Some(working) = self.working(window, cx) {
                list = list.child(working);
            }
        }
        if !loose.is_empty() {
            // While a turn runs, what it stages has no turn yet to sit under.
            if !busy && !turns.is_empty() {
                let note = if loose.len() == 1 {
                    "Proposed outside this conversation.".to_owned()
                } else {
                    format!("{} proposals made outside this conversation.", loose.len())
                };
                list = list.child(ui::label(note, text::BODY, theme::ink3()));
            }
            for proposal in &loose {
                let pinned = pinned.as_deref() == Some(proposal.proposal_id.as_str());
                list = list.child(self.proposal_card(proposal, pinned, window, cx));
            }
        }
        list = list.child(self.makings.clone());
        if self.follow {
            self.scroll.scroll_to_bottom();
            self.follow = false;
        }
        let composer = self.composer(cx);

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("conversation")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div().w_full().flex().justify_center().px(px(24.0)).child(
                            div()
                                .w_full()
                                .max_w(px(layout::CONVERSATION_COLUMN))
                                .child(list),
                        ),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .flex()
                    .justify_center()
                    .px(px(24.0))
                    .pb(px(16.0))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(layout::CONVERSATION_COLUMN))
                            .child(composer),
                    ),
            )
    }
}
