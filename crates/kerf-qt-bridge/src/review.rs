//! The review log as the editor sees it: the comment commands, the
//! `comments` event, and agent turns kept as comments.
//!
//! Comments live in kerfd's review log (`review/comments.jsonl`), never in
//! the document. The bridge writes the editor's own notes as the person
//! Git says this is, re-sends the whole list after every change it makes,
//! and follows `reviewRevision` in the state snapshot so notes written by
//! anyone else (an MCP agent, a Git merge) appear live.

use crate::{emit, Shared};
use kerf_runtime::{
    Author, Comment, CommentPatch, NewComment, ReviewLog, RuntimeClient, RuntimeError, Stroke,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::SyncSender;

/// What reads and writes the review log: kerfd over its socket, or a
/// runtime in this process in the tests.
pub(crate) trait Reviews {
    fn list(&self) -> Result<ReviewLog, RuntimeError>;
    fn add(&self, actor: &str, comment: NewComment) -> Result<(Comment, ReviewLog), RuntimeError>;
    fn set(
        &self,
        actor: &str,
        id: &str,
        patch: CommentPatch,
    ) -> Result<(Option<Comment>, ReviewLog), RuntimeError>;
}

impl Reviews for RuntimeClient {
    fn list(&self) -> Result<ReviewLog, RuntimeError> {
        self.comments()
    }

    fn add(&self, actor: &str, comment: NewComment) -> Result<(Comment, ReviewLog), RuntimeError> {
        self.add_comment(actor, comment)
    }

    fn set(
        &self,
        actor: &str,
        id: &str,
        patch: CommentPatch,
    ) -> Result<(Option<Comment>, ReviewLog), RuntimeError> {
        self.set_comment(actor, id, patch)
    }
}

/// `comment_add {body, at?, duration?, parent?, tags?, blocking?, drawing?,
/// proposal?, id?}`: a note by the person using the editor. A client-chosen
/// `id` makes a retried add harmless.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct AddRequest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub at: Option<f64>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default)]
    pub drawing: Vec<Stroke>,
    #[serde(default)]
    pub proposal: Option<String>,
}

/// `comment_set {id, resolved?, body?, tags?, blocking?, deleted?}`.
#[derive(Debug, Deserialize)]
pub(crate) struct SetRequest {
    pub id: String,
    #[serde(default)]
    pub resolved: Option<bool>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub blocking: Option<bool>,
    #[serde(default)]
    pub deleted: Option<bool>,
}

/// `agent {…, comment: {at?, duration?, parent?, drawing?}}`: keep the turn
/// in the review log, where on the program it is about, and in which
/// thread when `parent` names one.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(crate) struct TurnComment {
    #[serde(default)]
    pub at: Option<f64>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub drawing: Vec<Stroke>,
}

/// The person using the editor, as Git knows them: who "You" is.
pub(crate) fn person(directory: &Path) -> Author {
    match kerf_collab::git::user(directory) {
        Some(user) if !user.name.trim().is_empty() => Author {
            kind: "person".into(),
            name: user.name,
            id: user.email,
            role: None,
        },
        _ => Author::new("person", "You"),
    }
}

/// Kerf's own agent, which answers in the editor.
fn kerf() -> Author {
    Author::kerf()
}

/// Send the comments as `log` has them: `comments {items, reviewRevision,
/// comment?}`, `comment` being the one a command just wrote. A list older
/// than one already sent is dropped, so a slow reader never puts an older
/// list over a newer one; the same revision is sent again only when
/// `force`, as `comment_list` asks.
pub(crate) fn emit_log(
    shared: &Shared,
    tx: &SyncSender<Value>,
    log: ReviewLog,
    comment: Option<&Comment>,
    force: bool,
) {
    let Ok(mut seen) = shared.review_seen.lock() else {
        return;
    };
    match *seen {
        Some(last) if log.review_revision < last => return,
        Some(last) if log.review_revision == last && !force && comment.is_none() => return,
        _ => {}
    }
    *seen = Some(log.review_revision);
    let mut event = json!({
        "type": "comments",
        "items": log.items,
        "reviewRevision": log.review_revision,
    });
    if let Some(comment) = comment {
        event["comment"] = json!(comment);
    }
    // Sent while holding the lock, so lists reach the editor in the order
    // their revisions say.
    emit(tx, event);
}

/// Forget which list was sent: a new connection may be to a new kerfd,
/// whose counter starts again.
pub(crate) fn reset(shared: &Shared) {
    if let Ok(mut seen) = shared.review_seen.lock() {
        *seen = None;
    }
}

/// The state said the log is at `revision`: send the comments again when
/// that is not the list the editor has.
pub(crate) fn follow(
    reviews: &impl Reviews,
    revision: u64,
    shared: &Shared,
    tx: &SyncSender<Value>,
) -> Result<(), RuntimeError> {
    let known = shared
        .review_seen
        .lock()
        .map_or(true, |seen| *seen == Some(revision));
    if known {
        return Ok(());
    }
    let log = reviews.list()?;
    emit_log(shared, tx, log, None, false);
    Ok(())
}

/// A comment command that kerfd refused: `error {message, code, command,
/// id?}`, so the composer can say why its note did not post.
pub(crate) fn refused(
    tx: &SyncSender<Value>,
    command: &str,
    id: Option<&str>,
    error: &RuntimeError,
) {
    let mut event = json!({
        "type": "error",
        "message": error.message,
        "code": error.code,
        "command": command,
    });
    if let Some(id) = id {
        event["id"] = json!(id);
    }
    emit(tx, event);
}

/// `comment_list`: the comments, sent even when the editor has them.
pub(crate) fn list(
    reviews: &impl Reviews,
    shared: &Shared,
    tx: &SyncSender<Value>,
) -> Result<(), RuntimeError> {
    let log = reviews.list()?;
    emit_log(shared, tx, log, None, true);
    Ok(())
}

/// `comment_add`, written by `author`.
pub(crate) fn add(
    reviews: &impl Reviews,
    author: Author,
    request: AddRequest,
    shared: &Shared,
    tx: &SyncSender<Value>,
) -> Result<(), RuntimeError> {
    let mut comment = NewComment::new(author, request.body);
    comment.id = request.id;
    comment.at = request.at;
    comment.duration = request.duration;
    comment.parent = request.parent;
    comment.tags = request.tags;
    comment.blocking = request.blocking;
    comment.drawing = request.drawing;
    comment.proposal = request.proposal;
    let (comment, log) = reviews.add("ui", comment)?;
    emit_log(shared, tx, log, Some(&comment), false);
    Ok(())
}

/// `comment_set`, by `by` (who resolved, reopened or deleted it).
pub(crate) fn set(
    reviews: &impl Reviews,
    by: Author,
    request: SetRequest,
    shared: &Shared,
    tx: &SyncSender<Value>,
) -> Result<(), RuntimeError> {
    let patch = CommentPatch {
        resolved: request.resolved,
        body: request.body,
        tags: request.tags,
        blocking: request.blocking,
        deleted: request.deleted,
        by: Some(by),
        ..CommentPatch::default()
    };
    let (comment, log) = reviews.set("ui", &request.id, patch)?;
    emit_log(shared, tx, log, comment.as_ref(), false);
    Ok(())
}

/// Carry out `comment_list`, `comment_add` or `comment_set` on the runtime
/// worker, as the person Git says uses the editor in `directory`. Returns
/// the command's name and the comment id it names, for a refusal.
pub(crate) fn command(
    command: crate::Command,
    reviews: &impl Reviews,
    directory: &Path,
    shared: &Shared,
    tx: &SyncSender<Value>,
) -> (&'static str, Option<String>, Result<(), RuntimeError>) {
    match command {
        crate::Command::CommentAdd(request) => {
            let id = request.id.clone();
            let result = add(reviews, person(directory), request, shared, tx);
            ("comment_add", id, result)
        }
        crate::Command::CommentSet(request) => {
            let id = Some(request.id.clone());
            let result = set(reviews, person(directory), request, shared, tx);
            ("comment_set", id, result)
        }
        _ => ("comment_list", None, list(reviews, shared, tx)),
    }
}

/// The most steps one reply keeps (kerfd's own limit).
const MAX_STEPS: usize = 256;
/// The longest step kept, in bytes.
const MAX_STEP_BYTES: usize = 512;

/// What the bridge writes for an agent turn asked with a `comment`: the
/// person's request as their comment, then Kerf's answer as a reply with
/// the steps it took and the proposal it staged.
pub(crate) struct Turn {
    /// The thread's root, which the reply goes under.
    root: String,
    steps: Vec<String>,
    answer: Option<String>,
    stopped: bool,
}

/// How a turn ended.
pub(crate) enum Outcome {
    /// The agent finished; its last words, if the stream gave none.
    Done(String),
    /// It failed, saying why.
    Failed(String),
}

impl Turn {
    /// Record the person's request `prompt` before the agent runs.
    pub(crate) fn begin(
        reviews: &impl Reviews,
        person: Author,
        prompt: &str,
        comment: TurnComment,
        shared: &Shared,
        tx: &SyncSender<Value>,
    ) -> Result<Self, RuntimeError> {
        let mut asked = NewComment::new(person, prompt.trim());
        asked.at = comment.at;
        asked.duration = comment.duration;
        asked.parent = comment.parent.clone();
        asked.drawing = comment.drawing;
        let (asked, log) = reviews.add("ui", asked)?;
        emit_log(shared, tx, log, Some(&asked), false);
        let root = comment.parent.unwrap_or(asked.id);
        Ok(Self {
            root,
            steps: Vec::new(),
            answer: None,
            stopped: false,
        })
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

    /// Write Kerf's reply: the answer, or the failure with `error`. A
    /// proposal the turn staged is linked from the reply, and from the
    /// thread's root when that is about no proposal yet, so accepting it
    /// resolves the thread.
    pub(crate) fn finish(
        self,
        reviews: &impl Reviews,
        outcome: Outcome,
        proposal: Option<String>,
        shared: &Shared,
        tx: &SyncSender<Value>,
    ) -> Result<Comment, RuntimeError> {
        let (body, failed) = match outcome {
            Outcome::Failed(message) => (message, true),
            Outcome::Done(fallback) => {
                let answer = self
                    .answer
                    .filter(|answer| !answer.trim().is_empty())
                    .unwrap_or(fallback);
                (answer, false)
            }
        };
        let body = match body.trim() {
            "" if self.stopped => "Stopped before finishing.".to_owned(),
            "" if failed => "The agent failed.".to_owned(),
            "" => "Done.".to_owned(),
            text => text.to_owned(),
        };
        let mut reply = NewComment::new(kerf(), body);
        reply.parent = Some(self.root.clone());
        reply.steps = self.steps;
        reply.error = failed;
        reply.proposal = proposal.clone();
        let (reply, log) = reviews.add("agent", reply)?;
        let mut last = (log, Some(reply.clone()));
        if let Some(proposal) = proposal {
            let root = last.0.items.iter().find(|item| item.id == self.root);
            if root.is_some_and(|root| root.proposal.is_none()) {
                let patch = CommentPatch {
                    proposal: Some(proposal),
                    ..CommentPatch::default()
                };
                let (_, log) = reviews.set("agent", &self.root, patch)?;
                last.0 = log;
            }
        }
        emit_log(shared, tx, last.0, last.1.as_ref(), false);
        Ok(reply)
    }
}

/// The ids of the open proposals in `state`, before a turn runs.
pub(crate) fn open_proposals(state: &kerf_runtime::StateSnapshot) -> HashSet<String> {
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
    before: &HashSet<String>,
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
pub(crate) mod tests {
    use super::*;
    use kerf_runtime::{RuntimeHandle, RuntimeRequest, RuntimeResponse};
    use std::sync::mpsc;

    /// A runtime in this process: kerfd's review log without a socket.
    pub(crate) struct Local(pub RuntimeHandle);

    fn comments(response: RuntimeResponse) -> (ReviewLog, Option<Comment>) {
        match response {
            RuntimeResponse::Comments { log, comment } => (log, comment),
            other => panic!("expected comments, got {other:?}"),
        }
    }

    impl Reviews for Local {
        fn list(&self) -> Result<ReviewLog, RuntimeError> {
            Ok(comments(self.0.request(RuntimeRequest::ListComments)?).0)
        }

        fn add(
            &self,
            actor: &str,
            comment: NewComment,
        ) -> Result<(Comment, ReviewLog), RuntimeError> {
            let (log, comment) = comments(self.0.request(RuntimeRequest::AddComment {
                actor: actor.into(),
                comment,
            })?);
            Ok((comment.expect("the comment added"), log))
        }

        fn set(
            &self,
            actor: &str,
            id: &str,
            patch: CommentPatch,
        ) -> Result<(Option<Comment>, ReviewLog), RuntimeError> {
            let (log, comment) = comments(self.0.request(RuntimeRequest::SetComment {
                actor: actor.into(),
                id: id.into(),
                patch,
            })?);
            Ok((comment, log))
        }
    }

    pub(crate) fn local(directory: &Path) -> Local {
        Local(RuntimeHandle::open(directory).unwrap())
    }

    fn you() -> Author {
        Author {
            kind: "person".into(),
            name: "Mina Park".into(),
            id: Some("mina@example.com".into()),
            role: None,
        }
    }

    fn events(rx: &mpsc::Receiver<Value>) -> Vec<Value> {
        rx.try_iter().collect()
    }

    #[test]
    fn comment_commands_read_as_the_contract_says() {
        let Ok(crate::Command::CommentAdd(add)) = serde_json::from_value(json!({
            "type": "comment_add", "body": "Open on the road", "at": 3.0, "duration": 1.5,
            "parent": "c-1", "tags": ["color"], "blocking": true, "proposal": "proposal-2",
            "drawing": [{"tool": "ellipse", "points": [[0.6, 0.2], [0.8, 0.4]], "color": "#ededed"}],
        })) else {
            panic!("expected comment_add");
        };
        assert_eq!(add.body, "Open on the road");
        assert_eq!((add.at, add.duration), (Some(3.0), Some(1.5)));
        assert_eq!(add.parent.as_deref(), Some("c-1"));
        assert!(add.blocking && add.tags == ["color"]);
        assert_eq!(add.drawing[0].tool, "ellipse");
        assert_eq!(add.proposal.as_deref(), Some("proposal-2"));
        assert!(matches!(
            serde_json::from_value(json!({"type": "comment_add", "body": "x"})).unwrap(),
            crate::Command::CommentAdd(AddRequest { at: None, .. })
        ));
        let Ok(crate::Command::CommentSet(set)) = serde_json::from_value(json!({
            "type": "comment_set", "id": "c-1", "resolved": true, "deleted": false,
        })) else {
            panic!("expected comment_set");
        };
        assert_eq!(set.id, "c-1");
        assert_eq!(
            (set.resolved, set.deleted, set.body),
            (Some(true), Some(false), None)
        );
        assert!(serde_json::from_value::<crate::Command>(json!({"type": "comment_set"})).is_err());
        assert!(matches!(
            serde_json::from_value(json!({"type": "comment_list"})).unwrap(),
            crate::Command::CommentList
        ));
        let Ok(crate::Command::Agent { comment, .. }) = serde_json::from_value(json!({
            "type": "agent", "text": "limit the music bus here",
            "comment": {"at": 27.72, "duration": 1.08, "parent": "c-2718"},
        })) else {
            panic!("expected agent");
        };
        let comment = comment.expect("the turn's comment");
        assert_eq!(comment.parent.as_deref(), Some("c-2718"));
        assert_eq!(comment.at, Some(27.72));
        assert!(matches!(
            serde_json::from_value(json!({"type": "agent", "text": "x"})).unwrap(),
            crate::Command::Agent { comment: None, .. }
        ));
    }

    #[test]
    fn notes_written_here_are_the_persons_and_each_change_resends_the_list() {
        let directory = tempfile::tempdir().unwrap();
        let reviews = local(directory.path());
        let shared = crate::tests_support::shared(directory.path());
        let (tx, rx) = mpsc::sync_channel(16);

        add(
            &reviews,
            you(),
            AddRequest {
                id: Some("c-qt-1".into()),
                body: "Open on the road, not the sky".into(),
                at: Some(3.0),
                tags: vec!["director".into()],
                ..AddRequest::default()
            },
            &shared,
            &tx,
        )
        .unwrap();
        let sent = events(&rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["type"], "comments");
        assert_eq!(sent[0]["comment"]["id"], "c-qt-1");
        let items = sent[0]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["author"]["kind"], "person");
        assert_eq!(items[0]["author"]["id"], "mina@example.com");
        assert_eq!(items[0]["at"], 3.0);
        let first = sent[0]["reviewRevision"].as_u64().unwrap();

        set(
            &reviews,
            you(),
            SetRequest {
                id: "c-qt-1".into(),
                resolved: Some(true),
                body: None,
                tags: None,
                blocking: None,
                deleted: None,
            },
            &shared,
            &tx,
        )
        .unwrap();
        let sent = events(&rx);
        assert_eq!(sent[0]["items"][0]["resolved"], true);
        assert_eq!(sent[0]["items"][0]["resolvedBy"]["name"], "Mina Park");
        assert!(sent[0]["reviewRevision"].as_u64().unwrap() > first);

        // The state poll at this revision sends nothing new; a listing does.
        let revision = sent[0]["reviewRevision"].as_u64().unwrap();
        follow(&reviews, revision, &shared, &tx).unwrap();
        assert!(events(&rx).is_empty());
        list(&reviews, &shared, &tx).unwrap();
        assert_eq!(events(&rx).len(), 1);

        // Deleted notes leave the list.
        set(
            &reviews,
            you(),
            SetRequest {
                id: "c-qt-1".into(),
                resolved: None,
                body: None,
                tags: None,
                blocking: None,
                deleted: Some(true),
            },
            &shared,
            &tx,
        )
        .unwrap();
        assert_eq!(events(&rx)[0]["items"], json!([]));

        // kerfd's refusal is not a list.
        let empty = add(&reviews, you(), AddRequest::default(), &shared, &tx).unwrap_err();
        assert_eq!(empty.code, "INVALID_COMMENT");
        assert!(events(&rx).is_empty());
    }

    #[test]
    fn notes_from_other_clients_arrive_when_the_review_revision_moves() {
        let directory = tempfile::tempdir().unwrap();
        let reviews = local(directory.path());
        let shared = crate::tests_support::shared(directory.path());
        let (tx, rx) = mpsc::sync_channel(16);

        // A new connection always sends the list once.
        follow(&reviews, 0, &shared, &tx).unwrap();
        assert_eq!(events(&rx)[0]["items"], json!([]));
        follow(&reviews, 0, &shared, &tx).unwrap();
        assert!(events(&rx).is_empty());

        // An MCP agent's note, written through its own client.
        let mut note = NewComment::new(
            Author {
                kind: "mcp".into(),
                name: "Claude Code".into(),
                id: Some("claude-code".into()),
                role: Some("MCP · via Kerf".into()),
            },
            "Updated to v2",
        );
        note.at = Some(26.5);
        let (_, log) = reviews.add("mcp", note).unwrap();
        follow(&reviews, log.review_revision, &shared, &tx).unwrap();
        let sent = events(&rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["items"][0]["author"]["name"], "Claude Code");

        // An older list never replaces a newer one.
        emit_log(&shared, &tx, ReviewLog::default(), None, true);
        assert!(events(&rx).is_empty());
        reset(&shared);
        emit_log(&shared, &tx, ReviewLog::default(), None, false);
        assert_eq!(events(&rx).len(), 1, "a new connection starts again");
    }

    /// The person's turn, then the events a stub agent would stream.
    fn stub_turn(
        reviews: &Local,
        shared: &Shared,
        tx: &SyncSender<Value>,
        comment: TurnComment,
    ) -> Turn {
        let mut turn = Turn::begin(
            reviews,
            you(),
            "  Sea back toward cyan  ",
            comment,
            shared,
            tx,
        )
        .unwrap();
        for line in [
            "inspect · done",
            "inspect · done",
            "edit · done · awaiting_review",
        ] {
            turn.step(line);
        }
        turn
    }

    #[test]
    fn an_agent_turn_becomes_a_comment_and_kerfs_reply() {
        let directory = tempfile::tempdir().unwrap();
        let reviews = local(directory.path());
        let shared = crate::tests_support::shared(directory.path());
        let (tx, rx) = mpsc::sync_channel(32);

        let mut turn = stub_turn(
            &reviews,
            &shared,
            &tx,
            TurnComment {
                at: Some(9.12),
                duration: Some(2.0),
                ..TurnComment::default()
            },
        );
        // The person's comment is in the log before the agent answers.
        let asked = events(&rx);
        assert_eq!(asked.len(), 1);
        let root = asked[0]["comment"].clone();
        assert_eq!(root["body"], "Sea back toward cyan");
        assert_eq!(root["author"]["kind"], "person");
        assert_eq!(root["at"], 9.12);
        assert_eq!(root["parent"], Value::Null);

        turn.done("Pulled the sea back toward cyan.");
        let reply = turn
            .finish(
                &reviews,
                Outcome::Done(String::new()),
                Some("proposal-3".into()),
                &shared,
                &tx,
            )
            .unwrap();
        assert_eq!(reply.author.kind, "kerf");
        assert_eq!(reply.parent.as_deref(), root["id"].as_str());
        assert_eq!(reply.body, "Pulled the sea back toward cyan.");
        assert_eq!(
            reply.steps,
            ["inspect · done", "edit · done · awaiting_review"]
        );
        assert!(!reply.error);
        assert_eq!(reply.proposal.as_deref(), Some("proposal-3"));

        // One list after the reply, with the root linked to the proposal so
        // accepting it resolves the thread.
        let sent = events(&rx);
        assert_eq!(sent.len(), 1, "{sent:?}");
        let items = sent[0]["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["proposal"], "proposal-3");
        assert_eq!(sent[0]["comment"]["id"], json!(reply.id));
    }

    #[test]
    fn a_failed_turn_replies_with_the_error_in_the_thread_it_was_asked_in() {
        let directory = tempfile::tempdir().unwrap();
        let reviews = local(directory.path());
        let shared = crate::tests_support::shared(directory.path());
        let (tx, rx) = mpsc::sync_channel(32);
        let (qc, _) = reviews
            .add(
                "system",
                NewComment::new(Author::new("qc", "QC"), "True peak −0.4 dBTP"),
            )
            .unwrap();

        let turn = stub_turn(
            &reviews,
            &shared,
            &tx,
            TurnComment {
                parent: Some(qc.id.clone()),
                ..TurnComment::default()
            },
        );
        let asked = events(&rx);
        assert_eq!(asked[0]["comment"]["parent"], json!(qc.id));
        let reply = turn
            .finish(
                &reviews,
                Outcome::Failed("A2 Music is locked. Unlock it, then ask again".into()),
                None,
                &shared,
                &tx,
            )
            .unwrap();
        assert!(reply.error);
        assert_eq!(reply.parent.as_deref(), Some(qc.id.as_str()));
        assert_eq!(reply.body, "A2 Music is locked. Unlock it, then ask again");
        assert_eq!(reply.steps.len(), 2);
        let log = reviews.list().unwrap();
        assert_eq!(log.items.len(), 3);
        assert!(log.items.iter().all(|item| item.proposal.is_none()));
        let _ = events(&rx);

        // A stopped turn with nothing said still answers.
        let mut turn = stub_turn(&reviews, &shared, &tx, TurnComment::default());
        turn.stopped();
        let reply = turn
            .finish(&reviews, Outcome::Done(String::new()), None, &shared, &tx)
            .unwrap();
        assert_eq!(reply.body, "Stopped before finishing.");
        assert!(!reply.error);
    }

    #[test]
    fn a_turn_finds_the_proposal_it_staged() {
        use kerf_runtime::{ProposalEntry, ProposalFreshness, ProposalVersion};
        let entry = |id: &str, status: &str| ProposalEntry {
            proposal_id: id.into(),
            status: status.into(),
            freshness: ProposalFreshness::Current,
            base_revision: 1,
            applied_revision: None,
            script: String::new(),
            source: String::new(),
            document_diff: String::new(),
            author: Author::kerf(),
            version: 1,
            supersedes: None,
            versions: vec![ProposalVersion {
                version: 1,
                proposal_id: id.into(),
                base_revision: 1,
                script: String::new(),
                source: String::new(),
                document_diff: String::new(),
            }],
            staged: None,
        };
        let mut state = crate::tests::snapshot("kerf 1\n", None);
        state.proposals = vec![entry("p1", "pending"), entry("p0", "accepted")];
        let before = open_proposals(&state);
        assert_eq!(staged_proposal(&before, &state), None);
        state.proposals.insert(0, entry("p2", "pending"));
        assert_eq!(staged_proposal(&before, &state).as_deref(), Some("p2"));
    }

    #[test]
    fn long_steps_are_cut_on_a_character() {
        let directory = tempfile::tempdir().unwrap();
        let reviews = local(directory.path());
        let shared = crate::tests_support::shared(directory.path());
        let (tx, _rx) = mpsc::sync_channel(32);
        let mut turn =
            Turn::begin(&reviews, you(), "x", TurnComment::default(), &shared, &tx).unwrap();
        turn.step(&"é".repeat(400));
        turn.step("   ");
        assert_eq!(turn.steps.len(), 1);
        assert!(turn.steps[0].len() <= MAX_STEP_BYTES);
    }
}
