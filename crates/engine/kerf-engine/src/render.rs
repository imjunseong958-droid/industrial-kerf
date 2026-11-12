//! Render argument construction and ffmpeg spawn for `kerf-model` projects.
//!
//! A render draws the picture with the compositor (`kerf-compose`) and
//! pipes its frames to FFmpeg, which encodes them with the program's sound
//! as the graph mixes it. Effects draw as part of each frame, so nothing is
//! baked first.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use kerf_compose::{ComposeOptions, Compositor, Frame};
use kerf_model::{ColorSpace, Rate, Time};
use serde::{Deserialize, Serialize};

use crate::encoder::{
    encode_output_args, keep_encoders_that_open, parse_encoder_capabilities, select_video_encoder,
    EncoderSelection, HardwareEncoderFamily, SelectEncoderOptions, VideoCodec,
};
use crate::error::{EngineError, Result};
use crate::ffmpeg::{
    cached_encoder_listing, parse_time_seconds, resolve_ffmpeg_bin, run_allow_failure, tail,
};
use crate::program::{compile_program, ProgramOptions};
use crate::resources::{budget, should_try_hardware_encode};
use crate::sidecar::{self, CancellationToken};

const CANCEL_FILE_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub fn detect_and_select_encoder(options: &SelectEncoderOptions) -> Result<EncoderSelection> {
    let ffmpeg = resolve_ffmpeg_bin();
    let listing = cached_encoder_listing()?;
    let mut capabilities = parse_encoder_capabilities(listing, ffmpeg);
    keep_encoders_that_open(&mut capabilities.encoders, options, |family| {
        hardware_encoder_opens(family, options.codec)
    });
    Ok(select_video_encoder(&capabilities.encoders, options))
}

/// Whether `family`'s encoder for `codec` encodes a frame with the
/// arguments and pixel format a render gives it. Tried once per process.
fn hardware_encoder_opens(family: HardwareEncoderFamily, codec: VideoCodec) -> bool {
    static OPENS: Mutex<BTreeMap<&'static str, bool>> = Mutex::new(BTreeMap::new());
    let encoder = family.encoder_name(codec);
    let mut opens = OPENS.lock().unwrap_or_else(PoisonError::into_inner);
    *opens.entry(encoder).or_insert_with(|| {
        let selection = EncoderSelection {
            encoder: encoder.to_owned(),
            hardware: true,
            family: Some(family),
            reason: String::new(),
        };
        let budget = budget();
        let mut args: Vec<String> = [
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:s=256x256:r=30",
            "-frames:v",
            "1",
        ]
        .map(String::from)
        .into();
        args.extend(encode_output_args(
            &selection,
            budget.software_crf,
            budget.software_preset,
        ));
        args.extend(["-pix_fmt", "yuv420p", "-f", "null", "-"].map(String::from));
        run_allow_failure(resolve_ffmpeg_bin(), &args).is_ok_and(|output| output.code == 0)
    })
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

/// How a master converts drawn RGBA: the Rec.709 matrix into limited
/// range, with the frames tagged so the encoder writes the tags too.
const MASTER_CONVERT: &str = "scale=out_color_matrix=bt709:out_range=tv,\
    setparams=colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv";
/// The same for an HDR master, whose frames come as its signal.
const MASTER_CONVERT_PQ: &str = "scale=out_color_matrix=bt2020nc:out_range=tv,\
    setparams=colorspace=bt2020nc:color_primaries=bt2020:color_trc=smpte2084:range=tv";
const MASTER_CONVERT_HLG: &str = "scale=out_color_matrix=bt2020nc:out_range=tv,\
    setparams=colorspace=bt2020nc:color_primaries=bt2020:color_trc=arib-std-b67:range=tv";

/// The review copy's video bit rate.
pub const REVIEW_BITRATE: &str = "12M";
const REVIEW_BUFFER: &str = "24M";

/// What a render's file is for, which decides how it is encoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RenderPreset {
    /// [`RenderOptions::codec`] at the resource budget's quality, as
    /// renders have always been encoded.
    #[default]
    #[serde(rename = "standard")]
    Standard,
    /// A review copy: full size and rate, H.264 at 12 Mb/s, in MP4.
    #[serde(rename = "review")]
    Review,
    /// The master: Apple ProRes 422 HQ, 10-bit 4:2:2, tagged Rec.709
    /// limited range, in QuickTime with 24-bit PCM sound.
    #[serde(rename = "prores_hq")]
    ProRes422Hq,
    /// HDR delivery: HEVC Main 10 in MP4, from a `rec2100-pq` or `rec2100-hlg` sequence, tagged, and
    /// for PQ with HDR10's mastering display and content light levels. Encoded by `libx265`.
    #[serde(rename = "hevc_hdr")]
    HevcHdr,
}

impl RenderPreset {
    pub const ALL: [Self; 4] = [
        Self::Standard,
        Self::Review,
        Self::ProRes422Hq,
        Self::HevcHdr,
    ];

    /// `standard`, `review`, `prores_hq` or `hevc_hdr`, as the bridge and the runtime
    /// name them.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Review => "review",
            Self::ProRes422Hq => "prores_hq",
            Self::HevcHdr => "hevc_hdr",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "standard" => Some(Self::Standard),
            "review" => Some(Self::Review),
            "prores_hq" | "prores" | "prores_422_hq" | "prores422hq" => Some(Self::ProRes422Hq),
            "hevc_hdr" | "hdr10" => Some(Self::HevcHdr),
            _ => None,
        }
    }

    /// The file extension the preset's container takes.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Standard | Self::Review | Self::HevcHdr => "mp4",
            Self::ProRes422Hq => "mov",
        }
    }

    /// A short description for a render queue: `ProRes 422 HQ`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "H.264",
            Self::Review => "H.264 review",
            Self::ProRes422Hq => "ProRes 422 HQ",
            Self::HevcHdr => "HEVC 10-bit HDR",
        }
    }
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
    /// What the file is for. A draft has its own encoding and takes only
    /// [`RenderPreset::Standard`].
    pub preset: RenderPreset,
    /// The display an HDR10 file says it was mastered on. Its light levels are measured.
    pub mastering: MasteringDisplay,
    /// HDR10's content light levels when already known; otherwise they are measured from the frames
    /// before encoding.
    pub content_light: Option<ContentLight>,
    /// The size of the file's picture when it is not the sequence's, as a guide drawn at a
    /// model's size is. The picture fills it, keeping its shape, and what is over is cut off
    /// evenly from both sides; it is drawn at that size, not drawn whole and brought down.
    pub size: Option<(u32, u32)>,
    /// The rate frames are drawn at when it is not the sequence's: each is the sequence at
    /// that frame's time, so the file plays as long as the sequence does.
    pub rate: Option<Rate>,
    /// Whether the file carries the sequence's sound. A guide a model follows carries none.
    pub sound: bool,
}

/// The display a picture was graded on: SMPTE ST 2086, which an HDR10 file carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MasteringDisplay {
    /// Primaries and white point in CIE xy: green, blue, red, white.
    pub primaries: [[f64; 2]; 4],
    /// Peak and black, in nits.
    pub peak: f64,
    pub black: f64,
}

impl MasteringDisplay {
    /// Display P3 with a D65 white, as most HDR is graded on, at 1000 nits.
    pub const P3_1000: Self = Self {
        primaries: [[0.265, 0.690], [0.150, 0.060], [0.680, 0.320], [0.3127, 0.3290]],
        peak: 1000.0,
        black: 0.0001,
    };

    /// As `x265` takes it: chromaticities in 50,000ths, luminance in 10,000ths of a nit.
    fn x265(&self) -> String {
        let point = |p: [f64; 2]| {
            format!(
                "({},{})",
                (p[0] * 50_000.0).round() as u32,
                (p[1] * 50_000.0).round() as u32
            )
        };
        let [g, b, r, w] = self.primaries;
        format!(
            "G{}B{}R{}WP{}L({},{})",
            point(g),
            point(b),
            point(r),
            point(w),
            (self.peak * 10_000.0).round() as u64,
            (self.black * 10_000.0).round() as u64
        )
    }
}

impl Default for MasteringDisplay {
    fn default() -> Self {
        Self::P3_1000
    }
}

/// How bright a picture is, as CTA-861.3 measures it, in nits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContentLight {
    /// The brightest any pixel's brightest channel gets.
    pub max_cll: u32,
    /// The brightest any frame's average of those gets.
    pub max_fall: u32,
}

/// Measures 16-bit PQ frames as they pass.
struct LightMeter {
    nits: Vec<f32>,
    max_cll: f32,
    max_fall: f32,
}

impl LightMeter {
    fn new() -> Self {
        Self {
            nits: (0..=u16::MAX)
                .map(|code| kerf_color::pq_eotf(f32::from(code) / 65_535.0) * 10_000.0)
                .collect(),
            max_cll: 0.0,
            max_fall: 0.0,
        }
    }

    /// `rgba` is 16-bit little-endian RGBA, holding the PQ signal.
    fn add(&mut self, rgba: &[u8]) {
        let (mut brightest, mut sum, mut count) = (0.0f32, 0.0f64, 0u64);
        for pixel in rgba.as_chunks::<8>().0 {
            let channel = |at: usize| u16::from_le_bytes([pixel[at], pixel[at + 1]]) as usize;
            let code = channel(0).max(channel(2)).max(channel(4));
            let nits = self.nits[code];
            brightest = brightest.max(nits);
            sum += f64::from(nits);
            count += 1;
        }
        self.max_cll = self.max_cll.max(brightest);
        self.max_fall = self.max_fall.max((sum / count.max(1) as f64) as f32);
    }

    fn levels(&self) -> ContentLight {
        ContentLight {
            max_cll: self.max_cll.round() as u32,
            max_fall: self.max_fall.round() as u32,
        }
    }
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
            preset: RenderPreset::Standard,
            mastering: MasteringDisplay::P3_1000,
            content_light: None,
            size: None,
            rate: None,
            sound: true,
        }
    }
}

/// How a render's output is encoded: the video codec arguments, the pixel
/// format, a conversion the picture needs first, and the sound, container
/// and colour tag arguments that follow.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Encoding {
    video: Vec<String>,
    pix_fmt: &'static str,
    /// A filter that converts drawn RGBA frames the way the codec needs.
    convert: Option<&'static str>,
    output: Vec<String>,
}

fn encoding(opts: &RenderOptions, color: ColorSpace) -> Result<Encoding> {
    let plain = |video| Encoding {
        video,
        pix_fmt: "yuv420p",
        convert: None,
        output: Vec::new(),
    };
    if opts.draft {
        if opts.preset != RenderPreset::Standard {
            return Err(EngineError::msg(format!(
                "a draft render has its own encoding; it cannot be a {} render",
                opts.preset.label()
            )));
        }
        return Ok(plain(draft_codec_args()));
    }
    match opts.preset {
        RenderPreset::Standard => Ok(plain(hardware_or_software_codec(
            opts.prefer_hardware,
            opts.force_software,
            opts.codec,
        )?)),
        RenderPreset::Review => {
            let mut encoding = plain(review_codec_args(
                opts.prefer_hardware,
                opts.force_software,
            )?);
            encoding.output = ["-c:a", "aac", "-b:a", "320k", "-movflags", "+faststart"]
                .map(String::from)
                .into();
            Ok(encoding)
        }
        RenderPreset::HevcHdr => hevc_hdr_encoding(opts, color),
        RenderPreset::ProRes422Hq => {
            let (video, pix_fmt) = prores_hq_args(opts.prefer_hardware, opts.force_software);
            let (matrix, primaries, transfer, convert) = match color {
                ColorSpace::Rec2100Pq => ("bt2020nc", "bt2020", "smpte2084", MASTER_CONVERT_PQ),
                ColorSpace::Rec2100Hlg => {
                    ("bt2020nc", "bt2020", "arib-std-b67", MASTER_CONVERT_HLG)
                }
                _ => ("bt709", "bt709", "bt709", MASTER_CONVERT),
            };
            let mut output: Vec<String> = [
                "-c:a",
                "pcm_s24le",
                "-colorspace",
                matrix,
                "-color_primaries",
                primaries,
                "-color_trc",
                transfer,
                "-color_range",
                "tv",
                // The ProRes frame headers carry the tags too, whichever
                // way the picture was drawn.
                "-bsf:v",
                &format!(
                    "prores_metadata=color_primaries={primaries}:color_trc={transfer}:colorspace={matrix}"
                ),
                "-f",
                "mov",
            ]
            .map(String::from)
            .into();
            if pix_fmt == "yuv422p10le" {
                // prores_ks tags the file as Apple's encoder does.
                output.splice(0..0, ["-vendor".to_owned(), "apl0".to_owned()]);
            }
            Ok(Encoding {
                video,
                pix_fmt,
                convert: Some(convert),
                output,
            })
        }
    }
}

/// HEVC Main 10 from `libx265`, whose stream carries the transfer and, for PQ, HDR10's metadata.
fn hevc_hdr_encoding(opts: &RenderOptions, color: ColorSpace) -> Result<Encoding> {
    if !encoder_available("libx265") {
        return Err(EngineError::msg(
            "HEVC HDR needs an FFmpeg with libx265, which this one does not have",
        ));
    }
    let (transfer, convert, hdr) = match color {
        ColorSpace::Rec2100Pq => ("smpte2084", MASTER_CONVERT_PQ, true),
        ColorSpace::Rec2100Hlg => ("arib-std-b67", MASTER_CONVERT_HLG, false),
        _ => {
            return Err(EngineError::msg(
                "HEVC HDR needs a sequence finished in HDR: color=rec2100-pq or rec2100-hlg",
            ))
        }
    };
    let mut params = format!(
        "profile=main10:repeat-headers=1:colorprim=bt2020:transfer={transfer}:colormatrix=bt2020nc:range=limited"
    );
    if hdr {
        params.push_str(":hdr10=1:hdr10-opt=1");
        params.push_str(&format!(":master-display={}", opts.mastering.x265()));
        let light = opts.content_light.unwrap_or_default();
        params.push_str(&format!(":max-cll={},{}", light.max_cll, light.max_fall));
    } else {
        // HLG for players that read only the alternative transfer.
        params.push_str(":atc-sei=18:pic-struct=0");
    }
    let budget = budget();
    let video: Vec<String> = [
        "-c:v".to_owned(),
        "libx265".to_owned(),
        "-crf".to_owned(),
        budget.software_crf.to_string(),
        "-preset".to_owned(),
        budget.software_preset.to_string(),
        "-profile:v".to_owned(),
        "main10".to_owned(),
        "-x265-params".to_owned(),
        params,
        // QuickTime and Apple's players want it tagged so.
        "-tag:v".to_owned(),
        "hvc1".to_owned(),
    ]
    .into();
    let output = [
        "-c:a",
        "aac",
        "-b:a",
        "320k",
        "-colorspace",
        "bt2020nc",
        "-color_primaries",
        "bt2020",
        "-color_trc",
        transfer,
        "-color_range",
        "tv",
        "-movflags",
        "+faststart",
    ]
    .map(String::from)
    .into();
    Ok(Encoding {
        video,
        pix_fmt: "yuv420p10le",
        convert: Some(convert),
        output,
    })
}

/// H.264 at [`REVIEW_BITRATE`], on a hardware encoder when the budget
/// allows one and it opens.
fn review_codec_args(prefer_hardware: bool, force_software: bool) -> Result<Vec<String>> {
    let budget = budget();
    let selection = if should_try_hardware_encode(prefer_hardware, force_software) {
        detect_and_select_encoder(&SelectEncoderOptions {
            prefer_hardware: true,
            codec: VideoCodec::H264,
            ..Default::default()
        })?
    } else {
        select_video_encoder(&[], &SelectEncoderOptions::default())
    };
    let mut args = crate::encoder::encoder_args(&selection);
    args.extend(
        [
            "-b:v",
            REVIEW_BITRATE,
            "-maxrate",
            REVIEW_BITRATE,
            "-bufsize",
            REVIEW_BUFFER,
        ]
        .map(String::from),
    );
    if !selection.hardware {
        args.extend(["-preset".to_owned(), budget.software_preset.to_owned()]);
    }
    Ok(args)
}

/// ProRes 422 HQ: Apple's VideoToolbox encoder on a Mac when the budget
/// allows hardware and it opens here, otherwise FFmpeg's `prores_ks`
/// profile 3. The video arguments and the pixel format each takes.
fn prores_hq_args(prefer_hardware: bool, force_software: bool) -> (Vec<String>, &'static str) {
    if cfg!(target_os = "macos")
        && should_try_hardware_encode(prefer_hardware, force_software)
        && prores_videotoolbox_opens()
    {
        return (
            ["-c:v", "prores_videotoolbox", "-profile:v", "hq"]
                .map(String::from)
                .into(),
            "p210le",
        );
    }
    (
        ["-c:v", "prores_ks", "-profile:v", "3"]
            .map(String::from)
            .into(),
        "yuv422p10le",
    )
}

/// Whether `prores_videotoolbox` is listed and encodes a 10-bit 4:2:2
/// frame. Tried once per process: a sandbox or a busy media engine can
/// refuse a compression session.
fn prores_videotoolbox_opens() -> bool {
    static OPENS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OPENS.get_or_init(|| {
        let listed = cached_encoder_listing().is_ok_and(|listing| {
            listing.lines().any(|line| {
                line.split_whitespace()
                    .any(|token| token == "prores_videotoolbox")
            })
        });
        listed
            && run_allow_failure(
                resolve_ffmpeg_bin(),
                &[
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=black:s=256x144:r=24",
                    "-frames:v",
                    "1",
                    "-c:v",
                    "prores_videotoolbox",
                    "-profile:v",
                    "hq",
                    "-pix_fmt",
                    "p210le",
                    "-f",
                    "null",
                    "-",
                ]
                .map(String::from),
            )
            .is_ok_and(|output| output.code == 0)
    })
}

/// Whether the encoder `name` is in this FFmpeg's `-encoders` listing.
pub fn encoder_available(name: &str) -> bool {
    cached_encoder_listing().is_ok_and(|listing| {
        listing
            .lines()
            .any(|line| line.split_whitespace().any(|token| token == name))
    })
}

/// Whether `sequence` of `project` (the first when absent) renders as
/// [`render_program`] would render it: the error says what it cannot draw
/// or mix. Reads no media.
pub fn renderable(project: &kerf_model::Project, sequence: Option<&str>) -> Result<()> {
    let catalog = kerf_model::Catalog::builtin();
    Compositor::new(
        project,
        catalog,
        &ComposeOptions {
            sequence: sequence.map(str::to_owned),
            captions: true,
            ..Default::default()
        },
    )?;
    compile_program(
        project,
        catalog,
        &ProgramOptions {
            sequence: sequence.map(str::to_owned),
            ..ProgramOptions::default()
        },
    )?;
    Ok(())
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
    render_composited(project, sequence, range, opts, cancellation, on_progress)
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

/// The FFmpeg arguments (without the binary) that mix the sound of `[from, to)` of a sequence
/// (all of it when no range) to `output` as mono 16-bit PCM in a WAV file at `sample_rate`,
/// with no picture read, drawn or written.
fn sound_args(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(Time, Time)>,
    sample_rate: u32,
    output: &Path,
) -> Result<Vec<String>> {
    let heard = crate::ramps::with_sound(project);
    let sound = compile_program(
        &heard,
        kerf_model::Catalog::builtin(),
        &ProgramOptions {
            sequence: sequence.map(str::to_owned),
            range,
        },
    )?;
    let mut args = vec!["-hide_banner".to_owned(), "-nostats".to_owned()];
    args.extend(sound.argv.iter().skip(1).cloned());
    args.extend(
        ["-ac", "1", "-ar"]
            .map(String::from)
            .into_iter()
            .chain([sample_rate.to_string()])
            .chain(["-c:a", "pcm_s16le", "-t"].map(String::from))
            .chain([crate::program::secs(sound.duration)])
            .chain(["-f", "wav"].map(String::from))
            .chain([output.to_string_lossy().into_owned()]),
    );
    Ok(args)
}

/// Mix the sound of `[from, to)` of a sequence (all of it when no range) to `output` alone:
/// mono 16-bit PCM in a WAV file at `sample_rate`, which is what a transcriber reads. No frame
/// is drawn, so it takes the time the sound takes to decode and mix, not the picture's.
pub fn render_sound(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(Time, Time)>,
    sample_rate: u32,
    output: &Path,
    cancellation: &CancellationToken,
) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    let mut command = crate::ffmpeg::hidden_command(resolve_ffmpeg_bin());
    command.args(sound_args(project, sequence, range, sample_rate, output)?);
    let finished = sidecar::run(command, cancellation, |_| {})?;
    if !finished.status.success() {
        return Err(EngineError::Ffmpeg {
            code: finished.status.code().unwrap_or(-1),
            stderr: tail(finished.stderr.trim(), 2000).to_owned(),
        });
    }
    Ok(())
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
    /// Frames go to FFmpeg at 16 bits a component.
    deep: bool,
}

/// The share of a `canvas` to draw so that the picture fills `size`: the larger of the two
/// sides' shares, within what the compositor draws. What is over on the other side is cut off,
/// and a size larger than the canvas is scaled up to.
fn filling(canvas: (u32, u32), size: (u32, u32)) -> f64 {
    let share = |wanted: u32, have: u32| f64::from(wanted) / f64::from(have.max(1));
    share(size.0, canvas.0)
        .max(share(size.1, canvas.1))
        .clamp(1.0 / 16.0, 1.0)
}

/// How long `[from, to)` of a sequence is (all of it when no range), and its rate, read
/// without its sound.
fn picture_span(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(Time, Time)>,
) -> Result<(Time, Rate)> {
    let top = kerf_core::program::program_sequence(project, sequence)?;
    let (from, to) = range.unwrap_or((Time::ZERO, top.duration()));
    if from.is_negative() || to <= from {
        return Err(EngineError::msg(if range.is_some() {
            "a render range needs 0 <= from < to".to_owned()
        } else {
            format!("sequence `{}` is empty", top.id)
        }));
    }
    Ok((to - from, top.rate))
}

/// The frames of `[from, to)` of a sequence (all of it when no range) and
/// the FFmpeg command encoding them, with the sound unless the options
/// leave it out. A draft is drawn 360 pixels tall at 15 fps; a size and a
/// rate the options name are drawn in place of the sequence's, or a
/// draft's.
fn composited_render(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(Time, Time)>,
    opts: &RenderOptions,
    canvas: (u32, u32),
) -> Result<CompositedRender> {
    // The graph mixes the sound; the picture comes from stdin.
    let (mut args, duration, own_rate) = if opts.sound {
        let heard = crate::ramps::with_sound(project);
        let sound = compile_program(
            &heard,
            kerf_model::Catalog::builtin(),
            &ProgramOptions {
                sequence: sequence.map(str::to_owned),
                range,
            },
        )?;
        let args: Vec<String> = sound.argv.iter().skip(1).cloned().collect();
        (args, sound.duration, sound.rate)
    } else {
        let (duration, rate) = picture_span(project, sequence, range)?;
        (vec!["-y".to_owned()], duration, rate)
    };
    let from = range.map_or(Time::ZERO, |(from, _)| from);
    let rate = match opts.rate {
        Some(rate) => rate,
        None if opts.draft => Rate::new(15, 1).expect("15 fps is a rate"),
        None => own_rate,
    };
    let scale = match opts.size {
        Some(size) => filling(canvas, size),
        None if opts.draft => (360.0 / f64::from(canvas.1.max(1))).min(1.0),
        None => 1.0,
    };
    let frame = rate.frame_duration().flicks();
    let ceil = |time: Time| -(-time.flicks()).div_euclid(frame);
    let count = ceil(duration).max(1);
    let first = ceil(from);
    let times = (first..first + count)
        .map(|index| rate.time_of(index))
        .collect();
    let size = (
        ((f64::from(canvas.0) * scale).round() as u32).max(1),
        ((f64::from(canvas.1) * scale).round() as u32).max(1),
    );
    let color = kerf_core::program::program_sequence(project, sequence)
        .map_or(ColorSpace::default(), |top| top.color);
    let encoding = encoding(opts, color)?;
    // A deeper file is worth deeper frames: 8 bits would only be stretched to fill it.
    let deep = encoding.pix_fmt != "yuv420p";
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
            if deep { "rgba64le" } else { "rgba" }.to_owned(),
            "-s".to_owned(),
            format!("{}x{}", size.0, size.1),
            "-framerate".to_owned(),
            crate::program::rate_arg(rate),
            "-i".to_owned(),
            "pipe:0".to_owned(),
        ],
    );
    args.extend(["-map".to_owned(), format!("{picture}:v")]);
    if !opts.sound {
        args.push("-an".to_owned());
    }
    args.extend(encoding.video);
    let mut filters = Vec::new();
    // What was drawn fills the size asked for; what is over is cut off evenly.
    let written = match opts.size.filter(|wanted| *wanted != size) {
        Some((width, height)) => {
            filters.push(format!(
                "scale={width}:{height}:force_original_aspect_ratio=increase:flags=bicubic,crop={width}:{height},setsar=1"
            ));
            (width, height)
        }
        None => size,
    };
    if written.0 % 2 == 1 || written.1 % 2 == 1 {
        // 4:2:0 needs even sides.
        filters.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".to_owned());
    }
    filters.extend(encoding.convert.map(str::to_owned));
    if !filters.is_empty() {
        args.extend(["-vf".to_owned(), filters.join(",")]);
    }
    args.extend(["-pix_fmt".to_owned(), encoding.pix_fmt.to_owned()]);
    args.extend(encoding.output);
    args.extend([
        "-r".to_owned(),
        crate::program::rate_arg(rate),
        "-t".to_owned(),
        crate::program::secs(duration),
        opts.output.clone(),
    ]);
    Ok(CompositedRender {
        args,
        times,
        scale,
        size,
        deep,
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
            // Rendering again after a small edit draws only what it changed.
            frame_folder: kerf_compose::default_frame_folder(),
            ..Default::default()
        },
    )?;
    // HDR10 says how bright the picture is before its first frame, so the frames are measured first;
    // kept on disk, they are not drawn twice.
    let measured;
    let opts = if needs_measuring(project, sequence, opts) {
        let light = measure_light(&compositor, project, sequence, range, opts, cancellation)?;
        measured = RenderOptions {
            content_light: Some(light),
            ..opts.clone()
        };
        &measured
    } else {
        opts
    };
    let render = composited_render(project, sequence, range, opts, compositor.size())?;
    let mut child = crate::ffmpeg::hidden_command(resolve_ffmpeg_bin())
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
        |index| {
            if render.deep {
                compositor.frame16(render.times[index], render.scale)
            } else {
                compositor.frame(render.times[index], render.scale)
            }
        },
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
    compositor.flush_frames();
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

/// Whether a render still has to work out HDR10's content light levels.
fn needs_measuring(
    project: &kerf_model::Project,
    sequence: Option<&str>,
    opts: &RenderOptions,
) -> bool {
    opts.preset == RenderPreset::HevcHdr
        && opts.content_light.is_none()
        && kerf_core::program::program_sequence(project, sequence)
            .is_ok_and(|top| top.color == ColorSpace::Rec2100Pq)
}

/// How bright the frames a render would encode are.
fn measure_light(
    compositor: &Compositor,
    project: &kerf_model::Project,
    sequence: Option<&str>,
    range: Option<(Time, Time)>,
    opts: &RenderOptions,
    cancellation: &CancellationToken,
) -> Result<ContentLight> {
    let render = composited_render(project, sequence, range, opts, compositor.size())?;
    let mut meter = LightMeter::new();
    draw_in_order(
        render.times.len(),
        RENDER_DRAWERS,
        |index| compositor.frame16(render.times[index], render.scale),
        |frame| {
            meter.add(&frame.rgba);
            Ok(())
        },
        cancellation,
    )?;
    compositor.flush_frames();
    Ok(meter.levels())
}

/// Draw `count` frames on `threads` threads, handing them to `write` in
/// order. Drawing stays at most a few frames ahead of writing.
pub(crate) fn draw_in_order(
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
        // Said under the lock the drawing threads wait with: one that had just found it must wait
        // and was not yet waiting would miss the call, and wait for a frame nobody will write.
        {
            let _waiting = written.0.lock().unwrap_or_else(PoisonError::into_inner);
            stop.store(true, Ordering::SeqCst);
        }
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
    let mut command = crate::ffmpeg::hidden_command(resolve_ffmpeg_bin());
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
        let render = composited_render(
            &project(),
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
        let args = &render.args;
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-c:v" && pair[1] == "libx264"));
        assert!(args
            .iter()
            .any(|arg| arg == "veryfast" || arg == "faster" || arg == "medium"));
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
    fn the_sound_alone_reads_no_picture_and_writes_none() {
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
        let second = |seconds| Time::from_seconds(seconds).expect("time");
        let args = sound_args(
            &project,
            None,
            Some((second(2.0), second(5.0))),
            16_000,
            Path::new("sound.wav"),
        )
        .expect("args");
        // The clip's file is the only input, and its sound the only thing mapped.
        assert_eq!(
            args.iter().filter(|arg| *arg == "-i").count(),
            1,
            "{args:?}"
        );
        assert!(!args.iter().any(|arg| arg == "pipe:0"), "{args:?}");
        assert_eq!(
            args.iter().filter(|arg| *arg == "-map").count(),
            1,
            "{args:?}"
        );
        assert!(!args.iter().any(|arg| arg == "-c:v"), "{args:?}");
        assert_eq!(
            args[args.len() - 11..],
            [
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "-t",
                "3",
                "-f",
                "wav",
                "sound.wav"
            ]
        );
        // What cannot be mixed is still said before anything runs.
        let empty = kerf_lang::parse_document(
            "kerf 1\nproject name=\"render\"\nsequence s1 rate=24 size=160x120 audio=48000\n",
            kerf_model::Catalog::builtin(),
        )
        .expect("project")
        .project;
        assert!(sound_args(&empty, None, None, 16_000, Path::new("sound.wav")).is_err());
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
    fn a_guide_is_drawn_at_the_size_and_rate_asked_with_no_sound() {
        let guide = |size, rate| RenderOptions {
            output: "guide.mp4".into(),
            force_software: true,
            size,
            rate,
            sound: false,
            ..RenderOptions::default()
        };
        let sixteen = Rate::new(16, 1).expect("a rate");
        // A wider picture than the one asked for fills its height, and its sides are cut off.
        let wide = composited_render(
            &project(),
            None,
            None,
            &guide(Some((832, 480)), Some(sixteen)),
            (1920, 1080),
        )
        .expect("render");
        assert_eq!(wide.times.len(), 160, "ten seconds at 16 a second");
        assert_eq!(wide.times[1], sixteen.time_of(1));
        assert_eq!(wide.size, (853, 480));
        assert!((wide.scale - 480.0 / 1080.0).abs() < 1e-12);
        let args = &wide.args;
        assert_eq!(
            flag(args, "-vf"),
            Some("scale=832:480:force_original_aspect_ratio=increase:flags=bicubic,crop=832:480,setsar=1")
        );
        assert_eq!(flag(args, "-s"), Some("853x480"));
        assert_eq!(flag(args, "-framerate"), Some("16/1"));
        assert_eq!(flag(args, "-r"), Some("16/1"));
        assert_eq!(flag(args, "-t"), Some("10"));
        assert_eq!(flag(args, "-map"), Some("0:v"));
        // No sound is read, mixed, or written.
        assert!(args.iter().any(|arg| arg == "-an"));
        assert!(!args.iter().any(|arg| arg == "-filter_complex"));
        assert_eq!(args.iter().filter(|arg| *arg == "-i").count(), 1);
        assert_eq!(args.last().map(String::as_str), Some("guide.mp4"));

        // A taller one fills its width, and its top and bottom are cut off.
        let tall = composited_render(
            &project(),
            None,
            None,
            &guide(Some((832, 480)), None),
            (1080, 1920),
        )
        .expect("render");
        assert_eq!(tall.size, (832, 1479));
        assert_eq!(tall.times.len(), 240, "the sequence's own rate");
        // The same shape is drawn at the size itself, with nothing to cut.
        let same = composited_render(
            &project(),
            None,
            None,
            &guide(Some((1280, 720)), None),
            (1920, 1080),
        )
        .expect("render");
        assert_eq!(same.size, (1280, 720));
        assert_eq!(flag(&same.args, "-vf"), None);
        // A size larger than the sequence is the sequence's picture, scaled up.
        let large = composited_render(
            &project(),
            None,
            None,
            &guide(Some((1920, 1080)), None),
            (160, 120),
        )
        .expect("render");
        assert_eq!((large.size, large.scale), ((160, 120), 1.0));
        assert!(
            flag(&large.args, "-vf").is_some_and(|filter| filter.starts_with("scale=1920:1080:"))
        );
        // A rate alone keeps the size, and the file still has no sound.
        let slower = composited_render(
            &project(),
            None,
            None,
            &guide(None, Some(sixteen)),
            (160, 120),
        )
        .expect("render");
        assert_eq!((slower.size, slower.times.len()), ((160, 120), 160));
        assert_eq!(flag(&slower.args, "-vf"), None);
    }

    #[test]
    fn a_render_without_sound_still_says_what_cannot_be_rendered() {
        let silent = RenderOptions {
            output: "guide.mp4".into(),
            sound: false,
            ..RenderOptions::default()
        };
        let none = composited_render(&project(), Some("s9"), None, &silent, (160, 120))
            .expect_err("no such sequence");
        assert!(none.to_string().contains("no sequence `s9`"), "{none}");
        let empty = kerf_lang::parse_document(
            "kerf 1\nproject name=\"render\"\nsequence s1 rate=24 size=160x120 audio=48000\n\x20 track V1 video\n",
            kerf_model::Catalog::builtin(),
        )
        .expect("project")
        .project;
        let nothing =
            composited_render(&empty, None, None, &silent, (160, 120)).expect_err("nothing in it");
        assert!(
            nothing.to_string().contains("sequence `s1` is empty"),
            "{nothing}"
        );
    }

    /// The value of the last `name` flag: output options follow the inputs'.
    fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
        args.windows(2)
            .rfind(|pair| pair[0] == name)
            .map(|pair| pair[1].as_str())
    }

    #[test]
    fn the_master_is_prores_hq_in_quicktime_with_24_bit_pcm() {
        let opts = RenderOptions {
            output: "master.mov".into(),
            force_software: true,
            preset: RenderPreset::ProRes422Hq,
            ..RenderOptions::default()
        };
        let render = composited_render(&project(), None, None, &opts, (161, 120)).expect("render");
        let args = &render.args;
        assert_eq!(flag(args, "-c:v"), Some("prores_ks"));
        assert_eq!(flag(args, "-profile:v"), Some("3"));
        assert_eq!(flag(args, "-vendor"), Some("apl0"));
        assert_eq!(flag(args, "-pix_fmt"), Some("yuv422p10le"));
        assert_eq!(flag(args, "-c:a"), Some("pcm_s24le"));
        assert_eq!(flag(args, "-f"), Some("mov"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-f" && pair[1] == "rawvideo"));
        assert_eq!(flag(args, "-colorspace"), Some("bt709"));
        assert_eq!(flag(args, "-color_range"), Some("tv"));
        assert_eq!(
            flag(args, "-vf"),
            Some(format!("scale=trunc(iw/2)*2:trunc(ih/2)*2,{MASTER_CONVERT}").as_str())
        );
        assert!(!args.iter().any(|arg| arg == "yuv420p"));
        assert_eq!(args.last().map(String::as_str), Some("master.mov"));
    }

    #[test]
    fn the_review_copy_is_full_size_h264_at_12_mbps() {
        let opts = RenderOptions {
            output: "review.mp4".into(),
            force_software: true,
            preset: RenderPreset::Review,
            ..RenderOptions::default()
        };
        let render =
            composited_render(&project(), None, None, &opts, (1920, 1080)).expect("render");
        assert_eq!((render.size, render.scale), ((1920, 1080), 1.0));
        assert_eq!(render.times.len(), 240, "the sequence's own rate");
        let args = &render.args;
        assert_eq!(flag(args, "-c:v"), Some("libx264"));
        assert_eq!(flag(args, "-b:v"), Some("12M"));
        assert_eq!(flag(args, "-maxrate"), Some("12M"));
        assert_eq!(flag(args, "-pix_fmt"), Some("yuv420p"));
        assert!(!args.iter().any(|arg| arg == "-crf"));
    }

    #[test]
    fn a_meter_reads_nits_from_the_pq_signal() {
        let mut meter = LightMeter::new();
        let code = |nits: f32| {
            (kerf_color::pq_inverse_eotf(nits / 10_000.0) * 65_535.0).round() as u16
        };
        let pixel = |r: u16, g: u16, b: u16| {
            [r, g, b, u16::MAX]
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        // A frame half at 1000 nits in one channel and half black: its average is 500.
        let mut frame = pixel(code(1000.0), code(100.0), 0);
        frame.extend(pixel(0, 0, 0));
        meter.add(&frame);
        // Another, dimmer overall but with a brighter peak among many.
        let mut frame = pixel(0, code(4000.0), 0);
        for _ in 0..39 {
            frame.extend(pixel(0, 0, 0));
        }
        meter.add(&frame);
        let levels = meter.levels();
        assert!(levels.max_cll.abs_diff(4000) <= 3, "{levels:?}");
        assert!(levels.max_fall.abs_diff(500) <= 2, "{levels:?}");
    }

    #[test]
    fn mastering_display_is_written_as_x265_reads_it() {
        assert_eq!(
            MasteringDisplay::P3_1000.x265(),
            "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)"
        );
    }

    #[test]
    fn hevc_hdr_refuses_what_is_not_an_hdr_sequence() {
        if !encoder_available("libx265") {
            return;
        }
        let opts = RenderOptions {
            output: "out.mp4".into(),
            preset: RenderPreset::HevcHdr,
            ..RenderOptions::default()
        };
        let refused = composited_render(&project(), None, None, &opts, (160, 120))
            .expect_err("an SDR sequence");
        assert!(refused.to_string().contains("rec2100"), "{refused}");
    }

    #[test]
    fn standard_renders_are_unchanged_and_drafts_refuse_other_presets() {
        let standard = RenderOptions {
            output: "out.mp4".into(),
            force_software: true,
            ..RenderOptions::default()
        };
        let render =
            composited_render(&project(), None, None, &standard, (160, 120)).expect("render");
        assert!(!render.args.iter().any(|arg| arg == "-c:a" || arg == "-vf"));
        assert_eq!(flag(&render.args, "-pix_fmt"), Some("yuv420p"));

        let draft = RenderOptions {
            output: "out.mov".into(),
            draft: true,
            preset: RenderPreset::ProRes422Hq,
            ..RenderOptions::default()
        };
        assert!(composited_render(&project(), None, None, &draft, (160, 120)).is_err());
    }

    #[test]
    fn presets_have_names_extensions_and_serde_names() {
        for preset in RenderPreset::ALL {
            assert_eq!(RenderPreset::parse(preset.as_str()), Some(preset));
            assert_eq!(
                serde_json::to_value(preset).expect("serializes"),
                preset.as_str()
            );
        }
        assert_eq!(
            RenderPreset::parse("ProRes_HQ"),
            Some(RenderPreset::ProRes422Hq)
        );
        assert_eq!(RenderPreset::parse("dnxhd"), None);
        assert_eq!(RenderPreset::ProRes422Hq.extension(), "mov");
        assert_eq!(RenderPreset::Review.extension(), "mp4");
        assert_eq!(RenderPreset::default(), RenderPreset::Standard);
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

    #[test]
    fn drawing_that_is_stopped_leaves_no_thread_waiting() {
        // The frames cost nothing to draw, so the drawing threads are going to wait for the writing
        // as it stops at its first frame: what a render that is cancelled does to them.
        let (finished, all) = mpsc::channel();
        thread::spawn(move || {
            for _ in 0..20_000 {
                let stopped = draw_in_order(
                    64,
                    RENDER_DRAWERS,
                    |_| Frame {
                        width: 1,
                        height: 1,
                        rgba: vec![0; 4],
                    },
                    |_| Err(EngineError::msg("stopped")),
                    &CancellationToken::new(),
                );
                assert!(stopped.is_err());
            }
            let _ = finished.send(());
        });
        assert!(
            all.recv_timeout(Duration::from_secs(120)).is_ok(),
            "a drawing thread went on waiting for a frame nobody will write"
        );
    }
}
