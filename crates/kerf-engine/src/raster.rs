//! Baking raster effects: a media clip's source frames through its raster
//! effects into a lossless file that the graph plays in the clip's place.
//! Which clips need it, and how the project changes, is
//! `kerf_core::raster`; this makes the files, cached by source and job.

use crate::derivatives::{CacheEntry, DerivativeCache};
use crate::error::{EngineError, Result};
use crate::sidecar::CancellationToken;
pub use kerf_core::raster::{plan, BakeJob, Planned};
use kerf_fx::RasterStack;
use kerf_model::{Catalog, Project};
use rayon::prelude::*;
use serde_json::json;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

const BAKE_FILE: &str = "frames.mkv";

/// Plan the bakes `project` needs, placed in `cache`. Its media must be
/// readable: each bake is keyed by its source as the source is now.
pub fn plan_bakes(
    project: &Project,
    cache: &DerivativeCache,
) -> Result<(Planned, Vec<CacheEntry>)> {
    let mut entries = Vec::new();
    let planned = plan(project, Catalog::builtin(), |job| {
        let entry = cache.source_entry(
            Path::new(&job.source),
            "raster",
            &json!({"job": job.identity(), "effectcraft": kerf_fx::EFFECTCRAFT_REV}),
        )?;
        let path = entry.path(BAKE_FILE).to_string_lossy().into_owned();
        entries.push(entry);
        Ok::<_, EngineError>(path)
    })?;
    Ok((planned, entries))
}

/// `project` as a render plays it: every raster effect on a media clip
/// baked into `cache`, making the bakes that are not there yet.
pub fn bake(
    project: &Project,
    cache: &DerivativeCache,
    cancellation: &CancellationToken,
) -> Result<Project> {
    let (planned, entries) = plan_bakes(project, cache)?;
    for (job, entry) in planned.jobs.iter().zip(&entries) {
        if !entry.is_complete(&[BAKE_FILE]) {
            bake_job(job, entry, cancellation)?;
        }
    }
    Ok(planned.project)
}

fn bake_job(job: &BakeJob, entry: &CacheEntry, cancellation: &CancellationToken) -> Result<()> {
    let stack = RasterStack::new(&job.effects, job.width, job.height, job.seed)
        .map_err(EngineError::msg)?;
    let probe = crate::probe::info(&job.source)?;
    let (Some(width), Some(height)) = (probe.width, probe.height) else {
        return Err(EngineError::msg(format!("{} has no picture", job.source)));
    };
    let frame = job.rate.frame_duration();
    let count = usize::try_from((job.to - job.from).flicks() / frame.flicks())
        .unwrap_or(0)
        .max(1);
    let rate = format!("{}/{}", job.rate.num(), job.rate.den());
    let size = width as usize * height as usize * 4;
    let partial = entry.path("frames.partial.mkv");

    let ffmpeg = crate::ffmpeg::resolve_ffmpeg_bin();
    let mut decoder = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostats", "-v", "error", "-ss"])
        .arg(crate::program::secs(job.from))
        .arg("-i")
        .arg(&job.source)
        .args(["-map", "0:v:0", "-an", "-sn", "-vf"])
        .arg(format!("scale={width}:{height},format=rgba"))
        .args(["-fps_mode", "cfr", "-r", &rate, "-frames:v"])
        .arg(count.to_string())
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut encoder = match Command::new(ffmpeg)
        .args(["-hide_banner", "-nostats", "-v", "error", "-y"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s"])
        .arg(format!("{width}x{height}"))
        .args(["-r", &rate, "-i", "-"])
        .args([
            "-c:v", "ffv1", "-level", "3", "-pix_fmt", "bgra", "-f", "matroska",
        ])
        .arg(&partial)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(encoder) => encoder,
        Err(error) => {
            let _ = decoder.kill();
            return Err(error.into());
        }
    };

    let fps = job.rate.fps();
    let result = (|| -> Result<()> {
        let mut input = decoder.stdout.take().expect("piped");
        let mut output = encoder.stdin.take().expect("piped");
        let batch = rayon::current_num_threads() * 2;
        let mut last: Option<Vec<u8>> = None;
        let mut next = 0;
        while next < count {
            if cancellation.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            let mut frames = Vec::with_capacity(batch);
            while frames.len() < batch && next + frames.len() < count {
                let mut frame = vec![0; size];
                if read_frame(&mut input, &mut frame)? {
                    frames.push(frame);
                } else {
                    // A source that ends early holds its last frame.
                    match &last {
                        Some(held) => frames.push(held.clone()),
                        None => {
                            return Err(EngineError::msg(format!(
                                "{} gave no frames to bake",
                                job.source
                            )))
                        }
                    }
                }
            }
            let first = next;
            frames
                .par_iter_mut()
                .enumerate()
                .for_each(|(index, frame)| {
                    let at = job.from + job.rate.time_of((first + index) as i64);
                    stack.apply(frame, width, height, job.clip_time(at), fps);
                });
            for frame in &frames {
                output.write_all(frame)?;
            }
            next += frames.len();
            last = frames.pop();
        }
        Ok(())
    })();
    let decoded = finish(&mut decoder, result.is_err());
    let encoded = finish(&mut encoder, false);
    let outcome = result
        .and_then(|()| {
            decoded.map_err(|error| EngineError::msg(format!("decoding {}: {error}", job.source)))
        })
        .and_then(|()| {
            encoded.map_err(|error| EngineError::msg(format!("baking {}: {error}", job.source)))
        });
    if let Err(error) = outcome {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    std::fs::rename(&partial, entry.path(BAKE_FILE))?;
    entry.mark_complete(&[BAKE_FILE])?;
    Ok(())
}

/// Fill `frame` from `input`; false at the end of the stream.
fn read_frame(input: &mut impl Read, frame: &mut [u8]) -> Result<bool> {
    let mut filled = 0;
    while filled < frame.len() {
        match input.read(&mut frame[filled..])? {
            0 if filled == 0 => return Ok(false),
            0 => return Err(EngineError::msg("a frame ended early")),
            read => filled += read,
        }
    }
    Ok(true)
}

/// Wait for an FFmpeg process (killing it first when `kill`), reporting
/// its errors.
fn finish(child: &mut Child, kill: bool) -> std::result::Result<(), String> {
    if kill {
        let _ = child.kill();
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    match child.wait() {
        Ok(status) if status.success() || kill => Ok(()),
        Ok(status) => Err(format!("{status}: {}", stderr.trim())),
        Err(error) => Err(error.to_string()),
    }
}
