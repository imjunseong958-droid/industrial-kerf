//! `make`: an agent asks for a media to be made from its steps, which the document holds
//! under the media's line or a recipe's file the line names.
//!
//! What it gets depends on the person's own permissions
//! ([`crate::permissions`]). Asking, the default, it is shown what the
//! making would run, as a person is, and the person is asked in the
//! conversation (the editor shows the plan as a card with Make), or makes it
//! with `kerf make` or the editor's Make…, which show the same plan and ask. Where the person lets an agent make on its own, the
//! making runs here, on a thread of this process, as a job the agent reads
//! and can stop, and the media becomes the new take once it is made: an
//! edit undo takes back, and a take kept beside the ones before it.

use super::{KerfMcpRouter, McpError};
use crate::harness::operation_key;
use kerf_engine::recipe::{self, model::Models};
use kerf_engine::{CancellationToken, EngineError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// How a making's asking starts.
const ASKS: &str = "Asks to make ";

/// The lines a job keeps of what its steps say: the latest.
const KEPT_LINES: usize = 20;
/// How long `wait` waits before it answers that the making goes on.
const MOST_WAIT: Duration = Duration::from_secs(20 * 60);
const WAIT_PAUSE: Duration = Duration::from_millis(200);

/// What a making this session started on its own is doing, for whoever shows makings: an
/// editor says in its toolbar what is being made, as it does of one its person started, and
/// can stop it.
#[derive(Clone, Debug)]
pub enum MakingEvent {
    /// It began: the take it is making, and what stops it.
    Began {
        media: String,
        take: u32,
        stop: CancellationToken,
    },
    /// A line one of its steps said.
    Said {
        media: String,
        take: u32,
        line: String,
    },
    /// Its take is made, and the document has it.
    Made { media: String, take: u32 },
    /// It was stopped, and the media is as it was.
    Stopped { media: String },
    /// It made no take, and why.
    Failed { media: String, why: String },
}

/// Who is told of this session's makings: see [`KerfMcpRouter::watch_makings`].
pub type MakingWatcher = Arc<dyn Fn(MakingEvent) + Send + Sync>;

/// The makings this session started, shared by every copy of the router.
#[derive(Default)]
pub(crate) struct Makings {
    /// How many it ran without asking, which the person's limit counts.
    started: u32,
    jobs: Vec<Job>,
    /// Who is told what they do.
    watcher: Option<MakingWatcher>,
}

struct Job {
    id: String,
    media: String,
    take: u32,
    token: CancellationToken,
    state: Arc<Mutex<JobState>>,
    thread: Option<thread::JoinHandle<()>>,
}

#[derive(Clone)]
enum Status {
    Making,
    Made {
        revision: u64,
        path: String,
        /// The takes it made, the last of which the media is.
        takes: Vec<u32>,
    },
    Failed(String),
    Stopped,
}

struct JobState {
    status: Status,
    said: VecDeque<String>,
}

impl JobState {
    fn json(&self, id: &str, media: &str, take: u32) -> Value {
        let mut value = json!({
            "job": id,
            "media": media,
            "take": take,
            "said": self.said.iter().collect::<Vec<_>>(),
        });
        match &self.status {
            Status::Making => value["status"] = json!("making"),
            Status::Made {
                revision,
                path,
                takes,
            } => {
                let take = takes.last().copied().unwrap_or(take);
                value["status"] = json!("made");
                value["revision"] = json!(revision);
                value["path"] = json!(path);
                value["take"] = json!(take);
                value["next"] = json!(if takes.len() > 1 {
                    value["takes"] = json!(takes);
                    format!(
                        "{media} is take {take} now, the last of takes {}: look at each with \
                         `look` `media` and `sheet`, say which kept the most, and have the media \
                         be it with its `use` line",
                        takes
                            .iter()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    format!(
                        "{media} is take {take} now: look at it with `look` `media` and `sheet`, \
                         and say what the model kept and what it lost"
                    )
                });
            }
            Status::Failed(why) => {
                value["status"] = json!("failed");
                value["error"] = json!(why);
            }
            Status::Stopped => {
                value["status"] = json!("stopped");
                value["error"] = json!(format!(
                    "stopped: no take was made, and {media} is as it was"
                ));
            }
        }
        value
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MakeInput {
    media: Option<String>,
    why: Option<String>,
    job: Option<String>,
    #[serde(default)]
    wait: bool,
    #[serde(default)]
    cancel: bool,
    /// How many takes to make, each with its seeds one on from the last.
    rolls: Option<u32>,
}

pub(super) fn metadata() -> (&'static str, Value, super::ToolAnnotations) {
    (
        "Make a media from the steps under its line in the document. Returns the plan: each step as it would run and the take it \
         would write. The person's own permissions say what happens next: where they let an agent \
         make on its own it runs (`making`), as a `job` to read with `job`, `wait` for, or stop \
         with `cancel`, and the media becomes the new take when it is made; else they are asked \
         (`asked`), in the conversation, and make it themselves. Call it right after the edit \
         that wrote the steps, once that is accepted. Mend what `look`'s `toMake` says would \
         refuse it first.",
        json!({
            "type": "object",
            "properties": {
                "media": {"type": "string", "description": "The media to make."},
                "rolls": {"type": "integer", "minimum": 1, "maximum": 8, "description": "With `media`: make this many takes, one after another, each with its steps' seeds one on from the last, to choose among. Each is a making the person pays for and their limit counts: ask for more than one only when they asked for takes to choose from."},
                "why": {"type": "string", "description": "What this making is for, said to the person with the plan: what changed since the last take, and what to look for."},
                "job": {"type": "string", "description": "A making this session started: what it has said, and how it ended."},
                "wait": {"type": "boolean", "description": "Answer once the making has ended, up to twenty minutes, rather than at once."},
                "cancel": {"type": "boolean", "description": "With `job`: stop it. Its step is asked to end, so a model's program can take back what it asked a server for."}
            },
            "additionalProperties": false
        }),
        super::ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(false),
        },
    )
}

impl KerfMcpRouter {
    pub(super) fn make(&self, arguments: Value) -> Result<Value, McpError> {
        let input: MakeInput = super::harness::input(arguments)?;
        match (&input.media, &input.job) {
            (None, Some(job)) => {
                if input.why.is_some() || input.rolls.is_some() {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        "`why` and `rolls` go with a `media` to make",
                    ));
                }
                self.making_job(job, input.cancel, input.wait)
            }
            (Some(media), None) => {
                if input.cancel {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        "`cancel` stops a `job`; pass the job a making answered with",
                    ));
                }
                let rolls = input.rolls.unwrap_or(1);
                if !(1..=8).contains(&rolls) {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        "`rolls` is how many takes to make, 1 to 8",
                    ));
                }
                self.make_media(media, input.why.as_deref(), input.wait, rolls)
            }
            _ => Err(McpError::new(
                "INVALID_ARGUMENTS",
                "pass `media` to make one, or `job` to read a making",
            )),
        }
    }

    fn make_media(
        &self,
        media: &str,
        why: Option<&str>,
        wait: bool,
        rolls: u32,
    ) -> Result<Value, McpError> {
        let (_, project) = self.project_at(None)?;
        // A media an edit has just added is in its proposal, not in the project, until the
        // person accepts it: a making is of the project, and is not asked of a proposal.
        if project.media(media).is_none() {
            // Its script names it, and what it would make of the project has it.
            let adds = |proposal: &String| {
                self.project_at(Some(proposal))
                    .is_ok_and(|(_, proposed)| proposed.media(media).is_some())
            };
            if let Some(proposal) = self.pending_with(media).filter(adds) {
                return Err(McpError::new(
                    "CANNOT_MAKE",
                    format!(
                        "no media `{media}` in this project yet: it is in proposal {proposal}, which waits for the person; ask for the making once it is accepted"
                    ),
                ));
            }
        }
        let models =
            Models::load().map_err(|error| McpError::new("CANNOT_MAKE", error.to_string()))?;
        let cannot = |error: EngineError| {
            McpError::new(
                "CANNOT_MAKE",
                super::harness::cannot_said(error.to_string(), Some(&models)),
            )
        };
        // One making is the recipe as it is written. Several are rolls of it, each with its
        // seeds one on from the last, from the first roll no take of it has had.
        let first = if rolls > 1 {
            recipe::next_roll(&project, &self.dir, media).map_err(cannot)?
        } else {
            0
        };
        let plan =
            recipe::plan_rolled(&project, &self.dir, media, &models, first).map_err(cannot)?;
        let permissions = self.permissions().map_err(|error| {
            McpError::new(
                "PERMISSIONS",
                format!("{error}; the person mends their own permissions file"),
            )
        })?;
        let mut makings = self.makings.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(running) = makings.jobs.iter().find(|job| {
            job.media == media
                && matches!(
                    job.state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .status,
                    Status::Making
                )
        }) {
            return Err(McpError::new(
                "MAKING",
                format!("{media} is being made already: job {}", running.id),
            ));
        }
        let mut answer = json!({
            "media": media,
            "take": plan.take,
            "steps": plan.steps,
            "permissions": permissions.mode.name(),
        });
        if !plan.notes.is_empty() {
            answer["notes"] = json!(plan.notes);
        }
        if rolls > 1 {
            answer["rolls"] = json!(rolls);
            answer["takes"] = json!((plan.take..plan.take + rolls).collect::<Vec<_>>());
        }
        // Every roll is a making the person's limit counts: all of them are let through, or
        // the person is asked for them all.
        if !permissions.runs_makings(makings.started + rolls - 1) {
            drop(makings);
            let takes = if rolls > 1 {
                format!("takes {} to {}", plan.take, plan.take + rolls - 1)
            } else {
                format!("take {}", plan.take)
            };
            let mut body = format!(
                "{ASKS}{media}, as {takes}, from the steps under its line in the document:"
            );
            if let Some(why) = why.map(str::trim).filter(|why| !why.is_empty()) {
                body = format!("{body}\n{why}");
            }
            for (number, step) in plan.steps.iter().enumerate() {
                body.push_str(&format!("\n{}. {step}", number + 1));
            }
            for note in &plan.notes {
                body.push_str(&format!("\nNote: {note}"));
            }
            body.push_str(&if rolls > 1 {
                format!(
                    "\nEach take after the first runs the same steps with every seed one on from the last.\nMake them with `kerf make {media} --rolls {rolls}`, or Make Several in the editor's bin."
                )
            } else {
                format!("\nMake it with `kerf make {media}`, or Make… in the editor's bin.")
            });
            // What the person is asked, for the agent to say in the conversation.
            answer["status"] = json!("asked");
            answer["ask"] = json!(body);
            answer["make"] = json!(if rolls > 1 {
                format!(
                    "say `ask` to the person, who makes them with `kerf make {media} --rolls \
                     {rolls}` or Make Several; `look` `media` lists the takes once they are made"
                )
            } else {
                format!(
                    "say `ask` to the person, who makes it with `kerf make {media}` or Make…; \
                     `look` lists the take once it is made"
                )
            });
            if permissions.mode == crate::permissions::Mode::Auto {
                answer["why"] = json!(format!(
                    "this session has made the {} its person lets an agent make on its own",
                    permissions.makes
                ));
            }
            return Ok(answer);
        }
        makings.started += rolls;
        let take = plan.take;
        let id = format!("make-{}", operation_key("job"));
        let token = CancellationToken::new();
        let state = Arc::new(Mutex::new(JobState {
            status: Status::Making,
            said: VecDeque::new(),
        }));
        // Whoever shows makings is told of this one as it goes, from its start.
        let tell = {
            let watcher = makings.watcher.clone();
            move |event: MakingEvent| {
                if let Some(watcher) = &watcher {
                    watcher(event);
                }
            }
        };
        tell(MakingEvent::Began {
            media: media.to_owned(),
            take,
            stop: token.clone(),
        });
        let thread = {
            let (router, token, state) = (self.clone(), token.clone(), state.clone());
            let media = media.to_owned();
            thread::spawn(move || {
                // The takes made, in order, and why the making after the last was not.
                let mut all: Vec<recipe::Made> = Vec::new();
                let mut ended = Ok(());
                for roll in 0..rolls {
                    let plan = match roll {
                        0 => Ok(plan.clone()),
                        roll => recipe::plan_rolled(
                            &project,
                            &router.dir,
                            &media,
                            &models,
                            first + roll,
                        ),
                    };
                    let made = plan.and_then(|plan| {
                        let take = plan.take;
                        if roll > 0 {
                            tell(MakingEvent::Began {
                                media: media.clone(),
                                take,
                                stop: token.clone(),
                            });
                        }
                        let mut last = String::new();
                        recipe::make(&project, &router.dir, &plan, &models, &token, |line| {
                            let line = line.trim();
                            if line.is_empty() || line == last {
                                return;
                            }
                            last = line.to_owned();
                            tell(MakingEvent::Said {
                                media: media.clone(),
                                take,
                                line: line.to_owned(),
                            });
                            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                            // How far a step has come is one line, said again as it goes on.
                            let further = |said: &str| {
                                let named = |line: &str| {
                                    line.rsplit_once(": ").map(|(named, _)| named.to_owned())
                                };
                                line.ends_with('%')
                                    && said.ends_with('%')
                                    && named(line) == named(said)
                            };
                            if state.said.back().is_some_and(|said| further(said)) {
                                state.said.pop_back();
                            } else if state.said.len() == KEPT_LINES {
                                state.said.pop_front();
                            }
                            state.said.push_back(line.to_owned());
                        })
                    });
                    match made {
                        Ok(made) => all.push(made),
                        Err(error) => {
                            ended = Err(error);
                            break;
                        }
                    }
                }
                // What was made is kept, whatever the making after it came to: the media
                // becomes the last take there is, and the job says why it has no more.
                let takes: Vec<u32> = all.iter().map(|made| made.take).collect();
                let made = match (all.pop(), ended) {
                    (Some(made), ended) => {
                        if let Err(error) = ended {
                            state
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .said
                                .push_back(format!(
                                    "made {} of the {rolls} takes, and the next was not: {error}",
                                    takes.len()
                                ));
                        }
                        Ok(made)
                    }
                    (None, Err(error)) => Err(error),
                    (None, Ok(())) => unreachable!("a making is of one take at least"),
                };
                let status = match made {
                    Ok(made) => match router.place_take(&media, &made.stored) {
                        Ok(revision) => Status::Made {
                            revision,
                            path: made.stored,
                            takes: takes.clone(),
                        },
                        Err(error) => Status::Failed(format!(
                            "take {} was made, and the document could not take it: {}; its `use` \
                             line in `look` `media` has the media be it",
                            made.take, error.message
                        )),
                    },
                    Err(EngineError::Cancelled) => Status::Stopped,
                    Err(error) => Status::Failed(error.to_string()),
                };
                tell(match &status {
                    Status::Made { takes, .. } => MakingEvent::Made {
                        media: media.clone(),
                        take: takes.last().copied().unwrap_or(take),
                    },
                    Status::Stopped => MakingEvent::Stopped {
                        media: media.clone(),
                    },
                    Status::Failed(why) => MakingEvent::Failed {
                        media: media.clone(),
                        why: why.clone(),
                    },
                    Status::Making => unreachable!("a making that ended is not under way"),
                });
                state.lock().unwrap_or_else(PoisonError::into_inner).status = status;
            })
        };
        makings
            .jobs
            .retain(|job| job.thread.as_ref().is_some_and(|t| !t.is_finished()));
        makings.jobs.push(Job {
            id: id.clone(),
            media: media.to_owned(),
            take,
            token,
            state,
            thread: Some(thread),
        });
        drop(makings);
        if wait {
            return self.making_job(&id, false, true).map(|mut job| {
                job["steps"] = answer["steps"].take();
                job["permissions"] = answer["permissions"].take();
                job
            });
        }
        answer["status"] = json!("making");
        answer["job"] = json!(id);
        Ok(answer)
    }

    /// What a making this session started has said and how it ended: once it has, with `wait`.
    fn making_job(&self, id: &str, cancel: bool, wait: bool) -> Result<Value, McpError> {
        let found = {
            let makings = self.makings.lock().unwrap_or_else(PoisonError::into_inner);
            makings.jobs.iter().find(|job| job.id == id).map(|job| {
                (
                    job.media.clone(),
                    job.take,
                    job.token.clone(),
                    job.state.clone(),
                )
            })
        };
        let (media, take, token, state) = found.ok_or_else(|| {
            McpError::new(
                "UNKNOWN_JOB",
                format!("no making `{id}` in this session: a job lasts as long as the session that started it"),
            )
        })?;
        if cancel {
            token.cancel();
        }
        let started = Instant::now();
        loop {
            {
                let state = state.lock().unwrap_or_else(PoisonError::into_inner);
                let ended = !matches!(state.status, Status::Making);
                if ended || !(wait || cancel) || started.elapsed() >= MOST_WAIT {
                    let mut value = state.json(id, &media, take);
                    if !ended && wait {
                        value["still"] = json!("it goes on; pass `job` to read it again");
                    }
                    return Ok(value);
                }
            }
            thread::sleep(WAIT_PAUSE);
        }
    }

    /// Have media `media` be the take at `stored`, as `kerf make` does once it is made: the
    /// revision that is.
    fn place_take(&self, media: &str, stored: &str) -> Result<u64, McpError> {
        let revision = self.runtime.get_state()?.revision;
        let author = self.author();
        let actor = if author.kind == "mcp" { "mcp" } else { "agent" };
        let state = self.runtime.replace_media_request(
            actor,
            &operation_key("agent-make"),
            media,
            stored,
            revision,
        )?;
        Ok(state.revision)
    }

    /// Wait for the makings this session started, stopping them first when `cancel`. They run on
    /// this process's threads, so a process that ends with one running leaves its model's job
    /// on a server: call this before exiting.
    pub fn finish_makings(&self, cancel: bool) {
        let jobs = std::mem::take(
            &mut self
                .makings
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .jobs,
        );
        if cancel {
            for job in &jobs {
                job.token.cancel();
            }
        }
        for mut job in jobs {
            if let Some(thread) = job.thread.take() {
                // A making thread that panicked has said so already; it is only waited for here.
                let _ = thread.join();
            }
        }
    }

    /// Have `watcher` told what the makings this session starts on its own do, from each
    /// one's start to its end: through this router and every copy of it. An editor shows
    /// them where it shows the makings its person starts.
    pub fn watch_makings(&self, watcher: MakingWatcher) {
        self.makings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .watcher = Some(watcher);
    }

    /// How many makings this session is running.
    pub fn running_makings(&self) -> usize {
        self.makings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .jobs
            .iter()
            .filter(|job| {
                job.thread
                    .as_ref()
                    .is_some_and(|thread| !thread.is_finished())
            })
            .count()
    }
}
