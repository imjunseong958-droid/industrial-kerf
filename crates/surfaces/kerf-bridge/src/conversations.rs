//! Conversations with Kerf: what the person asked and what Kerf answered,
//! turn by turn, kept beside the project as a coding agent keeps its
//! sessions.
//!
//! Each conversation is one file, `.kerf/conversations/<id>.json` in the
//! project folder, written whole after every turn (a temporary file renamed
//! over it, so a reader never sees half of one). The editor lists them in its
//! sidebar, opens one, archives it or deletes it; an agent turn names the
//! conversation it belongs to, or starts a new one with its first words as the
//! title. Nothing here touches the document: a turn's edits are proposals
//! kerfd keeps, and an item only names the proposal its turn staged.

use crate::emit;
use crate::event::{ConversationSummary, ErrorEvent, Event};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::time::Instant;

/// Where conversations are kept, relative to the project folder.
pub(crate) const FOLDER: &str = ".kerf/conversations";

/// The longest title kept, in characters; the first words of the first request.
const TITLE_CHARS: usize = 60;
/// The most steps one turn keeps, and the longest of them, in bytes.
const MAX_STEPS: usize = 256;
const MAX_STEP_BYTES: usize = 512;

/// One conversation as its file holds it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Conversation {
    pub id: String,
    pub title: String,
    pub created: String,
    pub updated: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub items: Vec<Item>,
    /// Claude Code's own session for this conversation, which a later turn
    /// resumes; Kerf's own agent keeps its transcript beside it instead
    /// (`sessions.rs`). Never sent to the editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session: Option<String>,
}

/// One line of a conversation: the person's request (`you`) or Kerf's answer
/// (`kerf`), with what Kerf did on the way.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub at: String,
    /// Where the playhead was when it was asked, in seconds of the program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timecode: Option<f64>,
    /// Kerf's finished tool calls and statuses, one line each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    /// How long Kerf worked, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub error: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
    /// The proposal the turn staged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
    /// The makings Kerf asked the person to run, each a card with Make.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asks: Vec<Ask>,
}

/// A making an agent asked for and the person has to say yes to: the media,
/// how many takes (one unless said), the first of them, and the plan as the
/// person is asked about it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Ask {
    pub media: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rolls: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<u32>,
    pub text: String,
}

/// A conversation as the editor gets it: its file without Claude Code's session.
impl From<&Conversation> for crate::event::Conversation {
    fn from(conversation: &Conversation) -> Self {
        Self {
            id: conversation.id.clone(),
            title: conversation.title.clone(),
            created: conversation.created.clone(),
            updated: conversation.updated.clone(),
            archived: conversation.archived,
            items: conversation.items.iter().map(Into::into).collect(),
        }
    }
}

impl From<&Item> for crate::event::ConversationItem {
    fn from(item: &Item) -> Self {
        Self {
            id: item.id.clone(),
            kind: item.kind.clone(),
            text: item.text.clone(),
            at: item.at.clone(),
            timecode: item.timecode,
            steps: item.steps.clone(),
            seconds: item.seconds,
            error: item.error,
            stopped: item.stopped,
            proposal: item.proposal.clone(),
            asks: item.asks.iter().map(Into::into).collect(),
        }
    }
}

impl From<&Ask> for crate::event::ConversationAsk {
    fn from(ask: &Ask) -> Self {
        Self {
            media: ask.media.clone(),
            rolls: ask.rolls,
            take: ask.take,
            text: ask.text.clone(),
        }
    }
}

/// The most of the earlier conversation a new session hears, in bytes.
const MOST_EARLIER_BYTES: usize = 24 * 1024;
/// The most makings one turn asks for that it keeps.
const MOST_ASKS: usize = 16;

/// The conversations of one project folder.
pub(crate) struct Store {
    folder: PathBuf,
}

impl Store {
    pub(crate) fn of(project: &Path) -> Self {
        Self {
            folder: project.join(FOLDER),
        }
    }

    fn file(&self, id: &str) -> Option<PathBuf> {
        valid_id(id).then(|| self.folder.join(format!("{id}.json")))
    }

    /// Every conversation, the latest first. A file that does not read is
    /// left out, not deleted.
    pub(crate) fn list(&self) -> Vec<Conversation> {
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut all: Vec<Conversation> = entries
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| fs::read_to_string(entry.path()).ok())
            .filter_map(|text| serde_json::from_str::<Conversation>(&text).ok())
            .filter(|conversation| valid_id(&conversation.id))
            .collect();
        all.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
        all
    }

    pub(crate) fn load(&self, id: &str) -> Option<Conversation> {
        let text = fs::read_to_string(self.file(id)?).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub(crate) fn save(&self, conversation: &Conversation) -> Result<(), String> {
        let file = self
            .file(&conversation.id)
            .ok_or_else(|| format!("`{}` is not a conversation id", conversation.id))?;
        fs::create_dir_all(&self.folder).map_err(|error| {
            format!(
                "cannot keep conversations in {}: {error}",
                self.folder.display()
            )
        })?;
        let text = serde_json::to_string_pretty(conversation).map_err(|error| error.to_string())?;
        let partial = file.with_extension("json.partial");
        fs::write(&partial, text)
            .and_then(|()| fs::rename(&partial, &file))
            .map_err(|error| format!("cannot write {}: {error}", file.display()))
    }

    /// A new, empty conversation titled for `request`, not yet written.
    pub(crate) fn start(&self, request: &str) -> Conversation {
        let now = kerf_runtime::author::timestamp();
        Conversation {
            id: format!("c-{}", fresh_id()),
            title: title_of(request),
            created: now.clone(),
            updated: now,
            archived: false,
            items: Vec::new(),
            claude_session: None,
        }
    }

    pub(crate) fn archive(&self, id: &str, archived: bool) -> Result<Conversation, String> {
        let mut conversation = self
            .load(id)
            .ok_or_else(|| format!("no conversation `{id}`"))?;
        conversation.archived = archived;
        self.save(&conversation)?;
        Ok(conversation)
    }

    pub(crate) fn rename(&self, id: &str, title: &str) -> Result<Conversation, String> {
        let mut conversation = self
            .load(id)
            .ok_or_else(|| format!("no conversation `{id}`"))?;
        let title = title.trim();
        if title.is_empty() {
            return Err("a conversation's title has words".to_owned());
        }
        conversation.title = title.chars().take(TITLE_CHARS * 2).collect();
        self.save(&conversation)?;
        Ok(conversation)
    }

    pub(crate) fn delete(&self, id: &str) -> Result<(), String> {
        let file = self
            .file(id)
            .ok_or_else(|| format!("`{id}` is not a conversation id"))?;
        // What its agent's session kept goes with it.
        if let Some(project) = self.folder.parent().and_then(Path::parent) {
            crate::sessions::remove(project, id);
        }
        match fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("cannot delete {}: {error}", file.display())),
        }
    }
}

/// Ids this store makes and reads: `c-` and letters, digits, `-` or `_`, so an
/// id never names a path outside the folder.
pub(crate) fn valid_id(id: &str) -> bool {
    id.len() <= 80
        && id.starts_with("c-")
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
}

/// A conversation's title: the request's first line, cut at a word.
fn title_of(request: &str) -> String {
    let line = request
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if line.chars().count() <= TITLE_CHARS {
        return if line.is_empty() {
            "New conversation".to_owned()
        } else {
            line.to_owned()
        };
    }
    let cut: String = line.chars().take(TITLE_CHARS).collect();
    let cut = cut
        .rsplit_once(' ')
        .map(|(words, _)| words)
        .filter(|words| words.chars().count() > TITLE_CHARS / 2)
        .unwrap_or(&cut);
    format!("{}…", cut.trim_end_matches([',', '.', ';', ':', ' ']))
}

/// The list the sidebar shows: `conversations {items: [{id, title, created,
/// updated, archived, count}]}`.
pub(crate) fn emit_list(store: &Store, tx: &SyncSender<Event>) {
    let items = store
        .list()
        .into_iter()
        .map(|conversation| ConversationSummary {
            count: conversation.items.len(),
            id: conversation.id,
            title: conversation.title,
            created: conversation.created,
            updated: conversation.updated,
            archived: conversation.archived,
        })
        .collect();
    emit(tx, Event::Conversations { items });
}

/// One conversation whole, without what is only the file's (Claude Code's session).
pub(crate) fn emit_one(conversation: &Conversation, tx: &SyncSender<Event>) {
    emit(tx, Event::Conversation(conversation.into()));
}

/// The conversation commands the editor sends, as the bridge reads them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Request {
    /// Every conversation: `conversations`.
    List,
    /// One conversation: `conversation`.
    Open {
        id: String,
    },
    /// Archive it, or bring it back.
    Archive {
        id: String,
        #[serde(default = "yes")]
        archived: bool,
    },
    Rename {
        id: String,
        title: String,
    },
    /// Delete its file. It cannot be undone.
    Delete {
        id: String,
    },
}

/// A new id, unique on this machine: process, time and a counter.
fn fresh_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{nanos:x}{:x}{:x}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn yes() -> bool {
    true
}

/// Carry out `request`, then send what changed.
pub(crate) fn command(store: &Store, request: Request, tx: &SyncSender<Event>) {
    let changed = match request {
        Request::List => Ok(None),
        Request::Open { id } => match store.load(&id) {
            Some(conversation) => {
                emit_one(&conversation, tx);
                return;
            }
            None => Err(format!("no conversation `{id}`")),
        },
        Request::Archive { id, archived } => store.archive(&id, archived).map(Some),
        Request::Rename { id, title } => store.rename(&id, &title).map(Some),
        Request::Delete { id } => store.delete(&id).map(|()| None),
    };
    match changed {
        Ok(conversation) => {
            if let Some(conversation) = conversation {
                emit_one(&conversation, tx);
            }
            emit_list(store, tx);
        }
        Err(message) => emit(
            tx,
            Event::Error(ErrorEvent {
                message,
                command: Some("conversation".to_owned()),
                ..ErrorEvent::default()
            }),
        ),
    }
}

/// How a turn ended.
pub(crate) enum Outcome {
    /// The agent finished; its last words, if the stream gave none.
    Done(String),
    /// It failed, saying why.
    Failed(String),
}

/// An agent turn as its conversation keeps it: the person's request first,
/// then, once it ends, Kerf's answer with the steps it took.
pub(crate) struct Turn {
    conversation: Conversation,
    started: Instant,
    steps: Vec<String>,
    asks: Vec<Ask>,
    answer: Option<String>,
    stopped: bool,
}

impl Turn {
    /// Record `prompt` in conversation `id`, or in a new one when it names
    /// none (or one that is gone), before the agent runs.
    pub(crate) fn begin(
        store: &Store,
        id: Option<&str>,
        prompt: &str,
        timecode: Option<f64>,
        tx: &SyncSender<Event>,
    ) -> Result<Self, String> {
        let mut conversation = id
            .and_then(|id| store.load(id))
            .unwrap_or_else(|| store.start(prompt));
        // Talking again in an archived conversation brings it back.
        conversation.archived = false;
        let now = kerf_runtime::author::timestamp();
        conversation.items.push(Item {
            id: format!("i-{}", fresh_id()),
            kind: "you".into(),
            text: prompt.trim().to_owned(),
            at: now.clone(),
            timecode: timecode.filter(|seconds| seconds.is_finite() && *seconds >= 0.0),
            ..Item::default()
        });
        conversation.updated = now;
        store.save(&conversation)?;
        emit_one(&conversation, tx);
        emit_list(store, tx);
        Ok(Self {
            conversation,
            started: Instant::now(),
            steps: Vec::new(),
            asks: Vec::new(),
            answer: None,
            stopped: false,
        })
    }

    /// The conversation the turn is in.
    pub(crate) fn id(&self) -> &str {
        &self.conversation.id
    }

    /// What was asked and answered before this turn, as text for an agent
    /// that starts again on a conversation it has nothing kept of: the
    /// latest turns, as many as fit, with what Kerf did in each and the
    /// proposal it made.
    pub(crate) fn earlier(&self) -> Option<String> {
        let before = &self.conversation.items[..self.conversation.items.len().saturating_sub(1)];
        if before.is_empty() {
            return None;
        }
        let mut lines = Vec::new();
        let mut size = 0;
        for item in before.iter().rev() {
            let line = if item.kind == "you" {
                format!("The person: {}", item.text.trim())
            } else {
                let mut line = String::from("You (Kerf)");
                if !item.steps.is_empty() {
                    line.push_str(&format!(" [{}]", item.steps.join("; ")));
                }
                if let Some(proposal) = &item.proposal {
                    line.push_str(&format!(" [proposal {proposal}]"));
                }
                format!("{line}: {}", item.text.trim())
            };
            if size + line.len() > MOST_EARLIER_BYTES {
                lines.push("(earlier turns are left out)".to_owned());
                break;
            }
            size += line.len() + 1;
            lines.push(line);
        }
        lines.reverse();
        Some(format!(
            "Earlier in this conversation:\n{}\n",
            lines.join("\n")
        ))
    }

    /// One line of what the agent did: a finished tool call or a status.
    pub(crate) fn step(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || self.steps.len() >= MAX_STEPS {
            return;
        }
        if self.steps.last().is_some_and(|last| last == line) {
            return;
        }
        let mut end = line.len().min(MAX_STEP_BYTES);
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        self.steps.push(line[..end].to_owned());
    }

    /// A making the agent asked the person to run.
    pub(crate) fn ask(&mut self, ask: Ask) {
        if self.asks.len() < MOST_ASKS && !self.asks.contains(&ask) {
            self.asks.push(ask);
        }
    }

    /// The agent's answer, as its `done` event gave it.
    pub(crate) fn done(&mut self, text: &str) {
        self.answer = Some(text.to_owned());
    }

    /// The person stopped the agent.
    pub(crate) fn stopped(&mut self) {
        self.stopped = true;
    }

    /// Write Kerf's answer, or the failure, with the proposal the turn staged.
    pub(crate) fn finish(
        mut self,
        store: &Store,
        outcome: Outcome,
        proposal: Option<String>,
        claude_session: Option<String>,
        tx: &SyncSender<Event>,
    ) -> Result<(), String> {
        let (text, failed) = match outcome {
            Outcome::Failed(message) => (message, true),
            Outcome::Done(fallback) => (
                self.answer
                    .take()
                    .filter(|answer| !answer.trim().is_empty())
                    .unwrap_or(fallback),
                false,
            ),
        };
        let text = match text.trim() {
            "" if self.stopped => "Stopped before finishing.".to_owned(),
            "" if failed => "The agent failed.".to_owned(),
            "" => "Done.".to_owned(),
            text => text.to_owned(),
        };
        let now = kerf_runtime::author::timestamp();
        self.conversation.items.push(Item {
            id: format!("i-{}", fresh_id()),
            kind: "kerf".into(),
            text,
            at: now.clone(),
            steps: std::mem::take(&mut self.steps),
            seconds: Some(self.started.elapsed().as_secs_f64()),
            error: failed,
            stopped: self.stopped,
            proposal,
            asks: std::mem::take(&mut self.asks),
            ..Item::default()
        });
        if claude_session.is_some() {
            self.conversation.claude_session = claude_session;
        }
        self.conversation.updated = now;
        store.save(&self.conversation)?;
        emit_one(&self.conversation, tx);
        emit_list(store, tx);
        Ok(())
    }
}

/// The ids of the open proposals in `state`, before a turn runs.
pub(crate) fn open_proposals(
    state: &kerf_runtime::StateSnapshot,
) -> std::collections::HashSet<String> {
    state
        .proposals
        .iter()
        .filter(|entry| entry.status == "pending")
        .map(|entry| entry.proposal_id.clone())
        .collect()
}

/// The proposal a turn staged: the newest open one that was not open
/// before it (a revise opens a new id too).
pub(crate) fn staged_proposal(
    before: &std::collections::HashSet<String>,
    after: &kerf_runtime::StateSnapshot,
) -> Option<String> {
    after
        .proposals
        .iter()
        .filter(|entry| entry.status == "pending")
        .find(|entry| !before.contains(&entry.proposal_id))
        .map(|entry| entry.proposal_id.clone())
}
