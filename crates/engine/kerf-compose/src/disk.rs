//! Frames kept on disk by what they depend on, between sessions and renders.
//!
//! A frame is a file named for its key, `<folder>/<stamp>/<2 hex>/<32 hex>`, compressed with LZ4. The
//! key is everything the frame depends on (`Drawing::frame_key`), footage files' sizes and times
//! among them, so a changed document or file never reads stale pixels: its keys simply change. The
//! stamp is the code that draws (`build.rs`), so neither does a new Kerf. Each file ends up whole or
//! not at all, written under another name and renamed, and one that does not read back as written
//! is deleted and drawn again, so any number of processes can share a folder. Writing goes on behind
//! the drawing, and when it falls behind a frame is dropped rather than making drawing wait. Past a
//! size limit the least recently used go.

use crate::Frame;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

const MAGIC: &[u8; 4] = b"KRFF";
const HEADER: usize = 4 + 4 + 4 + 1 + 8;
/// Frames waiting to be written before more are dropped.
const QUEUE: usize = 48;
/// How many threads compress and write.
const WRITERS: usize = 2;
/// How long a folder of another Kerf's frames stays, unread, before it goes.
const STALE: Duration = Duration::from_secs(3 * 24 * 3600);
const DEFAULT_GIGABYTES: u64 = 10;

/// What a folder of frames has done since this process opened it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub writes: u64,
    pub dropped: u64,
    pub damaged: u64,
    pub evicted: u64,
}

struct Job {
    key: u128,
    deep: bool,
    frame: Frame,
}

pub(crate) struct DiskFrames {
    folder: PathBuf,
    queue: SyncSender<Job>,
    /// Frames asked to be written and not yet.
    pending: Arc<AtomicU64>,
    hits: AtomicU64,
    misses: AtomicU64,
    writes: Arc<AtomicU64>,
    dropped: AtomicU64,
    damaged: AtomicU64,
    evicted: Arc<AtomicU64>,
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

fn encode(frame: &Frame, deep: bool) -> Vec<u8> {
    let packed = lz4_flex::compress_prepend_size(&frame.rgba);
    let mut out = Vec::with_capacity(HEADER + packed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&frame.width.to_le_bytes());
    out.extend_from_slice(&frame.height.to_le_bytes());
    out.push(u8::from(deep));
    out.extend_from_slice(&checksum(&packed).to_le_bytes());
    out.extend_from_slice(&packed);
    out
}

/// The frame in `bytes`, unless they are not what [`encode`] wrote.
fn decode(bytes: &[u8], deep: bool) -> Option<Frame> {
    if bytes.len() < HEADER || &bytes[..4] != MAGIC || bytes[12] != u8::from(deep) {
        return None;
    }
    let word =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let (width, height) = (word(4), word(8));
    let mut stored = [0u8; 8];
    stored.copy_from_slice(&bytes[13..21]);
    let packed = &bytes[HEADER..];
    if checksum(packed) != u64::from_le_bytes(stored) {
        return None;
    }
    let rgba = lz4_flex::decompress_size_prepended(packed).ok()?;
    let bytes_a_pixel = if deep { 8 } else { 4 };
    (rgba.len() == width as usize * height as usize * bytes_a_pixel).then_some(Frame {
        width,
        height,
        rgba,
    })
}

fn files_under(folder: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut found = Vec::new();
    let Ok(shards) = std::fs::read_dir(folder) else {
        return found;
    };
    for shard in shards.flatten() {
        let Ok(entries) = std::fs::read_dir(shard.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    let used = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    found.push((entry.path(), metadata.len(), used));
                }
            }
        }
    }
    found
}

/// The default folder for frames: the user's cache folder, or `KERF_FRAME_CACHE`, which `off` turns
/// them off. `None` when frames are off or there is nowhere to put them.
pub fn default_frame_folder() -> Option<PathBuf> {
    match std::env::var_os("KERF_FRAME_CACHE") {
        Some(value) if value == "off" || value == "0" || value.is_empty() => return None,
        Some(value) => return Some(PathBuf::from(value)),
        None => {}
    }
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        var("LOCALAPPDATA").map(|folder| folder.join("Kerf").join("frames"))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Caches/Kerf/frames"))
    } else {
        var("XDG_CACHE_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".cache")))
            .map(|folder| folder.join("kerf").join("frames"))
    }
}

/// What `folder` has done since this process opened it; nothing if it has not.
pub fn frame_cache_stats(folder: &Path) -> FrameCacheStats {
    opened()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(folder)
        .map_or_else(FrameCacheStats::default, |frames| frames.stats())
}

fn opened() -> &'static Mutex<HashMap<PathBuf, Arc<DiskFrames>>> {
    static OPENED: OnceLock<Mutex<HashMap<PathBuf, Arc<DiskFrames>>>> = OnceLock::new();
    OPENED.get_or_init(Default::default)
}

/// The frames in `folder`, shared by every compositor in this process.
pub(crate) fn open(folder: &Path) -> Arc<DiskFrames> {
    let mut opened = opened().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(frames) = opened.get(folder) {
        return frames.clone();
    }
    let gigabytes = std::env::var("KERF_FRAME_CACHE_GB")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(DEFAULT_GIGABYTES);
    let frames = Arc::new(DiskFrames::new(
        folder,
        env!("KERF_DRAW_STAMP"),
        gigabytes << 30,
    ));
    opened.insert(folder.to_owned(), frames.clone());
    frames
}

impl DiskFrames {
    fn new(root: &Path, stamp: &str, budget: u64) -> Self {
        let folder = root.join(stamp);
        let (queue, jobs) = sync_channel(QUEUE);
        let jobs = Arc::new(Mutex::new(jobs));
        let total = Arc::new(AtomicU64::new(0));
        let writes = Arc::new(AtomicU64::new(0));
        let evicted = Arc::new(AtomicU64::new(0));
        let evicting = Arc::new(Mutex::new(()));
        let pending = Arc::new(AtomicU64::new(0));
        let writer = || Writer {
            jobs: jobs.clone(),
            folder: folder.clone(),
            budget,
            total: total.clone(),
            writes: writes.clone(),
            evicted: evicted.clone(),
            evicting: evicting.clone(),
            pending: pending.clone(),
        };
        let first = writer();
        let others: Vec<Writer> = (1..WRITERS).map(|_| writer()).collect();
        let (root, stamp) = (root.to_owned(), stamp.to_owned());
        start_writer(0, move || {
            // Before anyone writes: a frame written during the count might be counted as well,
            // and a count too high sends frames away that fit.
            first.count();
            for (index, other) in (1..).zip(others) {
                start_writer(index, move || other.run());
            }
            first.tidy(&root, &stamp);
            first.run();
        });
        Self {
            folder,
            queue,
            pending,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            writes,
            dropped: AtomicU64::new(0),
            damaged: AtomicU64::new(0),
            evicted,
        }
    }

    fn path(folder: &Path, key: u128, deep: bool) -> PathBuf {
        let name = format!("{key:032x}{}", if deep { "d" } else { "" });
        folder.join(&name[..2]).join(name)
    }

    /// The frame kept under `key`, which then counts as just used.
    pub fn get(&self, key: u128, deep: bool) -> Option<Frame> {
        let path = Self::path(&self.folder, key, deep);
        let Ok(bytes) = std::fs::read(&path) else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        match decode(&bytes, deep) {
            Some(frame) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                if let Ok(file) = std::fs::File::options().write(true).open(&path) {
                    let _ = file.set_modified(SystemTime::now());
                }
                Some(frame)
            }
            None => {
                self.damaged.fetch_add(1, Ordering::Relaxed);
                self.misses.fetch_add(1, Ordering::Relaxed);
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// Keep `frame` under `key`, soon, or not at all if writing has fallen behind.
    pub fn put(&self, key: u128, deep: bool, frame: Frame) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if let Err(error) = self.queue.try_send(Job { key, deep, frame }) {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            if matches!(error, TrySendError::Full(_)) {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Wait for what has been asked to be written.
    pub fn flush(&self) {
        while self.pending.load(Ordering::SeqCst) > 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn stats(&self) -> FrameCacheStats {
        FrameCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            writes: self.writes.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            damaged: self.damaged.load(Ordering::Relaxed),
            evicted: self.evicted.load(Ordering::Relaxed),
        }
    }
}

fn start_writer(index: usize, work: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name(format!("kerf-frames-{index}"))
        .spawn(work);
}

struct Writer {
    jobs: Arc<Mutex<Receiver<Job>>>,
    folder: PathBuf,
    budget: u64,
    total: Arc<AtomicU64>,
    writes: Arc<AtomicU64>,
    evicted: Arc<AtomicU64>,
    /// Keeps eviction to one writer at a time.
    evicting: Arc<Mutex<()>>,
    pending: Arc<AtomicU64>,
}

impl Writer {
    /// Count what is there.
    fn count(&self) {
        let here: u64 = files_under(&self.folder).iter().map(|file| file.1).sum();
        self.total.fetch_add(here, Ordering::Relaxed);
    }

    /// Let another Kerf's frames go once nothing has read them for a while.
    fn tidy(&self, root: &Path, stamp: &str) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy() == stamp {
                continue;
            }
            let newest = files_under(&entry.path())
                .iter()
                .map(|file| file.2)
                .max()
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if newest.elapsed().is_ok_and(|age| age > STALE) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    fn run(&self) {
        loop {
            let job = self
                .jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv();
            let Ok(job) = job else {
                return;
            };
            self.write(job.key, job.deep, &job.frame);
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn write(&self, key: u128, deep: bool, frame: &Frame) {
        let path = DiskFrames::path(&self.folder, key, deep);
        if path.is_file() {
            return;
        }
        let Some(shard) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(shard).is_err() {
            return;
        }
        let bytes = encode(frame, deep);
        let partial = path.with_extension(format!("{}.partial", std::process::id()));
        if std::fs::write(&partial, &bytes).is_err() {
            return;
        }
        if std::fs::rename(&partial, &path).is_err() {
            let _ = std::fs::remove_file(&partial);
            return;
        }
        self.writes.fetch_add(1, Ordering::Relaxed);
        let total =
            self.total.fetch_add(bytes.len() as u64, Ordering::Relaxed) + bytes.len() as u64;
        if total > self.budget {
            self.evict();
        }
    }

    /// Delete the least recently used down to nine tenths of the budget, going by what is really
    /// there, since other processes write here too.
    fn evict(&self) {
        let Ok(_alone) = self.evicting.try_lock() else {
            return;
        };
        let mut files = files_under(&self.folder);
        files.sort_by_key(|file| file.2);
        let mut total: u64 = files.iter().map(|file| file.1).sum();
        let goal = self.budget / 10 * 9;
        for (path, bytes, _) in files {
            if total <= goal {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total -= bytes;
                self.evicted.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.total.store(total, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seed: u8, width: u32, height: u32, deep: bool) -> Frame {
        let size = width as usize * height as usize * if deep { 8 } else { 4 };
        Frame {
            width,
            height,
            rgba: (0..size)
                .map(|i| (i as u8).wrapping_mul(seed).wrapping_add(seed))
                .collect(),
        }
    }

    #[test]
    fn a_frame_comes_back_as_it_went_in() {
        let dir = tempfile::tempdir().expect("a folder");
        let frames = DiskFrames::new(dir.path(), "stamp", 1 << 30);
        let kept = frame(3, 64, 36, false);
        frames.put(7, false, kept.clone());
        frames.flush();
        assert_eq!(frames.get(7, false), Some(kept));
        assert_eq!(frames.get(8, false), None);
        let stats = frames.stats();
        assert_eq!((stats.hits, stats.misses, stats.writes), (1, 1, 1));
    }

    #[test]
    fn eight_and_sixteen_bit_frames_are_kept_apart() {
        let dir = tempfile::tempdir().expect("a folder");
        let frames = DiskFrames::new(dir.path(), "stamp", 1 << 30);
        let (shallow, deep) = (frame(3, 16, 9, false), frame(5, 16, 9, true));
        frames.put(1, false, shallow.clone());
        frames.put(1, true, deep.clone());
        frames.flush();
        assert_eq!(frames.get(1, false), Some(shallow));
        assert_eq!(frames.get(1, true), Some(deep));
    }

    #[test]
    fn they_stay_for_the_next_process_and_not_for_another_kerf() {
        let dir = tempfile::tempdir().expect("a folder");
        let kept = frame(3, 16, 9, false);
        {
            let frames = DiskFrames::new(dir.path(), "stamp", 1 << 30);
            frames.put(1, false, kept.clone());
            frames.flush();
        }
        let again = DiskFrames::new(dir.path(), "stamp", 1 << 30);
        assert_eq!(again.get(1, false), Some(kept));
        let newer = DiskFrames::new(dir.path(), "another", 1 << 30);
        assert_eq!(newer.get(1, false), None);
    }

    #[test]
    fn a_damaged_file_is_a_miss_and_goes() {
        let dir = tempfile::tempdir().expect("a folder");
        let frames = DiskFrames::new(dir.path(), "stamp", 1 << 30);
        for (index, damage) in [
            // A bit flipped in the picture.
            (
                1u128,
                Box::new(|bytes: &mut Vec<u8>| {
                    let last = bytes.len() - 1;
                    bytes[last] ^= 1;
                }) as Box<dyn Fn(&mut Vec<u8>)>,
            ),
            // Cut short.
            (
                2,
                Box::new(|bytes: &mut Vec<u8>| bytes.truncate(bytes.len() / 2)),
            ),
            // Not a frame at all.
            (3, Box::new(|bytes: &mut Vec<u8>| *bytes = vec![0; 7])),
        ] {
            frames.put(index, false, frame(9, 32, 18, false));
            frames.flush();
            let path = DiskFrames::path(&frames.folder, index, false);
            let mut bytes = std::fs::read(&path).expect("written");
            damage(&mut bytes);
            std::fs::write(&path, bytes).expect("damaged");
            assert_eq!(frames.get(index, false), None);
            assert!(!path.exists(), "the damaged file stays");
        }
        assert_eq!(frames.stats().damaged, 3);
    }

    #[test]
    fn past_the_limit_the_least_recently_used_go() {
        let dir = tempfile::tempdir().expect("a folder");
        let one = encode(&frame(3, 64, 64, false), false).len() as u64;
        let frames = DiskFrames::new(dir.path(), "stamp", one * 4);
        // Dated by hand, long ago and an hour apart, so that the order hangs neither on how long a
        // step takes nor on how finely the file system tells the time.
        let long_ago = SystemTime::now() - Duration::from_secs(24 * 3600);
        for key in 0..3u128 {
            frames.put(key, false, frame(3, 64, 64, false));
            frames.flush();
            std::fs::File::options()
                .write(true)
                .open(DiskFrames::path(&frames.folder, key, false))
                .expect("kept")
                .set_modified(long_ago + Duration::from_secs(3600 * key as u64))
                .expect("dated");
        }
        // Reading the first makes it the newest.
        assert!(frames.get(0, false).is_some());
        for key in 3..6u128 {
            frames.put(key, false, frame(3, 64, 64, false));
            frames.flush();
        }
        // The fifth goes past the limit, and the two unread since 0 go to make room: only those.
        assert_eq!(frames.stats().evicted, 2);
        assert!(frames.get(1, false).is_none(), "the oldest stays");
        assert!(frames.get(2, false).is_none(), "the next oldest stays");
        assert!(frames.get(0, false).is_some(), "what was just used goes");
        assert!(frames.get(5, false).is_some(), "the newest goes");
        let on_disk: u64 = files_under(&frames.folder).iter().map(|f| f.1).sum();
        assert!(on_disk <= one * 4, "{on_disk} of {}", one * 4);
    }

    #[test]
    fn what_is_written_while_the_folder_is_counted_counts_once() {
        let dir = tempfile::tempdir().expect("a folder");
        let kept = frame(3, 16, 9, false);
        let one = encode(&kept, false).len() as u64;
        let spread = |index: u64| (u128::from(index % 256) << 120) | u128::from(index);
        // Enough already there, across every shard, that counting it takes a while.
        let (there, soon, later) = (512u64, 32u64, 32u64);
        for index in 0..there {
            let path = DiskFrames::path(&dir.path().join("stamp"), spread(index), false);
            std::fs::create_dir_all(path.parent().expect("a shard")).expect("made");
            std::fs::write(path, encode(&kept, false)).expect("written");
        }
        // Exactly full once these are written, and not past it, so nothing has to go.
        let frames = DiskFrames::new(dir.path(), "stamp", one * (there + soon + later));
        // Some at once, which land while it counts...
        for index in there..there + soon {
            frames.put(spread(index), false, kept.clone());
        }
        // ...and some one by one, the last of which would see a count that came out too high.
        for index in there + soon..there + soon + later {
            frames.put(spread(index), false, kept.clone());
            frames.flush();
        }
        let stats = frames.stats();
        assert_eq!(stats.writes, soon + later, "{stats:?}");
        assert_eq!(stats.evicted, 0, "{stats:?}");
    }

    #[test]
    fn another_kerfs_frames_go_only_once_unread_for_a_while() {
        let dir = tempfile::tempdir().expect("a folder");
        let (old, recent) = (dir.path().join("old"), dir.path().join("recent"));
        for folder in [&old, &recent] {
            std::fs::create_dir_all(folder.join("ab")).expect("made");
            std::fs::write(folder.join("ab").join("frame"), [0u8; 8]).expect("written");
        }
        let file = std::fs::File::options()
            .write(true)
            .open(old.join("ab").join("frame"))
            .expect("opens");
        file.set_modified(SystemTime::now() - STALE - Duration::from_secs(60))
            .expect("set");
        let _frames = DiskFrames::new(dir.path(), "stamp", 1 << 30);
        // Tidying goes on behind whatever is asked of the frames, and takes as long as it takes.
        let since = std::time::Instant::now();
        while old.exists() && since.elapsed() < Duration::from_secs(60) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!old.exists());
        assert!(recent.exists());
    }

    #[test]
    fn a_full_queue_drops_rather_than_waits() {
        let dir = tempfile::tempdir().expect("a folder");
        let frames = DiskFrames::new(dir.path(), "stamp", 1 << 40);
        // Far more than the queue holds, in no time at all.
        let many = frame(3, 128, 128, false);
        for key in 0..2_000u128 {
            frames.put(key, false, many.clone());
        }
        frames.flush();
        let stats = frames.stats();
        assert!(stats.dropped > 0, "{stats:?}");
        assert_eq!(stats.writes + stats.dropped, 2_000, "{stats:?}");
    }

    #[test]
    fn the_folder_is_off_when_asked() {
        // Read at each open, so that a test can set it for itself; here it only reads the value.
        let folder = default_frame_folder();
        if let Some(value) = std::env::var_os("KERF_FRAME_CACHE") {
            assert_eq!(
                folder.is_none(),
                value == "off" || value == "0" || value.is_empty()
            );
        }
    }
}
