//! Render argument construction and ffmpeg spawn for `kerf-model` projects.
//!
//! A render draws the picture with the compositor (`kerf-compose`) and
//! pipes its frames to FFmpeg, which encodes them with the program's sound
//! as the graph mixes it. Effects draw as part of each frame, so nothing is
//! baked first. The graph drawing the picture too, as renders did before
//! the compositor, is kept for one release: [`RenderOptions::graph`], or
//! `KERF_GRAPH_RENDER=1` in the environment.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use kerf_compose::{ComposeOptions, Compositor, Device, Frame};
use kerf_model::{Rate, Time};

use crate::encoder::{
    encode_output_args, parse_encoder_capabilities, select_video_encoder, EncoderSelection,
    SelectEncoderOptions, VideoCodec,
};
use crate::error::{EngineError, Result};
use crate::ffmpeg::{cached_encoder_listing, parse_time_seconds, resolve_ffmpeg_bin, tail};
use crate::program::{compile_program, ProgramOptions};
use crate::resources::{budget, should_try_hardware_encode};
use crate::sidecar::{self, CancellationToken};

const CANCEL_FILE_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub fn detect_and_select_encoder(options: &SelectEncoderOptions) -> Result<EncoderSelection> {
    let ffmpeg = resolve_ffmpeg_bin();
    let listing = cached_encoder_listing()?;
    let capabilities = parse_encoder_capabilities(listing, ffmpeg);
    Ok(select_video_encoder(&capabilities.encoders, options))
}

fn draft_post_filters() -> Vec<String> {
    vec!["scale=-2:360".into(), "fps=15".into()]
}

fn draft_codec_args() -> Vec<String> {
    vec![
        "-crf".into(),
        "35".into(),
        "-preset".into(),
        "ultrafast".into(),
    ]
}

fn production_codec_args() -> Vec<String> {
    let budget = budget();
    vec![
        "-c:v".into(),
        "libx264".into(),
        "-crf".into(),
        budget.software_crf.to_string(),
        "-preset".into(),
        budget.software_preset.to_string(),
    ]
}

fn hardware_or_software_codec(
    prefer_hardware: bool,
    force_software: bool,
    codec: VideoCodec,
) -> Result<Vec<String>> {
    if !should_try_hardware_encode(prefer_hardware, force_software) {
        return Ok(production_codec_args());
    }
    let select = SelectEncoderOptions {
        prefer_hardware: true,
        codec,
        ..Default::default()
    };
    let selection = detect_and_select_encoder(&select)?;
    let budget = budget();
    Ok(encode_output_args(
        &selection,
        budget.software_crf,
        budget.software_preset,
    ))
}

/// How to encode a rendered program.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub output: String,
    /// 360p at 15 fps with fast, low-quality encoding.
    pub draft: bool,
    pub prefer_hardware: bool,
    pub force_software: bool,
    pub codec: VideoCodec,
    /// The project directory, whose cache holds baked raster effects
    /// when the graph draws the picture. Without one, the graph cannot
    /// render a project with raster effects.
    pub project_dir: Option<std::path::PathBuf>,
    /// Draw the picture with the FFmpeg graph, as renders did before the
    /// compositor. Kept for one release.
    pub graph: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            output: String::new(),
            draft: false,
            prefer_hardware: false,
            force_software: false,
            codec: VideoCodec::H264,
            project_dir: None,
            graph: false,
        }
    }
}

/// Whether a render draws the picture with the graph.
fn graph_render(opts: &RenderOptions) -> bool {
    opts.graph || std::env::var_os("KERF_GRAPH_RENDER").is_some_and(|value| value == "1")
}

/// Whether `sequence` of `project` (the first when absent) renders as
/// [`render_program`] would render it: the error says what it cannot draw
/// or mix. Reads no media.
pub fn renderable(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    opts: &RenderOptions,
) -> Result<()> {
    let catalog = kerf_model::Catalog::builtin();
    let options = ProgramOptions {
        sequence: sequence.map(str::to_owned),
        audio_only: !graph_render(opts),
        ..ProgramOptions::default()
    };
    if graph_render(opts) {
        // The graph bakes raster effects first; compile what it would.
        let planned = crate::raster::plan(project, catalog, sequence, |_| {
            Ok::<_, std::convert::Infallible>("baked.mkv".to_owned())
        })
        .unwrap_or_else(|never| match never {});
        compile_program(&planned.project, catalog, &options)?;
    } else {
        Compositor::new(
            project,
            catalog,
            &ComposeOptions {
                sequence: sequence.map(str::to_owned),
                captions: true,
                device: Device::Cpu,
            },
        )?;
        compile_program(project, catalog, &options)?;
    }
    Ok(())
}

/// The ffmpeg arguments that render `sequence` (the first one when `None`)
/// of `project`, or `[from, to)` of it, and the output duration in seconds.
pub fn render_program_args(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(kerf_model::Time, kerf_model::Time)>,
    opts: &RenderOptions,
) -> Result<(Vec<String>, f64)> {
    let compiled = compile_program(
        project,
        kerf_model::Catalog::builtin(),
        &ProgramOptions {
            sequence: sequence.map(str::to_owned),
            range,
            video_post_filters: if opts.draft {
                draft_post_filters()
            } else {
                Vec::new()
            },
            ..ProgramOptions::default()
        },
    )?;
    let mut args = crate::ffmpeg::program_decode_prefix(&compiled.argv);
    args.extend(compiled.argv.iter().skip(1).cloned());
    args.extend(if opts.draft {
        draft_codec_args()
    } else {
        hardware_or_software_codec(opts.prefer_hardware, opts.force_software, opts.codec)?
    });
    args.extend([
        "-r".into(),
        compiled.rate_arg(),
        "-t".into(),
        crate::program::secs(compiled.duration),
        opts.output.clone(),
    ]);
    Ok((args, compiled.duration.seconds()))
}

/// Render a project with a caller-owned cancellation token.
pub fn render_program_with_cancellation<F>(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(kerf_model::Time, kerf_model::Time)>,
    opts: &RenderOptions,
    cancellation: &CancellationToken,
    on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    if !graph_render(opts) {
        return render_composited(project, sequence, range, opts, cancellation, on_progress);
    }
    let baked;
    let project = match &opts.project_dir {
        Some(directory) => {
            let cache = crate::derivatives::DerivativeCache::new(directory);
            baked = crate::raster::bake(project, sequence, &cache, cancellation)?;
            &baked
        }
        None => project,
    };
    let (args, duration) = render_program_args(project, sequence, range, opts)?;
    run_ffmpeg_with_progress(&args, duration, cancellation, on_progress)
}

pub fn render_program<F>(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(kerf_model::Time, kerf_model::Time)>,
    opts: &RenderOptions,
    on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let cancellation = CancellationToken::new();
    render_program_with_cancellation(project, sequence, range, opts, &cancellation, on_progress)
}

/// Frames drawn at once for a render.
const RENDER_DRAWERS: usize = 3;

/// What a composited render draws and how FFmpeg encodes it.
#[derive(Debug)]
struct CompositedRender {
    /// FFmpeg's arguments, reading the frames from stdin.
    args: Vec<String>,
    /// The sequence time of each frame, in order.
    times: Vec<Time>,
    /// The share of the canvas drawn, and the frame size that gives.
    scale: f64,
    size: (u32, u32),
}

/// The frames of `[from, to)` of a sequence (all of it when no range) and
/// the FFmpeg command encoding them with the sound. A draft is drawn 360
/// pixels tall at 15 fps.
fn composited_render(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(Time, Time)>,
    opts: &RenderOptions,
    canvas: (u32, u32),
) -> Result<CompositedRender> {
    // The graph mixes the sound; the picture comes from stdin.
    let sound = compile_program(
        project,
        kerf_model::Catalog::builtin(),
        &ProgramOptions {
            sequence: sequence.map(str::to_owned),
            range,
            audio_only: true,
            ..ProgramOptions::default()
        },
    )?;
    let from = range.map_or(Time::ZERO, |(from, _)| from);
    let (rate, scale) = if opts.draft {
        let tall = f64::from(canvas.1.max(1));
        (
            Rate::new(15, 1).expect("15 fps is a rate"),
            (360.0 / tall).min(1.0),
        )
    } else {
        (sound.rate, 1.0)
    };
    let frame = rate.frame_duration().flicks();
    let ceil = |time: Time| -(-time.flicks()).div_euclid(frame);
    let count = ceil(sound.duration).max(1);
    let first = ceil(from);
    let times = (first..first + count)
        .map(|index| rate.time_of(index))
        .collect();
    let size = (
        ((f64::from(canvas.0) * scale).round() as u32).max(1),
        ((f64::from(canvas.1) * scale).round() as u32).max(1),
    );
    let mut args: Vec<String> = sound.argv.iter().skip(1).cloned().collect();
    let picture = args.iter().filter(|arg| *arg == "-i").count();
    let at = args
        .iter()
        .position(|arg| arg == "-filter_complex")
        .unwrap_or(args.len());
    args.splice(
        at..at,
        [
            "-f".to_owned(),
            "rawvideo".to_owned(),
            "-pix_fmt".to_owned(),
            "rgba".to_owned(),
            "-s".to_owned(),
            format!("{}x{}", size.0, size.1),
            "-framerate".to_owned(),
            crate::program::rate_arg(rate),
            "-i".to_owned(),
            "pipe:0".to_owned(),
        ],
    );
    args.extend(["-map".to_owned(), format!("{picture}:v")]);
    args.extend(if opts.draft {
        draft_codec_args()
    } else {
        hardware_or_software_codec(opts.prefer_hardware, opts.force_software, opts.codec)?
    });
    if size.0 % 2 == 1 || size.1 % 2 == 1 {
        // 4:2:0 needs even sides.
        args.extend([
            "-vf".to_owned(),
            "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_owned(),
        ]);
    }
    args.extend([
        "-pix_fmt".to_owned(),
        "yuv420p".to_owned(),
        "-r".to_owned(),
        crate::program::rate_arg(rate),
        "-t".to_owned(),
        crate::program::secs(sound.duration),
        opts.output.clone(),
    ]);
    Ok(CompositedRender {
        args,
        times,
        scale,
        size,
    })
}

/// Render with the compositor drawing the picture.
fn render_composited<F>(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(Time, Time)>,
    opts: &RenderOptions,
    cancellation: &CancellationToken,
    mut on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    if cancellation.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    let compositor = Compositor::new(
        project,
        kerf_model::Catalog::builtin(),
        &ComposeOptions {
            sequence: sequence.map(str::to_owned),
            captions: true,
            device: Device::Auto,
        },
    )?;
    let render = composited_render(project, sequence, range, opts, compositor.size())?;
    let mut child = Command::new(resolve_ffmpeg_bin())
        .args(&render.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| EngineError::msg(format!("ffmpeg could not start: {error}")))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let total = render.times.len();
    let mut written = 0;
    let drawn = draw_in_order(
        total,
        RENDER_DRAWERS,
        |index| compositor.frame(render.times[index], render.scale),
        |frame| {
            if (frame.width, frame.height) != render.size {
                return Err(EngineError::msg(format!(
                    "a frame came out {}x{}, not {}x{}",
                    frame.width, frame.height, render.size.0, render.size.1
                )));
            }
            stdin.write_all(&frame.rgba)?;
            written += 1;
            if let Some(callback) = on_progress.as_mut() {
                callback(written as f64 / total as f64);
            }
            Ok(())
        },
        cancellation,
    );
    drop(stdin);
    if drawn.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let stderr = reader.join().unwrap_or_default();
    match drawn {
        Err(EngineError::Cancelled) => Err(EngineError::Cancelled),
        // FFmpeg stopping reading is FFmpeg's error to tell.
        Err(EngineError::Io(_)) | Ok(()) if !status.success() => Err(EngineError::Ffmpeg {
            code: status.code().unwrap_or(-1),
            stderr: tail(stderr.trim(), 2000).to_owned(),
        }),
        Err(error) => Err(error),
        Ok(()) => Ok(()),
    }
}

/// Draw `count` frames on `threads` threads, handing them to `write` in
/// order. Drawing stays at most a few frames ahead of writing.
fn draw_in_order(
    count: usize,
    threads: usize,
    draw: impl Fn(usize) -> Frame + Sync,
    mut write: impl FnMut(Frame) -> Result<()>,
    cancellation: &CancellationToken,
) -> Result<()> {
    let ahead = threads * 2;
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let written = (Mutex::new(0_usize), Condvar::new());
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel::<(usize, Frame)>();
        for _ in 0..threads.max(1) {
            let sender = sender.clone();
            let (next, stop, written, draw) = (&next, &stop, &written, &draw);
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::SeqCst);
                if index >= count {
                    return;
                }
                {
                    let (lock, ready) = written;
                    let mut done = lock
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    while index >= *done + ahead && !stop.load(Ordering::SeqCst) {
                        done = ready
                            .wait(done)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                }
                if stop.load(Ordering::SeqCst) || sender.send((index, draw(index))).is_err() {
                    return;
                }
            });
        }
        drop(sender);
        let mut pending = BTreeMap::new();
        let mut done = 0;
        let result = (|| {
            while done < count {
                if cancellation.is_cancelled() {
                    return Err(EngineError::Cancelled);
                }
                match receiver.recv_timeout(CANCEL_FILE_POLL_INTERVAL) {
                    Ok((index, frame)) => {
                        pending.insert(index, frame);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(EngineError::msg("the drawing threads stopped"));
                    }
                }
                while let Some(frame) = pending.remove(&done) {
                    write(frame)?;
                    done += 1;
                    let (lock, ready) = &written;
                    *lock
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = done;
                    ready.notify_all();
                }
            }
            Ok(())
        })();
        stop.store(true, Ordering::SeqCst);
        written.1.notify_all();
        result
    })
}

fn extract_time(line: &str) -> Option<&str> {
    let index = line.find("time=")?;
    let rest = &line[index + 5..];
    let end = rest.find(' ').unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Run ffmpeg to completion, reporting progress through `duration` seconds
/// of output when it is positive.
pub(crate) fn run_ffmpeg_with_progress<F>(
    args: &[String],
    duration: f64,
    cancellation: &CancellationToken,
    mut on_progress: Option<F>,
) -> Result<()>
where
    F: FnMut(f64),
{
    let mut command = Command::new(resolve_ffmpeg_bin());
    command.args(args);
    let finished = sidecar::run(command, cancellation, |line| {
        let Some(seconds) = extract_time(line).and_then(parse_time_seconds) else {
            return;
        };
        if let (Some(callback), true) = (on_progress.as_mut(), duration > 0.0) {
            callback((seconds / duration).clamp(0.0, 1.0));
        }
    })?;
    if finished.status.success() {
        return Ok(());
    }
    let detail = finished.stderr.trim();
    Err(EngineError::Ffmpeg {
        code: finished.status.code().unwrap_or(-1),
        stderr: if detail.is_empty() {
            "ffmpeg failed".into()
        } else {
            tail(detail, 2000).to_owned()
        },
    })
}

/// Watch a project-local cancellation marker for one render invocation.
pub fn spawn_cancel_file_watcher(
    path: PathBuf,
    cancellation: CancellationToken,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !cancellation.is_cancelled() {
            if path.is_file() {
                cancellation.cancel();
                break;
            }
            thread::sleep(CANCEL_FILE_POLL_INTERVAL);
        }
    })
}

/// Progress callback that writes `{"progress":n}` JSON lines to stderr.
pub fn stderr_progress_callback() -> impl FnMut(f64) {
    let mut last = -1.0_f64;
    move |ratio: f64| {
        let rounded = (ratio * 10_000.0).round() / 10_000.0;
        if (rounded - last).abs() < 0.0001 {
            return;
        }
        last = rounded;
        let _ = writeln!(std::io::stderr(), r#"{{"progress":{rounded}}}"#);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Ten seconds of one source, cut at 24 fps.
    fn project() -> kerf_model::Project {
        kerf_lang::parse_document(
            "kerf 1\nproject name=\"render\"\n\
             media s path=\"/tmp/s.mp4\" rate=24 dur=30s video=160x120\n\
             sequence s1 rate=24 size=160x120 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip v media=s src=10s at=0f dur=10s\n",
            kerf_model::Catalog::builtin(),
        )
        .expect("project")
        .project
    }

    #[test]
    fn production_software_uses_budgeted_x264() {
        let (args, _) = render_program_args(
            &project(),
            None,
            None,
            &RenderOptions {
                output: "out.mp4".into(),
                force_software: true,
                ..RenderOptions::default()
            },
        )
        .expect("args");
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-c:v" && pair[1] == "libx264"));
        assert!(args
            .iter()
            .any(|arg| arg == "veryfast" || arg == "faster" || arg == "medium"));
        assert!(args.windows(2).any(|pair| pair[0] == "-threads"));
    }

    #[test]
    fn draft_renders_scale_down_with_fast_encoding() {
        let (args, _) = render_program_args(
            &project(),
            None,
            None,
            &RenderOptions {
                output: "out.mp4".into(),
                draft: true,
                ..RenderOptions::default()
            },
        )
        .expect("args");
        let graph = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| args[index + 1].as_str())
            .expect("filter_complex");
        assert!(graph.contains("scale=-2:360") && graph.contains("fps=15"));
        assert!(args.iter().any(|arg| arg == "ultrafast"));
        assert_eq!(args.last().map(String::as_str), Some("out.mp4"));
    }

    #[test]
    fn range_render_args_seek_before_input_not_after_the_graph() {
        let rate = kerf_model::Rate::FPS_24;
        let (args, duration) = render_program_args(
            &project(),
            None,
            Some((rate.time_of(48), rate.time_of(120))),
            &RenderOptions {
                output: "out.mp4".into(),
                draft: true,
                ..RenderOptions::default()
            },
        )
        .expect("args");
        assert_eq!(duration, 3.0);
        let graph = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .expect("filter");
        assert!(args[..graph].windows(2).any(|pair| pair[0] == "-ss"));
        assert!(!args[graph..].windows(2).any(|pair| pair[0] == "-ss"));
    }

    #[test]
    fn composited_renders_pipe_frames_in_beside_the_sound() {
        let project = kerf_lang::parse_document(
            "kerf 1\nproject name=\"render\"\n\
             media s path=\"/tmp/s.mp4\" rate=24 dur=30s video=160x120 audio=2ch@48000\n\
             sequence s1 rate=24 size=160x120 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip v media=s src=10s at=0f dur=10s\n\
             \x20 track A1 audio\n\
             \x20   clip a media=s src=10s at=0f dur=10s\n",
            kerf_model::Catalog::builtin(),
        )
        .expect("project")
        .project;
        let render = composited_render(
            &project,
            None,
            None,
            &RenderOptions {
                output: "out.mp4".into(),
                force_software: true,
                ..RenderOptions::default()
            },
            (160, 120),
        )
        .expect("render");
        assert_eq!(render.times.len(), 240);
        assert_eq!((render.size, render.scale), ((160, 120), 1.0));
        let args = &render.args;
        let pipe = args.iter().position(|arg| arg == "pipe:0").expect("pipe");
        let graph = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .expect("filter_complex");
        assert!(pipe < graph, "inputs come before the graph: {args:?}");
        assert_eq!(
            args[pipe - 6..pipe - 1],
            ["rgba", "-s", "160x120", "-framerate", "24/1"]
        );
        // The sound reads the clip's file; the picture is input 1.
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-map" && pair[1] == "1:v"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-pix_fmt" && pair[1] == "yuv420p"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-c:v" && pair[1] == "libx264"));
        assert_eq!(args.last().map(String::as_str), Some("out.mp4"));
    }

    #[test]
    fn composited_drafts_and_ranges_draw_their_frames() {
        let rate = kerf_model::Rate::FPS_24;
        let draft = composited_render(
            &project(),
            None,
            Some((rate.time_of(48), rate.time_of(120))),
            &RenderOptions {
                output: "out.mp4".into(),
                draft: true,
                ..RenderOptions::default()
            },
            (1920, 1080),
        )
        .expect("render");
        // Three seconds at 15 fps, 360 pixels tall, from two seconds in.
        assert_eq!(draft.times.len(), 45);
        assert_eq!(
            draft.times[0],
            kerf_model::Time::from_flicks(2 * 705_600_000)
        );
        assert_eq!(draft.size, (640, 360));
        assert!(draft.args.iter().any(|arg| arg == "ultrafast"));
        assert!(draft
            .args
            .windows(2)
            .any(|pair| pair[0] == "-t" && pair[1] == "3"));
    }

    #[test]
    fn cancellation_marker_only_cancels_its_own_render_token() {
        let directory = tempfile::tempdir().expect("temp directory");
        let first_path = directory.path().join("first.cancel");
        let second_path = directory.path().join("second.cancel");
        let first = CancellationToken::new();
        let second = CancellationToken::new();
        let first_watcher = spawn_cancel_file_watcher(first_path.clone(), first.clone());
        let second_watcher = spawn_cancel_file_watcher(second_path, second.clone());
        fs::write(&first_path, b"cancelled\n").expect("cancel marker");
        for _ in 0..100 {
            if first.is_cancelled() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        second.cancel();
        first_watcher.join().expect("first watcher");
        second_watcher.join().expect("second watcher");
    }
}
