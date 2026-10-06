use std::io::{self, Read};
use std::path::PathBuf;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use cut_engine::derivatives::DerivativeCache;
use cut_engine::encoder::{
    encoder_args, parse_encoder_capabilities, select_video_encoder, SelectEncoderOptions,
    VideoCodec,
};
use cut_engine::ffmpeg::{resolve_ffmpeg_bin, run_allow_failure};
use cut_engine::probe::{
    cached_filmstrip, cached_waveform, detect_scenes, extract_exact_frames, extract_frames, info,
    parse_scene_times, parse_silencedetect, silence_detect,
};
use cut_engine::proxy::{
    create_proxy_cache_key, proxy_ffmpeg_args, proxy_run, proxy_settings_from_value, ProxySettings,
    ProxySourceStat,
};
use cut_engine::resources::budget;
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
                "extractParallelism": budget.extract_parallelism,
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
    }
    Ok(())
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
