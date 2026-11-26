//! Render argument construction and ffmpeg spawn for `kerf-model` projects.
//!
//! A render draws the picture with the compositor (`kerf-compose`) and
//! pipes its frames to FFmpeg, which encodes them with the program's sound
//! as the graph mixes it. Effects draw as part of each frame, so nothing is
//! baked first.
//!
//! The modules beside it are what a render is made with. Those whose names do not say it:
//!
//! - [`sections`]: the picture, encoded a stretch at a time and kept by what each stretch's
//!   frames depend on, so that a render after an edit encodes only what the edit touched.
//! - `kept_sound`: the sound, kept the same way: each clip's decoded, and the encoded sound in
//!   pieces.
//! - [`ramps`]: the sound of clips that change speed, made ready for the program's graph to mix.
//!
//! [`encoder`] chooses how the picture is encoded, and [`still`] draws one frame as a render
//! draws it and writes it as an image.

pub mod encoder;
mod kept_sound;
pub mod ramps;
pub mod sections;
pub mod still;

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

use crate::error::{EngineError, Result};
use crate::process::ffmpeg::{
    cached_encoder_listing, parse_time_seconds, resolve_ffmpeg_bin, run_allow_failure, tail,
};
use crate::process::resources::{budget, should_try_hardware_encode};
use crate::process::sidecar::{self, CancellationToken};
use crate::program::{compile_program, ProgramOptions};
use crate::render::encoder::{
    encode_output_args, keep_encoders_that_open, parse_encoder_capabilities, select_video_encoder,
    EncoderSelection, HardwareEncoderFamily, SelectEncoderOptions, VideoCodec,
};
use crate::render::sections::Kept;

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
    /// What of the sequence is drawn: its picture, or a pass of it for a model that reads one.
    pub pass: kerf_compose::Pass,
    /// A clip of the sequence to draw alone, white on black, in place of the picture.
    pub matte: Option<String>,
    /// Where the frames drawn are kept, and read back from when they are drawn again.
    pub frame_cache: CacheFolder,
    /// Where the stretches encoded are kept, and joined again where nothing in them changed.
    pub section_cache: CacheFolder,
}

/// Where a render keeps what it can use again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CacheFolder {
    /// The user's cache folder, or the one the environment names or turns off
    /// (`KERF_FRAME_CACHE` for frames, `KERF_SECTION_CACHE` for stretches).
    #[default]
    Default,
    /// None: nothing is kept, and nothing read back.
    Off,
    /// This folder.
    At(PathBuf),
}

impl CacheFolder {
    /// The folder, with `default` the one [`CacheFolder::Default`] is.
    pub fn folder(&self, default: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
        match self {
            Self::Default => default(),
            Self::Off => None,
            Self::At(folder) => Some(folder.clone()),
        }
    }
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
        primaries: [
            [0.265, 0.690],
            [0.150, 0.060],
            [0.680, 0.320],
            [0.3127, 0.3290],
        ],
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
            pass: kerf_compose::Pass::Picture,
            matte: None,
            frame_cache: CacheFolder::Default,
            section_cache: CacheFolder::Default,
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
    /// What the drawn frames go to FFmpeg as.
    frames: FramesIn,
    /// A pass: no pixel of it is made of its neighbors when it is brought to size.
    pass: bool,
}

/// What drawn frames go to FFmpeg as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FramesIn {
    /// RGBA, 8 bits a component.
    Rgba,
    /// RGBA, 16 bits a component: a deeper file is worth deeper frames, as 8 bits would only be
    /// stretched to fill it.
    Rgba16,
    /// One grey of 16 bits a pixel, the first component of a 16-bit frame: a pass of how near
    /// things are, whose three components are the same.
    Grey16,
    /// [`Self::Grey16`] rounded to 10 bits, for a file that holds no more, so that FFmpeg has
    /// nothing to bring down, which it would dither.
    Grey10,
}

impl FramesIn {
    fn pix_fmt(self) -> &'static str {
        match self {
            Self::Rgba => "rgba",
            Self::Rgba16 => "rgba64le",
            Self::Grey16 => "gray16le",
            Self::Grey10 => "gray10le",
        }
    }

    /// Whether they are drawn at 16 bits a component.
    fn deep(self) -> bool {
        self != Self::Rgba
    }
}

/// How a pass is written to a file: with no loss, each pixel as it was drawn, in the container
/// the file's extension names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PassCodec {
    /// The video codec's arguments, but for its pixel format.
    pub video: Vec<String>,
    pub pix_fmt: &'static str,
    /// The container, as FFmpeg's `-f` names it.
    pub format: &'static str,
}

/// Whether a render of `pass`, or of a matte, is of greys: how near each pixel is.
pub(crate) fn pass_of_greys(pass: kerf_compose::Pass, matte: bool) -> bool {
    !matte
        && matches!(
            pass,
            kerf_compose::Pass::Depth | kerf_compose::Pass::Relative
        )
}

/// How a pass is written to `file`, of greys or of colors ([`pass_of_greys`]). No container
/// holds every kind of pass at every depth, so the extension decides:
///
/// - `.mp4`, `.mov`, `.m4v`: H.264 with no loss (`-qp 0`), colors as RGB at 8 bits a channel,
///   with none shared between pixels as 4:2:0 shares them, and greys at 10 bits, full range.
/// - `.mkv`: FFV1, colors as RGB at 8 bits a channel and greys at 16.
pub(crate) fn pass_codec(file: &Path, greys: bool) -> Result<PassCodec> {
    let extension = file
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let video = |args: &[&str]| -> Vec<String> {
        // Greys are of the whole range, as a depth pass is drawn.
        let range = if greys {
            &["-color_range", "pc"][..]
        } else {
            &[]
        };
        args.iter()
            .chain(range)
            .map(|arg| (*arg).to_owned())
            .collect()
    };
    match extension.as_str() {
        "mp4" | "mov" | "m4v" => Ok(PassCodec {
            video: video(&[
                "-c:v",
                if greys { "libx264" } else { "libx264rgb" },
                "-qp",
                "0",
                "-preset",
                budget().software_preset,
            ]),
            pix_fmt: if greys { "gray10le" } else { "rgb24" },
            format: if extension == "mov" { "mov" } else { "mp4" },
        }),
        "mkv" => Ok(PassCodec {
            video: video(&["-c:v", "ffv1", "-level", "3"]),
            pix_fmt: if greys { "gray16le" } else { "gbrp" },
            format: "matroska",
        }),
        _ => Err(EngineError::msg(format!(
            "a pass is written with no loss to an .mp4, .mov, .m4v or .mkv file, not to `{}`",
            file.file_name().unwrap_or_default().to_string_lossy()
        ))),
    }
}

/// A pass, or a matte, written with no loss ([`pass_codec`]), with no sound.
fn pass_encoding(opts: &RenderOptions) -> Result<Encoding> {
    if opts.draft || opts.preset != RenderPreset::Standard {
        return Err(EngineError::msg(format!(
            "a pass is written with no loss; it cannot be a {} render",
            if opts.draft {
                "draft"
            } else {
                opts.preset.label()
            }
        )));
    }
    let greys = pass_of_greys(opts.pass, opts.matte.is_some());
    let codec = pass_codec(Path::new(&opts.output), greys)?;
    let frames = match codec.pix_fmt {
        "gray16le" => FramesIn::Grey16,
        "gray10le" => FramesIn::Grey10,
        _ => FramesIn::Rgba,
    };
    Ok(Encoding {
        video: codec.video,
        pix_fmt: codec.pix_fmt,
        convert: None,
        output: Vec::new(),
        frames,
        pass: true,
    })
}

fn encoding(opts: &RenderOptions, color: ColorSpace) -> Result<Encoding> {
    if opts.pass != kerf_compose::Pass::Picture || opts.matte.is_some() {
        return pass_encoding(opts);
    }
    let plain = |video| Encoding {
        video,
        pix_fmt: "yuv420p",
        convert: None,
        output: Vec::new(),
        frames: FramesIn::Rgba,
        pass: false,
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
                frames: FramesIn::Rgba16,
                pass: false,
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
        frames: FramesIn::Rgba16,
        pass: false,
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
    let mut args = crate::render::encoder::encoder_args(&selection);
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
    let heard = crate::render::ramps::with_sound(project);
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
    let mut command = crate::process::ffmpeg::hidden_command(resolve_ffmpeg_bin());
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
    /// What frames go to FFmpeg as.
    frames: FramesIn,
    /// How the picture is encoded a stretch at a time and joined, where it can be.
    sections: Option<SectionArgs>,
    /// How many frames a second it draws.
    rate: Rate,
}

/// FFmpeg's arguments for a render whose picture is encoded a stretch at a time
/// ([`crate::render::sections`]).
#[derive(Debug)]
struct SectionArgs {
    /// To encode a stretch's frames from stdin to a file with no sound; the file follows.
    encode: Vec<String>,
    /// To mix the sound and encode it to a file with no picture; the file follows. None for a
    /// render with no sound.
    sound: Option<Vec<String>>,
    /// The same sound as what it is mixed from and how it is encoded, to make from what is
    /// kept of it ([`crate::render::kept_sound`]).
    mix: Option<crate::render::kept_sound::Mix>,
    /// To copy the stretches and the sound into the render's file: what follows the inputs,
    /// which are the sound's file, when there is one, and then the list of stretches.
    join: Vec<String>,
    /// What kind of file a stretch is, and the sound: the render's own.
    extension: String,
}

/// Arguments that say how a picture is encoded, each with its value, which a picture copied
/// as it is has no use for.
const ENCODER_ONLY: [&str; 5] = [
    "-vendor",
    "-colorspace",
    "-color_primaries",
    "-color_trc",
    "-color_range",
];

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
    let mut mixed = None;
    let (mut args, duration, own_rate) = if opts.sound {
        let heard = crate::render::ramps::with_sound(project);
        let sound = compile_program(
            &heard,
            kerf_model::Catalog::builtin(),
            &ProgramOptions {
                sequence: sequence.map(str::to_owned),
                range,
            },
        )?;
        let args: Vec<String> = sound.argv.iter().skip(1).cloned().collect();
        let (duration, rate) = (sound.duration, sound.rate);
        mixed = Some(sound);
        (args, duration, rate)
    } else {
        let (duration, rate) = picture_span(project, sequence, range)?;
        (vec!["-y".to_owned()], duration, rate)
    };
    let from = range.map_or(Time::ZERO, |(from, _)| from);
    let rate = match opts.rate {
        Some(rate) => rate,
        None if opts.draft => Rate::new(15, 1).expect("15 fps is a whole number of flicks a frame"),
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
    let frames = encoding.frames;
    let picture = args.iter().filter(|arg| *arg == "-i").count();
    let at = args
        .iter()
        .position(|arg| arg == "-filter_complex")
        .unwrap_or(args.len());
    let frames_in = [
        "-f".to_owned(),
        "rawvideo".to_owned(),
        "-pix_fmt".to_owned(),
        frames.pix_fmt().to_owned(),
        "-s".to_owned(),
        format!("{}x{}", size.0, size.1),
        "-framerate".to_owned(),
        crate::program::rate_arg(rate),
        "-i".to_owned(),
        "pipe:0".to_owned(),
    ];
    // In stretches the sound is mixed and encoded alone, as it would be with the picture, and
    // the two are copied into the file. Of what follows the codec, the sound's file takes what
    // is the sound's and the file's, and the copy what is the file's and the picture's own.
    let (mut codec, mut wrap, mut copied) = (Vec::new(), Vec::new(), Vec::new());
    let mut output = encoding.output.iter();
    while let Some(arg) = output.next() {
        let value = output.next().cloned().unwrap_or_default();
        match arg.as_str() {
            arg if ENCODER_ONLY.contains(&arg) => {}
            "-c:a" | "-b:a" => codec.extend([arg.clone(), value]),
            "-bsf:v" => copied.extend([arg.clone(), value]),
            _ => {
                wrap.extend([arg.clone(), value.clone()]);
                copied.extend([arg.clone(), value]);
            }
        }
    }
    let lasts = ["-t".to_owned(), crate::program::secs(duration)];
    let sound = opts.sound.then(|| {
        let mut sound = args.clone();
        sound.push("-vn".to_owned());
        sound.extend(codec.iter().cloned());
        sound.extend(wrap.iter().cloned());
        sound.extend(lasts.iter().cloned());
        sound
    });
    let mix = mixed.map(|sound| crate::render::kept_sound::Mix {
        graph: vec![
            "-filter_complex".to_owned(),
            sound.decoded_filter,
            "-map".to_owned(),
            format!("[{}]", sound.mixed),
        ],
        inputs: sound.inputs,
        codec,
        wrap,
        duration,
    });
    let mut join: Vec<String> = match sound {
        Some(_) => ["-map", "1:v", "-map", "0:a"].map(String::from).into(),
        None => ["-map", "0:v", "-an"].map(String::from).into(),
    };
    join.extend(["-c".to_owned(), "copy".to_owned()]);
    join.extend(copied);
    join.extend(lasts);
    join.push(opts.output.clone());
    let mut encode = vec!["-y".to_owned(), "-hide_banner".to_owned()];
    encode.extend(frames_in.iter().cloned());
    encode.push("-an".to_owned());
    encode.extend(encoding.video.iter().cloned());
    args.splice(at..at, frames_in);
    args.extend(["-map".to_owned(), format!("{picture}:v")]);
    if !opts.sound {
        args.push("-an".to_owned());
    }
    args.extend(encoding.video);
    let mut filters = Vec::new();
    // A pass is numbers: a pixel brought to size is the one nearest, not a mix of its
    // neighbors, which would be a color that is no clip's, or a distance nothing is at.
    let (resample, even) = if encoding.pass {
        ("neighbor", ":flags=neighbor")
    } else {
        ("bicubic", "")
    };
    // What was drawn fills the size asked for; what is over is cut off evenly.
    let written = match opts.size.filter(|wanted| *wanted != size) {
        Some((width, height)) => {
            filters.push(format!(
                "scale={width}:{height}:force_original_aspect_ratio=increase:flags={resample},crop={width}:{height},setsar=1"
            ));
            (width, height)
        }
        None => size,
    };
    if written.0 % 2 == 1 || written.1 % 2 == 1 {
        // 4:2:0 needs even sides.
        filters.push(format!("scale=trunc(iw/2)*2:trunc(ih/2)*2{even}"));
    }
    filters.extend(encoding.convert.map(str::to_owned));
    if !filters.is_empty() {
        let filters = ["-vf".to_owned(), filters.join(",")];
        encode.extend(filters.iter().cloned());
        args.extend(filters);
    }
    let encoded = ["-pix_fmt".to_owned(), encoding.pix_fmt.to_owned()];
    encode.extend(encoded.iter().cloned());
    encode.extend(encoding.output.iter().cloned());
    encode.extend(["-r".to_owned(), crate::program::rate_arg(rate)]);
    args.extend(encoded);
    args.extend(encoding.output);
    args.extend([
        "-r".to_owned(),
        crate::program::rate_arg(rate),
        "-t".to_owned(),
        crate::program::secs(duration),
        opts.output.clone(),
    ]);
    // HDR10 says how bright the whole picture is in every stretch of it, so a stretch would be
    // another file whenever any other changed. Matroska times frames in thousandths of a second,
    // so stretches at a rate whose frames are not a whole number of them, such as 16 or 24 a
    // second, join to a file whose frames are not evenly timed, and are encoded whole after all.
    let extension = Path::new(&opts.output)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("mp4")
        .to_owned();
    let timed = !extension.eq_ignore_ascii_case("mkv")
        || (1000 * u64::from(rate.den())).is_multiple_of(u64::from(rate.num()));
    let joins = opts.preset != RenderPreset::HevcHdr && timed;
    let sections = joins.then_some(SectionArgs {
        encode,
        sound,
        mix,
        join,
        extension,
    });
    Ok(CompositedRender {
        args,
        times,
        scale,
        size,
        frames,
        sections,
        rate,
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
            frame_folder: opts.frame_cache.folder(kerf_compose::default_frame_folder),
            pass: opts.pass,
            matte: opts.matte.clone(),
            // A file of the pass is read a pixel at a time, each taken for one clip's.
            hard_edges: true,
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
    let total = render.times.len();
    if let Some((sections, kept)) = render
        .sections
        .as_ref()
        .zip(Kept::open(&opts.section_cache))
    {
        let mut progress = |frames: usize| {
            if let Some(callback) = on_progress.as_mut() {
                callback(frames as f64 / total as f64);
            }
        };
        match render_in_sections(
            &compositor,
            &render,
            sections,
            &kept,
            cancellation,
            &mut progress,
        ) {
            Ok(true) => return Ok(()),
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            // Not worth doing in stretches, or it went wrong: the picture is encoded whole, which
            // says what is wrong if anything is.
            Ok(false) => {}
            Err(_) => crate::render::sections::count_whole(),
        }
    }
    let mut written = 0;
    let drawn = encode_frames(
        &compositor,
        &render,
        0..total,
        &render.args,
        || {
            written += 1;
            if let Some(callback) = on_progress.as_mut() {
                callback(written as f64 / total as f64);
            }
        },
        cancellation,
    );
    compositor.flush_frames();
    drawn
}

/// Draw `frames` of `render` and hand them to an FFmpeg run with `args`, calling `drawn` as each
/// goes.
fn encode_frames(
    compositor: &Compositor,
    render: &CompositedRender,
    frames: std::ops::Range<usize>,
    args: &[String],
    mut drawn: impl FnMut(),
    cancellation: &CancellationToken,
) -> Result<()> {
    // A sidecar: it is ended with the render whichever way that ends, a panic while drawing too.
    let mut ffmpeg = sidecar::Sidecar::start(
        crate::process::ffmpeg::hidden_command(resolve_ffmpeg_bin())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )
    .map_err(|error| EngineError::msg(format!("ffmpeg could not start: {error}")))?;
    let mut stdin = ffmpeg.stdin.take().expect("stdin is piped");
    let mut stderr = ffmpeg.stderr.take().expect("stderr is piped");
    let reader = thread::spawn(move || {
        let mut text = String::new();
        if let Err(error) = stderr.read_to_string(&mut text) {
            log::warn!("what FFmpeg said while it encoded could not be read: {error}");
        }
        text
    });
    let first = frames.start;
    let result = draw_in_order(
        frames.len(),
        RENDER_DRAWERS,
        |index| {
            if render.frames.deep() {
                compositor.frame16(render.times[first + index], render.scale)
            } else {
                compositor.frame(render.times[first + index], render.scale)
            }
        },
        |frame| {
            if (frame.width, frame.height) != render.size {
                return Err(EngineError::msg(format!(
                    "a frame came out {}x{}, not {}x{}",
                    frame.width, frame.height, render.size.0, render.size.1
                )));
            }
            match render.frames {
                FramesIn::Rgba | FramesIn::Rgba16 => stdin.write_all(&frame.rgba)?,
                FramesIn::Grey16 | FramesIn::Grey10 => {
                    stdin.write_all(&greys(&frame.rgba, render.frames == FramesIn::Grey10))?;
                }
            }
            drawn();
            Ok(())
        },
        cancellation,
    );
    drop(stdin);
    if result.is_err() {
        // Killed at once, as what it has written is not wanted: nothing of it is waited for.
        // It may have ended already; the wait below says how it ended either way.
        ffmpeg.kill();
    }
    let status = ffmpeg.wait()?;
    let stderr = reader.join().unwrap_or_default();
    match result {
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

/// The first component of each pixel of 16-bit RGBA, as 16-bit greys, or rounded to 10 bits.
fn greys(rgba: &[u8], ten: bool) -> Vec<u8> {
    rgba.as_chunks::<8>()
        .0
        .iter()
        .flat_map(|pixel| {
            let grey = u16::from_le_bytes([pixel[0], pixel[1]]);
            let grey = if ten {
                ((u32::from(grey) * 1023 + 32_767) / 65_535) as u16
            } else {
                grey
            };
            grey.to_le_bytes()
        })
        .collect()
}

/// Render with the picture encoded a stretch at a time ([`crate::render::sections`]): the stretches
/// `kept` has are joined as they are, and the rest are drawn, encoded, and kept. `progress` is
/// told how many frames are done. `Ok(false)` when the render is not one for stretches, with
/// nothing written.
fn render_in_sections(
    compositor: &Compositor,
    render: &CompositedRender,
    sections: &SectionArgs,
    kept: &Kept,
    cancellation: &CancellationToken,
    progress: &mut dyn FnMut(usize),
) -> Result<bool> {
    let total = render.times.len();
    let keys: Vec<Option<u64>> = render
        .times
        .iter()
        .map(|at| compositor.frame_key(*at, render.scale))
        .collect();
    // A render shorter than a stretch is not worth another FFmpeg to join it, and where most
    // frames have no number there is nothing to keep.
    if total < crate::render::sections::least_frames(render.rate.fps())
        || keys.iter().flatten().count() * 2 < total
    {
        return Ok(false);
    }
    // Everything a stretch's file depends on but its frames: the FFmpeg and how it encodes.
    let mut encoded = format!("{}\n{}", resolve_ffmpeg_bin(), ffmpeg_version());
    for arg in &sections.encode {
        encoded.push('\n');
        encoded.push_str(arg);
    }
    let mut files = Vec::new();
    let mut scratch = Vec::new();
    let mut done = 0;
    // Each stretch, the name it is kept by if it has one, and its file if it is kept.
    let stretches: Vec<_> = crate::render::sections::stretches(&keys, render.rate.fps())
        .into_iter()
        .map(|stretch| {
            let name = stretch.keyed.then(|| {
                crate::render::sections::name(encoded.as_bytes(), &keys[stretch.frames.clone()])
            });
            let file = name
                .as_deref()
                .and_then(|name| kept.find(name, &sections.extension));
            (stretch.frames, name, file)
        })
        .collect();
    // With an eighth of the picture to draw and encode, nothing waits for the sound.
    let drawn: usize = stretches
        .iter()
        .filter(|(_, _, file)| file.is_none())
        .map(|(frames, ..)| frames.len())
        .sum();
    let busy = drawn * 8 > total;
    // The sound is the same file for as long as what is mixed and how it is encoded are the
    // same: a clip drawn another way leaves it as it was.
    let sound_name = sections.sound.as_deref().and_then(sound_name);
    let sound_kept = sound_name
        .as_deref()
        .and_then(|name| kept.find(name, &sections.extension));
    // The sound's file once it is made, to put out if the render then fails.
    let sound_made = Mutex::new(None);
    let sound_file = &sound_made;
    let made = thread::scope(|scope| {
        // Mixed and encoded while the picture is, since it waits for nothing of it.
        let mixing = match (&sections.sound, &sound_kept) {
            (Some(args), None) => Some(scope.spawn(move || {
                // From what is kept of it; and if that cannot be, as it was before any was.
                let from_kept = match &sections.mix {
                    Some(mix) => crate::render::kept_sound::make(
                        mix,
                        kept,
                        &sections.extension,
                        busy,
                        cancellation,
                    ),
                    None => Err(EngineError::msg("a sound with nothing to mix")),
                };
                match from_kept {
                    Err(EngineError::Cancelled) => Err(EngineError::Cancelled),
                    Err(_) => {
                        let file = kept.making(&sections.extension);
                        let mut command =
                            crate::process::ffmpeg::hidden_command(resolve_ffmpeg_bin());
                        command.args(args).arg(&file);
                        let mixed =
                            sidecar::run(command, cancellation, |_| {}).and_then(finished_well);
                        if mixed.is_err() {
                            // There may be none to take away; nothing reads one that stays.
                            let _ = std::fs::remove_file(&file);
                        }
                        mixed.map(|()| crate::render::kept_sound::Made {
                            file,
                            used: Vec::new(),
                        })
                    }
                    made => made,
                }
                .inspect(|made| {
                    *sound_file.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(made.file.clone());
                })
            })),
            _ => None,
        };
        for (frames, name, file) in stretches {
            if cancellation.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            if let Some(file) = file {
                crate::render::sections::count_kept(frames.len());
                done += frames.len();
                progress(done);
                files.push((file, frames.len()));
                continue;
            }
            let making = kept.making(&sections.extension);
            scratch.push(making.clone());
            let mut args = sections.encode.clone();
            args.push(making.to_string_lossy().into_owned());
            encode_frames(
                compositor,
                render,
                frames.clone(),
                &args,
                || {
                    done += 1;
                    progress(done);
                },
                cancellation,
            )?;
            crate::render::sections::count_encoded(frames.len());
            // One with no name is joined from where it was made, and goes when the render ends.
            let file = match &name {
                Some(name) => kept.keep(&making, name, &sections.extension)?,
                None => making,
            };
            files.push((file, frames.len()));
        }
        let sound = match (mixing, sound_kept.clone()) {
            (Some(mixing), _) => {
                let made = mixing
                    .join()
                    .map_err(|_| EngineError::msg("mixing the sound stopped"))??;
                crate::render::sections::count_sound(false);
                files.extend(made.used.into_iter().map(|file| (file, 0)));
                Some(match &sound_name {
                    Some(name) => kept.keep(&made.file, name, &sections.extension)?,
                    None => {
                        scratch.push(made.file.clone());
                        made.file
                    }
                })
            }
            (None, Some(kept)) => {
                crate::render::sections::count_sound(true);
                Some(kept)
            }
            (None, None) => None,
        };
        // The list FFmpeg joins: each stretch and exactly how long it plays.
        let mut list = String::from("ffconcat version 1.0\n");
        for (file, frames) in &files {
            let seconds = render.rate.time_of(*frames as i64).seconds();
            list.push_str(&format!(
                "file '{}'\nduration {seconds:.9}\n",
                file.to_string_lossy().replace('\'', "'\\''")
            ));
        }
        let listed = kept.making("ffconcat");
        scratch.push(listed.clone());
        std::fs::write(&listed, list)?;
        let mut command = crate::process::ffmpeg::hidden_command(resolve_ffmpeg_bin());
        command.args(["-y", "-hide_banner"]);
        if let Some(sound) = &sound {
            command.arg("-i").arg(sound);
        }
        command
            .args(["-f", "concat", "-safe", "0", "-i"])
            .arg(&listed)
            .args(&sections.join);
        finished_well(sidecar::run(command, cancellation, |_| {})?)?;
        files.extend(sound.map(|sound| (sound, 0)));
        // A join that lost or doubled a frame is no render: the picture is encoded whole.
        let output = sections
            .join
            .last()
            .expect("the section arguments push the render's file last onto `join`");
        match picture_frames(output) {
            Some(frames) if frames == total => Ok(()),
            found => Err(EngineError::msg(format!(
                "the stretches joined to {found:?} frames, not {total}"
            ))),
        }
    });
    compositor.flush_frames();
    if made.is_err() {
        scratch.extend(
            sound_made
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner),
        );
    }
    for file in scratch {
        // Scratch that is not there, or stays, changes nothing the render made.
        let _ = std::fs::remove_file(file);
    }
    let joined: Vec<PathBuf> = files.into_iter().map(|(file, _)| file).collect();
    kept.tidy(&joined);
    made.map(|()| true)
}

/// An FFmpeg that ended, as an error unless it ended well.
pub(crate) fn finished_well(finished: sidecar::Finished) -> Result<()> {
    if finished.status.success() {
        return Ok(());
    }
    Err(EngineError::Ffmpeg {
        code: finished.status.code().unwrap_or(-1),
        stderr: tail(finished.stderr.trim(), 2000).to_owned(),
    })
}

/// The name a render's sound is kept by: the FFmpeg, the arguments that mix and encode it, and
/// each file they read as it is now. None when one of those files cannot be read.
fn sound_name(args: &[String]) -> Option<String> {
    let mut said = format!("sound\n{}\n{}", resolve_ffmpeg_bin(), ffmpeg_version());
    for (at, arg) in args.iter().enumerate() {
        said.push('\n');
        said.push_str(arg);
        if at > 0 && args[at - 1] == "-i" {
            let file = std::fs::metadata(arg).ok()?;
            let changed = file
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?;
            said.push_str(&format!(" {} {}", file.len(), changed.as_nanos()));
        }
    }
    Some(crate::render::sections::name(said.as_bytes(), &[]))
}

/// How many frames the picture of the file at `path` holds, counted from its packets; none
/// unless each is decoded one frame after the one before it. Stretches whose encoders timed
/// them differently join into a file whose frames are shown when they should be and decoded at
/// odd times, which FFmpeg makes up for with frames of its own when it reads the file again.
fn picture_frames(path: &str) -> Option<usize> {
    let output = run_allow_failure(
        crate::process::ffmpeg::resolve_ffprobe_bin(),
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=dts",
            "-of",
            "csv=p=0",
            path,
        ]
        .map(String::from),
    )
    .ok()?;
    let decoded: Vec<i64> = output
        .stdout
        .lines()
        .map(|line| line.trim().trim_end_matches(',').parse())
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    let step = decoded.get(1)? - decoded[0];
    decoded
        .windows(2)
        .all(|pair| pair[1] - pair[0] == step)
        .then_some(decoded.len())
}

/// What FFmpeg says it is, once: a stretch another FFmpeg encoded is not joined to this one's.
pub(crate) fn ffmpeg_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        run_allow_failure(resolve_ffmpeg_bin(), &["-version".to_owned()])
            .map(|output| output.stdout.lines().next().unwrap_or_default().to_owned())
            .unwrap_or_default()
    })
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
    let mut command = crate::process::ffmpeg::hidden_command(resolve_ffmpeg_bin());
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
        // Progress is only told: a closed stderr must not stop the render.
        let _ = writeln!(std::io::stderr(), r#"{{"progress":{rounded}}}"#);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
