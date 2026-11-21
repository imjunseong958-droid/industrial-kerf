//! The editor's state, as the bridge's events leave it. Pure data: no
//! window, no GPUI, so every event can be tested on its own. The window
//! draws from it; nothing here keeps a second editable copy of the
//! project, which only kerfd owns.

use crate::model::{Conversation, ConversationSummary, RenderJob, Snapshot};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// The connection to kerfd.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Connection {
    #[default]
    Connecting,
    Connected,
    Failed(String),
}

/// A row of the work log: one tool call.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub id: String,
    pub text: String,
    pub state: StepState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Running,
    Done,
    Failed,
}

/// What Kerf is doing in the turn under way.
#[derive(Clone, Debug, Default)]
pub struct Work {
    /// When the turn began; None between turns.
    pub started: Option<Instant>,
    /// What it says it is doing ("Planning next step…").
    pub status: String,
    /// Its answer so far, as it streams.
    pub answer: String,
    pub steps: Vec<Step>,
    /// running, awaiting_review, aborted, completed, failed.
    pub state: String,
    pub message: Option<String>,
    /// The conversation the turn goes on in.
    pub conversation: Option<String>,
    /// The request, as it was sent, until the conversation keeps it.
    pub request: Option<String>,
}

impl Work {
    pub fn busy(&self) -> bool {
        self.started.is_some()
    }
}

/// A making under way: a media being made from its steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Making {
    pub take: u32,
    pub line: String,
    pub since: Instant,
}

/// What a making's words say while it runs, as Qt's status bar said them:
/// "Making take 2 of m4 from its steps…" before any step has spoken, then
/// "Making m4: <what the step last said>". `name` is the media's name.
pub fn making_status(name: &str, making: &Making) -> String {
    if making.line.trim().is_empty() {
        format!("Making take {} of {name} from its steps…", making.take)
    } else {
        format!("Making {name}: {}", making.line.trim())
    }
}

/// What a media's file is short of, from `media_status`: its badge on a
/// tile, and what its tooltip says to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    /// The file is missing.
    Offline,
    /// A Git LFS file not downloaded yet.
    NotDownloaded,
    /// Outside the project folder, or ignored by Git there.
    External,
}

impl Badge {
    pub fn words(self) -> &'static str {
        match self {
            Badge::Offline => "Offline",
            Badge::NotDownloaded => "Not downloaded",
            Badge::External => "External",
        }
    }

    /// What it means, and what to do about it.
    pub fn tip(self) -> &'static str {
        match self {
            Badge::Offline => {
                "Offline: the file is missing. Replace Media… points this slot at another file."
            }
            Badge::NotDownloaded => "Not downloaded: a Git LFS file that a sync downloads.",
            Badge::External => {
                "Outside the project folder, or ignored by Git there, so collaborators do not have it. Consolidate Media copies it into media/."
            }
        }
    }
}

/// The last `media_status`: media whose file is missing, Git LFS files not
/// downloaded, and media outside the project folder, by id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaStatus {
    pub missing: BTreeSet<String>,
    pub pointers: BTreeSet<String>,
    pub external: BTreeSet<String>,
}

impl MediaStatus {
    pub fn from_value(event: &Value) -> Self {
        let ids = |key: &str| {
            event[key]
                .as_array()
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        Self {
            missing: ids("missing"),
            pointers: ids("pointers"),
            external: ids("external"),
        }
    }

    /// Media `id`'s badge: a missing file outranks the rest, as it cannot
    /// be downloaded or consolidated.
    pub fn badge(&self, id: &str) -> Option<Badge> {
        if self.missing.contains(id) {
            Some(Badge::Offline)
        } else if self.pointers.contains(id) {
            Some(Badge::NotDownloaded)
        } else if self.external.contains(id) {
            Some(Badge::External)
        } else {
            None
        }
    }
}

/// What a media is doing, for its tile and its clips.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaActivity {
    Ready,
    /// Being made: its take and when it began.
    Making {
        take: u32,
        since: Instant,
    },
    /// Waiting its turn on the server ("asked…", "waiting, 2 ahead"): no
    /// stripes and no clock, a dashed amber outline.
    Queued {
        ahead: Option<u32>,
    },
    /// A shot its recipe or steps have not made yet: a dashed hole.
    NotMade,
    /// Its file is gone.
    Offline,
}

impl MediaActivity {
    /// With no file to show: a hole in its lane, a dashed tile.
    pub fn absent(&self) -> bool {
        matches!(self, MediaActivity::NotMade | MediaActivity::Offline)
    }
}

/// How full Kerf's context is, as a turn last said it (`agent_context`):
/// of which conversation (None while the turn's conversation has no id
/// yet), how much is taken of how much, and whether that is counted in the
/// model's tokens or in Kerf's own measure.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextUse {
    pub conversation: Option<String>,
    pub used: f64,
    pub limit: f64,
    pub tokens: bool,
}

/// How full the context is, as its ring is coloured: ink2, the warning's
/// amber from three quarters, red from nine tenths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextLevel {
    Room,
    Filling,
    Full,
}

impl ContextUse {
    /// How much is taken, 0..1.
    pub fn fraction(&self) -> f64 {
        if self.limit > 0.0 && self.used.is_finite() {
            (self.used / self.limit).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The share taken, in whole percent.
    pub fn percent(&self) -> u32 {
        (self.fraction() * 100.0).round() as u32
    }

    pub fn level(&self) -> ContextLevel {
        let full = self.fraction();
        if full >= 0.9 {
            ContextLevel::Full
        } else if full >= 0.75 {
            ContextLevel::Filling
        } else {
            ContextLevel::Room
        }
    }

    /// What it is of: "68k of 200k tokens", or, in Kerf's own measure,
    /// what Kerf does as it fills.
    pub fn note(&self) -> String {
        if self.tokens {
            format!(
                "{} of {} tokens",
                short_count(self.used),
                short_count(self.limit)
            )
        } else {
            "Kerf puts older steps away as it fills".to_owned()
        }
    }

    /// "Context 34% full · 68k of 200k tokens": its tip.
    pub fn describe(&self) -> String {
        format!("Context {}% full · {}", self.percent(), self.note())
    }
}

/// A count as a reader takes it in: "950", "68k", "1.2M", "12M".
pub fn short_count(count: f64) -> String {
    let count = count.max(0.0).round();
    if count >= 1_000_000.0 {
        let millions = count / 1_000_000.0;
        if count >= 10_000_000.0 {
            format!("{:.0}M", millions)
        } else {
            format!("{:.1}M", millions)
        }
    } else if count >= 1000.0 {
        format!("{:.0}k", (count / 1000.0).round())
    } else {
        format!("{count:.0}")
    }
}

/// A note the window shows for a while: an error, or what finished.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    /// Its place among every notice there has been, to tell it from the rest.
    pub id: u64,
    pub text: String,
    pub error: bool,
    pub at: Instant,
    /// How long it stays, when it keeps a time of its own (a sync's
    /// summary, its pull request's line and a cast saved, Qt's 8 s:
    /// `theme::motion::NOTICE_LONG_HOLD`); None stays the window's
    /// (`NOTICE_HOLD`, an error `NOTICE_ERROR_HOLD`).
    pub hold: Option<Duration>,
}

impl Notice {
    /// How long it stays: its own hold, else `hold`, or `error_hold` for
    /// an error.
    pub fn held_for(&self, hold: Duration, error_hold: Duration) -> Duration {
        self.hold
            .unwrap_or(if self.error { error_hold } else { hold })
    }
}

/// A rendered preview's state (`preview` events).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub status: String,
    pub progress: Option<f64>,
    pub message: Option<String>,
    pub proposal: Option<String>,
}

/// What a preview needing a render says when the bridge gives no reason
/// (Qt's words).
pub const PREVIEW_REQUIRED: &str = "Rendered preview required";

/// How long "Preview ready" and "Live preview: …" stay, as Qt's status bar
/// held them (4 s).
pub use crate::theme::motion::NOTICE_SHORT_HOLD as PREVIEW_NOTICE_HOLD;

impl Preview {
    pub fn rendering(&self) -> bool {
        self.status == "running"
    }

    /// Why the picture needs a rendered preview, while it does (`required`,
    /// or `unavailable`): the bridge's reason (what the live picture cannot
    /// show yet, or why the last render failed), else "Rendered preview
    /// required". What the stage's Render preview button says over it.
    pub fn reason(&self) -> Option<String> {
        matches!(self.status.as_str(), "required" | "unavailable").then(|| {
            self.message
                .as_deref()
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .unwrap_or(PREVIEW_REQUIRED)
                .to_owned()
        })
    }
}

/// What a `preview` event says, as Qt's status bar said it, and for how
/// long (None: the window's `NOTICE_HOLD`, Qt's 6 s): why a rendered
/// preview is needed ([`Preview::reason`]), "Preview ready", or "Live
/// preview: <what it cannot match yet>" ([`PREVIEW_NOTICE_HOLD`]). A render
/// running, one stopped or left, and one that failed (an error notice of
/// its own, which the reason the bridge gives next takes the place of)
/// say nothing here.
pub fn preview_notice(event: &Value) -> Option<(String, Option<Duration>)> {
    let message = event["message"]
        .as_str()
        .map(str::trim)
        .filter(|message| !message.is_empty());
    match event["status"].as_str()? {
        "required" | "unavailable" => Some((message.unwrap_or(PREVIEW_REQUIRED).to_owned(), None)),
        "ready" => Some(("Preview ready".to_owned(), Some(PREVIEW_NOTICE_HOLD))),
        "live" => Some((
            match message {
                Some(note) => format!("Live preview: {note}"),
                None => "Live preview".to_owned(),
            },
            Some(PREVIEW_NOTICE_HOLD),
        )),
        _ => None,
    }
}

/// A next step Kerf offers (`agent_next`): the button's few words, and the
/// request pressing it puts in the composer, in the person's words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NextStep {
    pub label: String,
    pub request: String,
}

impl NextStep {
    /// A step whose button says `label` and that asks `request`.
    pub fn new(label: impl Into<String>, request: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            request: request.into(),
        }
    }

    /// One of `agent_next`'s steps: `{label, request}` as the bridge sends
    /// them, or plain words, both the label and the request (as Qt read
    /// either). A step with no label is none; one with no request asks its
    /// label.
    pub fn read(value: &Value) -> Option<Self> {
        let (label, request) = match value {
            Value::String(words) => (words.as_str(), words.as_str()),
            Value::Object(step) => (
                step.get("label")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                step.get("request")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            _ => return None,
        };
        let label = label.trim();
        if label.is_empty() {
            return None;
        }
        let request = Some(request.trim()).filter(|request| !request.is_empty());
        Some(Self::new(label, request.unwrap_or(label)))
    }
}

impl From<&str> for NextStep {
    /// Words that are both the step's label and its request.
    fn from(words: &str) -> Self {
        Self::new(words, words)
    }
}

/// The steps an `agent_next` event offers, in its order.
pub fn next_steps(event: &Value) -> Vec<NextStep> {
    event["steps"]
        .as_array()
        .map(|steps| steps.iter().filter_map(NextStep::read).collect())
        .unwrap_or_default()
}

/// What a change of state means for the window, so views can follow it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Snapshot,
    Connection,
    Conversations,
    Conversation(String),
    Work,
    TurnEnded,
    Clock,
    Playback,
    Preview,
    Renders,
    Notice,
    Making,
    ViewerReset,
    Other,
}

#[derive(Default)]
pub struct EditorState {
    pub connection: Connection,
    pub snapshot: Snapshot,
    /// Whether any snapshot has come yet.
    pub has_snapshot: bool,
    pub conversations: Vec<ConversationSummary>,
    /// The conversation the person chose; None is a new one, until the
    /// request sent in it starts one.
    pub chosen: Option<String>,
    /// The chosen conversation, as kept beside the project.
    pub open_conversation: Option<Conversation>,
    pub work: Work,
    /// Next steps Kerf offers as buttons (the composer's, and an empty
    /// conversation's Try asking), until the next request goes.
    pub next_steps: Vec<NextStep>,
    pub position: f64,
    pub duration: f64,
    pub dropped: u64,
    pub playing: bool,
    pub preview: Preview,
    pub renders: Vec<RenderJob>,
    pub makings: BTreeMap<String, Making>,
    pub notices: Vec<Notice>,
    /// Clips and times Kerf is looking at, for its cursor.
    pub agent_focus: Vec<f64>,
    pub agent_clips: Vec<String>,
    /// How full Kerf's context is, as the last turn said; shown only in
    /// the conversation it is of ([`EditorState::context_shown`]).
    pub context: Option<ContextUse>,
    /// The caption under the playhead.
    pub caption: String,
    /// Peak level, left and right, dBFS.
    pub meter: Option<[f64; 2]>,
    /// Conversations that finished while you looked elsewhere.
    pub settled: Vec<String>,
    /// The project's Git standing, as the last `collab` event said it.
    pub collab: Option<Value>,
    /// The name Save Current as Preset sent, until the bridge answers: a
    /// refusal for a preset of that name already there is then asked
    /// about ([`crate::presets::SavePreset`]), not said.
    pub saving_preset: Option<String>,
    /// The preset that refusal named (`presets/Intro.kerfpreset`).
    pub preset_exists: Option<String>,
    /// The preset file last read (`preset_inspect`): its path, and what it
    /// holds.
    pub inspected_preset: Option<(String, Value)>,
    /// The last `media_status`.
    pub media_status: MediaStatus,
    /// What `media_info` said of each media (`pixFmt`, `codec`…), merged
    /// as it comes, and the media it has been asked about since connecting.
    pub media_info: BTreeMap<String, Value>,
    pub info_asked: BTreeSet<String>,
    /// Each media's `media_index` (its shots, silences and loudest
    /// moments), what refused one, and those asked for and not come yet.
    /// One goes when its media's file changes.
    pub media_indexes: BTreeMap<String, Value>,
    pub media_index_errors: BTreeMap<String, String>,
    pub index_asked: BTreeSet<String>,
    /// The shots the last `cast` event said want making as the cast
    /// stands, each with why; one made since is taken off.
    pub wants_making: BTreeMap<String, String>,
    /// The last `cast` event: the project's cast and styles.
    pub cast: Option<Value>,
    /// Whether the picture waits on a rendered preview: from the moment
    /// one is needed (`required`, `unavailable`) or runs (`running`) until
    /// it plays again (`ready`, `live`), as Qt's `m_previewRequired` had
    /// it; a render that ends unshown (`stopped`, `exited`, `failed`)
    /// leaves the picture as it stood before ([`EditorState::picture_status`]).
    /// Play is unavailable while it does; a pause never is.
    pub preview_required: bool,
    /// What the picture kerfd plays last said of itself: `required`,
    /// `unavailable`, `ready` or `live` (empty before it has). A render
    /// stopped, left or failed before it showed goes back to it.
    pub picture_status: String,
    /// How many notices there have been.
    noticed: u64,
    /// The notice each kind of line last left ([`StatusLine`]): the next of
    /// its kind takes its place, as a line of Qt's status bar took the
    /// last one's.
    said_in_place: BTreeMap<StatusLine, u64>,
    /// Activity lines that only the state knows an event leaves (a preset
    /// written), until the session takes them ([`EditorState::take_logged`]).
    logged: Vec<(&'static str, String)>,
}

/// A kind of notice that, as Qt said it on its one status line, takes the
/// place of the last of its kind: what the preview says, and the mode the
/// composer was set to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusLine {
    Preview,
    Permissions,
}

/// Commands whose refusals the view that sent them shows in place: a
/// making's plan, a shot's graph, the Index page, the Cast dialog, a
/// media's description.
fn answered_in_place(command: &str) -> bool {
    matches!(
        command,
        "media_make" | "media_carry" | "takes_read" | "media_index" | "media_info" | "cast"
    ) || command.starts_with("recipe")
        || command.starts_with("shot_")
        || command.starts_with("workflow")
}

/// The first line of a request as a title says it: no list mark, no bold
/// or code marks.
pub fn title_line(body: &str) -> String {
    let line = body.lines().next().unwrap_or_default().trim_start();
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .unwrap_or(line);
    line.replace("**", "").replace('`', "").trim().to_owned()
}

/// Words on one line: runs of white space as one space, none at the ends.
pub fn simplified(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What saving the cast says: which shots want making for it now.
pub fn cast_saved(event: &Value) -> String {
    let wanting: Vec<&str> = event["toMake"]
        .as_array()
        .map(|shots| {
            shots
                .iter()
                .filter_map(|shot| shot["media"].as_str())
                .collect()
        })
        .unwrap_or_default();
    match wanting.len() {
        0 => "Saved the cast; no shot wants making for it".to_owned(),
        1 => format!("Saved the cast; {} wants making again", wanting[0]),
        _ => format!("Saved the cast; {} want making again", wanting.join(", ")),
    }
}

impl EditorState {
    /// Whether the project is up (its first snapshot has come) or kerfd
    /// could not be reached: what waits on the project opening, such as
    /// the warnings a new or cloned project leaves, can be said now. Not
    /// while it is only connecting, or connected with no snapshot yet.
    /// `app::say_when_up` waits for it.
    pub fn opened_or_failed(&self) -> bool {
        self.has_snapshot || matches!(self.connection, Connection::Failed(_))
    }

    /// Folds one bridge event into the state.
    pub fn apply(&mut self, event: &Value) -> Change {
        let kind = event["type"].as_str().unwrap_or_default();
        match kind {
            "state" => {
                let snapshot = Snapshot::from_value(&event["snapshot"]);
                self.forget_relinked(&snapshot);
                self.snapshot = snapshot;
                self.has_snapshot = true;
                if self.duration <= 0.0 {
                    self.duration = self.snapshot.duration;
                }
                Change::Snapshot
            }
            "connection" => {
                let status = event["status"].as_str().unwrap_or_default();
                let message = event["message"].as_str().unwrap_or_default().to_owned();
                self.connection = match status {
                    "connected" | "ready" => Connection::Connected,
                    "failed" | "error" | "disconnected" => {
                        Connection::Failed(if message.is_empty() {
                            "kerfd is not running".into()
                        } else {
                            message
                        })
                    }
                    _ => Connection::Connecting,
                };
                // A new connection knows nothing of what was asked before.
                self.info_asked.clear();
                self.index_asked.clear();
                // Not connected, nothing plays and the caption goes with the
                // picture (the session drops the frame).
                if self.connection != Connection::Connected {
                    self.playing = false;
                    self.caption.clear();
                }
                Change::Connection
            }
            "error" => {
                let command = event["command"].as_str().unwrap_or_default();
                // A making that failed is over: its stripes and its Stop go.
                if command == "media_make" {
                    if let Some(media) = event["media"].as_str() {
                        self.makings.remove(media);
                    }
                }
                // The Index page says why its media could not be indexed.
                if command == "media_index" {
                    let media = event["media"].as_str().unwrap_or_default().to_owned();
                    let message = event["message"]
                        .as_str()
                        .unwrap_or("This media could not be indexed")
                        .to_owned();
                    self.index_asked.remove(&media);
                    self.media_index_errors.insert(media, message);
                }
                // What a shot's graph or a making's plan asked for is answered
                // where it was asked, not again over the conversation.
                if answered_in_place(command) {
                    return Change::Making;
                }
                let message = event["message"].as_str().unwrap_or("Something went wrong");
                // Saving over a preset of that name is asked about, not refused.
                if self.saving_preset.is_some() {
                    if let Some(path) = crate::presets::existing_preset(message) {
                        self.preset_exists = Some(path.to_owned());
                        return Change::Other;
                    }
                }
                self.notify(message, true);
                Change::Notice
            }
            "conversations" => {
                self.conversations = event["items"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| serde_json::from_value(item.clone()).ok())
                            .collect()
                    })
                    .unwrap_or_default();
                // The chosen one deleted (here or in another window): a new one.
                if let Some(chosen) = self.chosen.clone() {
                    if !self.conversations.iter().any(|known| known.id == chosen) {
                        self.choose(None);
                    }
                }
                Change::Conversations
            }
            "conversation" => {
                let Some(conversation) = Conversation::from_value(event) else {
                    return Change::Other;
                };
                let id = conversation.id.clone();
                // A request sent with no conversation chosen starts one: that
                // one becomes the chosen, and its turn goes on in it.
                let started = self.chosen.is_none()
                    && self.work.conversation.is_none()
                    && (self.work.busy() || self.work.request.is_some());
                if started {
                    self.chosen = Some(id.clone());
                    self.work.conversation = Some(id.clone());
                    // A reading said before the conversation had its id is of it.
                    if let Some(context) =
                        self.context.as_mut().filter(|c| c.conversation.is_none())
                    {
                        context.conversation = Some(id.clone());
                    }
                }
                // Only the chosen conversation shows: another window's turns
                // in other conversations change the sidebar's list, not this.
                if self.chosen.as_deref() == Some(id.as_str()) {
                    // What it keeps of the request replaces the one shown while sending.
                    if conversation
                        .items
                        .iter()
                        .any(|turn| Some(&turn.text) == self.work.request.as_ref())
                    {
                        self.work.request = None;
                    }
                    self.open_conversation = Some(conversation);
                }
                Change::Conversation(id)
            }
            "agent_busy" => {
                let busy = event["busy"].as_bool().unwrap_or(false);
                if busy {
                    if !self.work.busy() {
                        self.work.started = Some(Instant::now());
                        self.work.answer.clear();
                        self.work.steps.clear();
                        self.work.status.clear();
                        self.work.state = "running".into();
                        self.work.message = None;
                        self.next_steps.clear();
                    }
                    Change::Work
                } else {
                    let was = self.work.busy();
                    self.end_turn();
                    if was {
                        Change::TurnEnded
                    } else {
                        Change::Work
                    }
                }
            }
            "agent_state" => {
                self.work.state = event["state"].as_str().unwrap_or_default().to_owned();
                self.work.message = event["message"].as_str().map(str::to_owned);
                if self.work.state == "failed" {
                    let message = self
                        .work
                        .message
                        .clone()
                        .unwrap_or_else(|| "Kerf stopped on an error".into());
                    self.notify(&message, true);
                }
                Change::Work
            }
            "agent" => {
                let text = event["text"].as_str().unwrap_or_default();
                match event["kind"].as_str().unwrap_or_default() {
                    "delta" => self.work.answer.push_str(text),
                    "status" => self.work.status = simplified(text),
                    "done" => {
                        if !text.is_empty() {
                            self.work.answer = text.to_owned();
                        }
                    }
                    "tool" => {
                        let id = event["id"].as_str().unwrap_or_default().to_owned();
                        let state = match event["state"].as_str() {
                            Some("done") => StepState::Done,
                            Some("failed") => StepState::Failed,
                            _ => StepState::Running,
                        };
                        // The step it is on is what the work row says beside "Working".
                        let said = simplified(text);
                        if !said.is_empty() {
                            self.work.status = said;
                        }
                        match self.work.steps.iter_mut().find(|step| step.id == id) {
                            Some(step) => {
                                step.text = text.to_owned();
                                step.state = state;
                            }
                            None => self.work.steps.push(Step {
                                id,
                                text: text.to_owned(),
                                state,
                            }),
                        }
                    }
                    _ => {}
                }
                Change::Work
            }
            "agent_next" => {
                self.next_steps = next_steps(event);
                Change::Work
            }
            "agent_focus" => {
                // Seconds, or a time as a tool wrote it ("01:00:04:12", "96f").
                let timeline = &self.snapshot.timeline;
                self.agent_focus = event["times"]
                    .as_array()
                    .map(|times| {
                        times
                            .iter()
                            .filter_map(|time| {
                                crate::model::focus_seconds(time, timeline.rate(), timeline.start)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.agent_clips = event["clips"]
                    .as_array()
                    .map(|clips| {
                        clips
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                Change::Work
            }
            "agent_context" => {
                if let (Some(used), Some(limit)) = (event["used"].as_f64(), event["limit"].as_f64())
                {
                    // Of the conversation it names, else of the turn's own.
                    let conversation = event["conversation"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .or_else(|| self.work.conversation.clone().filter(|_| self.work.busy()));
                    self.context = Some(ContextUse {
                        conversation,
                        used: used.max(0.0),
                        limit: limit.max(0.0),
                        tokens: event["unit"].as_str() == Some("tokens"),
                    });
                }
                Change::Work
            }
            "clock" => {
                if let Some(position) = event["position"].as_f64() {
                    self.position = position;
                }
                if let Some(duration) = event["duration"].as_f64() {
                    self.duration = duration;
                }
                self.dropped = event["dropped"].as_u64().unwrap_or(self.dropped);
                Change::Clock
            }
            // With kerfd gone, nothing plays and no caption shows (Qt's
            // `m_playing = m_connected && playing`, its caption cleared):
            // what is left of its playback says nothing more.
            "playback" => {
                let gone = matches!(self.connection, Connection::Failed(_));
                self.playing = !gone && event["playing"].as_bool().unwrap_or(false);
                Change::Playback
            }
            "caption" => {
                self.caption = if matches!(self.connection, Connection::Failed(_)) {
                    String::new()
                } else {
                    event["text"].as_str().unwrap_or_default().to_owned()
                };
                Change::Clock
            }
            "meter" => {
                self.meter = event["peak"]
                    .as_array()
                    .and_then(|peak| Some([peak.first()?.as_f64()?, peak.get(1)?.as_f64()?]));
                Change::Other
            }
            "viewer_reset" => Change::ViewerReset,
            "collab" => {
                if event["status"].is_object() {
                    self.collab = Some(event["status"].clone());
                }
                Change::Other
            }
            "preview" => {
                self.preview = Preview {
                    status: event["status"].as_str().unwrap_or_default().to_owned(),
                    progress: event["progress"].as_f64(),
                    message: event["message"].as_str().map(str::to_owned),
                    proposal: event["proposalId"].as_str().map(str::to_owned),
                };
                // Play waits from the moment a render is needed or runs
                // until the picture plays again (Qt's `m_previewRequired`).
                // A render that ends without showing leaves the picture as
                // it was, and Play with it: a stop answers only `stopped`,
                // an exit with nothing shown only `exited`, and the
                // picture says nothing new after either.
                match self.preview.status.as_str() {
                    status @ ("required" | "unavailable") => {
                        self.picture_status = status.to_owned();
                        self.preview_required = true;
                    }
                    status @ ("ready" | "live") => {
                        self.picture_status = status.to_owned();
                        self.preview_required = false;
                    }
                    "running" => self.preview_required = true,
                    "stopped" | "exited" | "failed" => {
                        self.preview_required =
                            matches!(self.picture_status.as_str(), "required" | "unavailable");
                    }
                    _ => {}
                }
                if self.preview.status == "failed" {
                    let message = self
                        .preview
                        .message
                        .clone()
                        .unwrap_or_else(|| "The preview could not render".into());
                    self.say_in_place(StatusLine::Preview, &message, true, None);
                } else if let Some((said, hold)) = preview_notice(event) {
                    // Why a render is needed, said just as one failed, is
                    // the failure said in the bridge's words: still an
                    // error, for Qt's 6 s.
                    let failure =
                        matches!(self.preview.status.as_str(), "required" | "unavailable")
                            && self.preview_line().is_some_and(|line| line.error);
                    let hold = if failure {
                        Some(crate::theme::motion::NOTICE_HOLD)
                    } else {
                        hold
                    };
                    self.say_in_place(StatusLine::Preview, &said, failure, hold);
                }
                Change::Preview
            }
            "render_queue" => {
                self.renders = event["jobs"]
                    .as_array()
                    .map(|jobs| {
                        jobs.iter()
                            .filter_map(|job| serde_json::from_value(job.clone()).ok())
                            .collect()
                    })
                    .unwrap_or_default();
                Change::Renders
            }
            "export" => {
                let status = event["status"].as_str().unwrap_or_default();
                if let Some(job) = event["jobId"].as_str() {
                    let entry = match self.renders.iter_mut().find(|known| known.job_id == job) {
                        Some(entry) => entry,
                        None => {
                            self.renders.push(RenderJob {
                                job_id: job.to_owned(),
                                ..RenderJob::default()
                            });
                            self.renders.last_mut().expect("just pushed")
                        }
                    };
                    entry.status = status.to_owned();
                    if let Some(progress) = event["progress"].as_f64() {
                        entry.progress = progress;
                    }
                    if let Some(name) = event["name"].as_str() {
                        entry.name = name.to_owned();
                    }
                    if let Some(path) = event["path"].as_str() {
                        entry.output_path = path.to_owned();
                    }
                    if let Some(quality) = event["quality"].as_str() {
                        entry.quality = quality.to_owned();
                    }
                }
                match status {
                    "completed" => {
                        let path = event["path"].as_str().unwrap_or_default();
                        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
                        self.notify(&format!("Exported {name}"), false);
                    }
                    "failed" => {
                        let message = event["message"].as_str().unwrap_or("The export failed");
                        self.notify(message, true);
                    }
                    _ => {}
                }
                Change::Renders
            }
            "media_making" => {
                let media = event["media"].as_str().unwrap_or_default().to_owned();
                let take = event["take"].as_u64().unwrap_or(1) as u32;
                let line = event["line"].as_str().unwrap_or_default().to_owned();
                // Its clock starts as its take does: a new take of a making
                // of several starts afresh, with no line yet. Each event says
                // the step's line as it is now, an empty one among them.
                let entry = self.makings.entry(media).or_insert_with(|| Making {
                    take,
                    line: String::new(),
                    since: Instant::now(),
                });
                if entry.take != take {
                    entry.take = take;
                    entry.since = Instant::now();
                }
                entry.line = line;
                Change::Making
            }
            "media_made" | "media_make_cancelled" => {
                let media = event["media"].as_str().unwrap_or_default();
                self.makings.remove(media);
                if media.is_empty() {
                    return Change::Making;
                }
                if kind == "media_made" {
                    // Made again, it no longer wants making.
                    self.wants_making.remove(media);
                    let name = self.snapshot.timeline.media_name(media);
                    let take = event["take"].as_u64().unwrap_or(1);
                    self.notify(&format!("Made take {take} of {name} from its steps"), false);
                } else {
                    self.notify(&format!("Stopped making {media}; it is as it was"), false);
                }
                Change::Making
            }
            "media_status" => {
                self.media_status = MediaStatus::from_value(event);
                Change::Other
            }
            "media_info" => {
                if let Some(media) = event["media"].as_object() {
                    for (id, info) in media {
                        self.media_info.insert(id.clone(), info.clone());
                    }
                }
                Change::Other
            }
            "media_index" => {
                if let Some(media) = event["media"].as_str().filter(|media| !media.is_empty()) {
                    self.index_asked.remove(media);
                    self.media_index_errors.remove(media);
                    self.media_indexes.insert(media.to_owned(), event.clone());
                }
                Change::Other
            }
            "cast" => {
                self.wants_making = event["toMake"]
                    .as_array()
                    .map(|shots| {
                        shots
                            .iter()
                            .filter_map(|shot| {
                                let media = shot["media"].as_str()?;
                                let why = shot["why"].as_str().unwrap_or_default();
                                Some((media.to_owned(), why.to_owned()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.cast = Some(event.clone());
                if event["saved"].as_bool() == Some(true) {
                    // Held as Qt held it (main_window.cpp's 8 s).
                    let said = cast_saved(event);
                    self.notify_held(&said, false, Some(crate::theme::motion::NOTICE_LONG_HOLD));
                    Change::Notice
                } else {
                    Change::Other
                }
            }
            "preset_inspect" => {
                let path = event["path"].as_str().unwrap_or_default().to_owned();
                self.inspected_preset = Some((path, event["preset"].clone()));
                Change::Other
            }
            "preset_applied" => {
                self.notify("Preset proposal ready for review", false);
                Change::Notice
            }
            // The bridge hands back the document; the editor writes it.
            "preset_export" => {
                let path = event["path"].as_str().unwrap_or_default();
                let document = event["document"].as_str().unwrap_or_default();
                match crate::presets::write_document(std::path::Path::new(path), document) {
                    Ok(()) => {
                        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
                        self.notify(&format!("Preset exported: {name}"), false);
                        self.log("preset", path);
                    }
                    Err(error) => self.notify(&format!("Could not export preset: {error}"), true),
                }
                Change::Notice
            }
            "preset_saved" => {
                self.saving_preset = None;
                self.preset_exists = None;
                let path = event["path"].as_str().unwrap_or_default();
                self.notify(&format!("Saved preset {path}"), false);
                self.log("preset", path);
                Change::Notice
            }
            _ => Change::Other,
        }
    }

    /// Lets go of what was learned of a media whose file another snapshot
    /// names otherwise, or that it no longer has: an index belongs to the
    /// file it was made of.
    fn forget_relinked(&mut self, next: &Snapshot) {
        let before = &self.snapshot.timeline.sources;
        let gone: Vec<String> = before
            .iter()
            .filter(|old| {
                next.timeline
                    .source(&old.id)
                    .is_none_or(|new| new.path != old.path)
            })
            .map(|old| old.id.clone())
            .collect();
        for id in gone {
            self.media_indexes.remove(&id);
            self.media_index_errors.remove(&id);
            self.index_asked.remove(&id);
            self.media_info.remove(&id);
            self.info_asked.remove(&id);
        }
    }

    /// What media `id` is doing: being made or waiting its turn (as its
    /// making's line says, `stage::chip::phase`), else not made yet or
    /// offline when its file is missing (a shot made from steps, or a file
    /// that is gone), else ready.
    pub fn media_activity(&self, id: &str) -> MediaActivity {
        if let Some(making) = self.makings.get(id) {
            return match crate::stage::chip::phase(&making.line) {
                crate::stage::chip::Phase::Waiting { ahead } => MediaActivity::Queued { ahead },
                phase if phase.queued() => MediaActivity::Queued { ahead: None },
                _ => MediaActivity::Making {
                    take: making.take,
                    since: making.since,
                },
            };
        }
        if self.media_status.missing.contains(id) {
            let made = self
                .snapshot
                .timeline
                .source(id)
                .is_some_and(crate::model::Source::is_made);
            return if made {
                MediaActivity::NotMade
            } else {
                MediaActivity::Offline
            };
        }
        MediaActivity::Ready
    }

    /// How full Kerf's context is, when that reading is of the conversation
    /// shown: the one it names, or (named by none yet) the turn under way in
    /// a conversation just started. None elsewhere, and before any turn
    /// has said.
    pub fn context_shown(&self) -> Option<&ContextUse> {
        let context = self
            .context
            .as_ref()
            .filter(|context| context.limit > 0.0)?;
        let here = match (&context.conversation, &self.chosen) {
            (Some(of), Some(chosen)) => of == chosen,
            (None, chosen) => {
                self.work.busy() && chosen.as_ref() == self.work.conversation.as_ref()
            }
            (Some(_), None) => false,
        };
        here.then_some(context)
    }

    /// Chooses the conversation to show; None is a new one.
    pub fn choose(&mut self, conversation: Option<String>) {
        if conversation != self.chosen {
            self.open_conversation = None;
        }
        if let Some(id) = &conversation {
            self.looked_at(id);
        }
        self.chosen = conversation;
    }

    fn end_turn(&mut self) {
        if let Some(conversation) = self.work.conversation.take() {
            if self.chosen.as_deref() != Some(conversation.as_str())
                && !self.settled.contains(&conversation)
            {
                self.settled.push(conversation);
            }
        }
        self.work.started = None;
        self.work.request = None;
        self.agent_focus.clear();
        self.agent_clips.clear();
    }

    /// The conversation shown, as its title says it: the one chosen, by its
    /// title; else, for a new one, the first line of the request running
    /// in it; else "New conversation".
    pub fn conversation_title(&self) -> String {
        let titled = |title: &str| Some(title.trim().to_owned()).filter(|title| !title.is_empty());
        if let Some(chosen) = &self.chosen {
            let title = self
                .conversations
                .iter()
                .find(|known| &known.id == chosen)
                .and_then(|known| titled(&known.title))
                .or_else(|| {
                    self.open_conversation
                        .as_ref()
                        .filter(|open| &open.id == chosen)
                        .and_then(|open| titled(&open.title))
                });
            if let Some(title) = title {
                return title;
            }
        }
        let live = self
            .work
            .request
            .as_deref()
            .filter(|_| self.work.conversation == self.chosen)
            .map(title_line)
            .filter(|line| !line.is_empty());
        live.unwrap_or_else(|| "New conversation".to_owned())
    }

    /// Says `said` in place of what the last of its kind (`line`) said, as
    /// a line of Qt's status bar took the last one's place: what a
    /// `preview` event says ([`preview_notice`]), the mode the composer was
    /// set to. The same words again stay where they are, held afresh.
    pub fn say_in_place(
        &mut self,
        line: StatusLine,
        said: &str,
        error: bool,
        hold: Option<Duration>,
    ) {
        let shown = self
            .said_in_place
            .get(&line)
            .and_then(|id| self.notices.iter().position(|notice| notice.id == *id));
        match shown {
            Some(at) if self.notices[at].text == said && self.notices[at].error == error => {
                self.notices[at].at = Instant::now();
                self.notices[at].hold = hold;
            }
            _ => {
                if let Some(at) = shown {
                    self.notices.remove(at);
                }
                self.notify_held(said, error, hold);
                match self.notices.last().filter(|notice| notice.text == said) {
                    Some(notice) => {
                        self.said_in_place.insert(line, notice.id);
                    }
                    None => {
                        self.said_in_place.remove(&line);
                    }
                }
            }
        }
    }

    /// The notice the last of `line`'s kind left, while it shows.
    pub fn line_shown(&self, line: StatusLine) -> Option<&Notice> {
        let id = self.said_in_place.get(&line)?;
        self.notices.iter().find(|notice| notice.id == *id)
    }

    /// The notice the last `preview` event left, while it shows.
    pub fn preview_line(&self) -> Option<&Notice> {
        self.line_shown(StatusLine::Preview)
    }

    /// Whether a preview can be rendered now (Render Preview): only while
    /// the picture needs one (`required`) or plays live (`live`), as Qt's
    /// `previewRenderable` had it; not before the bridge has said, nor while
    /// one renders, nor once it is ready. A render that ended unshown
    /// (`stopped`, `exited`, `failed`) goes by what the picture last said.
    pub fn preview_renderable(&self) -> bool {
        let status = match self.preview.status.as_str() {
            "stopped" | "exited" | "failed" => self.picture_status.as_str(),
            status => status,
        };
        matches!(status, "required" | "live")
    }

    /// Whether Play can run: kerfd is connected, the picture waits on no
    /// rendered preview ([`EditorState::preview_required`]), and there is
    /// something to play (Qt's `togglePlayback`).
    pub fn playable(&self) -> bool {
        self.connection == Connection::Connected
            && !self.preview_required
            && self.program_length() > 0.0
    }

    /// What Play / Pause sends now: `pause` whenever something plays (kerfd
    /// plays on through a render started meanwhile), else `play` while Play
    /// can run ([`EditorState::playable`]); None while it cannot, as Qt's
    /// `togglePlayback` returned.
    pub fn playback_toggle(&self) -> Option<&'static str> {
        if self.playing {
            Some("pause")
        } else {
            self.playable().then_some("play")
        }
    }

    /// Whether Play / Pause waits on a rendered preview, as the command
    /// registry is told (its `previewRequired`): while one is needed or
    /// runs ([`EditorState::preview_required`]) and nothing plays. What
    /// plays (live, as a render runs) can always be paused.
    pub fn play_waits(&self) -> bool {
        self.preview_required && !self.playing
    }

    /// Keeps an Activity line the session takes ([`EditorState::take_logged`]).
    fn log(&mut self, kind: &'static str, text: &str) {
        let text = text.trim();
        if !text.is_empty() {
            self.logged.push((kind, text.to_owned()));
        }
    }

    /// The Activity lines events left that only the state knows of (a
    /// preset exported or saved, by its path, as Qt kept them), oldest
    /// first; taken, they are gone.
    pub fn take_logged(&mut self) -> Vec<(&'static str, String)> {
        std::mem::take(&mut self.logged)
    }

    pub fn notify(&mut self, text: &str, error: bool) {
        self.notify_held(text, error, None);
    }

    /// Says `text` as [`EditorState::notify`] does, for `hold` when it
    /// keeps a time of its own (None: the window's). Qt's sync summary, its
    /// pull request's line and a cast saved stayed 8 s
    /// (`theme::motion::NOTICE_LONG_HOLD`).
    pub fn notify_held(&mut self, text: &str, error: bool, hold: Option<Duration>) {
        // The same words again, at once, say nothing new.
        if self
            .notices
            .last()
            .is_some_and(|last| last.text == text && last.at.elapsed().as_secs_f32() < 1.0)
        {
            return;
        }
        self.noticed += 1;
        self.notices.push(Notice {
            id: self.noticed,
            text: text.to_owned(),
            error,
            at: Instant::now(),
            hold,
        });
        // Only the last few are worth showing.
        if self.notices.len() > 4 {
            self.notices.remove(0);
        }
    }

    /// Drops notices that have been said for `hold`, and errors for
    /// `error_hold` (what went wrong takes longer to read), each that
    /// keeps a time of its own for that ([`Notice::held_for`]); true when
    /// any went.
    pub fn expire_notices(&mut self, hold: Duration, error_hold: Duration) -> bool {
        let before = self.notices.len();
        self.notices
            .retain(|notice| notice.at.elapsed() < notice.held_for(hold, error_hold));
        before != self.notices.len()
    }

    /// Whether anything renders: a preview or a job in the queue.
    pub fn rendering(&self) -> Option<String> {
        if self.preview.rendering() {
            let percent = self
                .preview
                .progress
                .map(|p| format!(" {:.0}%", p * 100.0))
                .unwrap_or_default();
            return Some(format!("Rendering preview{percent}"));
        }
        self.renders
            .iter()
            .find(|job| job.status == "running")
            .map(|job| {
                let name = if job.name.is_empty() {
                    "export"
                } else {
                    &job.name
                };
                format!("Rendering {name} {:.0}%", job.progress * 100.0)
            })
    }

    /// What a making says on the stage's chip.
    pub fn making_label(&self) -> Option<String> {
        self.makings.iter().next().map(|(media, making)| {
            let elapsed = making.since.elapsed().as_secs();
            format!("Making {media} · {}:{:02}", elapsed / 60, elapsed % 60)
        })
    }

    /// Marks a conversation looked at: its settled dot goes.
    pub fn looked_at(&mut self, conversation: &str) {
        self.settled.retain(|id| id != conversation);
    }

    /// The sequence's length in seconds, as the clock or the snapshot says.
    pub fn program_length(&self) -> f64 {
        if self.duration > 0.0 {
            self.duration
        } else {
            self.snapshot.timeline.duration.max(self.snapshot.duration)
        }
    }
}
