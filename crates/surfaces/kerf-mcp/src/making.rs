//! `make`: an agent asks for a media to be made from its recipe.
//!
//! What it gets depends on the person's own permissions
//! ([`crate::permissions`]). Asking, the default, it is shown what the
//! making would run, as a person is, and the person is asked in the review
//! thread, to make it with `kerf make` or Make from Recipe…, which show the
//! same plan and ask. Where the person lets an agent make on its own, the
//! making runs here, on a thread of this process, as a job the agent reads
//! and can stop, and the media becomes the new take once it is made: an
//! edit undo takes back, and a take kept beside the ones before it.

use super::{KerfMcpRouter, McpError};
use crate::harness::operation_key;
use kerf_engine::recipe::{self, model::Models};
use kerf_engine::{CancellationToken, EngineError};
use kerf_runtime::{Comment, CommentPatch, NewComment, RuntimeClient};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The tag a making's asking has in the review, and how its body starts.
const MAKE_TAG: &str = "make";
const ASKS: &str = "Asks to make ";

/// The lines a job keeps of what its steps say: the latest.
const KEPT_LINES: usize = 20;
/// How long `wait` waits before it answers that the making goes on.
const MOST_WAIT: Duration = Duration::from_secs(20 * 60);
const WAIT_PAUSE: Duration = Duration::from_millis(200);

/// The makings this session started, shared by every copy of the router.
#[derive(Default)]
pub(crate) struct Makings {
    /// How many it ran without asking, which the person's limit counts.
    started: u32,
    jobs: Vec<Job>,
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
    Made { revision: u64, path: String },
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
            Status::Made { revision, path } => {
                value["status"] = json!("made");
                value["revision"] = json!(revision);
                value["path"] = json!(path);
                value["next"] = json!(format!(
                    "{media} is take {take} now: look at it with `look` `media` and `sheet`, and \
                     say what the model kept and what it lost"
                ));
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
}

pub(super) fn metadata() -> (&'static str, Value, super::ToolAnnotations) {
    (
        "Ask for a media to be made from its recipe (`recipe=` on its line). Returns the plan: each \
         step as it would run and the take it would write. The person's own permissions say what \
         happens next: by default they are asked (`asked`), in the review thread, and make it \
         themselves; where they let an agent make on its own it runs (`making`), as a `job` to read \
         with `job`, `wait` for, or stop with `cancel`, and the media becomes the new take when it \
         is made. Mend what `look`'s `toMake` says would refuse it first.",
        json!({
            "type": "object",
            "properties": {
                "media": {"type": "string", "description": "The media to make."},
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
                if input.why.is_some() {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        "`why` goes with a `media` to make",
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
                self.make_media(media, input.why.as_deref(), input.wait)
            }
            _ => Err(McpError::new(
                "INVALID_ARGUMENTS",
                "pass `media` to make one, or `job` to read a making",
            )),
        }
    }

    fn make_media(&self, media: &str, why: Option<&str>, wait: bool) -> Result<Value, McpError> {
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
        let plan = recipe::plan(&project, &self.dir, media, &models).map_err(|error| {
            McpError::new(
                "CANNOT_MAKE",
                super::harness::cannot_said(error.to_string(), Some(&models)),
            )
        })?;
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
        if !permissions.runs_makings(makings.started) {
            drop(makings);
            let mut body = format!(
                "{ASKS}{media}, as take {}, from {}:",
                plan.take,
                std::path::absolute(&self.dir)
                    .ok()
                    .and_then(|dir| plan.recipe.strip_prefix(dir).ok())
                    .unwrap_or(&plan.recipe)
                    .display()
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
            body.push_str(&format!(
                "\nMake it with `kerf make {media}`, or Make from Recipe… in the editor's bin."
            ));
            let author = self.author();
            let actor = if author.kind == "mcp" { "mcp" } else { "agent" };
            let mut comment = NewComment::new(author, &body);
            comment.tags = vec![MAKE_TAG.to_owned()];
            let (comment, _) = self.runtime.add_comment(actor, comment)?;
            answer["status"] = json!("asked");
            answer["comment"] = json!(comment.id);
            answer["make"] = json!(format!(
                "the person is asked, in the review thread, and makes it with `kerf make {media}` \
                 or Make from Recipe…; `look` lists the take once it is made"
            ));
            if permissions.mode == crate::permissions::Mode::Auto {
                answer["why"] = json!(format!(
                    "this session has made the {} its person lets an agent make on its own",
                    permissions.makes
                ));
            }
            return Ok(answer);
        }
        makings.started += 1;
        let take = plan.take;
        let id = format!("make-{}", operation_key("job"));
        let token = CancellationToken::new();
        let state = Arc::new(Mutex::new(JobState {
            status: Status::Making,
            said: VecDeque::new(),
        }));
        let thread = {
            let (router, token, state) = (self.clone(), token.clone(), state.clone());
            let media = media.to_owned();
            thread::spawn(move || {
                let mut last = String::new();
                let made = recipe::make(&project, &router.dir, &plan, &models, &token, |line| {
                    let line = line.trim();
                    if line.is_empty() || line == last {
                        return;
                    }
                    last = line.to_owned();
                    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                    // How far a step has come is one line, said again as it goes on.
                    let further = |said: &str| {
                        let named =
                            |line: &str| line.rsplit_once(": ").map(|(named, _)| named.to_owned());
                        line.ends_with('%') && said.ends_with('%') && named(line) == named(said)
                    };
                    if state.said.back().is_some_and(|said| further(said)) {
                        state.said.pop_back();
                    } else if state.said.len() == KEPT_LINES {
                        state.said.pop_front();
                    }
                    state.said.push_back(line.to_owned());
                });
                let status = match made {
                    Ok(made) => match router.place_take(&media, &made.stored) {
                        Ok(revision) => Status::Made {
                            revision,
                            path: made.stored,
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
                let _ = thread.join();
            }
        }
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

/// The makings agents have asked for and nobody has made: the open comments of the review that
/// `make` left, each with the media it is for.
pub fn make_requests(runtime: &RuntimeClient) -> Vec<(String, Comment)> {
    let Ok(log) = runtime.comments() else {
        return Vec::new();
    };
    log.items
        .into_iter()
        .filter(|comment| !comment.resolved && comment.tags.iter().any(|tag| tag == MAKE_TAG))
        .filter_map(|comment| {
            let media = comment
                .body
                .strip_prefix(ASKS)?
                .split_once(',')?
                .0
                .to_owned();
            Some((media, comment))
        })
        .collect()
}

/// Resolve what agents asked to have made of `media`, once a person has made it.
pub fn answer_make_requests(runtime: &RuntimeClient, media: &str) {
    for (_, comment) in make_requests(runtime)
        .into_iter()
        .filter(|(asked, _)| asked == media)
    {
        let patch = CommentPatch {
            resolved: Some(true),
            ..Default::default()
        };
        let _ = runtime.set_comment("ui", &comment.id, patch);
    }
}
