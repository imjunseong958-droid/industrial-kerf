//! Each kernel against the CPU's code for its effect: on the GPU when there is one, and to the bit
//! for the kernels written once in `kerf_kernel`.

use super::*;
use crate::video::effects::apply;
use crate::video::project::find_effect;
use crate::video::tick::Tick;
use kerf_draw::blend::Blend;

/// A picture with bright spots, every kind of alpha, and hues.
fn picture(w: usize, h: usize) -> Image {
    let mut px = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let bright = (x / 9 + y / 7) % 5 == 0;
            let a = match y % 11 {
                0 => 0.0,
                1 | 2 => 0.6,
                _ => 1.0,
            };
            let c = if bright {
                [1.0, 0.95, 0.8]
            } else {
                [u, v, 1.0 - u]
            };
            px.extend_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
        }
    }
    Image { w, h, px }
}

fn context(seconds: f64) -> FxCtx<'static> {
    FxCtx {
        t: Tick::ZERO,
        px_scale: 1.0,
        seconds,
        timecode: "",
        clip_name: "",
        project: None,
        env: None,
        frame_rate: 24.0,
        working: kerf_color::WorkingSpace::Rec709,
    }
}

fn effect(id: &str, params: &[(&str, ParamValue)]) -> EffectInstance {
    let mut e = find_effect(id).unwrap_or_else(|| panic!("{id}")).instance();
    for (key, value) in params {
        e.params
            .get_mut(*key)
            .unwrap_or_else(|| panic!("{id}.{key}"))
            .value = value.clone();
    }
    e
}

fn on_gpu(gpu: &'static Gpu, image: &Image, kernel: &Kernel) -> Vec<f32> {
    let mut frame = gpu.frame();
    let picture = frame.image(image);
    let out = kernel.run(&mut frame, picture).expect("the stage builds");
    frame.read_floats(&out).expect("read back")
}

#[test]
fn kernels_match_the_effects() {
    let Some(gpu) = kerf_gpu::shared() else {
        eprintln!("no GPU; skipping");
        return;
    };
    let fl = ParamValue::Float;
    let on = ParamValue::Bool(true);
    let choice = ParamValue::Choice;
    let cases: Vec<(&str, Vec<(&str, ParamValue)>)> = vec![
        ("noise", vec![("amount", fl(30.0))]),
        (
            "noise",
            vec![
                ("amount", fl(40.0)),
                ("color", on.clone()),
                ("grain_size", fl(3.0)),
            ],
        ),
        ("noise", vec![("amount", fl(20.0)), ("clip", on.clone())]),
        ("noise", vec![("amount", fl(0.0))]),
        ("color_key", vec![]),
        (
            "color_key",
            vec![
                ("color", ParamValue::Color([0.1, 0.8, 0.2, 1.0])),
                ("tolerance", fl(90.0)),
                ("feather", fl(20.0)),
            ],
        ),
        ("luma_key", vec![]),
        (
            "luma_key",
            vec![("threshold", fl(50.0)), ("cutoff", fl(47.0))],
        ),
        (
            "luma_key",
            vec![("threshold", fl(60.0)), ("cutoff", fl(20.0))],
        ),
        ("logo_cutout", vec![]),
        (
            "logo_cutout",
            vec![("background", choice(1)), ("threshold", fl(15.0))],
        ),
        (
            "logo_cutout",
            vec![
                ("background", choice(2)),
                ("color", ParamValue::Color([0.2, 0.3, 0.9, 1.0])),
                ("invert", on.clone()),
            ],
        ),
        ("ultra_key", vec![]),
        (
            "ultra_key",
            vec![
                ("key_color", ParamValue::Color([0.1, 0.2, 0.9, 1.0])),
                ("setting", choice(3)),
                ("transparency", fl(70.0)),
                ("contrast", fl(30.0)),
                ("spill", fl(80.0)),
            ],
        ),
        (
            "ultra_key",
            vec![
                ("key_color", ParamValue::Color([0.3, 0.45, 1.0, 1.0])),
                ("setting", choice(3)),
                ("shadow", fl(100.0)),
                ("highlight", fl(60.0)),
                ("tolerance", fl(90.0)),
            ],
        ),
        ("ultra_key", vec![("output", choice(1))]),
        ("ultra_key", vec![("output", choice(2))]),
        (
            "ultra_key",
            vec![
                ("cc_saturation", fl(150.0)),
                ("cc_hue", fl(40.0)),
                ("cc_luminance", fl(80.0)),
            ],
        ),
        // Its presets set these, so only as they are set by hand.
        (
            "ultra_key",
            vec![("setting", choice(3)), ("choke", fl(20.0))],
        ),
        (
            "ultra_key",
            vec![("setting", choice(3)), ("soften", fl(30.0))],
        ),
        (
            "ultra_key",
            vec![
                ("setting", choice(3)),
                ("choke", fl(60.0)),
                ("soften", fl(10.0)),
                ("contrast", fl(30.0)),
                ("output", choice(1)),
            ],
        ),
    ];
    let cases: Vec<_> = cases.into_iter().chain(shaped_cases()).collect();
    let image = picture(96, 54);
    let mut failures = Vec::new();
    for (id, params) in cases {
        let e = effect(id, &params);
        let cx = context(1.3);
        let kernel = Kernel::eval(&e, &cx, (image.w, image.h)).expect("a kernel");
        let mut cpu = image.clone();
        apply(&mut cpu, &e, &cx);
        let gpu_px = on_gpu(gpu, &image, &kernel);
        let worst = cpu
            .px
            .iter()
            .zip(&gpu_px)
            .map(|(a, b)| (a - b).abs() / (1.0 + a.abs()))
            .fold(0.0f32, f32::max);
        if worst > 2e-3 {
            failures.push(format!("{id} {params:?}: differs by {worst:.2e}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Drop Shadow, Mosaic, Lens Distortion, and the Cineon Converter, each way they can be set.
fn shaped_cases() -> Vec<(&'static str, Vec<(&'static str, ParamValue)>)> {
    let fl = ParamValue::Float;
    let on = ParamValue::Bool(true);
    let color = |r: f32, g: f32, b: f32| ParamValue::Color([r, g, b, 1.0]);
    vec![
        ("drop_shadow", vec![]),
        (
            "drop_shadow",
            vec![
                ("distance", fl(12.0)),
                ("softness", fl(8.0)),
                ("opacity", fl(80.0)),
            ],
        ),
        (
            "drop_shadow",
            vec![
                ("distance", fl(7.5)),
                ("direction", fl(250.0)),
                ("color", color(0.9, 0.2, 0.1)),
                ("only", on.clone()),
            ],
        ),
        ("mosaic", vec![]),
        (
            "mosaic",
            vec![
                ("horizontal", fl(7.0)),
                ("vertical", fl(4.0)),
                ("softness", fl(60.0)),
            ],
        ),
        // More blocks across than there are pixels.
        (
            "mosaic",
            vec![("horizontal", fl(200.0)), ("vertical", fl(3.0))],
        ),
        // Blocks of a whole number of pixels, and all the way soft.
        (
            "mosaic",
            vec![
                ("horizontal", fl(16.0)),
                ("vertical", fl(9.0)),
                ("softness", fl(100.0)),
            ],
        ),
        // A 360° frame turned each way, seen through a narrower view, and one of two views.
        ("vr_rotate_sphere", vec![("pan", fl(40.0))]),
        (
            "vr_rotate_sphere",
            vec![("tilt", fl(25.0)), ("pan", fl(-110.0)), ("roll", fl(15.0))],
        ),
        (
            "vr_rotate_sphere",
            vec![("tilt", fl(-35.0)), ("frame_layout", ParamValue::Choice(1))],
        ),
        ("vr_projection", vec![("fov", fl(120.0))]),
        (
            "vr_projection",
            vec![
                ("fov", fl(200.0)),
                ("pan", fl(30.0)),
                ("stretch", ParamValue::Bool(false)),
                ("frame_layout", ParamValue::Choice(2)),
            ],
        ),
        // A flat picture on the sphere, turned, and with a soft edge.
        ("vr_plane_to_sphere", vec![]),
        (
            "vr_plane_to_sphere",
            vec![
                ("scale", fl(100.0)),
                ("pan", fl(-50.0)),
                ("tilt", fl(20.0)),
                ("roll", fl(10.0)),
                ("feather", fl(30.0)),
            ],
        ),
        (
            "vr_plane_to_sphere",
            vec![("scale", fl(80.0)), ("frame_layout", ParamValue::Choice(1))],
        ),
        // A lens that bulges, one that pinches, and one off its center.
        ("lens_distortion", vec![("curvature", fl(40.0))]),
        ("lens_distortion", vec![("curvature", fl(-60.0))]),
        (
            "lens_distortion",
            vec![
                ("curvature", fl(25.0)),
                ("h_decentering", fl(30.0)),
                ("v_decentering", fl(-45.0)),
            ],
        ),
        // The Cineon Converter, each way it goes.
        ("cineon_converter", vec![]),
        (
            "cineon_converter",
            vec![
                ("conversion", ParamValue::Choice(0)),
                ("gamma", fl(2.2)),
                ("rolloff", fl(40.0)),
                ("black_internal", fl(10.0)),
            ],
        ),
        (
            "cineon_converter",
            vec![
                ("conversion", ParamValue::Choice(1)),
                ("white10", fl(700.0)),
                ("white_internal", fl(230.0)),
            ],
        ),
        (
            "cineon_converter",
            vec![
                ("conversion", ParamValue::Choice(2)),
                ("black10", fl(120.0)),
            ],
        ),
    ]
}

/// What a kernel written once in `kerf_kernel` draws on the CPU, as `f32`, with the passes around it
/// that the CPU's own code makes; none for a kernel written by hand.
fn from_one_source(kernel: &Kernel, image: &Image) -> Option<Image> {
    let each = |from: &Image, pixel: &dyn Fn([f32; 4], f32, f32) -> [f32; 4]| {
        let mut out = from.clone();
        for (at, p) in out.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (at % from.w, at / from.w);
            *p = pixel(*p, x as f32 + 0.5, y as f32 + 0.5);
        }
        out
    };
    let (w, h) = (image.w, image.h);
    Some(match kernel {
        Kernel::DropShadow {
            offset,
            color,
            opacity,
            blur,
            only,
        } => {
            let moved = image.transformed(w, h, &Affine::translate(offset[0], offset[1]));
            let mut cast = each(&moved, &|p, _, _| kerf_kernel::shadow(p, *color, *opacity));
            if let Some(blur) = blur {
                blur.apply(&mut cast);
            }
            if !only {
                kerf_draw::blend::composite(&mut cast, image, 1.0, Blend::Normal);
            }
            cast
        }
        Kernel::Cineon { way, told } => {
            each(image, &|p, _, _| kerf_kernel::cineon_px(p, *way, told))
        }
        _ => return None,
    })
}

/// The kernels written once in `kerf_kernel` come, as `f32`, to the very values the CPU's code
/// comes to, which needs no GPU to know: what the GPU runs is that source written out.
#[test]
fn kernels_from_one_source_are_the_effects_to_the_bit() {
    let image = picture(96, 54);
    let cx = context(0.0);
    let mut held = 0;
    for (id, params) in shaped_cases() {
        let e = effect(id, &params);
        let kernel = Kernel::eval(&e, &cx, (image.w, image.h)).expect("a kernel");
        let Some(twin) = from_one_source(&kernel, &image) else {
            let by_hand = [
                "mosaic",
                "lens_distortion",
                "vr_rotate_sphere",
                "vr_projection",
                "vr_plane_to_sphere",
            ];
            assert!(by_hand.contains(&id), "{id} is written by hand");
            continue;
        };
        let mut cpu = image.clone();
        apply(&mut cpu, &e, &cx);
        // (equal rather than the same bits: a zero may come out with another sign)
        assert!(twin.px == cpu.px, "{id} {params:?}");
        assert_ne!(cpu.px, image.px, "{id} {params:?} changes nothing");
        held += 1;
    }
    assert_eq!(held, 3 + 4);
}

/// A Lumetri's curves, LUTs, HSL Secondary, and sharpening, each and together, and with what comes
/// before them in the effect, where the order shows.
#[test]
fn lumetri_past_its_plain_numbers_matches_the_effect() {
    let Some(gpu) = kerf_gpu::shared() else {
        eprintln!("no GPU; skipping");
        return;
    };
    let fl = ParamValue::Float;
    let on = ParamValue::Bool(true);
    let curve = |points: &[[f32; 2]]| ParamValue::Curve(points.to_vec());
    let bend = curve(&[[0.0, 0.0], [0.3, 0.2], [0.7, 0.85], [1.0, 1.0]]);
    let around = curve(&[[0.0, 0.5], [0.33, 0.85], [0.66, 0.25]]);
    let lut = ParamValue::Text("builtin:look-bleach-bypass".into());
    let log = ParamValue::Text("builtin:slog3-sgamut3cine-to-rec709".into());
    let cases: Vec<(&str, Vec<(&str, ParamValue)>)> = vec![
        ("sharpen", vec![("sharpen", fl(50.0))]),
        ("soften by sharpening", vec![("sharpen", fl(-40.0))]),
        ("luma curve", vec![("curve_luma", bend.clone())]),
        ("red curve", vec![("curve_red", bend.clone())]),
        (
            "green and blue curves",
            vec![("curve_green", bend.clone()), ("curve_blue", bend.clone())],
        ),
        ("hue vs sat", vec![("hue_vs_sat", around.clone())]),
        ("hue vs hue", vec![("hue_vs_hue", around.clone())]),
        ("hue vs luma", vec![("hue_vs_luma", around.clone())]),
        ("luma vs sat", vec![("luma_vs_sat", around.clone())]),
        ("sat vs sat", vec![("sat_vs_sat", around.clone())]),
        ("look LUT", vec![("look_lut", lut.clone())]),
        (
            "half a look LUT, then wheels",
            vec![
                ("look_lut", lut.clone()),
                ("look_intensity", fl(50.0)),
                ("wheel_shadows_l", fl(30.0)),
            ],
        ),
        ("input LUT", vec![("input_lut", log.clone())]),
        (
            "named look and curve",
            vec![
                ("look", ParamValue::Choice(6)),
                ("curve_luma", bend.clone()),
            ],
        ),
        ("HSL", vec![("hsl_on", on.clone()), ("hsl_hue", fl(200.0))]),
        (
            "HSL corrected",
            vec![
                ("hsl_on", on.clone()),
                ("hsl_hue", fl(120.0)),
                ("hsl_hue_range", fl(90.0)),
                ("hsl_sat", fl(150.0)),
                ("hsl_hue_shift", fl(30.0)),
                ("hsl_temp", fl(40.0)),
                ("hsl_tint", fl(-30.0)),
            ],
        ),
        (
            "HSL blurred",
            vec![
                ("hsl_on", on.clone()),
                ("hsl_hue_range", fl(120.0)),
                ("hsl_sat", fl(40.0)),
                ("hsl_blur", fl(20.0)),
            ],
        ),
        (
            "HSL mask, gray",
            vec![
                ("hsl_on", on.clone()),
                ("hsl_show_mask", ParamValue::Choice(1)),
            ],
        ),
        (
            "HSL mask, black",
            vec![
                ("hsl_on", on.clone()),
                ("hsl_show_mask", ParamValue::Choice(2)),
            ],
        ),
        (
            "HSL mask, white",
            vec![
                ("hsl_on", on.clone()),
                ("hsl_show_mask", ParamValue::Choice(3)),
            ],
        ),
        (
            "everything at once",
            vec![
                ("exposure", fl(0.4)),
                ("input_lut", log.clone()),
                ("look_lut", lut.clone()),
                ("wheel_shadows_l", fl(20.0)),
                ("curve_luma", bend.clone()),
                ("hue_vs_sat", around.clone()),
                ("hsl_on", on.clone()),
                ("hsl_sat", fl(60.0)),
                ("sharpen", fl(30.0)),
                ("vignette_amount", fl(-2.0)),
            ],
        ),
    ];
    let image = picture(96, 54);
    let mut failures = Vec::new();
    for (name, params) in cases {
        let e = effect("lumetri", &params);
        let cx = context(0.0);
        let kernel = Kernel::eval(&e, &cx, (image.w, image.h))
            .unwrap_or_else(|| panic!("{name} has no kernel"));
        let mut cpu = image.clone();
        apply(&mut cpu, &e, &cx);
        assert_ne!(cpu.px, image.px, "{name} changes nothing");
        let gpu_px = on_gpu(gpu, &image, &kernel);
        let worst = cpu
            .px
            .iter()
            .zip(&gpu_px)
            .map(|(a, b)| (a - b).abs() / (1.0 + a.abs()))
            .fold(0.0f32, f32::max);
        if worst > 2e-3 {
            failures.push(format!("{name}: differs by {worst:.2e}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(kerf_gpu::shared().is_some(), "the device failed");
}
