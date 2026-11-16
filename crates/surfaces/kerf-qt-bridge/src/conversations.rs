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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
}

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
        match fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("cannot delete {}: {error}", file.display())),
        }
    }
}

/// Ids this store makes and reads: `c-` and letters, digits, `-` or `_`, so an
/// id never names a path outside the folder.
fn valid_id(id: &str) -> bool {
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
pub(crate) fn emit_list(store: &Store, tx: &SyncSender<Value>) {
    let items: Vec<Value> = store
        .list()
        .into_iter()
        .map(|conversation| {
            json!({
                "id": conversation.id,
                "title": conversation.title,
                "created": conversation.created,
                "updated": conversation.updated,
                "archived": conversation.archived,
                "count": conversation.items.len(),
            })
        })
        .collect();
    emit(tx, json!({"type": "conversations", "items": items}));
}

/// One conversation whole: `conversation {id, title, …, items}`.
pub(crate) fn emit_one(conversation: &Conversation, tx: &SyncSender<Value>) {
    let mut event = serde_json::to_value(conversation).unwrap_or_else(|_| json!({}));
    event["type"] = json!("conversation");
    emit(tx, event);
}

/// The conversation commands the editor sends, as the bridge reads them.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Request {
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
pub(crate) fn command(store: &Store, request: Request, tx: &SyncSender<Value>) {
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
            json!({"type": "error", "command": "conversation", "message": message}),
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
        tx: &SyncSender<Value>,
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
            answer: None,
            stopped: false,
        })
    }

    /// The conversation the turn is in.
    pub(crate) fn id(&self) -> &str {
        &self.conversation.id
    }

    /// Everything asked and answered before this turn, as text for an agent
    /// that starts again on a conversation it has not seen.
    pub(crate) fn earlier(&self) -> Option<String> {
        let before = &self.conversation.items[..self.conversation.items.len().saturating_sub(1)];
        if before.is_empty() {
            return None;
        }
        let mut text = String::from("Earlier in this conversation:\n");
        for item in before {
            let who = if item.kind == "you" {
                "The person"
            } else {
                "You (Kerf)"
            };
            text.push_str(&format!("{who}: {}\n", item.text.trim()));
        }
        Some(text)
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
        tx: &SyncSender<Value>,
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
            ..Item::default()
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn events(rx: &mpsc::Receiver<Value>) -> Vec<Value> {
        rx.try_iter().collect()
    }

    #[test]
    fn a_turn_is_kept_in_its_conversation_and_listed_latest_first() {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::of(folder.path());
        let (tx, rx) = mpsc::sync_channel(64);
        let mut turn = Turn::begin(
            &store,
            None,
            "Make the title say Welcome instead",
            Some(1.5),
            &tx,
        )
        .unwrap();
        let id = turn.id().to_owned();
        assert!(turn.earlier().is_none());
        turn.step("edit · done · awaiting_review");
        turn.step("edit · done · awaiting_review");
        turn.done("The title now reads Welcome.");
        turn.finish(
            &store,
            Outcome::Done(String::new()),
            Some("p-1".into()),
            &tx,
        )
        .unwrap();
        let kept = store.load(&id).unwrap();
        assert_eq!(kept.title, "Make the title say Welcome instead");
        assert_eq!(kept.items.len(), 2);
        assert_eq!(
            (kept.items[0].kind.as_str(), kept.items[0].timecode),
            ("you", Some(1.5))
        );
        assert_eq!(kept.items[1].kind, "kerf");
        assert_eq!(kept.items[1].steps, ["edit · done · awaiting_review"]);
        assert_eq!(kept.items[1].proposal.as_deref(), Some("p-1"));
        assert!(kept.items[1].seconds.is_some());
        let sent = events(&rx);
        assert!(sent.iter().any(|event| event["type"] == "conversation"
            && event["items"].as_array().unwrap().len() == 2));
        assert!(sent.iter().any(|event| event["type"] == "conversations"));

        // The next turn goes on in it, and knows what came before.
        let second = Turn::begin(&store, Some(&id), "Now make it yellow", None, &tx).unwrap();
        assert!(second.earlier().unwrap().contains("Welcome"));
        second
            .finish(&store, Outcome::Failed("no key".into()), None, &tx)
            .unwrap();
        let kept = store.load(&id).unwrap();
        assert_eq!(kept.items.len(), 4);
        assert!(kept.items[3].error);

        // Another conversation is listed above it once it is newer.
        let other = Turn::begin(&store, None, "Cut the bridge", None, &tx).unwrap();
        let other_id = other.id().to_owned();
        let listed: Vec<String> = store.list().into_iter().map(|c| c.id).collect();
        assert_eq!(listed, [other_id.clone(), id.clone()]);
    }

    #[test]
    fn a_conversation_is_archived_renamed_and_deleted() {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::of(folder.path());
        let (tx, rx) = mpsc::sync_channel(64);
        let turn = Turn::begin(&store, None, "A", None, &tx).unwrap();
        let id = turn.id().to_owned();
        events(&rx);
        command(
            &store,
            Request::Archive {
                id: id.clone(),
                archived: true,
            },
            &tx,
        );
        assert!(store.load(&id).unwrap().archived);
        // Asking in it again brings it back.
        Turn::begin(&store, Some(&id), "B", None, &tx).unwrap();
        assert!(!store.load(&id).unwrap().archived);
        command(
            &store,
            Request::Rename {
                id: id.clone(),
                title: "Bridge".into(),
            },
            &tx,
        );
        assert_eq!(store.load(&id).unwrap().title, "Bridge");
        events(&rx);
        command(&store, Request::Delete { id: id.clone() }, &tx);
        assert!(store.load(&id).is_none());
        let sent = events(&rx);
        assert_eq!(sent.last().unwrap()["items"], json!([]));
        // An id that is no file name of this folder is refused.
        command(
            &store,
            Request::Delete {
                id: "../project".into(),
            },
            &tx,
        );
        assert_eq!(events(&rx)[0]["type"], "error");
        assert!(folder.path().join(FOLDER).is_dir());
    }

    #[test]
    fn a_long_request_is_titled_by_its_first_words() {
        assert_eq!(title_of("  \nShort one\nsecond line"), "Short one");
        let long =
            "Make a short sequence on a big suspension bridge: cars drive past in both directions";
        let title = title_of(long);
        assert!(
            title.ends_with('…') && title.chars().count() <= TITLE_CHARS + 1,
            "{title}"
        );
        assert_eq!(title_of(""), "New conversation");
    }

    #[test]
    fn requests_read_as_the_editor_writes_them() {
        let read = |value: Value| serde_json::from_value::<Request>(value).unwrap();
        assert_eq!(read(json!({"action": "list"})), Request::List);
        assert_eq!(
            read(json!({"action": "archive", "id": "c-1"})),
            Request::Archive {
                id: "c-1".into(),
                archived: true
            }
        );
        assert_eq!(
            read(json!({"action": "archive", "id": "c-1", "archived": false})),
            Request::Archive {
                id: "c-1".into(),
                archived: false
            }
        );
    }
}
