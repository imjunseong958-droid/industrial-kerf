//! What Kerf carries of the Premiere 26 video-effect catalogue beyond the original core set
//! (M5.11): definitions in its folders (Adjust … Video), the Immersive Video (VR) set, the Legacy
//! bin and the obsolete effects old projects still reference, plus the Effects-panel badge table.
//! The effects that make a look a model makes better, and those a clip's own properties do, are
//! not carried.
//!
//! Names, folders and parameter names follow the public effect list; parameter ranges/defaults are
//! our own where the reference does not pin them. Pixel implementations live in
//! `filmcraft_render::vfx`.

use super::*;

const IMMERSIVE: &[&str] = &["Video Effects", "Immersive Video"];
const TIME: &[&str] = &["Video Effects", "Time"];
pub(super) const UTILITY: &[&str] = &["Video Effects", "Utility"];
/// Hidden-in-Premiere effects that old projects still render (kept so they round-trip and render).
pub(super) const OBSOLETE: &[&str] = &["Video Effects", "Obsolete"];
/// The Effects panel's Legacy bin.
pub(super) const LEGACY: &[&str] = &["Legacy", "Video Effects"];

/// Track choices for effects that read another video track (Track Matte Key…).
pub const TRACK_CHOICES: &[&str] = &[
    "None", "Video 1", "Video 2", "Video 3", "Video 4", "Video 5", "Video 6", "Video 7", "Video 8",
];
/// Equirectangular frame layouts for the Immersive Video effects.
pub const FRAME_LAYOUTS: &[&str] = &[
    "Monoscopic",
    "Stereoscopic - Over/Under",
    "Stereoscopic - Side by Side",
];
/// Easing for the animated Transform presets (Grow, Shrink, Move, Spin).
pub const EASINGS: &[&str] = &["Linear", "Ease In", "Ease Out", "Ease In and Out"];
pub const ECHO_OPERATORS: &[&str] = &[
    "Add",
    "Maximum",
    "Minimum",
    "Screen",
    "Composite In Back",
    "Composite In Front",
    "Blend",
];
pub const SIMPLE_BLEND: &[&str] = &["Normal", "Add", "Screen", "Multiply", "Overlay"];

/// Per-light parameter ids of Lighting Effects: (type, color, center, major, minor, angle, intensity, focus).
pub const LIGHT_IDS: [[&str; 8]; 5] = [
    [
        "l1_type",
        "l1_color",
        "l1_center",
        "l1_major",
        "l1_minor",
        "l1_angle",
        "l1_intensity",
        "l1_focus",
    ],
    [
        "l2_type",
        "l2_color",
        "l2_center",
        "l2_major",
        "l2_minor",
        "l2_angle",
        "l2_intensity",
        "l2_focus",
    ],
    [
        "l3_type",
        "l3_color",
        "l3_center",
        "l3_major",
        "l3_minor",
        "l3_angle",
        "l3_intensity",
        "l3_focus",
    ],
    [
        "l4_type",
        "l4_color",
        "l4_center",
        "l4_major",
        "l4_minor",
        "l4_angle",
        "l4_intensity",
        "l4_focus",
    ],
    [
        "l5_type",
        "l5_color",
        "l5_center",
        "l5_major",
        "l5_minor",
        "l5_angle",
        "l5_intensity",
        "l5_focus",
    ],
];

fn pct(id: &'static str, label: &'static str, def: f64) -> ParamDef {
    f(id, label, def, 0.0, 100.0, "%")
}
fn layout() -> ParamDef {
    ch("frame_layout", "Frame Layout", FRAME_LAYOUTS, 0)
}

fn immersive() -> Vec<EffectDef> {
    let vr = |id, name, mut ps: Vec<ParamDef>| {
        ps.insert(0, layout());
        video(id, name, IMMERSIVE, ps)
    };
    vec![
        vr(
            "vr_blur",
            "VR Blur",
            vec![fs(
                "blurriness",
                "Blurriness",
                0.0,
                (0.0, 1000.0),
                (0.0, 100.0),
                "",
                1,
            )],
        ),
        vr(
            "vr_denoise",
            "VR De-Noise",
            vec![
                pct("amount", "Noise Level", 0.0),
                ch("noise_type", "Noise Type", &["Fast", "Slow"], 0),
                b("show_noise", "Show Only Noise", false),
            ],
        ),
        vr(
            "vr_plane_to_sphere",
            "VR Plane to Sphere",
            vec![
                fs(
                    "scale",
                    "Scale (Degrees)",
                    60.0,
                    (1.0, 180.0),
                    (1.0, 180.0),
                    "°",
                    1,
                ),
                fs("pan", "Pan", 0.0, (-180.0, 180.0), (-180.0, 180.0), "°", 1),
                fs("tilt", "Tilt", 0.0, (-90.0, 90.0), (-90.0, 90.0), "°", 1),
                ang("roll", "Roll", 0.0),
                pct("feather", "Edge Feather", 0.0),
            ],
        ),
        vr(
            "vr_projection",
            "VR Projection",
            vec![
                fs(
                    "fov",
                    "Target Field of View",
                    360.0,
                    (10.0, 360.0),
                    (10.0, 360.0),
                    "°",
                    1,
                ),
                ang("tilt", "Tilt (X Axis)", 0.0),
                ang("pan", "Pan (Y Axis)", 0.0),
                ang("roll", "Roll (Z Axis)", 0.0),
                b("stretch", "Stretch to Fit Frame", true),
            ],
        ),
        vr(
            "vr_rotate_sphere",
            "VR Rotate Sphere",
            vec![
                ang("tilt", "Tilt (X Axis)", 0.0),
                ang("pan", "Pan (Y Axis)", 0.0),
                ang("roll", "Roll (Z Axis)", 0.0),
            ],
        ),
        vr(
            "vr_sharpen",
            "VR Sharpen",
            vec![fs(
                "amount",
                "Sharpen Amount",
                0.0,
                (0.0, 500.0),
                (0.0, 100.0),
                "",
                0,
            )],
        ),
    ]
}

/// The new effect definitions (appended after the core set).
pub(super) fn defs() -> Vec<EffectDef> {
    let mut v = vec![
        // ---- Blur & Sharpen ----
        video(
            "reduce_interlace_flicker",
            "Reduce Interlace Flicker",
            BLUR,
            vec![fs(
                "softness",
                "Softness",
                0.0,
                (0.0, 10.0),
                (0.0, 10.0),
                "",
                2,
            )],
        ),
        // ---- Color Correction ----
        video(
            "asc_cdl",
            "ASC CDL",
            COLOR_CORR,
            vec![
                fs("r_slope", "Red Slope", 1.0, (0.0, 10.0), (0.0, 4.0), "", 3),
                fs(
                    "r_offset",
                    "Red Offset",
                    0.0,
                    (-1.0, 1.0),
                    (-1.0, 1.0),
                    "",
                    3,
                ),
                fs("r_power", "Red Power", 1.0, (0.0, 10.0), (0.0, 4.0), "", 3),
                fs(
                    "g_slope",
                    "Green Slope",
                    1.0,
                    (0.0, 10.0),
                    (0.0, 4.0),
                    "",
                    3,
                ),
                fs(
                    "g_offset",
                    "Green Offset",
                    0.0,
                    (-1.0, 1.0),
                    (-1.0, 1.0),
                    "",
                    3,
                ),
                fs(
                    "g_power",
                    "Green Power",
                    1.0,
                    (0.0, 10.0),
                    (0.0, 4.0),
                    "",
                    3,
                ),
                fs("b_slope", "Blue Slope", 1.0, (0.0, 10.0), (0.0, 4.0), "", 3),
                fs(
                    "b_offset",
                    "Blue Offset",
                    0.0,
                    (-1.0, 1.0),
                    (-1.0, 1.0),
                    "",
                    3,
                ),
                fs("b_power", "Blue Power", 1.0, (0.0, 10.0), (0.0, 4.0), "", 3),
                fs(
                    "saturation",
                    "Saturation",
                    1.0,
                    (0.0, 10.0),
                    (0.0, 4.0),
                    "",
                    3,
                ),
            ],
        ),
        video(
            "video_limiter",
            "Video Limiter",
            COLOR_CORR,
            vec![
                ch(
                    "clip_level",
                    "Clip Level",
                    &[
                        "100 IRE", "101 IRE", "102 IRE", "103 IRE", "104 IRE", "105 IRE",
                        "106 IRE", "107 IRE", "108 IRE", "109 IRE",
                    ],
                    0,
                ),
                ch(
                    "compression",
                    "Compression Before Clipping",
                    &["None", "3%", "5%", "10%", "20%"],
                    1,
                ),
                ch(
                    "axis",
                    "Reduce Axis",
                    &["Luma", "Chroma", "Luma and Chroma", "Smart Limit"],
                    3,
                ),
                b("gamut_warning", "Gamut Warning", false),
                col("warning_color", "Gamut Warning Color", [1.0, 0.0, 0.0, 1.0]),
            ],
        ),
        video(
            "vignette",
            "Vignette",
            COLOR_CORR,
            vec![
                fs(
                    "amount",
                    "Amount",
                    -50.0,
                    (-100.0, 100.0),
                    (-100.0, 100.0),
                    "",
                    1,
                ),
                pct("midpoint", "Midpoint", 50.0),
                fs(
                    "roundness",
                    "Roundness",
                    0.0,
                    (-100.0, 100.0),
                    (-100.0, 100.0),
                    "",
                    1,
                ),
                pct("feather", "Feather", 50.0),
                col("color", "Color", [0.0, 0.0, 0.0, 1.0]),
            ],
        ),
        // ---- Distort ----
        video(
            "warp_stabilizer",
            "Warp Stabilizer",
            DISTORT,
            vec![
                grp(
                    ch("result", "Result", &["Smooth Motion", "No Motion"], 0),
                    "Stabilization",
                ),
                grp(
                    fs(
                        "smoothness",
                        "Smoothness",
                        50.0,
                        (0.0, 1000.0),
                        (0.0, 200.0),
                        "%",
                        0,
                    ),
                    "Stabilization",
                ),
                grp(
                    ch(
                        "method",
                        "Method",
                        &[
                            "Position",
                            "Position, Scale, Rotation",
                            "Perspective",
                            "Subspace Warp",
                        ],
                        1,
                    ),
                    "Stabilization",
                ),
                grp(
                    b("preserve_scale", "Preserve Scale", false),
                    "Stabilization",
                ),
                grp(
                    ch(
                        "framing",
                        "Framing",
                        &[
                            "Stabilize Only",
                            "Stabilize, Crop",
                            "Stabilize, Crop, Auto-scale",
                            "Stabilize, Synthesize Edges",
                        ],
                        2,
                    ),
                    "Borders",
                ),
                grp(
                    fs(
                        "max_scale",
                        "Maximum Scale",
                        150.0,
                        (100.0, 1000.0),
                        (100.0, 300.0),
                        "%",
                        0,
                    ),
                    "Borders",
                ),
                grp(pct("action_safe", "Action-Safe Margin", 0.0), "Borders"),
                grp(
                    fs(
                        "additional_scale",
                        "Additional Scale",
                        100.0,
                        (50.0, 400.0),
                        (50.0, 200.0),
                        "%",
                        0,
                    ),
                    "Borders",
                ),
                grp(b("detailed", "Detailed Analysis", false), "Advanced"),
                grp(
                    pct("crop_less", "Crop Less <-> Smooth More", 50.0),
                    "Advanced",
                ),
            ],
        ),
        // ---- Generate ----
        // ---- Image Control ----
        video(
            "channel_mix",
            "Channel Mix",
            IMAGE_CONTROL,
            vec![
                f("rr", "Red-Red", 100.0, -200.0, 200.0, ""),
                f("rg", "Red-Green", 0.0, -200.0, 200.0, ""),
                f("rb", "Red-Blue", 0.0, -200.0, 200.0, ""),
                f("rc", "Red-Const", 0.0, -200.0, 200.0, ""),
                f("gr", "Green-Red", 0.0, -200.0, 200.0, ""),
                f("gg", "Green-Green", 100.0, -200.0, 200.0, ""),
                f("gb", "Green-Blue", 0.0, -200.0, 200.0, ""),
                f("gc", "Green-Const", 0.0, -200.0, 200.0, ""),
                f("br", "Blue-Red", 0.0, -200.0, 200.0, ""),
                f("bg", "Blue-Green", 0.0, -200.0, 200.0, ""),
                f("bb", "Blue-Blue", 100.0, -200.0, 200.0, ""),
                f("bc", "Blue-Const", 0.0, -200.0, 200.0, ""),
                b("monochrome", "Monochrome", false),
            ],
        ),
        video(
            "color_replace",
            "Color Replace",
            IMAGE_CONTROL,
            vec![
                pct("similarity", "Similarity", 10.0),
                b("solid", "Solid Colors", false),
                col("target", "Target Color", [1.0, 0.0, 0.0, 1.0]),
                col("replace", "Replace Color", [0.0, 0.0, 1.0, 1.0]),
            ],
        ),
        // ---- Keying ----
        video(
            "alpha_adjust",
            "Alpha Adjust",
            KEYING,
            vec![
                pct("opacity", "Opacity", 100.0),
                b("ignore", "Ignore Alpha", false),
                b("invert", "Invert Alpha", false),
                b("mask_only", "Mask Only", false),
            ],
        ),
        video(
            "logo_cutout",
            "Logo Cutout",
            KEYING,
            vec![
                ch(
                    "background",
                    "Background",
                    &["White", "Black", "Custom Color"],
                    0,
                ),
                col("color", "Background Color", [1.0, 1.0, 1.0, 1.0]),
                pct("threshold", "Threshold", 5.0),
                pct("softness", "Softness", 10.0),
                b("unmultiply", "Remove Fringe", true),
                b("invert", "Invert", false),
            ],
        ),
        // ---- Lights & Glows ----
        // ---- Perspective ----
        // ---- Stylize ----
        // ---- Time ----
        video(
            "posterize_time",
            "Posterize Time",
            TIME,
            vec![fs(
                "rate",
                "Frame Rate",
                12.0,
                (0.1, 99.0),
                (1.0, 60.0),
                "",
                2,
            )],
        ),
        // ---- Transform ----
        video(
            "auto_reframe",
            "Auto Reframe",
            TRANSFORM,
            vec![
                ch(
                    "preset",
                    "Motion Preset",
                    &["Slower Motion", "Default", "Faster Motion"],
                    1,
                ),
                pt("offset", "Reframe Offset", 0.0, 0.0),
                fs(
                    "zoom",
                    "Reframe Zoom",
                    100.0,
                    (100.0, 400.0),
                    (100.0, 200.0),
                    "%",
                    1,
                ),
                ch(
                    "aspect",
                    "Target Aspect",
                    &[
                        "Sequence",
                        "Vertical 9:16",
                        "Square 1:1",
                        "Vertical 4:5",
                        "Horizontal 16:9",
                    ],
                    0,
                ),
            ],
        ),
        // ---- Utility ----
        video(
            "cineon_converter",
            "Cineon Converter",
            UTILITY,
            vec![
                ch(
                    "conversion",
                    "Conversion Type",
                    &["Log to Linear", "Linear to Log", "Log to Log"],
                    0,
                ),
                fs(
                    "black10",
                    "10 Bit Black Point",
                    95.0,
                    (0.0, 1023.0),
                    (0.0, 1023.0),
                    "",
                    0,
                ),
                fs(
                    "black_internal",
                    "Internal Black Point",
                    0.0,
                    (0.0, 255.0),
                    (0.0, 255.0),
                    "",
                    0,
                ),
                fs(
                    "white10",
                    "10 Bit White Point",
                    685.0,
                    (0.0, 1023.0),
                    (0.0, 1023.0),
                    "",
                    0,
                ),
                fs(
                    "white_internal",
                    "Internal White Point",
                    255.0,
                    (0.0, 255.0),
                    (0.0, 255.0),
                    "",
                    0,
                ),
                fs("gamma", "Gamma", 1.7, (0.1, 5.0), (0.1, 5.0), "", 2),
                fs(
                    "rolloff",
                    "Highlight Rolloff",
                    20.0,
                    (0.0, 255.0),
                    (0.0, 100.0),
                    "",
                    0,
                ),
            ],
        ),
        // ---- Video ----
        video(
            "metadata_burnin",
            "Metadata & Timecode Burn-in",
            VIDEO,
            vec![
                ch(
                    "source",
                    "Source",
                    &[
                        "Sequence Timecode",
                        "Media Timecode",
                        "Clip Name",
                        "File Name",
                        "Frame Count",
                        "Sequence Name",
                    ],
                    0,
                ),
                pt("position", "Position", f64::NAN, f64::NAN),
                ch(
                    "alignment",
                    "Alignment",
                    &[
                        "Bottom Center",
                        "Bottom Left",
                        "Bottom Right",
                        "Top Center",
                        "Top Left",
                        "Top Right",
                        "Custom",
                    ],
                    0,
                ),
                fs("size", "Size", 6.0, (1.0, 100.0), (1.0, 30.0), "%", 1),
                col("color", "Text Color", [1.0, 1.0, 1.0, 1.0]),
                pct("opacity", "Background Opacity", 60.0),
                txt("prefix", "Prefix"),
            ],
        ),
        // ---- Obsolete (still render in old projects) ----
        // ---- Legacy bin ----
    ];
    v.extend(immersive());
    v
}

/// Parameter values written when Ultra Key's Setting is Default, Relaxed or Aggressive.
/// Custom (index 3) leaves the current values. Aggressive matches the matte that cleans
/// light and dark clothing on a typical green screen.
pub fn ultra_key_setting(setting: u32) -> Option<&'static [(&'static str, f64)]> {
    Some(match setting {
        0 => &[
            ("transparency", 45.0),
            ("highlight", 10.0),
            ("shadow", 50.0),
            ("tolerance", 50.0),
            ("pedestal", 10.0),
            ("choke", 0.0),
            ("soften", 0.0),
            ("contrast", 0.0),
            ("mid_point", 50.0),
            ("desaturate", 25.0),
            ("range", 50.0),
            ("spill", 50.0),
            ("spill_luma", 50.0),
        ],
        1 => &[
            ("transparency", 30.0),
            ("highlight", 15.0),
            ("shadow", 35.0),
            ("tolerance", 70.0),
            ("pedestal", 5.0),
            ("choke", 0.0),
            ("soften", 8.0),
            ("contrast", 0.0),
            ("mid_point", 50.0),
            ("desaturate", 15.0),
            ("range", 50.0),
            ("spill", 40.0),
            ("spill_luma", 50.0),
        ],
        2 => &[
            ("transparency", 40.0),
            ("highlight", 10.0),
            ("shadow", 55.0),
            ("tolerance", 90.0),
            ("pedestal", 50.0),
            ("choke", 10.0),
            ("soften", 10.0),
            ("contrast", 10.0),
            ("mid_point", 50.0),
            ("desaturate", 50.0),
            ("range", 50.0),
            ("spill", 50.0),
            ("spill_luma", 50.0),
        ],
        _ => return None,
    })
}

/// Extra parameters added to core effects by the 26.x rebuilds (old instances fall back to the
/// defaults, so projects round-trip unchanged).
pub(super) fn extend_core(v: &mut [EffectDef]) {
    for e in v.iter_mut() {
        match e.id {
            "ultra_key" => e.params.extend([
                grp(
                    f("contrast", "Contrast", 0.0, 0.0, 100.0, ""),
                    "Matte Cleanup",
                ),
                grp(
                    f("mid_point", "Mid Point", 50.0, 0.0, 100.0, ""),
                    "Matte Cleanup",
                ),
                grp(
                    f("desaturate", "Desaturate", 25.0, 0.0, 100.0, ""),
                    "Spill Suppression",
                ),
                grp(
                    f("range", "Range", 50.0, 0.0, 100.0, ""),
                    "Spill Suppression",
                ),
                grp(
                    f("spill_luma", "Luma", 50.0, 0.0, 100.0, ""),
                    "Spill Suppression",
                ),
                grp(
                    fs(
                        "cc_saturation",
                        "Saturation",
                        100.0,
                        (0.0, 200.0),
                        (0.0, 200.0),
                        "",
                        1,
                    ),
                    "Color Correction",
                ),
                grp(ang("cc_hue", "Hue", 0.0), "Color Correction"),
                grp(
                    fs(
                        "cc_luminance",
                        "Luminance",
                        100.0,
                        (0.0, 200.0),
                        (0.0, 200.0),
                        "",
                        1,
                    ),
                    "Color Correction",
                ),
            ]),
            "track_matte" => {
                if let Some(p) = e.params.iter_mut().find(|p| p.id == "matte") {
                    p.kind = ParamKind::Choice(TRACK_CHOICES);
                }
            }
            "noise" => e.params.push(fs(
                "grain_size",
                "Grain Size",
                1.0,
                (1.0, 10.0),
                (1.0, 6.0),
                "",
                1,
            )),
            "mosaic" => e.params.push(pct("softness", "Softness", 0.0)),
            _ => {}
        }
    }
}

/// Effects-panel badges (Accelerated, 32-bit, YUV) per the Premiere 26 list.
pub(super) fn apply_badges(v: &mut [EffectDef]) {
    for e in v
        .iter_mut()
        .filter(|e| e.kind == EffectKind::Video && !e.intrinsic)
    {
        let id = e.id;
        let folder = e.category.get(1).copied().unwrap_or("");
        let a32 = matches!(
            id,
            "directional_blur"
                | "gaussian_blur"
                | "asc_cdl"
                | "lumetri"
                | "video_limiter"
                | "vignette"
                | "channel_mix"
                | "logo_cutout"
                | "luma_key"
                | "track_matte"
                | "noise"
                | "mosaic"
                | "proc_amp"
        );
        let yuv = matches!(
            id,
            "proc_amp"
                | "lumetri"
                | "video_limiter"
                | "black_white"
                | "alpha_adjust"
                | "luma_key"
                | "track_matte"
        );
        let accel = match e.category.first().copied() {
            Some("Legacy") => true,
            _ if folder == "Obsolete" => false,
            _ => id != "clone",
        };
        e.accelerated = accel;
        e.float32 = a32;
        e.yuv = yuv;
    }
}
