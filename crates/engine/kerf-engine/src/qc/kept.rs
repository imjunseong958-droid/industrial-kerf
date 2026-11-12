//! What a QC pass measured, kept in the project between passes.
//!
//! A frame's measurements are kept under the number the compositor gives
//! for everything the frame depends on (`Compositor::frame_key`): the
//! clips that draw it and their place in the sequence, their effects, the
//! footage files' sizes and times. A frame whose number is known is not
//! drawn or measured again, so a pass after an edit measures the frames
//! the edit changed. A sound measurement is kept whole, under the FFmpeg
//! command that made it and the files it read.
//!
//! The files are in `.kerf/cache/qc`, which is nobody's to keep: named for
//! what measures (the code that draws, the filter graph, this FFmpeg), so a
//! new Kerf or FFmpeg starts again, and written whole or not at all.

use super::Measure;
use crate::derivatives::SourceIdentity;
use crate::ffmpeg::resolve_ffmpeg_bin;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Where a project keeps what QC measured.
const FOLDER: &str = ".kerf/cache/qc";
const MAGIC: &[u8; 8] = b"KERFQC01";
/// A key, four levels, and whether the frame is black.
const RECORD: usize = 8 + 4 * 8 + 1;
/// The most frames kept: some five hours at 30 a second, 25 MB.
const MOST: usize = 600_000;

fn fnv(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

const FNV_START: u64 = 0xCBF2_9CE4_8422_2325;

/// The FFmpeg that measures, by what it says it is.
fn ffmpeg_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        crate::ffmpeg::hidden_command(resolve_ffmpeg_bin())
            .arg("-version")
            .output()
            .ok()
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .map(str::to_owned)
            })
            .unwrap_or_default()
    })
}

/// A name for what measures: `parts`, this FFmpeg, and the code that draws.
fn stamp(parts: &[&str]) -> String {
    let mut hash = fnv(FNV_START, kerf_compose::DRAW_STAMP.as_bytes());
    hash = fnv(hash, ffmpeg_version().as_bytes());
    for part in parts {
        hash = fnv(fnv(hash, &[0]), part.as_bytes());
    }
    format!("{hash:016x}")
}

/// Write `bytes` to `file` whole or not at all, and take away the files of
/// its kind (`prefix`) that another Kerf or FFmpeg left beside it.
fn write(file: &Path, prefix: &str, bytes: &[u8]) {
    let Some(folder) = file.parent() else {
        return;
    };
    if fs::create_dir_all(folder).is_err() {
        return;
    }
    let partial = file.with_extension(format!("{}.partial", std::process::id()));
    if fs::write(&partial, bytes).is_err() || fs::rename(&partial, file).is_err() {
        let _ = fs::remove_file(&partial);
        return;
    }
    for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        let stale = path != file
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(prefix))
            && path.extension().is_none_or(|extension| extension != "partial");
        if stale {
            let _ = fs::remove_file(path);
        }
    }
}

/// The measurements of frames kept in a project, by their frame keys.
pub(super) struct KeptFrames {
    /// `None` where there is no project folder to keep them in.
    file: Option<PathBuf>,
    measures: HashMap<u64, Measure>,
    /// The keys as the file had them: newest first.
    order: Vec<u64>,
}

impl KeptFrames {
    /// What the project in `dir` keeps of frames measured by `graph`.
    pub(super) fn open(dir: &Path, graph: &str) -> Self {
        let file = dir.is_dir().then(|| {
            dir.join(FOLDER)
                .join(format!("picture-{}.bin", stamp(&[graph])))
        });
        let mut kept = Self {
            file,
            measures: HashMap::new(),
            order: Vec::new(),
        };
        let bytes = kept.file.as_ref().and_then(|file| fs::read(file).ok());
        // A file cut short, or not one of these, holds nothing to trust.
        let records = bytes
            .as_deref()
            .and_then(|bytes| bytes.strip_prefix(MAGIC))
            .map(<[u8]>::as_chunks::<RECORD>);
        if let Some((records, [])) = records {
            for record in records {
                let number = |at: usize| {
                    f64::from_le_bytes(record[at..at + 8].try_into().expect("eight bytes"))
                };
                let key = u64::from_le_bytes(record[..8].try_into().expect("eight bytes"));
                let measure = Measure {
                    ymin: number(8),
                    ymax: number(16),
                    yavg: number(24),
                    brng: number(32),
                    black: record[40] != 0,
                };
                if kept.measures.insert(key, measure).is_none() {
                    kept.order.push(key);
                }
            }
        }
        kept
    }

    pub(super) fn get(&self, key: u64) -> Option<Measure> {
        self.measures.get(&key).copied()
    }

    pub(super) fn put(&mut self, key: u64, measure: Measure) {
        self.measures.insert(key, measure);
    }

    /// Keep the frames of this pass (`used`) and, after them, as many of
    /// the ones kept before as there is room for.
    pub(super) fn save(&self, used: impl Iterator<Item = u64>) {
        let Some(file) = &self.file else {
            return;
        };
        let mut bytes = MAGIC.to_vec();
        let mut written = std::collections::HashSet::new();
        for key in used.chain(self.order.iter().copied()) {
            if written.len() >= MOST {
                break;
            }
            let Some(measure) = self.measures.get(&key) else {
                continue;
            };
            if !written.insert(key) {
                continue;
            }
            bytes.extend_from_slice(&key.to_le_bytes());
            for level in [measure.ymin, measure.ymax, measure.yavg, measure.brng] {
                bytes.extend_from_slice(&level.to_le_bytes());
            }
            bytes.push(u8::from(measure.black));
        }
        write(file, "picture-", &bytes);
    }
}

/// Where the project in `dir` keeps the sound measurement FFmpeg makes
/// with `args`, or `None` where there is no project folder or a file the
/// command reads cannot be told apart from another. The name holds the
/// command and each file's size and time, so one that reads anything else
/// has another name.
pub(crate) fn sound_file(dir: &Path, args: &[String]) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let mut parts: Vec<String> = args.to_vec();
    for input in args
        .windows(2)
        .filter(|pair| pair[0] == "-i")
        .map(|pair| &pair[1])
    {
        // What is not a file is a source FFmpeg makes itself, which the
        // command already says in full.
        let path = dir.join(input);
        if path.is_file() {
            let identity = SourceIdentity::from_path(&path).ok()?;
            parts.push(format!(
                "{}:{}:{:?}",
                identity.path, identity.size, identity.modified_ns
            ));
        }
    }
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    Some(
        dir.join(FOLDER)
            .join(format!("sound-{}.json", stamp(&parts))),
    )
}

/// Keep the sound measurement `text` in `file`, in place of the last one.
pub(crate) fn save_sound(file: &Path, text: &str) {
    write(file, "sound-", text.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(level: f64, black: bool) -> Measure {
        Measure {
            ymin: level,
            ymax: level + 1.0,
            yavg: level + 0.5,
            brng: 0.25,
            black,
        }
    }

    #[test]
    fn frames_are_kept_between_passes_newest_first_and_whole() {
        let dir = tempfile::tempdir().unwrap();
        let mut kept = KeptFrames::open(dir.path(), "graph");
        assert!(kept.get(7).is_none());
        kept.put(7, measure(100.0, false));
        kept.put(8, measure(64.0, true));
        kept.save([7, 8].into_iter());

        let mut again = KeptFrames::open(dir.path(), "graph");
        assert_eq!(again.get(7), Some(measure(100.0, false)));
        assert_eq!(again.get(8), Some(measure(64.0, true)));
        // A pass that uses one of them and measures another keeps all three.
        again.put(9, measure(200.0, false));
        again.save([9, 7].into_iter());
        let third = KeptFrames::open(dir.path(), "graph");
        assert_eq!(third.order, [9, 7, 8]);

        // Another graph measures other things: nothing of this is its.
        assert!(KeptFrames::open(dir.path(), "another").get(7).is_none());
        // And once it keeps something, the first graph's file is gone.
        let mut other = KeptFrames::open(dir.path(), "another");
        other.put(1, measure(1.0, false));
        other.save([1].into_iter());
        assert!(KeptFrames::open(dir.path(), "graph").get(7).is_none());

        // A file cut short is not read as measurements.
        let file = other.file.clone().unwrap();
        let bytes = fs::read(&file).unwrap();
        fs::write(&file, &bytes[..bytes.len() - 3]).unwrap();
        assert!(KeptFrames::open(dir.path(), "another").get(1).is_none());
    }

    #[test]
    fn nothing_is_kept_where_there_is_no_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nowhere");
        let mut kept = KeptFrames::open(&missing, "graph");
        kept.put(1, measure(1.0, false));
        kept.save([1].into_iter());
        assert!(!missing.exists());
        assert!(sound_file(&missing, &[]).is_none());
    }

    #[test]
    fn a_sound_measurement_is_named_for_its_command_and_what_it_reads() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("a.wav");
        fs::write(&media, b"one").unwrap();
        let args = |graph: &str| -> Vec<String> {
            ["-i", "a.wav", "-filter_complex", graph]
                .map(String::from)
                .to_vec()
        };
        let first = sound_file(dir.path(), &args("volume=1")).unwrap();
        assert_eq!(sound_file(dir.path(), &args("volume=1")).unwrap(), first);
        assert_ne!(sound_file(dir.path(), &args("volume=2")).unwrap(), first);
        // The same command over a file that has changed is another measurement.
        fs::write(&media, b"three").unwrap();
        assert_ne!(sound_file(dir.path(), &args("volume=1")).unwrap(), first);
    }
}
