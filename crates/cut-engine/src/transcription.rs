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

/// The transcripts already made of `src` as it is now, of any range, by
/// any model. Nothing is transcribed.
pub fn cached_transcripts(src: &str, cache: &DerivativeCache) -> Result<Vec<Transcript>> {
    let mut transcripts = Vec::new();
    for entry in cache.source_entries(Path::new(src), "transcription")? {
        if !entry.is_complete(&["transcript.json"]) {
            continue;
        }
        if let Ok(transcript) = entry.read_json::<Transcript>("transcript.json") {
            if transcript.schema_version == TRANSCRIPTION_SCHEMA_VERSION {
                transcripts.push(transcript);
            }
        }
    }
    Ok(transcripts)
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

    let audio_path = entry.path("audio.wav");
    if !audio_complete {
        on_progress(TranscriptionProgress {
            stage: TranscriptionStage::PrepareAudio,
            progress: 0.0,
        });
        extract_audio_range(
            src,
            options.range,
            &audio_path,
            cancellation,
            &mut on_progress,
        )?;
        ensure_not_cancelled(cancellation)?;
        on_progress(TranscriptionProgress {
            stage: TranscriptionStage::PrepareAudio,
            progress: 1.0,
        });
    }

    let transcript = if transcript_complete {
        read_cached_transcript(&entry, options.range)?
    } else {
        on_progress(TranscriptionProgress {
            stage: TranscriptionStage::Transcribe,
            progress: 0.0,
        });
        let output_base = entry.path("transcript");
        let args = whisper_args(
            &config,
            &audio_path,
            &output_base,
            options.language.as_deref(),
        );
        run_sidecar(
            &config.binary,
            &args,
            cancellation,
            TranscriptionStage::Transcribe,
            &mut on_progress,
        )?;
        ensure_not_cancelled(cancellation)?;
        let output_path = output_base.with_extension("json");
        let output = read_bounded_text(&output_path, MAX_TRANSCRIPT_BYTES)?;
        let transcript = parse_whisper_json(&output, options.range)?;
        ensure_not_cancelled(cancellation)?;
        entry.write_json("transcript.json", &transcript)?;
        transcript
    };

    ensure_not_cancelled(cancellation)?;
    let required = vec!["transcript.json", "audio.wav"];
    entry.mark_complete(&required)?;
    Ok(result_from_cache(
        &entry,
        transcript,
        &config,
        include_audio,
        false,
    ))
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
        .or_else(|| std::env::var_os("CUT_WHISPER_BIN").map(PathBuf::from))
        .or_else(discover_whisper_binary)
        .ok_or_else(|| {
            EngineError::capability_unavailable(
                "transcription",
                "whisper.cpp sidecar is unavailable; install whisper-cli or set CUT_WHISPER_BIN",
            )
        })?;
    let binary = resolve_path_command(binary);
    if !sidecar_available(&binary) {
        return Err(EngineError::capability_unavailable(
            "transcription",
            format!(
                "whisper.cpp sidecar is unavailable at {}; set CUT_WHISPER_BIN to whisper-cli",
                binary.display()
            ),
        ));
    }

    let model = options
        .model
        .clone()
        .or_else(|| std::env::var_os("CUT_WHISPER_MODEL").map(PathBuf::from))
        .ok_or_else(|| {
            EngineError::capability_unavailable(
                "transcription",
                "whisper.cpp model is not configured; set CUT_WHISPER_MODEL or pass a model",
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
        "CUT_WHISPER_CPP_VERSION",
        DEFAULT_WHISPER_BACKEND_VERSION,
    );
    let fallback_model_version = format!(
        "file:{}:{}",
        model_identity.size,
        model_identity.modified_ns.unwrap_or_default()
    );
    let model_version = configured_value(
        options.model_version.as_deref(),
        "CUT_WHISPER_MODEL_VERSION",
        &fallback_model_version,
    );
    Ok(WhisperConfig {
        binary,
        model,
        model_identity,
        binary_identity,
        backend_version,
        model_version,
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
    ];
    let language = language
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("auto");
    args.extend(["-l".into(), language.trim().into()]);
    args
}

fn extract_audio_range(
    src: &str,
    range: TimeRange,
    output: &Path,
    cancellation: &CancellationToken,
    on_progress: &mut impl FnMut(TranscriptionProgress),
) -> Result<()> {
    let args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        src.into(),
        "-ss".into(),
        range.start.to_string(),
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
    ];
    run_sidecar(
        Path::new(resolve_ffmpeg_bin()),
        &args,
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
    let mut words = Vec::new();
    let mut current: Option<TranscriptWord> = None;
    let mut confidence_sum = 0.0;
    let mut confidence_count = 0usize;
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
        let starts_new = current.is_some()
            && (raw_text.starts_with(char::is_whitespace) || raw_text.starts_with('▁'));
        if starts_new {
            if let Some(mut word) = current.take() {
                if confidence_count > 0 {
                    word.confidence = Some(confidence_sum / confidence_count as f64);
                }
                words.push(word);
            }
            confidence_sum = 0.0;
            confidence_count = 0;
        }
        let text = raw_text.trim().trim_start_matches('▁');
        if text.is_empty() {
            continue;
        }
        if let Some(word) = current.as_mut() {
            word.text.push_str(text);
            word.end = end.max(word.end);
        } else {
            current = Some(TranscriptWord {
                start,
                end,
                text: text.into(),
                confidence: None,
            });
        }
        if let Some(value) = confidence(object) {
            confidence_sum += value;
            confidence_count += 1;
        }
    }
    if let Some(mut word) = current {
        if confidence_count > 0 {
            word.confidence = Some(confidence_sum / confidence_count as f64);
        }
        words.push(word);
    }
    words
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
        let words = &transcript.cues[0].words;
        assert_eq!((words[0].start, words[0].end), (0.32, 0.32));
        assert_eq!((words[1].start, words[1].end), (0.33, 0.53));
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
        let result = run_sidecar(
            Path::new("sh"),
            &["-c".into(), "sleep 30".into()],
            &token,
            TranscriptionStage::Transcribe,
            &mut progress,
        );
        thread.join().expect("canceller");
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(start.elapsed() < Duration::from_secs(3));
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
    }
}
