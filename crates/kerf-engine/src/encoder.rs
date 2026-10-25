//! Hardware encoder capability discovery and explicit selection.
//!
//! Defines the native encoder selection policy. Hardware is never selected
//! unless the caller opts in with `prefer_hardware`.

use serde::{Deserialize, Serialize};

pub const SOFTWARE_H264: &str = "libx264";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HardwareEncoderFamily {
    Videotoolbox,
    Nvenc,
    Qsv,
    Amf,
}

impl HardwareEncoderFamily {
    pub const ALL: [Self; 4] = [Self::Videotoolbox, Self::Nvenc, Self::Qsv, Self::Amf];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Videotoolbox => "videotoolbox",
            Self::Nvenc => "nvenc",
            Self::Qsv => "qsv",
            Self::Amf => "amf",
        }
    }

    pub fn encoder_name(self, codec: VideoCodec) -> &'static str {
        match (self, codec) {
            (Self::Videotoolbox, VideoCodec::H264) => "h264_videotoolbox",
            (Self::Videotoolbox, VideoCodec::Hevc) => "hevc_videotoolbox",
            (Self::Nvenc, VideoCodec::H264) => "h264_nvenc",
            (Self::Nvenc, VideoCodec::Hevc) => "hevc_nvenc",
            (Self::Qsv, VideoCodec::H264) => "h264_qsv",
            (Self::Qsv, VideoCodec::Hevc) => "hevc_qsv",
            (Self::Amf, VideoCodec::H264) => "h264_amf",
            (Self::Amf, VideoCodec::Hevc) => "hevc_amf",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    Hevc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderCapability {
    pub family: HardwareEncoderFamily,
    pub encoder: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderCapabilities {
    #[serde(rename = "ffmpegBin")]
    pub ffmpeg_bin: String,
    pub encoders: Vec<EncoderCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderSelection {
    pub encoder: String,
    pub hardware: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<HardwareEncoderFamily>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct SelectEncoderOptions {
    pub prefer_hardware: bool,
    pub preference: Vec<HardwareEncoderFamily>,
    pub codec: VideoCodec,
    pub software_encoder: String,
}

impl Default for SelectEncoderOptions {
    fn default() -> Self {
        Self {
            prefer_hardware: false,
            preference: HardwareEncoderFamily::ALL.to_vec(),
            codec: VideoCodec::H264,
            software_encoder: SOFTWARE_H264.to_string(),
        }
    }
}

/// Parse a captured `ffmpeg -encoders` listing without running a process.
pub fn parse_encoder_capabilities(output: &str, ffmpeg_bin: &str) -> EncoderCapabilities {
    let mut encoders = Vec::with_capacity(HardwareEncoderFamily::ALL.len() * 2);
    for family in HardwareEncoderFamily::ALL {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let encoder = family.encoder_name(codec);
            let available = encoder_listed(output, encoder);
            encoders.push(EncoderCapability {
                family,
                encoder: encoder.to_string(),
                available,
                reason: (!available).then(|| "Encoder was not listed by ffmpeg".into()),
            });
        }
    }
    EncoderCapabilities {
        ffmpeg_bin: ffmpeg_bin.to_string(),
        encoders,
        raw: Some(output.to_string()),
    }
}

fn encoder_listed(output: &str, name: &str) -> bool {
    output
        .lines()
        .any(|line| line.split_whitespace().any(|token| token == name))
}

pub fn select_video_encoder(
    capabilities: &[EncoderCapability],
    options: &SelectEncoderOptions,
) -> EncoderSelection {
    let software = options.software_encoder.as_str();
    if !options.prefer_hardware {
        return EncoderSelection {
            encoder: software.to_string(),
            hardware: false,
            family: None,
            reason: "Hardware encoding is opt-in".into(),
        };
    }
    for family in &options.preference {
        let encoder = family.encoder_name(options.codec);
        if capabilities
            .iter()
            .any(|entry| entry.family == *family && entry.encoder == encoder && entry.available)
        {
            return EncoderSelection {
                encoder: encoder.to_string(),
                hardware: true,
                family: Some(*family),
                reason: format!("{} encoder selected", family.as_str()),
            };
        }
    }
    EncoderSelection {
        encoder: software.to_string(),
        hardware: false,
        family: None,
        reason: "No requested hardware encoder is available; software fallback selected".into(),
    }
}

/// Mark the listed hardware encoders that do not open as unavailable, in
/// the order `options` prefers them, stopping at the first that opens:
/// FFmpeg builds list NVENC, QSV, and AMF whether or not the machine has
/// the hardware and its driver. `opens` tries one family's encoder.
pub fn keep_encoders_that_open(
    capabilities: &mut [EncoderCapability],
    options: &SelectEncoderOptions,
    mut opens: impl FnMut(HardwareEncoderFamily) -> bool,
) {
    if !options.prefer_hardware {
        return;
    }
    for family in &options.preference {
        let encoder = family.encoder_name(options.codec);
        let Some(capability) = capabilities
            .iter_mut()
            .find(|entry| entry.family == *family && entry.encoder == encoder && entry.available)
        else {
            continue;
        };
        if opens(*family) {
            return;
        }
        capability.available = false;
        capability.reason = Some("Encoder is listed but does not open on this machine".into());
    }
}

pub fn encoder_args(selection: &EncoderSelection) -> Vec<String> {
    vec!["-c:v".into(), selection.encoder.clone()]
}

/// Full encode argv: codec plus laptop-friendly quality flags.
pub fn encode_output_args(
    selection: &EncoderSelection,
    software_crf: u8,
    software_preset: &str,
) -> Vec<String> {
    let mut args = encoder_args(selection);
    if selection.hardware {
        match selection.family {
            Some(HardwareEncoderFamily::Videotoolbox) => {
                args.extend(["-q:v".into(), "65".into()]);
            }
            Some(HardwareEncoderFamily::Nvenc) => {
                args.extend([
                    "-rc".into(),
                    "constqp".into(),
                    "-qp".into(),
                    software_crf.to_string(),
                    "-preset".into(),
                    "p4".into(),
                ]);
            }
            Some(HardwareEncoderFamily::Qsv) => {
                args.extend([
                    "-global_quality".into(),
                    software_crf.to_string(),
                    "-preset".into(),
                    "veryfast".into(),
                ]);
            }
            Some(HardwareEncoderFamily::Amf) => {
                args.extend([
                    "-rc".into(),
                    "cqp".into(),
                    "-qp_i".into(),
                    software_crf.to_string(),
                    "-qp_p".into(),
                    software_crf.to_string(),
                    "-quality".into(),
                    "speed".into(),
                ]);
            }
            None => {}
        }
        return args;
    }
    args.extend([
        "-crf".into(),
        software_crf.to_string(),
        "-preset".into(),
        software_preset.to_string(),
    ]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_encoders_and_keeps_software_default() {
        let listing = [
            " V....D h264_videotoolbox VideoToolbox H.264 Encoder",
            " V....D hevc_qsv Intel Quick Sync Video",
        ]
        .join("\n");
        let capabilities = parse_encoder_capabilities(&listing, "ffmpeg");
        let software =
            select_video_encoder(&capabilities.encoders, &SelectEncoderOptions::default());
        assert_eq!(software.encoder, "libx264");
        assert!(!software.hardware);

        let mut hw = SelectEncoderOptions {
            prefer_hardware: true,
            ..Default::default()
        };
        let selected = select_video_encoder(&capabilities.encoders, &hw);
        assert_eq!(selected.encoder, "h264_videotoolbox");
        assert_eq!(selected.family, Some(HardwareEncoderFamily::Videotoolbox));

        hw.codec = VideoCodec::Hevc;
        let hevc = select_video_encoder(&capabilities.encoders, &hw);
        assert_eq!(hevc.encoder, "hevc_qsv");
        assert_eq!(hevc.family, Some(HardwareEncoderFamily::Qsv));
    }

    #[test]
    fn listed_hardware_encoders_that_do_not_open_fall_through() {
        let listing = [
            " V....D h264_nvenc NVIDIA NVENC H.264 encoder",
            " V....D h264_qsv H.264 (Intel Quick Sync Video acceleration)",
            " V....D h264_amf AMD AMF H.264 Encoder",
        ]
        .join("\n");
        let options = SelectEncoderOptions {
            prefer_hardware: true,
            ..Default::default()
        };

        let mut capabilities = parse_encoder_capabilities(&listing, "ffmpeg").encoders;
        let mut tried = Vec::new();
        keep_encoders_that_open(&mut capabilities, &options, |family| {
            tried.push(family);
            family == HardwareEncoderFamily::Qsv
        });
        assert_eq!(
            tried,
            [HardwareEncoderFamily::Nvenc, HardwareEncoderFamily::Qsv]
        );
        assert_eq!(
            select_video_encoder(&capabilities, &options).encoder,
            "h264_qsv"
        );

        let mut capabilities = parse_encoder_capabilities(&listing, "ffmpeg").encoders;
        keep_encoders_that_open(&mut capabilities, &options, |_| false);
        let selection = select_video_encoder(&capabilities, &options);
        assert_eq!(selection.encoder, "libx264");
        assert!(!selection.hardware);
    }

    #[test]
    fn hardware_encode_args_include_family_quality_flags() {
        let nvenc = EncoderSelection {
            encoder: "h264_nvenc".into(),
            hardware: true,
            family: Some(HardwareEncoderFamily::Nvenc),
            reason: "test".into(),
        };
        let args = encode_output_args(&nvenc, 23, "veryfast");
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-preset" && pair[1] == "p4"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-qp" && pair[1] == "23"));

        let qsv = EncoderSelection {
            encoder: "h264_qsv".into(),
            hardware: true,
            family: Some(HardwareEncoderFamily::Qsv),
            reason: "test".into(),
        };
        let args = encode_output_args(&qsv, 22, "veryfast");
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-global_quality" && pair[1] == "22"));
    }
}
