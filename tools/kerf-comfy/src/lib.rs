//! Asks a ComfyUI server for a shot.
//!
//! Kerf has no model of its own: a `generate` step writes what is wanted
//! as a request and runs the program a person's `~/.kerf/models.json` names
//! for that model. This is such a program, for a model kept
//! on a ComfyUI server:
//!
//! ```json
//! { "shot": { "run": ["kerf-comfy", "--workflow", "shot", "{request}"] } }
//! ```
//!
//! It fits the guide to the model ([`guide`]), sends up the files the
//! request gives, fills the workflow's blanks with what the request asks
//! ([`workflow`]), queues it, waits, and writes what the server made where
//! the request says ([`server`]). What it sent is kept beside the request:
//! the guide as the model was given it, and the graph with its blanks
//! filled.
//!
//! Asked to end while it waits, as Kerf asks a making that is cancelled
//! (SIGTERM, or a terminal's Ctrl-C), it takes its job back from the server
//! before it does, so nothing goes on being made that nobody will fetch.

pub mod guide;
pub mod progress;
pub mod request;
pub mod server;

// A workflow's file, and how an editor shows one, are read by more than this
// program (`crates/surfaces/kerf-workflow`): they are named here as they were.
pub use kerf_workflow::{detail, workflow};

use guide::{Shape, Tools};
use request::{names_what_is_handed, signal_named, Request};
use serde_json::json;
use server::{Server, State};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use workflow::{value_of, Values, Workflow};

/// The blanks the request itself fills.
const ASKED: [&str; 6] = ["prompt", "seed", "width", "height", "frames", "fps"];

/// The guide as it went up, and the graph as it was queued, beside the
/// request.
const GUIDE_SENT: &str = "guide.comfy.mp4";
const GRAPH_SENT: &str = "workflow.comfy.json";
/// The blank a small black picture on the server fills, for a node that will not run
/// without pictures it does not read.
const NO_PICTURE: &str = "no_picture";

/// How often a server that does not answer, or does not know the job, is
/// asked again before that is taken as its answer.
const TRIES: u32 = 5;

/// How long a job is waited for, and how often it is asked after.
#[derive(Clone, Copy, Debug)]
pub struct Patience {
    pub most: Duration,
    pub pause: Duration,
    /// Whether to listen for which step a job is on, where the server says it
    /// ([`progress`]). `COMFY_PROGRESS=off` does without.
    pub listen: bool,
}

impl Patience {
    /// Half an hour, or the seconds `COMFY_TIMEOUT` names, asking every two
    /// seconds.
    pub fn load() -> Self {
        let most = std::env::var("COMFY_TIMEOUT")
            .ok()
            .and_then(|seconds| seconds.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .unwrap_or(30 * 60);
        Self {
            most: Duration::from_secs(most),
            pause: Duration::from_secs(2),
            listen: std::env::var("COMFY_PROGRESS").map_or(true, |listen| listen.trim() != "off"),
        }
    }
}

/// Sixteen hex digits of the SHA-256 of `bytes`: a name for a file by what
/// it holds.
pub(crate) fn hash(bytes: &[u8]) -> String {
    hex::encode(&Sha256::digest(bytes)[..8])
}

/// A file's extension, in small letters.
fn extension(file: &Path) -> Option<String> {
    file.extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
}

/// Which of a `generate` line's names set the inputs a failed node's message
/// names, as " (on the line: confidence_threshold is confidence=, …)", or nothing:
/// the server names its node's inputs, which no line can set by those names.
fn on_the_line(workflow: &Workflow, why: &str) -> String {
    let Some(node) = why
        .split_once("(node ")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(node, _)| node)
    else {
        return String::new();
    };
    let names: Vec<String> = workflow
        .line_names(node, why)
        .into_iter()
        .map(|(input, name)| format!("{input} is {name}="))
        .collect();
    if names.is_empty() {
        String::new()
    } else {
        format!(" (on the generate line, {})", names.join(", "))
    }
}

/// A seed for a request that names none.
fn any_seed() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    // What a number in JSON holds exactly wherever it is read.
    (now % (1 << 48)) as u64
}

/// How often a wait looks whether this program was asked to end.
const STOP_CHECK: Duration = Duration::from_millis(50);

/// Wait `pause`, or until `stop` is set: whether it was.
fn stopped_within(stop: &AtomicBool, pause: Duration) -> bool {
    let until = Instant::now() + pause;
    loop {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        std::thread::sleep(left.min(STOP_CHECK));
    }
}

/// Make what `request` asks with `workflow` on `server`, writing the file
/// the request names. `work` is the folder the request is in, where what is
/// sent is kept. Each thing worth telling goes to `say` as a line. `stop`
/// is set when this program is asked to end: a job queued by then is taken
/// back from the server, and nothing is queued after.
// Each argument is a distinct thing one run needs, and none of them belong together.
#[allow(clippy::too_many_arguments)]
pub fn make(
    request: &Request,
    work: &Path,
    workflow: &Workflow,
    server: &Server,
    tools: &Tools,
    patience: &Patience,
    stop: &AtomicBool,
    say: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let blanks = workflow.blanks();
    // A workflow that makes a file of its guide goes by the guide: its rate, its size, its frames.
    let read = if workflow.of_guide() {
        let guide = request.guide.as_deref().ok_or(
            "this workflow makes a file of a guide, and the line gives none: `guide=<file>`",
        )?;
        Some(tools.picture(guide)?)
    } else {
        None
    };
    if workflow.of_file() && request.references.is_empty() {
        return Err(
            "this workflow makes a file of a file, and the line gives none: `ref=<file>`"
                .to_owned(),
        );
    }
    let fps = read.map_or(workflow.fps, |read| read.fps);
    let (width, height) = workflow.size(request.size.or(read.map(|read| read.size)));
    if let Some((asked_width, asked_height)) =
        request.size.filter(|asked| *asked != (width, height))
    {
        say(&format!(
            "{asked_width}x{asked_height} was asked; this model makes {}x{} or as much, so it is asked for {width}x{height}",
            workflow.size.0, workflow.size.1
        ));
    }

    // How long the shot is: as asked, or as long as its guide.
    let guide = request
        .guide
        .as_deref()
        .filter(|_| blanks.contains("guide"));
    let guide_frames = match guide {
        Some(guide) => Some((tools.seconds(guide)? * fps).round() as u32),
        None => None,
    };
    let frames = match read {
        Some(read) => workflow.frames_of(read.frames, request.seconds, fps),
        None => workflow.frames(
            request.seconds,
            guide_frames.map(|frames| f64::from(frames) / fps),
        ),
    };
    if read.is_some() && frames < workflow.frames.least {
        return Err(format!(
            "this workflow reads at least {} frames of its guide together, and is handed {frames}",
            workflow.frames.least
        ));
    }
    if let Some(read) = read.filter(|read| read.frames > frames && request.seconds.is_none()) {
        say(&format!(
            "the guide is {} frames, and this workflow reads {frames} at once: the rest is left out",
            read.frames
        ));
    }
    if let Some(seconds) = request.seconds.filter(|_| read.is_none()) {
        if (seconds * fps).round() > f64::from(frames) {
            say(&format!(
                "{seconds} s was asked; this model makes at most {frames} frames at {fps} a second, {:.2} s",
                f64::from(frames) / fps
            ));
        }
    }
    // A file of a guide has the guide's frames and no more.
    let hold = if read.is_some() {
        0
    } else {
        workflow.frames.step
    };
    if let Some(guide_frames) =
        guide_frames.filter(|guide_frames| read.is_none() && guide_frames + hold < frames)
    {
        say(&format!(
            "the guide is {guide_frames} frames at {fps} a second, and {frames} are asked for: past its end the model has nothing to follow"
        ));
    }

    // What fills the blanks: the workflow's own values, then the line's, then what was asked.
    let mut values: Values = workflow.set.clone();
    let mut unread = Vec::new();
    for (name, text) in &request.more {
        // Points are the line's own to give, where the workflow has a blank for them.
        let points = request::names_points(name) && blanks.contains(name);
        if !points
            && (ASKED.contains(&name.as_str())
                || names_what_is_handed(name)
                || !blanks.contains(name))
        {
            unread.push(format!("`{name}`"));
        } else {
            values.insert(name.clone(), value_of(name, text, workflow.set.get(name))?);
        }
    }
    // Points on a thing of the guide's first frame, and off it, as the pixels a node reads
    // them in. With none given, the nodes that read them are left out.
    for name in [request::INSIDE, request::OUTSIDE] {
        let given = match values.remove(name) {
            Some(serde_json::Value::String(given)) if !given.trim().is_empty() => given,
            _ => continue,
        };
        values.insert(
            name.to_owned(),
            json!(request::points(name, &given, (width, height))?),
        );
    }
    let seed = request.seed.unwrap_or_else(any_seed);
    values.extend([
        ("prompt".to_owned(), json!(request.prompt)),
        ("seed".to_owned(), json!(seed)),
        ("width".to_owned(), json!(width)),
        ("height".to_owned(), json!(height)),
        ("frames".to_owned(), json!(frames)),
        ("fps".to_owned(), json!(fps)),
    ]);

    if blanks.contains(NO_PICTURE) {
        let file = work.join("no_picture.comfy.png");
        tools.black(&file)?;
        let bytes = std::fs::read(&file)
            .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
        let sent = server.upload(&file, &format!("kerf-{}.png", hash(&bytes)))?;
        values.insert(NO_PICTURE.to_owned(), json!(sent));
    }

    // The files the workflow reads go up, each named by what it holds.
    for name in request.files() {
        if !blanks.contains(&name) {
            unread.push(format!("`{name}`"));
            continue;
        }
        // `files` names each of the request's `guides` as `guide_` and its key, and a key
        // `guide_named` does not read back (a capital, a `_`) gives no file under that name.
        let Some(given) = request.file(&name) else {
            return Err(format!(
                "the request hands a guide as `{name}`, and a guide's name after `guide_` is \
                 lowercase letters and digits"
            ));
        };
        // Every guide is brought to the size, rate, and frames asked for, each to a file
        // of its own.
        let fitted = match request::guide_named(&name) {
            Some(of) => work.join(format!("guide_{of}.comfy.mp4")),
            None => work.join(GUIDE_SENT),
        };
        let file = if name == "guide" || request::guide_named(&name).is_some() {
            let shape = Shape {
                size: (width, height),
                fps,
                frames,
                hold,
            };
            tools.conform(given, &fitted, &shape)?;
            fitted.as_path()
        } else {
            given
        };
        let bytes = std::fs::read(file)
            .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
        let named = format!(
            "kerf-{}.{}",
            hash(&bytes),
            extension(file).unwrap_or_else(|| "bin".to_owned())
        );
        let sent = server.upload(file, &named)?;
        say(&format!(
            "sent the {name}, {} kB",
            bytes.len().div_ceil(1024)
        ));
        values.insert(name, json!(sent));
    }
    // The signals the workflow reads, each as a number for every frame asked for.
    for (index, signal) in request.signals.iter().enumerate() {
        let name = signal_named(index);
        let as_video = request::signal_video_named(index);
        if !blanks.contains(&name) && !blanks.contains(&as_video) {
            unread.push(format!("`{name}`"));
            continue;
        }
        if blanks.contains(&as_video) {
            // As a grey video of the frames asked for, for a node that reads a mask a frame.
            let file = work.join(format!("{as_video}.comfy.mp4"));
            tools.levels(&signal.over(frames, fps), (width, height), fps, &file)?;
            let bytes = std::fs::read(&file)
                .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
            let sent = server.upload(&file, &format!("kerf-{}.mp4", hash(&bytes)))?;
            values.insert(as_video, json!(sent));
            say(&format!(
                "handed signal `{}` as a video of {frames} frames",
                signal.name
            ));
        }
        if blanks.contains(&name) {
            values.insert(name, json!(signal.text(frames, fps)));
            say(&format!(
                "handed signal `{}` as {frames} values",
                signal.name
            ));
        }
    }
    // Where clips go, as one list of tracks in the pixels and frames asked for.
    if !request.moves.is_empty() && blanks.contains(request::MOVES_VIDEO) {
        // As a layout a model follows: white, with each clip's box where it is on each frame.
        let file = work.join("moves_video.comfy.mp4");
        let drawn = request::layout(&request.moves, frames, fps, (width, height));
        tools.greys(&drawn, (width, height), fps, &file)?;
        let bytes = std::fs::read(&file)
            .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
        let sent = server.upload(&file, &format!("kerf-{}.mp4", hash(&bytes)))?;
        values.insert(request::MOVES_VIDEO.to_owned(), json!(sent));
        say(&format!(
            "handed where {} goes as a layout of {frames} frames",
            request
                .moves
                .iter()
                .map(|clip| format!("`{}`", clip.name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else if !request.moves.is_empty() && blanks.contains(request::MOVES_BOX) {
        values.insert(
            request::MOVES_BOX.to_owned(),
            json!(request::tracks_of_boxes(
                &request.moves,
                frames,
                fps,
                (width, height)
            )),
        );
        say(&format!(
            "handed where {} goes as five tracks of {frames} points each",
            request
                .moves
                .iter()
                .map(|clip| format!("`{}`", clip.name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else if !request.moves.is_empty() {
        if blanks.contains(request::MOVES) {
            values.insert(
                request::MOVES.to_owned(),
                json!(request::tracks(
                    &request.moves,
                    frames,
                    fps,
                    (width, height)
                )),
            );
            say(&format!(
                "handed where {} goes as {frames} points each",
                request
                    .moves
                    .iter()
                    .map(|clip| format!("`{}`", clip.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        } else {
            unread.push(format!("`{}`", request::MOVES));
        }
    }
    if !unread.is_empty() {
        say(&format!(
            "this workflow reads no {}: left out",
            unread.join(", ")
        ));
    }

    let graph = workflow.graph(&values, &mut |class| server.required(class))?;
    // With what the line does not give left out, and all that cannot do without it, a
    // workflow can have nothing left that saves a file: said here, with what it wants,
    // where the server would say only that the graph has no end.
    if !workflow.ends().iter().any(|id| graph.get(id).is_some()) {
        let wanted = workflow.not_given(&values);
        return Err(if wanted.is_empty() {
            "nothing of this workflow is left to save a file".to_owned()
        } else {
            format!(
                "this workflow makes nothing without {}, which the line does not give",
                wanted
                    .iter()
                    .map(|name| format!("`{name}=`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
    }
    let kept = serde_json::to_vec_pretty(&graph).map_err(|error| error.to_string())?;
    std::fs::write(work.join(GRAPH_SENT), kept)
        .map_err(|error| format!("cannot write into {}: {error}", work.display()))?;
    if stop.load(Ordering::SeqCst) {
        return Err("asked to stop before the server was asked for anything".to_owned());
    }
    // Listening before the job is queued, so that its first step is heard too.
    let progress = patience.listen.then(|| server.progress()).flatten();
    let id = server.queue(&graph)?;
    // Not where the server is: what is said here reaches whoever asked for the making, an agent
    // among them, and the server is the person's to name.
    if workflow.of_file() {
        say(&format!(
            "asked the server for a file of the file: job {id}"
        ));
    } else if frames == 1 && workflow.frames.most == 1 {
        say(&format!(
            "asked the server for a still of {width}x{height}, seed {seed}: job {id}"
        ));
    } else {
        say(&format!(
            "asked the server for {frames} frames of {width}x{height} at {fps} a second, seed {seed}: job {id}"
        ));
    }

    let started = Instant::now();
    let mut told = (String::new(), Instant::now());
    let (mut unanswered, mut unknown) = (0, 0);
    let made = loop {
        if stopped_within(stop, patience.pause) {
            server.cancel(&id);
            // Said once, as the program's last line, like any end that is not a take.
            return Err(format!("asked to stop: took job {id} back from the server"));
        }
        let waited = started.elapsed();
        if waited > patience.most {
            server.cancel(&id);
            return Err(format!(
                "gave the job up after {} s and took it back; COMFY_TIMEOUT sets how many seconds to wait",
                waited.as_secs()
            ));
        }
        let now = match server.state(&id) {
            // A line that drops for a moment is not the job's end.
            Err(error) if unanswered < TRIES => {
                unanswered += 1;
                say(&format!("{error}; asking again"));
                continue;
            }
            Err(error) => return Err(error),
            Ok(State::Made(files)) => break files,
            Ok(State::Failed(why)) => {
                return Err(format!(
                    "the server could not make it: {why}{}",
                    on_the_line(workflow, &why)
                ))
            }
            // Between leaving the queue and being kept, a job is in neither.
            Ok(State::Unknown) if unknown < TRIES => {
                unknown += 1;
                continue;
            }
            Ok(State::Unknown) => {
                return Err(format!(
                    "the server no longer knows job {id}: it was restarted, or its history cleared"
                ))
            }
            Ok(State::Waiting(ahead)) => format!("waiting, {ahead} ahead"),
            // Which step of how many, where the server says: else only that it runs.
            Ok(State::Running) => match progress.as_ref().and_then(|progress| progress.step(&id)) {
                Some((step, of)) => format!("running, step {step} of {of}"),
                None => "running".to_owned(),
            },
        };
        (unanswered, unknown) = (0, 0);
        if now != told.0 || told.1.elapsed() >= Duration::from_secs(20) {
            say(&format!("{now}, {} s", waited.as_secs()));
            told = (now, Instant::now());
        }
    };

    // The shot's file is of the kind its name says, so what is fetched is too.
    let wanted = extension(&request.out);
    let chosen = made
        .iter()
        .find(|file| wanted.is_none() || extension(Path::new(&file.filename)) == wanted)
        .ok_or_else(|| {
            if made.is_empty() {
                "the job ended and saved no file: the workflow needs a node that saves one"
                    .to_owned()
            } else {
                format!(
                    "the job made {}, and the file asked for is a .{}",
                    made.iter()
                        .map(|file| file.filename.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    wanted.as_deref().unwrap_or_default()
                )
            }
        })?;
    server.fetch(chosen, &request.out)?;
    say(&format!("made in {} s", started.elapsed().as_secs()));
    Ok(())
}

#[cfg(test)]
mod tests;
