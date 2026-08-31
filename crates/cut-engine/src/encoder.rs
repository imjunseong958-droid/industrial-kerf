//! Hardware encoder capability discovery and explicit selection.
//!
//! Mirrors `@cut/media` encoder.ts. Hardware is never selected unless the
//! caller opts in with `prefer_hardware`.

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

pub fn known_hardware_encoders() -> Vec<EncoderCapability> {
    let mut out = Vec::with_capacity(HardwareEncoderFamily::ALL.len() * 2);
    for family in HardwareEncoderFamily::ALL {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            out.push(EncoderCapability {
                family,
                encoder: family.encoder_name(codec).to_string(),
                available: false,
                reason: Some("Not detected".into()),
            });
        }
    }
    out
}

/// Parse a captured `ffmpeg -encoders` listing without running a process.
pub fn parse_encoder_capabilities(output: &str, ffmpeg_bin: &str) -> EncoderCapabilities {
    capabilities_for_output(output, ffmpeg_bin)
}

fn capabilities_for_output(output: &str, ffmpeg_bin: &str) -> EncoderCapabilities {
    let mut available_names = std::collections::HashSet::new();
    for family in HardwareEncoderFamily::ALL {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let name = family.encoder_name(codec);
            if encoder_listed(output, name) {
                available_names.insert(name);
            }
        }
    }
    let mut encoders = Vec::with_capacity(HardwareEncoderFamily::ALL.len() * 2);
    for family in HardwareEncoderFamily::ALL {
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let encoder = family.encoder_name(codec).to_string();
            if available_names.contains(encoder.as_str()) {
                encoders.push(EncoderCapability {
                    family,
                    encoder,
                    available: true,
                    reason: None,
                });
            } else {
                encoders.push(EncoderCapability {
                    family,
                    encoder,
                    available: false,
                    reason: Some("Encoder was not listed by ffmpeg".into()),
                });
            }
        }
    }
    EncoderCapabilities {
        ffmpeg_bin: ffmpeg_bin.to_string(),
        encoders,
        raw: Some(output.to_string()),
    }
}

fn encoder_listed(output: &str, name: &str) -> bool {
    for line in output.lines() {
        for token in line.split_whitespace() {
            if token == name {
                return true;
            }
        }
    }
    false
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
        if selection.family == Some(HardwareEncoderFamily::Videotoolbox) {
            args.extend(["-q:v".into(), "65".into()]);
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

        let mut hw = SelectEncoderOptions::default();
        hw.prefer_hardware = true;
        let selected = select_video_encoder(&capabilities.encoders, &hw);
        assert_eq!(selected.encoder, "h264_videotoolbox");
        assert_eq!(selected.family, Some(HardwareEncoderFamily::Videotoolbox));

        hw.codec = VideoCodec::Hevc;
        let hevc = select_video_encoder(&capabilities.encoders, &hw);
        assert_eq!(hevc.encoder, "hevc_qsv");
        assert_eq!(hevc.family, Some(HardwareEncoderFamily::Qsv));
    }
}
