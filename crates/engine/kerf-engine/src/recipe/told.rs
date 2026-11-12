//! A making under way, written where another program can see it.
//!
//! A making runs in the program that was asked for it: the editor, `kerf
//! make`, or an agent's own `kerf-mcp`. Each of them says what it is doing
//! in a file of the project, `.kerf/cache/makings/<media>.json`: the take
//! it makes, the line its steps last said, and how it ended. It says so
//! again every half second while it goes, so that one whose program has
//! gone is told from one that is still at it. Whoever shows makings, the
//! editor, reads the files ([`told`]) and shows another program's making
//! as it shows its own; and it can ask one to stop ([`ask_to_stop`]),
//! which the making sees the next time it says it is going.
//!
//! The files are nobody's to keep: git ignores the folder, one that ended
//! goes when the media is next made or ten minutes on, and a making that
//! cannot write its file is made all the same.

use crate::error::EngineError;
use crate::sidecar::CancellationToken;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where a project's makings say what they are doing.
const FOLDER: &str = ".kerf/cache/makings";
/// How often a making says it is still going, and looks for a word to stop.
const BEAT: Duration = Duration::from_millis(500);
/// How long a making may say nothing before its program is taken for gone.
const GONE_AFTER: Duration = Duration::from_secs(5);
/// How long what a making ended as is kept for those who were watching it.
const KEPT: Duration = Duration::from_secs(600);

/// How a making is going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Going {
    Making,
    Made,
    Stopped,
    Failed,
}

/// What a making says of itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Told {
    /// The media it makes, by its id.
    pub media: String,
    pub take: u32,
    /// The program making it.
    pub pid: u32,
    pub going: Going,
    /// The line its steps last said.
    #[serde(default)]
    pub line: String,
    /// Why it failed, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// When it last said so, in milliseconds since 1970.
    pub at_ms: u64,
}

impl Told {
    /// Whether it says it is being made and has not said so for too long:
    /// the program making it has gone without saying how it ended.
    pub fn gone(&self) -> bool {
        self.going == Going::Making && now_ms().saturating_sub(self.at_ms) > GONE_AFTER.as_millis() as u64
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// A media's id as a file name: ids are letters, digits, `_` and `-`, and
/// anything else a document got past that is not let near a path.
fn named(media: &str) -> String {
    media
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

fn file_of(directory: &Path, media: &str) -> PathBuf {
    directory.join(FOLDER).join(format!("{}.json", named(media)))
}

fn stop_of(directory: &Path, media: &str) -> PathBuf {
    directory.join(FOLDER).join(format!("{}.stop", named(media)))
}

/// Write `told` whole or not at all. A making goes on whether or not it can.
fn write(file: &Path, told: &Told) {
    let Some(folder) = file.parent() else {
        return;
    };
    if std::fs::create_dir_all(folder).is_err() {
        return;
    }
    let Ok(text) = serde_json::to_vec(told) else {
        return;
    };
    let partial = file.with_extension(format!("{}.partial", std::process::id()));
    if std::fs::write(&partial, text).is_err() || std::fs::rename(&partial, file).is_err() {
        let _ = std::fs::remove_file(&partial);
    }
}

/// A making saying what it is doing, from its start to its end.
pub(crate) struct Telling {
    file: PathBuf,
    told: Arc<Mutex<Told>>,
    ended: Arc<AtomicBool>,
    beat: Option<JoinHandle<()>>,
}

impl Telling {
    /// Begin to say that take `take` of `media` is being made in the project
    /// in `directory`, and to listen for a word to stop, which cancels
    /// `cancellation`.
    pub(crate) fn begin(
        directory: &Path,
        media: &str,
        take: u32,
        cancellation: &CancellationToken,
    ) -> Self {
        let file = file_of(directory, media);
        let stop = stop_of(directory, media);
        // A word to stop an earlier making is not one to stop this.
        let _ = std::fs::remove_file(&stop);
        forget_old(directory);
        let told = Arc::new(Mutex::new(Told {
            media: media.to_owned(),
            take,
            pid: std::process::id(),
            going: Going::Making,
            line: String::new(),
            why: None,
            at_ms: now_ms(),
        }));
        write(&file, &told.lock().unwrap_or_else(PoisonError::into_inner));
        let ended = Arc::new(AtomicBool::new(false));
        let beat = {
            let (file, told, ended, cancellation) =
                (file.clone(), told.clone(), ended.clone(), cancellation.clone());
            thread::spawn(move || {
                let step = Duration::from_millis(50);
                let mut waited = Duration::ZERO;
                while !ended.load(Ordering::SeqCst) {
                    thread::sleep(step);
                    waited += step;
                    if waited < BEAT {
                        continue;
                    }
                    waited = Duration::ZERO;
                    if std::fs::remove_file(&stop).is_ok() {
                        cancellation.cancel();
                    }
                    let mut told = told.lock().unwrap_or_else(PoisonError::into_inner);
                    // Its end is written by whoever ends it, and not over.
                    if ended.load(Ordering::SeqCst) {
                        break;
                    }
                    told.at_ms = now_ms();
                    write(&file, &told);
                }
            })
        };
        Self {
            file,
            told,
            ended,
            beat: Some(beat),
        }
    }

    /// A step said `line`.
    pub(crate) fn say(&self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let mut told = self.told.lock().unwrap_or_else(PoisonError::into_inner);
        if told.line != line {
            told.line = line.to_owned();
            told.at_ms = now_ms();
            write(&self.file, &told);
        }
    }

    /// The making is over: `Ok` with the take made, or why not.
    pub(crate) fn end(mut self, made: Result<u32, &EngineError>) {
        let (going, why) = match made {
            Ok(_) => (Going::Made, None),
            Err(EngineError::Cancelled) => (Going::Stopped, None),
            Err(error) => (Going::Failed, Some(error.to_string())),
        };
        self.finish(going, made.ok(), why);
    }

    fn finish(&mut self, going: Going, take: Option<u32>, why: Option<String>) {
        let mut told = self.told.lock().unwrap_or_else(PoisonError::into_inner);
        self.ended.store(true, Ordering::SeqCst);
        told.going = going;
        told.why = why;
        if let Some(take) = take {
            told.take = take;
        }
        told.at_ms = now_ms();
        write(&self.file, &told);
        drop(told);
        if let Some(beat) = self.beat.take() {
            let _ = beat.join();
        }
    }
}

impl Drop for Telling {
    fn drop(&mut self) {
        // One let go of without a word ended somehow other than well.
        if !self.ended.load(Ordering::SeqCst) {
            self.finish(Going::Failed, None, Some("the making ended without saying how".into()));
        }
    }
}

/// Take away what makings that ended long ago said.
fn forget_old(directory: &Path) {
    for told in told(directory) {
        let old = now_ms().saturating_sub(told.at_ms) > KEPT.as_millis() as u64;
        if old && (told.going != Going::Making || told.gone()) {
            let _ = std::fs::remove_file(file_of(directory, &told.media));
        }
    }
}

/// What every making of the project in `directory` last said of itself:
/// those under way, in any program, and those that ended a while ago.
pub fn told(directory: &Path) -> Vec<Told> {
    let Ok(entries) = std::fs::read_dir(directory.join(FOLDER)) else {
        return Vec::new();
    };
    let mut all: Vec<Told> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .filter_map(|path| serde_json::from_slice(&std::fs::read(path).ok()?).ok())
        .collect();
    all.sort_by(|a, b| a.media.cmp(&b.media));
    all
}

/// Ask the making of `media` in the project in `directory` to stop,
/// whichever program is making it. `false` when none says it is being made.
pub fn ask_to_stop(directory: &Path, media: &str) -> bool {
    let making = told(directory)
        .iter()
        .any(|told| told.media == media && told.going == Going::Making && !told.gone());
    making && std::fs::write(stop_of(directory, media), b"").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !check() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_making_says_what_it_is_doing_and_how_it_ended() {
        let dir = tempfile::tempdir().unwrap();
        assert!(told(dir.path()).is_empty());
        let cancellation = CancellationToken::new();
        let telling = Telling::begin(dir.path(), "m1", 3, &cancellation);
        let said = told(dir.path());
        assert_eq!(said.len(), 1);
        assert_eq!(
            (said[0].media.as_str(), said[0].take, said[0].pid, said[0].going),
            ("m1", 3, std::process::id(), Going::Making)
        );
        assert!(!said[0].gone());
        telling.say("  step 2 of 5\n");
        telling.say("");
        assert_eq!(told(dir.path())[0].line, "step 2 of 5");
        // It goes on saying it is at it while its steps say nothing.
        let first = told(dir.path())[0].at_ms;
        wait_for("it says it is still going", || told(dir.path())[0].at_ms > first);
        telling.end(Ok(4));
        let said = told(dir.path());
        assert_eq!((said[0].going, said[0].take), (Going::Made, 4));
        // And says nothing after its end.
        thread::sleep(BEAT + Duration::from_millis(200));
        assert_eq!(told(dir.path()), said);

        // One that fails says why, and one that was stopped that it was.
        let failed = Telling::begin(dir.path(), "m2", 1, &cancellation);
        failed.end(Err(&EngineError::msg("the model would not start")));
        let stopped = Telling::begin(dir.path(), "m3", 1, &cancellation);
        stopped.end(Err(&EngineError::Cancelled));
        let said = told(dir.path());
        assert_eq!(said[1].going, Going::Failed);
        assert_eq!(said[1].why.as_deref(), Some("the model would not start"));
        assert_eq!((said[2].going, said[2].why.clone()), (Going::Stopped, None));
        // One dropped on the way out of an error did not end well.
        drop(Telling::begin(dir.path(), "m3", 2, &cancellation));
        assert_eq!(told(dir.path())[2].going, Going::Failed);
    }

    #[test]
    fn a_word_to_stop_reaches_the_making_and_only_one_under_way() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!ask_to_stop(dir.path(), "m1"), "nothing is being made");
        let cancellation = CancellationToken::new();
        let telling = Telling::begin(dir.path(), "m1", 1, &cancellation);
        assert!(!ask_to_stop(dir.path(), "m2"), "another media is not");
        assert!(ask_to_stop(dir.path(), "m1"));
        wait_for("the making hears it", || cancellation.is_cancelled());
        telling.end(Err(&EngineError::Cancelled));
        assert!(!ask_to_stop(dir.path(), "m1"), "it has ended");

        // A word left from before does not stop the next making.
        std::fs::write(stop_of(dir.path(), "m1"), b"").unwrap();
        let again = CancellationToken::new();
        let telling = Telling::begin(dir.path(), "m1", 2, &again);
        thread::sleep(BEAT + Duration::from_millis(200));
        assert!(!again.is_cancelled());
        telling.end(Ok(2));
    }

    #[test]
    fn a_making_whose_program_has_gone_is_told_from_one_still_at_it() {
        let dir = tempfile::tempdir().unwrap();
        let old = Told {
            media: "m1".into(),
            take: 1,
            pid: 1,
            going: Going::Making,
            line: "half way".into(),
            why: None,
            at_ms: now_ms() - GONE_AFTER.as_millis() as u64 - 1000,
        };
        write(&file_of(dir.path(), "m1"), &old);
        assert!(told(dir.path())[0].gone());
        assert!(!ask_to_stop(dir.path(), "m1"));
        // What ended long ago goes when something is next made.
        let long_ago = Told {
            media: "m2".into(),
            going: Going::Made,
            at_ms: now_ms() - KEPT.as_millis() as u64 - 1000,
            ..old.clone()
        };
        write(&file_of(dir.path(), "m2"), &long_ago);
        Telling::begin(dir.path(), "m3", 1, &CancellationToken::new()).end(Ok(1));
        let media: Vec<String> = told(dir.path()).into_iter().map(|told| told.media).collect();
        assert_eq!(media, ["m1", "m3"]);
        // A file that is not one of these is not read as one.
        std::fs::write(dir.path().join(FOLDER).join("junk.json"), b"{").unwrap();
        assert_eq!(told(dir.path()).len(), 2);
    }
}
