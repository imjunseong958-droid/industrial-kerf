//! What each carried effect does and when an editor reaches for it, as
//! agents read it in the catalog. One line each: the effect, then a few
//! moments or looks it serves.

pub(crate) const GUIDE: &[(&str, &str)] = &[
    // Blur & Sharpen
    (
        "blur.bilateral",
        "Smooths flat areas while keeping edges; skin smoothing and beauty passes.",
    ),
    (
        "blur.radialzoom",
        "Streaks outward from a point like a fast zoom; impact hits and speed moments.",
    ),
    // Color Correction
    (
        "color.curves",
        "Bends tone with curves for RGB and each channel, each written as points x,y from 0 to 1 (rgb=\"0,0 0.5,0.6 1,1\" lifts the mids); contrast, fades, and color casts.",
    ),
    (
        "color.autocolor",
        "Automatic color balance; a quick fix for color casts.",
    ),
    (
        "color.autocontrast",
        "Automatic contrast; flat footage that needs punch.",
    ),
    (
        "color.autolevels",
        "Automatic levels per channel; quick normalization.",
    ),
    (
        "color.huesaturation",
        "Hue, saturation, lightness, or colorize; desaturate, recolor, boost color.",
    ),
    (
        "color.photofilter",
        "A colored lens filter, warming or cooling; a quick mood tint.",
    ),
    (
        "color.selectivecolor",
        "Adjusts the color mix within color ranges; targeted color tweaks.",
    ),
    (
        "color.shadowhighlight",
        "Lifts shadows and recovers highlights; backlit or contrasty shots.",
    ),
    // Distort
    (
        "distort.rollingshutterrepair",
        "Straightens the skew and wobble a rolling shutter leaves, from the motion between frames; fast pans on phones and DSLRs.",
    ),
    (
        "distort.upscale",
        "Enlarges while keeping detail; low-resolution footage.",
    ),
    (
        "distort.opticscompensation",
        "Adds or removes lens barrel distortion; fisheye looks or wide-lens fixes.",
    ),
    // Generate
    (
        "generate.audiospectrum",
        "Draws a sound track's frequencies as bars or lines (audioLayer=A1), along a mask if one is named; music visualizers and podcast clips.",
    ),
    (
        "generate.audiowaveform",
        "Draws a sound track's waveform as a line (audioLayer=A1), along a mask if one is named; voice clips and music visualizers.",
    ),
    // Keying
    (
        "key.advancedspill",
        "Removes green or blue spill; cleanup after keying.",
    ),
    (
        "key.keycleaner",
        "Restores detail at key edges; hair and motion blur.",
    ),
    (
        "key.unmult",
        "Makes black transparent; lay light leaks, fire, and sparks over footage.",
    ),
    // Matte
    (
        "matte.simplechoker",
        "Shrinks or grows the matte edge; quick edge cleanup.",
    ),
    // Noise & Grain
    (
        "noise.addgrain",
        "Film grain; a filmic texture, or matching footage.",
    ),
    (
        "noise.dustscratches",
        "Removes small specks and scratches; clean old scans.",
    ),
    (
        "noise.removegrain",
        "Removes grain and noise; low-light footage.",
    ),
];
