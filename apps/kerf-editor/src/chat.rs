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
//! Proposal cards say what each proposal does and the clips it changes (a
//! colour change on one clip in a line of its own), with its versions; the
//! open one nearest the playhead is pinned. A proposal has one card, under
//! the newest turn that names it; an earlier one that named it says so in a
//! line. Esc puts a menu of the conversation away first.
//!
//! The composer's chips name the folder, the model (Claude Code's chosen
//! there) and what Kerf does without asking, that one with its mode's icon
//! and tone ([`permission_look`]). Where nothing can be asked (no
//! folder, kerfd gone, no key or no Kerf account) its placeholder says so in
//! Qt's words, a hover says why, and Send is off. A request written while
//! Kerf works keeps the playhead time it was written at.

use crate::commands::{RevealPicture, RunCommand};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::model::{
    self, ClipShow, Proposal, Snapshot, Speaker, SummaryShow, Timeline, TimelineShow, TrackShow,
    Turn, TurnShow, ViewEffect,
};
use crate::motion::{self, Tween};
use crate::prefs::{Permissions, Provider};
use crate::selectable::{self, Selections};
use crate::session::{BridgeEvent, Focused, Session};
use crate::store::{Change, Connection, ContextLevel, ContextUse, NextStep, StepState};
use crate::theme::{self, layout, motion as scene, text};
use crate::ui::{self, icon_button};
use gpui::RenderImage;
use gpui::{
    anchored, deferred, div, linear_color_stop, linear_gradient, point, prelude::*, px, radians,
    AnimationExt, AnyElement, App, ClipboardItem, Context, Corner, DismissEvent, Edges, ElementId,
    Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Rgba, ScrollHandle, SharedString,
    Subscription, Task, TextRun, Transformation, Window,
};
use kerf_bridge::event::{AgentState, Check, CheckSummary};
use kerf_bridge::{Command, Event};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The most the composer's text grows to before it scrolls, in points.
pub const COMPOSER_MOST: f32 = 110.0;

/// The widest the composer's folder chip grows before its name is cut.
pub use crate::theme::layout::COMPOSER_FOLDER_MOST as FOLDER_CHIP_MOST;

/// The key context of the composer's box, where Shift-Tab changes what
/// Kerf does without asking.
pub const COMPOSER_CONTEXT: &str = "Composer";

gpui::actions!(chat, [CyclePermissions]);

/// What the conversation asks of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatEvent {
    /// A cut's card was pressed: its picture grows out of the card into the
    /// stage (`proposal` names the cut).
    GrowPicture { proposal: String },
}

/// The mode after `permissions`, as Shift-Tab goes round them.
pub fn next_permissions(permissions: Permissions) -> Permissions {
    match permissions {
        Permissions::Ask => Permissions::AcceptEdits,
        Permissions::AcceptEdits => Permissions::Auto,
        Permissions::Auto => Permissions::Ask,
    }
}

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

/// The frame a timecode names at `fps`, counted from 00:00:00:00: whole
/// frames of the rounded rate, as [`model::timecode`] writes them. At 29.97
/// or 59.94 with `;` before the frames it is drop-frame, as Kerf writes
/// them: the frame numbers skipped at each minute but every tenth (two a
/// minute, four at 59.94) are not counted.
fn timecode_frames(timecode: &str, fps: f64) -> Option<i64> {
    if !is_timecode(timecode.as_bytes()) {
        return None;
    }
    let base = fps.round().max(1.0) as i64;
    let part = |range: std::ops::Range<usize>| timecode[range].parse::<i64>().ok();
    let (hours, minutes, seconds, frames) = (part(0..2)?, part(3..5)?, part(6..8)?, part(9..11)?);
    let total_minutes = hours * 60 + minutes;
    let mut total = (total_minutes * 60 + seconds) * base + frames;
    let drop_frame = timecode.as_bytes()[8] == b';'
        && matches!(base, 30 | 60)
        && (fps - base as f64).abs() > 1e-3;
    if drop_frame {
        total -= base / 15 * (total_minutes - total_minutes / 10);
    }
    Some(total)
}

fn usable_rate(fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    }
}

/// The seconds a timecode names at `fps`, from 00:00:00:00: whole frames of
/// the rounded rate, drop-frame where it says so (`;` at 29.97 or 59.94).
/// None for one that does not read.
pub fn timecode_seconds(timecode: &str, fps: f64) -> Option<f64> {
    let fps = usable_rate(fps);
    timecode_frames(timecode, fps).map(|frames| frames as f64 / fps)
}

/// Where on the program a timecode Kerf wrote lies, in seconds. Kerf writes
/// the sequence's own timecode, which starts at the sequence's start (often
/// 01:00:00:00); one before that start is read as the editor writes them,
/// from the program's first frame. None for one that does not read.
pub fn timecode_position(timecode: &str, timeline: &Timeline) -> Option<f64> {
    let fps = usable_rate(timeline.rate());
    let frames = timecode_frames(timecode, fps)?;
    let start = timeline.start.unwrap_or(0.0);
    let start = if start.is_finite() {
        (start.max(0.0) * fps).round() as i64
    } else {
        0
    };
    let into = if frames >= start {
        frames - start
    } else {
        frames
    };
    Some(into as f64 / fps)
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

/// What waits for Kerf to finish: the requests written while it works,
/// joined as one, and the playhead time the latest of them was written at
/// (Qt's queue took the time again with each request); while all that
/// waits is one Ask again, also what that request went about the first
/// time ([`Queued::push_again`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queued(Option<Waiting>);

#[derive(Clone, Debug, PartialEq)]
struct Waiting {
    text: String,
    at: f64,
    about: Option<Focused>,
}

impl Queued {
    /// `text`, written with the playhead at `at`, joins what waits, and the
    /// whole goes with `at`, about the sequence there as the composer's
    /// requests go; nothing written leaves the queue as it is.
    pub fn push(&mut self, text: &str, at: f64) {
        if text.trim().is_empty() {
            return;
        }
        let text = join_queued(self.text(), text);
        self.0 = Some(Waiting {
            text,
            at,
            about: None,
        });
    }

    /// Ask again pressed while Kerf works, with the playhead at `at`: `text`
    /// waits, and while it is all that waits it goes about `about` when the
    /// turn ends (what it went about when this window first sent it, the
    /// Ask popover's clip or scope), else as [`Queued::push`]'s requests
    /// go. Joined with another request, it goes as they do; pressed again
    /// while the same words wait, nothing changes.
    pub fn push_again(&mut self, text: &str, at: f64, about: Option<Focused>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match &self.0 {
            None => {
                self.0 = Some(Waiting {
                    text: text.to_owned(),
                    at,
                    about,
                })
            }
            Some(waiting) if waiting.text == text => {}
            Some(_) => self.push(text, at),
        }
    }

    /// The request waiting, if one is.
    pub fn text(&self) -> Option<&str> {
        self.0.as_ref().map(|waiting| waiting.text.as_str())
    }

    /// What the request waiting goes about, when it is one Ask again of
    /// words this window sent before; None for the sequence at its time.
    pub fn about(&self) -> Option<&Focused> {
        self.0.as_ref().and_then(|waiting| waiting.about.as_ref())
    }

    /// The request waiting and its playhead time, taken to be sent: nothing
    /// waits after.
    pub fn take(&mut self) -> Option<(String, f64)> {
        self.0.take().map(|waiting| (waiting.text, waiting.at))
    }

    /// Nothing waits any more (Cancel).
    pub fn clear(&mut self) {
        self.0 = None;
    }
}

/// How long Copy under an answer says "Copied", and the drafting dots'
/// beat (`theme::motion`).
pub use crate::theme::motion::{COPIED_HOLD, DRAFT_BEAT};

/// How long a turn worked, as its fold says it: "41s", "1m 12s", "1h 4m";
/// never less than a second.
pub fn work_length(seconds: f64) -> String {
    let whole = if seconds.is_finite() {
        seconds.max(0.0).round().max(1.0) as u64
    } else {
        1
    };
    if whole < 60 {
        format!("{whole}s")
    } else if whole < 3600 {
        format!("{}m {}s", whole / 60, whole % 60)
    } else {
        format!("{}h {}m", whole / 3600, (whole % 3600) / 60)
    }
}

/// The clock beside "Working": minutes and seconds so far, "0:07", "12:40".
pub fn work_clock(seconds: f64) -> String {
    let whole = if seconds.is_finite() {
        seconds.max(0.0).floor() as u64
    } else {
        0
    };
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// Which of the three drafting dots is lit `elapsed` into the pulse; None
/// under Reduce Motion, when all three hold still.
pub fn draft_dot(elapsed: Duration, reduced: bool) -> Option<usize> {
    if reduced {
        return None;
    }
    Some((elapsed.as_millis() / DRAFT_BEAT.as_millis().max(1)) as usize % 3)
}

/// How long ago something was settled here, as a card says it: "now",
/// "4 min", "2 h", then "N d".
pub fn settled_age(ago: Duration) -> String {
    let seconds = ago.as_secs();
    if seconds < 60 {
        "now".to_owned()
    } else if seconds < 3600 {
        format!("{} min", seconds / 60)
    } else if seconds < 86_400 {
        format!("{} h", seconds / 3600)
    } else {
        format!("{} d", seconds / 86_400)
    }
}

/// What a settled card says: "Accepted in rev 23 · now", "Accepted",
/// "Rejected · 4 min". The age only for one settled in this window.
pub fn settled_words(accepted: bool, revision: Option<u64>, ago: Option<Duration>) -> String {
    let words = match (accepted, revision) {
        (true, Some(revision)) => format!("Accepted in rev {revision}"),
        (true, None) => "Accepted".to_owned(),
        (false, _) => "Rejected".to_owned(),
    };
    match ago {
        Some(ago) => format!("{words} · {}", settled_age(ago)),
        None => words,
    }
}

/// The line over proposals that no turn of this conversation staged: who
/// proposed the one over MCP when an agent did (the runtime's author kind
/// `mcp`, Qt's `Author::isAgent`), else how many there are.
pub fn strays_note(strays: &[Proposal]) -> String {
    if let [only] = strays {
        let agent = only.author.kind == "mcp";
        let name = only.author.name.trim();
        if agent && !name.is_empty() {
            return format!("{name} proposed this over MCP.");
        }
        return "Proposed outside this conversation.".to_owned();
    }
    format!("{} proposals made outside this conversation.", strays.len())
}

/// The latest conversations a new one offers to go back to: those not
/// archived and not `open`, newest first, at most `most`.
pub fn recent_conversations<'a>(
    conversations: &'a [model::ConversationSummary],
    open: Option<&str>,
    most: usize,
) -> Vec<&'a model::ConversationSummary> {
    let mut recent: Vec<&model::ConversationSummary> = conversations
        .iter()
        .filter(|summary| !summary.archived && Some(summary.id.as_str()) != open)
        .collect();
    recent.sort_by(|a, b| b.updated.cmp(&a.updated));
    recent.truncate(most);
    recent
}

/// What a new conversation offers to ask, as Qt's welcome did: Kerf's next
/// steps (each shows its label and fills the composer with its request),
/// else the first requests Qt offered; at most four.
pub fn try_asking(next_steps: &[NextStep]) -> Vec<NextStep> {
    let offered: Vec<NextStep> = next_steps
        .iter()
        .filter_map(|step| {
            let label = step.label.trim();
            let request = step.request.trim();
            (!label.is_empty())
                .then(|| NextStep::new(label, if request.is_empty() { label } else { request }))
        })
        .take(4)
        .collect();
    if !offered.is_empty() {
        return offered;
    }
    [
        "Make a 30-second cut from the best takes",
        "Tighten the pauses in the interview",
        "Add captions to the whole sequence",
    ]
    .map(NextStep::from)
    .to_vec()
}

/// "Worked for 1m 12s · 6 steps": what a finished turn folds to.
pub fn worked_headline(seconds: Option<f64>, steps: usize) -> String {
    let length = work_length(seconds.unwrap_or(0.0));
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
        .find(|turn| turn.speaker() == Speaker::You)
        .map(|turn| turn.text.as_str())
        .filter(|text| !text.trim().is_empty())
}

/// Which turn holds each proposal's card, by the proposal's current id: the
/// newest of Kerf's turns that names it by any of its ids. An earlier turn
/// that names it says so in a line instead, so a proposal has one card.
pub fn card_turns(
    turns: &[Turn],
    proposals: &[Proposal],
    book: &ProposalBook,
) -> HashMap<String, usize> {
    let mut holders = HashMap::new();
    for (index, turn) in turns.iter().enumerate() {
        if turn.speaker() != Speaker::Kerf {
            continue;
        }
        if let Some(proposal) = turn
            .proposal
            .as_deref()
            .and_then(|id| book.find(proposals, id))
        {
            holders.insert(proposal.proposal_id.clone(), index);
        }
    }
    holders
}

/// What the request bubble's timecode chip says on a rest: where pressing
/// it goes ("Go to 00:00:17:08"), as Qt's TimecodeChip said it.
pub fn stamp_tip(timecode: &str) -> String {
    format!("Go to {timecode}")
}

/// The proposal card's Preview, Script and version button say what they
/// do on a rest, as Qt's card's did.
pub const PREVIEW_TIP: &str = "Preview the proposal on the program";
pub const SCRIPT_TIP: &str = "Show the script";
pub const VERSIONS_TIP: &str = "Proposal versions";

/// How long a mode chosen stays said: Qt's status line held it 3 s.
pub use crate::theme::motion::NOTICE_BRIEF_HOLD as MODE_HOLD;

/// Says the mode `permissions` and what it means ("Auto · Edits apply, and
/// Kerf makes its shots") in place of the mode said last, for [`MODE_HOLD`]:
/// choosing again and again leaves one line, the newest.
pub fn say_permissions(state: &mut crate::store::EditorState, permissions: Permissions) {
    let (label, line) = permission_words(permissions);
    state.say_in_place(
        crate::store::StatusLine::Permissions,
        &format!("{label} · {line}"),
        false,
        Some(MODE_HOLD),
    );
}

/// What the permission chip says, and its line in the chip's menu.
pub fn permission_words(permissions: Permissions) -> (&'static str, &'static str) {
    match permissions {
        Permissions::Ask => ("Ask before edits", "Each edit waits for you"),
        Permissions::AcceptEdits => ("Accept edits", "Edits apply as Kerf makes them"),
        Permissions::Auto => ("Auto", "Edits apply, and Kerf makes its shots"),
    }
}

/// How the permission chip looks, as Qt's did (`modeIcon`, `modeTone`): a
/// lock while each edit waits, a check while edits apply, Kerf's own mark
/// under Auto; drawn in the accent's ink for Accept edits and the GPU's
/// colour for Auto, so the modes where Kerf does more on its own stand
/// out. None draws it as the other chips are drawn.
pub fn permission_look(permissions: Permissions) -> (Icon, Option<Rgba>) {
    match permissions {
        Permissions::Ask => (Icon::Lock, None),
        Permissions::AcceptEdits => (Icon::Check, Some(theme::accent_ink())),
        Permissions::Auto => (Icon::Kerf, Some(theme::activity::gpu())),
    }
}

/// Whether a proposal's preview in `status` is showing or on its way, so
/// its card says Previewing: not once it failed, stopped or was left, nor
/// while it waits to be rendered (`required`, `unavailable`), when
/// pressing Preview tries again.
pub fn previewing(status: &str) -> bool {
    matches!(status, "running" | "completed" | "ready" | "live")
}

/// Why the composer cannot ask: what it says in its placeholder's place,
/// and the reason a hover over it (or Return) gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub placeholder: &'static str,
    pub reason: String,
}

/// Why nothing can be asked now, in Qt's words: no folder open, kerfd not
/// connected, or (while Kerf is not working) a connection that cannot take
/// requests, `ask_reason` being the registry's reason for `ai.ask` ("Add
/// your API key in Settings → Chat", the Kerf account's). None when a
/// request can go.
pub fn refusal(
    has_project: bool,
    disconnected: bool,
    busy: bool,
    ask_reason: Option<&str>,
) -> Option<Refusal> {
    let refused = |placeholder, reason: &str| {
        Some(Refusal {
            placeholder,
            reason: reason.to_owned(),
        })
    };
    if !has_project {
        refused(
            "Open a folder to ask Kerf",
            "Open a folder or start a project first",
        )
    } else if disconnected {
        refused("Connect to ask Kerf", "Connect to start editing")
    } else {
        match ask_reason.filter(|reason| !busy && !reason.trim().is_empty()) {
            Some(reason) => refused("Set up Kerf in Settings to ask", reason),
            None => None,
        }
    }
}

/// The registry's reason Ask cannot run on a connection whose access is
/// `access` (`noKey`, or a Kerf account's standing such as `signedOut`),
/// whatever else holds; None for `ready`.
pub fn access_reason(access: &str) -> Option<String> {
    let states =
        kerf_bridge::commands_states(&serde_json::json!({"connected": true, "aiAccess": access}));
    states["ai.ask"].as_str().map(str::to_owned)
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
    /// Reads a `state` event's snapshot. Its proposals' timelines are named
    /// ([`TimelineShow::name_clips`]) as the program's are.
    pub fn read(snapshot: &Snapshot) -> Self {
        let mut book = Self::default();
        for entry in &snapshot.proposals {
            let id = entry.proposal_id.as_str();
            if id.is_empty() {
                continue;
            }
            let versions: Vec<Version> = entry
                .versions
                .iter()
                .map(|version| Version {
                    version: version.version,
                    proposal_id: version.proposal_id.clone(),
                    script: version.script.clone(),
                    document_diff: version.document_diff.clone(),
                })
                .collect();
            if !versions.is_empty() {
                book.versions.insert(id.to_owned(), versions);
            }
            if let Some(replaced) = entry.supersedes.as_deref().filter(|id| !id.is_empty()) {
                book.supersedes.insert(id.to_owned(), replaced.to_owned());
            }
            if let Some(revision) = entry.applied_revision {
                book.applied.insert(id.to_owned(), revision);
            }
        }
        for (id, timeline) in &snapshot.proposal_timelines {
            let mut timeline = timeline.clone();
            timeline.name_clips();
            book.timelines.insert(id.clone(), timeline);
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

    /// Whether ids `a` and `b` name one proposal: the same id, or two of
    /// the ids it has gone by (its versions, what it replaced).
    pub fn same(&self, a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        self.versions.iter().any(|(current, versions)| {
            let names = |id: &str| {
                current == id
                    || self.supersedes.get(current).map(String::as_str) == Some(id)
                    || versions.iter().any(|version| version.proposal_id == id)
            };
            names(a) && names(b)
        })
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
    /// The clip's id, or the transition's.
    pub clip: String,
    /// A transition between clips, not a clip.
    pub transition: bool,
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
        let name = if self.transition {
            "Transition"
        } else {
            &self.name
        };
        let name = if self.track.is_empty() {
            name.to_owned()
        } else {
            format!("{name} · {}", self.track)
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

/// One stretch of a change reel: where it plays on the program as it is
/// (`before`) and on what the proposal makes (`after`), and what it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Segment {
    pub before: f64,
    pub before_end: f64,
    pub after: f64,
    pub after_end: f64,
    pub title: String,
}

impl Segment {
    pub fn length(&self) -> f64 {
        self.after_end - self.after
    }
}

/// What a change does, in words: "Trim sea", "Add title", "Transition".
pub fn change_title(change: &ClipChange) -> String {
    let name = if change.name.is_empty() {
        &change.clip
    } else {
        &change.name
    };
    if change.transition {
        "Transition".to_owned()
    } else if change.added {
        format!("Add {name}")
    } else if change.removed {
        format!("Remove {name}")
    } else if change.trimmed {
        format!("Trim {name}")
    } else if change.moved || change.retracked {
        format!("Move {name}")
    } else if change.effects {
        format!("Effects on {name}")
    } else {
        format!("Change {name}")
    }
}

/// The change reel of `changes`: each change with `handle` seconds either
/// side, in order on the proposal, those that touch merged ("Trim sea and
/// 2 more"), each side's ends kept to its length (`before_length` the
/// program's, `after_length` the proposal's). What a clamp leaves nothing
/// of goes.
pub fn reel_plan(
    changes: &[ClipChange],
    before_length: f64,
    after_length: f64,
    handle: f64,
) -> Vec<Segment> {
    let mut raw: Vec<Segment> = changes
        .iter()
        .map(|change| Segment {
            // What is gone has no place on the proposal but where it was,
            // and what is new none on the program but where it lands.
            after: if change.removed {
                change.old_start
            } else {
                change.new_start
            },
            after_end: if change.removed {
                change.old_start
            } else {
                change.new_end
            },
            before: if change.added {
                change.new_start
            } else {
                change.old_start
            },
            before_end: if change.added {
                change.new_start
            } else {
                change.old_end
            },
            title: change_title(change),
        })
        .collect();
    raw.sort_by(|a, b| a.after.total_cmp(&b.after));
    let mut out: Vec<Segment> = Vec::new();
    let mut merged = 0;
    let close = |out: &mut Vec<Segment>, merged: &mut usize| {
        if let (Some(last), true) = (out.last_mut(), *merged > 0) {
            last.title = format!("{} and {} more", last.title, merged);
        }
        *merged = 0;
    };
    for mut segment in raw {
        segment.after = (segment.after - handle).max(0.0);
        segment.after_end = (segment.after_end + handle).min(after_length.max(0.0));
        segment.before = (segment.before - handle).max(0.0);
        segment.before_end = (segment.before_end + handle).min(before_length.max(0.0));
        if let Some(last) = out
            .last_mut()
            .filter(|last| segment.after <= last.after_end)
        {
            last.after_end = last.after_end.max(segment.after_end);
            last.before = last.before.min(segment.before);
            last.before_end = last.before_end.max(segment.before_end);
            merged += 1;
            continue;
        }
        close(&mut out, &mut merged);
        out.push(segment);
    }
    close(&mut out, &mut merged);
    out.retain(|segment| segment.length() > 0.0);
    out
}

/// How long a reel takes to watch, in seconds.
pub fn reel_total(segments: &[Segment]) -> f64 {
    segments.iter().map(Segment::length).sum()
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
            || clip.props != was.props
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
    changes.extend(transition_changes(program, proposed));
    changes
}

/// The transitions `proposed` adds, moves, cuts, restyles or takes off
/// against `program`, by id: a crossfade is a change as a clip is.
fn transition_changes(program: &Timeline, proposed: &Timeline) -> Vec<ClipChange> {
    const NEAR: f64 = 1e-4;
    fn fades(timeline: &Timeline) -> Vec<(&model::Track, &model::Transition)> {
        let mut seen = HashSet::new();
        timeline
            .tracks
            .iter()
            .flat_map(|track| track.transitions.iter().map(move |fade| (track, fade)))
            .filter(|(_, fade)| !fade.id.is_empty() && seen.insert(fade.id.clone()))
            .collect()
    }
    let before = fades(program);
    let after = fades(proposed);
    let mut changes = Vec::new();
    for (track, fade) in &after {
        let mut change = ClipChange {
            clip: fade.id.clone(),
            transition: true,
            name: "Transition".to_owned(),
            track: track.label().to_owned(),
            new_start: fade.start,
            new_end: fade.end,
            ..ClipChange::default()
        };
        let Some((was_track, was)) = before.iter().find(|(_, was)| was.id == fade.id) else {
            change.added = true;
            changes.push(change);
            continue;
        };
        change.old_start = was.start;
        change.old_end = was.end;
        change.moved = (fade.start - was.start).abs() > NEAR;
        change.trimmed = ((fade.end - fade.start) - (was.end - was.start)).abs() > NEAR;
        change.properties = fade.from != was.from || fade.to != was.to || fade.fx != was.fx;
        change.retracked = track.id != was_track.id;
        if change.moved || change.trimmed || change.properties || change.retracked {
            changes.push(change);
        }
    }
    for (track, fade) in &before {
        if after.iter().any(|(_, now)| now.id == fade.id) {
            continue;
        }
        changes.push(ClipChange {
            clip: fade.id.clone(),
            transition: true,
            name: "Transition".to_owned(),
            track: track.label().to_owned(),
            removed: true,
            old_start: fade.start,
            old_end: fade.end,
            ..ClipChange::default()
        });
    }
    changes
}

/// Whether an effect is a colour one, by its id: Qt's rule.
fn is_colour_effect(fx: &str) -> bool {
    let id = fx.to_lowercase();
    id.starts_with("color.")
        || id.starts_with("colour.")
        || [
            "hue",
            "grade",
            "lut",
            "look",
            "curve",
            "balance",
            "saturation",
        ]
        .iter()
        .any(|word| id.contains(word))
}

/// An effect's or a parameter's id as a card says it: "color.white_balance"
/// is "White balance".
pub fn effect_label(id: &str) -> String {
    let name = id.rsplit('.').next().unwrap_or(id).replace('_', " ");
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => id.to_owned(),
    }
}

/// A parameter's value as a card says it: "−9", "0.5", "on"; None for
/// keyframes, which are no one value.
fn param_value(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => {
            let number = number.as_f64()?;
            // Four significant figures, as Qt's 'g' wrote them.
            let magnitude = number.abs();
            let decimals = if magnitude == 0.0 {
                0
            } else {
                (3 - magnitude.log10().floor() as i32).max(0) as usize
            };
            let text = format!("{magnitude:.decimals$}");
            let text = if text.contains('.') {
                text.trim_end_matches('0').trim_end_matches('.').to_owned()
            } else {
                text
            };
            Some(if number < 0.0 {
                format!("−{text}")
            } else {
                text
            })
        }
        Value::Bool(on) => Some(if *on { "on" } else { "off" }.to_owned()),
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// A clip's colour change, said on its card in a line of its own when it
/// is all the proposal changes: "B003_C011 · Hue vs hue", and what it sets
/// ("Hue −9, Sat −6"). None for anything else.
pub fn colour_change(
    program: &Timeline,
    proposed: &Timeline,
    changes: &[ClipChange],
) -> Option<(String, String)> {
    let [change] = changes else {
        return None;
    };
    if !change.effects || change.transition || change.added || change.removed {
        return None;
    }
    let before = program
        .clip(&change.clip)
        .map(|(_, clip)| clip.effects.clone())
        .unwrap_or_default();
    let (_, now) = proposed.clip(&change.clip)?;
    // Only what holds still: keyframes are no one value.
    let still = |effect: &ViewEffect| -> serde_json::Map<String, Value> {
        effect
            .params
            .iter()
            .filter(|(_, value)| !value.is_array() && !value.is_object())
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    };
    for effect in &now.effects {
        let fx = effect.fx.as_str();
        let old = before.iter().find(|old| old.fx == fx);
        let old_params = old.map(still).unwrap_or_default();
        let new_params = still(effect);
        if old.is_some() && old_params == new_params {
            continue;
        }
        if !is_colour_effect(fx) {
            return None;
        }
        let title = format!("{} · {}", change.name, effect_label(fx));
        let params = new_params
            .iter()
            .filter(|(key, value)| old.is_none() || old_params.get(*key) != Some(*value))
            .filter_map(|(key, value)| {
                param_value(value).map(|value| format!("{} {value}", effect_label(key)))
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Some((title, params));
    }
    None
}

/// What a finished check's summary (`check {status: "done", summary:
/// {rows}}`) offers to ask: "Fix the true peak" for each failing row with a
/// name, with its timecode where it has one ("Fix the true peak at
/// 00:00:04:12"): the words shown, and the request they fill the composer
/// with. A time that is not finite is none, as JSON's `null`.
pub fn check_suggestions(summary: &CheckSummary, fps: f64) -> Vec<(String, String)> {
    summary
        .rows
        .iter()
        .filter(|row| !row.passed)
        .filter_map(|row| {
            let name = row.name.trim();
            let mut chars = name.chars();
            let first = chars.next()?;
            let lowered: String = first.to_lowercase().chain(chars).collect();
            let label = format!("Fix the {lowered}");
            let request = match row.at.filter(|at| at.is_finite()) {
                Some(at) => format!("{label} at {}", model::timecode(at, fps)),
                None => label.clone(),
            };
            Some((label, request))
        })
        .collect()
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

/// A script line's words, a quoted phrase ("A lower third") one word.
pub fn script_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in line.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c.is_whitespace() && !quoted {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        word.push(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// The value of `key=` among a line's words, unquoted; empty for none.
fn script_value(words: &[String], key: &str) -> String {
    let prefix = format!("{key}=");
    words
        .iter()
        .find_map(|word| word.strip_prefix(&prefix))
        .map(|value| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(value)
                .to_owned()
        })
        .unwrap_or_default()
}

/// Seconds in a script's amount or position ("-36f", "+1.5s", "96f",
/// "00:00:12:00"); None for anything else (an anchor).
pub fn script_seconds(text: &str, fps: f64) -> Option<f64> {
    let fps = fps.max(1.0);
    let text = text.trim();
    let (sign, text) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('+').unwrap_or(text)),
    };
    let value = if let Some(frames) = text.strip_suffix('f') {
        frames.parse::<f64>().ok()? / fps
    } else if let Some(seconds) = text.strip_suffix('s') {
        seconds.parse::<f64>().ok()?
    } else {
        let parts: Vec<f64> = text
            .split([':', ';'])
            .map(|part| part.parse::<f64>().ok())
            .collect::<Option<_>>()?;
        let [hours, minutes, seconds, frames] = parts.as_slice() else {
            return None;
        };
        hours * 3600.0 + minutes * 60.0 + seconds + frames / fps
    };
    value.is_finite().then_some(sign * value)
}

/// A name with a media file's own extension taken off ("sea.mov" is
/// "sea"); any other dot stays ("clip.v2").
pub fn media_stem(name: &str) -> &str {
    const MEDIA: [&str; 20] = [
        "mov", "mp4", "m4v", "mkv", "webm", "avi", "mxf", "wav", "mp3", "m4a", "aac", "aif",
        "aiff", "flac", "png", "jpg", "jpeg", "gif", "tif", "tiff",
    ];
    match name.rfind('.') {
        Some(at)
            if at > 0
                && MEDIA
                    .iter()
                    .any(|suffix| name[at + 1..].eq_ignore_ascii_case(suffix)) =>
        {
            &name[..at]
        }
        _ => name,
    }
}

/// "0:12", "1:04:30": a position as a reader says it.
pub fn clock_time(seconds: f64) -> String {
    let whole = seconds.max(0.0).round() as u64;
    let (hours, minutes, rest) = (whole / 3600, (whole / 60) % 60, whole % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{rest:02}")
    } else {
        format!("{minutes}:{rest:02}")
    }
}

/// "−1.5 s", "+12 s": an amount, its sign always shown (a true minus).
pub fn signed_seconds(seconds: f64) -> String {
    let magnitude = seconds.abs();
    let text = if magnitude < 10.0 {
        format!("{magnitude:.1}")
    } else {
        format!("{magnitude:.0}")
    };
    let text = text.strip_suffix(".0").unwrap_or(&text).to_owned();
    format!("{}{text} s", if seconds < 0.0 { "−" } else { "+" })
}

/// A script line as a change a reader follows (Qt's describeScriptLine):
/// what it does, to which clip by its name, and how much or where ("Trim
/// fisher-close", "end −1.5 s"). A line not known here reads as itself.
/// `program` names the clips and media; `proposed` names what it adds.
pub fn describe_script_line(
    line: &str,
    program: &Timeline,
    proposed: Option<&Timeline>,
) -> (String, String) {
    let line = line.trim();
    let mut chars = line.chars();
    let as_written = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    let words = script_words(line);
    let Some(verb) = words.first().map(String::as_str) else {
        return (as_written, String::new());
    };
    let fps = program.rate();
    let word = |index: usize| words.get(index).cloned().unwrap_or_default();
    let clip_name = |id: &str| {
        program
            .clip(id)
            .or_else(|| proposed.and_then(|cut| cut.clip(id)))
            .map(|(_, clip)| media_stem(clip.label()).to_owned())
            .unwrap_or_else(|| id.to_owned())
    };
    let span = |id: &str| {
        program.clip(id).map(|(_, clip)| {
            format!(
                "{}\u{2009}–\u{2009}{}",
                clock_time(clip.timeline_start),
                clock_time(clip.end())
            )
        })
    };
    let at = |value: String| match script_seconds(&value, fps) {
        Some(seconds) => clock_time(seconds),
        None => value,
    };
    let subject = word(1);
    let (description, detail) = match verb {
        "trim" if words.len() >= 3 => {
            let edge = if word(2) == "out" { "end" } else { "start" };
            let detail = match script_seconds(&word(3), fps) {
                Some(amount) => format!("{edge} {}", signed_seconds(amount)),
                None if !script_value(&words, "to").is_empty() => {
                    format!("{edge} to {}", at(script_value(&words, "to")))
                }
                None => edge.to_owned(),
            };
            (format!("Trim {}", clip_name(&subject)), detail)
        }
        "remove" | "delete" | "ripple-delete" | "extract" => (
            format!("Remove {}", clip_name(&subject)),
            span(&subject).unwrap_or_default(),
        ),
        "add" if words.len() >= 2 => {
            let kind = word(1);
            let mut text = script_value(&words, "text");
            if text.is_empty() {
                text = proposed
                    .and_then(|cut| cut.clip(&word(2)))
                    .and_then(|(_, clip)| clip.name.clone())
                    .unwrap_or_default();
            }
            let description = if text.is_empty() {
                format!("Add a {kind}")
            } else {
                format!("Add {kind} “{text}”")
            };
            let place = match words.iter().position(|word| word == "at") {
                Some(index) => word(index + 1),
                None => script_value(&words, "at"),
            };
            let detail = if place.is_empty() {
                String::new()
            } else {
                format!("at {}", at(place))
            };
            (description, detail)
        }
        "insert" | "overwrite" => {
            let media = script_value(&words, "media");
            let name = if program.source(&media).is_some() {
                media_stem(&program.media_name(&media)).to_owned()
            } else {
                media
            };
            let verb = if verb == "insert" {
                "Insert"
            } else {
                "Overwrite with"
            };
            let place = script_value(&words, "at");
            let detail = if place.is_empty() {
                String::new()
            } else {
                format!("at {}", at(place))
            };
            (format!("{verb} {name}"), detail)
        }
        "move" | "roll" | "slide" | "slip" if words.len() >= 2 => {
            let mut title = verb.chars();
            let title: String = title
                .next()
                .map(|first| first.to_uppercase().chain(title).collect())
                .unwrap_or_default();
            let detail = match script_seconds(&word(2), fps) {
                Some(amount) => signed_seconds(amount),
                None if !script_value(&words, "at").is_empty() => {
                    format!("to {}", at(script_value(&words, "at")))
                }
                None if !script_value(&words, "to").is_empty() => {
                    format!("to {}", at(script_value(&words, "to")))
                }
                None => String::new(),
            };
            (format!("{title} {}", clip_name(&subject)), detail)
        }
        "split" if words.len() >= 2 => {
            let place = script_value(&words, "at");
            let detail = if place.is_empty() {
                String::new()
            } else {
                format!("at {}", at(place))
            };
            (format!("Split {}", clip_name(&subject)), detail)
        }
        "set" if words.len() >= 3 => {
            let keys: Vec<&str> = words[2..]
                .iter()
                .map(|word| word.split('=').next().unwrap_or_default())
                .collect();
            let gain = script_value(&words, "gain");
            if !gain.is_empty() || keys.contains(&"volume") {
                let detail = gain
                    .parse::<f64>()
                    .ok()
                    .map(|db| {
                        let sign = if db < 0.0 { "−" } else { "" };
                        let value = param_value(&serde_json::json!(db.abs())).unwrap_or_default();
                        format!("{sign}{value} dB")
                    })
                    .unwrap_or_default();
                (format!("Change {}’s level", clip_name(&subject)), detail)
            } else {
                (
                    format!("Change {}’s {}", clip_name(&subject), keys.join(", ")),
                    String::new(),
                )
            }
        }
        "fx" if words.len() >= 3 => {
            let effect = word(3).rsplit('.').next().unwrap_or_default().to_owned();
            let name = clip_name(&subject);
            let description = match word(2).as_str() {
                "add" => format!("Add {effect} to {name}"),
                "remove" => format!("Remove an effect from {name}"),
                _ => format!("Change the effects on {name}"),
            };
            (description, String::new())
        }
        "mask" if words.len() >= 3 => {
            let name = clip_name(&subject);
            let description = if word(2) == "add" {
                format!("Add a mask to {name}")
            } else {
                format!("Change the masks on {name}")
            };
            (description, String::new())
        }
        "transition" => {
            let kind = script_value(&words, "fx");
            let kind = kind.rsplit('.').next().unwrap_or_default();
            let description = if kind.is_empty() {
                "Add a transition".to_owned()
            } else {
                format!("Add a {kind}")
            };
            let detail = script_seconds(&script_value(&words, "dur"), fps)
                .map(|length| {
                    signed_seconds(length)
                        .trim_start_matches(['+', '−'])
                        .to_owned()
                })
                .unwrap_or_default();
            (description, detail)
        }
        "captions" if words.len() >= 2 => {
            (format!("Caption {}", clip_name(&subject)), String::new())
        }
        _ => (as_written, String::new()),
    };
    (description, detail)
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
    /// The folder chip's: the folders, Open Folder… and New Project….
    Folder,
    /// A card's versions, by the proposal's id.
    Versions(String),
    /// The context meter's card.
    Context,
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
    /// A request written while Kerf worked, to go when the turn ends, and
    /// the playhead time it goes with.
    queued: Queued,
    /// The turn under way was stopped, here or from a menu or the palette,
    /// and has not ended yet.
    stopping: bool,
    /// The answer just copied, which says so for a moment.
    copied: Option<String>,
    menu: Option<Menu>,
    /// The open menu's rows, as the kit's menu (all but the context card's).
    kit_menu: Option<Entity<ui::Menu>>,
    /// Esc just put a menu of the conversation away: that Esc is the
    /// menu's alone, and the window's own Esc order takes it as a menu's
    /// ([`ChatView::take_escape`]), not as a render's to stop.
    escape_took_menu: bool,
    /// The menu of the conversation's words (Copy, Select All) a
    /// right-click on them opened, and where, while it is open.
    words_menu: Option<(Entity<ui::Menu>, gpui::Point<gpui::Pixels>)>,
    /// The menu a press outside it closed, and when: that press on its own
    /// chip does not open it again.
    menu_closed: Option<(Menu, Instant)>,
    placeholder: SharedString,
    /// Why nothing can be asked now (its placeholder, and the reason), as
    /// last worked out; the connection's access (None: it can take
    /// requests) it was worked out for, and the registry's reason for it.
    refusal: Option<Refusal>,
    access: Option<Option<String>>,
    ask_reason: Option<String>,
    /// Under Reduce Motion the working clock goes on once a second: the one
    /// timer that says so, while Kerf works.
    clock: Option<Task<()>>,
    /// What the last check found failing, as requests to fill the composer
    /// with: the words shown, and the request.
    fixes: Vec<(String, String)>,
    /// The keys a card's Accept and Reject name, as the keymap they were
    /// read from has them.
    keys: Option<(String, HashMap<&'static str, String>)>,
    /// The Ask popover, while it is open.
    ask: Option<Entity<crate::ask::AskView>>,
    /// The cut whose picture went to the stage from its card.
    stage_proposal: Option<String>,
    /// Whether the picture showed as last seen.
    picture_seen: Option<bool>,
    /// A cut's picture under the pointer: the proposal, and how far across.
    scrub: Option<(String, f32)>,
    /// Where each cut's picture was last laid out, by proposal id, for the
    /// pointer's place over it.
    pictures: std::rc::Rc<std::cell::RefCell<HashMap<String, gpui::Bounds<gpui::Pixels>>>>,
    /// The first cut Kerf is making, while it does; and the one it just
    /// finished, and when, which says Done for a moment.
    building: Option<String>,
    built: Option<(String, Instant)>,
    /// The poster a building cut showed before its newest clip, and when
    /// the newest began to show over it.
    turning: Option<(String, Instant)>,
    /// The media of the newest clip a building cut's picture shows.
    newest: Option<String>,
    /// The footage gathered as Kerf starts on a project with no cut, and
    /// when it gathered.
    footage: Option<(Vec<String>, Instant)>,
    /// Whether footage is to gather for the turn under way, once its
    /// posters have come.
    footage_wanted: bool,
    /// The turn last seen starting.
    turn_seen: Option<Instant>,
    /// When the view began: what repeats goes on from here.
    started_at: Instant,
    /// How far the conversation sits back while a change reel plays.
    behind: Tween,
    /// What is chosen of the requests' and answers' words.
    selections: selectable::Shared,
    /// Where each footage tile was last laid out, by its media, for what
    /// goes on from it to fly from; and the tiles that have gone on.
    tiles: std::rc::Rc<std::cell::RefCell<HashMap<String, gpui::Bounds<gpui::Pixels>>>>,
    taken: HashSet<String>,
    /// Where the composer was last laid out.
    composer_at: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ChatEvent> for ChatView {}

const PLACEHOLDER: &str = "Ask Kerf to cut, trim, find, or make…";

impl ChatView {
    /// Whether a menu of the conversation is open (a composer chip's, a
    /// card's versions, the context card).
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// The menu of the conversation's words at `at`, from a right-click on
    /// run `run`: Copy what is chosen (greyed with nothing chosen) and
    /// Select All of the run, as Qt's selectable labels had.
    fn open_words_menu(
        &mut self,
        run: SharedString,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let copied = self.selections.borrow().copied();
        let shared = self.selections.clone();
        let items = vec![
            ui::MenuItem::new("Copy")
                .key(crate::commands::shown_keys("Ctrl+C"))
                .disabled(copied.is_none())
                .on_choose(move |_, cx| {
                    if let Some(words) = copied.clone() {
                        cx.write_to_clipboard(ClipboardItem::new_string(words));
                    }
                }),
            ui::MenuItem::new("Select All").on_choose(move |window, _| {
                if shared.borrow_mut().choose_all(&run) {
                    window.refresh();
                }
            }),
        ];
        let menu = cx.new(|cx| ui::Menu::new(items, window, cx));
        cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this
                .words_menu
                .as_ref()
                .is_some_and(|(open, _)| *open == menu)
            {
                this.words_menu = None;
                cx.notify();
            }
        })
        .detach();
        self.words_menu = Some((menu, at));
        cx.notify();
    }

    /// Whether the Esc being handled put a menu of the conversation away
    /// (its own keystroke intercept runs before the window's), once.
    pub fn take_escape(&mut self) -> bool {
        std::mem::take(&mut self.escape_took_menu)
    }

    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextInput::new(cx, PLACEHOLDER)
                .multiline()
                .max_height(px(COMPOSER_MOST))
        });
        let composer_focus = composer.read(cx).focus_handle(cx);
        let selections = Selections::shared();
        let subscriptions = vec![
            cx.observe(&session, |this, _, cx| {
                this.refresh_placeholder(cx);
                cx.notify()
            }),
            // Run QC is offered while the box has the keys and is empty.
            cx.on_focus(&composer_focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&composer_focus, window, |_, _, cx| cx.notify()),
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
                match &event.0 {
                    Event::State { snapshot } => {
                        this.book = ProposalBook::read(snapshot);
                        cx.notify();
                    }
                    Event::Check(Check::Done { summary, .. }) => {
                        // A check's failing rows are offered as what to ask next.
                        let fps = this.session.read(cx).timeline().rate();
                        this.fixes = check_suggestions(summary, fps);
                        cx.notify();
                    }
                    Event::Cloud { .. } => {
                        // Kerf's account, as every window keeps it, says whether
                        // Kerf can be asked.
                        crate::app::hear_cloud(
                            &serde_json::to_value(&event.0).unwrap_or_default(),
                            cx,
                        );
                        this.access = None;
                        this.refresh_placeholder(cx);
                        cx.notify();
                    }
                    Event::AgentState {
                        state: AgentState::Aborted,
                        ..
                    } => {
                        // Stopped from anywhere (here, a menu, the palette). The
                        // turn says "completed" after this, so it is kept here
                        // until the turn ends, which only a turn under way says.
                        this.stopping = true;
                        cx.notify();
                    }
                    _ => {}
                }
            }),
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.send(window, cx),
                    // Esc stops Kerf while it works; else leaves the box and
                    // puts away what the window has open (the Media band, a
                    // preview rendering), as it would outside the box.
                    InputEvent::Cancel => {
                        if this.session.read(cx).state.work.busy() {
                            this.stop(cx);
                        } else {
                            // From the box, so it reaches the window, before
                            // the box lets the keys go.
                            window.dispatch_action(Box::new(crate::commands::Escape), cx);
                            window.blur();
                        }
                    }
                    InputEvent::Changed => cx.notify(),
                },
            ),
            // ⌘C (Ctrl+C) copies the words chosen in the conversation,
            // wherever the keys are: a press anywhere else would have let
            // them go, and so does any other key a field takes (what is
            // typed or chosen there is then what is copied).
            {
                let chosen = selections.clone();
                let handle = window.window_handle();
                cx.intercept_keystrokes(move |event, window, cx| {
                    if window.window_handle() != handle {
                        return;
                    }
                    let keystroke = &event.keystroke;
                    let copy = keystroke.key == "c"
                        && keystroke.modifiers.secondary()
                        && !keystroke.modifiers.shift
                        && !keystroke.modifiers.alt;
                    if !copy {
                        let in_a_field = event
                            .context_stack
                            .iter()
                            .any(|context| context.contains(crate::input::CONTEXT));
                        if in_a_field && chosen.borrow_mut().clear() {
                            window.refresh();
                        }
                        return;
                    }
                    let Some(words) = chosen.borrow().copied() else {
                        return;
                    };
                    cx.write_to_clipboard(ClipboardItem::new_string(words));
                    cx.stop_propagation();
                })
            },
            // Esc puts a menu of the conversation away wherever the keys
            // are (a press on its chip leaves them where they were), before
            // the box or the window has it.
            {
                let this = cx.weak_entity();
                let handle = window.window_handle();
                cx.intercept_keystrokes(move |event, window, cx| {
                    let keystroke = &event.keystroke;
                    if window.window_handle() != handle
                        || keystroke.key != "escape"
                        || keystroke.modifiers.modified()
                    {
                        return;
                    }
                    let closed = this
                        .update(cx, |this, cx| {
                            let open = this.menu.take().is_some();
                            if open {
                                this.escape_took_menu = true;
                                if let Some(menu) = this.kit_menu.take() {
                                    menu.update(cx, |menu, cx| menu.close(window, cx));
                                }
                                cx.notify();
                            }
                            open
                        })
                        .unwrap_or(false);
                    if closed {
                        cx.stop_propagation();
                    }
                })
            },
        ];
        // A right-click on the conversation's words opens their menu.
        {
            let this = cx.weak_entity();
            selections
                .borrow_mut()
                .on_menu(std::rc::Rc::new(move |run, at, window, cx| {
                    let _ = this.update(cx, |chat, cx| chat.open_words_menu(run, at, window, cx));
                }));
        }
        let makings = cx.new(|cx| crate::making::Makings::new(session.clone(), window, cx));
        // Each turn draws the cards of what it asked to have made; the list's
        // foot only those heard as Kerf works, no turn holding them yet.
        makings.update(cx, |makings, cx| {
            makings.set_cards_in_turns(true, cx);
            // Their plans' words are chosen and copied with the conversation's.
            makings.share_selections(selections.clone());
        });
        let mut view = Self {
            session,
            composer,
            makings,
            scroll: ScrollHandle::new(),
            follow: true,
            scripts: HashSet::new(),
            folds: HashMap::new(),
            shown: HashMap::new(),
            book: ProposalBook::default(),
            queued: Queued::default(),
            stopping: false,
            copied: None,
            menu: None,
            escape_took_menu: false,
            words_menu: None,
            kit_menu: None,
            menu_closed: None,
            placeholder: PLACEHOLDER.into(),
            refusal: None,
            access: None,
            ask_reason: None,
            clock: None,
            fixes: Vec::new(),
            keys: None,
            ask: None,
            stage_proposal: None,
            picture_seen: None,
            scrub: None,
            pictures: Default::default(),
            building: None,
            built: None,
            turning: None,
            newest: None,
            footage: None,
            footage_wanted: false,
            turn_seen: None,
            started_at: Instant::now(),
            behind: Tween::at(1.0),
            selections,
            tiles: Default::default(),
            taken: HashSet::new(),
            composer_at: Default::default(),
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        view.refresh_placeholder(cx);
        view
    }

    pub fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.read(cx).focus(window);
    }

    /// Opens the Ask popover over `anchor` (in the window: the chosen clip
    /// on the timeline, else the timeline at the playhead), with the
    /// playhead's time as its title; its request goes about the chosen
    /// clip or the whole sequence ([`crate::ask::AskView`]). The window
    /// calls it for Ask Kerf… (`ai.ask`, ⌘J). A second call while it is
    /// open moves it.
    pub fn open_ask(
        &mut self,
        anchor: gpui::Bounds<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.clone();
        let ask = cx.new(|cx| crate::ask::AskView::new(session, anchor, window, cx));
        cx.subscribe(&ask, |this, ask, _: &gpui::DismissEvent, cx| {
            if this.ask.as_ref() == Some(&ask) {
                this.ask = None;
                cx.notify();
            }
        })
        .detach();
        ask.read(cx).focus(window, cx);
        self.ask = Some(ask);
        cx.notify();
    }

    /// Whether the Ask popover is open.
    pub fn asking(&self) -> bool {
        self.ask.is_some()
    }

    /// What Kerf does without asking, chosen here (the chip's menu or
    /// Shift-Tab): kept, and said with what it means in place of the mode
    /// said last, for [`MODE_HOLD`], as Qt said it on its one status line.
    fn set_permissions(&mut self, permissions: Permissions, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.prefs.permissions = permissions;
            session.save_prefs();
            say_permissions(&mut session.state, permissions);
            cx.notify();
        });
        cx.notify();
    }

    /// The keys a card's actions name, read again when the keymap changes.
    fn refresh_keys(&mut self, cx: &App) {
        let keymap = self.session.read(cx).prefs.keymap.wire().to_owned();
        if self.keys.as_ref().is_some_and(|(read, _)| *read == keymap) {
            return;
        }
        let catalog = crate::commands::catalog(&keymap);
        let keys = [
            "proposal.accept",
            "proposal.reject",
            "proposal.forward",
            "ai.stop",
        ]
        .into_iter()
        .filter_map(|id| {
            let command = catalog.iter().find(|command| command.id == id)?;
            let shown = crate::palette::shortcut(command);
            (!shown.is_empty()).then_some((id, shown))
        })
        .collect();
        self.keys = Some((keymap, keys));
    }

    /// The key command `id` has in the keymap, as a tip names it.
    fn key_of(&self, id: &str) -> Option<String> {
        self.keys.as_ref()?.1.get(id).cloned()
    }

    /// The request waiting for Kerf to finish, if one is.
    pub fn queued(&self) -> Option<&str> {
        self.queued.text()
    }

    /// The pinned proposal's id, and the id of the version its card shows.
    fn pinned(&self, cx: &App) -> Option<(String, String)> {
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
        let shown = snapshot
            .proposals
            .iter()
            .find(|proposal| proposal.proposal_id == pinned)
            .and_then(|proposal| {
                let shown = self.shown.get(&pinned)?;
                self.book
                    .versions_of(proposal)
                    .iter()
                    .any(|version| &version.proposal_id == shown)
                    .then(|| shown.clone())
            })
            .unwrap_or_else(|| pinned.clone());
        Some((pinned, shown))
    }

    /// The version id the pinned proposal's card shows: what its Accept, and
    /// Accept from the keyboard, takes.
    pub fn pinned_proposal(&self, cx: &App) -> Option<String> {
        self.pinned(cx).map(|(_, shown)| shown)
    }

    /// The proposal id the registry's `proposal.accept`, `proposal.reject`
    /// or `proposal.forward` acts on, as the pinned card's own buttons do:
    /// Accept takes the version the card shows, Reject and Bring forward the
    /// proposal. None with no open proposal, or for another command.
    pub fn proposal_target(&self, command: &str, cx: &App) -> Option<String> {
        let (proposal, shown) = self.pinned(cx)?;
        match command {
            "proposal.accept" => Some(shown),
            "proposal.reject" | "proposal.forward" => Some(proposal),
            _ => None,
        }
    }

    fn near_bottom(&self) -> bool {
        let offset = -f32::from(self.scroll.offset().y);
        let most = f32::from(self.scroll.max_offset().height);
        most - offset < 48.0
    }

    /// Why the composer cannot ask now, as last worked out; None when it can.
    fn cannot_ask(&self) -> Option<&Refusal> {
        self.refusal.as_ref()
    }

    /// Works out again whether a request can go, and says so in the
    /// composer's placeholder. The connection's access (its key, Kerf's
    /// account) is asked of the registry only when it changes.
    fn refresh_placeholder(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let busy = session.state.work.busy();
        let cloud = crate::app::cloud(cx);
        let access = (!busy).then(|| session.ai_access(cloud)).flatten();
        if self.access.as_ref() != Some(&access) {
            self.ask_reason = access.as_deref().and_then(access_reason);
            self.access = Some(access);
        }
        self.refusal = refusal(
            session.has_project(),
            // Connecting counts as not connected, as Qt's composer was
            // disabled until kerfd was ready.
            session.state.connection != Connection::Connected,
            busy,
            self.ask_reason.as_deref(),
        );
        let placeholder: SharedString = self
            .refusal
            .as_ref()
            .map_or(PLACEHOLDER, |refused| refused.placeholder)
            .into();
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
        if text.is_empty() {
            return;
        }
        if self.cannot_ask().is_some() {
            // A key or a choice saved in another window since counts.
            self.session
                .update(cx, |session, _| session.prefs.refresh());
            self.access = None;
            self.refresh_placeholder(cx);
        }
        if let Some(refused) = self.cannot_ask() {
            let reason = refused.reason.clone();
            self.session.update(cx, |session, cx| {
                session.state.notify(&reason, true);
                cx.notify();
            });
            return;
        }
        let session = self.session.read(cx);
        if session.state.work.busy() {
            // It goes with the playhead time the latest request was written at.
            let at = session.state.position;
            self.queued.push(&text, at);
            self.composer.update(cx, |composer, cx| composer.clear(cx));
            self.focus_composer(window, cx);
            cx.notify();
            return;
        }
        if self.ask(&text, None, cx) {
            self.composer.update(cx, |composer, cx| composer.clear(cx));
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Asks Kerf `text`, stamped `at` (the playhead time it was written at;
    /// None for now).
    fn ask(&mut self, text: &str, at: Option<f64>, cx: &mut Context<Self>) -> bool {
        let sent = self.session.update(cx, |session, cx| {
            let at = at.unwrap_or(session.state.position);
            let sent = session.ask_at(text, at);
            cx.notify();
            sent
        });
        if sent {
            self.follow = true;
        }
        sent
    }

    /// Asks Kerf `text` about what `about` went about ([`Session::ask_as`]).
    fn ask_as(&mut self, text: &str, about: Focused, cx: &mut Context<Self>) -> bool {
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.ask_as(text, about);
            cx.notify();
            sent
        });
        if sent {
            self.follow = true;
        }
        sent
    }

    /// Sends `text` again about what it was about the first time (the Ask
    /// popover's clip or scope, or the sequence frame it was asked at,
    /// [`Session::ask_again`]): at once, or once the turn under way ends,
    /// still about that ([`Queued::push_again`]).
    fn ask_again(&mut self, text: String, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if session.state.work.busy() {
            let at = session.state.position;
            let about = session.asked().of(&text).cloned();
            self.queued.push_again(&text, at, about);
        } else {
            let sent = self.session.update(cx, |session, cx| {
                let sent = session.ask_again(&text);
                cx.notify();
                sent
            });
            if sent {
                self.follow = true;
            }
        }
        cx.notify();
    }

    /// Stops Kerf's turn: the work row says "Stopping…" until it ends, and
    /// what waits goes back in the composer then.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.session.read(cx).state.work.busy() {
            self.stopping = true;
        }
        self.session.update(cx, |session, _| session.stop());
        cx.notify();
    }

    /// The turn ended: what waited goes now, unless the turn was stopped or
    /// failed, when it goes back in the composer to send or change.
    fn turn_ended(&mut self, cx: &mut Context<Self>) {
        let stopped = std::mem::take(&mut self.stopping);
        self.clock = None;
        let about = self.queued.about().cloned();
        let Some((text, at)) = self.queued.take() else {
            cx.notify();
            return;
        };
        // A stopped turn says "completed" once it ends: `stopping` keeps it.
        let troubled = {
            let session = self.session.read(cx);
            stopped
                || matches!(session.state.work.state.as_str(), "aborted" | "failed")
                || session.state.connection != Connection::Connected
        };
        // An Ask again that waited alone goes about what it went about.
        let sent = !troubled
            && match about {
                Some(about) => self.ask_as(&text, about, cx),
                None => self.ask(&text, Some(at), cx),
            };
        if sent {
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
    /// Puts `words` in the composer and gives it the keys, to be sent or
    /// changed first (a suggestion, Finish's and Export's Ask Kerf).
    pub fn fill(&mut self, words: String, window: &mut Window, cx: &mut Context<Self>) {
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

    fn toggle_menu(&mut self, menu: Menu, window: &mut Window, cx: &mut Context<Self>) {
        let closed_by_this_press = self.menu_closed.take().is_some_and(|(closed, at)| {
            closed == menu && at.elapsed() < Duration::from_millis(500)
        });
        if self.menu.as_ref() == Some(&menu) {
            self.menu = None;
            if let Some(kit) = self.kit_menu.take() {
                kit.update(cx, |kit, cx| kit.close(window, cx));
            }
        } else if !closed_by_this_press {
            if let Some(kit) = self.kit_menu.take() {
                kit.update(cx, |kit, cx| kit.close(window, cx));
            }
            self.open_kit_menu(&menu, window, cx);
            self.menu = Some(menu);
        }
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() {
            self.menu_closed = Some((menu, Instant::now()));
        }
        self.kit_menu = None;
        cx.notify();
    }

    /// The kit's menu for `menu` (all but the context card, which is a
    /// card): its keys, its letters, Esc and a press outside close it.
    fn open_kit_menu(&mut self, menu: &Menu, window: &mut Window, cx: &mut Context<Self>) {
        let (items, header, width): (Vec<ui::MenuItem>, Option<&'static str>, f32) = match menu {
            Menu::Context => return,
            Menu::Folder => (self.folder_items(cx), None, 240.0),
            Menu::Permissions => (
                self.permissions_items(cx),
                Some("What Kerf does without asking"),
                280.0,
            ),
            Menu::Model => {
                let provider = self.session.read(cx).prefs.provider;
                (self.model_items(cx), Some(provider.label()), 240.0)
            }
            Menu::Versions(id) => (self.version_items(id, cx), None, 260.0),
        };
        let kit = cx.new(|cx| {
            let kit = ui::Menu::new(items, window, cx).width(width);
            match header {
                Some(header) => kit.header(header),
                None => kit,
            }
        });
        let open = menu.clone();
        cx.subscribe(&kit, move |this, kit, _: &DismissEvent, cx| {
            if this.kit_menu.as_ref() == Some(&kit) {
                this.kit_menu = None;
                if this.menu.as_ref() == Some(&open) {
                    this.close_menu(cx);
                }
            }
        })
        .detach();
        self.kit_menu = Some(kit);
    }

    fn copy(&mut self, turn: String, words: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(words.trim().to_owned()));
        self.copied = Some(turn.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_HOLD).await;
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

    // Every turn reads the turns, proposals, holders and pin the view gathers once.
    #[allow(clippy::too_many_arguments)]
    fn turn(
        &self,
        index: usize,
        turns: &[Turn],
        proposals: &[Proposal],
        holders: &HashMap<String, usize>,
        pinned: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let turn = &turns[index];
        let id = SharedString::from(format!("turn-{}", turn.id));
        let body = match turn.speaker() {
            // A request can be copied, as an answer can.
            Speaker::You => {
                let copy = (!turn.text.trim().is_empty()).then(|| turn.text.clone());
                let actions = self.turn_actions(&turn.id, copy, None, cx);
                self.you(&turn.text, turn.timecode, &turn.id, cx)
                    .child(actions)
            }
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
                if let Some((named, proposal)) = turn.proposal.as_deref().and_then(|named| {
                    self.book
                        .find(proposals, named)
                        .map(|proposal| (named, proposal))
                }) {
                    // The newest turn that names it holds its card.
                    if holders.get(&proposal.proposal_id) == Some(&index) {
                        let pinned = pinned == Some(proposal.proposal_id.as_str());
                        column = column.child(self.proposal_card(proposal, pinned, window, cx));
                    } else {
                        column = column.child(self.proposal_named(proposal, named));
                    }
                }
                // What it asked to have made, a card each, under its answer.
                if !turn.asks.is_empty() {
                    let cards = self
                        .makings
                        .update(cx, |makings, cx| makings.cards_for(&turn.asks, cx));
                    column = column.children(cards);
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
    /// asked at over it, which goes there. Its words can be chosen with
    /// the pointer.
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
                let timecode = model::timecode(at, fps);
                // Its tip says where pressing it goes, as Qt's TimecodeChip's did.
                let tip = ui::Tip::new(stamp_tip(&timecode));
                column.child(ui::tip_on(
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
                        .child(timecode)
                        .on_click(move |_, window, cx| {
                            session.update(cx, |session, _| session.seek(at, false));
                            window.dispatch_action(Box::new(RevealPicture), cx);
                        }),
                    Some(tip),
                ))
            })
            .child(
                theme::reading(div())
                    .max_w(gpui::relative(0.8))
                    .px(px(14.0))
                    .py(px(9.0))
                    .rounded(px(theme::radius::PANEL))
                    .bg(theme::raised())
                    .text_color(theme::ink())
                    .child(selectable::selectable(
                        &self.selections,
                        ElementId::Name(format!("you-words-{key}").into()),
                        format!("you-{key}"),
                        0,
                        words.to_owned(),
                    )),
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
    /// Its words can be chosen with the pointer as one run, `key`'s.
    fn prose(&self, words: &str, key: &str, ink: Rgba, cx: &App) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(8.0));
        let mut code: Option<Vec<String>> = None;
        let mut paragraph: Vec<String> = Vec::new();
        // The pieces so far, and whether the last was a list's item.
        let mut piece = 0usize;
        let mut listing = false;
        for line in words.lines() {
            if line.trim_start().starts_with("```") {
                match code.take() {
                    Some(lines) => {
                        column = column.child(self.code_block(key, piece, lines));
                        piece += 1;
                    }
                    None => {
                        column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                        code = Some(Vec::new());
                    }
                }
                listing = false;
                continue;
            }
            if let Some(lines) = code.as_mut() {
                lines.push(line.to_owned());
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                listing = false;
            } else if let Some(item) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
            {
                column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                let before = if listing { "\n" } else { "\n\n" };
                column = column.child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(theme::reading(div()).text_color(theme::ink3()).child("•"))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.rich(key, piece, before, item, ink, cx)),
                        ),
                );
                piece += 1;
                listing = true;
            } else if let Some(heading) = trimmed.strip_prefix('#') {
                column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                let words: String = spans(heading.trim_start_matches('#').trim())
                    .into_iter()
                    .map(|span| match span {
                        Span::Plain(words)
                        | Span::Bold(words)
                        | Span::Code(words)
                        | Span::Timecode(words) => words,
                    })
                    .collect();
                column = column.child(
                    theme::type_style(div(), text::TITLE)
                        .text_color(theme::ink())
                        .child(selectable::selectable(
                            &self.selections,
                            ElementId::Name(format!("{key}-{piece}").into()),
                            key.to_owned(),
                            piece,
                            words,
                        )),
                );
                piece += 1;
                listing = false;
            } else {
                if listing {
                    column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                }
                paragraph.push(trimmed.to_owned());
                listing = false;
            }
        }
        if let Some(lines) = code {
            column = column.child(self.code_block(key, piece, lines));
            piece += 1;
        }
        self.flush(column, key, &mut piece, &mut paragraph, ink, cx)
    }

    /// The paragraph gathered so far, as a piece of `key`'s run.
    fn flush(
        &self,
        column: gpui::Div,
        key: &str,
        piece: &mut usize,
        paragraph: &mut Vec<String>,
        ink: Rgba,
        cx: &App,
    ) -> gpui::Div {
        if paragraph.is_empty() {
            return column;
        }
        let text = paragraph.join("\n");
        paragraph.clear();
        let element = self.rich(key, *piece, "\n\n", &text, ink, cx);
        *piece += 1;
        column.child(element)
    }

    /// A paragraph of what Kerf wrote, piece `piece` of `key`'s run (after
    /// `before` when copied with the one before): bold in the medium face,
    /// code in mono, and each timecode a mono link in accent ink that moves
    /// the playhead there and brings the picture in. Its words can be chosen.
    fn rich(
        &self,
        key: &str,
        piece: usize,
        before: &'static str,
        words: &str,
        ink: Rgba,
        cx: &App,
    ) -> AnyElement {
        let timeline = self.session.read(cx).timeline();
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
                if let Some(seconds) = timecode_position(timecode, timeline) {
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
        let mut words = selectable::selectable(
            &self.selections,
            ElementId::Name(format!("{key}-{piece}").into()),
            key.to_owned(),
            piece,
            shown,
        )
        .runs(runs)
        .before(before);
        if !links.is_empty() {
            // A press on a link that does not drag goes there.
            let session = self.session.clone();
            words = words.links(links, move |link, window, cx| {
                let Some(at) = moments.get(link).copied() else {
                    return;
                };
                session.update(cx, |session, cx| {
                    session.seek(at, false);
                    cx.notify();
                });
                window.dispatch_action(Box::new(RevealPicture), cx);
            });
        }
        theme::reading(div())
            .text_color(ink)
            .child(words)
            .into_any_element()
    }

    /// Code Kerf wrote, in mono on the well: piece `piece` of `key`'s run.
    fn code_block(&self, key: &str, piece: usize, lines: Vec<String>) -> gpui::Div {
        theme::type_style(div(), text::TIMECODE)
            .p(px(10.0))
            .rounded(px(theme::radius::CONTROL))
            .bg(theme::well())
            .text_color(theme::ink2())
            .child(selectable::selectable(
                &self.selections,
                ElementId::Name(format!("{key}-{piece}").into()),
                key.to_owned(),
                piece,
                lines.join("\n"),
            ))
    }

    // ------------------------------------------------------------ proposals

    /// A proposal a later turn holds the card of, where an earlier turn
    /// named it: the version it named, in a line ("v1, revised below").
    fn proposal_named(&self, proposal: &Proposal, named: &str) -> AnyElement {
        let versions = self.book.versions_of(proposal);
        let newest = versions
            .iter()
            .map(|version| version.version)
            .max()
            .unwrap_or(proposal.version);
        let (script, number) = versions
            .iter()
            .find(|version| version.proposal_id == named)
            .map_or((proposal.script.as_str(), proposal.version), |version| {
                (version.script.as_str(), version.version)
            });
        let words = if number < newest {
            format!("v{}, revised below", number.max(1))
        } else {
            "Shown below".to_owned()
        };
        div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(Icon::Film.view(12.0, theme::ink3()))
            .child(
                ui::line(script_summary(script, number), text::CONTROL, theme::ink2())
                    .flex_shrink()
                    .min_w_0(),
            )
            .child(ui::label("·", text::CONTROL, theme::ink3()).flex_none())
            .child(ui::label(words, text::CONTROL, theme::ink3()).flex_none())
            .into_any_element()
    }

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
            // Settled, it keeps to one line; settled here, it says when.
            let accepted = proposal.status == "accepted";
            let ago = session
                .settled_at(|sent| self.book.is(proposal, sent))
                .map(|at| at.elapsed());
            let words = settled_words(accepted, self.book.applied.get(&id).copied(), ago);
            let (icon, color) = if accepted {
                (Icon::Check, theme::accent_ink())
            } else {
                (Icon::Close, theme::ink2())
            };
            return div()
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(icon.view(12.0, color))
                .child(
                    ui::line(
                        script_summary(&proposal.script, proposal.version),
                        text::CONTROL,
                        theme::ink2(),
                    )
                    .flex_shrink()
                    .min_w_0(),
                )
                .child(ui::label("·", text::CONTROL, theme::ink3()).flex_none())
                .child(ui::label(words, text::CONTROL, color).flex_none())
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
        // A colour change on one clip is said in a line of its own.
        let colour =
            timeline.and_then(|proposed| colour_change(session.timeline(), proposed, &changes));
        let rows: Vec<(String, String, String)> = if changes.is_empty() {
            // Without its timeline, the script's lines say what it does, in words.
            version
                .script
                .lines()
                .map(str::trim)
                .filter(|line| {
                    !line.is_empty() && !line.starts_with('#') && !line.starts_with("use ")
                })
                .map(|line| {
                    let (said, detail) = describe_script_line(line, session.timeline(), timeline);
                    (said, detail, String::new())
                })
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
            && previewing(&session.state.preview.status);
        // Only what it changes, two seconds either side, one stretch after
        // another.
        let watch = timeline.filter(|_| !stale).and_then(|proposed| {
            let program = session.timeline();
            let segments = reel_plan(
                &changes,
                session.state.program_length().max(program.duration),
                proposed.duration.max(cut_frames(proposed).1),
                2.0,
            );
            (!segments.is_empty()).then_some(segments)
        });
        let changes_key = format!("changes-{id}");
        let changes_open = self.is_open(&changes_key);
        let script_open = self.scripts.contains(&id);
        let count = rows.len();
        // A first cut has its picture at the card's top.
        let first_cut = timeline
            .filter(|cut| !session.timeline().has_clips() && cut.has_clips())
            .cloned();

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
        if let Some(cut) = &first_cut {
            let state = self.picture_state(&id, cx);
            card = card.child(self.cut_picture(&id, cut, state, window, cx));
        }
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
        let toggle_title = changes_key.clone();
        card = card.child(
            div()
                .flex()
                .items_start()
                .gap(px(8.0))
                .child(div().pt(px(2.0)).child(Icon::Film.view(16.0, theme::pos())))
                .child(
                    // Pressed, the title opens or closes the card's changes.
                    ui::label(
                        script_summary(&version.script, version.version),
                        text::BODY_NAME,
                        theme::ink(),
                    )
                    .id(ElementId::Name(format!("title-{id}").into()))
                    .flex_1()
                    .min_w_0()
                    .when(count > 0, |title| {
                        title.cursor_pointer().on_click(
                            cx.listener(move |this, _, _, cx| this.toggle_fold(&toggle_title, cx)),
                        )
                    }),
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
                    row.child(ui::tip_on(
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
                        Some(ui::Tip::new(SCRIPT_TIP)),
                    ))
                })
                .child(div().flex_1())
                .when(versions.len() > 1, |row| {
                    row.child(self.versions_button(&id, &shown_id, &versions, cx))
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
                    .when_some(colour, |rows, (title, params)| {
                        rows.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(theme::space::S4))
                                .py(px(theme::space::S4))
                                .child(ui::line(title, text::CONTROL, theme::ink()))
                                .when(!params.is_empty(), |line| {
                                    line.child(ui::line(params, text::MONO_SMALL, theme::ink2()))
                                }),
                        )
                    })
                    .with_animation(
                        ElementId::Name(format!("{changes_key}-open").into()),
                        motion::animation(scene::FADE),
                        |rows, t| rows.opacity(t),
                    ),
            );
        }
        if script_open && !script.is_empty() {
            card = card.child(diff(&script, &id, &self.selections));
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
                    let mut button =
                        ui::button(ElementId::Name(format!("accept-{id}").into()), "Accept")
                            .small()
                            .icon(Icon::Check)
                            .tooltip("Accept")
                            .reason("Connect to start editing")
                            .disabled(!connected);
                    if let Some(key) = self.key_of("proposal.accept") {
                        button = button.key(key);
                    }
                    let button = button.on_click({
                        let session = self.session.clone();
                        move |_, _, cx| session.update(cx, |session, _| session.accept(&accept))
                    });
                    row.child(if pinned { button.primary() } else { button })
                })
                .when(stale, |row| {
                    let button = ui::button(
                        ElementId::Name(format!("forward-{id}").into()),
                        "Bring forward",
                    )
                    .small()
                    .tooltip("Out of date · run it again on the edit as it is…")
                    .reason("Connect to start editing")
                    .disabled(!connected)
                    .on_click({
                        let session = self.session.clone();
                        move |_, _, cx| session.update(cx, |session, _| session.forward(&forward))
                    });
                    row.child(if pinned { button.primary() } else { button })
                })
                .when_some(watch, |row, segments| {
                    let session = self.session.clone();
                    let proposal = shown_id.clone();
                    let seconds = reel_total(&segments).ceil() as i64;
                    row.child(
                        ui::button(
                            ElementId::Name(format!("watch-{id}").into()),
                            format!("Watch what changed · {seconds} s"),
                        )
                        .small()
                        .icon(Icon::Eye)
                        .tooltip("Plays only what this proposal changes")
                        .reason("Connect to start editing")
                        .disabled(!connected)
                        .on_click(move |_, window, cx| {
                            session.update(cx, |session, cx| {
                                session.watch_changes(&proposal, segments.clone());
                                cx.notify();
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
                    .tooltip(PREVIEW_TIP)
                    .reason("Connect to start editing")
                    .disabled(!connected)
                    .on_click({
                        let session = self.session.clone();
                        move |_, window, cx| {
                            // On, it shows what the proposal makes; off, the program again.
                            session.update(cx, |session, _| {
                                if preview_shown {
                                    session.send(Command::PreviewExit);
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
                        .tooltip("Reject")
                        .key(
                            self.key_of("proposal.reject")
                                .unwrap_or_else(|| "Esc".to_owned()),
                        )
                        .reason("Connect to start editing")
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
        cx: &Context<Self>,
    ) -> AnyElement {
        let menu = Menu::Versions(id.to_owned());
        let open = self.menu.as_ref() == Some(&menu);
        let shown = versions
            .iter()
            .find(|version| version.proposal_id == shown_id)
            .map_or(1, |version| version.version);
        let toggle = menu.clone();
        let button =
            theme::type_style(div(), text::MONO_SMALL)
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
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_menu(toggle.clone(), window, cx)
                }));
        // Its menu says all the tip would, so the tip waits while it is open.
        let button = ui::tip_on(button, (!open).then(|| ui::Tip::new(VERSIONS_TIP)));
        let mut wrapper = div().relative().flex_none().child(button);
        if let Some(body) = self.kit_menu.clone().filter(|_| open) {
            wrapper = wrapper.child(
                div().absolute().top_full().right_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopRight)
                            .offset(point(px(0.0), px(theme::space::S4)))
                            .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                            .child(body),
                    )
                    .with_priority(1),
                ),
            );
        }
        wrapper.into_any_element()
    }

    // ------------------------------------------------------------ while Kerf works

    /// Under Reduce Motion, the working clock's one timer: it draws the
    /// conversation again once each `CLOCK_FRAME` while Kerf works, and
    /// stops when it does not. Drawing more often starts no other.
    fn keep_clock(&mut self, cx: &mut Context<Self>) {
        if self.clock.is_some() {
            return;
        }
        self.clock = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(scene::CLOCK_FRAME).await;
            let going = this
                .update(cx, |this, cx| {
                    let busy = this.session.read(cx).state.work.busy();
                    if busy && motion::reduced() {
                        cx.notify();
                    } else {
                        this.clock = None;
                    }
                    busy && motion::reduced()
                })
                .unwrap_or(false);
            if !going {
                break;
            }
        }));
    }

    /// What Kerf is doing now, under your request: "Working", the time so
    /// far and the step it is on; its last steps; the first cut it is
    /// making, as a card at work; the footage it starts on; three dots
    /// until it says anything; and its answer as it streams.
    fn working(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let started = self.session.read(cx).state.work.started?;
        // The clock goes on once a second; the light 30 times a second while it shows.
        if motion::reduced() {
            self.keep_clock(cx);
        } else {
            window.request_animation_frame();
        }
        let session = self.session.read(cx);
        let work = &session.state.work;
        let clock = format!(" · {}", work_clock(started.elapsed().as_secs_f64()));
        let phase = motion::phase(started, scene::WORK_SHIMMER);
        let status = if self.stopping {
            "Stopping…".to_owned()
        } else {
            work.status.clone()
        };
        let mut label = ui::label("Working", text::CONTROL_STRONG, theme::ink2())
            .relative()
            .flex_none()
            .overflow_hidden();
        if let Some(at) = motion::shimmer_at(phase, 1.0).filter(|_| !motion::reduced()) {
            // A band of light, brightest at its middle, passing over the word.
            let half = |rising: bool| {
                let (from, to) = if rising { (0.0, 0.28) } else { (0.28, 0.0) };
                div().h_full().w(gpui::relative(0.5)).bg(linear_gradient(
                    90.0,
                    linear_color_stop(theme::with_alpha(theme::ink(), from), 0.0),
                    linear_color_stop(theme::with_alpha(theme::ink(), to), 1.0),
                ))
            };
            label = label.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(gpui::relative(at - 0.2))
                    .w(gpui::relative(0.4))
                    .flex()
                    .child(half(true))
                    .child(half(false)),
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
                .min_w_0()
                .child(label)
                .child(ui::label(clock, text::MONO_SMALL, theme::ink3()).flex_none())
                .when(!status.trim().is_empty(), |row| {
                    row.child(
                        ui::line(status, text::CONTROL, theme::ink3())
                            .ml(px(theme::space::S8))
                            .flex_1()
                            .min_w_0(),
                    )
                }),
        );
        if !steps.is_empty() {
            let latest = steps.len() - 1;
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .pl(px(10.0))
                    .border_l_1()
                    .border_color(theme::line2())
                    .children(steps.into_iter().enumerate().map(|(index, step)| {
                        let (icon, color) = match step.state {
                            StepState::Running => (Icon::Dot, theme::activity::kerf()),
                            StepState::Done => (Icon::Check, theme::ink3()),
                            StepState::Failed => (Icon::Close, theme::neg()),
                        };
                        let ink = match step.state {
                            StepState::Failed => theme::neg(),
                            _ if index == latest => theme::ink(),
                            _ => theme::ink2(),
                        };
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(icon.view(12.0, color))
                            .child(ui::line(step.text, text::CONTROL, ink))
                    })),
            );
        }
        // The first cut it is making: its card, at work.
        if let Some(card) = self.building_card(window, cx) {
            column = column.child(card);
        }
        // The footage it will choose from.
        if let Some(tiles) = self.footage_sheet(cx) {
            column = column.child(tiles);
        }
        if answer.trim().is_empty() {
            // Before it has said anything, three dots in turn.
            let lit = draft_dot(started.elapsed(), motion::reduced());
            column = column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .h(px(16.0))
                    .pl(px(2.0))
                    .children((0..3).map(|dot| {
                        let full = lit.is_none_or(|lit| lit == dot);
                        div()
                            .size(px(3.0))
                            .rounded_full()
                            .bg(theme::ink3())
                            .opacity(if full { 1.0 } else { 0.45 })
                    })),
            );
        } else {
            column = column.child(self.prose(&answer, "live", theme::ink(), cx));
        }
        Some(column.into_any_element())
    }

    /// Follows the turn and the first cut it makes: footage is wanted as
    /// a turn starts on footage with no cut, and goes when it ends; the
    /// cut being made is followed, and when it is done it says so.
    fn follow_turn(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let started = session.state.work.started;
        if started != self.turn_seen {
            self.turn_seen = started;
            self.footage = None;
            self.taken.clear();
            let timeline = session.timeline();
            self.footage_wanted = started.is_some()
                && !motion::reduced()
                && session.picture_open != Some(true)
                && !timeline.has_clips()
                && !timeline.sources.is_empty();
        }
        if started.is_none() {
            self.footage = None;
            self.footage_wanted = false;
        }
        let cutting = session
            .cutting()
            .then(|| {
                session
                    .first_cut()
                    .map(|(proposal, _)| proposal.proposal_id.clone())
            })
            .flatten();
        match cutting {
            Some(id) => self.building = Some(id),
            None => {
                if let Some(was) = self.building.take() {
                    self.built = Some((was, Instant::now()));
                    self.newest = None;
                    self.turning = None;
                }
            }
        }
        if self
            .built
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= CARD_DONE)
        {
            self.built = None;
        }
        // The picture went back from the stage: the card has it again. (A
        // card just pressed, the picture not yet in, keeps it.)
        let open = session.picture_open;
        if self.picture_seen == Some(true) && open == Some(false) {
            self.stage_proposal = None;
        }
        self.picture_seen = open;
    }

    /// The card of the first cut Kerf is making: its picture at work, its
    /// title, and nothing to do with it yet.
    fn building_card(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.building.clone()?;
        let (title, cut) = {
            let session = self.session.read(cx);
            let (proposal, cut) = session.first_cut()?;
            (
                script_summary(&proposal.script, proposal.version),
                cut.clone(),
            )
        };
        let state = self.building_state(&id, &cut, cx);
        let picture = self.cut_picture(&id, &cut, state, window, cx);
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .p(px(10.0))
                .rounded(px(theme::radius::CARD))
                .bg(theme::raised())
                .border_1()
                .border_color(theme::line())
                .child(picture)
                .child(
                    div()
                        .px(px(4.0))
                        .child(ui::line(title, text::BODY_NAME, theme::ink())),
                )
                .with_animation(
                    "building-card",
                    motion::animation(scene::APPEAR),
                    |card, t| card.opacity(t),
                )
                .into_any_element(),
        )
    }

    /// How cut `id`'s picture stands now: just done, on the stage, under
    /// the pointer.
    fn picture_state(&self, id: &str, cx: &App) -> PictureState {
        let session = self.session.read(cx);
        PictureState {
            on_stage: session.picture_open == Some(true)
                && self
                    .stage_proposal
                    .as_deref()
                    .is_some_and(|shown| self.book.same(shown, id)),
            scrub: self
                .scrub
                .as_ref()
                .filter(|(over, _)| over == id)
                .map(|(_, across)| *across),
            finished: self
                .built
                .as_ref()
                .filter(|(built, _)| self.book.same(built, id))
                .map(|(_, at)| at.elapsed()),
            ..PictureState::default()
        }
    }

    /// How a building cut's picture stands: at work, the newest clip
    /// placed turning in over the last.
    fn building_state(&mut self, id: &str, cut: &Timeline, cx: &App) -> PictureState {
        let mut state = PictureState {
            working: true,
            finished: None,
            ..self.picture_state(id, cx)
        };
        let session = self.session.read(cx);
        let (frames, _) = cut_frames(cut);
        let newest = frames
            .iter()
            .rev()
            .find(|(_, source)| session.artwork.poster(source).is_some())
            .map(|(_, source)| source.clone());
        if newest != self.newest {
            if let (Some(old), false) = (self.newest.take(), motion::reduced()) {
                self.turning = Some((old, Instant::now()));
            }
            self.newest = newest;
        }
        if let Some((before, at)) = &self.turning {
            let t = at.elapsed().as_secs_f32() / CARD_CROSSFADE.as_secs_f32();
            match session.artwork.poster(before).filter(|_| t < 1.0) {
                Some(poster) => state.before = Some((poster, motion::ease(t))),
                None => self.turning = None,
            }
        }
        state
    }

    /// The footage Kerf starts on, as named poster tiles popping in one
    /// after another; it goes when the turn ends.
    fn footage_sheet(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        let timeline = session.timeline();
        if self.footage_wanted && self.footage.is_none() {
            let posters: Vec<String> = timeline
                .sources
                .iter()
                .filter(|source| session.artwork.poster(&source.id).is_some())
                .take(FOOTAGE_MOST)
                .map(|source| source.id.clone())
                .collect();
            // A folder just brought in has its posters still to come.
            if !timeline.sources.is_empty()
                && posters.len() >= timeline.sources.len().min(FOOTAGE_MOST)
            {
                self.footage = Some((posters, Instant::now()));
                self.footage_wanted = false;
            }
        }
        let (ids, _) = self.footage.as_ref()?;
        let session = self.session.read(cx);
        let timeline = session.timeline();
        let tiles = ids.iter().enumerate().filter_map(|(index, id)| {
            let poster = session.artwork.poster(id)?;
            let name = timeline.media_name(id);
            let delay = scene::POP_STAGGER.as_secs_f32() * index as f32;
            let pop = scene::POP.as_secs_f32();
            let whole = Duration::from_secs_f32(delay + pop);
            // Gone on from here to become a clip, its place stays.
            let taken = self.taken.contains(id);
            let (tiles, key) = (self.tiles.clone(), id.clone());
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .w(px(FOOTAGE_TILE.0))
                    .when(taken, |tile| tile.invisible())
                    .child(
                        div()
                            .relative()
                            .w(px(FOOTAGE_TILE.0))
                            .h(px(FOOTAGE_TILE.1))
                            .rounded(px(theme::radius::SMALL))
                            .overflow_hidden()
                            .bg(theme::well())
                            .child(poster_layer(poster, theme::radius::SMALL))
                            // Where it is, for what goes on from it.
                            .child(
                                gpui::canvas(
                                    move |bounds, _, _| {
                                        tiles.borrow_mut().insert(key.clone(), bounds);
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0(),
                            ),
                    )
                    .child(ui::line(name, text::META, theme::ink2()))
                    .with_animation(
                        ElementId::Name(format!("footage-{id}").into()),
                        gpui::Animation::new(motion::duration(whole).max(Duration::from_millis(1))),
                        move |tile, t| {
                            let local = ((t * (delay + pop) - delay) / pop).clamp(0.0, 1.0);
                            let eased = motion::ease(local);
                            tile.opacity(eased)
                                .mt(px(scene::FOOTAGE_RISE * (1.0 - eased)))
                        },
                    ),
            )
        });
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(px(theme::space::S12))
                .children(tiles)
                .into_any_element(),
        )
    }

    /// A conversation not yet begun: where to work (the folders, the open
    /// one ticked, Open Folder… and New Project…), then the latest other
    /// conversations here, or with none, things to ask.
    fn empty(&self, cx: &mut Context<Self>) -> AnyElement {
        use crate::sidebar::Entry;
        let session = self.session.read(cx);
        let folders: Vec<_> =
            crate::sidebar::folder_entries(&session.prefs.recent, session.project.as_deref())
                .into_iter()
                .filter(|entry| matches!(entry.entry, Entry::Open(_) | Entry::Command(_)))
                .collect();
        let mut shown_folders = 0;
        let mut folder_rows = Vec::new();
        for (index, entry) in folders.into_iter().enumerate() {
            if matches!(entry.entry, Entry::Open(_)) {
                shown_folders += 1;
                if shown_folders > 4 {
                    continue;
                }
            }
            // No line over the card's first row.
            if entry.separated && !folder_rows.is_empty() {
                folder_rows.push(
                    div()
                        .my(px(4.0))
                        .child(ui::hairline(true))
                        .into_any_element(),
                );
            }
            let chosen = entry.checked;
            let run = entry.entry.clone();
            let icon = entry.icon;
            let row = welcome_row(
                ElementId::Name(format!("welcome-folder-{index}").into()),
                icon,
                entry.words,
                chosen,
            )
            .on_click(cx.listener(move |_, _, window, cx| match &run {
                Entry::Open(folder) if !chosen => window.dispatch_action(
                    Box::new(crate::commands::OpenRecent {
                        path: folder.to_string_lossy().into_owned().into(),
                    }),
                    cx,
                ),
                Entry::Command(id) => window.dispatch_action(Box::new(RunCommand::new(id)), cx),
                _ => {}
            }));
            folder_rows.push(match entry.tip {
                Some(tip) => ui::tipped(
                    ElementId::Name(format!("welcome-folder-tip-{index}").into()),
                    ui::Tip::new(tip),
                    row.w_full(),
                )
                .into_any_element(),
                None => row.into_any_element(),
            });
        }
        let recent: Vec<(String, String)> = recent_conversations(
            &session.state.conversations,
            session.conversation.as_deref(),
            3,
        )
        .into_iter()
        .map(|summary| (summary.id.clone(), summary.label().to_owned()))
        .collect();
        let second: Vec<AnyElement> = if recent.is_empty() {
            try_asking(&session.state.next_steps)
                .into_iter()
                .enumerate()
                .map(|(index, step)| {
                    let request = step.request;
                    welcome_row(
                        ElementId::Name(format!("suggest-{index}").into()),
                        Some(Icon::Sparkle),
                        step.label,
                        false,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.fill(request.clone(), window, cx)
                    }))
                    .into_any_element()
                })
                .collect()
        } else {
            recent
                .into_iter()
                .map(|(id, title)| {
                    let open = id.clone();
                    let row = welcome_row(
                        ElementId::Name(format!("welcome-conversation-{id}").into()),
                        None,
                        title.clone(),
                        false,
                    )
                    .w_full()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.session.update(cx, |session, cx| {
                            session.open_conversation(Some(open.clone()));
                            cx.notify();
                        });
                        this.focus_composer(window, cx);
                    }));
                    ui::tipped(
                        ElementId::Name(format!("welcome-conversation-tip-{id}").into()),
                        ui::Tip::new(title),
                        row,
                    )
                    .into_any_element()
                })
                .collect()
        };
        let card = |rows: Vec<AnyElement>| {
            div()
                .flex()
                .flex_col()
                .p(px(theme::space::S4))
                .rounded(px(theme::radius::PANEL))
                .bg(theme::card())
                .border_1()
                .border_color(theme::line())
                .children(rows)
        };
        div()
            .flex()
            .justify_center()
            .pt(px(96.0))
            .child(
                div()
                    .w(px(WELCOME_COLUMN))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S12))
                    .child(card(folder_rows))
                    .child(card(second)),
            )
            .with_animation("empty-rise", motion::animation(scene::RISE), |empty, t| {
                empty.opacity(t).mt(px(scene::PAGE_RISE * (1.0 - t)))
            })
            .into_any_element()
    }

    // ------------------------------------------------------------ the composer

    /// A chip in the composer's row that opens its menu over it: its icon
    /// (if any) and words, and what a hover over it says.
    fn menu_chip(
        &self,
        menu: Menu,
        id: &'static str,
        (icon, words, tip): (Option<Icon>, String, ui::Tip),
        tone: Option<Rgba>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let open = self.menu.as_ref() == Some(&menu);
        let toggle = menu.clone();
        // A chip with a tone of its own (the permission mode's) keeps it,
        // open or not, its icon too, as Qt's ComposerChip painted it.
        let ink = match tone {
            Some(tone) => tone,
            None if open => theme::ink(),
            None => theme::ink3(),
        };
        // Where the row is short of room the model's words are cut
        // ("OpenAI · fake-mo…"), as Qt elided its chips'; the folder's are
        // cut only past `FOLDER_CHIP_MOST`, and the mode is said whole.
        let shrink = if menu == Menu::Model { 1.0 } else { 0.0 };
        let chip =
            theme::type_style(div(), text::META)
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(4.0))
                .h(px(20.0))
                .pl(px(8.0))
                .rounded(px(10.0))
                .bg(theme::raised())
                .text_color(ink)
                .when_some(icon, |chip, icon| {
                    chip.child(
                        div()
                            .flex_none()
                            .child(icon.view(12.0, tone.unwrap_or(theme::ink3()))),
                    )
                })
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(ui::cut_line(words)),
                )
                .id(id)
                .pr(px(4.0))
                .cursor_pointer()
                .hover(move |chip| {
                    let chip = chip.bg(theme::hover2());
                    if tone.is_some() {
                        chip
                    } else {
                        chip.text_color(theme::ink2())
                    }
                })
                .child(
                    div()
                        .flex_none()
                        .child(Icon::ChevronDown.view(12.0, theme::ink3())),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_menu(toggle.clone(), window, cx)
                }));
        // Its menu says all the tip would, so the tip waits while it is open.
        let chip = ui::tip_on(chip, (!open).then_some(tip));
        let mut wrapper = div()
            .relative()
            .min_w_0()
            .when(menu == Menu::Folder, |wrapper| {
                wrapper.max_w(px(FOLDER_CHIP_MOST))
            })
            .child(chip);
        wrapper.style().flex_shrink = Some(shrink);
        if let Some(body) = self.kit_menu.clone().filter(|_| open) {
            wrapper = wrapper.child(
                // Over the chip, its left edge on the chip's, over everything.
                div().absolute().bottom_full().left_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::BottomLeft)
                            .offset(point(px(0.0), px(-theme::space::S4)))
                            .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                            .child(body),
                    )
                    .with_priority(1),
                ),
            );
        }
        wrapper.into_any_element()
    }

    /// How full Kerf's context is, beside Send: its ring, and pressed, its
    /// card ("Context 34%", a bar, what that is of) over it.
    fn context_meter(&self, context: ContextUse, cx: &Context<Self>) -> AnyElement {
        let open = self.menu.as_ref() == Some(&Menu::Context);
        let color = context_color(context.level());
        let fraction = context.fraction() as f32;
        let meter = div()
            .id("context-meter")
            .size(px(24.0))
            .flex_none()
            .rounded(px(theme::radius::CONTROL))
            .cursor_pointer()
            .hover(|meter| meter.bg(theme::hover2()))
            .child(context_ring(fraction, color))
            .on_click(
                cx.listener(|this, _, window, cx| this.toggle_menu(Menu::Context, window, cx)),
            );
        let meter = if open {
            meter.into_any_element()
        } else {
            ui::tipped("context-meter-tip", ui::Tip::new(context.describe()), meter)
                .into_any_element()
        };
        let mut wrapper = div().relative().flex_none().child(meter);
        if open {
            let card = menu_frame(px(260.0), cx)
                .p(px(theme::space::S12))
                .gap(px(theme::space::S8))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(ui::label("Context", text::MENU_ITEM, theme::ink()).flex_1())
                        .child(ui::label(
                            format!("{}%", context.percent()),
                            text::MENU_QUIET,
                            theme::ink3(),
                        )),
                )
                .child(
                    div()
                        .h(px(4.0))
                        .w_full()
                        .rounded_full()
                        .bg(theme::track())
                        .child(
                            div()
                                .h_full()
                                .min_w(px(4.0))
                                .w(gpui::relative(fraction))
                                .rounded_full()
                                .bg(color),
                        ),
                )
                .child(
                    theme::type_style(div(), text::MENU_QUIET)
                        .text_color(theme::ink3())
                        .child(context.note()),
                );
            wrapper = wrapper.child(
                div().absolute().bottom_full().right_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::BottomRight)
                            .offset(point(px(0.0), px(-theme::space::S4)))
                            .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                            .child(fade_in(card, "context-card".to_owned())),
                    )
                    .with_priority(1),
                ),
            );
        }
        wrapper.into_any_element()
    }

    /// The folder chip's menu: the folders (the open one ticked), Open
    /// Folder… and New Project…. Another folder opens in place of this one.
    fn folder_items(&self, cx: &App) -> Vec<ui::MenuItem> {
        use crate::sidebar::Entry;
        let session = self.session.read(cx);
        let entries =
            crate::sidebar::folder_entries(&session.prefs.recent, session.project.as_deref());
        entries
            .into_iter()
            .filter(|entry| matches!(entry.entry, Entry::Open(_) | Entry::Command(_)))
            .map(|entry| {
                let chosen = entry.checked;
                let run = entry.entry.clone();
                let mut item =
                    ui::MenuItem::new(entry.words)
                        .checked(chosen)
                        .on_choose(move |window, cx| match &run {
                            Entry::Open(folder) if !chosen => window.dispatch_action(
                                Box::new(crate::commands::OpenRecent {
                                    path: folder.to_string_lossy().into_owned().into(),
                                }),
                                cx,
                            ),
                            Entry::Command(id) => {
                                window.dispatch_action(Box::new(RunCommand::new(id)), cx)
                            }
                            _ => {}
                        });
                if entry.separated {
                    item = item.separated();
                }
                if let Some(tip) = entry.tip {
                    item = item.tip(tip);
                }
                item
            })
            .collect()
    }

    /// "What Kerf does without asking": the three modes, each with what it
    /// means in its tip.
    fn permissions_items(&self, cx: &mut Context<Self>) -> Vec<ui::MenuItem> {
        let current = self.session.read(cx).prefs.permissions;
        let this = cx.entity().downgrade();
        Permissions::ALL
            .iter()
            .map(|permissions| {
                let permissions = *permissions;
                let (label, line) = permission_words(permissions);
                let this = this.clone();
                ui::MenuItem::new(label)
                    .checked(permissions == current)
                    .tip(line)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |this, cx| this.set_permissions(permissions, cx));
                    })
            })
            .collect()
    }

    /// The models that can be chosen, Agent settings…, and whether a request
    /// carries the playhead time.
    fn model_items(&self, cx: &App) -> Vec<ui::MenuItem> {
        let session = self.session.read(cx);
        let provider = session.prefs.provider;
        let configured = session.prefs.model().to_owned();
        let current = if configured.is_empty() {
            "default".to_owned()
        } else {
            configured.clone()
        };
        // Claude Code's aliases are chosen here. A key's model is the one
        // typed in Settings: it is said, and not offered as a choice.
        let choosable = provider == Provider::ClaudeCode;
        let stamped = session.prefs.send_playhead;
        let mut items: Vec<ui::MenuItem> = model_choices(provider, &configured)
            .into_iter()
            .filter(|model| choosable || *model == current)
            .map(|model| {
                let chosen = model == current;
                let session = self.session.clone();
                ui::MenuItem::new(model_label(&model))
                    .checked(chosen)
                    .disabled(!choosable)
                    .on_choose(move |_, cx| {
                        let model = model.clone();
                        session.update(cx, |session, cx| {
                            session.prefs.claude_code_model = if model == "default" {
                                String::new()
                            } else {
                                model
                            };
                            session.save_prefs();
                            cx.notify();
                        });
                    })
            })
            .collect();
        items.push(
            ui::MenuItem::new("Agent settings…")
                .separated()
                .on_choose(|window, cx| {
                    window.dispatch_action(Box::new(RunCommand::new("app.preferences")), cx)
                }),
        );
        let session = self.session.clone();
        items.push(
            ui::MenuItem::new("Send playhead time")
                .checked(stamped)
                .tip("A request says where the playhead is")
                .on_choose(move |_, cx| {
                    session.update(cx, |session, cx| {
                        session.prefs.send_playhead = !session.prefs.send_playhead;
                        session.save_prefs();
                        cx.notify();
                    });
                }),
        );
        items
    }

    /// A card's versions ("v2 00:00:04:12 · +6 fr"), the one shown ticked:
    /// one chosen is shown on the card, previewed, and the picture comes in.
    fn version_items(&self, id: &str, cx: &mut Context<Self>) -> Vec<ui::MenuItem> {
        let session = self.session.read(cx);
        let snapshot = &session.state.snapshot;
        let Some(proposal) = self.book.find(&snapshot.proposals, id) else {
            return Vec::new();
        };
        let versions = self.book.versions_of(proposal);
        let shown_id = self.shown.get(id).cloned().unwrap_or_else(|| id.to_owned());
        let program = session.timeline().clone();
        let fps = program.rate();
        let this = cx.entity().downgrade();
        versions
            .iter()
            .map(|version| {
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
                let this = this.clone();
                ui::MenuItem::new(label)
                    .checked(version.proposal_id == shown_id)
                    .on_choose(move |window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            if chosen == proposal {
                                this.shown.remove(&proposal);
                            } else {
                                this.shown.insert(proposal.clone(), chosen.clone());
                            }
                            let chosen = chosen.clone();
                            this.session
                                .update(cx, |session, _| session.preview(Some(&chosen)));
                            cx.notify();
                        });
                        window.dispatch_action(Box::new(RevealPicture), cx);
                    })
            })
            .collect()
    }

    fn composer(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let busy = session.state.work.busy();
        let empty = self.composer.read(cx).text().trim().is_empty();
        let typing_in = self.composer.read(cx).focus_handle(cx).is_focused(window);
        let refused = self.cannot_ask().map(|refused| refused.reason.clone());
        let refused_now = refused.clone();
        let provider = session.prefs.provider;
        let model = session.prefs.model().to_owned();
        let permissions = session.prefs.permissions;
        let context = session.state.context_shown().cloned();
        // What to ask next: Kerf's next steps, then what the last check found
        // failing; each fills the composer to be changed before it is sent.
        let mut next: Vec<(String, String)> = session
            .state
            .next_steps
            .iter()
            .map(|step| (step.label.clone(), step.request.clone()))
            .collect();
        next.extend(self.fixes.iter().cloned());
        // Run QC is offered while the box waits empty for words.
        let offer_qc = typing_in && empty && refused_now.is_none();
        // With the stamp taken off (and a picture to stamp), its chip says so
        // with the live time, struck through; pressed, the time goes again.
        let unstamped = (!session.prefs.send_playhead && session.picture_open != Some(false))
            .then(|| model::timecode(session.state.position, session.timeline().rate()));
        let model_words = if model.is_empty() {
            provider.label().to_owned()
        } else {
            format!("{} · {}", provider.label(), model_label(&model))
        };
        // Where there is a choice it is named as one.
        let model_tip = if provider == Provider::ClaudeCode {
            format!(
                "Model · {}",
                model_label(if model.is_empty() { "default" } else { &model })
            )
        } else {
            format!("Answers come from {model_words}")
        };
        // Where a request goes: the open folder, or the way to one.
        let (folder_words, folder_tip) = match session.project.as_deref() {
            Some(project) => (
                project
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| project.display().to_string()),
                project.display().to_string(),
            ),
            None => ("No folder".to_owned(), "Open a folder".to_owned()),
        };
        let (mode_words, mode_line) = permission_words(permissions);
        let (mode_icon, mode_tone) = permission_look(permissions);
        let mode_tip = ui::Tip::new(format!("{mode_words} · {mode_line}")).key("⇧Tab");
        let queued = self.queued.text().map(str::to_owned);
        let stopping = self.stopping;
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .when((!next.is_empty() || offer_qc) && !busy, |column| {
                column.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.0))
                        // A press here leaves the keys in the box, as Qt's
                        // list of suggestions did: Run QC, offered only while
                        // the box has them, stays under the pointer to be let
                        // go on, and the row does not shift as it goes.
                        .on_any_mouse_down(|_, window, _| window.prevent_default())
                        .children(
                            next.into_iter()
                                .enumerate()
                                .map(|(index, (words, request))| {
                                    ui::button(
                                        ElementId::Name(format!("next-{index}").into()),
                                        words,
                                    )
                                    .small()
                                    .icon(Icon::Sparkle)
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.fill(request.clone(), window, cx)
                                        },
                                    ))
                                }),
                        )
                        .when(offer_qc, |row| {
                            row.child(
                                ui::button("run-qc", "Run QC on the whole sequence")
                                    .small()
                                    .icon(Icon::Check)
                                    .on_click(cx.listener(|_, _, window, cx| {
                                        window.dispatch_action(
                                            Box::new(RunCommand::new("qc.run")),
                                            cx,
                                        )
                                    })),
                            )
                        }),
                )
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
                                    this.queued.clear();
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
                    .key_context(COMPOSER_CONTEXT)
                    .on_action(cx.listener(|this, _: &CyclePermissions, _, cx| {
                        let next = next_permissions(this.session.read(cx).prefs.permissions);
                        this.set_permissions(next, cx);
                    }))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .rounded(px(theme::radius::CARD + 4.0))
                    .bg(theme::raised())
                    .border_1()
                    .border_color(theme::line2())
                    .child({
                        // Refused, a hover over the words says why.
                        let field = div()
                            .id("composer-field")
                            .w_full()
                            .child(self.composer.clone());
                        match refused.clone() {
                            Some(reason) => {
                                ui::tipped("composer-field-tip", ui::Tip::new(reason), field)
                                    .into_any_element()
                            }
                            None => field.into_any_element(),
                        }
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(self.menu_chip(
                                Menu::Folder,
                                "folder-chip",
                                (Some(Icon::Folder), folder_words, ui::Tip::new(folder_tip)),
                                None,
                                cx,
                            ))
                            .when_some(unstamped, |row, timecode| {
                                row.child(stamp_chip(timecode, cx))
                            })
                            .child(self.menu_chip(
                                Menu::Model,
                                "model-chip",
                                (None, model_words, ui::Tip::new(model_tip)),
                                None,
                                cx,
                            ))
                            .child(self.menu_chip(
                                Menu::Permissions,
                                "permissions-chip",
                                (Some(mode_icon), mode_words.to_owned(), mode_tip),
                                mode_tone,
                                cx,
                            ))
                            .child(div().flex_1())
                            .when_some(context, |row, context| {
                                row.child(self.context_meter(context, cx))
                            })
                            .child(if busy {
                                icon_button("stop", Icon::Stop)
                                    .tooltip(if stopping { "Stopping…" } else { "Stop Kerf" })
                                    .key("Esc")
                                    .color(theme::ink())
                                    .disabled(stopping)
                                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                                    .into_any_element()
                            } else {
                                let live = !empty && refused.is_none();
                                let tip = match refused.clone() {
                                    Some(reason) => ui::Tip::new(reason),
                                    None => ui::Tip::new("Send").key("↩"),
                                };
                                let send = div()
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
                                    );
                                ui::tipped("send-tip", tip, send).into_any_element()
                            }),
                    ),
            )
            .into_any_element()
    }
}

/// How wide a new conversation's column of folders and conversations is.
pub use crate::theme::layout::WELCOME_COLUMN;

/// A row of a new conversation's cards: an icon where it has one, its
/// words, a tick where it is the one open.
fn welcome_row(
    id: ElementId,
    icon: Option<Icon>,
    words: String,
    ticked: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.0))
        .h(px(32.0))
        .px(px(10.0))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .hover(|row| row.bg(theme::hover2()))
        .when_some(icon, |row, icon| row.child(icon.view(14.0, theme::ink3())))
        .child(
            ui::line(words, text::MENU_QUIET, theme::ink())
                .flex_1()
                .min_w_0(),
        )
        .when(ticked, |row| {
            row.child(Icon::Check.view(14.0, theme::ink2()))
        })
}

/// The conversation while what a proposal changed is watched: it falls
/// back, the picture looked at (Qt's `chatBehind`).
pub use crate::theme::motion::CHAT_BEHIND;

/// The change reel's strip, to float over the picture while it plays:
/// "2 / 5", what the segment is with the seconds left ("· getting the
/// picture" while it waits, "Watched every change" at its end), previous
/// and next, Before|After, and Exit. None while no reel plays. The window
/// places it over the picture's foot (`StageView`).
pub fn reel_strip(session: &Entity<Session>, cx: &App) -> Option<AnyElement> {
    let reel = session.read(cx).reel()?;
    let playhead = session.read(cx).state.position;
    let count = reel.segments.len();
    let index = reel.index;
    let side = reel.side;
    let words = reel.words(playhead);
    let step = |forward: bool| {
        let session = session.clone();
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            session.update(cx, |session, cx| {
                session.reel_step(forward);
                cx.notify();
            })
        }
    };
    let sides = {
        let session = session.clone();
        ui::segmented(
            "reel-sides",
            &[
                (crate::session::ReelSide::Before, "Before"),
                (crate::session::ReelSide::After, "After"),
            ],
            side,
            move |side, _, cx| {
                session.update(cx, |session, cx| {
                    session.reel_side(side);
                    cx.notify();
                })
            },
        )
        .tips(&["The program as it is", "What the proposal makes"])
    };
    let exit = {
        let session = session.clone();
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            session.update(cx, |session, cx| {
                session.reel_stop();
                cx.notify();
            })
        }
    };
    Some(
        div()
            .id("change-reel")
            .h(px(40.0))
            .pl(px(theme::space::S12))
            .pr(px(theme::space::S8))
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .rounded(px(theme::radius::PANEL))
            .bg(theme::with_alpha(theme::chrome(), 0.94))
            .border_1()
            .border_color(theme::line2())
            .occlude()
            .child(ui::label(
                format!("{} / {count}", index + 1),
                text::TIMECODE,
                theme::accent_ink(),
            ))
            .child(
                ui::line(words, text::CONTROL, theme::ink2())
                    .flex_1()
                    .min_w_0(),
            )
            .child(
                icon_button("reel-previous", Icon::ChevronLeft)
                    .tooltip("Previous change")
                    .disabled(index == 0)
                    .on_click(step(false)),
            )
            .child(
                icon_button("reel-next", Icon::ChevronRight)
                    .tooltip("Next change")
                    .disabled(index + 1 >= count)
                    .on_click(step(true)),
            )
            .child(sides)
            .child(
                ui::button("reel-exit", "Exit")
                    .small()
                    .ghost()
                    .on_click(exit),
            )
            .with_animation(
                "change-reel-in",
                motion::animation(scene::FADE),
                |strip, t| strip.opacity(t),
            )
            .into_any_element(),
    )
}

/// The stamp: the request goes without the playhead's time, which shows
/// struck through; pressed, the time goes with requests again.
fn stamp_chip(timecode: String, cx: &Context<ChatView>) -> AnyElement {
    let chip = theme::type_style(div(), text::TIMECODE)
        .id("composer-stamp")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .h(px(20.0))
        .px(px(6.0))
        .rounded(px(theme::radius::CONTROL))
        .text_color(theme::ink3())
        .cursor_pointer()
        .hover(|chip| chip.bg(theme::hover2()))
        .child(Icon::Clock.view(12.0, theme::ink3()))
        .child(div().line_through().child(timecode))
        .on_click(cx.listener(|this, _, _, cx| {
            this.session.update(cx, |session, cx| {
                session.prefs.send_playhead = true;
                session.save_prefs();
                cx.notify();
            });
            cx.notify();
        }));
    ui::tipped(
        "composer-stamp-tip",
        ui::Tip::new("Time not sent · click to send"),
        chip,
    )
    .into_any_element()
}

/// The context ring: a 14 pt circle filled clockwise from the top as the
/// conversation fills what a request can hold, in `color`.
fn context_ring(fraction: f32, color: Rgba) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let across = 14.0;
            let stroke = 2.0;
            let center = bounds.center();
            let radius = (across - stroke) / 2.0;
            let ring = |from: f32, to: f32| {
                let steps = ((to - from) * 72.0).ceil().max(2.0) as usize;
                (0..=steps)
                    .map(|step| {
                        let turn = from + (to - from) * step as f32 / steps as f32;
                        let angle = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                        point(
                            center.x + px(radius * angle.cos()),
                            center.y + px(radius * angle.sin()),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let mut track = gpui::PathBuilder::stroke(px(stroke));
            track.add_polygon(&ring(0.0, 1.0), true);
            if let Ok(path) = track.build() {
                window.paint_path(path, theme::track());
            }
            if fraction > 0.0 {
                let mut arc = gpui::PathBuilder::stroke(px(stroke));
                arc.add_polygon(&ring(0.0, fraction.max(0.03)), false);
                if let Ok(path) = arc.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .size(px(24.0))
}

/// The ring's colour for how full the context is.
fn context_color(level: ContextLevel) -> Rgba {
    match level {
        ContextLevel::Room => theme::ink2(),
        ContextLevel::Filling => theme::warn(),
        ContextLevel::Full => theme::neg(),
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

/// What opens from a press, fading in over `FADE` as it rises
/// `MENU_RISE` into place, as the kit's menus do.
fn fade_in(body: impl IntoElement, id: String) -> impl IntoElement {
    div().relative().child(body).with_animation(
        ElementId::Name(id.into()),
        motion::animation(scene::FADE),
        |menu, t| menu.opacity(t).top(px(scene::MENU_RISE * (1.0 - t))),
    )
}

/// A proposal's changed lines, its additions sage and removals brick;
/// they can be chosen and copied as the conversation's words can (Qt's
/// script was selectable), one line after another.
fn diff(lines: &[String], id: &str, shared: &selectable::Shared) -> impl IntoElement {
    let run: SharedString = format!("diff-{id}").into();
    div()
        .id(ElementId::Name(format!("diff-{id}").into()))
        .max_h(px(260.0))
        .overflow_y_scroll()
        .p(px(8.0))
        .rounded(px(theme::radius::CONTROL))
        .bg(theme::well())
        .children(lines.iter().take(400).enumerate().map(|(index, line)| {
            let color = if line.starts_with('+') {
                theme::pos()
            } else if line.starts_with('-') {
                theme::neg()
            } else if line.starts_with("@@") {
                theme::accent_ink()
            } else {
                theme::ink2()
            };
            theme::type_style(div(), text::MONO_SMALL)
                .text_color(color)
                .child(
                    selectable::selectable(
                        shared,
                        ElementId::Name(format!("{run}-{index}").into()),
                        run.clone(),
                        index,
                        line.clone(),
                    )
                    .before("\n"),
                )
        }))
}

// ---------------------------------------------------------------- a cut's picture

/// A cut card's timings (Qt's cardTurn, cardCrossfade, cardRing, cardDone)
/// and sizes, kept in the theme.
pub use crate::theme::layout::{
    CARD_PICTURE_RING, CARD_PICTURE_TALLEST, CARD_PICTURE_WIDEST, CARD_PLAY_MARK, FOOTAGE_TILE,
};
pub use crate::theme::motion::{
    CARD_CROSSFADE, CARD_DONE, CARD_DONE_FADE, CARD_MARKS_AFTER, CARD_MARKS_FADE, CARD_RING,
    CARD_TURN,
};
/// The most footage tiles that gather.
pub const FOOTAGE_MOST: usize = 8;

/// A first cut's clips, in time order, on its first picture track that
/// has any: where each starts and the media it shows; and how long the
/// whole cut is, every track counted.
pub fn cut_frames(cut: &Timeline) -> (Vec<(f64, String)>, f64) {
    let duration = cut
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| clip.end())
        .fold(0.0, f64::max);
    let mut frames: Vec<(f64, String)> = cut
        .tracks
        .iter()
        .find(|track| track.kind == model::TrackKind::Video && !track.clips.is_empty())
        .map(|track| {
            track
                .clips
                .iter()
                .map(|clip| {
                    (
                        clip.timeline_start,
                        clip.source_id.clone().unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    frames.sort_by(|a, b| a.0.total_cmp(&b.0));
    (frames, duration)
}

/// Which of a cut's frames (in time order, each with whether its poster
/// has come) its picture shows: while Kerf works, the newest with a
/// poster; else the one under the pointer (`scrub`, 0..1 across), or the
/// first. None until a poster has come.
pub fn shown_frame(
    frames: &[(f64, bool)],
    duration: f64,
    scrub: Option<f32>,
    working: bool,
) -> Option<usize> {
    if working {
        return frames.iter().rposition(|(_, poster)| *poster);
    }
    if !frames.iter().any(|(_, poster)| *poster) {
        return None;
    }
    let at = scrub.map_or(0.0, |scrub| f64::from(scrub.clamp(0.0, 0.999)) * duration);
    let mut shown = 0;
    for (index, (start, poster)) in frames.iter().enumerate() {
        if *start <= at && *poster {
            shown = index;
        }
    }
    Some(shown)
}

/// The length badge on a cut's picture: "0:42", "12:05".
pub fn length_badge(seconds: f64) -> String {
    let whole = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// How a finished cut's picture stands `since` it was done: how much it
/// is still at work (its shade and ring going out over `CARD_RING`), the
/// ring's light, Done's opacity (gone over the last `CARD_DONE_FADE`), and
/// the length and play mark's (coming after `CARD_MARKS_AFTER`). Past
/// `CARD_DONE` it simply stands, its marks on. As drawn, see
/// [`finish_drawn`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finish {
    pub work: f32,
    pub ring: f32,
    pub chip: f32,
    pub marks: f32,
}

pub fn finishing(since: Duration) -> Finish {
    let ramp = |from: Duration, length: Duration| {
        if length.is_zero() {
            return if since >= from { 1.0 } else { 0.0 };
        }
        (since.saturating_sub(from).as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
    };
    if since >= CARD_DONE {
        return Finish {
            work: 0.0,
            ring: 0.0,
            chip: 0.0,
            marks: 1.0,
        };
    }
    let out = ramp(Duration::ZERO, CARD_RING);
    Finish {
        work: 1.0 - out,
        ring: 1.0 - out,
        chip: 1.0 - ramp(CARD_DONE - CARD_DONE_FADE, CARD_DONE_FADE),
        marks: ramp(CARD_MARKS_AFTER, CARD_MARKS_FADE),
    }
}

/// How a finished cut's picture is drawn `since` it was done: as
/// [`finishing`] has it, or under Reduce Motion (`reduced`) simply as it
/// stands at the end, at once, with no frames to wait for (Qt's `still`).
pub fn finish_drawn(since: Duration, reduced: bool) -> Finish {
    finishing(if reduced { CARD_DONE } else { since })
}

/// The picture's box on its card: as wide as it may be at its shape, an
/// upright one held to the tallest.
pub fn card_picture_size(aspect: f32) -> (f32, f32) {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let mut width = CARD_PICTURE_WIDEST;
    let mut height = width / aspect;
    if height > CARD_PICTURE_TALLEST {
        height = CARD_PICTURE_TALLEST;
        width = height * aspect;
    }
    (width, height)
}

/// The point `t` (0..1) of the way round a rounded rectangle, clockwise
/// from the middle of its top edge.
pub fn rim_point(
    bounds: gpui::Bounds<gpui::Pixels>,
    radius: f32,
    t: f32,
) -> gpui::Point<gpui::Pixels> {
    let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let (straight_w, straight_h) = (w - 2.0 * r, h - 2.0 * r);
    let arc = std::f32::consts::FRAC_PI_2 * r;
    let total = 2.0 * (straight_w + straight_h) + 4.0 * arc;
    if total <= 0.0 {
        return bounds.origin;
    }
    let mut left = t.rem_euclid(1.0) * total;
    // The legs, clockwise from the top's middle: each a line or a quarter arc.
    let legs: [(f32, u8); 9] = [
        (straight_w / 2.0, 0),
        (arc, 1),
        (straight_h, 2),
        (arc, 3),
        (straight_w, 4),
        (arc, 5),
        (straight_h, 6),
        (arc, 7),
        (straight_w / 2.0, 8),
    ];
    for (length, leg) in legs {
        if left > length && leg != 8 {
            left -= length;
            continue;
        }
        let along = if length > 0.0 { left / length } else { 0.0 };
        let corner = |cx: f32, cy: f32, from: f32| {
            let angle = from + along * std::f32::consts::FRAC_PI_2;
            (cx + r * angle.cos(), cy + r * angle.sin())
        };
        let (px_, py_) = match leg {
            0 => (x + w / 2.0 + left, y),
            1 => corner(x + w - r, y + r, -std::f32::consts::FRAC_PI_2),
            2 => (x + w, y + r + left),
            3 => corner(x + w - r, y + h - r, 0.0),
            4 => (x + w - r - left, y + h),
            5 => corner(x + r, y + h - r, std::f32::consts::FRAC_PI_2),
            6 => (x, y + h - r - left),
            7 => corner(x + r, y + r, std::f32::consts::PI),
            _ => (x + r + left, y),
        };
        return point(px(px_), px(py_));
    }
    bounds.origin
}

/// The light running round a cut's picture while Kerf makes it: a head of
/// white going into the accent's blue, `tail` of the way round behind it,
/// at `turn` (0..1). Under Reduce Motion, a still ring.
fn paint_rim(
    bounds: gpui::Bounds<gpui::Pixels>,
    radius: f32,
    turn: Option<f32>,
    window: &mut Window,
) {
    const RUNS: usize = 18;
    const TAIL: f32 = scene::CARD_TAIL;
    const POINTS: usize = 6;
    let blue = gpui::Rgba {
        r: 143.0 / 255.0,
        g: 176.0 / 255.0,
        b: 1.0,
        a: 1.0,
    };
    let Some(turn) = turn else {
        let mut ring = gpui::PathBuilder::stroke(px(2.0));
        let points: Vec<_> = (0..=96)
            .map(|step| rim_point(bounds, radius, step as f32 / 96.0))
            .collect();
        ring.add_polygon(&points, true);
        if let Ok(path) = ring.build() {
            window.paint_path(path, theme::with_alpha(blue, 0.6));
        }
        return;
    };
    for run in 0..RUNS {
        let fade = 1.0 - run as f32 / RUNS as f32;
        let mix = (run as f32 / 3.0).min(1.0);
        let shade = gpui::Rgba {
            r: 1.0 + (blue.r - 1.0) * mix,
            g: 1.0 + (blue.g - 1.0) * mix,
            b: 1.0,
            a: 0.95 * fade * fade,
        };
        let head = turn - run as f32 * TAIL / RUNS as f32;
        let points: Vec<_> = (0..=POINTS)
            .map(|step| {
                let at = head - step as f32 * TAIL / RUNS as f32 / POINTS as f32;
                rim_point(bounds, radius, at)
            })
            .collect();
        let mut line = gpui::PathBuilder::stroke(px(2.0));
        line.add_polygon(&points, false);
        if let Ok(path) = line.build() {
            window.paint_path(path, shade);
        }
    }
}

/// A ring round a cut's picture lit once as it is done, going out.
fn paint_ring(bounds: gpui::Bounds<gpui::Pixels>, radius: f32, light: f32, window: &mut Window) {
    if light <= 0.0 {
        return;
    }
    let points: Vec<_> = (0..=96)
        .map(|step| rim_point(bounds, radius, step as f32 / 96.0))
        .collect();
    let mut ring = gpui::PathBuilder::stroke(px(2.0));
    ring.add_polygon(&points, true);
    if let Ok(path) = ring.build() {
        window.paint_path(path, theme::with_alpha(gpui::white().into(), 0.94 * light));
    }
}

/// How a cut's picture is drawn now.
#[derive(Clone, Debug, Default)]
struct PictureState {
    /// Kerf is making it.
    working: bool,
    /// It finished this long ago (Done, then its marks).
    finished: Option<Duration>,
    /// Its picture is on the stage, from here.
    on_stage: bool,
    /// The pointer's place across it.
    scrub: Option<f32>,
    /// The poster it showed before the newest clip, and how far the
    /// newest has come over it (0..1).
    before: Option<(Arc<RenderImage>, f32)>,
}

impl ChatView {
    /// A first cut's picture (`cut` is its timeline), for its card: the
    /// poster of the clip it shows, how long it is and a play mark; while
    /// Kerf makes it, light round it and "Cutting"; just done, "Done".
    /// Moving over it runs through its clips; pressing it brings the
    /// picture in.
    fn cut_picture(
        &self,
        id: &str,
        cut: &Timeline,
        state: PictureState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = self.session.read(cx);
        let artwork = &session.artwork;
        let (frames, duration) = cut_frames(cut);
        let posters: Vec<Option<Arc<RenderImage>>> = frames
            .iter()
            .map(|(_, source)| artwork.poster(source))
            .collect();
        let shown = shown_frame(
            &frames
                .iter()
                .zip(&posters)
                .map(|((at, _), poster)| (*at, poster.is_some()))
                .collect::<Vec<_>>(),
            duration,
            state.scrub,
            state.working,
        )
        .and_then(|index| posters.get(index).cloned().flatten());
        let (width, height) = card_picture_size(cut.aspect());
        let reduced = motion::reduced();
        let finish = state.finished.map(|since| finish_drawn(since, reduced));
        let work = if state.working {
            1.0
        } else {
            finish.map_or(0.0, |finish| finish.work)
        };
        let moving = !reduced
            && (state.working
                || finish.is_some_and(|finish| finish.chip > 0.0 || finish.marks < 1.0));
        if moving || state.before.is_some() {
            window.request_animation_frame();
        }
        let chip = if state.working {
            1.0
        } else {
            finish.map_or(0.0, |finish| finish.chip)
        };
        let marks = if state.working {
            0.0
        } else {
            finish.map_or(1.0, |finish| finish.marks)
        };
        let radius = theme::radius::CARD;
        let mut sized = div().w(px(width)).max_w_full();
        // Its height follows its width at the cut's shape, narrowed or not.
        sized.style().aspect_ratio = Some(width / height.max(1.0));
        let mut picture = sized
            .id(ElementId::Name(format!("cut-picture-{id}").into()))
            .relative()
            .rounded(px(radius))
            .overflow_hidden()
            .bg(theme::overlay_screen());
        // The picture: the poster it shows (the one before fading under the
        // newest while a clip turns), else a wash in the room's greys and the
        // accent, never black.
        match (&shown, &state.before) {
            (Some(poster), Some((before, t))) => {
                picture = picture.child(poster_layer(before.clone(), radius)).child(
                    div()
                        .absolute()
                        .inset_0()
                        .opacity(*t)
                        .child(poster_layer(poster.clone(), radius)),
                );
            }
            (Some(poster), None) => picture = picture.child(poster_layer(poster.clone(), radius)),
            (None, _) => {
                picture = picture.child(div().absolute().inset_0().bg(gpui::rgb(0x141720)).child(
                    div().absolute().inset_0().bg(linear_gradient(
                        135.0,
                        linear_color_stop(gpui::rgba(0x8fb0ff46), 0.0),
                        linear_color_stop(gpui::rgba(0xb4aaff00), 1.0),
                    )),
                ))
            }
        }
        // The pointer's place, a line at its foot.
        if let Some(scrub) = state.scrub.filter(|_| !state.working && !state.on_stage) {
            picture = picture.child(
                div()
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .h(px(2.0))
                    .w(gpui::relative(scrub))
                    .bg(gpui::rgba(0xffffffe6)),
            );
        }
        // At work the picture sits a little back.
        if work > 0.0 {
            picture = picture.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(theme::with_alpha(gpui::black().into(), 0.22 * work)),
            );
        }
        if state.on_stage {
            // It is on the stage now: what is left here says so.
            picture = picture.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::rgba(0x121212c7))
                    .child(ui::label("On the stage", text::CONTROL, theme::ink2())),
            );
        } else if marks > 0.0 {
            // How long it is, at its foot, and that it plays.
            picture = picture
                .child(
                    div()
                        .absolute()
                        .right(px(6.0))
                        .bottom(px(6.0))
                        .h(px(18.0))
                        .px(px(5.0))
                        .flex()
                        .items_center()
                        .rounded(px(theme::radius::SMALL))
                        .bg(gpui::rgba(0x000000a8))
                        .opacity(marks)
                        .child(ui::label(
                            length_badge(duration),
                            text::MONO_SMALL,
                            gpui::white().into(),
                        )),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(marks)
                        .child(
                            div()
                                .size(px(CARD_PLAY_MARK))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(gpui::rgba(0x00000080))
                                .child(Icon::Play.view(16.0, gpui::white().into())),
                        ),
                );
        }
        // What Kerf is doing, at its top left: hopping dots and Cutting, then
        // a tick and Done.
        if chip > 0.0 && !state.on_stage {
            let since = self.started_at.elapsed();
            let dots = (0..3).map(|dot| {
                let t = if reduced {
                    0.0
                } else {
                    ((since.as_secs_f32() / scene::CHIP_HOP.as_secs_f32())
                        - dot as f32 * scene::CHIP_HOP_STAGGER
                        + 1.0)
                        .rem_euclid(1.0)
                };
                let hop = if t < 0.6 {
                    (t / 0.6 * std::f32::consts::PI).sin()
                } else {
                    0.0
                };
                div()
                    .size(px(4.0))
                    .mt(px(-scene::CHIP_HOP_RISE * hop))
                    .rounded_full()
                    .bg(theme::accent_ink())
                    .opacity(0.5 + 0.5 * hop)
            });
            picture = picture.child(
                div()
                    .absolute()
                    .left(px(8.0))
                    .top(px(8.0))
                    .h(px(22.0))
                    .pl(px(8.0))
                    .pr(px(9.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded_full()
                    .bg(gpui::rgba(0x0a0c129e))
                    .opacity(chip)
                    .child(if state.working {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .children(dots)
                            .into_any_element()
                    } else {
                        Icon::Check
                            .view(12.0, theme::accent_ink())
                            .into_any_element()
                    })
                    .child(ui::label(
                        if state.working { "Cutting" } else { "Done" },
                        text::META,
                        gpui::white().into(),
                    )),
            );
        }
        // Where it was laid out, for the pointer's place over it.
        let pictures = self.pictures.clone();
        let key = id.to_owned();
        picture = picture.child(
            gpui::canvas(
                move |bounds, _, _| {
                    pictures.borrow_mut().insert(key.clone(), bounds);
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        );
        let scrub_id = id.to_owned();
        let leave_id = id.to_owned();
        let open_id = id.to_owned();
        let interactive = !state.working;
        let tipped = interactive && !state.on_stage;
        let tip_id = ElementId::Name(format!("cut-picture-tip-{id}").into());
        let picture = picture.when(tipped, |picture| {
            picture
                .cursor_pointer()
                .on_mouse_move(
                    cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                        let Some(bounds) = this.pictures.borrow().get(&scrub_id).copied() else {
                            return;
                        };
                        let width = f32::from(bounds.size.width);
                        let across = if width > 0.0 && bounds.contains(&event.position) {
                            Some(
                                ((f32::from(event.position.x - bounds.origin.x)) / width)
                                    .clamp(0.0, 0.999),
                            )
                        } else {
                            None
                        };
                        let now = across.map(|across| (scrub_id.clone(), across));
                        if this.scrub != now {
                            this.scrub = now;
                            cx.notify();
                        }
                    }),
                )
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if !*hovered && this.scrub.as_ref().is_some_and(|(id, _)| *id == leave_id) {
                        this.scrub = None;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_cut(open_id.clone(), window, cx)
                }))
        });
        // Round it, the light: running while Kerf works, lit once and going
        // out as it is done; under it, a line of light runs while it works.
        let ring = finish.map_or(0.0, |finish| finish.ring);
        let working = state.working;
        let started = self.started_at;
        let rim = gpui::canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let inner = gpui::Bounds {
                    origin: point(
                        bounds.origin.x + px(CARD_PICTURE_RING / 2.0),
                        bounds.origin.y + px(CARD_PICTURE_RING / 2.0),
                    ),
                    size: gpui::size(
                        bounds.size.width - px(CARD_PICTURE_RING),
                        bounds.size.height - px(CARD_PICTURE_RING + 7.0),
                    ),
                };
                let radius = theme::radius::CARD + CARD_PICTURE_RING / 2.0;
                if working {
                    let turn = (!reduced).then(|| motion::phase(started, CARD_TURN));
                    paint_rim(inner, radius, turn, window);
                } else {
                    paint_ring(inner, radius, ring, window);
                }
            },
        )
        .absolute()
        .inset_0();
        let line = (work > 0.0 && !reduced).then(|| {
            let at = motion::phase(self.started_at, scene::WORK_SHIMMER);
            div()
                .relative()
                .h(px(2.0))
                .mt(px(3.0))
                .mx(px(CARD_PICTURE_RING))
                .overflow_hidden()
                .opacity(work)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(gpui::relative(at * 1.4 - 0.4))
                        .w(gpui::relative(0.4))
                        .bg(linear_gradient(
                            90.0,
                            linear_color_stop(gpui::rgba(0x8fb0ff00), 0.0),
                            linear_color_stop(gpui::rgba(0x8fb0ffcc), 1.0),
                        )),
                )
        });
        div()
            .relative()
            .flex()
            .flex_col()
            .p(px(CARD_PICTURE_RING))
            .pb(px(CARD_PICTURE_RING + 4.0))
            .child(if tipped {
                ui::tipped(tip_id, ui::Tip::new("Open on the stage"), picture).into_any_element()
            } else {
                picture.into_any_element()
            })
            .children(line)
            .child(rim)
            .into_any_element()
    }

    /// The picture of cut `id` comes in from its card: the stage shows it
    /// (rendered when it is not yet), growing out of the card's picture
    /// (the window's [`ChatEvent::GrowPicture`]), and the card says it is
    /// there.
    fn open_cut(&mut self, id: String, _window: &mut Window, cx: &mut Context<Self>) {
        self.stage_proposal = Some(id.clone());
        self.scrub = None;
        self.session.update(cx, |session, cx| {
            let showing = session.state.preview.proposal.as_deref() == Some(id.as_str())
                && previewing(&session.state.preview.status);
            if !showing {
                session.preview(Some(&id));
            }
            cx.notify();
        });
        cx.emit(ChatEvent::GrowPicture { proposal: id });
        cx.notify();
    }

    /// The cut whose picture went to the stage from its card, while it is there.
    pub fn stage_proposal(&self) -> Option<&str> {
        self.stage_proposal.as_deref()
    }

    /// Where the visible part of the conversation was last laid out.
    fn in_sight(&self, bounds: gpui::Bounds<gpui::Pixels>) -> bool {
        let view = self.scroll.bounds();
        view.size.width > px(0.0) && in_sight(bounds, view)
    }

    /// Where cut `proposal`'s card shows its picture in the window, as last
    /// laid out, while it is in sight: what the stage grows out of, and
    /// what it goes back into.
    pub fn cut_picture_bounds(&self, proposal: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        let pictures = self.pictures.borrow();
        let bounds = pictures.get(proposal).copied().or_else(|| {
            pictures
                .iter()
                .find(|(id, _)| self.book.same(id, proposal))
                .map(|(_, bounds)| *bounds)
        })?;
        self.in_sight(bounds).then_some(bounds)
    }

    /// The poster cut `proposal`'s card shows now (the clip under the
    /// pointer, else its first), to stand in as its picture grows.
    pub fn cut_poster(&self, proposal: &str, cx: &App) -> Option<Arc<RenderImage>> {
        let session = self.session.read(cx);
        let (shown, cut) = session.first_cut()?;
        if !self.book.same(&shown.proposal_id, proposal) {
            return None;
        }
        let (frames, duration) = cut_frames(cut);
        let posters: Vec<Option<Arc<RenderImage>>> = frames
            .iter()
            .map(|(_, source)| session.artwork.poster(source))
            .collect();
        let scrub = self
            .scrub
            .as_ref()
            .filter(|(over, _)| self.book.same(over, proposal))
            .map(|(_, across)| *across);
        let shown = shown_frame(
            &frames
                .iter()
                .zip(&posters)
                .map(|((at, _), poster)| (*at, poster.is_some()))
                .collect::<Vec<_>>(),
            duration,
            scrub,
            false,
        )?;
        posters.get(shown).cloned().flatten()
    }

    /// Where media `source`'s footage tile shows in the window, as last
    /// laid out, while it is in sight and has not gone on.
    pub fn footage_tile_bounds(&self, source: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        if self.taken.contains(source) {
            return None;
        }
        let bounds = self.tiles.borrow().get(source).copied()?;
        self.in_sight(bounds).then_some(bounds)
    }

    /// Footage `source` has gone on from its tile (to become a clip): its
    /// place stays, empty, until the sheet goes.
    pub fn take_footage(&mut self, source: &str, cx: &mut Context<Self>) {
        if self.taken.insert(source.to_owned()) {
            cx.notify();
        }
    }

    /// Where what comes from the conversation with no tile of its own
    /// leaves from: a poster's room over the composer's middle, as Qt's.
    pub fn over_composer(&self) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.composer_at.get().map(over_composer)
    }

    /// The top of the composer with what stands over it (Kerf's next steps,
    /// Run QC, what waits to be asked), as last laid out: the window's
    /// notices sit above it rather than over them.
    pub fn composer_top(&self) -> Option<f32> {
        self.composer_at
            .get()
            .map(|bounds| f32::from(bounds.origin.y))
    }
}

/// Whether `bounds` lies whole inside the conversation's visible part
/// `view` (a point either way spared): only then does a picture go into it
/// or come out of it, as Qt's.
pub fn in_sight(bounds: gpui::Bounds<gpui::Pixels>, view: gpui::Bounds<gpui::Pixels>) -> bool {
    let spare = px(1.0);
    bounds.left() >= view.left() - spare
        && bounds.right() <= view.right() + spare
        && bounds.top() >= view.top() - spare
        && bounds.bottom() <= view.bottom() + spare
}

/// Where what comes from the conversation with no tile of its own leaves
/// from, over a composer laid out at `composer`: a poster's room,
/// `OVER_COMPOSER` big, centred over it and `OVER_COMPOSER_RISE` above its
/// top, as Qt's flightStart.
pub fn over_composer(composer: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    let (width, height) = OVER_COMPOSER;
    gpui::Bounds::new(
        point(
            composer.center().x - px(width / 2.0),
            composer.top() - px(OVER_COMPOSER_RISE),
        ),
        gpui::size(px(width), px(height)),
    )
}

/// The room over the composer a flight leaves from, and how far above the
/// composer's top it starts.
pub use crate::theme::layout::{OVER_COMPOSER, OVER_COMPOSER_RISE};

/// A poster filling the picture's box, cut to its shape.
fn poster_layer(poster: Arc<RenderImage>, radius: f32) -> impl IntoElement {
    gpui::img(poster)
        .absolute()
        .inset_0()
        .size_full()
        .object_fit(gpui::ObjectFit::Cover)
        .rounded(px(radius))
}

impl Focusable for ChatView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_keys(cx);
        self.follow_turn(cx);
        // What is laid out this frame says again where it is.
        self.selections.borrow_mut().begin_frame();
        self.pictures.borrow_mut().clear();
        self.tiles.borrow_mut().clear();
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
        // The turn under way shows in its own conversation only (a request
        // sent with none open goes on in the new one it starts).
        let showing_this = session.state.work.conversation == session.conversation;
        // A conversation chosen whose turns have not come yet: its title is
        // in the toolbar, and nothing here says it is empty.
        let loading = session.state.chosen.is_some() && session.state.open_conversation.is_none();
        let building = self.building.clone();
        let pins = pin_order(
            &proposals,
            &self.book,
            session.timeline(),
            session.state.snapshot.proposal_timeline.as_ref(),
            session.state.position,
        );
        let pinned = pins.first().cloned();
        // An open proposal no turn shows yet shows at the end, the pinned first.
        let holders = card_turns(&turns, &proposals, &self.book);
        let loose: Vec<Proposal> = pins
            .iter()
            .filter(|id| !holders.contains_key(*id))
            // The cut Kerf is making is its card at work, in the live answer.
            .filter(|id| {
                building
                    .as_deref()
                    .is_none_or(|building| !self.book.same(building, id))
            })
            .filter_map(|id| {
                proposals
                    .iter()
                    .find(|proposal| &proposal.proposal_id == id)
            })
            .cloned()
            .collect();
        // A turn under way elsewhere leaves a new conversation as it is.
        let live_here = showing_this && (busy || request.is_some());
        let nothing = !loading && turns.is_empty() && !live_here && loose.is_empty();

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
            list = list.child(self.turn(
                index,
                &turns,
                &proposals,
                &holders,
                pinned.as_deref(),
                window,
                cx,
            ));
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
            if !busy {
                list = list.child(ui::label(strays_note(&loose), text::BODY, theme::ink3()));
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
        let composer = self.composer(window, cx);
        let ask = self.ask.clone();
        // While what a proposal changed plays, the conversation falls back.
        let watching = self.session.read(cx).reel().is_some();
        let behind = if watching { CHAT_BEHIND } else { 1.0 };
        if (self.behind.target() - behind).abs() > f32::EPSILON {
            self.behind.go(behind, scene::SETTLE);
        }
        let opacity = self.behind.sample(window);

        div()
            .size_full()
            .flex()
            .flex_col()
            .opacity(opacity)
            .children(ask)
            .child(
                div()
                    .id("conversation")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div().w_full().flex().justify_center().px(px(24.0)).child(
                            // Its words wrap to the column: what they would be
                            // on one line never widens it.
                            div()
                                .w_full()
                                .min_w_0()
                                .max_w(px(layout::CONVERSATION_COLUMN))
                                .child(list),
                        ),
                    ),
            )
            // Its fades: where an answer is cut at the top, and at the foot
            // while there is more below.
            .child(ui::scroll_fade(&self.scroll, theme::surround()))
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
                            .relative()
                            .w_full()
                            .min_w_0()
                            .max_w(px(layout::CONVERSATION_COLUMN))
                            .child(composer)
                            // Where it is, for what comes from over it.
                            .child({
                                let at = self.composer_at.clone();
                                gpui::canvas(
                                    move |bounds, _, _| at.set(Some(bounds)),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0()
                            }),
                    ),
            )
            .children(self.words_menu.as_ref().map(|(menu, at)| {
                deferred(
                    anchored()
                        .position(*at)
                        .snap_to_window_with_margin(px(theme::space::S8))
                        .child(menu.clone()),
                )
                .with_priority(2)
            }))
    }
}
