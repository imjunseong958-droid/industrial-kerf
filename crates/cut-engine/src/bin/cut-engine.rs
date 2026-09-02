use std::collections::HashMap;
use std::io::{self, Read};
use std::path::PathBuf;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use cut_engine::compile::{compile_filter_graph_v2, timeline_duration_v2, CompileV2Options};
use cut_engine::compile_v1::{atempo_filters, compile_filter_graph_v1, timeline_duration_v1};
use cut_engine::derivatives::DerivativeCache;
use cut_engine::encoder::{
    encoder_args, parse_encoder_capabilities, select_video_encoder, SelectEncoderOptions,
    VideoCodec,
};
use cut_engine::enrich::{enrich_project_v2, enrich_timeline_v1};
use cut_engine::ffmpeg::{resolve_ffmpeg_bin, run_allow_failure};
use cut_engine::overlay::{
    captions_filter, escape_drawtext_text, escape_filter_path, subtitles_filter, CaptionCue,
};
use cut_engine::probe::{
    cached_filmstrip, cached_waveform, detect_scenes, extract_exact_frames, extract_frames, info,
    parse_scene_times, parse_silencedetect, silence_detect,
};
use cut_engine::proxy::{
    create_proxy_cache_key, proxy_ffmpeg_args, proxy_run, proxy_settings_from_value, ProxySettings,
    ProxySourceStat,
};
use cut_engine::render::{
    render_args_v1, render_args_v2, stderr_progress_callback, RenderV1Options, RenderV2Options,
};
use cut_engine::resources::{budget, tracking_sample_fps};
use cut_engine::vision::{
    build_tracking_plan, crop_plan_from_json, normalize_detection_frames,
    parse_and_normalize_tracking_output, validate_yolo26_config, FfmpegRgbFrameSource,
    NormalizeOptions, TrackDecodeOptions, TrackingPlanInput, TrackingPlanOptions,
    UnknownLabelPolicy, VisionTarget, Yolo26TrackingConfig, YoloEngine, DEFAULT_IMGSZ,
};
use cut_engine::{
    CancellationToken, TimeRange, TranscriptionBackend, TranscriptionOptions,
    TranscriptionProgress, TranscriptionStage,
};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(name = "cut-engine", about = "Cut native media engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Probe {
        #[command(subcommand)]
        action: ProbeAction,
    },
    Encoders {
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        detect: bool,
    },
    Resources,
    SelectEncoder {
        #[arg(long)]
        prefer_hardware: bool,
        #[arg(long, default_value = "h264")]
        codec: CliCodec,
        #[arg(long)]
        software: Option<String>,
    },
    CompileV1 {
        #[arg(long)]
        draft: bool,
        #[arg(long)]
        post_filter: Vec<String>,
    },
    CompileV2 {
        #[arg(long)]
        include_embedded_audio: Option<bool>,
        #[arg(long)]
        draft: bool,
        #[arg(long)]
        post_filter: Vec<String>,
    },
    DurationV1,
    DurationV2,
    RenderV1 {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        draft: bool,
        #[arg(long)]
        run: bool,
        #[arg(long)]
        cancel_file: Option<PathBuf>,
    },
    RenderV2 {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        draft: bool,
        #[arg(long)]
        prefer_hardware: bool,
        #[arg(long)]
        software: bool,
        #[arg(long)]
        run: bool,
        #[arg(long)]
        include_embedded_audio: Option<bool>,
        #[arg(long)]
        start: Option<f64>,
        #[arg(long)]
        duration: Option<f64>,
        #[arg(long)]
        cancel_file: Option<PathBuf>,
    },
    ProxyKey {
        #[arg(long)]
        source: String,
        #[arg(long)]
        size: u64,
        #[arg(long)]
        mtime_ms: f64,
        #[arg(long)]
        id: Option<String>,
    },
    ProxyArgs {
        #[arg(long)]
        input: String,
        #[arg(long)]
        output: String,
        #[arg(long)]
        width: Option<f64>,
        #[arg(long)]
        height: Option<f64>,
        #[arg(long)]
        video_codec: Option<String>,
    },
    ProxyRun {
        #[arg(long)]
        input: String,
        #[arg(long)]
        output: String,
    },
    Normalize {
        #[arg(long)]
        label_map: String,
        #[arg(long, default_value = "ignore")]
        unknown: String,
        #[arg(long)]
        fps: Option<f64>,
        #[arg(long)]
        frame_width: Option<f64>,
        #[arg(long)]
        frame_height: Option<f64>,
    },
    Plan {
        #[arg(long)]
        min_confidence: Option<f64>,
        #[arg(long)]
        max_gap: Option<f64>,
        #[arg(long)]
        smoothing: Option<f64>,
        #[arg(long)]
        crop_aspect: Option<f64>,
    },
    Overlay {
        #[command(subcommand)]
        action: OverlayAction,
    },
    Crop,
    Track {
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        weights: Option<String>,
        #[arg(long)]
        label_map: Option<String>,
        #[arg(long)]
        conf: Option<f64>,
        #[arg(long)]
        iou: Option<f64>,
        #[arg(long)]
        fps: Option<f64>,
        #[arg(long)]
        imgsz: Option<u32>,
        #[arg(long)]
        max_frames: Option<usize>,
    },
}

#[derive(Subcommand)]
enum ProbeAction {
    Info {
        src: String,
    },
    Silence {
        src: String,
        #[arg(long, default_value_t = -35.0, allow_hyphen_values = true)]
        threshold_db: f64,
        #[arg(long, default_value_t = 0.5)]
        min_duration: f64,
        #[arg(long)]
        parse: Option<String>,
    },
    Scenes {
        src: String,
        #[arg(long, default_value_t = 0.4)]
        threshold: f64,
        #[arg(long)]
        parse: Option<String>,
    },
    Frames {
        src: String,
        #[arg(long)]
        times: String,
        #[arg(long)]
        out_dir: String,
        #[arg(long, default_value_t = 1280)]
        max_width: u32,
    },
    Frame {
        src: String,
        #[arg(long)]
        time: f64,
        #[arg(long)]
        out_dir: String,
        #[arg(long, default_value_t = 1280)]
        max_width: u32,
    },
    Filmstrip {
        src: String,
        #[arg(long)]
        times: Option<String>,
        #[arg(long, default_value_t = 8)]
        frames: usize,
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long, default_value_t = 1280)]
        max_width: u32,
        #[arg(long, default_value_t = 4)]
        columns: usize,
    },
    Waveform {
        src: String,
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long, default_value_t = 256)]
        buckets: usize,
        #[arg(long)]
        include_image: bool,
        #[arg(long, default_value_t = 1200)]
        image_width: u32,
        #[arg(long, default_value_t = 240)]
        image_height: u32,
    },
    Transcribe {
        #[command(flatten)]
        options: TranscriptionArgs,
    },
    Listen {
        #[command(flatten)]
        options: TranscriptionArgs,
    },
}

#[derive(clap::Args)]
struct TranscriptionArgs {
    src: String,
    #[arg(long)]
    start: Option<f64>,
    #[arg(long)]
    end: Option<f64>,
    #[arg(long)]
    duration: Option<f64>,
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,
    #[arg(long)]
    model: Option<PathBuf>,
    #[arg(long)]
    model_version: Option<String>,
    #[arg(long)]
    backend_version: Option<String>,
    #[arg(long)]
    language: Option<String>,
    #[arg(long, default_value = "local")]
    backend: String,
}

#[derive(Subcommand)]
enum OverlayAction {
    EscapePath {
        #[arg(long)]
        path: String,
    },
    EscapeDrawtext {
        #[arg(long)]
        text: String,
    },
    Atempo {
        #[arg(long)]
        speed: f64,
    },
    Captions,
    Subtitles,
}

#[derive(Clone, ValueEnum)]
enum CliCodec {
    H264,
    Hevc,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Probe { action } => match action {
            ProbeAction::Info { src } => print_json(&info(&src)?)?,
            ProbeAction::Silence {
                src,
                threshold_db,
                min_duration,
                parse,
            } => {
                let ranges = if let Some(text) = parse {
                    parse_silencedetect(&text)
                } else {
                    silence_detect(&src, threshold_db, min_duration)?
                };
                print_json(&ranges)?;
            }
            ProbeAction::Scenes {
                src,
                threshold,
                parse,
            } => {
                let ranges = if let Some(text) = parse {
                    parse_scene_times(&text)
                } else {
                    detect_scenes(&src, threshold)?
                };
                print_json(&ranges)?;
            }
            ProbeAction::Frames {
                src,
                times,
                out_dir,
                max_width,
            } => {
                let parsed: Vec<f64> = times
                    .split(',')
                    .filter(|part| !part.is_empty())
                    .map(|part| part.parse::<f64>())
                    .collect::<std::result::Result<_, _>>()
                    .context("times must be comma-separated numbers")?;
                print_json(&extract_frames(&src, &parsed, &out_dir, max_width)?)?;
            }
            ProbeAction::Frame {
                src,
                time,
                out_dir,
                max_width,
            } => {
                print_json(&extract_exact_frames(
                    &src,
                    &[time],
                    std::path::Path::new(&out_dir),
                    max_width,
                )?)?;
            }
            ProbeAction::Filmstrip {
                src,
                times,
                frames,
                project_dir,
                max_width,
                columns,
            } => {
                let meta = info(&src)?;
                let duration = meta.duration.unwrap_or(60.0).max(0.01);
                let times = if let Some(times) = times {
                    parse_times(&times)?
                } else {
                    let count = frames.clamp(1, 32);
                    (0..count)
                        .map(|index| duration * (index as f64 + 0.5) / count as f64)
                        .collect()
                };
                print_json(&cached_filmstrip(
                    &src,
                    &DerivativeCache::new(project_dir),
                    &times,
                    max_width,
                    columns,
                )?)?;
            }
            ProbeAction::Waveform {
                src,
                project_dir,
                buckets,
                include_image,
                image_width,
                image_height,
            } => {
                print_json(&cached_waveform(
                    &src,
                    &DerivativeCache::new(project_dir),
                    buckets,
                    include_image,
                    image_width,
                    image_height,
                )?)?;
            }
            ProbeAction::Transcribe { options } => {
                print_json(&run_transcription(&options, false)?)?;
            }
            ProbeAction::Listen { options } => {
                print_json(&run_transcription(&options, true)?)?;
            }
        },
        Commands::Encoders { text, detect } => {
            let listing = if let Some(text) = text {
                text
            } else if detect {
                let ffmpeg = resolve_ffmpeg_bin();
                let output =
                    run_allow_failure(ffmpeg, &["-hide_banner".into(), "-encoders".into()])?;
                format!("{}\n{}", output.stdout, output.stderr)
            } else {
                read_stdin()?
            };
            print_json(&parse_encoder_capabilities(&listing, resolve_ffmpeg_bin()))?;
        }
        Commands::Resources => {
            let budget = budget();
            print_json(&json!({
                "logicalCpus": budget.logical_cpus,
                "ffmpegThreads": budget.ffmpeg_threads,
                "decodeThreads": budget.decode_threads,
                "onnxIntraThreads": budget.onnx_intra_threads,
                "extractParallelism": budget.extract_parallelism,
                "trackingFps": budget.tracking_fps,
                "softwarePreset": budget.software_preset,
                "softwareCrf": budget.software_crf,
                "hardwareEncode": match budget.hardware_encode {
                    cut_engine::HardwareEncodePolicy::Auto => "auto",
                    cut_engine::HardwareEncodePolicy::Always => "hardware",
                    cut_engine::HardwareEncodePolicy::Never => "software",
                },
            }))?;
        }
        Commands::SelectEncoder {
            prefer_hardware,
            codec,
            software,
        } => {
            let raw: Value = serde_json::from_str(&read_stdin()?)?;
            let capabilities: Vec<cut_engine::EncoderCapability> =
                if let Some(encoders) = raw.get("encoders") {
                    serde_json::from_value(encoders.clone())?
                } else {
                    serde_json::from_value(raw)?
                };
            let mut options = SelectEncoderOptions {
                prefer_hardware,
                codec: match codec {
                    CliCodec::H264 => VideoCodec::H264,
                    CliCodec::Hevc => VideoCodec::Hevc,
                },
                ..Default::default()
            };
            if let Some(software) = software {
                options.software_encoder = software;
            }
            let selection = select_video_encoder(&capabilities, &options);
            let mut payload = serde_json::to_value(&selection)?;
            payload["args"] = json!(encoder_args(&selection));
            print_json(&payload)?;
        }
        Commands::CompileV1 { draft, post_filter } => {
            let timeline: Value = serde_json::from_str(&read_stdin()?)?;
            let video_post_filters = merge_post_filters(draft, post_filter);
            let compiled = compile_filter_graph_v1(&timeline, &video_post_filters)?;
            let duration = timeline_duration_v1(&timeline)?;
            print_json(&json!({
                "argv": compiled.argv,
                "command": compiled.command,
                "filterComplex": compiled.filter_complex,
                "duration": duration,
            }))?;
        }
        Commands::CompileV2 {
            include_embedded_audio,
            draft,
            post_filter,
        } => {
            let project: Value = serde_json::from_str(&read_stdin()?)?;
            let opts = CompileV2Options {
                video_post_filters: merge_post_filters(draft, post_filter),
                include_embedded_audio,
                range: None,
            };
            let compiled = compile_filter_graph_v2(&project, opts)?;
            let duration = timeline_duration_v2(&project)?;
            print_json(&json!({
                "argv": compiled.argv,
                "command": compiled.command,
                "filterComplex": compiled.filter_complex,
                "duration": duration,
                "width": compiled.width,
                "height": compiled.height,
            }))?;
        }
        Commands::DurationV1 => {
            let timeline: Value = serde_json::from_str(&read_stdin()?)?;
            print_json(&json!({ "duration": timeline_duration_v1(&timeline)? }))?;
        }
        Commands::DurationV2 => {
            let project: Value = serde_json::from_str(&read_stdin()?)?;
            print_json(&json!({ "duration": timeline_duration_v2(&project)? }))?;
        }
        Commands::RenderV1 {
            output,
            draft,
            run,
            cancel_file,
        } => {
            let timeline: Value = serde_json::from_str(&read_stdin()?)?;
            let opts = RenderV1Options {
                output: output.to_string_lossy().into_owned(),
                draft,
            };
            if run {
                let timeline = enrich_timeline_v1(&timeline)?;
                let cancellation = CancellationToken::new();
                cancellation.register_signal_handlers()?;
                let watcher = cancel_file.map(|path| {
                    cut_engine::render::spawn_cancel_file_watcher(path, cancellation.clone())
                });
                let result = cut_engine::render::render_v1_with_cancellation(
                    &timeline,
                    &opts,
                    &cancellation,
                    Some(stderr_progress_callback()),
                );
                cancellation.cancel();
                if let Some(watcher) = watcher {
                    let _ = watcher.join();
                }
                result?;
                print_json(&json!({ "ok": true, "output": opts.output }))?;
            } else {
                print_json(&json!({ "args": render_args_v1(&timeline, &opts)? }))?;
            }
        }
        Commands::RenderV2 {
            output,
            draft,
            prefer_hardware,
            software,
            run,
            include_embedded_audio,
            start,
            duration,
            cancel_file,
        } => {
            let project: Value = serde_json::from_str(&read_stdin()?)?;
            let opts = RenderV2Options {
                output: output.to_string_lossy().into_owned(),
                draft,
                include_embedded_audio,
                prefer_hardware,
                force_software: software,
                codec: VideoCodec::H264,
            };
            if run {
                let project = enrich_project_v2(&project)?;
                if start.is_some() != duration.is_some() {
                    bail!("render-v2 --start and --duration must be provided together");
                }
                let cancellation = CancellationToken::new();
                cancellation.register_signal_handlers()?;
                let watcher = cancel_file.map(|path| {
                    cut_engine::render::spawn_cancel_file_watcher(path, cancellation.clone())
                });
                let result = match (start, duration) {
                    (Some(start), Some(duration)) => {
                        cut_engine::render::render_v2_range_with_cancellation(
                            &project,
                            &opts,
                            start,
                            duration,
                            &cancellation,
                            Some(stderr_progress_callback()),
                        )
                    }
                    (None, None) => cut_engine::render::render_v2_with_cancellation(
                        &project,
                        &opts,
                        &cancellation,
                        Some(stderr_progress_callback()),
                    ),
                    _ => unreachable!("render range presence was checked above"),
                };
                cancellation.cancel();
                if let Some(watcher) = watcher {
                    let _ = watcher.join();
                }
                result?;
                print_json(&json!({ "ok": true, "output": opts.output }))?;
            } else {
                print_json(&json!({ "args": render_args_v2(&project, &opts)? }))?;
            }
        }
        Commands::ProxyKey {
            source,
            size,
            mtime_ms,
            id,
        } => {
            let settings: Value =
                serde_json::from_str(&read_stdin()?).unwrap_or_else(|_| json!({}));
            let key = create_proxy_cache_key(
                &source,
                id.as_deref(),
                &ProxySourceStat {
                    size,
                    mtime_ms,
                    ..ProxySourceStat::default()
                },
                &settings,
            )?;
            print_json(&json!({ "key": key }))?;
        }
        Commands::ProxyArgs {
            input,
            output,
            width,
            height,
            video_codec,
        } => {
            let stdin = read_stdin().unwrap_or_default();
            let mut settings = if stdin.trim().is_empty() {
                ProxySettings::default()
            } else {
                let parsed: Value = serde_json::from_str(&stdin).unwrap_or_else(|_| json!({}));
                proxy_settings_from_value(&parsed)
            };
            if width.is_some() {
                settings.width = width;
            }
            if height.is_some() {
                settings.height = height;
            }
            if video_codec.is_some() {
                settings.video_codec = video_codec;
            }
            print_json(&proxy_ffmpeg_args(&input, &output, &settings)?)?;
        }
        Commands::ProxyRun { input, output } => {
            let stdin = read_stdin().unwrap_or_default();
            let settings = if stdin.trim().is_empty() {
                ProxySettings::default()
            } else {
                let parsed: Value = serde_json::from_str(&stdin).unwrap_or_else(|_| json!({}));
                proxy_settings_from_value(&parsed)
            };
            let duration = settings.duration;
            proxy_run(&input, &output, &settings, duration)?;
            print_json(&json!({ "ok": true, "output": output }))?;
        }
        Commands::Normalize {
            label_map,
            unknown,
            fps,
            frame_width,
            frame_height,
        } => {
            let stdout = read_stdin()?;
            print_json(&normalize_stdout(
                &stdout,
                &label_map,
                &unknown,
                fps,
                frame_width,
                frame_height,
            )?)?;
        }
        Commands::Plan {
            min_confidence,
            max_gap,
            smoothing,
            crop_aspect,
        } => {
            let input: Value = serde_json::from_str(&read_stdin()?)?;
            let detections: Vec<cut_engine::vision::NormalizedDetection> =
                if let Some(list) = input.get("detections") {
                    serde_json::from_value(list.clone())?
                } else {
                    serde_json::from_value(input.clone())?
                };
            let plan = build_tracking_plan(
                TrackingPlanInput::Detections(&detections),
                &TrackingPlanOptions {
                    min_confidence,
                    max_gap_seconds: max_gap,
                    smoothing_factor: smoothing,
                    ..TrackingPlanOptions::default()
                },
            )?;
            let mut payload = serde_json::to_value(&plan)?;
            if let Some(aspect) = crop_aspect {
                payload["crop"] = serde_json::to_value(crop_plan_from_json(&json!({
                    "aspectRatio": aspect
                }))?)?;
            }
            print_json(&payload)?;
        }
        Commands::Overlay { action } => match action {
            OverlayAction::EscapePath { path } => print_json(&escape_filter_path(&path))?,
            OverlayAction::EscapeDrawtext { text } => print_json(&escape_drawtext_text(&text))?,
            OverlayAction::Atempo { speed } => print_json(&atempo_filters(speed)?)?,
            OverlayAction::Captions => {
                let input: Value = serde_json::from_str(&read_stdin()?)?;
                let cue = CaptionCue {
                    start: input
                        .get("start")
                        .and_then(Value::as_f64)
                        .context("captions require start")?,
                    end: input
                        .get("end")
                        .and_then(Value::as_f64)
                        .context("captions require end")?,
                    text: input
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                };
                print_json(&captions_filter(&cue, input.get("style")))?;
            }
            OverlayAction::Subtitles => {
                let input: Value = serde_json::from_str(&read_stdin()?)?;
                let path = input
                    .get("path")
                    .and_then(Value::as_str)
                    .context("subtitles require path")?;
                print_json(&subtitles_filter(path, input.get("style")))?;
            }
        },
        Commands::Crop => {
            let input: Value = serde_json::from_str(&read_stdin()?)?;
            print_json(&crop_plan_from_json(&input)?)?;
        }
        Commands::Track {
            source,
            weights,
            label_map,
            conf,
            iou,
            fps,
            imgsz,
            max_frames,
        } => {
            let stdout = read_stdin().unwrap_or_default();
            if source.is_none() && !stdout.trim().is_empty() {
                let labels = label_map.unwrap_or_else(|| "{}".into());
                print_json(&normalize_stdout(
                    &stdout, &labels, "ignore", None, None, None,
                )?)?;
                return Ok(());
            }
            let source = source.context("track requires --source or JSONL on stdin")?;
            let weights = weights.context("track requires --weights")?;
            let labels = parse_label_map(&label_map.unwrap_or_else(|| "{}".into()))?;
            let config = validate_yolo26_config(&Yolo26TrackingConfig {
                weights: weights.clone(),
                label_map: labels,
                targets: None,
                cwd: None,
                tracker: None,
                confidence: conf,
                conf: None,
                iou,
                fps,
                device: None,
                timeout_ms: None,
                unknown_label_policy: Some(UnknownLabelPolicy::Ignore),
            })?;
            let native_weights = cut_engine::vision::resolve_inference_weights(&config.weights)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let frames = native_track(
                &source,
                native_weights
                    .to_str()
                    .context("weights path must be UTF-8")?,
                max_frames,
                conf,
                iou,
                fps.or(config.fps),
                imgsz,
            )?;
            print_json(&normalize_detection_frames(
                &frames,
                &NormalizeOptions {
                    label_map: config.label_map,
                    unknown_label_policy: config.unknown_label_policy,
                    fps: config.fps,
                    frame_width: None,
                    frame_height: None,
                },
            )?)?;
        }
    }
    Ok(())
}

fn merge_post_filters(draft: bool, post_filter: Vec<String>) -> Vec<String> {
    if !post_filter.is_empty() {
        post_filter
    } else if draft {
        vec!["scale=-2:360".into(), "fps=15".into()]
    } else {
        Vec::new()
    }
}

fn parse_label_map(raw: &str) -> Result<HashMap<String, VisionTarget>> {
    let value: Value = serde_json::from_str(raw)?;
    let mut map = HashMap::new();
    let Some(object) = value.as_object() else {
        bail!("label_map must be a JSON object");
    };
    for (key, value) in object {
        let Some(label) = value
            .as_str()
            .and_then(|value| value.parse::<VisionTarget>().ok())
        else {
            bail!("label_map.{key} must be cam, chat, or content");
        };
        map.insert(key.clone(), label);
    }
    Ok(map)
}

fn normalize_stdout(
    stdout: &str,
    label_map: &str,
    unknown: &str,
    fps: Option<f64>,
    frame_width: Option<f64>,
    frame_height: Option<f64>,
) -> Result<cut_engine::vision::ParsedTrackingOutput> {
    let parsed = parse_label_map(label_map)?;
    let map = cut_engine::vision::resolve_label_map(&parsed)?;
    let policy = if unknown == "error" {
        UnknownLabelPolicy::Error
    } else {
        UnknownLabelPolicy::Ignore
    };
    Ok(parse_and_normalize_tracking_output(
        stdout,
        &NormalizeOptions {
            label_map: map,
            unknown_label_policy: policy,
            fps,
            frame_width,
            frame_height,
        },
    )?)
}

fn native_track(
    source: &str,
    weights: &str,
    max_frames: Option<usize>,
    conf: Option<f64>,
    iou: Option<f64>,
    fps: Option<f64>,
    imgsz: Option<u32>,
) -> Result<Vec<cut_engine::vision::DetectionFrame>> {
    let mut engine = YoloEngine::from_weights(weights)?;
    if let Some(conf) = conf {
        engine.set_confidence(conf as f32);
    }
    if let Some(iou) = iou {
        engine.set_iou(iou as f32);
    }
    let sample_fps = tracking_sample_fps(fps);
    let imgsz = imgsz.unwrap_or(DEFAULT_IMGSZ);
    let mut frames = FfmpegRgbFrameSource::open_with(
        source,
        TrackDecodeOptions {
            max_frames,
            fps: Some(sample_fps),
            imgsz,
        },
    )?;
    Ok(engine.track_video_frames(
        &mut frames,
        &cut_engine::vision::TrackOptions {
            max_frames,
            fps: Some(sample_fps),
            conf: conf.map(|value| value as f32),
            iou: iou.map(|value| value as f32),
            imgsz: Some(imgsz),
            ..cut_engine::vision::TrackOptions::default()
        },
        None,
    )?)
}

fn run_transcription(
    args: &TranscriptionArgs,
    listen: bool,
) -> Result<cut_engine::TranscriptionResult> {
    if listen && args.start.is_none() {
        bail!("listen requires --start");
    }
    if listen && args.end.is_none() && args.duration.is_none() {
        bail!("listen requires --end or --duration");
    }
    if args.end.is_some() && args.duration.is_some() {
        bail!("use either --end or --duration, not both");
    }
    let metadata = info(&args.src)?;
    let start = args.start.unwrap_or(0.0);
    let end = if let Some(end) = args.end {
        end
    } else if let Some(duration) = args.duration {
        if !duration.is_finite() || duration <= 0.0 {
            bail!("--duration must be finite and positive");
        }
        start + duration
    } else {
        metadata
            .duration
            .context("source duration is unavailable; pass --end or --duration")?
    };
    let range = TimeRange::new(start, end).context("invalid transcription range")?;
    let limit = if listen {
        cut_engine::MAX_LISTEN_RANGE_SECONDS
    } else {
        cut_engine::MAX_TRANSCRIPTION_RANGE_SECONDS
    };
    if range.duration() > limit + 1e-9 {
        bail!("selected range exceeds the {limit:.0}s limit");
    }
    if let Some(duration) = metadata.duration {
        if !duration.is_finite() || duration <= 0.0 || range.end > duration + 1e-6 {
            bail!("selected range must be inside source duration {duration:.3}s");
        }
    }

    let backend = match args.backend.as_str() {
        "local" | "whisper.cpp" | "whisper_cpp" => TranscriptionBackend::LocalWhisperCpp,
        "provider" => TranscriptionBackend::ExplicitProvider {
            name: "provider".into(),
        },
        other => bail!("unknown transcription backend {other:?}; expected local or provider"),
    };
    let mut options = TranscriptionOptions::new(range);
    options.backend = backend;
    options.model = args.model.clone();
    options.model_version = args.model_version.clone();
    options.backend_version = args.backend_version.clone();
    options.language = args.language.clone();

    let cancellation = CancellationToken::new();
    let mut last_stage: Option<TranscriptionStage> = None;
    let mut last_percent = -1;
    let mut on_progress = |progress: TranscriptionProgress| {
        let percent = (progress.progress * 100.0).round() as i32;
        if last_stage != Some(progress.stage) || percent == 100 || percent >= last_percent + 5 {
            eprintln!("transcription {:?}: {percent}%", progress.stage);
            last_stage = Some(progress.stage);
            last_percent = percent;
        }
    };
    if listen {
        Ok(cut_engine::listen_cached(
            &args.src,
            &DerivativeCache::new(&args.project_dir),
            &options,
            &cancellation,
            &mut on_progress,
        )?)
    } else {
        Ok(cut_engine::transcribe_cached(
            &args.src,
            &DerivativeCache::new(&args.project_dir),
            &options,
            &cancellation,
            &mut on_progress,
        )?)
    }
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

fn parse_times(value: &str) -> Result<Vec<f64>> {
    value
        .split(',')
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .context("times must be comma-separated numbers")
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
