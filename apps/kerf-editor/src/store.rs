//! The editor's state, as the bridge's events leave it. Pure data: no
//! window, no GPUI, so every event can be tested on its own. The window
//! draws from it; nothing here keeps a second editable copy of the
//! project, which only kerfd owns.

use crate::model::{Conversation, ConversationSummary, RenderJob, Snapshot};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Instant;

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

/// A note the window shows for a while: an error, or what finished.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    /// Its place among every notice there has been, to tell it from the rest.
    pub id: u64,
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

/// A rendered preview's state (`preview` events).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub status: String,
    pub progress: Option<f64>,
    pub message: Option<String>,
    pub proposal: Option<String>,
}

impl Preview {
    pub fn rendering(&self) -> bool {
        self.status == "running"
    }
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
    /// Next steps Kerf offers as buttons.
    pub next_steps: Vec<String>,
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
    /// How full Kerf's context is: used, limit.
    pub context: Option<(f64, f64)>,
    /// The caption under the playhead.
    pub caption: String,
    /// Peak level, left and right, dBFS.
    pub meter: Option<[f64; 2]>,
    /// Conversations that finished while you looked elsewhere.
    pub settled: Vec<String>,
    /// The project's Git standing, as the last `collab` event said it.
    pub collab: Option<Value>,
    /// How many notices there have been.
    noticed: u64,
}

/// Commands whose refusals the view that sent them shows in place.
fn answered_in_place(command: &str) -> bool {
    matches!(command, "media_make" | "media_carry" | "takes_read")
        || command.starts_with("recipe")
        || command.starts_with("shot_")
        || command.starts_with("workflow")
}

impl EditorState {
    /// Folds one bridge event into the state.
    pub fn apply(&mut self, event: &Value) -> Change {
        let kind = event["type"].as_str().unwrap_or_default();
        match kind {
            "state" => {
                self.snapshot = Snapshot::from_value(&event["snapshot"]);
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
                // What a shot's graph or a making's plan asked for is answered
                // where it was asked, not again over the conversation.
                if answered_in_place(command) {
                    return Change::Making;
                }
                let message = event["message"].as_str().unwrap_or("Something went wrong");
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
                        self.work.status = "Working".into();
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
                    "status" => self.work.status = text.to_owned(),
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
                self.next_steps = event["steps"]
                    .as_array()
                    .map(|steps| {
                        steps
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                Change::Work
            }
            "agent_focus" => {
                self.agent_focus = event["times"]
                    .as_array()
                    .map(|times| times.iter().filter_map(Value::as_f64).collect())
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
                    self.context = Some((used, limit));
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
            "playback" => {
                self.playing = event["playing"].as_bool().unwrap_or(false);
                Change::Playback
            }
            "caption" => {
                self.caption = event["text"].as_str().unwrap_or_default().to_owned();
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
                if self.preview.status == "failed" {
                    let message = self
                        .preview
                        .message
                        .clone()
                        .unwrap_or_else(|| "The preview could not render".into());
                    self.notify(&message, true);
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
                let entry = self.makings.entry(media).or_insert_with(|| Making {
                    take,
                    line: String::new(),
                    since: Instant::now(),
                });
                entry.take = take;
                if !line.is_empty() {
                    entry.line = line;
                }
                Change::Making
            }
            "media_made" | "media_make_cancelled" => {
                if let Some(media) = event["media"].as_str() {
                    self.makings.remove(media);
                }
                Change::Making
            }
            _ => Change::Other,
        }
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

    pub fn notify(&mut self, text: &str, error: bool) {
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
        });
        // Only the last few are worth showing.
        if self.notices.len() > 4 {
            self.notices.remove(0);
        }
    }

    /// Drops notices older than `seconds`; true when any went.
    pub fn expire_notices(&mut self, seconds: f32) -> bool {
        let before = self.notices.len();
        self.notices
            .retain(|notice| notice.at.elapsed().as_secs_f32() < seconds);
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
