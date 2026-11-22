//! What `look` reads of footage, a media's or the sequence's as it plays: a media's index,
//! frames, contact sheets, and what is said, and the cache the program's are kept in.

use super::tools::LookInput;
use super::{
    catalog, inline_image, io_error, program, program_position, resolve_project_source_path,
    MAX_SHEET_FRAMES,
};
use crate::{engine_error, KerfMcpRouter, McpError};
use kerf_engine::media::derivatives::{DerivativeCache, SourceIdentity};
use kerf_engine::render::{render_program, RenderOptions};
use kerf_lang::{print_document, Clock, PrintOptions};
use kerf_model::{Content, Media, Project, Time};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

/// Onsets a media index lists without a range.
const MAX_INDEX_ONSETS: usize = 100;
const SHEET_COLUMNS: usize = 4;
/// How many mixes of a program's sound (each up to 15 minutes, with its
/// transcript) the cache keeps besides the one in use; older revisions'
/// make way.
const KEPT_PROGRAM_SOUNDS: usize = 4;
/// How many captures of a program's frames the cache keeps besides the one
/// in use.
const KEPT_PROGRAM_FRAMES: usize = 32;

/// The cues of `transcript`, `offset` seconds later, on `clock`; with
/// `words`, each word's time too.
fn cues(transcript: &kerf_engine::Transcript, offset: f64, clock: &Clock, words: bool) -> Value {
    let at = |seconds: f64| print_seconds(clock, offset + seconds);
    json!(transcript
        .cues
        .iter()
        .map(|cue| {
            let mut value = json!({
                "from": at(cue.start),
                "to": at(cue.end),
                "text": cue.text.trim(),
            });
            if words {
                value["words"] = json!(cue
                    .words
                    .iter()
                    .map(|word| json!({
                        "from": at(word.start),
                        "to": at(word.end),
                        "text": word.text.trim(),
                    }))
                    .collect::<Vec<_>>());
            }
            value
        })
        .collect::<Vec<_>>())
}

/// What is said beside the stretches a transcript may have missed.
const UNHEARD_NOTE: &str = "These stretches sound like speech and the transcript has no words in them: \
    the transcriber may have left something out, as it does with a quiet voice beside a louder one. \
    It may also be music or another sound with a beat. Listen to them before trusting captions or cuts \
    made from this transcript.";

/// Where the sound of `file`, kept by the transcripts the project in `dir` has of it, reads as
/// speech with no words, within `from`–`to` seconds of it, as positions on `clock` `offset`
/// seconds later.
fn unheard(
    file: &std::path::Path,
    dir: &std::path::Path,
    (from, to): (f64, f64),
    offset: f64,
    clock: &Clock,
) -> Vec<Value> {
    kerf_engine::unheard_speech(&file.to_string_lossy(), &DerivativeCache::new(dir))
        .into_iter()
        .filter_map(|stretch| {
            let (start, end) = (stretch.start.max(from), stretch.end.min(to));
            (end - start >= 1.0).then(|| {
                json!({
                    "from": print_seconds(clock, offset + start),
                    "to": print_seconds(clock, offset + end),
                    "seconds": ((end - start) * 10.0).round() / 10.0,
                })
            })
        })
        .collect()
}

fn position(clock: &Clock, text: &str) -> Result<Time, McpError> {
    clock
        .position(text)
        .map_err(|error| McpError::new("INVALID_ARGUMENTS", error))
}

/// `seconds` from the clock's origin, on the nearest frame.
fn print_seconds(clock: &Clock, seconds: f64) -> String {
    let time = Time::from_seconds(seconds).unwrap_or(Time::ZERO);
    clock.print_position(clock.rate.round(time))
}

impl KerfMcpRouter {
    pub(super) fn media_index(
        &self,
        project: &Project,
        media: &Media,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        let clock = Clock::of_media(media);
        let mut uses = Vec::new();
        for sequence in &project.sequences {
            let at = Clock::of_sequence(sequence);
            for track in &sequence.tracks {
                for clip in &track.clips {
                    let Content::Media { media: id, .. } = &clip.content else {
                        continue;
                    };
                    if *id != media.id {
                        continue;
                    }
                    let (source_in, source_out) = clip.source_range().unwrap_or_default();
                    uses.push(json!({
                        "clip": clip.id.to_string(),
                        "sequence": sequence.id.to_string(),
                        "track": track.id.to_string(),
                        "at": at.print_position(clip.start),
                        "end": at.print_position(clip.end()),
                        "src": clock.print_position(source_in),
                        "out": clock.print_position(source_out),
                    }));
                }
            }
        }
        let path = resolve_project_source_path(&self.dir, &media.path);
        let mut result = json!({
            "media": media.id.to_string(),
            "path": media.path,
            "start": clock.print_position(Time::ZERO),
            "end": clock.print_position(media.duration),
            "uses": uses,
        });
        if let Some(takes) = self.takes(project, media) {
            if media.recipe.is_some() {
                result["recipe"] = json!(media.recipe);
            }
            result["takes"] = takes;
        }
        let index = kerf_engine::media_index(
            &path,
            &DerivativeCache::new(&self.dir),
            &kerf_engine::CancellationToken::new(),
        );
        let index_for_facts = index.as_ref().ok().cloned();
        match index {
            Ok(index) => {
                let at = |seconds: f64| print_seconds(&clock, seconds);
                result["cuts"] = json!(index.cuts.iter().map(|cut| at(*cut)).collect::<Vec<_>>());
                result["pauses"] = json!(index
                    .pauses
                    .iter()
                    .map(|(from, to)| json!({"from": at(*from), "to": at(*to)}))
                    .collect::<Vec<_>>());
                result["loudest"] = json!(index
                    .peaks
                    .iter()
                    .map(|peak| json!({"at": at(peak.at), "level": format!("{} dB", peak.level)}))
                    .collect::<Vec<_>>());
                if let Some(tempo) = &index.tempo {
                    result["tempo"] = json!({
                        "bpm": tempo.bpm,
                        "firstBeat": at(tempo.first),
                        "confidence": tempo.confidence,
                    });
                }
                let (from, to) = self.media_range(media, &clock, input)?;
                let onsets: Vec<String> = index
                    .onsets
                    .iter()
                    .filter(|onset| (from..to).contains(*onset))
                    .map(|onset| at(*onset))
                    .collect();
                let ranged = input.from.is_some() || input.to.is_some();
                if !ranged && onsets.len() > MAX_INDEX_ONSETS {
                    result["onsets"] = json!(onsets[..MAX_INDEX_ONSETS]);
                    result["moreOnsets"] = json!(format!(
                        "{} more; pass from and to for those in a range",
                        onsets.len() - MAX_INDEX_ONSETS
                    ));
                } else {
                    result["onsets"] = json!(onsets);
                }
            }
            // A media its steps make has no file until it is made, which is no fault of its line.
            Err(_) if (!media.steps.is_empty() || media.recipe.is_some()) && !path.is_file() => {
                result["analysis"] = json!(match kerf_engine::recipe::uncarried(media) {
                    Some(recipe) => format!(
                        "unavailable: `{}` is not made yet, and its steps are in the recipe file `{recipe}`: `edit` with `carry: [\"{}\"]` writes them under the media, and `make` then makes it",
                        media.path, media.id
                    ),
                    None => format!(
                        "unavailable: `{}` is not made yet; `make` makes it from its steps (`kerf make {}` where the person is asked)",
                        media.path, media.id
                    ),
                });
            }
            Err(error) => result["analysis"] = json!(format!("unavailable: {error}")),
        }
        if input.facts {
            let cache = DerivativeCache::new(&self.dir);
            let at = |seconds: f64| print_seconds(&clock, seconds);
            result["facts"] = match index_for_facts {
                Some(index) => crate::media_facts::sheet(&path, media, &index, &cache, &at),
                None => json!({"unmeasured": "its index could not be made"}),
            };
        }
        if input.transcript {
            result["transcript"] = self.transcript(&path, media, &clock, input)?;
            let (from, to) = self.media_range(media, &clock, input)?;
            let unheard = unheard(&path, &self.dir, (from, to), 0.0, &clock);
            if !unheard.is_empty() {
                result["unheard"] = json!(unheard);
                result["unheardNote"] = json!(UNHEARD_NOTE);
            }
        }
        Ok(result)
    }

    /// `from`–`to` of `media` in seconds, all of it by default.
    fn media_range(
        &self,
        media: &Media,
        clock: &Clock,
        input: &LookInput,
    ) -> Result<(f64, f64), McpError> {
        let from = match &input.from {
            Some(text) => position(clock, text)?,
            None => Time::ZERO,
        };
        let to = match &input.to {
            Some(text) => position(clock, text)?.min(media.duration),
            None => media.duration,
        };
        if from.is_negative() || to <= from {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("choose a range inside media `{}`", media.id),
            ));
        }
        Ok((from.seconds(), to.seconds()))
    }

    /// A contact sheet of `media`: `input.sheet` frames spread over its
    /// range, from its proxy when one is made and wide enough.
    pub(super) fn source_sheet(
        &self,
        media: &Media,
        input: &LookInput,
        width: u32,
    ) -> Result<Value, McpError> {
        if media.video.is_none() {
            return Err(McpError::new(
                "NO_VIDEO",
                format!("media `{}` has no picture", media.id),
            ));
        }
        let clock = Clock::of_media(media);
        let (from, to) = self.media_range(media, &clock, input)?;
        let count = input.sheet.unwrap_or(MAX_SHEET_FRAMES);
        let times: Vec<f64> = (0..count)
            .map(|index| {
                let time =
                    Time::from_seconds(from + (to - from) * (index as f64 + 0.5) / count as f64)
                        .unwrap_or(Time::ZERO);
                media.rate.round(time).seconds()
            })
            .collect();
        let grabbed = kerf_engine::cached_exact_frames(
            &self.viewable(media, width).to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &times,
            width,
        )
        .map_err(engine_error)?;
        let paths: Vec<String> = grabbed
            .frames
            .iter()
            .map(|frame| frame.path.clone())
            .collect();
        let labels: Vec<String> = times
            .iter()
            .map(|time| print_seconds(&clock, *time))
            .collect();
        // Each frame with its time on it. The frames are kept by what they are of; the sheet
        // beside them by what it says on them.
        let sheet = Path::new(&paths[0]).with_file_name("sheet-timed.jpg");
        if !sheet.is_file() {
            kerf_engine::create_labelled_contact_sheet(
                &paths,
                &sheet,
                SHEET_COLUMNS.min(count),
                &labels,
                &sheet_font(),
            )
            .map_err(engine_error)?;
        }
        let mut result = json!({
            "media": media.id.to_string(),
            "frames": labels,
            "path": sheet,
        });
        inline_image(&mut result)?;
        Ok(result)
    }

    /// A contact sheet of the sequence: `input.sheet` program frames spread
    /// over its range.
    pub(super) fn program_sheet(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
        width: u32,
    ) -> Result<Value, McpError> {
        let sequence = program(project, input.sequence.as_deref())?;
        let clock = Clock::of_sequence(sequence);
        let duration = sequence.duration();
        let from = match &input.from {
            Some(text) => program_position(project, sequence, text)?,
            None => Time::ZERO,
        };
        let to = match &input.to {
            Some(text) => program_position(project, sequence, text)?.min(duration),
            None => duration,
        };
        if from.is_negative() || to <= from {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("choose a range inside sequence `{}`", sequence.id),
            ));
        }
        let count = input.sheet.unwrap_or(MAX_SHEET_FRAMES);
        let span = (to - from).seconds();
        let mut times: Vec<Time> = (0..count)
            .map(|index| {
                let offset = Time::from_seconds(span * (index as f64 + 0.5) / count as f64)
                    .unwrap_or(Time::ZERO);
                sequence
                    .rate
                    .round(from + offset)
                    .min(duration - sequence.rate.frame_duration())
            })
            .collect();
        times.dedup();
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let capture = self.capture_frames(
            &resolved,
            sequence.id.as_str(),
            &times,
            width,
            Some(SHEET_COLUMNS.min(times.len())),
        )?;
        let mut result = json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "frames": times.iter().map(|time| clock.print_position(*time)).collect::<Vec<_>>(),
            "path": capture.sheet,
        });
        inline_image(&mut result)?;
        Ok(result)
    }

    /// The file to take frames of `media` from at `width`: its proxy when
    /// one is made and wide enough, or else the media itself.
    fn viewable(&self, media: &Media, width: u32) -> PathBuf {
        let path = resolve_project_source_path(&self.dir, &media.path);
        if width > kerf_engine::PROXY_WIDTH {
            return path;
        }
        kerf_engine::cached_proxy(&path, &DerivativeCache::new(&self.dir)).unwrap_or(path)
    }

    fn transcript(
        &self,
        path: &std::path::Path,
        media: &Media,
        clock: &Clock,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        if media.audio.is_none() {
            return Err(McpError::new(
                "NO_AUDIO",
                format!("media `{}` has no audio", media.id),
            ));
        }
        let from = match &input.from {
            Some(text) => position(clock, text)?,
            None => Time::ZERO,
        };
        let to = match &input.to {
            Some(text) => position(clock, text)?,
            None => media.duration,
        };
        let (from, to) = (from.seconds(), to.seconds().min(media.duration.seconds()));
        if to - from > kerf_engine::MAX_TRANSCRIPTION_RANGE_SECONDS + 1e-9 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "a transcript covers at most 15 minutes; choose a range with from and to",
            ));
        }
        let range = kerf_engine::TimeRange::new(from, to)
            .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
        let result = kerf_engine::transcribe_cached(
            &path.to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &kerf_engine::TranscriptionOptions::new(range),
            &kerf_engine::CancellationToken::new(),
            |_| {},
        )
        .map_err(engine_error)?;
        Ok(cues(&result.transcript, 0.0, clock, input.words))
    }

    /// What the sequence says from `from` to `to`, as it plays: its sound
    /// is mixed, with no picture drawn, and transcribed. Times are sequence
    /// positions.
    pub(super) fn program_transcript(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        let sequence = program(project, input.sequence.as_deref())?;
        let clock = Clock::of_sequence(sequence);
        let duration = sequence.duration();
        let from = match &input.from {
            Some(text) => program_position(project, sequence, text)?,
            None => Time::ZERO,
        };
        let to = match &input.to {
            Some(text) => program_position(project, sequence, text)?.min(duration),
            None => duration,
        };
        if from.is_negative() || to <= from {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("choose a range inside sequence `{}`", sequence.id),
            ));
        }
        if (to - from).seconds() > kerf_engine::MAX_TRANSCRIPTION_RANGE_SECONDS + 1e-9 {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "a transcript covers at most 15 minutes; choose a range with from and to",
            ));
        }
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        kerf_runtime::use_proxies(&mut resolved, &self.dir);
        let entry = self.program_entry(
            &resolved,
            sequence.id.as_str(),
            "program-sound",
            &json!({"from": from.flicks(), "to": to.flicks()}),
        )?;
        let sound = entry.path("sound.wav");
        if !entry.is_complete(&["sound.wav"]) {
            kerf_engine::render::render_sound(
                &resolved,
                Some(sequence.id.as_str()),
                Some((from, to)),
                kerf_engine::TRANSCRIPTION_SAMPLE_RATE,
                &sound,
                &kerf_engine::CancellationToken::new(),
            )
            .map_err(engine_error)?;
            entry.mark_complete(&["sound.wav"]).map_err(engine_error)?;
        }
        // A program's sound is mixed for each revision listened to.
        let _ = DerivativeCache::new(&self.dir).keep_newest(&entry, KEPT_PROGRAM_SOUNDS);
        let range = kerf_engine::TimeRange::new(0.0, (to - from).seconds())
            .map_err(|error| McpError::new("INVALID_ARGUMENTS", error.to_string()))?;
        let result = kerf_engine::transcribe_cached(
            &sound.to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &kerf_engine::TranscriptionOptions::new(range),
            &kerf_engine::CancellationToken::new(),
            |_| {},
        )
        .map_err(engine_error)?;
        let mut heard = json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "from": clock.print_position(from),
            "to": clock.print_position(to),
            "transcript": cues(&result.transcript, from.seconds(), &clock, input.words),
        });
        let unheard = unheard(
            &sound,
            &self.dir,
            (0.0, (to - from).seconds()),
            from.seconds(),
            &clock,
        );
        if !unheard.is_empty() {
            heard["unheard"] = json!(unheard);
            heard["unheardNote"] = json!(UNHEARD_NOTE);
        }
        Ok(heard)
    }

    /// A cache entry for what `sequence` of `project`, whose media paths
    /// are resolved, renders to; refused when it cannot be rendered. Its
    /// key is the whole document, so each revision makes new ones: keep a
    /// few with [`DerivativeCache::keep_newest`].
    pub(super) fn program_entry(
        &self,
        project: &Project,
        sequence: &str,
        kind: &str,
        options: &Value,
    ) -> Result<kerf_engine::media::derivatives::CacheEntry, McpError> {
        kerf_engine::render::renderable(project, Some(sequence))
            .map_err(|error| McpError::new("UNRENDERABLE", error.to_string()))?;
        let document = print_document(project, catalog(), PrintOptions::default());
        let identities = crate::qc::used_media(project)
            .map(|media| {
                SourceIdentity::from_path(std::path::Path::new(&media.path)).map_err(|error| {
                    McpError::new(
                        "MEDIA_UNAVAILABLE",
                        format!("media `{}` cannot be read: {error}", media.id),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut options = options.clone();
        options["sequence"] = json!(sequence);
        DerivativeCache::new(&self.dir)
            .project_entry(&json!(document), &identities, kind, &options)
            .map_err(engine_error)
    }

    pub(super) fn source_frames(
        &self,
        media: &Media,
        at: &[String],
        width: u32,
    ) -> Result<Value, McpError> {
        if media.video.is_none() {
            return Err(McpError::new(
                "NO_VIDEO",
                format!("media `{}` has no picture", media.id),
            ));
        }
        let clock = Clock::of_media(media);
        let times = at
            .iter()
            .map(|text| {
                let time = position(&clock, text)?;
                if time.is_negative() || time >= media.duration {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        format!("`{text}` is outside media `{}`", media.id),
                    ));
                }
                Ok(time.seconds())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let grabbed = kerf_engine::cached_exact_frames(
            &self.viewable(media, width).to_string_lossy(),
            &DerivativeCache::new(&self.dir),
            &times,
            width,
        )
        .map_err(engine_error)?;
        let mut frames = Vec::new();
        for (text, frame) in at.iter().zip(grabbed.frames) {
            let mut value = json!({"at": text, "path": frame.path});
            inline_image(&mut value)?;
            frames.push(value);
        }
        Ok(json!({"media": media.id.to_string(), "frames": frames}))
    }

    pub(super) fn program_frames(
        &self,
        project: &Project,
        sequence: Option<&str>,
        revision: u64,
        at: &[String],
        width: u32,
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let duration = sequence.duration();
        let times = at
            .iter()
            .map(|text| {
                let time = program_position(project, sequence, text)?;
                if time.is_negative() || time >= duration {
                    return Err(McpError::new(
                        "INVALID_ARGUMENTS",
                        format!(
                            "`{text}` is outside sequence `{}`{}",
                            sequence.id,
                            self.any_pending("proposal")
                        ),
                    ));
                }
                Ok(time)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let capture = self.capture_frames(&resolved, sequence.id.as_str(), &times, width, None)?;
        let mut frames = Vec::new();
        for (text, path) in at.iter().zip(capture.frames) {
            let mut value = json!({"at": text, "path": path});
            inline_image(&mut value)?;
            frames.push(value);
        }
        Ok(json!({"sequence": sequence.id.to_string(), "revision": revision, "frames": frames}))
    }

    /// Render the program at each of `times` at full size and quality and
    /// take its frame there, at most `width` wide; with `columns`, also a
    /// contact sheet of them. `project` has its media paths resolved.
    /// Results are cached by document, media identity, and options.
    pub(super) fn capture_frames(
        &self,
        project: &Project,
        sequence: &str,
        times: &[Time],
        width: u32,
        columns: Option<usize>,
    ) -> Result<Capture, McpError> {
        let entry = self.program_entry(
            project,
            sequence,
            "program-frames",
            &json!({
                "times": times.iter().map(|time| time.flicks()).collect::<Vec<_>>(),
                "maxWidth": width,
                "columns": columns,
            }),
        )?;
        let sequence = program(project, Some(sequence))?;
        let names: Vec<String> = (0..times.len())
            .map(kerf_engine::media::probe::frame_file_name)
            .collect();
        let mut required: Vec<&str> = names.iter().map(String::as_str).collect();
        if columns.is_some() {
            required.push("sheet.jpg");
        }
        if !entry.is_complete(&required) {
            let frame = sequence.rate.frame_duration();
            let duration = sequence.duration();
            // One frame long render of each time, so a frame never comes
            // from a draft's lower size or rate.
            let render = |index: usize| -> Result<(), McpError> {
                let start = times[index];
                let clip = entry.path(&format!("frame-{index:03}.mp4"));
                render_program::<fn(f64)>(
                    project,
                    Some(sequence.id.as_str()),
                    Some((start, (start + frame).min(duration))),
                    &RenderOptions {
                        output: clip.to_string_lossy().into_owned(),
                        force_software: true,
                        project_dir: Some(self.dir.clone()),
                        ..RenderOptions::default()
                    },
                    None,
                )
                .map_err(engine_error)?;
                let directory = entry.path(&format!("frame-{index:03}"));
                let grabbed = kerf_engine::media::probe::extract_exact_frames(
                    &clip.to_string_lossy(),
                    &[0.0],
                    &directory,
                    width,
                )
                .map_err(engine_error)?;
                fs::rename(&grabbed[0].path, entry.path(&names[index])).map_err(io_error)?;
                let _ = fs::remove_dir_all(directory);
                let _ = fs::remove_file(clip);
                Ok(())
            };
            let render = &render;
            thread::scope(|scope| {
                (0..times.len())
                    .map(|index| scope.spawn(move || render(index)))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .try_for_each(|handle| handle.join().expect("frame render panicked"))
            })?;
            if let Some(columns) = columns {
                let paths: Vec<String> = names
                    .iter()
                    .map(|name| entry.path(name).to_string_lossy().into_owned())
                    .collect();
                // Each frame with its time on it, as the sequence counts time.
                let clock = Clock::of_sequence(sequence);
                let labels: Vec<String> = times
                    .iter()
                    .map(|time| clock.print_position(*time))
                    .collect();
                kerf_engine::create_labelled_contact_sheet(
                    &paths,
                    &entry.path("sheet.jpg"),
                    columns.min(times.len()),
                    &labels,
                    &sheet_font(),
                )
                .map_err(engine_error)?;
            }
            entry.mark_complete(&required).map_err(engine_error)?;
        }
        // Every edit's preview captures two revisions.
        let _ = DerivativeCache::new(&self.dir).keep_newest(&entry, KEPT_PROGRAM_FRAMES);
        Ok(Capture {
            frames: names
                .iter()
                .map(|name| entry.path(name).to_string_lossy().into_owned())
                .collect(),
            sheet: columns.map(|_| entry.path("sheet.jpg").to_string_lossy().into_owned()),
        })
    }
}

/// Frames of the program, and their contact sheet when one was asked for.
pub(super) struct Capture {
    pub(super) frames: Vec<String>,
    pub(super) sheet: Option<String>,
}

/// The font file a sheet's times are drawn in: Kerf's own Inter, written out once where
/// ffmpeg can read it.
fn sheet_font() -> PathBuf {
    static FONT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        let font = kerf_text::fonts::INTER_MEDIUM;
        let path = std::env::temp_dir().join(format!("kerf-inter-medium-{}.ttf", font.len()));
        if fs::metadata(&path).map_or(true, |found| found.len() != font.len() as u64) {
            // Beside it first, so that no reader finds half a font.
            let partial = path.with_extension(format!("{}.part", std::process::id()));
            if fs::write(&partial, font).is_ok() && fs::rename(&partial, &path).is_err() {
                let _ = fs::remove_file(&partial);
            }
        }
        path
    })
    .clone()
}
