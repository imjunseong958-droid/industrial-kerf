//! A render's sound, made from what earlier renders kept of it, so that after
//! an edit only what the edit touched is decoded and encoded again.
//!
//! Two things are kept, beside the picture's stretches ([`crate::sections`]):
//!
//! - **Each clip's sound, decoded.** What a media clip reads of its file, as
//!   stereo floats at the program's rate, named by the file and the part of
//!   it read. Opening a file once for each clip was most of what mixing took;
//!   a mix from these reads sixty small files of samples in place of sixty
//!   seeks into the footage, and is the same samples.
//! - **The encoded sound, in pieces.** AAC is frames of 1024 samples on a
//!   grid that starts with the program, each overlapping its neighbours, so
//!   a piece is a run of whole frames on that grid, named by the samples it
//!   was encoded from. An edit that changes the sound for a while changes
//!   the pieces it falls in. One that closes a gap moves everything after it
//!   to another place on the grid, and those pieces are encoded again, each
//!   by an encoder of its own, at once.
//!
//! A piece cannot simply follow another. An encoder that starts in the
//! middle of a sound knows nothing of what came before, and its first frames
//! are made for a start: short windows where its neighbour's last frame has
//! a long one, and the two halves no longer cancel. So each piece is encoded
//! from [`LEAD`] frames before its first, which are kept with it, and two
//! pieces are joined at a frame where both have the same window before it:
//! the one piece's own frame there, and the other's lead. Then what is heard
//! across the join is each encoder's own error and nothing else. A join with
//! no such frame is not made: the sound is encoded whole.
//!
//! Sound that is not AAC (the master's PCM) is mixed from the kept clips
//! straight into its file.

use crate::error::{EngineError, Result};
use crate::ffmpeg::{hidden_command, resolve_ffmpeg_bin};
use crate::render::{ffmpeg_version, finished_well};
use crate::sections::{self, Kept};
use crate::sidecar::{self, CancellationToken};
use kerf_core::program::{decoded_filters, secs, SoundInput, SAMPLE_RATE};
use kerf_model::Time;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Samples in a frame of AAC, and before the first: an encoder's first
/// frame holds what comes before the sound, and a file says to skip it.
const FRAME: u64 = 1024;
/// Frames in a piece: 21.8 seconds. Measured on five minutes of a film's
/// sound at 128 kb/s, an encoder is 0.4 dB noisier for its first seconds
/// than one that has been running; pieces a quarter as long made that 0.8.
const PIECE: u64 = 1024;
/// Frames encoded before a piece's first, and kept with it. An encoder's
/// first three are made for a start; from the fourth its windows were the
/// running encoder's at every one of 54 joins measured, and the rest are
/// other places to join when a frame's window cannot be read.
const LEAD: u64 = 16;
/// Frames of sound an encoder is given past a piece's last, which it looks
/// at to choose that frame's window.
const AHEAD: u64 = 2;
/// Bytes in a sample of the mix: two channels of 32-bit floats.
const BYTES: u64 = 8;
const MAGIC: &[u8; 8] = b"KERFAAC1";

/// What a render's sound is mixed from and how it is encoded.
#[derive(Debug)]
pub(crate) struct Mix {
    /// What the graph reads of each file.
    pub inputs: Vec<SoundInput>,
    /// The graph, for inputs already decoded, and the stream of it to write.
    pub graph: Vec<String>,
    /// How the sound is encoded: `-c:a` and `-b:a` with their values, or
    /// nothing for what the file's kind encodes by default.
    pub codec: Vec<String>,
    /// What else the sound's file is told.
    pub wrap: Vec<String>,
    pub duration: Time,
}

/// A render's sound, made.
pub(crate) struct Made {
    /// Its file, for the render to keep or put out.
    pub file: PathBuf,
    /// The kept files it was made from.
    pub used: Vec<PathBuf>,
}

/// Files that go when the work that made them ends, however it ends.
#[derive(Default)]
struct Scratch(Vec<PathBuf>);

impl Scratch {
    fn file(&mut self, kept: &Kept, extension: &str) -> PathBuf {
        let path = kept.making(extension);
        self.0.push(path.clone());
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for file in &self.0 {
            let _ = std::fs::remove_file(file);
        }
    }
}

fn ffmpeg(args: &[String], cancellation: &CancellationToken) -> Result<()> {
    let mut command = hidden_command(resolve_ffmpeg_bin());
    command.args(["-y", "-hide_banner", "-nostats"]).args(args);
    finished_well(sidecar::run(command, cancellation, |_| {})?)
}

fn strings<const N: usize>(args: [&str; N]) -> Vec<String> {
    args.map(String::from).into()
}

/// `work` done to each of `items`, as many at once as the machine has
/// cores, in their order. The first that fails stops the rest.
fn at_once<T: Sync, R: Send>(
    items: &[T],
    cancellation: &CancellationToken,
    work: impl Fn(&T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    let workers = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .min(8)
        .min(items.len());
    let (next, failed) = (AtomicUsize::new(0), AtomicBool::new(false));
    let done: Mutex<Vec<Option<Result<R>>>> = Mutex::new(items.iter().map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, Ordering::Relaxed);
                if at >= items.len() || failed.load(Ordering::Relaxed) {
                    break;
                }
                let result = if cancellation.is_cancelled() {
                    Err(EngineError::Cancelled)
                } else {
                    work(&items[at])
                };
                if result.is_err() {
                    failed.store(true, Ordering::Relaxed);
                }
                done.lock().unwrap_or_else(|poisoned| poisoned.into_inner())[at] = Some(result);
            });
        }
    });
    let done = done
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut results = Vec::with_capacity(items.len());
    let mut stopped = false;
    for result in done {
        match result {
            Some(Ok(result)) => results.push(result),
            Some(Err(error)) => return Err(error),
            None => stopped = true,
        }
    }
    if stopped {
        return Err(EngineError::msg("work on the sound stopped"));
    }
    Ok(results)
}

/// The first sixteen bytes of a hash, as a kept file is named.
fn named(hash: Sha256) -> String {
    hex::encode(&hash.finalize()[..16])
}

/// The name a clip's decoded sound is kept by: the FFmpeg that decodes it,
/// the file as it is now, and the part of it read.
fn clip_name(input: &SoundInput) -> Result<String> {
    let file = std::fs::metadata(&input.path)?;
    let changed = file
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| EngineError::msg("a file older than time"))?;
    let mut hash = Sha256::new();
    hash.update(format!(
        "clip\n{}\n{}\n{}\n{} {}\n{} {}",
        resolve_ffmpeg_bin(),
        ffmpeg_version(),
        input.path,
        file.len(),
        changed.as_nanos(),
        input.seek.flicks(),
        input.span.flicks(),
    ));
    Ok(named(hash))
}

/// Each input's sound decoded, in the inputs' order: the kept file, or one
/// made now and kept.
fn clips(mix: &Mix, kept: &Kept, cancellation: &CancellationToken) -> Result<Vec<PathBuf>> {
    let names: Vec<String> = mix.inputs.iter().map(clip_name).collect::<Result<_>>()?;
    let mut files: Vec<Option<PathBuf>> = names.iter().map(|name| kept.find(name, "f32")).collect();
    let missing: Vec<usize> = (0..files.len()).filter(|at| files[*at].is_none()).collect();
    sections::count_sound_clips(files.len() - missing.len(), missing.len());
    let decoded = at_once(&missing, cancellation, |at| {
        let input = &mix.inputs[*at];
        let making = kept.making("f32");
        let mut args = input.args();
        args.extend(["-i".to_owned(), input.path.clone(), "-vn".to_owned()]);
        args.extend(["-af".to_owned(), decoded_filters(input.span)]);
        args.extend(strings(["-f", "f32le"]));
        args.push(making.to_string_lossy().into_owned());
        let made = ffmpeg(&args, cancellation)
            .and_then(|()| Ok(kept.keep(&making, &names[*at], "f32")?));
        if made.is_err() {
            let _ = std::fs::remove_file(&making);
        }
        made
    })?;
    for (at, file) in missing.into_iter().zip(decoded) {
        files[at] = Some(file);
    }
    Ok(files.into_iter().flatten().collect())
}

/// FFmpeg's arguments to mix the decoded `clips`, up to where the sound's
/// encoding and file follow.
fn mix_args(mix: &Mix, clips: &[PathBuf]) -> Vec<String> {
    let mut args = Vec::new();
    for clip in clips {
        args.extend(raw_input());
        args.push(clip.to_string_lossy().into_owned());
    }
    args.extend(mix.graph.iter().cloned());
    args.push("-vn".to_owned());
    args
}

/// What FFmpeg is told to read a file of the mix's samples; the file follows.
fn raw_input() -> Vec<String> {
    let mut args = strings(["-f", "f32le", "-ar"]);
    args.push(SAMPLE_RATE.to_string());
    args.extend(strings(["-ac", "2", "-i"]));
    args
}

/// Whether the sound is AAC from FFmpeg's own encoder, whose frames and
/// start this file knows: asked for by name, or what an MP4 or QuickTime
/// file encodes when nothing is asked.
fn is_aac(codec: &[String], extension: &str) -> bool {
    match codec.iter().position(|arg| arg == "-c:a") {
        Some(at) => codec.get(at + 1).is_some_and(|name| name == "aac"),
        None => matches!(extension, "mp4" | "mov"),
    }
}

/// Make a render's sound as a file of the render's kind (`extension`), from
/// what `kept` has of it. `busy` says the picture has enough to encode that
/// nothing waits for the sound.
pub(crate) fn make(
    mix: &Mix,
    kept: &Kept,
    extension: &str,
    busy: bool,
    cancellation: &CancellationToken,
) -> Result<Made> {
    let mut scratch = Scratch::default();
    let clips = clips(mix, kept, cancellation)?;
    let mut used = clips.clone();
    let mut args = mix_args(mix, &clips);
    let file = kept.making(extension);
    let lasts = ["-t".to_owned(), secs(mix.duration)];
    if !is_aac(&mix.codec, extension) {
        // Nothing of such a sound is kept encoded: PCM takes no time to encode.
        args.extend(mix.codec.iter().cloned());
        args.extend(mix.wrap.iter().cloned());
        args.extend(lasts);
        args.push(file.to_string_lossy().into_owned());
        scratch.0.push(file.clone());
        ffmpeg(&args, cancellation)?;
        sections::count_sound_whole();
        scratch.0.pop();
        return Ok(Made { file, used });
    }
    let pcm = scratch.file(kept, "f32");
    args.extend(lasts.iter().cloned());
    args.extend(strings(["-f", "f32le"]));
    args.push(pcm.to_string_lossy().into_owned());
    ffmpeg(&args, cancellation)?;
    let samples = kerf_core::program::samples(mix.duration).max(0) as u64;
    if std::fs::metadata(&pcm)?.len() != samples * BYTES {
        return Err(EngineError::msg("the mix is not as long as the program"));
    }
    let encoder = Encoder {
        pcm: &pcm,
        samples,
        codec: &mix.codec,
        kept,
        cancellation,
    };
    let (frames, pieces) = encoder.frames(busy)?;
    used.extend(pieces);
    // The frames as a stream, and then as the render's kind of file with each at its time.
    let stream = scratch.file(kept, "aac");
    std::fs::write(&stream, frames.concat())?;
    let last = frames.len() as u64 - 1;
    let rest = samples - (last - 1) * FRAME;
    let per = format!("/({SAMPLE_RATE}*TB)");
    let mut wrap = vec![
        // The first frame is before the sound: the file starts at nought, and
        // the picture with it.
        "-itsoffset".to_owned(),
        format!("-{:.6}", FRAME as f64 / f64::from(SAMPLE_RATE)),
    ];
    wrap.extend(strings(["-f", "aac", "-i"]));
    wrap.push(stream.to_string_lossy().into_owned());
    wrap.extend(strings(["-vn", "-c:a", "copy", "-bsf:a"]));
    wrap.push(format!(
        "setts=pts=(N-1)*{FRAME}{per}:dts=(N-1)*{FRAME}{per}:\
         duration=if(eq(N\\,{last})\\,{rest}\\,{FRAME}){per}"
    ));
    wrap.extend(mix.wrap.iter().cloned());
    wrap.extend(lasts);
    wrap.push(file.to_string_lossy().into_owned());
    scratch.0.push(file.clone());
    ffmpeg(&wrap, cancellation)?;
    // A sound a frame late is 21 ms out with its picture: the file is read back.
    if !timed(&file, frames.len(), rest) {
        return Err(EngineError::msg(
            "the sound's frames are not at their times",
        ));
    }
    scratch.0.pop();
    Ok(Made { file, used })
}

/// Whether the sound in `file` is `frames` packets a frame of AAC apart, the first before
/// nought, where a file says to skip it, and the last `rest` samples long: what an encoder
/// writing the file itself gives it.
fn timed(file: &Path, frames: usize, rest: u64) -> bool {
    let args = strings(["-v", "error", "-select_streams", "a:0", "-show_entries"])
        .into_iter()
        .chain(strings(["packet=pts,duration", "-of", "csv=p=0"]))
        .chain([file.to_string_lossy().into_owned()])
        .collect::<Vec<_>>();
    let Ok(output) = crate::ffmpeg::run_allow_failure(crate::ffmpeg::resolve_ffprobe_bin(), &args)
    else {
        return false;
    };
    let packets: Option<Vec<(i64, u64)>> = output
        .stdout
        .lines()
        .map(|line| {
            let mut fields = line.trim().split(',');
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect();
    packets.is_some_and(|packets| {
        packets.len() == frames
            && packets.iter().enumerate().all(|(at, (pts, duration))| {
                let whole = if at + 1 == frames { rest } else { FRAME };
                *pts == (at as i64 - 1) * FRAME as i64 && *duration == whole
            })
    })
}

/// A run of the program's frames of AAC, `from..to`, and how many before
/// `from` are encoded with it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Piece {
    from: u64,
    to: u64,
    lead: u64,
}

/// The pieces of a sound of `total` frames: [`PIECE`] frames each, and what
/// is left at the end with the last unless it is half a piece or more.
fn pieces(total: u64) -> Vec<Piece> {
    let mut starts: Vec<u64> = (0..total).step_by(PIECE as usize).collect();
    if starts.len() > 1 && total - starts[starts.len() - 1] < PIECE / 2 {
        starts.pop();
    }
    (0..starts.len())
        .map(|at| Piece {
            from: starts[at],
            to: starts.get(at + 1).copied().unwrap_or(total),
            lead: starts[at].min(LEAD),
        })
        .collect()
}

/// A piece's frames, each with its ADTS header: the lead and then its own.
#[derive(Debug, PartialEq, Eq)]
struct Packets {
    lead: Vec<Vec<u8>>,
    body: Vec<Vec<u8>>,
}

/// The frames of an ADTS stream; none if it is not one all through.
fn adts(stream: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut frames = Vec::new();
    let mut at = 0;
    while at < stream.len() {
        let header = stream.get(at..at + 7)?;
        if header[0] != 0xFF || header[1] & 0xF6 != 0xF0 {
            return None;
        }
        let length = (usize::from(header[3] & 3) << 11)
            | (usize::from(header[4]) << 3)
            | usize::from(header[5] >> 5);
        if length < 7 {
            return None;
        }
        frames.push(stream.get(at..at + length)?.to_vec());
        at += length;
    }
    Some(frames)
}

/// The window a frame ends on, as the frame after it was encoded to follow:
/// which of the four sequences, and which shape. None for a frame whose two
/// channels have windows of their own, where the second's is behind all the
/// first's sound, or that holds something else.
fn window(frame: &[u8]) -> Option<(u8, u8)> {
    // A header is nine bytes when it carries a check of the frame.
    let data = frame.get(if frame.get(1)? & 1 == 0 { 9 } else { 7 }..)?;
    let take = |at: usize, count: usize| -> Option<usize> {
        (at..at + count).try_fold(0, |bits, at| {
            let bit = data.get(at / 8)? >> (7 - at % 8) & 1;
            Some(bits << 1 | usize::from(bit))
        })
    };
    let pair =
        |sequence: usize, shape: usize| Some((take(sequence, 2)? as u8, take(shape, 1)? as u8));
    let mut at = 0;
    loop {
        // An element says what it is in three bits and which in four.
        match take(at, 3)? {
            // One channel: its gain, a bit kept back, then its window.
            0 => return pair(at + 16, at + 18),
            // Two that say they share a window say it next.
            1 if take(at + 7, 1)? == 1 => return pair(at + 9, at + 11),
            // Filling, which the encoder signs its name in every 256 frames: so many
            // bytes, and then the sound.
            6 => {
                let mut bytes = take(at + 3, 4)?;
                at += 7;
                if bytes == 15 {
                    bytes += take(at, 8)?.checked_sub(1)?;
                    at += 8;
                }
                at += bytes * 8;
            }
            _ => return None,
        }
    }
}

/// How many frames before the second piece's first the two are joined: the
/// fewest for which the frame before the join has the same window in both,
/// so that the frame after it, made to follow the one, follows the other as
/// well. None when no frame of the lead is such a frame.
fn seam(before: &Packets, after: &Packets) -> Option<usize> {
    (0..after.lead.len().min(before.body.len())).find(|back| {
        let ours = &before.body[before.body.len() - 1 - back];
        let theirs = &after.lead[after.lead.len() - 1 - back];
        ours == theirs || window(ours).is_some_and(|ours| Some(ours) == window(theirs))
    })
}

/// The frames of pieces laid end to end, each joined to the next where
/// [`seam`] says; none when two of them do not join.
fn joined(pieces: &[Packets]) -> Option<Vec<&[u8]>> {
    let mut frames: Vec<&[u8]> = Vec::new();
    let mut back = 0;
    for (at, piece) in pieces.iter().enumerate() {
        let next = match pieces.get(at + 1) {
            Some(next) => seam(piece, next)?,
            None => 0,
        };
        frames.extend(
            piece.lead[piece.lead.len() - back..]
                .iter()
                .map(Vec::as_slice),
        );
        frames.extend(
            piece.body[..piece.body.len() - next]
                .iter()
                .map(Vec::as_slice),
        );
        back = next;
    }
    Some(frames)
}

/// A kept piece's file: how many frames of lead and of its own, and the
/// frames.
fn written(packets: &Packets) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend((packets.lead.len() as u32).to_le_bytes());
    bytes.extend((packets.body.len() as u32).to_le_bytes());
    for frame in packets.lead.iter().chain(&packets.body) {
        bytes.extend(frame);
    }
    bytes
}

/// The piece a kept file holds, if it is `piece` whole.
fn read(bytes: &[u8], piece: &Piece) -> Option<Packets> {
    let count = |at: usize| -> Option<usize> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize)
    };
    let (lead, body) = (count(8)?, count(12)?);
    if bytes.get(..8)? != MAGIC || lead as u64 != piece.lead || body as u64 != piece.to - piece.from
    {
        return None;
    }
    let mut frames = adts(&bytes[16..])?;
    if frames.len() != lead + body {
        return None;
    }
    let body = frames.split_off(lead);
    Some(Packets { lead: frames, body })
}

/// The mix as samples in a file, and how its AAC is made.
struct Encoder<'a> {
    pcm: &'a Path,
    samples: u64,
    codec: &'a [String],
    kept: &'a Kept,
    cancellation: &'a CancellationToken,
}

impl Encoder<'_> {
    /// How many frames the sound is: one before it, and the last filled out.
    fn total(&self) -> u64 {
        self.samples.div_ceil(FRAME) + 1
    }

    /// The bytes of the mix a piece is encoded from: from its lead to
    /// [`AHEAD`] frames past its last, or to the end of the sound.
    fn span(&self, piece: &Piece) -> (u64, u64) {
        let from = (piece.from - piece.lead) * FRAME;
        let to = if piece.to == self.total() {
            self.samples
        } else {
            ((piece.to + AHEAD) * FRAME).min(self.samples)
        };
        (from * BYTES, to * BYTES)
    }

    /// The name a piece is kept by: the encoder, what it is told, and the
    /// samples it is given, wherever in a program they fall.
    fn name(&self, piece: &Piece) -> Result<String> {
        let mut hash = Sha256::new();
        hash.update(format!(
            "aac\n{}\n{}\n{}\n{LEAD} {AHEAD}\n{} {} {}\n",
            resolve_ffmpeg_bin(),
            ffmpeg_version(),
            self.codec.join("\n"),
            piece.lead,
            piece.to - piece.from,
            piece.to == self.total(),
        ));
        let (from, to) = self.span(piece);
        let mut file = std::fs::File::open(self.pcm)?;
        file.seek(SeekFrom::Start(from))?;
        let mut left = (to - from) as usize;
        let mut buffer = vec![0u8; 1 << 20];
        while left > 0 {
            let read = left.min(buffer.len());
            file.read_exact(&mut buffer[..read])?;
            hash.update(&buffer[..read]);
            left -= read;
        }
        Ok(named(hash))
    }

    /// A piece encoded by an encoder of its own, started [`LEAD`] frames
    /// before it.
    fn encode(&self, piece: &Piece) -> Result<Packets> {
        let making = self.kept.making("aac");
        let (from, _) = self.span(piece);
        let (lead, count) = (piece.lead as usize, (piece.to - piece.from) as usize);
        let last = piece.to == self.total();
        let mut args = Vec::new();
        if from > 0 {
            args.extend(["-skip_initial_bytes".to_owned(), from.to_string()]);
        }
        args.extend(raw_input());
        args.push(self.pcm.to_string_lossy().into_owned());
        args.push("-vn".to_owned());
        if !last {
            let samples = (piece.lead + piece.to - piece.from + AHEAD) * FRAME;
            args.extend(["-af".to_owned(), format!("atrim=end_sample={samples}")]);
        }
        if self.codec.is_empty() {
            args.extend(strings(["-c:a", "aac"]));
        }
        args.extend(self.codec.iter().cloned());
        args.extend(strings(["-f", "adts"]));
        args.push(making.to_string_lossy().into_owned());
        let encoded = ffmpeg(&args, self.cancellation).and_then(|()| Ok(std::fs::read(&making)?));
        let _ = std::fs::remove_file(&making);
        let mut frames = adts(&encoded?)
            .ok_or_else(|| EngineError::msg("the encoder's sound is not frames of AAC"))?;
        // The frames are the program's from the lead's first, so many and no fewer.
        if (last && frames.len() != lead + count) || frames.len() < lead + count {
            return Err(EngineError::msg(format!(
                "a piece of sound came to {} frames, not {}",
                frames.len(),
                lead + count
            )));
        }
        frames.truncate(lead + count);
        let body = frames.split_off(lead);
        Ok(Packets { lead: frames, body })
    }

    fn keep(&self, packets: &Packets, name: &str) -> Result<PathBuf> {
        let making = self.kept.making("aacs");
        std::fs::write(&making, written(packets))?;
        Ok(self.kept.keep(&making, name, "aacs")?)
    }

    /// Every frame of the sound in order, and the kept pieces they are in.
    /// The pieces `kept` has are used as they are and the rest encoded, at
    /// once; when none is kept and the picture is `busy`, or the pieces do
    /// not join, one encoder makes the whole sound and its frames are kept
    /// as the pieces.
    fn frames(&self, busy: bool) -> Result<(Vec<Vec<u8>>, Vec<PathBuf>)> {
        let plan = pieces(self.total());
        let names = at_once(&plan, self.cancellation, |piece| self.name(piece))?;
        let mut have: Vec<Option<(Packets, PathBuf)>> = plan
            .iter()
            .zip(&names)
            .map(|(piece, name)| {
                let file = self.kept.find(name, "aacs")?;
                Some((read(&std::fs::read(&file).ok()?, piece)?, file))
            })
            .collect();
        // One piece is the whole sound, from an encoder that starts where it does.
        let whole = plan.len() == 1 || (busy && have.iter().all(Option::is_none));
        if !whole {
            let missing: Vec<usize> = (0..plan.len()).filter(|at| have[*at].is_none()).collect();
            let encoded = at_once(&missing, self.cancellation, |at| {
                let packets = self.encode(&plan[*at])?;
                let file = self.keep(&packets, &names[*at])?;
                Ok((packets, file))
            })?;
            sections::count_sound_pieces(plan.len() - missing.len(), missing.len());
            for (at, piece) in missing.into_iter().zip(encoded) {
                have[at] = Some(piece);
            }
            let (packets, files): (Vec<Packets>, Vec<PathBuf>) = have.into_iter().flatten().unzip();
            match joined(&packets) {
                Some(frames) if frames.len() as u64 == self.total() => {
                    let frames = frames.into_iter().map(<[u8]>::to_vec).collect();
                    return Ok((frames, files));
                }
                // Two pieces with no frame to join at: one encoder has no joins.
                _ => sections::count_sound_unjoined(),
            }
        }
        let all = Piece {
            from: 0,
            to: self.total(),
            lead: 0,
        };
        let frames = self.encode(&all)?.body;
        sections::count_sound_whole();
        sections::count_sound_pieces(0, plan.len());
        // Its frames are the pieces, each with the frames before it as its lead.
        let mut files = Vec::new();
        for (piece, name) in plan.iter().zip(&names) {
            let (lead, from, to) = (
                (piece.from - piece.lead) as usize,
                piece.from as usize,
                piece.to as usize,
            );
            let packets = Packets {
                lead: frames[lead..from].to_vec(),
                body: frames[from..to].to_vec(),
            };
            files.push(self.keep(&packets, name)?);
        }
        Ok((frames, files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of AAC with a seven-byte header and `payload` after it.
    fn frame(payload: &[u8]) -> Vec<u8> {
        let length = 7 + payload.len();
        let mut frame = vec![
            0xFF,
            0xF1,
            0x4C,
            0x80 | (length >> 11) as u8,
            (length >> 3) as u8,
            ((length & 7) << 5) as u8 | 0x1F,
            0xFC,
        ];
        frame.extend(payload);
        frame
    }

    /// A frame of two channels sharing a window of `sequence` and `shape`,
    /// unlike any other with the same window by `tag`.
    fn shared(sequence: u8, shape: u8, tag: u8) -> Vec<u8> {
        // 001 tttt 1 | 0 ss w ....
        frame(&[0x21, (sequence << 5) | (shape << 4), tag, 0])
    }

    /// One of two channels with windows of their own.
    fn apart(tag: u8) -> Vec<u8> {
        frame(&[0x20, 0, tag, 0])
    }

    #[test]
    fn pieces_join_where_the_frame_before_has_the_same_window_in_both() {
        let piece = |lead: Vec<Vec<u8>>, body: Vec<Vec<u8>>| Packets { lead, body };
        // The frame before the second piece's first is a long window in both.
        let first = piece(
            vec![],
            vec![shared(0, 1, 1), shared(0, 1, 2), shared(0, 1, 3)],
        );
        let second = piece(
            vec![shared(2, 0, 10), shared(3, 1, 11), shared(0, 1, 12)],
            vec![shared(0, 1, 13), shared(0, 1, 14)],
        );
        assert_eq!(seam(&first, &second), Some(0));
        let both = [first, second];
        let frames = joined(&both).unwrap();
        assert_eq!(frames.len(), 5);
        assert_eq!(frames[2], shared(0, 1, 3));
        assert_eq!(frames[3], shared(0, 1, 13));
        // The first piece's last frame has a window that cannot be read: the join is one
        // frame earlier, and the second piece's lead takes that frame's place.
        let first = piece(vec![], vec![shared(0, 1, 1), shared(0, 1, 2), apart(3)]);
        let second = piece(
            vec![shared(2, 0, 10), shared(0, 1, 11), shared(1, 1, 12)],
            vec![shared(2, 1, 13), shared(3, 1, 14)],
        );
        assert_eq!(seam(&first, &second), Some(1));
        let both = [first, second];
        let frames = joined(&both).unwrap();
        assert_eq!(frames.len(), 5);
        assert_eq!(
            [frames[1], frames[2], frames[3]],
            [shared(0, 1, 2), shared(1, 1, 12), shared(2, 1, 13)]
        );
        // An encoder just started has short windows where the other has long ones, and
        // the windows' shapes differ where the sequences agree: no join.
        let first = piece(
            vec![],
            vec![shared(0, 1, 1), shared(0, 1, 2), shared(0, 1, 3)],
        );
        let second = piece(
            vec![shared(0, 0, 11), shared(2, 1, 12)],
            vec![shared(3, 1, 13)],
        );
        assert_eq!(seam(&first, &second), None);
        assert!(joined(&[first, second]).is_none());
        // The same frame is the same window, read or not: one encoder's frames cut in two.
        let first = piece(vec![], vec![shared(0, 1, 1), apart(2)]);
        let second = piece(vec![shared(0, 1, 1), apart(2)], vec![apart(3)]);
        assert_eq!(seam(&first, &second), Some(0));
        // A piece with no lead joins nothing before it.
        let first = piece(vec![], vec![shared(0, 1, 1)]);
        let second = piece(vec![], vec![shared(0, 1, 2)]);
        assert_eq!(seam(&first, &second), None);
    }

    /// A minute of sound an encoder has not the bits for: notes that start and stop, a
    /// tone that slides, a hiss, and bursts of noise, another in each channel.
    fn sounds(samples: usize) -> Vec<u8> {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut noise = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        };
        let mut bytes = Vec::with_capacity(samples * 8);
        for at in 0..samples {
            let t = at as f32 / SAMPLE_RATE as f32;
            let turn = std::f32::consts::TAU * t;
            for (note, every, held, burst) in [(220.0, 0.41, 0.3, 1.3), (277.0, 0.53, 0.35, 0.9)] {
                let step = (t * 2.7).floor() % 5.0;
                let mut level = 0.12 * (turn * (1900.0 + 600.0 * step)).sin() + 0.05 * noise();
                if t % every < held {
                    level += 0.25 * (turn * (note + 150.0 * step)).sin();
                }
                if t % burst < 0.07 {
                    level += 0.3 * noise();
                }
                bytes.extend(level.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn an_encoder_started_before_a_piece_joins_and_one_started_at_it_does_not() {
        if hidden_command(resolve_ffmpeg_bin())
            .arg("-version")
            .output()
            .is_err()
        {
            eprintln!("skipping: ffmpeg is not installed");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let kept = Kept::at(&dir.path().join("kept"), "stamp", 1 << 30).unwrap();
        let pcm = dir.path().join("mix.f32");
        let samples = 60 * SAMPLE_RATE as usize;
        let mut sound = sounds(samples);
        std::fs::write(&pcm, &sound).unwrap();
        let cancellation = CancellationToken::new();
        let encoder = Encoder {
            pcm: &pcm,
            samples: samples as u64,
            codec: &[],
            kept: &kept,
            cancellation: &cancellation,
        };
        // 2,813 frames of sound and the one before it: three pieces.
        let total = encoder.total();
        assert_eq!(total, 2814);
        let plan = pieces(total);
        assert_eq!(plan.len(), 3);
        let whole = encoder
            .encode(&Piece {
                from: 0,
                to: total,
                lead: 0,
            })
            .unwrap()
            .body;
        assert_eq!(whole.len() as u64, total);
        // The pieces one encoder's frames are, each with the frames before it as its lead.
        let cut = |piece: &Piece| Packets {
            lead: whole[(piece.from - piece.lead) as usize..piece.from as usize].to_vec(),
            body: whole[piece.from as usize..piece.to as usize].to_vec(),
        };
        // The second piece from an encoder of its own is other frames, and joins the first
        // and the third as they are.
        let own = encoder.encode(&plan[1]).unwrap();
        assert_eq!((own.lead.len(), own.body.len()), (16, 1024));
        assert!(
            own.body != cut(&plan[1]).body,
            "an encoder just started is the one running"
        );
        let mixed = [cut(&plan[0]), own, cut(&plan[2])];
        assert_eq!(joined(&mixed).expect("the pieces join").len() as u64, total);
        // Started two frames before the piece, its frame before the piece is the short
        // windows an encoder starts with, where the running encoder has a long one.
        let late = encoder
            .encode(&Piece {
                lead: 2,
                ..plan[1].clone()
            })
            .unwrap();
        assert_eq!(late.lead.len(), 2);
        assert_eq!(seam(&mixed[0], &late), None);
        assert!(joined(&[cut(&plan[0]), late]).is_none());

        // With the picture busy and nothing kept, one encoder makes the sound; its frames
        // are then the kept pieces, and the sound again is the same frames.
        let (first, files) = encoder.frames(true).unwrap();
        assert!(first == whole);
        assert_eq!(files.len(), 3);
        let (again, _) = encoder.frames(false).unwrap();
        assert!(again == whole);
        // Half a second of the second piece silenced: that piece is encoded again, alone,
        // and the frames of the other two are the frames they were.
        let (from, to) = (1500 * FRAME as usize * 8, 1524 * FRAME as usize * 8);
        sound[from..to].fill(0);
        std::fs::write(&pcm, &sound).unwrap();
        let (names, before) = (plan.iter().map(|piece| encoder.name(piece).unwrap()), files);
        let found: Vec<bool> = names
            .map(|name| kept.find(&name, "aacs").is_some())
            .collect();
        assert_eq!(found, [true, false, true]);
        let (edited, files) = encoder.frames(false).unwrap();
        assert_eq!(edited.len(), whole.len());
        assert!(edited[..1000] == whole[..1000] && edited[2048..] == whole[2048..]);
        assert!(edited[1024..2048] != whole[1024..2048]);
        assert_eq!((&files[0], &files[2]), (&before[0], &before[2]));
        assert_ne!(files[1], before[1]);
    }
}
