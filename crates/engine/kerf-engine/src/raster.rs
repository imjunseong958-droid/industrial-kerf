//! Baking raster effects: a clip's frames through its raster effects into a
//! lossless file that the graph plays in the clip's place. Which clips need
//! it, and how the project changes, is `kerf_core::raster`; this makes the
//! files, cached by what they are made from: a prerender (a title, solid,
//! or nested sequence rendered on its own, with its transparency) by the
//! sequences it renders and their media, and a bake by its source and job.

use crate::derivatives::{CacheEntry, DerivativeCache, SourceIdentity};
use crate::error::{EngineError, Result};
use crate::program::{compile_program, secs, ProgramOptions};
use crate::sidecar::CancellationToken;
pub use kerf_core::raster::{plan, BakeJob, Job, Planned, Prerender};
use kerf_fx::RasterStack;
use kerf_model::{Catalog, Project};
use rayon::prelude::*;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Stdio};
use std::thread;

const BAKE_FILE: &str = "frames.mkv";

/// What rendering `sequence` of `project` needs, placed in `cache`, with
/// the cache entry of each prerender and each bake. Its media must be
/// readable: files are keyed by their sources as the sources are now.
pub fn plan_bakes(
    project: &Project,
    sequence: Option<&str>,
    cache: &DerivativeCache,
) -> Result<(Planned, Vec<CacheEntry>, Vec<CacheEntry>)> {
    let options = json!({"effects": kerf_fx::EFFECTS_VERSION});
    let (mut prerenders, mut bakes) = (Vec::new(), Vec::new());
    let planned = plan(project, Catalog::builtin(), sequence, |job| {
        let (entry, entries) = match job {
            Job::Prerender(prerender) => {
                let sources = prerender
                    .media()
                    .iter()
                    .map(|media| SourceIdentity::from_path(Path::new(&media.path)))
                    .collect::<Result<Vec<_>>>()?;
                let entry = cache.project_entry(
                    &json!(prerender.identity()),
                    &sources,
                    "prerender",
                    &options,
                )?;
                (entry, &mut prerenders)
            }
            Job::Bake(job) if job.prerender.is_some() => {
                // A prerender's file is named for everything it is made from.
                let entry = cache.project_entry(&json!(job.identity()), &[], "raster", &options)?;
                (entry, &mut bakes)
            }
            Job::Bake(job) => {
                let entry = cache.source_entry(
                    Path::new(&job.source),
                    "raster",
                    &json!({"job": job.identity(), "effects": kerf_fx::EFFECTS_VERSION}),
                )?;
                (entry, &mut bakes)
            }
        };
        let path = entry.path(BAKE_FILE).to_string_lossy().into_owned();
        entries.push(entry);
        Ok::<_, EngineError>(path)
    })?;
    Ok((planned, prerenders, bakes))
}

/// `project` as rendering `sequence` of it plays it: every raster effect
/// baked into `cache`, making the files that are not there yet.
pub fn bake(
    project: &Project,
    sequence: Option<&str>,
    cache: &DerivativeCache,
    cancellation: &CancellationToken,
) -> Result<Project> {
    let (planned, prerenders, bakes) = plan_bakes(project, sequence, cache)?;
    for (prerender, entry) in planned.prerenders.iter().zip(&prerenders) {
        if !entry.is_complete(&[BAKE_FILE]) {
            prerender_job(prerender, entry, cache, cancellation)?;
        }
    }
    for (job, entry) in planned.jobs.iter().zip(&bakes) {
        if !entry.is_complete(&[BAKE_FILE]) {
            bake_job(job, entry, cancellation)?;
        }
    }
    Ok(planned.project)
}

/// Render a prerender's sequence, its own raster effects baked first, to a
/// lossless file that keeps its transparency.
fn prerender_job(
    prerender: &Prerender,
    entry: &CacheEntry,
    cache: &DerivativeCache,
    cancellation: &CancellationToken,
) -> Result<()> {
    let sequence = prerender.sequence.as_str();
    let project = bake(&prerender.project, Some(sequence), cache, cancellation)?;
    let compiled = compile_program(
        &project,
        Catalog::builtin(),
        &ProgramOptions {
            sequence: Some(sequence.to_owned()),
            range: Some((prerender.from, prerender.to)),
            transparent: true,
            ..ProgramOptions::default()
        },
    )?;
    let partial = entry.path("frames.partial.mkv");
    let mut args = crate::ffmpeg::program_decode_prefix(&compiled.argv);
    args.extend(compiled.argv.iter().skip(1).cloned());
    args.extend(
        [
            "-c:v",
            "ffv1",
            "-level",
            "3",
            "-pix_fmt",
            "bgra",
            "-c:a",
            "pcm_s16le",
            "-r",
        ]
        .map(str::to_owned),
    );
    args.extend([
        compiled.rate_arg(),
        "-t".to_owned(),
        secs(compiled.duration),
        "-f".to_owned(),
        "matroska".to_owned(),
        partial.to_string_lossy().into_owned(),
    ]);
    if let Err(error) = crate::render::run_ffmpeg_with_progress(
        &args,
        compiled.duration.seconds(),
        cancellation,
        None::<fn(f64)>,
    ) {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    std::fs::rename(&partial, entry.path(BAKE_FILE))?;
    entry.mark_complete(&[BAKE_FILE])?;
    Ok(())
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
    let mut decoder = crate::ffmpeg::hidden_command(ffmpeg)
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
    let mut encoder = match crate::ffmpeg::hidden_command(ffmpeg)
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
            let _ = decoder.wait();
            return Err(error.into());
        }
    };
    // Read as they write, so neither blocks on a full stderr pipe while
    // frames pass between them.
    let decoder_errors = drain_stderr(&mut decoder);
    let encoder_errors = drain_stderr(&mut encoder);

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
    let decoded = finish(&mut decoder, result.is_err(), decoder_errors);
    let encoded = finish(&mut encoder, false, encoder_errors);
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
pub(crate) fn read_frame(input: &mut impl Read, frame: &mut [u8]) -> Result<bool> {
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

/// Read `child`'s stderr to its end on a thread of its own, keeping the
/// last of it for [`finish`] to report.
pub(crate) fn drain_stderr(child: &mut Child) -> thread::JoinHandle<String> {
    let pipe = child.stderr.take();
    thread::spawn(move || {
        let mut tail = String::new();
        let Some(pipe) = pipe else {
            return tail;
        };
        for line in BufReader::new(pipe).split(b'\n') {
            let Ok(line) = line else {
                break;
            };
            crate::sidecar::append_bounded(&mut tail, &String::from_utf8_lossy(&line));
        }
        tail
    })
}

/// Wait for an FFmpeg process (killing it first when `kill`), reporting
/// its errors from `stderr`, what [`drain_stderr`] read.
fn finish(
    child: &mut Child,
    kill: bool,
    stderr: thread::JoinHandle<String>,
) -> std::result::Result<(), String> {
    if kill {
        let _ = child.kill();
    }
    let status = child.wait();
    let stderr = stderr.join().unwrap_or_default();
    match status {
        Ok(status) if status.success() || kill => Ok(()),
        Ok(status) => Err(format!("{status}: {}", stderr.trim())),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_child_that_fills_its_stderr_still_finishes() {
        // Far more than a pipe holds, before the stdout it is read for.
        let mut child = std::process::Command::new("sh")
            .args(["-c", "i=0; while [ $i -lt 4000 ]; do echo \"error while decoding MB $i\" >&2; i=$((i+1)); done; echo done"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let errors = drain_stderr(&mut child);
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert_eq!(output, "done\n");
        assert_eq!(finish(&mut child, false, errors), Ok(()));
    }
}
