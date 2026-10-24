//! Warp Stabilizer analyses: how a clip's frames move, found ahead of
//! drawing (EffectCraft's `effectcraft_track::stabilize`) and kept in the
//! derivative cache, keyed by the media and the frames the clip shows. A
//! renderer gets them through the effect's hidden `analysis` parameter
//! ([`attach_stabilizations`]); without one the effect shows the picture
//! as it is.
//!
//! Media clips playing forward are analysed, frame by frame as the clip
//! shows them on its sequence's grid; other clips are left unstabilized.

use crate::derivatives::{CacheEntry, DerivativeCache};
use crate::raster::{finish, read_frame};
use crate::{CancellationToken, EngineError, Result};
use effectcraft_raster::Image;
use effectcraft_track::stabilize::{AnalyzeOpts, Analyzer};
use effectcraft_track::Frame;
use kerf_model::{Clip, Content, Id, Media, Param, Project, Rate, Value};
use serde_json::json;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// The Warp Stabilizer's Kerf id.
const STABILIZER: &str = "distort.warpstabilizer";
/// The effect's hidden parameter holding the analysis (JSON).
pub const ANALYSIS: &str = "analysis";
const DETAILED: &str = "advanced/detailedAnalysis";
const FILE: &str = "analysis.json";

/// A media clip with a Warp Stabilizer, and what its analysis depends on.
struct Stabilized<'a> {
    at: (usize, usize, usize),
    clip: &'a Clip,
    media: &'a Media,
    rate: Rate,
    detailed: bool,
}

/// The clips of `project` that stabilize and can be analysed.
fn stabilized(project: &Project) -> Vec<Stabilized<'_>> {
    let mut out = Vec::new();
    for (s, sequence) in project.sequences.iter().enumerate() {
        for (t, track) in sequence.tracks.iter().enumerate() {
            for (c, clip) in track.clips.iter().enumerate() {
                let Some(effect) = clip
                    .effects
                    .iter()
                    .find(|effect| effect.enabled && effect.fx == STABILIZER)
                else {
                    continue;
                };
                let Content::Media { media, speed, .. } = &clip.content else {
                    continue;
                };
                let Some(media) = project.media(media.as_str()) else {
                    continue;
                };
                if media.video.is_none() || speed.is_reverse() {
                    continue;
                }
                let detailed = matches!(
                    effect.params.get(DETAILED),
                    Some(Param::Static(Value::Bool(true)))
                );
                out.push(Stabilized {
                    at: (s, t, c),
                    clip,
                    media,
                    rate: sequence.rate,
                    detailed,
                });
            }
        }
    }
    out
}

/// Where a clip's analysis is cached: its media, as the media is now, and
/// the frames the clip shows.
fn entry(cache: &DerivativeCache, clip: &Stabilized) -> Result<CacheEntry> {
    let Content::Media {
        source_in, speed, ..
    } = &clip.clip.content
    else {
        return Err(EngineError::msg("only media clips are analysed"));
    };
    let options = json!({
        "version": 1,
        "sourceIn": source_in.flicks(),
        "speed": speed.to_string(),
        "duration": clip.clip.duration.flicks(),
        "rate": [clip.rate.num(), clip.rate.den()],
        "detailed": clip.detailed,
    });
    cache.source_entry(Path::new(&clip.media.path), "stabilize", &options)
}

/// Put each stabilizing clip's cached analysis into its Warp Stabilizers.
/// Returns the clips with none yet. Media paths must be resolved.
pub fn attach_stabilizations(project: &mut Project, cache: &DerivativeCache) -> Vec<Id> {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for clip in stabilized(project) {
        let analysis = entry(cache, &clip)
            .ok()
            .filter(|entry| entry.is_complete(&[FILE]))
            .and_then(|entry| std::fs::read_to_string(entry.path(FILE)).ok());
        match analysis {
            Some(analysis) => found.push((clip.at, analysis)),
            None => missing.push(clip.clip.id.clone()),
        }
    }
    for ((s, t, c), analysis) in found {
        let clip = &mut project.sequences[s].tracks[t].clips[c];
        for effect in clip.effects.iter_mut().filter(|e| e.fx == STABILIZER) {
            effect.params.insert(
                ANALYSIS.to_owned(),
                Param::Static(Value::Text(analysis.clone())),
            );
        }
    }
    missing
}

/// Analyse every stabilizing clip of `project` the cache has no analysis
/// for. Media paths must be resolved.
pub fn analyze_stabilizations(
    project: &Project,
    cache: &DerivativeCache,
    cancellation: &CancellationToken,
) -> Result<()> {
    for clip in stabilized(project) {
        let entry = entry(cache, &clip)?;
        if !entry.is_complete(&[FILE]) {
            analyze(&clip, &entry, cancellation)?;
        }
    }
    Ok(())
}

/// Track the clip's frames as it shows them, at its sequence's rate, and
/// cache the analysis.
fn analyze(clip: &Stabilized, entry: &CacheEntry, cancellation: &CancellationToken) -> Result<()> {
    let Content::Media {
        source_in, speed, ..
    } = &clip.clip.content
    else {
        return Err(EngineError::msg("only media clips are analysed"));
    };
    let video = clip
        .media
        .video
        .ok_or_else(|| EngineError::msg(format!("{} has no picture", clip.media.path)))?;
    let (width, height) = (video.width, video.height);
    let count = usize::try_from(clip.rate.frames_floor(clip.clip.duration))
        .unwrap_or(0)
        .max(1);
    let rate = format!("{}/{}", clip.rate.num(), clip.rate.den());
    let ffmpeg = crate::ffmpeg::resolve_ffmpeg_bin();
    let mut decoder = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostats", "-v", "error", "-ss"])
        .arg(crate::program::secs(*source_in))
        .arg("-i")
        .arg(&clip.media.path)
        .args(["-map", "0:v:0", "-an", "-sn", "-vf"])
        .arg(format!(
            "setpts=(PTS-STARTPTS)/{},scale={width}:{height},format=rgba",
            speed.value()
        ))
        .args(["-fps_mode", "cfr", "-r", &rate, "-frames:v"])
        .arg(count.to_string())
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut analyzer = Analyzer::new(
        [f64::from(width), f64::from(height)],
        AnalyzeOpts {
            detailed: clip.detailed,
        },
    );
    let result = (|| -> Result<()> {
        let mut input = decoder.stdout.take().expect("piped");
        let mut bytes = vec![0; width as usize * height as usize * 4];
        let mut image = Image::new(width, height);
        for _ in 0..count {
            if cancellation.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            if !read_frame(&mut input, &mut bytes)? {
                break;
            }
            for (pixel, rgba) in image.data.iter_mut().zip(bytes.chunks_exact(4)) {
                let alpha = f32::from(rgba[3]) / 255.0;
                let channel = |value: u8| f32::from(value) / 255.0 * alpha;
                *pixel = [channel(rgba[0]), channel(rgba[1]), channel(rgba[2]), alpha];
            }
            analyzer.push(&Frame::new(&image));
        }
        Ok(())
    })();
    let finished = finish(&mut decoder, result.is_err());
    result?;
    finished.map_err(EngineError::msg)?;
    if analyzer.is_empty() {
        return Err(EngineError::msg(format!(
            "{} gave no frames to analyse",
            clip.media.path
        )));
    }
    // Analysis frame 0 is the clip's first frame: layer time is clip time.
    let analysis = analyzer.finish(0.0, clip.rate.frame_duration().seconds());
    let partial = entry.path("analysis.partial.json");
    std::fs::File::create(&partial)?.write_all(analysis.to_json().as_bytes())?;
    std::fs::rename(&partial, entry.path(FILE))?;
    entry.mark_complete(&[FILE])?;
    Ok(())
}
