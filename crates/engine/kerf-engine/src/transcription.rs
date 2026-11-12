//! Bounded local transcription and listening observations.
//!
//! The engine owns the sidecar boundary, but not the resulting transcript as
//! project state.  Audio and JSON output live in the project-local derived
//! cache and are published only after the sidecar has completed successfully.

use crate::derivatives::{CacheEntry, DerivativeCache, SourceIdentity};
use crate::error::{EngineError, Result};
use crate::ffmpeg::resolve_ffmpeg_bin;
use crate::probe::info;
use crate::sidecar::{self, CancellationToken};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Schema version for derived transcript JSON.
pub const TRANSCRIPTION_SCHEMA_VERSION: u8 = 2;
/// Audio sample rate passed to whisper.cpp.
pub const TRANSCRIPTION_SAMPLE_RATE: u32 = 16_000;
/// Maximum range accepted by a transcription request.
pub const MAX_TRANSCRIPTION_RANGE_SECONDS: f64 = 15.0 * 60.0;
/// Maximum range accepted by the selected-range listening operation.
pub const MAX_LISTEN_RANGE_SECONDS: f64 = 2.0 * 60.0;

const MAX_TRANSCRIPT_BYTES: u64 = 16 * 1024 * 1024;
/// How far a part transcribed beside one already heard is heard into it,
/// so that a word across their edge is heard whole by one of them and the
/// two can be joined where nobody is speaking.
const SEAM_OVERLAP_SECONDS: f64 = 5.0;
/// Two transcripts are joined in the middle of the quietest stretch this
/// long of what both heard, measured in windows of [`SEAM_WINDOW_SECONDS`].
const SEAM_QUIET_SECONDS: f64 = 0.2;
const SEAM_WINDOW_SECONDS: f64 = 0.02;
/// Ranges closer than this meet.
const RANGE_EPSILON: f64 = 1e-6;
/// How much quieter than the sound around it a moment has to be to be a
/// pause in the speech, as a share of that sound's energy: 25 dB down.
const PAUSE_ENERGY_SHARE: f64 = 0.003;
/// How far out a word's time may be: whisper times a word to within a
/// second or so, so the words two transcripts both heard are looked for
/// this far either side of what both were given.
const WORD_TIME_SLACK_SECONDS: f64 = 1.5;
/// How long the sound between two words stays quiet for it to be a pause
/// between them, and how quiet beside the loudest of them: a hundredth of
/// its energy, 20 dB down. The gap a stopped consonant leaves inside a word
/// is shorter.
const WORD_PAUSE_SECONDS: f64 = 0.16;
const WORD_PAUSE_SHARE: f64 = 0.01;
/// How long a word's sound may trail on past where alignment ends it.
const WORD_TAIL_SECONDS: f64 = 0.5;
const DEFAULT_WHISPER_BACKEND_VERSION: &str = "whisper.cpp/unknown";

/// A bounded project-time range used by transcription and listening.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    pub start: f64,
    pub end: f64,
}

impl TimeRange {
    /// Construct a finite, positive range.  Maximum lengths are operation
    /// specific and are checked by the operation that consumes the range.
    pub fn new(start: f64, end: f64) -> Result<Self> {
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
            return Err(EngineError::msg(
                "transcription range requires finite 0 <= start < end",
            ));
        }
        Ok(Self { start, end })
    }

    /// Return the range duration in seconds.
    pub fn duration(self) -> f64 {
        self.end - self.start
    }
}

/// Transcription processing mode.  Provider processing is represented so an
/// explicit request can be rejected without ever uploading media.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TranscriptionBackend {
    /// Use a locally installed whisper.cpp CLI and a local GGML model.
    #[default]
    LocalWhisperCpp,
    /// A future provider adapter requested explicitly by the caller.
    ExplicitProvider { name: String },
}

/// Options for one bounded transcription observation.
#[derive(Clone, Debug)]
pub struct TranscriptionOptions {
    pub range: TimeRange,
    pub backend: TranscriptionBackend,
    pub language: Option<String>,
    pub model: Option<PathBuf>,
    pub model_version: Option<String>,
    pub backend_version: Option<String>,
    /// Explicit sidecar path, primarily useful for packaging and tests.
    pub binary: Option<PathBuf>,
}

impl TranscriptionOptions {
    /// Create local whisper.cpp options for a bounded range.
    pub fn new(range: TimeRange) -> Self {
        Self {
            range,
            backend: TranscriptionBackend::LocalWhisperCpp,
            language: None,
            model: None,
            model_version: None,
            backend_version: None,
            binary: None,
        }
    }
}

/// Which sidecar stage emitted progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptionStage {
    PrepareAudio,
    Transcribe,
}

/// Bounded progress emitted while preparing audio or running whisper.cpp.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionProgress {
    pub stage: TranscriptionStage,
    pub progress: f64,
}

/// One word with a source-relative timestamp and optional sidecar confidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptWord {
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// One caption-like cue containing word-level timing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub words: Vec<TranscriptWord>,
}

/// Typed transcript data stored as a regenerable derived artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub schema_version: u8,
    pub range: TimeRange,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub text: String,
    pub cues: Vec<TranscriptCue>,
}

/// Transcription result returned to an adapter.  `audio_path` is populated by
/// `listen` only; audio bytes are never inlined or persisted in project state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionResult {
    pub key: String,
    #[serde(flatten)]
    pub transcript: Transcript,
    pub backend: String,
    pub backend_version: String,
    pub model: String,
    pub model_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_path: Option<String>,
    pub cached: bool,
}

#[derive(Clone, Debug)]
struct WhisperConfig {
    binary: PathBuf,
    model: PathBuf,
    model_identity: SourceIdentity,
    binary_identity: Value,
    backend_version: String,
    model_version: String,
    /// The alignment heads whisper.cpp has for the model, when it and the
    /// model are known to take them: words are then timed by them.
    alignment: Option<Alignment>,
}

/// How whisper.cpp is asked to time words by where the model attended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Alignment {
    /// The model's alignment heads, as `-dtw` names them.
    preset: &'static str,
    /// Whether flash attention, which it cannot run with, is there to turn
    /// off.
    without_flash_attention: bool,
}

/// Extract and parse a whisper.cpp JSON result without running a sidecar.
///
/// Timestamps emitted by whisper.cpp may be segment timestamps, token
/// timestamps, or millisecond offsets depending on the CLI flags and version.
/// The parser accepts all of those forms and normalizes them to source seconds.
pub fn parse_whisper_json(output: &str, range: TimeRange) -> Result<Transcript> {
    let value: Value = serde_json::from_str(output)?;
    parse_whisper_value(&value, range)
}

/// Transcribe a bounded range with the configured local whisper.cpp backend.
pub fn transcribe_cached<F>(
    src: &str,
    cache: &DerivativeCache,
    options: &TranscriptionOptions,
    cancellation: &CancellationToken,
    on_progress: F,
) -> Result<TranscriptionResult>
where
    F: FnMut(TranscriptionProgress),
{
    transcribe_cached_inner(src, cache, options, cancellation, on_progress, false)
}

/// What the transcripts already made of `src` as it is now heard, by any
/// model. Nothing is transcribed.
///
/// Transcripts made the same way that overlap or meet come as one, joined
/// as a request across them would be: a moment heard twice is given once,
/// so whoever reads the words (a check for cuts in words, captions made of
/// a clip's speech) finds each of them once.
pub fn cached_transcripts(src: &str, cache: &DerivativeCache) -> Result<Vec<Transcript>> {
    // Made the same way: the same options, whatever the range.
    let mut groups: Vec<(Value, Vec<Piece>)> = Vec::new();
    for entry in cache.source_entries(Path::new(src), "transcription")? {
        if !entry.is_complete(&["transcript.json"]) {
            continue;
        }
        let Ok(transcript) = entry.read_json::<Transcript>("transcript.json") else {
            continue;
        };
        if transcript.schema_version != TRANSCRIPTION_SCHEMA_VERSION {
            continue;
        }
        let piece = Piece {
            transcript,
            audio: entry.path("audio.wav"),
            edges: entry.read_json("edges.json").unwrap_or_default(),
        };
        let made = options_but_range(entry.options());
        match groups.iter_mut().find(|(options, _)| *options == made) {
            Some((_, pieces)) => pieces.push(piece),
            None => groups.push((made, vec![piece])),
        }
    }
    let mut transcripts = Vec::new();
    for (_, mut pieces) in groups {
        pieces.sort_by(|a, b| {
            a.transcript
                .range
                .start
                .total_cmp(&b.transcript.range.start)
        });
        // Each stretch the pieces cover without a break.
        let mut stretches: Vec<TimeRange> = Vec::new();
        for range in ranges(&pieces) {
            match stretches.last_mut() {
                Some(stretch) if range.start <= stretch.end + RANGE_EPSILON => {
                    stretch.end = stretch.end.max(range.end);
                }
                _ => stretches.push(range),
            }
        }
        transcripts.extend(
            stretches
                .into_iter()
                .filter_map(|stretch| stitched(stretch, &pieces)),
        );
    }
    Ok(transcripts)
}

/// The options a transcript was made with, but for its range.
fn options_but_range(options: &Value) -> Value {
    let mut options = options.clone();
    if let Some(options) = options.as_object_mut() {
        options.remove("range");
    }
    options
}

/// Prepare and transcribe a bounded range for a listening observation.
///
/// The same cached transcript as `transcribe_cached` is reused.  The result
/// additionally exposes a project-local WAV path so a human-facing adapter can
/// play the exact selected range without uploading it or embedding its bytes.
pub fn listen_cached<F>(
    src: &str,
    cache: &DerivativeCache,
    options: &TranscriptionOptions,
    cancellation: &CancellationToken,
    on_progress: F,
) -> Result<TranscriptionResult>
where
    F: FnMut(TranscriptionProgress),
{
    transcribe_cached_inner(src, cache, options, cancellation, on_progress, true)
}

fn transcribe_cached_inner<F>(
    src: &str,
    cache: &DerivativeCache,
    options: &TranscriptionOptions,
    cancellation: &CancellationToken,
    mut on_progress: F,
    include_audio: bool,
) -> Result<TranscriptionResult>
where
    F: FnMut(TranscriptionProgress),
{
    ensure_not_cancelled(cancellation)?;
    validate_operation_range(options.range, include_audio)?;
    let config = resolve_whisper_config(options)?;
    let metadata = info(src)?;
    if metadata.has_audio == Some(false) {
        return Err(EngineError::msg("source has no audio stream"));
    }
    if let Some(duration) = metadata.duration {
        if !duration.is_finite() || duration <= 0.0 || options.range.end > duration + 1e-6 {
            return Err(EngineError::msg(format!(
                "transcription range must be inside source duration {duration:.3}s"
            )));
        }
    }

    let key_options = cache_options(options, &config)?;
    let entry = cache.source_entry(Path::new(src), "transcription", &key_options)?;
    let transcript_complete = entry.is_complete(&["transcript.json"]);
    let audio_complete = entry.is_complete(&["transcript.json", "audio.wav"]);
    let complete = if include_audio {
        audio_complete
    } else {
        transcript_complete
    };

    if complete {
        let transcript = read_cached_transcript(&entry, options.range)?;
        on_progress(TranscriptionProgress {
            stage: TranscriptionStage::Transcribe,
            progress: 1.0,
        });
        return Ok(result_from_cache(
            &entry,
            transcript,
            &config,
            include_audio,
            true,
        ));
    }

    // What was heard before answers what it covers: only the parts no
    // transcript of this file by this model covers are transcribed now. A
    // listening observation also wants the range's own sound, so it is made
    // whole.
    if !include_audio {
        let source_end = metadata.duration.filter(|duration| duration.is_finite());
        let mut pieces = cached_pieces(src, cache, entry.options())?;
        let missing = uncovered(options.range, &ranges(&pieces));
        let cached = missing.is_empty();
        for gap in missing {
            let (range, edges) = heard_beside(gap, &pieces, source_end);
            let piece_options = TranscriptionOptions {
                range,
                ..options.clone()
            };
            let piece_entry = cache.source_entry(
                Path::new(src),
                "transcription",
                &cache_options(&piece_options, &config)?,
            )?;
            if edges != Edges::default() {
                piece_entry.write_json("edges.json", &edges)?;
            }
            let transcript = transcribe_piece(
                src,
                &piece_entry,
                range,
                options.language.as_deref(),
                &config,
                cancellation,
                &mut on_progress,
            )?;
            pieces.push(Piece {
                audio: piece_entry.path("audio.wav"),
                transcript,
                edges,
            });
        }
        if cached {
            on_progress(TranscriptionProgress {
                stage: TranscriptionStage::Transcribe,
                progress: 1.0,
            });
        }
        let transcript = stitched(options.range, &pieces).ok_or_else(|| {
            EngineError::msg("the transcripts made do not cover the range asked for")
        })?;
        // Nothing was made for this very range: leave no empty entry behind
        // for each range asked for.
        let _ = fs::remove_dir(&entry.directory);
        return Ok(result_from_cache(
            &entry, transcript, &config, false, cached,
        ));
    }

    let transcript = transcribe_piece(
        src,
        &entry,
        options.range,
        options.language.as_deref(),
        &config,
        cancellation,
        &mut on_progress,
    )?;
    Ok(result_from_cache(
        &entry,
        transcript,
        &config,
        include_audio,
        false,
    ))
}

/// Transcribe `range` of `src` into `entry`, whose key names that range:
/// its sound, then what whisper hears in it. What the entry already has is
/// kept.
fn transcribe_piece(
    src: &str,
    entry: &CacheEntry,
    range: TimeRange,
    language: Option<&str>,
    config: &WhisperConfig,
    cancellation: &CancellationToken,
    on_progress: &mut impl FnMut(TranscriptionProgress),
) -> Result<Transcript> {
    let transcript_complete = entry.is_complete(&["transcript.json"]);
    let audio_complete = entry.is_complete(&["transcript.json", "audio.wav"]);
    let audio_path = entry.path("audio.wav");
    if !audio_complete {
        on_progress(TranscriptionProgress {
            stage: TranscriptionStage::PrepareAudio,
            progress: 0.0,
        });
        extract_audio_range(src, range, &audio_path, cancellation, on_progress)?;
        ensure_not_cancelled(cancellation)?;
        on_progress(TranscriptionProgress {
            stage: TranscriptionStage::PrepareAudio,
            progress: 1.0,
        });
    }

    let transcript = if transcript_complete {
        read_cached_transcript(entry, range)?
    } else {
        on_progress(TranscriptionProgress {
            stage: TranscriptionStage::Transcribe,
            progress: 0.0,
        });
        let output_base = entry.path("transcript");
        let args = whisper_args(config, &audio_path, &output_base, language);
        run_sidecar(
            &config.binary,
            &args,
            cancellation,
            TranscriptionStage::Transcribe,
            on_progress,
        )?;
        ensure_not_cancelled(cancellation)?;
        let output_path = output_base.with_extension("json");
        let output = read_bounded_text(&output_path, MAX_TRANSCRIPT_BYTES)?;
        let mut transcript = parse_whisper_json(&output, range)?;
        if config.alignment.is_some() {
            settle_words(&mut transcript, &audio_path);
        }
        ensure_not_cancelled(cancellation)?;
        entry.write_json("transcript.json", &transcript)?;
        transcript
    };

    ensure_not_cancelled(cancellation)?;
    let required = vec!["transcript.json", "audio.wav"];
    entry.mark_complete(&required)?;
    Ok(transcript)
}

/// A transcript in the cache, and where the sound it was made from is kept.
struct Piece {
    transcript: Transcript,
    audio: PathBuf,
    edges: Edges,
}

/// Whether a transcript's range was made to start or end in a pause of the
/// speech, so that no word is cut there: kept beside it as `edges.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Edges {
    starts_in_pause: bool,
    ends_in_pause: bool,
}

fn ranges(pieces: &[Piece]) -> Vec<TimeRange> {
    pieces.iter().map(|piece| piece.transcript.range).collect()
}

/// The transcripts of `src` as it is now that were made as `key_options`
/// asks, of whatever range.
fn cached_pieces(src: &str, cache: &DerivativeCache, key_options: &Value) -> Result<Vec<Piece>> {
    let wanted = options_but_range(key_options);
    let mut pieces = Vec::new();
    for entry in cache.source_entries(Path::new(src), "transcription")? {
        if options_but_range(entry.options()) != wanted || !entry.is_complete(&["transcript.json"])
        {
            continue;
        }
        let Ok(transcript) = entry.read_json::<Transcript>("transcript.json") else {
            continue;
        };
        if transcript.schema_version == TRANSCRIPTION_SCHEMA_VERSION {
            pieces.push(Piece {
                transcript,
                audio: entry.path("audio.wav"),
                edges: entry.read_json("edges.json").unwrap_or_default(),
            });
        }
    }
    Ok(pieces)
}

/// The parts of `range` that none of `heard` covers, in order.
fn uncovered(range: TimeRange, heard: &[TimeRange]) -> Vec<TimeRange> {
    let mut heard: Vec<TimeRange> = heard.to_vec();
    heard.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut gaps = Vec::new();
    let mut at = range.start;
    for piece in heard {
        if piece.end <= at + RANGE_EPSILON {
            continue;
        }
        if piece.start >= range.end - RANGE_EPSILON {
            break;
        }
        if piece.start > at + RANGE_EPSILON {
            gaps.push(TimeRange {
                start: at,
                end: piece.start,
            });
        }
        at = piece.end;
    }
    if at < range.end - RANGE_EPSILON {
        gaps.push(TimeRange {
            start: at,
            end: range.end,
        });
    }
    gaps
}

/// The range to transcribe for `gap`, a part nothing has heard, and how it
/// meets what `heard` covers on either side of it.
///
/// A range that stopped at the gap's own edge would cut the word spoken
/// across it. So the gap is taken into the [`SEAM_OVERLAP_SECONDS`] its
/// neighbour heard, as far as the pause nearest the gap in that sound: the
/// new transcript starts (or ends) where nobody is speaking, and the two
/// meet there. Where the neighbour's sound has no pause, or is not kept,
/// the gap is taken the whole way in and the two are joined by what both
/// heard (see [`join`]). Where nothing was heard beside it, the gap is
/// transcribed as it is.
fn heard_beside(gap: TimeRange, heard: &[Piece], source_end: Option<f64>) -> (TimeRange, Edges) {
    let mut range = gap;
    let mut edges = Edges::default();
    let before = heard
        .iter()
        .filter(|piece| {
            let piece = piece.transcript.range;
            piece.start < gap.start - RANGE_EPSILON && piece.end >= gap.start - RANGE_EPSILON
        })
        .min_by(|a, b| {
            a.transcript
                .range
                .start
                .total_cmp(&b.transcript.range.start)
        });
    if let Some(piece) = before {
        let start = piece.transcript.range.start;
        let from = (gap.start - SEAM_OVERLAP_SECONDS).max(start);
        range.start = from;
        // The last pause before the gap, if there is one.
        if let Some((at, true)) = quietest(&piece.audio, from - start, gap.start - start, true) {
            range.start = from + at;
            edges.starts_in_pause = true;
        }
    }
    let after = heard
        .iter()
        .filter(|piece| {
            let piece = piece.transcript.range;
            piece.start <= gap.end + RANGE_EPSILON && piece.end > gap.end + RANGE_EPSILON
        })
        .max_by(|a, b| a.transcript.range.end.total_cmp(&b.transcript.range.end));
    if let Some(piece) = after {
        let (start, end) = (piece.transcript.range.start, piece.transcript.range.end);
        let to = (gap.end + SEAM_OVERLAP_SECONDS)
            .min(end)
            .min(source_end.unwrap_or(f64::INFINITY))
            .max(gap.end);
        range.end = to;
        // The first pause after the gap.
        if let Some((at, true)) = quietest(&piece.audio, gap.end - start, to - start, false) {
            range.end = gap.end + at;
            edges.ends_in_pause = true;
        }
    }
    (range, edges)
}

/// The transcript of `range` made of `pieces`. At the range's own edges a
/// word belongs to the range its middle is in; where two pieces overlap
/// they are joined at a word both heard (see [`join`]). `None` when the
/// pieces leave part of the range unheard.
fn stitched(range: TimeRange, pieces: &[Piece]) -> Option<Transcript> {
    // From where the range starts, the piece that has been hearing longest
    // there; then the next to start, where it overlaps the end of that one
    // or takes over where it stops; and so on. A moment is then taken from
    // a piece it is well inside of, not from the edge of one that happens
    // to reach it.
    let covers = |piece: &&Piece, at: f64| {
        piece.transcript.range.start <= at + RANGE_EPSILON
            && piece.transcript.range.end > at + RANGE_EPSILON
    };
    let earliest = |a: &&Piece, b: &&Piece| {
        let (a, b) = (a.transcript.range, b.transcript.range);
        a.start.total_cmp(&b.start).then(b.end.total_cmp(&a.end))
    };
    let mut chain: Vec<&Piece> = vec![pieces
        .iter()
        .filter(|piece| covers(piece, range.start))
        .min_by(earliest)?];
    loop {
        let current = chain[chain.len() - 1].transcript.range;
        let next = pieces
            .iter()
            .filter(|piece| {
                let next = piece.transcript.range;
                next.start > current.start
                    && next.start <= current.end + RANGE_EPSILON
                    && next.end > current.end + RANGE_EPSILON
                    && next.start < range.end - RANGE_EPSILON
            })
            .min_by(earliest);
        match next {
            Some(next) => chain.push(next),
            None if current.end >= range.end - RANGE_EPSILON => break,
            None => return None,
        }
    }
    // Each piece's share, as the words it gives, counted through its cues:
    // from the range's start, between joins, to the range's end.
    let middle = |word: &TranscriptWord| (word.start + word.end) / 2.0;
    let words: Vec<Vec<&TranscriptWord>> = chain
        .iter()
        .map(|piece| {
            piece
                .transcript
                .cues
                .iter()
                .flat_map(|cue| &cue.words)
                .collect()
        })
        .collect();
    let mut shares: Vec<(usize, usize)> = words.iter().map(|words| (0, words.len())).collect();
    shares[0].0 = words[0]
        .iter()
        .position(|word| middle(word) >= range.start)
        .unwrap_or(words[0].len());
    let last = chain.len() - 1;
    shares[last].1 = words[last]
        .iter()
        .rposition(|word| middle(word) <= range.end)
        .map_or(0, |index| index + 1);
    for index in 0..last {
        let (end, first) = join(
            chain[index],
            chain[index + 1],
            &words[index],
            &words[index + 1],
        );
        shares[index].1 = shares[index].1.min(end);
        shares[index + 1].0 = shares[index + 1].0.max(first);
    }
    // A join puts the words of one piece before those of the next, and
    // their times follow: whisper draws the last word before a pause out
    // across it, past where the next piece's first word starts.
    let upto = |index: usize| {
        (index + 1..chain.len())
            .find(|next| shares[*next].0 < shares[*next].1)
            .map_or(range.end, |next| {
                words[next][shares[next].0]
                    .start
                    .clamp(range.start, range.end)
            })
    };
    let mut cues = Vec::new();
    for (at, (piece, (first, end))) in chain.iter().zip(&shares).enumerate() {
        let upto = upto(at);
        let mut index = 0;
        for cue in &piece.transcript.cues {
            let own = index..index + cue.words.len();
            index = own.end;
            let kept: Vec<TranscriptWord> = cue
                .words
                .iter()
                .zip(own)
                .filter(|(_, index)| (*first..*end).contains(index))
                .map(|(word, _)| TranscriptWord {
                    start: word.start.clamp(range.start, upto),
                    end: word.end.clamp(range.start, upto),
                    ..word.clone()
                })
                .collect();
            let (Some(first_word), Some(last_word)) = (kept.first(), kept.last()) else {
                continue;
            };
            let text = if kept.len() == cue.words.len() {
                cue.text.clone()
            } else {
                kept.iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            cues.push(TranscriptCue {
                start: cue.start.clamp(range.start, upto).min(first_word.start),
                end: cue.end.clamp(range.start, upto).max(last_word.end),
                text,
                words: kept,
            });
        }
    }
    // Whisper's times are loose enough for two pieces' cues to cross.
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    let text = cues
        .iter()
        .map(|cue| cue.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Some(Transcript {
        schema_version: TRANSCRIPTION_SCHEMA_VERSION,
        range,
        language: chain
            .iter()
            .find_map(|piece| piece.transcript.language.clone()),
        text,
        cues,
    })
}

/// A word as two transcripts are compared by: its letters and digits.
fn spoken(word: &TranscriptWord) -> String {
    word.text
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Where `earlier` stops giving words and `later` starts, as how many of
/// its words `earlier` gives and which word of `later` comes next.
///
/// The two heard the same sound where they overlap, and said the same
/// words of it; the times they gave those words may be a second apart. So
/// they are joined at a word both have, the one nearest the quietest
/// moment of that sound: `earlier` gives up to it and `later` goes on
/// after it, and no word is lost or said twice. Where they share no word,
/// each gives the words on its side of that moment.
fn join(
    earlier: &Piece,
    later: &Piece,
    earlier_words: &[&TranscriptWord],
    later_words: &[&TranscriptWord],
) -> (usize, usize) {
    let (from, to) = (later.transcript.range.start, earlier.transcript.range.end);
    let middle = |word: &TranscriptWord| (word.start + word.end) / 2.0;
    // One that was made to start or end in a pause is whole from there:
    // the other gives what was said on its side of the pause.
    if later.edges.starts_in_pause {
        let kept = earlier_words
            .iter()
            .position(|word| word.start >= from)
            .unwrap_or(earlier_words.len());
        return (kept, 0);
    }
    if earlier.edges.ends_in_pause {
        let next = later_words
            .iter()
            .position(|word| middle(word) >= to)
            .unwrap_or(later_words.len());
        return (earlier_words.len(), next);
    }
    let quiet = seam(earlier, later);
    // The end of the one and the start of the other, where both listened.
    let tail = earlier_words
        .iter()
        .position(|word| word.end >= from - WORD_TIME_SLACK_SECONDS)
        .unwrap_or(earlier_words.len());
    let head = later_words
        .iter()
        .rposition(|word| word.start <= to + WORD_TIME_SLACK_SECONDS)
        .map_or(0, |index| index + 1);
    let (a, b) = (&earlier_words[tail..], &later_words[..head]);
    let (said_a, said_b): (Vec<String>, Vec<String>) = (
        a.iter().map(|word| spoken(word)).collect(),
        b.iter().map(|word| spoken(word)).collect(),
    );
    // The longest run of words both said, in order.
    let mut longest = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            longest[i][j] = if !said_a[i].is_empty() && said_a[i] == said_b[j] {
                longest[i + 1][j + 1] + 1
            } else {
                longest[i + 1][j].max(longest[i][j + 1])
            };
        }
    }
    let mut shared = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if !said_a[i].is_empty() && said_a[i] == said_b[j] {
            shared.push((i, j));
            i += 1;
            j += 1;
        } else if longest[i + 1][j] >= longest[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let nearest = shared
        .iter()
        .min_by(|x, y| {
            (middle(a[x.0]) - quiet)
                .abs()
                .total_cmp(&(middle(a[y.0]) - quiet).abs())
        })
        .copied();
    match nearest {
        Some((i, j)) => (tail + i + 1, j + 1),
        None => (
            earlier_words
                .iter()
                .position(|word| middle(word) >= quiet)
                .unwrap_or(earlier_words.len()),
            later_words
                .iter()
                .position(|word| middle(word) >= quiet)
                .unwrap_or(later_words.len()),
        ),
    }
}

/// Where the transcript of `earlier` gives way to that of `later`, which
/// starts before `earlier` ends: the quietest moment of the sound both
/// heard, or the middle of that sound when it is too short to tell or is
/// not kept.
fn seam(earlier: &Piece, later: &Piece) -> f64 {
    let (from, to) = (later.transcript.range.start, earlier.transcript.range.end);
    [later, earlier]
        .into_iter()
        .find_map(|piece| {
            let start = piece.transcript.range.start;
            quietest(&piece.audio, from - start, to - start, false)
        })
        .map_or((from + to) / 2.0, |(at, _)| from + at)
}

/// The middle of the quietest [`SEAM_QUIET_SECONDS`] of `[from, to)`
/// seconds of the sound kept at `path`, in seconds after `from`, and
/// whether it is a pause: quieter than the sound around it by
/// [`PAUSE_ENERGY_SHARE`]. Of stretches as quiet, the `last` or the first.
/// `None` when the sound is not kept or is shorter than that.
fn quietest(path: &Path, from: f64, to: f64, last: bool) -> Option<(f64, bool)> {
    let samples = wav_samples(path, from, to)?;
    let window = (SEAM_WINDOW_SECONDS * f64::from(TRANSCRIPTION_SAMPLE_RATE)) as usize;
    let energy: Vec<f64> = samples
        .chunks(window)
        .filter(|chunk| chunk.len() == window)
        .map(|chunk| chunk.iter().map(|sample| f64::from(*sample).powi(2)).sum())
        .collect();
    let run = (SEAM_QUIET_SECONDS / SEAM_WINDOW_SECONDS).round() as usize;
    if energy.len() < run {
        return None;
    }
    let runs: Vec<f64> = (0..=energy.len() - run)
        .map(|first| energy[first..first + run].iter().sum())
        .collect();
    let least = runs.iter().copied().fold(f64::INFINITY, f64::min);
    // As quiet as the quietest, to the ear: within a decibel of it, or
    // silent as it is.
    let as_quiet = |sum: &f64| *sum <= least * 1.25 + 1.0;
    let first = if last {
        runs.iter().rposition(as_quiet)?
    } else {
        runs.iter().position(as_quiet)?
    };
    let around = energy.iter().sum::<f64>() / energy.len() as f64 * run as f64;
    let pause = runs[first] <= around * PAUSE_ENERGY_SHARE;
    Some((
        (first as f64 + run as f64 / 2.0) * SEAM_WINDOW_SECONDS,
        pause,
    ))
}

/// Take the pauses out of words timed by alignment, by the sound whisper
/// was given (`audio`, which starts where the transcript's range does).
///
/// Alignment says where each word ends, and a word is taken to start where
/// the one before it ended, so a pause between two words is inside the
/// second. Where the sound between them falls quiet, the first goes on
/// until it does, the second starts when it stops being quiet, and a cut in
/// the pause is in neither. Cues take the times of their first and last
/// words. Without the sound nothing changes.
fn settle_words(transcript: &mut Transcript, audio: &Path) {
    let origin = transcript.range.start;
    let Some(samples) = wav_samples(audio, 0.0, transcript.range.end - origin) else {
        return;
    };
    let window = (SEAM_WINDOW_SECONDS * f64::from(TRANSCRIPTION_SAMPLE_RATE)) as usize;
    let energy: Vec<f64> = samples
        .chunks(window)
        .map(|chunk| {
            chunk
                .iter()
                .map(|sample| f64::from(*sample).powi(2))
                .sum::<f64>()
                / chunk.len() as f64
        })
        .collect();
    let window_of = |seconds: f64| {
        (((seconds - origin) / SEAM_WINDOW_SECONDS).max(0.0) as usize).min(energy.len())
    };
    let time_of = |window: usize| origin + window as f64 * SEAM_WINDOW_SECONDS;
    let run = (WORD_PAUSE_SECONDS / SEAM_WINDOW_SECONDS).round() as usize;
    // The quiet stretches of windows `from..to`, each as its first window
    // and the one after its last: quiet beside the loudest of `heard..to`.
    let pauses = |heard: usize, from: usize, to: usize| {
        let mut found: Vec<(usize, usize)> = Vec::new();
        if to <= from {
            return found;
        }
        let loudest = energy[heard.min(from)..to]
            .iter()
            .copied()
            .fold(0.0, f64::max);
        let mut first = None;
        // The window past the last ends a stretch that reaches it.
        let quiet = energy[from..to]
            .iter()
            .map(|energy| *energy <= loudest * WORD_PAUSE_SHARE)
            .chain(std::iter::once(false));
        for (at, quiet) in (from..).zip(quiet) {
            match (quiet, first) {
                (true, None) => first = Some(at),
                (false, Some(start)) => {
                    if at - start >= run {
                        found.push((start, at));
                    }
                    first = None;
                }
                _ => {}
            }
        }
        found
    };
    let mut words: Vec<&mut TranscriptWord> = transcript
        .cues
        .iter_mut()
        .flat_map(|cue| cue.words.iter_mut())
        .collect();
    for index in 0..words.len() {
        let (before, heard) = match index.checked_sub(1) {
            Some(last) => (words[last].end, window_of(words[last].start)),
            None => (origin, 0),
        };
        let end = words[index].end;
        let found = pauses(heard, window_of(before), window_of(end));
        match (found.first(), found.last()) {
            (Some(first), Some(last)) => {
                if index > 0 {
                    let tail = time_of(first.0).min(before + WORD_TAIL_SECONDS);
                    words[index - 1].end = tail.max(before);
                }
                words[index].start = time_of(last.1).min(end);
            }
            _ => words[index].start = before.min(end),
        }
    }
    // The last word trails on until the sound after it falls quiet.
    if let Some(last) = words.last_mut() {
        let (from, to) = (
            window_of(last.end),
            window_of(last.end + WORD_TAIL_SECONDS + WORD_PAUSE_SECONDS),
        );
        if let Some(pause) = pauses(window_of(last.start), from, to).first() {
            last.end = time_of(pause.0).max(last.end);
        }
    }
    drop(words);
    for cue in &mut transcript.cues {
        if let (Some(first), Some(last)) = (cue.words.first(), cue.words.last()) {
            cue.start = first.start;
            cue.end = last.end.max(first.start);
        }
    }
}

/// The samples of `[from, to)` seconds of a 16-bit mono WAV file at the
/// transcription rate, as the cache keeps a transcript's sound.
fn wav_samples(path: &Path, from: f64, to: f64) -> Option<Vec<i16>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).ok()?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return None;
    }
    // The chunks before the samples: the format, and whatever else was
    // written.
    let (data, length) = loop {
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk).ok()?;
        let length = u64::from(u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]));
        if &chunk[..4] == b"data" {
            break (file.stream_position().ok()?, length);
        }
        file.seek(SeekFrom::Current((length + length % 2) as i64))
            .ok()?;
    };
    let rate = f64::from(TRANSCRIPTION_SAMPLE_RATE);
    let first = ((from.max(0.0) * rate) as u64).min(length / 2);
    let last = ((to.max(0.0) * rate) as u64).min(length / 2);
    file.seek(SeekFrom::Start(data + first * 2)).ok()?;
    let mut bytes = vec![0u8; ((last.saturating_sub(first)) * 2) as usize];
    file.read_exact(&mut bytes).ok()?;
    Some(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect(),
    )
}

fn ensure_not_cancelled(cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(EngineError::Cancelled)
    } else {
        Ok(())
    }
}

fn result_from_cache(
    entry: &CacheEntry,
    transcript: Transcript,
    config: &WhisperConfig,
    include_audio: bool,
    cached: bool,
) -> TranscriptionResult {
    TranscriptionResult {
        key: entry.key.clone(),
        transcript,
        backend: "whisper.cpp".into(),
        backend_version: config.backend_version.clone(),
        model: config.model_identity.path.clone(),
        model_version: config.model_version.clone(),
        audio_path: include_audio.then(|| entry.path("audio.wav").to_string_lossy().into_owned()),
        cached,
    }
}

fn validate_operation_range(range: TimeRange, listen: bool) -> Result<()> {
    if !range.start.is_finite()
        || !range.end.is_finite()
        || range.start < 0.0
        || range.end <= range.start
    {
        return Err(EngineError::msg(
            "transcription range requires finite 0 <= start < end",
        ));
    }
    let max = if listen {
        MAX_LISTEN_RANGE_SECONDS
    } else {
        MAX_TRANSCRIPTION_RANGE_SECONDS
    };
    if range.duration() > max + 1e-9 {
        return Err(EngineError::msg(format!(
            "transcription range exceeds the {max:.0}s limit"
        )));
    }
    Ok(())
}

fn resolve_whisper_config(options: &TranscriptionOptions) -> Result<WhisperConfig> {
    if let TranscriptionBackend::ExplicitProvider { name } = &options.backend {
        return Err(EngineError::capability_unavailable(
            "transcription.provider",
            format!("provider backend {name:?} is disabled; select local whisper.cpp explicitly"),
        ));
    }

    let binary = options
        .binary
        .clone()
        .or_else(|| std::env::var_os("KERF_WHISPER_BIN").map(PathBuf::from))
        .or_else(discover_whisper_binary)
        .ok_or_else(|| {
            EngineError::capability_unavailable(
                "transcription",
                "whisper.cpp sidecar is unavailable; install whisper-cli or set KERF_WHISPER_BIN",
            )
        })?;
    let binary = resolve_path_command(binary);
    if !sidecar_available(&binary) {
        return Err(EngineError::capability_unavailable(
            "transcription",
            format!(
                "whisper.cpp sidecar is unavailable at {}; set KERF_WHISPER_BIN to whisper-cli",
                binary.display()
            ),
        ));
    }

    let model = options
        .model
        .clone()
        .or_else(|| std::env::var_os("KERF_WHISPER_MODEL").map(PathBuf::from))
        .ok_or_else(|| {
            EngineError::capability_unavailable(
                "transcription",
                "whisper.cpp model is not configured; set KERF_WHISPER_MODEL or pass a model",
            )
        })?;
    let model_identity = SourceIdentity::from_path(&model).map_err(|error| {
        EngineError::capability_unavailable(
            "transcription",
            format!(
                "whisper.cpp model is unavailable at {}: {error}",
                model.display()
            ),
        )
    })?;
    let binary_identity = binary_identity(&binary);
    let backend_version = configured_value(
        options.backend_version.as_deref(),
        "KERF_WHISPER_CPP_VERSION",
        DEFAULT_WHISPER_BACKEND_VERSION,
    );
    let fallback_model_version = format!(
        "file:{}:{}",
        model_identity.size,
        model_identity.modified_ns.unwrap_or_default()
    );
    let model_version = configured_value(
        options.model_version.as_deref(),
        "KERF_WHISPER_MODEL_VERSION",
        &fallback_model_version,
    );
    let alignment = alignment_preset(&model).and_then(|preset| alignment_of(&binary, preset));
    Ok(WhisperConfig {
        binary,
        model,
        model_identity,
        binary_identity,
        backend_version,
        model_version,
        alignment,
    })
}

/// The alignment heads whisper.cpp has for the model in `path`, as its
/// `-dtw` names them, going by what the file's header says the model is.
///
/// With them whisper times each token by where the model attended, a tenth
/// of a second or two out. Without, by the timestamps the model writes
/// between words, which put the first dozen tokens of a sentence at one
/// moment and the ones after up to two seconds early. `None` for a file it
/// cannot tell: not a ggml model, a distilled one, or a large model older
/// than v3, whose two versions read alike.
fn alignment_preset(model: &Path) -> Option<&'static str> {
    use std::io::Read;
    let mut header = [0u8; 48];
    fs::File::open(model).ok()?.read_exact(&mut header).ok()?;
    let field = |index: usize| {
        let at = index * 4;
        u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    // "ggml", then the vocabulary and the shape of the encoder and decoder.
    if field(0) != 0x6767_6d6c {
        return None;
    }
    let (vocabulary, audio_layers, text_layers) = (field(1), field(5), field(9));
    let english = vocabulary == 51_864;
    Some(match (audio_layers, text_layers, english) {
        (4, 4, false) => "tiny",
        (4, 4, true) => "tiny.en",
        (6, 6, false) => "base",
        (6, 6, true) => "base.en",
        (12, 12, false) => "small",
        (12, 12, true) => "small.en",
        (24, 24, false) => "medium",
        (24, 24, true) => "medium.en",
        (32, 32, false) if vocabulary == 51_866 => "large.v3",
        (32, 4, false) if vocabulary == 51_866 => "large.v3.turbo",
        _ => return None,
    })
}

/// How the whisper at `binary` is asked for alignment by `preset`, going
/// by the options its help lists; `None` for one that has no `-dtw`.
fn alignment_of(binary: &Path, preset: &'static str) -> Option<Alignment> {
    let help = Command::new(binary)
        .arg("--help")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let lists = |option: &str| {
        [&help.stdout, &help.stderr]
            .into_iter()
            .any(|text| String::from_utf8_lossy(text).contains(option))
    };
    lists("--dtw").then(|| Alignment {
        preset,
        without_flash_attention: lists("--no-flash-attn"),
    })
}

fn configured_value(explicit: Option<&str>, variable: &str, fallback: &str) -> String {
    explicit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            std::env::var(variable)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| fallback.to_owned())
}

fn cache_options(options: &TranscriptionOptions, config: &WhisperConfig) -> Result<Value> {
    let mut key = cache_key(options, config)?;
    // Words timed by alignment are not the words timed without: a
    // transcript made before is not one of these.
    if let Some(alignment) = config.alignment {
        key["wordTimes"] = json!({ "alignment": alignment.preset });
    }
    Ok(key)
}

fn cache_key(options: &TranscriptionOptions, config: &WhisperConfig) -> Result<Value> {
    Ok(json!({
        "schemaVersion": TRANSCRIPTION_SCHEMA_VERSION,
        "backend": "whisper.cpp",
        "backendVersion": config.backend_version,
        "binary": config.binary_identity,
        "model": serde_json::to_value(&config.model_identity)?,
        "modelVersion": config.model_version,
        "language": options.language.as_deref().unwrap_or("auto"),
        "range": options.range,
        "audio": {
            "channels": 1,
            "sampleRate": TRANSCRIPTION_SAMPLE_RATE,
            "codec": "pcm_s16le"
        }
    }))
}

fn binary_identity(binary: &Path) -> Value {
    let metadata = fs::metadata(binary).ok();
    json!({
        "path": binary.to_string_lossy(),
        "size": metadata.as_ref().map_or(0, fs::Metadata::len),
        "modifiedNs": metadata
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
    })
}

fn discover_whisper_binary() -> Option<PathBuf> {
    ["whisper-cli", "whisper-cpp", "main"]
        .into_iter()
        .map(PathBuf::from)
        .find(|candidate| sidecar_available(candidate))
}

fn resolve_path_command(binary: PathBuf) -> PathBuf {
    if binary.is_absolute() || binary.components().count() > 1 {
        return binary;
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(&binary))
        .find(|candidate| candidate.is_file())
        .unwrap_or(binary)
}

fn sidecar_available(binary: &Path) -> bool {
    if binary.is_absolute() || binary.components().count() > 1 {
        return binary.is_file();
    }
    Command::new(binary)
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn whisper_args(
    config: &WhisperConfig,
    audio_path: &Path,
    output_base: &Path,
    language: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-m".into(),
        config.model.to_string_lossy().into_owned(),
        "-f".into(),
        audio_path.to_string_lossy().into_owned(),
        "-ojf".into(),
        "-of".into(),
        output_base.to_string_lossy().into_owned(),
        "-pp".into(),
        // Its own default is four threads on any machine.
        "-t".into(),
        crate::resources::whisper_thread_count().to_string(),
    ];
    let language = language
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("auto");
    args.extend(["-l".into(), language.trim().into()]);
    if let Some(alignment) = config.alignment {
        args.extend(["-dtw".into(), alignment.preset.into()]);
        if alignment.without_flash_attention {
            args.push("-nfa".into());
        }
    }
    args
}

/// How far before a range FFmpeg is sent in the file to start reading, in
/// whole seconds: what is decoded on the way to the range settles the
/// decoder (an MP3's bit reservoir, a codec's overlap with the frame before)
/// and the resampler, and is then dropped.
const SEEK_LEAD_SECONDS: f64 = 2.0;

/// Where in the file FFmpeg starts reading for a range that starts at
/// `start`, or `None` to read from the top. A whole second, so the 16 kHz
/// samples fall where they do when the file is read from the top: the range
/// then comes out sample for sample the same, without the file being read
/// up to it.
fn seek_before(start: f64) -> Option<f64> {
    let at = start.floor() - SEEK_LEAD_SECONDS;
    (at >= 1.0).then_some(at)
}

/// `seconds` less the whole seconds `whole`, with the digits after the point
/// kept as they are: FFmpeg then cuts at the same microsecond as it does for
/// `seconds` itself.
fn seconds_after(seconds: f64, whole: f64) -> String {
    let text = seconds.to_string();
    let (before, after) = text.split_once('.').unwrap_or((&text, ""));
    let before = before
        .parse::<u64>()
        .unwrap_or(0)
        .saturating_sub(whole as u64);
    if after.is_empty() {
        before.to_string()
    } else {
        format!("{before}.{after}")
    }
}

/// FFmpeg's arguments to write `range` of `src` as the WAV whisper reads.
fn extract_audio_args(src: &str, range: TimeRange, output: &Path) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    // Before the input, `-ss` goes there in the file; after it, it reads
    // and drops everything up to the range.
    let seek = seek_before(range.start);
    if let Some(seek) = seek {
        args.extend(["-ss".into(), format!("{seek:.0}")]);
    }
    args.extend([
        "-i".into(),
        src.into(),
        "-ss".into(),
        seconds_after(range.start, seek.unwrap_or(0.0)),
        "-t".into(),
        range.duration().to_string(),
        "-vn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        TRANSCRIPTION_SAMPLE_RATE.to_string(),
        "-c:a".into(),
        "pcm_s16le".into(),
        "-f".into(),
        "wav".into(),
        "-y".into(),
        output.to_string_lossy().into_owned(),
    ]);
    args
}

fn extract_audio_range(
    src: &str,
    range: TimeRange,
    output: &Path,
    cancellation: &CancellationToken,
    on_progress: &mut impl FnMut(TranscriptionProgress),
) -> Result<()> {
    run_sidecar(
        Path::new(resolve_ffmpeg_bin()),
        &extract_audio_args(src, range, output),
        cancellation,
        TranscriptionStage::PrepareAudio,
        on_progress,
    )?;
    Ok(())
}

fn run_sidecar(
    program: &Path,
    args: &[String],
    cancellation: &CancellationToken,
    stage: TranscriptionStage,
    on_progress: &mut impl FnMut(TranscriptionProgress),
) -> Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    let finished = sidecar::run(command, cancellation, |line| {
        if let Some(progress) = parse_sidecar_progress(line) {
            on_progress(TranscriptionProgress { stage, progress });
        }
    })?;
    if !finished.status.success() {
        return Err(EngineError::msg(format!(
            "{} sidecar failed with {}: {}",
            program.display(),
            finished.status,
            finished.stderr.trim()
        )));
    }
    Ok(())
}

fn parse_sidecar_progress(line: &str) -> Option<f64> {
    if let Some(percent) = line.find('%') {
        let bytes = line.as_bytes();
        let mut start = percent;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }
        return line
            .get(start..percent)?
            .parse::<f64>()
            .ok()
            .map(|value| (value / 100.0).clamp(0.0, 1.0));
    }
    for marker in ["progress=", "progress:"] {
        let Some(index) = line.find(marker) else {
            continue;
        };
        let value = line[index + marker.len()..]
            .split_whitespace()
            .next()?
            .trim_end_matches(',')
            .parse::<f64>()
            .ok()?;
        if value.is_finite() {
            return Some(if value > 1.0 { value / 100.0 } else { value }.clamp(0.0, 1.0));
        }
    }
    None
}

fn read_bounded_text(path: &Path, max_bytes: u64) -> Result<String> {
    let size = fs::metadata(path)?.len();
    if size > max_bytes {
        return Err(EngineError::msg(format!(
            "transcript output exceeds {max_bytes} bytes"
        )));
    }
    Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned())
}

fn read_cached_transcript(entry: &CacheEntry, range: TimeRange) -> Result<Transcript> {
    let output = read_bounded_text(&entry.path("transcript.json"), MAX_TRANSCRIPT_BYTES)?;
    let transcript: Transcript = serde_json::from_str(&output)?;
    if transcript.schema_version != TRANSCRIPTION_SCHEMA_VERSION || transcript.range != range {
        return Err(EngineError::msg(
            "cached transcript schema or range does not match request",
        ));
    }
    Ok(transcript)
}

fn parse_whisper_value(value: &Value, range: TimeRange) -> Result<Transcript> {
    let language = value
        .get("result")
        .and_then(|result| result.get("language"))
        .or_else(|| value.get("language"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let segments = ["transcription", "segments", "cues"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_array));
    let mut cues = segments
        .map(|segments| {
            segments
                .iter()
                .filter_map(|segment| parse_segment(segment, range))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    cues.sort_by(|left, right| {
        left.start
            .partial_cmp(&right.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if cues.is_empty() {
        if let Some(text) = value.get("text").and_then(Value::as_str).map(str::trim) {
            if !text.is_empty() {
                cues.push(TranscriptCue {
                    start: range.start,
                    end: range.end,
                    text: text.into(),
                    words: vec![TranscriptWord {
                        start: range.start,
                        end: range.end,
                        text: text.into(),
                        confidence: None,
                    }],
                });
            }
        }
    }
    let text = cues
        .iter()
        .map(|cue| cue.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(Transcript {
        schema_version: TRANSCRIPTION_SCHEMA_VERSION,
        range,
        language,
        text,
        cues,
    })
}

fn parse_segment(value: &Value, range: TimeRange) -> Option<TranscriptCue> {
    let object = value.as_object()?;
    let (start, end) = shift_range(timestamp_pair(object)?, range);
    let (start, end) = clamp_range(start, end, range)?;
    let segment_text = object
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let words = if let Some(words) = object.get("words").and_then(Value::as_array) {
        parse_word_array(words, (start, end), range)
    } else if let Some(tokens) = object.get("tokens").and_then(Value::as_array) {
        parse_token_words(tokens, (start, end), range)
    } else {
        Vec::new()
    };
    let words = if words.is_empty() && !segment_text.is_empty() {
        vec![TranscriptWord {
            start,
            end,
            text: segment_text.into(),
            confidence: None,
        }]
    } else {
        words
    };
    let text = if segment_text.is_empty() {
        words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        segment_text.into()
    };
    if text.is_empty() {
        return None;
    }
    Some(TranscriptCue {
        start,
        end,
        text,
        words,
    })
}

fn parse_word_array(
    values: &[Value],
    fallback: (f64, f64),
    range: TimeRange,
) -> Vec<TranscriptWord> {
    values
        .iter()
        .filter_map(|value| {
            let object = value.as_object()?;
            let text = object
                .get("word")
                .or_else(|| object.get("text"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())?;
            let (start, end) = timestamp_pair(object)
                .map(|pair| shift_range(pair, range))
                .unwrap_or(fallback);
            let (start, end) = clamp_range(start, end, range)?;
            Some(TranscriptWord {
                start,
                end,
                text: text.into(),
                confidence: confidence(object),
            })
        })
        .collect()
}

fn parse_token_words(
    values: &[Value],
    fallback: (f64, f64),
    range: TimeRange,
) -> Vec<TranscriptWord> {
    /// A word as its tokens are read.
    struct Reading {
        word: TranscriptWord,
        /// Where alignment ends its last token that is said, and its last
        /// token of any kind: a full stop's place drifts into the silence
        /// after it.
        said: Option<f64>,
        marked: Option<f64>,
        confidence_sum: f64,
        confidence_count: usize,
    }
    let mut words: Vec<TranscriptWord> = Vec::new();
    let mut aligned = false;
    let mut finish = |reading: Reading, words: &mut Vec<TranscriptWord>| {
        let mut word = reading.word;
        if reading.confidence_count > 0 {
            word.confidence = Some(reading.confidence_sum / reading.confidence_count as f64);
        }
        // Timed by alignment, a word ends where its last token does and
        // starts where the word before it ended, the cue's start for the
        // first: `settle_words` then takes the pauses out.
        if let Some(end) = reading.said.or(reading.marked) {
            aligned = true;
            word.start = words.last().map_or(fallback.0, |before| before.end);
            word.end = end.max(word.start);
        }
        words.push(word);
    };
    let mut current: Option<Reading> = None;
    for value in values {
        let Some(object) = value.as_object() else {
            continue;
        };
        let Some(raw_text) = object.get("text").and_then(Value::as_str) else {
            continue;
        };
        if is_special_token(raw_text) {
            continue;
        }
        let raw_text = raw_text.trim_end();
        if raw_text.trim().is_empty() {
            continue;
        }
        let (start, end) = timestamp_pair(object)
            .map(|pair| shift_range(pair, range))
            .unwrap_or(fallback);
        // whisper.cpp times some tokens as an instant; that is where the
        // token is, not the whole cue.
        let (start, end) = clamp_range(start, end, range)
            .or_else(|| {
                (start.is_finite() && start == end)
                    .then(|| start.clamp(range.start, range.end))
                    .map(|at| (at, at))
            })
            .unwrap_or(fallback);
        // Where alignment ends the token, in hundredths of a second of the
        // sound whisper was given: -1 without it.
        let ends = object
            .get("t_dtw")
            .and_then(Value::as_f64)
            .filter(|at| *at >= 0.0)
            .map(|at| (at / 100.0 + range.start).clamp(range.start, range.end));
        let starts_new = current.is_some()
            && (raw_text.starts_with(char::is_whitespace) || raw_text.starts_with('▁'));
        if starts_new {
            if let Some(reading) = current.take() {
                finish(reading, &mut words);
            }
        }
        let text = raw_text.trim().trim_start_matches('▁');
        if text.is_empty() {
            continue;
        }
        let reading = current.get_or_insert_with(|| Reading {
            word: TranscriptWord {
                start,
                end,
                text: String::new(),
                confidence: None,
            },
            said: None,
            marked: None,
            confidence_sum: 0.0,
            confidence_count: 0,
        });
        reading.word.text.push_str(text);
        reading.word.end = end.max(reading.word.end);
        if text.chars().any(char::is_alphanumeric) {
            reading.said = ends.or(reading.said);
        }
        reading.marked = ends.or(reading.marked);
        if let Some(value) = confidence(object) {
            reading.confidence_sum += value;
            reading.confidence_count += 1;
        }
    }
    if let Some(reading) = current {
        finish(reading, &mut words);
    }
    if !aligned {
        spread_instants(&mut words, fallback.1);
    }
    words
}

/// Whisper's own timestamps put the first words of a sentence at one
/// moment, each with no length. So that each has a time of its own, in the
/// order they were said, such words share out the time up to the word after
/// them: to its end when it starts at that moment too, or else to its
/// start, or to the cue's end after the last. The times are no truer for
/// it, but captions made of them do not land on one frame and replace each
/// other.
fn spread_instants(words: &mut [TranscriptWord], cue_end: f64) {
    let mut first = 0;
    while first < words.len() {
        let at = words[first].start;
        if words[first].end > at {
            first += 1;
            continue;
        }
        let mut after = first;
        while after < words.len() && words[after].start == at && words[after].end == at {
            after += 1;
        }
        let (until, count) = match words.get(after) {
            Some(next) if next.start <= at => (next.end, after - first + 1),
            Some(next) => (next.start, after - first),
            None => (cue_end, after - first),
        };
        if until > at {
            let step = (until - at) / count as f64;
            for (index, word) in words[first..first + count].iter_mut().enumerate() {
                word.start = at + step * index as f64;
                word.end = at + step * (index + 1) as f64;
            }
        }
        first += count;
    }
}

fn is_special_token(text: &str) -> bool {
    let text = text.trim();
    text.starts_with("<|") || (text.starts_with('[') && text.ends_with(']'))
}

fn confidence(object: &Map<String, Value>) -> Option<f64> {
    ["confidence", "probability", "p"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_f64))
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0.0, 1.0))
}

fn timestamp_pair(object: &Map<String, Value>) -> Option<(f64, f64)> {
    if let Some(timestamps) = object.get("timestamps").and_then(Value::as_object) {
        let start = timestamp_value(timestamps.get("from").or_else(|| timestamps.get("start"))?)?;
        let end = timestamp_value(timestamps.get("to").or_else(|| timestamps.get("end"))?)?;
        return Some((start, end));
    }
    if let Some(offsets) = object.get("offsets").and_then(Value::as_object) {
        let start = offsets.get("from").and_then(Value::as_f64)? / 1000.0;
        let end = offsets.get("to").and_then(Value::as_f64)? / 1000.0;
        return Some((start, end));
    }
    let start = timestamp_value(object.get("start")?)?;
    let end = timestamp_value(object.get("end")?)?;
    Some((start, end))
}

fn shift_range((start, end): (f64, f64), source_range: TimeRange) -> (f64, f64) {
    (start + source_range.start, end + source_range.start)
}

fn timestamp_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(value) => value.as_f64().filter(|value| value.is_finite()),
        Value::String(value) => parse_timestamp(value),
        _ => None,
    }
}

fn parse_timestamp(value: &str) -> Option<f64> {
    let value = value.trim().replace(',', ".");
    let mut parts = value.split(':');
    let first = parts.next()?;
    let second = parts.next();
    let third = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let result = match (second, third) {
        (Some(minutes), Some(seconds)) => {
            first.parse::<f64>().ok()? * 3600.0
                + minutes.parse::<f64>().ok()? * 60.0
                + seconds.parse::<f64>().ok()?
        }
        (None, None) => first.parse::<f64>().ok()?,
        _ => return None,
    };
    result.is_finite().then_some(result)
}

fn clamp_range(start: f64, end: f64, range: TimeRange) -> Option<(f64, f64)> {
    if !start.is_finite() || !end.is_finite() || end <= start {
        return None;
    }
    let start = start.clamp(range.start, range.end);
    let end = end.clamp(range.start, range.end);
    (end > start).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    #[test]
    fn parses_segment_and_token_timestamps_into_source_words() {
        let range = TimeRange::new(10.0, 20.0).expect("range");
        let transcript = parse_whisper_json(
            &json!({
                "result": {"language": "en"},
                "transcription": [{
                    "timestamps": {"from": "00:00:00,500", "to": "00:00:02,000"},
                    "text": " Hello world",
                    "tokens": [
                        {"text": " Hello", "timestamps": {"from": "00:00:00,500", "to": "00:00:01,000"}, "p": 0.8},
                        {"text": " world", "timestamps": {"from": "00:00:01,000", "to": "00:00:02,000"}, "p": 0.9}
                    ]
                }]
            })
            .to_string(),
            range,
        )
        .expect("transcript");
        assert_eq!(transcript.language.as_deref(), Some("en"));
        assert_eq!(transcript.text, "Hello world");
        assert_eq!(transcript.cues[0].words.len(), 2);
        assert_eq!(transcript.cues[0].words[0].text, "Hello");
        assert_eq!(transcript.cues[0].words[0].start, 10.5);
        assert_eq!(transcript.cues[0].words[1].confidence, Some(0.9));
    }

    #[test]
    fn an_instant_token_keeps_its_time_instead_of_the_cue() {
        let range = TimeRange::new(0.0, 20.0).expect("range");
        let transcript = parse_whisper_json(
            &json!({
                "transcription": [{
                    "offsets": {"from": 0, "to": 10580},
                    "text": " And so",
                    "tokens": [
                        {"text": "[_BEG_]", "offsets": {"from": 0, "to": 0}},
                        {"text": " And", "offsets": {"from": 320, "to": 320}},
                        {"text": " so", "offsets": {"from": 330, "to": 530}}
                    ]
                }]
            })
            .to_string(),
            range,
        )
        .expect("transcript");
        // It starts where the token is, and lasts until the next word.
        let words = &transcript.cues[0].words;
        assert_eq!((words[0].start, words[0].end), (0.32, 0.33));
        assert_eq!((words[1].start, words[1].end), (0.33, 0.53));
    }

    #[test]
    fn words_whisper_puts_at_one_moment_share_the_time_to_the_next() {
        // As whisper times a Korean sentence: its first words at the
        // cue's start, with no length, and the one after them from there.
        let transcript = parse_whisper_json(
            &json!({
                "transcription": [{
                    "offsets": {"from": 17000, "to": 27000},
                    "text": " a b c d e",
                    "tokens": [
                        {"text": " a", "offsets": {"from": 17000, "to": 17000}},
                        {"text": " b", "offsets": {"from": 17000, "to": 17000}},
                        {"text": " c", "offsets": {"from": 17000, "to": 17400}},
                        {"text": " d", "offsets": {"from": 17400, "to": 18000}},
                        {"text": " e", "offsets": {"from": 26000, "to": 26000}}
                    ]
                }]
            })
            .to_string(),
            TimeRange::new(0.0, 30.0).expect("range"),
        )
        .expect("transcript");
        let times: Vec<(f64, f64)> = transcript.cues[0]
            .words
            .iter()
            .map(|word| ((word.start * 1000.0).round(), (word.end * 1000.0).round()))
            .collect();
        assert_eq!(
            times,
            [
                (17000.0, 17133.0),
                (17133.0, 17267.0),
                (17267.0, 17400.0),
                (17400.0, 18000.0),
                // The last has until the cue ends.
                (26000.0, 27000.0),
            ]
        );
    }

    #[test]
    fn words_are_timed_by_alignment_where_whisper_gives_it() {
        // `t_dtw` is where a token ends, in hundredths of a second: the
        // times whisper wrote between the words are not used.
        let range = TimeRange::new(100.0, 130.0).expect("range");
        let mut transcript = parse_whisper_json(
            &json!({
                "transcription": [{
                    "offsets": {"from": 1000, "to": 5000},
                    "text": " So we closed.",
                    "tokens": [
                        {"text": "[_BEG_]", "offsets": {"from": 1000, "to": 1000}, "t_dtw": -1},
                        {"text": " So", "offsets": {"from": 1000, "to": 1000}, "t_dtw": 160},
                        {"text": " we", "offsets": {"from": 1000, "to": 1000}, "t_dtw": 190},
                        {"text": " clo", "offsets": {"from": 1000, "to": 1200}, "t_dtw": 320},
                        {"text": "sed", "offsets": {"from": 1200, "to": 1400}, "t_dtw": 350},
                        {"text": ".", "offsets": {"from": 1400, "to": 5000}, "t_dtw": 480}
                    ]
                }]
            })
            .to_string(),
            range,
        )
        .expect("transcript");
        let times = |transcript: &Transcript| -> Vec<(String, f64, f64)> {
            transcript.cues[0]
                .words
                .iter()
                .map(|word| {
                    (
                        word.text.clone(),
                        (word.start * 100.0).round() / 100.0,
                        (word.end * 100.0).round() / 100.0,
                    )
                })
                .collect()
        };
        // Each ends where its last said token does, the full stop's place
        // aside, and starts where the one before ended: the cue's start
        // for the first.
        assert_eq!(
            times(&transcript),
            [
                ("So".to_owned(), 101.0, 101.6),
                ("we".to_owned(), 101.6, 101.9),
                ("closed.".to_owned(), 101.9, 103.5),
            ]
        );

        // The sound: "So we" from 1.3 to 1.9 seconds, a pause, "closed"
        // from 2.9 to 3.62, and silence after.
        let folder = tempdir().expect("folder");
        let audio = folder.path().join("audio.wav");
        tone_where(&audio, 30.0, |at| {
            (1.3..1.9).contains(&at) || (2.9..3.62).contains(&at)
        });
        settle_words(&mut transcript, &audio);
        assert_eq!(
            times(&transcript),
            [
                // It starts when the silence before it ends.
                ("So".to_owned(), 101.3, 101.6),
                ("we".to_owned(), 101.6, 101.9),
                // The pause is in neither word, and the last trails on
                // until its sound stops.
                ("closed.".to_owned(), 102.9, 103.62),
            ]
        );
        let cue = &transcript.cues[0];
        assert_eq!(
            ((cue.start * 100.0).round(), (cue.end * 100.0).round()),
            (10130.0, 10362.0)
        );
        // Without the sound they stay as alignment timed them.
        let mut alone = transcript.clone();
        settle_words(&mut alone, &folder.path().join("missing.wav"));
        assert_eq!(alone, transcript);
    }

    #[test]
    fn a_models_alignment_heads_are_told_from_its_header() {
        let folder = tempdir().expect("folder");
        let model = |name: &str, fields: [u32; 11]| {
            let path = folder.path().join(name);
            let mut bytes = 0x6767_6d6cu32.to_le_bytes().to_vec();
            for field in fields {
                bytes.extend(field.to_le_bytes());
            }
            fs::write(&path, bytes).expect("model");
            path
        };
        // The vocabulary, then the encoder's and the decoder's shape.
        let small = model(
            "small",
            [51865, 1500, 768, 12, 12, 448, 768, 12, 12, 80, 1009],
        );
        assert_eq!(alignment_preset(&small), Some("small"));
        let english = model("base.en", [51864, 1500, 512, 8, 6, 448, 512, 8, 6, 80, 1]);
        assert_eq!(alignment_preset(&english), Some("base.en"));
        let turbo = model(
            "turbo",
            [51866, 1500, 1280, 20, 32, 448, 1280, 20, 4, 128, 2008],
        );
        assert_eq!(alignment_preset(&turbo), Some("large.v3.turbo"));
        let third = model("v3", [51866, 1500, 1280, 20, 32, 448, 1280, 20, 32, 128, 1]);
        assert_eq!(alignment_preset(&third), Some("large.v3"));
        // The first two large models read alike, and a distilled one has
        // fewer layers than its heads are named for.
        let second = model("v2", [51865, 1500, 1280, 20, 32, 448, 1280, 20, 32, 80, 1]);
        assert_eq!(alignment_preset(&second), None);
        let distilled = model("distil", [51864, 1500, 768, 12, 12, 448, 768, 12, 4, 80, 1]);
        assert_eq!(alignment_preset(&distilled), None);
        let other = folder.path().join("other.bin");
        fs::write(
            &other,
            b"test model, and enough of it to read a header from",
        )
        .expect("other");
        assert_eq!(alignment_preset(&other), None);
        assert_eq!(alignment_preset(&folder.path().join("missing")), None);
    }

    #[test]
    fn parses_offsets_and_clamps_sidecar_padding_to_selected_range() {
        let range = TimeRange::new(5.0, 7.0).expect("range");
        let transcript = parse_whisper_json(
            &json!({
                "transcription": [{
                    "offsets": {"from": 0, "to": 3000},
                    "text": " padded"
                }]
            })
            .to_string(),
            range,
        )
        .expect("transcript");
        assert_eq!(transcript.cues[0].start, 5.0);
        assert_eq!(transcript.cues[0].end, 7.0);
        assert_eq!(transcript.cues[0].words[0].text, "padded");
    }

    #[test]
    fn cache_options_include_backend_and_model_versions() {
        let model = SourceIdentity {
            path: "/models/base.bin".into(),
            size: 12,
            modified_ns: Some(34),
        };
        let config = WhisperConfig {
            binary: "whisper-cli".into(),
            model: "/models/base.bin".into(),
            model_identity: model,
            binary_identity: json!({"path":"whisper-cli","size":1}),
            backend_version: "whisper.cpp/1".into(),
            model_version: "base-v1".into(),
            alignment: None,
        };
        let options = TranscriptionOptions::new(TimeRange::new(0.0, 2.0).expect("range"));
        let first = cache_options(&options, &config).expect("options");
        let mut changed = config.clone();
        changed.model_version = "base-v2".into();
        let second = cache_options(&options, &changed).expect("options");
        assert_ne!(first, second);
        assert_eq!(first["backendVersion"], "whisper.cpp/1");
        assert_eq!(first["modelVersion"], "base-v1");
    }

    #[test]
    fn whisper_is_given_the_threads_the_budget_has_for_it() {
        let config = WhisperConfig {
            binary: "whisper-cli".into(),
            model: "/models/base.bin".into(),
            model_identity: SourceIdentity {
                path: "/models/base.bin".into(),
                size: 12,
                modified_ns: Some(34),
            },
            binary_identity: json!({"path":"whisper-cli","size":1}),
            backend_version: "whisper.cpp/1".into(),
            model_version: "base-v1".into(),
            alignment: None,
        };
        let args = whisper_args(&config, Path::new("a.wav"), Path::new("out"), None);
        let threads = args.iter().position(|arg| arg == "-t").expect("-t");
        assert_eq!(
            args[threads + 1],
            crate::resources::whisper_thread_count().to_string()
        );
        // Half the machine, one at least and four at most.
        let threads = |cpus: usize| crate::resources::budget_for_cpus(cpus).whisper_threads;
        if std::env::var_os("KERF_WHISPER_THREADS").is_none() {
            assert_eq!(
                [1, 2, 3, 4, 6, 8, 16, 64].map(threads),
                [1, 1, 1, 2, 3, 4, 4, 4]
            );
        }
    }

    #[test]
    fn provider_backend_never_falls_back_to_a_local_sidecar() {
        let options = TranscriptionOptions {
            range: TimeRange::new(0.0, 1.0).expect("range"),
            backend: TranscriptionBackend::ExplicitProvider {
                name: "openai".into(),
            },
            language: None,
            model: None,
            model_version: None,
            backend_version: None,
            binary: None,
        };
        let error = resolve_whisper_config(&options).expect_err("provider must be unavailable");
        assert!(matches!(
            error,
            EngineError::CapabilityUnavailable { capability, .. } if capability == "transcription.provider"
        ));
    }

    #[test]
    fn progress_parser_handles_whisper_percent_output() {
        assert_eq!(parse_sidecar_progress("progress = 42%"), Some(0.42));
        assert_eq!(parse_sidecar_progress("progress: 0.5"), Some(0.5));
        assert_eq!(parse_sidecar_progress("no progress"), None);
    }

    #[test]
    fn a_range_is_reached_by_seeking_to_a_whole_second_before_it() {
        let args = |start: f64, end: f64| {
            let range = TimeRange::new(start, end).expect("range");
            extract_audio_args("in.mp4", range, Path::new("out.wav"))
        };
        // Near the top there is nothing to skip: the file is read from there.
        let near = args(2.75, 10.0);
        assert_eq!(near[3..9], ["-i", "in.mp4", "-ss", "2.75", "-t", "7.25"]);
        // Further in, FFmpeg goes two whole seconds before and drops the rest.
        let far = args(123.456, 143.5);
        assert_eq!(far[3..9], ["-ss", "121", "-i", "in.mp4", "-ss", "2.456"]);
        // The digits after the point are the range's own, not a subtraction's.
        assert_eq!(seconds_after(600.3, 598.0), "2.3");
        assert_ne!((600.3_f64 - 598.0).to_string(), "2.3");
        assert_eq!(seconds_after(3600.0, 3598.0), "2");
        assert_eq!(seek_before(2.999), None);
        assert_eq!(seek_before(3.0), Some(1.0));
    }

    /// The 16-bit samples of a WAV file FFmpeg wrote.
    fn wav_samples(path: &Path) -> Vec<i16> {
        let bytes = fs::read(path).expect("wav");
        let data = bytes
            .windows(4)
            .position(|tag| tag == b"data")
            .expect("data chunk")
            + 8;
        bytes[data..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect()
    }

    #[test]
    fn a_range_reached_by_seeking_has_the_samples_of_one_read_from_the_top() {
        let ffmpeg = resolve_ffmpeg_bin();
        let run = |args: &[String]| {
            Command::new(ffmpeg)
                .args(args)
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        };
        let root = tempdir().expect("temp directory");
        // Noise under a tone: no two moments of it sound alike.
        let sound = "anoisesrc=d=40:c=pink:r=44100:a=0.4:s=7,aformat=channel_layouts=stereo";
        let picture = "testsrc2=size=160x90:rate=30:duration=40";
        let sources: [(&str, &[&str]); 4] = [
            ("pcm.wav", &["-c:a", "pcm_s16le"]),
            ("lame.mp3", &["-c:a", "libmp3lame", "-q:a", "4"]),
            ("vorbis.ogg", &["-c:a", "libvorbis"]),
            (
                "aac.mp4",
                &["-f", "lavfi", "-i", picture, "-c:v", "libx264", "-preset"],
            ),
        ];
        let mut compared = 0;
        for (name, codec) in sources {
            let source = root.path().join(name);
            let mut make: Vec<String> = ["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"]
                .map(String::from)
                .into();
            make.push(sound.into());
            make.extend(codec.iter().map(|arg| (*arg).to_owned()));
            if name == "aac.mp4" {
                make.extend(
                    [
                        "ultrafast",
                        "-bf",
                        "2",
                        "-pix_fmt",
                        "yuv420p",
                        "-c:a",
                        "aac",
                    ]
                    .map(String::from),
                );
                make.extend(["-map", "1:v", "-map", "0:a", "-shortest"].map(String::from));
            }
            make.extend(["-y".to_owned(), source.to_string_lossy().into_owned()]);
            if !run(&make) {
                eprintln!("skipping {name}: this FFmpeg cannot make it");
                continue;
            }
            for (start, end) in [(7.3, 12.3), (21.0625, 33.0), (30.0, 39.5)] {
                let range = TimeRange::new(start, end).expect("range");
                let sought = root.path().join("sought.wav");
                let args = extract_audio_args(&source.to_string_lossy(), range, &sought);
                assert!(
                    args.iter().filter(|arg| *arg == "-ss").count() == 2,
                    "{args:?}"
                );
                assert!(run(&args), "{name} {start}");
                // As it was read before: from the top, dropping all up to the range.
                let whole = root.path().join("whole.wav");
                let mut from_top: Vec<String> =
                    args.iter().filter(|arg| *arg != "-ss").cloned().collect();
                let input = from_top.iter().position(|arg| arg == "-i").expect("-i");
                from_top.drain(input - 1..input);
                let cut = from_top.iter().position(|arg| arg == "-t").expect("-t");
                from_top[cut - 1] = start.to_string();
                from_top.insert(cut - 1, "-ss".into());
                *from_top.last_mut().expect("output") = whole.to_string_lossy().into_owned();
                assert!(run(&from_top), "{name} {start}: {from_top:?}");
                let (sought, whole) = (wav_samples(&sought), wav_samples(&whole));
                assert_eq!(sought.len(), whole.len(), "{name} from {start}");
                assert!(sought.len() as f64 > (end - start) * 15_900.0, "{name}");
                assert!(sought == whole, "{name} from {start}: the samples differ");
                compared += 1;
            }
        }
        if compared == 0 {
            eprintln!("skipping: FFmpeg is unavailable");
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_kills_running_sidecar() {
        let token = CancellationToken::new();
        let cancel = token.clone();
        let thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(60));
            cancel.cancel();
        });
        let start = Instant::now();
        let mut progress = |_| {};
        // Two commands, so every shell forks `sleep`, which outlives the
        // killed shell with its stderr.
        let result = run_sidecar(
            Path::new("sh"),
            &["-c".into(), "sleep 30; exit".into()],
            &token,
            TranscriptionStage::Transcribe,
            &mut progress,
        );
        thread.join().expect("canceller");
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    fn range(start: f64, end: f64) -> TimeRange {
        TimeRange::new(start, end).expect("range")
    }

    #[test]
    fn only_what_no_transcript_covers_is_left_to_transcribe() {
        let heard = [range(10.0, 60.0), range(60.0, 90.0), range(120.0, 150.0)];
        // Covered whole, by two transcripts that meet.
        assert_eq!(uncovered(range(20.0, 80.0), &heard), []);
        // Before, between and after.
        assert_eq!(
            uncovered(range(0.0, 200.0), &heard),
            [range(0.0, 10.0), range(90.0, 120.0), range(150.0, 200.0)]
        );
        assert_eq!(uncovered(range(95.0, 100.0), &heard), [range(95.0, 100.0)]);
        assert_eq!(uncovered(range(0.0, 5.0), &[]), [range(0.0, 5.0)]);
    }

    /// A transcript of `range` whose cue holds `words`, each with its start.
    fn piece(directory: &Path, name: &str, range: TimeRange, words: &[(f64, &str)]) -> Piece {
        let words: Vec<TranscriptWord> = words
            .iter()
            .map(|(start, text)| TranscriptWord {
                start: *start,
                end: start + 0.4,
                text: (*text).into(),
                confidence: None,
            })
            .collect();
        let text = words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        Piece {
            audio: directory.join(name),
            edges: Edges::default(),
            transcript: Transcript {
                schema_version: TRANSCRIPTION_SCHEMA_VERSION,
                range,
                language: Some("en".into()),
                text: text.clone(),
                cues: vec![TranscriptCue {
                    start: range.start,
                    end: range.end,
                    text,
                    words,
                }],
            },
        }
    }

    /// A WAV as the cache keeps one: a tone for `seconds`, silent for a
    /// quarter of a second from `quiet`.
    fn tone_with_a_pause(path: &Path, seconds: f64, quiet: f64) {
        tone_where(path, seconds, |at| !(quiet..quiet + 0.25).contains(&at));
    }

    /// A WAV as the cache keeps one: `seconds` long, a tone where `sounds`
    /// says and silent elsewhere.
    fn tone_where(path: &Path, seconds: f64, sounds: impl Fn(f64) -> bool) {
        let rate = f64::from(TRANSCRIPTION_SAMPLE_RATE);
        let samples: Vec<i16> = (0..(seconds * rate) as usize)
            .map(|index| {
                let at = index as f64 / rate;
                if sounds(at) {
                    ((at * 440.0 * std::f64::consts::TAU).sin() * 8000.0) as i16
                } else {
                    0
                }
            })
            .collect();
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + samples.len() as u32 * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(TRANSCRIPTION_SAMPLE_RATE.to_le_bytes());
        bytes.extend((TRANSCRIPTION_SAMPLE_RATE * 2).to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((samples.len() as u32 * 2).to_le_bytes());
        for sample in samples {
            bytes.extend(sample.to_le_bytes());
        }
        fs::write(path, bytes).expect("wav");
    }

    #[test]
    fn a_part_nothing_heard_is_taken_to_the_pause_nearest_it_in_what_was_heard_beside_it() {
        let folder = tempdir().expect("folder");
        // What was heard before the gap pauses from 57.6 to 57.85, and what
        // was heard after it from 92.0 to 92.25.
        let before = piece(folder.path(), "before.wav", range(10.0, 60.0), &[]);
        tone_with_a_pause(&before.audio, 50.0, 47.6);
        let after = piece(folder.path(), "after.wav", range(90.0, 150.0), &[]);
        tone_with_a_pause(&after.audio, 60.0, 2.0);
        let heard = [before, after];
        let (taken, edges) = heard_beside(range(60.0, 90.0), &heard, Some(300.0));
        assert!((57.6..57.85).contains(&taken.start), "{taken:?}");
        assert!((92.0..92.25).contains(&taken.end), "{taken:?}");
        assert!(edges.starts_in_pause && edges.ends_in_pause);

        // Where that sound has no pause, or is not kept, the gap is taken
        // five seconds in, or as far as there is of it or of the file.
        fs::write(&heard[1].audio, b"").expect("empty");
        tone_with_a_pause(&heard[0].audio, 50.0, 10.0);
        assert_eq!(
            heard_beside(range(60.0, 90.0), &heard, Some(300.0)),
            (range(55.0, 95.0), Edges::default())
        );
        assert_eq!(
            heard_beside(range(60.0, 90.0), &heard, Some(92.5)).0,
            range(55.0, 92.5)
        );
        let short = piece(folder.path(), "short.wav", range(10.0, 12.0), &[]);
        assert_eq!(
            heard_beside(range(0.0, 10.0), &[short], Some(300.0)).0,
            range(0.0, 12.0)
        );
        // With nothing heard beside it, it is transcribed as asked.
        assert_eq!(
            heard_beside(range(30.0, 40.0), &[], Some(300.0)),
            (range(30.0, 40.0), Edges::default())
        );
    }

    #[test]
    fn two_transcripts_are_joined_where_it_is_quiet_and_cut_by_the_middle_of_a_word() {
        let folder = tempdir().expect("folder");
        // The first hears 0 to 60; the second 55 to 120, where it is quiet
        // from 57.6 to 57.85 by its own sound.
        let earlier = piece(
            folder.path(),
            "earlier.wav",
            range(0.0, 60.0),
            &[
                (10.0, "one"),
                (56.0, "two"),
                (57.3, "thr"),
                (58.0, "CUT"),
                (59.7, "OFF"),
            ],
        );
        let later = piece(
            folder.path(),
            "later.wav",
            range(55.0, 120.0),
            &[
                (55.2, "LATE"),
                (57.25, "three"),
                (58.0, "four"),
                (59.7, "five"),
                (90.0, "six"),
            ],
        );
        tone_with_a_pause(&later.audio, 65.0, 2.6);
        let at = seam(&earlier, &later);
        assert!((57.6..57.85).contains(&at), "joined at {at}");

        let pieces = [earlier, later];
        let whole = stitched(range(0.0, 120.0), &pieces).expect("covered");
        // Before the seam the first transcript's words, after it the
        // second's: a word is whole in the one that gives it.
        assert_eq!(whole.text, "one two thr four five six");
        assert_eq!(whole.range, range(0.0, 120.0));
        assert_eq!(whole.language.as_deref(), Some("en"));

        // A range inside what was heard is cut out of it: a word belongs
        // to the range its middle is in, and its times stay inside.
        let inside = stitched(range(56.1, 59.8), &pieces).expect("covered");
        assert_eq!(inside.text, "two thr four");
        let words: Vec<&TranscriptWord> = inside.cues.iter().flat_map(|cue| &cue.words).collect();
        assert_eq!((words[0].start, words[0].end), (56.1, 56.4));
        assert!(inside
            .cues
            .iter()
            .all(|cue| cue.start >= 56.1 && cue.end <= 59.8));

        // Whisper's times are out by a second as often as not. Where the
        // two said the same words of what both heard, they are joined at
        // one of those words, whatever times each gave: here the second
        // puts "for example" before the quiet moment, where by its times
        // it would be lost, and the first ends on a word cut short.
        let before = piece(
            folder.path(),
            "before.wav",
            range(0.0, 60.0),
            &[
                (50.0, "they"),
                (56.0, "kept"),
                (56.6, "the"),
                (57.1, "majority."),
                (59.7, "fo"),
            ],
        );
        let after = piece(
            folder.path(),
            "later.wav",
            range(55.0, 120.0),
            &[
                (55.0, "ept"),
                (55.3, "the"),
                (55.8, "Majority"),
                (56.3, "For"),
                (56.8, "example,"),
                (61.0, "most"),
            ],
        );
        let joined = stitched(range(0.0, 120.0), &[before, after]).expect("covered");
        assert_eq!(joined.text, "they kept the majority. For example, most");

        // One made to start in a pause is whole from there, whatever times
        // its first words were given: the other gives what starts before.
        let mut from_a_pause = piece(
            folder.path(),
            "pause.wav",
            range(57.7, 120.0),
            &[(57.7, "For"), (57.9, "example,"), (61.0, "most")],
        );
        from_a_pause.edges.starts_in_pause = true;
        let to_the_pause = piece(
            folder.path(),
            "before.wav",
            range(0.0, 60.0),
            &[(50.0, "they"), (56.0, "kept"), (57.1, "it."), (59.6, "Fo")],
        );
        let mut to_the_pause = to_the_pause;
        // Whisper draws the last word before a pause out across it.
        to_the_pause.transcript.cues[0].words[2].end = 59.9;
        let met = stitched(range(0.0, 120.0), &[to_the_pause, from_a_pause]).expect("covered");
        assert_eq!(met.text, "they kept it. For example, most");
        // Their times follow the order they were put in: the last word
        // of the one ends where the first of the other starts.
        let words: Vec<&TranscriptWord> = met.cues.iter().flat_map(|cue| &cue.words).collect();
        assert_eq!((words[2].start, words[2].end), (57.1, 57.7));
        assert_eq!(words[3].start, 57.7);
        assert!(
            met.cues.windows(2).all(|pair| pair[0].end <= pair[1].start),
            "{:?}",
            met.cues
        );

        // Without the sound, or with too little heard twice, the middle.
        fs::remove_file(&pieces[1].audio).expect("remove");
        assert_eq!(seam(&pieces[0], &pieces[1]), 57.5);
        // What nothing heard cannot be answered.
        assert!(stitched(range(0.0, 130.0), &pieces).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn local_sidecar_result_and_listen_audio_reuse_the_derived_cache() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempdir().expect("temp directory");
        let source = root.path().join("source.wav");
        let ffmpeg = resolve_ffmpeg_bin();
        let available = Command::new(ffmpeg)
            .arg("-version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if !available {
            return;
        }
        let generated = Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=16000",
                "-t",
                "1.5",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "-y",
            ])
            .arg(&source)
            .status()
            .expect("generate source");
        assert!(generated.success());

        let model = root.path().join("model.bin");
        fs::write(&model, b"test model").expect("model");
        let whisper = root.path().join("whisper-cli");
        fs::write(
            &whisper,
            r##"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-of" ]; then out="$2"; shift 2
  else shift
  fi
done
printf '%s' '{"result":{"language":"en"},"transcription":[{"timestamps":{"from":"00:00:00,000","to":"00:00:00,500"},"text":" hello"}]}' > "${out}.json"
printf '%s\n' 'progress = 50%' >&2
printf 'run\n' >> "$(dirname "$0")/runs"
"##,
        )
        .expect("whisper script");
        let mut permissions = fs::metadata(&whisper)
            .expect("whisper metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&whisper, permissions).expect("whisper permissions");

        let range = TimeRange::new(0.25, 1.25).expect("range");
        let mut options = TranscriptionOptions::new(range);
        options.binary = Some(whisper);
        options.model = Some(model);
        options.model_version = Some("test-model-v1".into());
        let cache = DerivativeCache::new(root.path());
        let token = CancellationToken::new();
        let first = transcribe_cached(&source.to_string_lossy(), &cache, &options, &token, |_| {})
            .expect("transcribe");
        assert!(!first.cached);
        assert_eq!(first.transcript.text, "hello");
        assert_eq!(first.transcript.cues[0].start, 0.25);

        let second = transcribe_cached(&source.to_string_lossy(), &cache, &options, &token, |_| {})
            .expect("cached transcribe");
        assert!(second.cached);
        assert_eq!(first.key, second.key);

        let listened = listen_cached(&source.to_string_lossy(), &cache, &options, &token, |_| {})
            .expect("listen");
        assert!(listened.cached);
        assert!(listened
            .audio_path
            .as_deref()
            .is_some_and(|path| Path::new(path).is_file()));

        let runs = || {
            fs::read_to_string(root.path().join("runs"))
                .expect("runs")
                .lines()
                .count()
        };
        assert_eq!(runs(), 1, "whisper ran once for all of the above");
        // A range inside the one heard is cut out of it, with no whisper.
        let mut inside = options.clone();
        inside.range = TimeRange::new(0.25, 0.6).expect("range");
        let cut = transcribe_cached(&source.to_string_lossy(), &cache, &inside, &token, |_| {})
            .expect("inside");
        assert!(cut.cached);
        assert_eq!(cut.transcript.text, "hello");
        assert_eq!(cut.transcript.range, inside.range);
        assert_eq!(runs(), 1);
        // One that reaches past it has only what is past it transcribed,
        // and that is kept for the next to ask.
        let mut past = options.clone();
        past.range = TimeRange::new(0.5, 1.5).expect("range");
        let more = transcribe_cached(&source.to_string_lossy(), &cache, &past, &token, |_| {})
            .expect("past");
        assert!(!more.cached);
        assert_eq!(runs(), 2);
        // What was missing, 1.25 to 1.5, was heard from 0.25, where the
        // transcript beside it starts: less than five seconds in.
        let pieces = ranges(
            &cached_pieces(
                &source.to_string_lossy(),
                &cache,
                &cache_options(&options, &resolve_whisper_config(&options).expect("config"))
                    .expect("options"),
            )
            .expect("pieces"),
        );
        assert!(
            pieces.contains(&TimeRange::new(0.25, 1.5).expect("range")),
            "{pieces:?}"
        );
        // To whoever reads what was heard of the file, the two are one
        // transcript, with each word once.
        let heard = cached_transcripts(&source.to_string_lossy(), &cache).expect("cached");
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(heard[0].range, TimeRange::new(0.25, 1.5).expect("range"));
        assert_eq!(heard[0].text, "hello");
        let again = transcribe_cached(&source.to_string_lossy(), &cache, &past, &token, |_| {})
            .expect("again");
        assert!(again.cached);
        assert_eq!(runs(), 2);
    }
}
