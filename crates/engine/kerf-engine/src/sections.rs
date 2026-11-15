//! A render's picture, encoded a stretch at a time and kept by what each
//! stretch's frames depend on, so that rendering again after an edit draws
//! and encodes only the stretches the edit touched and joins the rest as
//! they are.
//!
//! Frames kept on disk (`kerf_compose::disk`) spare the drawing and not the
//! encoding, which is most of a render of plain footage, and at four
//! megabytes a frame of 1080p they hold a minute and a half of it. A
//! stretch of H.264 is a thousandth of that, so a whole program stays.
//!
//! Where one stretch ends and the next begins is decided by the frames
//! themselves: by the numbers that say what each depends on
//! (`Compositor::frame_key`), which do not change when a frame moves along
//! the timeline. So a cut near the start of a program moves the stretches
//! after it and leaves them the same stretches.

use sha2::{Digest, Sha256};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// The shortest stretch, in seconds: each is a file and an encoder of its
/// own, which cost more than they save under this.
const LEAST_SECONDS: f64 = 2.0;
/// Past the shortest, a stretch ends at one frame in this many seconds'
/// worth, so a stretch is about four seconds.
const EVERY_SECONDS: f64 = 2.0;
/// The longest stretch, in seconds: an edit to one frame of it draws and
/// encodes all of it again.
const MOST_SECONDS: f64 = 10.0;
/// How long a folder of another Kerf's stretches stays, unread, and a file
/// an encoder was stopped before finishing.
const STALE: Duration = Duration::from_secs(3 * 24 * 3600);
const DEFAULT_GIGABYTES: u64 = 10;

/// A stretch of a render's frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stretch {
    pub frames: Range<usize>,
    /// Whether every frame of it has a number for what it depends on. One
    /// that reads more than the document says, a LUT's file or another
    /// track, has none, and its stretch is encoded each time.
    pub keyed: bool,
}

/// How many frames the shortest stretch holds at `rate` frames a second. A
/// render with fewer is encoded whole.
pub(crate) fn least_frames(rate: f64) -> usize {
    ((LEAST_SECONDS * rate).round() as usize).max(LEAST_FRAMES)
}

/// No stretch is shorter than this many frames, however slow the rate. An
/// encoder that puts frames out of order holds a few back before the first
/// leaves it, and gives each file's frames times that allow for that; a file
/// of fewer frames than it holds back is timed as if it held none, and does
/// not join.
const LEAST_FRAMES: usize = 8;

/// The frames of a render as stretches, in order, where `keys` has each
/// frame's number and the render draws `rate` frames a second.
pub(crate) fn stretches(keys: &[Option<u64>], rate: f64) -> Vec<Stretch> {
    let frames = |seconds: f64| ((seconds * rate).round() as usize).max(1);
    let (least, every, most) = (
        least_frames(rate),
        frames(EVERY_SECONDS) as u64,
        frames(MOST_SECONDS).max(least_frames(rate)),
    );
    let mut stretches: Vec<Stretch> = Vec::new();
    let mut start = 0;
    for at in 1..=keys.len() {
        let length = at - start;
        let ends = match keys.get(at) {
            None => true,
            Some(next) => {
                next.is_some() != keys[at - 1].is_some()
                    || length >= most
                    // The frame decides, not where it falls, so that the same
                    // frames end a stretch wherever an edit has moved them.
                    || (length >= least && next.is_some_and(|key| key % every == 0))
            }
        };
        if ends {
            let keyed = keys[start].is_some();
            match stretches.last_mut() {
                // What is left at the end, or a few frames with no number among ones that have
                // one, is too short to be a file of its own: it goes with the stretch before
                // it, which is then kept only if both would be.
                Some(before) if length < least => {
                    before.frames.end = at;
                    before.keyed &= keyed;
                }
                _ => stretches.push(Stretch {
                    frames: start..at,
                    keyed,
                }),
            }
            start = at;
        }
    }
    // A short one at the very start goes with the one after it.
    if stretches.len() > 1 && stretches[0].frames.len() < least {
        let first = stretches.remove(0);
        stretches[0].frames.start = first.frames.start;
        stretches[0].keyed &= first.keyed;
    }
    stretches
}

/// The name a stretch is kept by: everything its file depends on. `encoded`
/// says how it is encoded, the same for every stretch of a render, and
/// `keys` what each of its frames is.
pub(crate) fn name(encoded: &[u8], keys: &[Option<u64>]) -> String {
    let mut hash = Sha256::new();
    hash.update((encoded.len() as u64).to_le_bytes());
    hash.update(encoded);
    for key in keys.iter().flatten() {
        hash.update(key.to_le_bytes());
    }
    hex::encode(&hash.finalize()[..16])
}

/// What renders have done with their stretches since this process started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SectionStats {
    /// Stretches joined as they were kept, and the frames in them.
    pub kept: u64,
    pub kept_frames: u64,
    /// Stretches drawn and encoded, and the frames in them.
    pub encoded: u64,
    pub encoded_frames: u64,
    /// Renders whose sound was copied as it was kept, and renders that mixed and encoded it.
    pub sound_kept: u64,
    pub sound_mixed: u64,
    /// Renders that gave up on stretches and encoded the picture whole.
    pub whole: u64,
}

static KEPT: AtomicU64 = AtomicU64::new(0);
static KEPT_FRAMES: AtomicU64 = AtomicU64::new(0);
static ENCODED: AtomicU64 = AtomicU64::new(0);
static ENCODED_FRAMES: AtomicU64 = AtomicU64::new(0);
static SOUND_KEPT: AtomicU64 = AtomicU64::new(0);
static SOUND_MIXED: AtomicU64 = AtomicU64::new(0);
static WHOLE: AtomicU64 = AtomicU64::new(0);

/// What renders in this process have done with their stretches.
pub fn section_stats() -> SectionStats {
    SectionStats {
        kept: KEPT.load(Ordering::Relaxed),
        kept_frames: KEPT_FRAMES.load(Ordering::Relaxed),
        encoded: ENCODED.load(Ordering::Relaxed),
        encoded_frames: ENCODED_FRAMES.load(Ordering::Relaxed),
        sound_kept: SOUND_KEPT.load(Ordering::Relaxed),
        sound_mixed: SOUND_MIXED.load(Ordering::Relaxed),
        whole: WHOLE.load(Ordering::Relaxed),
    }
}

pub(crate) fn count_sound(kept: bool) {
    if kept { &SOUND_KEPT } else { &SOUND_MIXED }.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn count_kept(frames: usize) {
    KEPT.fetch_add(1, Ordering::Relaxed);
    KEPT_FRAMES.fetch_add(frames as u64, Ordering::Relaxed);
}

pub(crate) fn count_encoded(frames: usize) {
    ENCODED.fetch_add(1, Ordering::Relaxed);
    ENCODED_FRAMES.fetch_add(frames as u64, Ordering::Relaxed);
}

pub(crate) fn count_whole() {
    WHOLE.fetch_add(1, Ordering::Relaxed);
}

/// The folder stretches are kept in: the user's cache folder, or
/// `KERF_SECTION_CACHE`, which `off` turns into none.
fn default_folder() -> Option<PathBuf> {
    match std::env::var_os("KERF_SECTION_CACHE") {
        Some(value) if value == "off" || value == "0" || value.is_empty() => return None,
        Some(value) => return Some(PathBuf::from(value)),
        None => {}
    }
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        var("LOCALAPPDATA").map(|folder| folder.join("Kerf").join("sections"))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Caches/Kerf/sections"))
    } else {
        var("XDG_CACHE_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".cache")))
            .map(|folder| folder.join("kerf").join("sections"))
    }
}

/// A folder of kept stretches: `<root>/<stamp>/<2 hex>/<32 hex>.<extension>`.
/// The stamp is the code that draws, so a new Kerf never joins what an older
/// one drew. A file is there whole or not at all: an encoder writes under
/// another name, in `<root>/<stamp>/making`, and the file is renamed when the
/// encoder ends well. Past a size limit the least recently joined go.
pub(crate) struct Kept {
    root: PathBuf,
    folder: PathBuf,
    budget: u64,
}

/// A file an encoder is writing, unlike any other's.
static MAKING: AtomicU64 = AtomicU64::new(0);

impl Kept {
    /// The user's folder, unless it is turned off or cannot be made.
    pub(crate) fn open() -> Option<Self> {
        let gigabytes = std::env::var("KERF_SECTION_CACHE_GB")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|gigabytes| *gigabytes > 0.0)
            .unwrap_or(DEFAULT_GIGABYTES as f64);
        Self::at(
            &default_folder()?,
            kerf_compose::DRAW_STAMP,
            (gigabytes * (1u64 << 30) as f64) as u64,
        )
    }

    fn at(root: &Path, stamp: &str, budget: u64) -> Option<Self> {
        let folder = root.join(stamp);
        std::fs::create_dir_all(folder.join("making")).ok()?;
        Some(Self {
            root: root.to_owned(),
            folder,
            budget,
        })
    }

    fn path(&self, name: &str, extension: &str) -> PathBuf {
        self.folder
            .join(&name[..2])
            .join(format!("{name}.{extension}"))
    }

    /// The stretch kept as `name`, if it is, marked as just read.
    pub(crate) fn find(&self, name: &str, extension: &str) -> Option<PathBuf> {
        let path = self.path(name, extension);
        let file = std::fs::File::options().append(true).open(&path).ok()?;
        let _ = file.set_modified(SystemTime::now());
        Some(path)
    }

    /// A file for an encoder to write, with `extension`.
    pub(crate) fn making(&self, extension: &str) -> PathBuf {
        self.folder.join("making").join(format!(
            "{}-{}.{extension}",
            std::process::id(),
            MAKING.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// Keep what an encoder wrote to `made` as `name`.
    pub(crate) fn keep(&self, made: &Path, name: &str, extension: &str) -> std::io::Result<PathBuf> {
        let path = self.path(name, extension);
        std::fs::create_dir_all(path.parent().expect("a kept file is in a folder"))?;
        std::fs::rename(made, &path)?;
        Ok(path)
    }

    /// Put out the least recently joined stretches past the size limit, but
    /// none of `joined`, which a render has just used; and what older Kerfs
    /// and stopped encoders left behind.
    pub(crate) fn tidy(&self, joined: &[PathBuf]) {
        let old = |path: &Path| {
            std::fs::metadata(path)
                .and_then(|data| data.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > STALE)
        };
        let listed = |folder: &Path| -> Vec<PathBuf> {
            std::fs::read_dir(folder)
                .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
                .unwrap_or_default()
        };
        for other in listed(&self.root) {
            if other != self.folder && other.is_dir() && old(&other) {
                let _ = std::fs::remove_dir_all(&other);
            }
        }
        let mut files = Vec::new();
        for folder in listed(&self.folder) {
            let making = folder.ends_with("making");
            for file in listed(&folder) {
                if making {
                    if old(&file) {
                        let _ = std::fs::remove_file(&file);
                    }
                } else if let Ok(data) = std::fs::metadata(&file) {
                    let read = data.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    files.push((read, data.len(), file));
                }
            }
        }
        let mut held: u64 = files.iter().map(|(_, size, _)| size).sum();
        if held <= self.budget {
            return;
        }
        files.sort();
        for (_, size, file) in files {
            if held <= self.budget / 10 * 9 {
                break;
            }
            if !joined.contains(&file) && std::fs::remove_file(&file).is_ok() {
                held -= size;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Numbers for `count` frames that come out the same every time.
    fn keys(count: usize, seed: u64) -> Vec<Option<u64>> {
        let mut state = seed;
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                Some(state)
            })
            .collect()
    }

    #[test]
    fn stretches_are_between_two_and_ten_seconds_and_about_four() {
        let keys = keys(24 * 600, 7);
        let found = stretches(&keys, 24.0);
        // Every frame is in one, in order.
        assert_eq!(found[0].frames.start, 0);
        assert_eq!(found.last().unwrap().frames.end, keys.len());
        assert!(found
            .windows(2)
            .all(|pair| pair[0].frames.end == pair[1].frames.start));
        let lengths: Vec<usize> = found.iter().map(|stretch| stretch.frames.len()).collect();
        let (last, rest) = lengths.split_last().unwrap();
        assert!(rest.iter().all(|length| (48..=240).contains(length)), "{lengths:?}");
        // What is left at the end is never a short stretch of its own.
        assert!((48..240 + 48).contains(last), "{lengths:?}");
        for count in 49..400 {
            let found = stretches(&keys[..count], 24.0);
            assert!(
                found.iter().all(|stretch| stretch.frames.len() >= 48),
                "{count} frames: {found:?}"
            );
            assert_eq!(found.last().unwrap().frames.end, count);
        }
        let mean = keys.len() as f64 / found.len() as f64 / 24.0;
        assert!((3.5..4.6).contains(&mean), "{mean}");
        assert!(found.iter().all(|stretch| stretch.keyed));
        // A render shorter than the shortest is one stretch, and none has none.
        assert_eq!(stretches(&keys[..10], 24.0).len(), 1);
        assert!(stretches(&[], 24.0).is_empty());
    }

    #[test]
    fn an_edit_changes_the_stretches_around_it_and_no_others() {
        let before = keys(24 * 300, 11);
        let names = |keys: &[Option<u64>]| -> Vec<String> {
            stretches(keys, 24.0)
                .into_iter()
                .map(|stretch| name(b"h264", &keys[stretch.frames]))
                .collect()
        };
        let kept = names(&before);
        // Two seconds cut out a minute in: what follows moves, and is the same.
        let mut cut = before.clone();
        cut.drain(24 * 60..24 * 62);
        let after = names(&cut);
        let lost = kept.iter().filter(|name| !after.contains(name)).count();
        assert!((1..=3).contains(&lost), "{lost} of {}", kept.len());
        // Five seconds drawn another way: the stretches they are in, and no more.
        let mut graded = before.clone();
        graded.splice(24 * 100..24 * 105, keys(24 * 5, 99));
        let after = names(&graded);
        let lost = kept.iter().filter(|name| !after.contains(name)).count();
        assert!((1..=5).contains(&lost), "{lost} of {}", kept.len());
        // Another encoding is other files.
        let other: Vec<String> = stretches(&before, 24.0)
            .into_iter()
            .map(|stretch| name(b"prores", &before[stretch.frames]))
            .collect();
        assert!(other.iter().all(|name| !kept.contains(name)));
    }

    #[test]
    fn frames_with_no_number_are_stretches_of_their_own() {
        let mut keys = keys(24 * 30, 3);
        for key in &mut keys[100..160] {
            *key = None;
        }
        let found = stretches(&keys, 24.0);
        let unkeyed: Vec<&Stretch> = found.iter().filter(|stretch| !stretch.keyed).collect();
        assert_eq!(unkeyed.len(), 1);
        assert_eq!(unkeyed[0].frames, 100..160);
        // No stretch with a name has such a frame in it.
        let named = |found: &[Stretch], keys: &[Option<u64>]| {
            found
                .iter()
                .filter(|stretch| stretch.keyed)
                .all(|stretch| keys[stretch.frames.clone()].iter().all(Option::is_some))
        };
        assert!(named(&found, &keys));
        // A few such frames are too short to be a file of their own: they go with the stretch
        // before them, which then has no name either; and at the very start, with the one after.
        for (from, to) in [(300, 303), (0, 2), (718, 720)] {
            let mut keys = self::keys(24 * 30, 3);
            for key in &mut keys[from..to] {
                *key = None;
            }
            let found = stretches(&keys, 24.0);
            assert!(found.iter().all(|stretch| stretch.frames.len() >= 48), "{found:?}");
            assert_eq!(found.iter().filter(|stretch| !stretch.keyed).count(), 1, "{found:?}");
            assert!(named(&found, &keys));
        }
    }

    #[test]
    fn a_stretch_is_kept_whole_and_the_least_recently_joined_go_first() {
        let dir = tempfile::tempdir().unwrap();
        let kept = Kept::at(dir.path(), "stamp", 2500).unwrap();
        let name = |index: usize| format!("{index:032x}");
        assert!(kept.find(&name(0), "mp4").is_none());
        let mut paths = Vec::new();
        for index in 0..3 {
            let making = kept.making("mp4");
            // Nothing is found while it is being written.
            std::fs::write(&making, vec![0u8; 1000]).unwrap();
            assert!(kept.find(&name(index), "mp4").is_none());
            paths.push(kept.keep(&making, &name(index), "mp4").unwrap());
            assert!(!making.exists());
            std::thread::sleep(Duration::from_millis(20));
        }
        // Reading the first makes the second the least recently joined.
        assert_eq!(kept.find(&name(0), "mp4").as_ref(), Some(&paths[0]));
        kept.tidy(&[]);
        assert!(paths[0].exists() && !paths[1].exists() && paths[2].exists());
        // What a render has just joined stays, however old.
        let making = kept.making("mp4");
        std::fs::write(&making, vec![0u8; 1000]).unwrap();
        let fourth = kept.keep(&making, &name(3), "mp4").unwrap();
        kept.tidy(&[paths[2].clone(), fourth.clone()]);
        assert!(!paths[0].exists() && paths[2].exists() && fourth.exists());
        // Another Kerf's folder is another folder.
        let other = Kept::at(dir.path(), "other", 2500).unwrap();
        assert!(other.find(&name(3), "mp4").is_none());
    }
}
