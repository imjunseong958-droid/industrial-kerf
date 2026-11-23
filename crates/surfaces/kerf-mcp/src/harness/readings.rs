//! What `look` measures of a sequence: what drawing each frame costs, the titles and captions
//! too faint to read, how its cuts sit in its sound, how loud it plays, what scopes read, what
//! shows of each clip on a frame, what each clip in depth stands on, and where the eye is
//! either side of its cuts; and how recordings line up by their sound.

use super::tools::LookInput;
use super::{catalog, program, program_position, resolve_project_source_path, SCOPE_WIDTH};
use crate::{engine_error, KerfMcpRouter, McpCode, McpError};
use kerf_engine::media::derivatives::DerivativeCache;
use kerf_lang::Clock;
use kerf_model::{Project, Sequence, Time};
use kerf_runtime::CachedFacts;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// How many measurements of a program's loudness the cache keeps besides
/// the one in use.
const KEPT_PROGRAM_LOUDNESS: usize = 16;

impl KerfMcpRouter {
    /// Who draws each frame at `at`, why, and what it costs.
    pub(super) fn frame_costs(
        &self,
        project: &Project,
        sequence: Option<&str>,
        revision: u64,
        at: &[String],
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let duration = sequence.duration();
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let compositor = kerf_compose::Compositor::new(
            &resolved,
            catalog(),
            &kerf_compose::ComposeOptions {
                sequence: Some(sequence.id.to_string()),
                accel: kerf_compose::Accel::Always,
                ..Default::default()
            },
        )
        .map_err(|error| McpError::new(McpCode::RenderFailed, error.to_string()))?;
        // The middle of three, the first having had a decoder to open. Only how long a
        // frame takes is wanted, not the frame.
        let median_ms = |at: Time, scale: f64| {
            let _ = compositor.frame(at, scale);
            let mut times: Vec<f64> = (0..3)
                .map(|_| {
                    let started = std::time::Instant::now();
                    let _ = compositor.frame(at, scale);
                    started.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            times.sort_by(f64::total_cmp);
            (times[1] * 10.0).round() / 10.0
        };
        let mut costs = Vec::with_capacity(at.len());
        for text in at {
            let time = program_position(project, sequence, text)?;
            if time.is_negative() || time >= duration {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    format!(
                        "`{text}` is outside sequence `{}`{}",
                        sequence.id,
                        self.any_pending("proposal")
                    ),
                ));
            }
            let because = compositor.why_on_cpu(time, 1.0);
            costs.push(json!({
                "at": text,
                "draws": if because.is_empty() { "gpu" } else { "cpu" },
                "because": because,
                "ms": median_ms(time, 1.0),
                "msHalf": median_ms(time, 0.5),
            }));
        }
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "size": format!("{}x{}", sequence.width, sequence.height),
            "frameMs": (1000.0 / sequence.rate.fps() * 10.0).round() / 10.0,
            "costs": costs,
        }))
    }

    /// What shows of each clip on each frame at `at`.
    pub(super) fn frames_show(
        &self,
        project: &Project,
        sequence: Option<&str>,
        revision: u64,
        at: &[String],
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let duration = sequence.duration();
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let mut frames = Vec::with_capacity(at.len());
        for text in at {
            let time = program_position(project, sequence, text)?;
            if time.is_negative() || time >= duration {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    format!(
                        "`{text}` is outside sequence `{}`{}",
                        sequence.id,
                        self.any_pending("proposal")
                    ),
                ));
            }
            let showing =
                kerf_engine::analysis::shows::showing(&resolved, Some(sequence.id.as_str()), time)
                    .map_err(|error| McpError::new(McpCode::RenderFailed, error.to_string()))?;
            let clips: Vec<Value> = showing
                .clips
                .iter()
                .map(|shown| {
                    let mut clip = json!({
                        "clip": shown.clip,
                        "area": shown.area,
                        "shows": shown.shows,
                        "hidden": shown.hidden,
                    });
                    if let Some(at) = shown.at {
                        clip["box"] = json!(at);
                    }
                    if !shown.by.is_empty() {
                        clip["by"] = shown
                            .by
                            .iter()
                            .map(|(by, share)| json!({"clip": by, "share": share}))
                            .collect();
                    }
                    if !shown.edges.is_empty() {
                        clip["edges"] = json!(shown.edges);
                    }
                    if let Some(opacity) = shown.opacity {
                        clip["opacity"] = json!(opacity);
                    }
                    clip
                })
                .collect();
            let mut frame = json!({"at": text, "clips": clips, "empty": showing.empty});
            if !showing.unread.is_empty() {
                frame["unread"] = json!(showing.unread);
            }
            frames.push(frame);
        }
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "frames": frames,
        }))
    }

    /// What each clip in depth stands on: on the frames at `at`, or with none on every frame.
    pub(super) fn clips_stand(
        &self,
        project: &Project,
        sequence: Option<&str>,
        at: &[String],
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let duration = sequence.duration();
        let mut times = Vec::with_capacity(at.len());
        for text in at {
            let time = program_position(project, sequence, text)?;
            if time.is_negative() || time >= duration {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    format!(
                        "`{text}` is outside sequence `{}`{}",
                        sequence.id,
                        self.any_pending("proposal")
                    ),
                ));
            }
            times.push(time);
        }
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let standing =
            kerf_engine::analysis::stands::standing(&resolved, Some(sequence.id.as_str()), &times)
                .map_err(|error| McpError::new(McpCode::RenderFailed, error.to_string()))?;
        Ok(crate::stands::said(&standing))
    }

    /// Where the eye is either side of each cut of the sequence's picture: `subjects` says
    /// what it is on in a shot, by the shot's media, where its steps do not.
    pub(super) fn cuts_seen(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
        subjects: &BTreeMap<String, String>,
    ) -> Result<Value, McpError> {
        let sequence = program(project, input.sequence.as_deref())?;
        if let Some(media) = subjects.keys().find(|media| project.media(media).is_none()) {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                format!(
                    "`subjects` names the media `{media}`, which the project does not have: each is a shot's media, with the clip of its guide the eye is on{}",
                    self.in_pending(media, "proposal")
                ),
            ));
        }
        let range = if input.from.is_some() || input.to.is_some() {
            let at = |text: &Option<String>, otherwise: Time| match text {
                Some(text) => program_position(project, sequence, text),
                None => Ok(otherwise),
            };
            Some((
                at(&input.from, Time::ZERO)?,
                at(&input.to, sequence.duration())?,
            ))
        } else {
            None
        };
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let seen = kerf_engine::analysis::eye::eye(
            &resolved,
            Some(sequence.id.as_str()),
            range,
            subjects,
            &mut kerf_engine::analysis::eye::file_picture,
        )
        .map_err(|error| McpError::new(McpCode::RenderFailed, error.to_string()))?;
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "eye": crate::eye::said(&seen, &Clock::of_sequence(sequence)),
        }))
    }

    /// How recordings fall against each other by their sound, and a sequence with a camera on each
    /// track for them to be a multicam of.
    pub(super) fn line_up(
        &self,
        project: &Project,
        revision: u64,
        ids: &[String],
    ) -> Result<Value, McpError> {
        let invalid = |message: String| McpError::new(McpCode::InvalidArguments, message);
        if ids.len() < 2 {
            return Err(invalid("`sync` lines up two or more media".to_owned()));
        }
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let mut media = Vec::with_capacity(ids.len());
        for id in ids {
            let found = resolved
                .media(id)
                .ok_or_else(|| invalid(format!("no media `{id}`")))?;
            if found.audio.is_none() {
                return Err(invalid(format!("`{id}` has no sound to line up by")));
            }
            media.push(found);
        }
        let paths: Vec<&std::path::Path> = media
            .iter()
            .map(|media| std::path::Path::new(&media.path))
            .collect();
        let found = kerf_engine::analysis::sync::align_sound(&paths)
            .map_err(|error| McpError::new(McpCode::RenderFailed, error.to_string()))?;
        let first = media[0];
        let rate = first.rate;
        let lined = crate::lined_up::lined_up(&found, rate.frame_duration().seconds());
        // Times start with whichever of those placed began first.
        let placed = || lined.iter().filter(|lined| lined.left.is_none());
        let earliest = placed().map(|lined| lined.offset).fold(0.0, f64::min);
        // Each on a whole frame from the first, so that none is more than half a frame from
        // where its offset has it.
        let frames = |seconds: f64| (seconds * rate.fps()).round() as i64;
        let mut id = "multicam".to_owned();
        let mut number = 1;
        while project.sequence(&id).is_some() {
            number += 1;
            id = format!("multicam{number}");
        }
        let (width, height) = first
            .video
            .map_or((1920, 1080), |video| (video.width, video.height));
        let mut lines = vec![
            format!("sequence add {id} rate={rate} size={width}x{height} name=\"Multicam\""),
            format!("use {id}"),
        ];
        // The document says how each lines up with the first, which a check holds clips to. A
        // first that was lined up with one of these is let go, since they are now with it.
        if media[1..]
            .iter()
            .any(|media| project.sync_comes_back(&media.id, &first.id))
        {
            lines.push(format!("media sync {} with=\"\"", first.id));
        }
        let mut rows = Vec::with_capacity(ids.len());
        for (index, (media, lined)) in media.iter().zip(&lined).enumerate() {
            let mut row = lined.row.clone();
            row["media"] = json!(media.id.as_str());
            if lined.left.is_none() {
                let at = frames(lined.offset) - frames(earliest);
                let camera = index + 1;
                // Whole frames of the sequence within the media: the nearer one can be past
                // the end of media at another rate.
                let dur = rate.frames_floor(media.duration).max(1);
                let mut targets = Vec::new();
                if media.video.is_some() {
                    lines.push(format!("track add V{camera} video"));
                    targets.push(format!("V{camera}"));
                }
                lines.push(format!("track add A{camera} audio"));
                targets.push(format!("A{camera}"));
                lines.push(format!(
                    "overwrite media={} src=0f at={at}f dur={dur}f on={}",
                    media.id,
                    targets.join(",")
                ));
                if index > 0 {
                    lines.push(format!(
                        "media sync {} with={}@{}s",
                        media.id,
                        first.id,
                        crate::lined_up::milliseconds(lined.offset)
                    ));
                }
                row["at"] = json!(format!("{at}f"));
            }
            rows.push(row);
        }
        let mut answer = json!({"revision": revision, "recordings": rows});
        // A sequence of one camera lines nothing up.
        if placed().count() >= 2 {
            answer["sequence"] = json!(id);
            answer["script"] = json!(lines.join("\n"));
        }
        Ok(answer)
    }

    /// Which of the sequence's titles and captions are too faint against what is behind them.
    pub(super) fn program_contrast(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        let sequence = program(project, input.sequence.as_deref())?;
        let duration = sequence.duration();
        let moment = |text: &Option<String>, otherwise: Time| match text {
            Some(text) => program_position(project, sequence, text),
            None => Ok(otherwise),
        };
        let (from, to) = (
            moment(&input.from, Time::ZERO)?,
            moment(&input.to, duration)?,
        );
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let mut findings = Vec::new();
        let (read, of) =
            crate::text_lint::contrast_of(&resolved, sequence, (from, to), |_| true, &mut findings);
        // The same frames say what the edge of the frame cuts off.
        let (clipped, findings): (Vec<_>, Vec<_>) = findings
            .into_iter()
            .partition(|finding| finding.code == "text.clipped");
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "read": read,
            "of": of,
            "clipped": clipped,
            "contrast": findings,
        }))
    }

    /// How the cuts of a sequence's sound sit in the sound (`cuts::report`). A media the
    /// sequence's sound plays that has no index yet is indexed first, as `look` with `media`
    /// would, since a cut is measured by the levels an index keeps.
    pub(super) fn program_cuts(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
        pauses: f64,
    ) -> Result<Value, McpError> {
        if input.media.is_some()
            || !input.at.is_empty()
            || input.transcript
            || input.loudness
            || input.sheet.is_some()
        {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "`cuts` is of a sequence's sound: use it with `sequence` or `proposal`, and \
                 `from` and `to` for a part of it",
            ));
        }
        let sequence = program(project, input.sequence.as_deref())?;
        let range = if input.from.is_some() || input.to.is_some() {
            let at = |text: &Option<String>, otherwise: Time| match text {
                Some(text) => program_position(project, sequence, text),
                None => Ok(otherwise),
            };
            let (from, to) = (
                at(&input.from, Time::ZERO)?,
                at(&input.to, sequence.duration())?,
            );
            if to <= from {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`to` must come after `from`",
                ));
            }
            Some((from, to))
        } else {
            None
        };
        let cache = DerivativeCache::new(&self.dir);
        let mut indexed = Vec::new();
        for cut in crate::cuts::sound_cuts(project, sequence) {
            if indexed.contains(&&cut.media.id) {
                continue;
            }
            indexed.push(&cut.media.id);
            let path = resolve_project_source_path(&self.dir, &cut.media.path);
            if kerf_engine::cached_media_index(&path, &cache).is_none() {
                // One that cannot be read is said by the report, as not measured, and why
                // it could not be is logged.
                if let Err(error) =
                    kerf_engine::media_index(&path, &cache, &kerf_engine::CancellationToken::new())
                {
                    log::warn!("{} was not indexed for its cuts: {error}", path.display());
                }
            }
        }
        let mut report = crate::cuts::report(
            project,
            sequence,
            &CachedFacts::new(&self.dir),
            &crate::cuts::Asked { pauses, range },
        );
        report["sequence"] = json!(sequence.id.to_string());
        report["revision"] = json!(revision);
        Ok(report)
    }

    /// How loud the sequence is as it plays, by R128: its whole mix through
    /// the meter a delivery is checked with, kept for the revision.
    pub(super) fn program_loudness(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        if input.media.is_some()
            || !input.at.is_empty()
            || input.from.is_some()
            || input.to.is_some()
            || input.transcript
            || input.sheet.is_some()
        {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "`loudness` is of a whole sequence as it plays: use it with `sequence` or \
                 `proposal` only; a media's own loudness is in its `facts`",
            ));
        }
        let sequence = program(project, input.sequence.as_deref())?;
        let clock = Clock::of_sequence(sequence);
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let entry = self.program_entry(
            &resolved,
            sequence.id.as_str(),
            "program-loudness",
            &json!({}),
        )?;
        let sound: kerf_engine::Loudness = if entry.is_complete(&["loudness.json"]) {
            entry.read_json("loudness.json").map_err(engine_error)?
        } else {
            let sound = kerf_engine::measure_program(
                &resolved,
                Some(sequence.id.as_str()),
                &self.dir,
                &kerf_engine::CancellationToken::new(),
            )
            .map_err(engine_error)?;
            entry
                .write_json("loudness.json", &sound)
                .map_err(engine_error)?;
            entry
                .mark_complete(&["loudness.json"])
                .map_err(engine_error)?;
            sound
        };
        // A measurement is made for each revision asked about.
        if let Err(error) =
            DerivativeCache::new(&self.dir).keep_newest(&entry, KEPT_PROGRAM_LOUDNESS)
        {
            log::warn!("older loudness measurements were not cleared from the cache: {error}");
        }
        // Two places, as a meter reads; silence has no level.
        let level = |value: f64| {
            if value.is_finite() && value > -120.0 {
                json!((value * 100.0).round() / 100.0)
            } else {
                Value::Null
            }
        };
        let ends = |seconds: Option<f64>| {
            seconds
                .and_then(Time::from_seconds)
                .map(|time| clock.print_position(sequence.rate.round(time)))
        };
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "loudness": {
                "integrated": level(sound.integrated),
                "target": kerf_engine::analysis::loudness::TARGET_LUFS,
                "range": level(sound.lra),
                "truePeak": level(sound.true_peak_max),
                "limit": kerf_engine::analysis::loudness::TRUE_PEAK_LIMIT_DBTP,
                "shortTermMax": {"lufs": level(sound.short_term_max), "ends": ends(sound.short_term_max_at())},
                "momentaryMax": {"lufs": level(sound.momentary_max), "ends": ends(sound.momentary_max_at())},
                "seconds": (sound.duration * 1000.0).round() / 1000.0,
            },
        }))
    }

    /// What scopes read of the program at each of `at`.
    /// What scopes read of the sequence at each of `at`. `before` is the project when
    /// `project` is what a proposal would make of it: each reading then also says what the
    /// project reads at that position, where it has one, so a grade is compared in one look.
    pub(super) fn program_scopes(
        &self,
        project: &Project,
        before: Option<&Project>,
        sequence: Option<&str>,
        revision: u64,
        at: &[String],
    ) -> Result<Value, McpError> {
        let sequence = program(project, sequence)?;
        let duration = sequence.duration();
        // What scopes read of `sequence` of `project` at `time`.
        let read = |project: &Project, sequence: &Sequence, time: Time| {
            let mut resolved = project.clone();
            kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
            // Scopes read a few hundred samples across, so a smaller frame reads the same.
            let scale = (SCOPE_WIDTH / f64::from(sequence.width.max(1))).min(1.0);
            // An HDR sequence is read by the signal it masters to, and not what a monitor
            // shows of it.
            let hdr = match sequence.color.curve() {
                kerf_color::Curve::Pq => Some(kerf_scopes::SignalCurve::Pq),
                kerf_color::Curve::Hlg => Some(kerf_scopes::SignalCurve::Hlg),
                _ => None,
            };
            let id = Some(sequence.id.as_str());
            let drawn = match hdr {
                Some(_) => kerf_engine::render::still::draw_signal(&resolved, id, time, scale),
                None => kerf_engine::render::still::draw_frame(&resolved, id, time, scale),
            };
            let frame =
                drawn.map_err(|error| McpError::new(McpCode::RenderFailed, error.to_string()))?;
            let (w, h) = (frame.width as usize, frame.height as usize);
            match hdr {
                Some(curve) => kerf_scopes::read_signal(&frame.rgba, w, h, curve),
                None => kerf_scopes::read(&frame.rgba, w, h),
            }
            .ok_or_else(|| McpError::new(McpCode::RenderFailed, "the frame came back empty"))
        };
        // The same sequence as the project has it, when it has it.
        let before =
            before.and_then(|project| Some((project, project.sequence(sequence.id.as_str())?)));
        let mut readings = Vec::new();
        for text in at {
            let time = program_position(project, sequence, text)?;
            if time.is_negative() || time >= duration {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    format!("`{text}` is outside sequence `{}`", sequence.id),
                ));
            }
            let mut reading = json!({"at": text, "scopes": read(project, sequence, time)?});
            if let Some((project, sequence)) = before
                && time < sequence.duration()
            {
                reading["before"] = json!(read(project, sequence, time)?);
            }
            readings.push(reading);
        }
        Ok(json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "readings": readings,
        }))
    }
}
