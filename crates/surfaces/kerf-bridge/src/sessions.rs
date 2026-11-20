//! The agent sessions behind the editor's conversations, one for each, as a
//! coding agent keeps a session per thread.
//!
//! Going back to a conversation goes on with its own session: the agent has
//! what it looked at, the frames it saw and the proposals it made, not a
//! summary of the words. A few stay open at once ([`MOST_OPEN`]); the one
//! used longest ago is kept and closed to make room. Each is kept beside
//! its conversation when a turn ends, so the next launch goes on from it:
//! Kerf's own agent as its transcript (`.kerf/conversations/sessions/<id>.json`),
//! Claude Code as its own session's id, which the conversation's file holds
//! and `--resume` takes.

use crate::conversations::{self, Store};
use crate::{Agent, AgentSettings};
use std::path::{Path, PathBuf};

/// The most sessions open at once.
pub(crate) const MOST_OPEN: usize = 4;

/// Where Kerf's own agent's transcripts are kept, in the conversations' folder.
const FOLDER: &str = "sessions";

/// How a turn's session came to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    /// It was open: it has everything said so far.
    Continued,
    /// It was kept, and goes on from where it was.
    Restored,
    /// Nothing was kept of it: it hears the conversation's words first.
    New,
}

struct Open {
    conversation: String,
    settings: AgentSettings,
    agent: Agent,
    used: u64,
}

pub(crate) struct Sessions {
    store: Store,
    folder: PathBuf,
    open: Vec<Open>,
    clock: u64,
}

impl Sessions {
    pub(crate) fn of(project: &Path) -> Self {
        Self {
            store: Store::of(project),
            folder: transcripts(project),
            open: Vec::new(),
            clock: 0,
        }
    }

    fn transcript(&self, conversation: &str) -> Option<PathBuf> {
        conversations::valid_id(conversation)
            .then(|| self.folder.join(format!("{conversation}.json")))
    }

    /// The session for `conversation` with `settings`: the open one, or one
    /// `open` makes, gone on from what was kept of it where anything was.
    pub(crate) fn get(
        &mut self,
        conversation: &str,
        settings: &AgentSettings,
        open: impl FnOnce() -> Result<Agent, String>,
    ) -> Result<(&mut Agent, Opened), String> {
        self.clock += 1;
        let clock = self.clock;
        if let Some(index) = self
            .open
            .iter()
            .position(|open| open.conversation == conversation)
        {
            if &self.open[index].settings == settings {
                let open = &mut self.open[index];
                open.used = clock;
                return Ok((&mut open.agent, Opened::Continued));
            }
            // Another model or provider: what this one knows is kept, and the
            // new one goes on from it.
            let closing = self.open.remove(index);
            self.keep(&closing);
        }
        let mut agent = open()?;
        let opened = self.restore(conversation, &mut agent);
        if self.open.len() >= MOST_OPEN {
            if let Some(index) = self
                .open
                .iter()
                .enumerate()
                .min_by_key(|(_, open)| open.used)
                .map(|(index, _)| index)
            {
                let closing = self.open.remove(index);
                self.keep(&closing);
            }
        }
        self.open.push(Open {
            conversation: conversation.to_owned(),
            settings: settings.clone(),
            agent,
            used: clock,
        });
        let open = self.open.last_mut().expect("just pushed");
        Ok((&mut open.agent, opened))
    }

    /// Go on in `agent` from what was kept of `conversation`.
    fn restore(&self, conversation: &str, agent: &mut Agent) -> Opened {
        match agent {
            Agent::Native(session, _) => match self.transcript(conversation) {
                Some(path) if path.is_file() && session.restore(&path).is_ok() => Opened::Restored,
                _ => Opened::New,
            },
            Agent::ClaudeCode(session) => {
                match self
                    .store
                    .load(conversation)
                    .and_then(|kept| kept.claude_session)
                {
                    Some(id) => {
                        session.resume(id);
                        Opened::Restored
                    }
                    None => Opened::New,
                }
            }
        }
    }

    /// Open `conversation` afresh, forgetting what was kept of its session:
    /// for a kept session that no longer goes on.
    pub(crate) fn forget(&mut self, conversation: &str) {
        self.open.retain(|open| open.conversation != conversation);
        if let Some(path) = self.transcript(conversation) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Keep what `conversation`'s session knows, at the end of its turn:
    /// Kerf's agent's transcript, or Claude Code's session id, which the
    /// conversation's file holds.
    pub(crate) fn kept(&self, conversation: &str) -> Option<String> {
        let open = self
            .open
            .iter()
            .find(|open| open.conversation == conversation)?;
        self.keep(open)
    }

    fn keep(&self, open: &Open) -> Option<String> {
        // A conversation deleted meanwhile is not written again.
        self.store.load(&open.conversation)?;
        match &open.agent {
            Agent::Native(session, _) => {
                if session.message_count() > 0 {
                    if let Some(path) = self.transcript(&open.conversation) {
                        let _ = session.keep(&path);
                    }
                }
                None
            }
            Agent::ClaudeCode(session) => session.session().map(str::to_owned),
        }
    }
}

/// Where a project's agent transcripts are kept.
pub(crate) fn transcripts(project: &Path) -> PathBuf {
    project.join(conversations::FOLDER).join(FOLDER)
}

/// Remove what was kept of `conversation`'s session, with the conversation.
pub(crate) fn remove(project: &Path, conversation: &str) {
    if conversations::valid_id(conversation) {
        let _ = std::fs::remove_file(transcripts(project).join(format!("{conversation}.json")));
    }
}
