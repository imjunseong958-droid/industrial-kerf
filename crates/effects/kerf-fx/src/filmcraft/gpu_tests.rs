//! GPU against CPU on the effects: [`FxOp::apply`] is the reference. Carried from FilmCraft's
//! `fx_tests.rs`.
//!
//! The working picture read back as f32 must match to [`EXACT`] per value (f32 arithmetic and the
//! GPU's `pow` and `sqrt` differ in the last bits). Step functions (Posterize, Color Pass, Leave
//! Color and Change to Color keys, Extract) may flip a pixel that lands on a threshold; at most
//! [`FLIPS`] of the values may differ by more, and the others by up to [`STEP`].

use super::*;
use crate::filmcraft::effects::FxCtx;
use crate::filmcraft::project::{find_effect, EffectInstance, ParamValue};
use crate::filmcraft::tick::Tick;
use kerf_draw::Image;
use kerf_geom::Vec2;

const EXACT: f32 = 1e-4;
const FLIPS: f64 = 0.01;
const STEP: f32 = 1.0 / 255.0;

fn cx(t: Tick, px_scale: f32) -> FxCtx<'static> {
    FxCtx {
        t,
        px_scale,
        seconds: 0.0,
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
    for (k, v) in params {
        e.params
            .get_mut(*k)
            .unwrap_or_else(|| panic!("{id}.{k}"))
            .value = v.clone();
    }
    e
}

fn fl(v: f64) -> ParamValue {
    ParamValue::Float(v)
}
fn col(r: f32, g: f32, b: f32) -> ParamValue {
    ParamValue::Color([r, g, b, 1.0])
}
fn ch(c: u32) -> ParamValue {
    ParamValue::Choice(c)
}
fn pt(x: f64, y: f64) -> ParamValue {
    ParamValue::Vec2(Vec2::new(x, y))
}

/// Linear premultiplied test picture: hue sweep along
/// x, lightness along y, a grey ramp, black and white columns, super-whites (up to 1.6), and
/// alpha 1 / partial / 0 bands.
fn picture(w: u32, h: u32) -> Vec<f32> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 / (w - 1) as f32;
            let fy = y as f32 / (h - 1) as f32;
            let c = match x % 11 {
                0 => [0.0, 0.0, 0.0],
                1 => [1.0, 1.0, 1.0],
                2 => [fy, fy, fy],
                3 => [1.6 * fy, 1.2 * fy, 0.3],
                _ => {
                    let rgb = kerf_color::hsl_to_rgb(
                        fx,
                        0.25 + 0.75 * ((y % 5) as f32 / 4.0),
                        0.1 + 0.8 * fy,
                    );
                    rgb.map(kerf_color::srgb_to_linear)
                }
            };
            let a = match y % 7 {
                0 => 0.0,
                1 | 2 => 0.5 + 0.4 * fx,
                3 => 0.25,
                _ => 1.0,
            };
            px.extend_from_slice(&[c[0] * a, c[1] * a, c[2] * a, a]);
        }
    }
    px
}

/// Max |cpu − gpu| (finite values required) and the share of values off by more than 0.02.
fn compare(cpu: &[f32], gpu: &[f32]) -> (f32, f64) {
    assert_eq!(cpu.len(), gpu.len());
    let mut worst = 0f32;
    let mut flips = 0usize;
    for (a, b) in cpu.iter().zip(gpu) {
        assert!(b.is_finite(), "non-finite GPU value");
        let d = (a - b).abs() / (1.0 + a.abs());
        if d > 0.02 {
            flips += 1;
        } else {
            worst = worst.max(d);
        }
    }
    (worst, flips as f64 / cpu.len() as f64)
}

/// An effect id, its parameters, and whether a threshold may flip pixels.
type Case = (&'static str, Vec<(&'static str, ParamValue)>, bool);

fn cases() -> Vec<Case> {
    vec![
        (
            "brightness_contrast",
            vec![("brightness", fl(30.0)), ("contrast", fl(40.0))],
            false,
        ),
        (
            "brightness_contrast",
            vec![("brightness", fl(-100.0)), ("contrast", fl(-100.0))],
            false,
        ),
        (
            "brightness_contrast",
            vec![("brightness", fl(100.0)), ("contrast", fl(100.0))],
            false,
        ),
        (
            "brightness_contrast",
            vec![("brightness", fl(-20.0)), ("contrast", fl(900.0))],
            false,
        ),
        (
            "proc_amp",
            vec![
                ("brightness", fl(10.0)),
                ("contrast", fl(130.0)),
                ("hue", fl(200.0)),
                ("saturation", fl(150.0)),
            ],
            false,
        ),
        (
            "proc_amp",
            vec![("hue", fl(-400.0)), ("saturation", fl(0.0))],
            false,
        ),
        ("tint", vec![], false),
        (
            "tint",
            vec![
                ("black", col(0.2, 0.1, 0.5)),
                ("white", col(1.0, 0.9, 0.3)),
                ("amount", fl(60.0)),
            ],
            false,
        ),
        ("black_white", vec![], false),
        (
            "color_balance",
            vec![
                ("shadow_r", fl(50.0)),
                ("mid_g", fl(-40.0)),
                ("hi_b", fl(80.0)),
                ("shadow_b", fl(-100.0)),
                ("hi_r", fl(100.0)),
            ],
            false,
        ),
        (
            "color_balance",
            vec![
                ("mid_r", fl(70.0)),
                ("hi_g", fl(-60.0)),
                ("preserve", ParamValue::Bool(true)),
            ],
            false,
        ),
        (
            "leave_color",
            vec![
                ("amount", fl(80.0)),
                ("tolerance", fl(20.0)),
                ("softness", fl(30.0)),
            ],
            true,
        ),
        (
            "leave_color",
            vec![
                ("amount", fl(100.0)),
                ("color", col(0.1, 0.8, 0.2)),
                ("tolerance", fl(0.0)),
                ("softness", fl(0.0)),
            ],
            true,
        ),
        ("change_to_color", vec![], true),
        (
            "change_to_color",
            vec![
                ("hue_tol", fl(30.0)),
                ("softness", fl(10.0)),
                ("to", col(0.9, 0.9, 0.1)),
            ],
            true,
        ),
        ("color_pass", vec![("similarity", fl(30.0))], true),
        (
            "color_pass",
            vec![
                ("similarity", fl(45.0)),
                ("reverse", ParamValue::Bool(true)),
                ("color", col(0.2, 0.4, 0.9)),
            ],
            true,
        ),
        ("gamma_correction", vec![("gamma", fl(5.0))], false),
        ("gamma_correction", vec![("gamma", fl(28.0))], false),
        ("gamma_correction", vec![("gamma", fl(0.0))], false),
        ("gamma_correction", vec![("gamma", fl(-3.0))], false),
        (
            "levels",
            vec![
                ("in_black", fl(20.0)),
                ("in_white", fl(230.0)),
                ("out_black", fl(10.0)),
                ("out_white", fl(240.0)),
                ("gamma", fl(150.0)),
            ],
            false,
        ),
        (
            "levels",
            vec![
                ("in_black", fl(200.0)),
                ("in_white", fl(0.0)),
                ("gamma", fl(5.0)),
                ("out_white", fl(-50.0)),
            ],
            true,
        ),
        ("extract", vec![], true),
        ("extract", vec![("invert", ParamValue::Bool(true))], true),
        (
            "extract",
            vec![
                ("black", fl(40.0)),
                ("white", fl(200.0)),
                ("softness", fl(30.0)),
            ],
            true,
        ),
        (
            "extract",
            vec![
                ("black", fl(90.0)),
                ("white", fl(160.0)),
                ("invert", ParamValue::Bool(true)),
            ],
            true,
        ),
        (
            "extract",
            vec![
                ("black", fl(90.0)),
                ("white", fl(160.0)),
                ("softness", fl(1.0)),
            ],
            true,
        ),
        (
            "extract",
            vec![
                ("black", fl(40.0)),
                ("white", fl(200.0)),
                ("softness", fl(100.0)),
            ],
            true,
        ),
        (
            "extract",
            vec![("black", fl(160.0)), ("white", fl(90.0))],
            true,
        ),
        (
            "extract",
            vec![("black", fl(90.0)), ("white", fl(90.0))],
            true,
        ),
        ("invert", vec![("blend", fl(30.0))], false),
        ("invert", vec![("channel", ParamValue::Choice(1))], false),
        (
            "invert",
            vec![("channel", ParamValue::Choice(2)), ("blend", fl(70.0))],
            false,
        ),
        ("invert", vec![("channel", ParamValue::Choice(3))], false),
        (
            "invert",
            vec![("channel", ParamValue::Choice(4)), ("blend", fl(20.0))],
            false,
        ),
        ("invert", vec![("channel", ParamValue::Choice(9))], false),
        ("posterize", vec![("levels", fl(2.0))], true),
        ("posterize", vec![("levels", fl(7.0))], true),
        ("posterize", vec![("levels", fl(255.0))], true),
        ("gaussian_blur", vec![("blurriness", fl(0.0))], false),
        ("gaussian_blur", vec![("blurriness", fl(5.0))], false),
        (
            "gaussian_blur",
            vec![
                ("blurriness", fl(5.0)),
                ("repeat_edge", ParamValue::Bool(true)),
            ],
            false,
        ),
        ("gaussian_blur", vec![("blurriness", fl(80.0))], false),
        (
            "gaussian_blur",
            vec![
                ("blurriness", fl(80.0)),
                ("repeat_edge", ParamValue::Bool(true)),
            ],
            false,
        ),
        (
            "gaussian_blur",
            vec![
                ("blurriness", fl(12.0)),
                ("dimensions", ParamValue::Choice(1)),
            ],
            false,
        ),
        (
            "gaussian_blur",
            vec![
                ("blurriness", fl(12.0)),
                ("dimensions", ParamValue::Choice(2)),
                ("repeat_edge", ParamValue::Bool(true)),
            ],
            false,
        ),
        (
            "gaussian_blur",
            vec![
                ("blurriness", fl(3000.0)),
                ("repeat_edge", ParamValue::Bool(true)),
            ],
            false,
        ),
        ("gaussian_blur", vec![("blurriness", fl(1e9))], false),
        ("gaussian_blur", vec![("blurriness", fl(-50.0))], false),
        ("camera_blur", vec![("percent", fl(20.0))], false),
        (
            "directional_blur",
            vec![("length", fl(10.0)), ("direction", fl(30.0))],
            false,
        ),
        ("directional_blur", vec![("length", fl(0.2))], false),
        (
            "directional_blur",
            vec![("length", fl(1000.0)), ("direction", fl(-100.0))],
            false,
        ),
        ("sharpen", vec![("amount", fl(80.0))], false),
        ("sharpen", vec![("amount", fl(4000.0))], false),
        (
            "unsharp_mask",
            vec![
                ("amount", fl(150.0)),
                ("radius", fl(3.0)),
                ("threshold", fl(10.0)),
            ],
            true,
        ),
        ("unsharp_mask", vec![("radius", fl(0.1))], false),
        (
            "unsharp_mask",
            vec![("amount", fl(300.0)), ("radius", fl(250.0))],
            false,
        ),
        (
            "crop",
            vec![
                ("left", fl(10.0)),
                ("top", fl(5.0)),
                ("right", fl(20.0)),
                ("bottom", fl(15.0)),
            ],
            false,
        ),
        (
            "crop",
            vec![
                ("left", fl(10.0)),
                ("top", fl(5.0)),
                ("right", fl(20.0)),
                ("bottom", fl(15.0)),
                ("feather", fl(12.0)),
            ],
            false,
        ),
        (
            "crop",
            vec![
                ("left", fl(12.0)),
                ("top", fl(5.0)),
                ("right", fl(20.0)),
                ("zoom", ParamValue::Bool(true)),
            ],
            false,
        ),
        (
            "crop",
            vec![
                ("left", fl(70.0)),
                ("right", fl(70.0)),
                ("feather", fl(5000.0)),
            ],
            false,
        ),
        ("edge_feather", vec![("amount", fl(30.0))], false),
        (
            "transform",
            vec![
                ("scale_height", fl(150.0)),
                ("rotation", fl(30.0)),
                ("position", pt(30.0, 20.0)),
                ("opacity", fl(60.0)),
            ],
            false,
        ),
        (
            "transform",
            vec![
                ("uniform_scale", ParamValue::Bool(false)),
                ("scale_width", fl(80.0)),
                ("scale_height", fl(120.0)),
                ("skew", fl(20.0)),
                ("skew_axis", fl(45.0)),
                ("anchor", pt(10.0, 30.0)),
            ],
            false,
        ),
        ("horizontal_flip", vec![], false),
        ("vertical_flip", vec![], false),
        (
            "mirror",
            vec![("center", pt(20.0, 10.0)), ("angle", fl(30.0))],
            false,
        ),
        ("mirror", vec![("angle", fl(200.0))], false),
        (
            "offset",
            vec![("shift", pt(10.0, 30.0)), ("blend", fl(25.0))],
            false,
        ),
        ("offset", vec![("shift", pt(-1e5, 7.25))], false),
        (
            "asc_cdl",
            vec![
                ("r_slope", fl(1.4)),
                ("g_offset", fl(-0.1)),
                ("b_power", fl(2.2)),
                ("saturation", fl(1.6)),
            ],
            false,
        ),
        (
            "asc_cdl",
            vec![
                ("r_power", fl(0.0)),
                ("g_slope", fl(0.0)),
                ("saturation", fl(0.0)),
            ],
            false,
        ),
        (
            "channel_mix",
            vec![
                ("rr", fl(50.0)),
                ("rg", fl(60.0)),
                ("gb", fl(-80.0)),
                ("bc", fl(30.0)),
            ],
            false,
        ),
        (
            "channel_mix",
            vec![("rb", fl(120.0)), ("monochrome", ParamValue::Bool(true))],
            false,
        ),
        ("color_replace", vec![("similarity", fl(40.0))], true),
        (
            "color_replace",
            vec![
                ("similarity", fl(25.0)),
                ("solid", ParamValue::Bool(true)),
                ("target", col(0.3, 0.6, 0.2)),
            ],
            true,
        ),
        ("color_replace", vec![("similarity", fl(0.0))], true),
        ("alpha_adjust", vec![("opacity", fl(60.0))], false),
        (
            "alpha_adjust",
            vec![
                ("ignore", ParamValue::Bool(true)),
                ("invert", ParamValue::Bool(true)),
            ],
            false,
        ),
        (
            "alpha_adjust",
            vec![
                ("invert", ParamValue::Bool(true)),
                ("mask_only", ParamValue::Bool(true)),
                ("opacity", fl(150.0)),
            ],
            false,
        ),
        ("vignette", vec![], false),
        (
            "vignette",
            vec![
                ("amount", fl(-100.0)),
                ("midpoint", fl(50.0)),
                ("roundness", fl(0.0)),
                ("feather", fl(50.0)),
            ],
            false,
        ),
        (
            "vignette",
            vec![
                ("amount", fl(-30.0)),
                ("midpoint", fl(20.0)),
                ("roundness", fl(-100.0)),
                ("feather", fl(0.0)),
                ("color", col(0.1, 0.4, 0.7)),
            ],
            false,
        ),
        (
            "vignette",
            vec![
                ("amount", fl(40.0)),
                ("midpoint", fl(90.0)),
                ("roundness", fl(100.0)),
                ("feather", fl(100.0)),
            ],
            false,
        ),
        ("video_limiter", vec![], false),
        (
            "video_limiter",
            vec![
                ("axis", ch(0)),
                ("clip_level", ch(0)),
                ("compression", ch(0)),
            ],
            false,
        ),
        (
            "video_limiter",
            vec![
                ("axis", ch(1)),
                ("clip_level", ch(9)),
                ("compression", ch(4)),
            ],
            false,
        ),
        (
            "video_limiter",
            vec![
                ("axis", ch(2)),
                ("clip_level", ch(5)),
                ("compression", ch(2)),
            ],
            false,
        ),
        (
            "video_limiter",
            vec![
                ("axis", ch(3)),
                ("clip_level", ch(0)),
                ("compression", ch(1)),
            ],
            false,
        ),
        (
            "video_limiter",
            vec![
                ("axis", ch(2)),
                ("clip_level", ch(0)),
                ("compression", ch(0)),
                ("gamut_warning", ParamValue::Bool(true)),
                ("warning_color", col(0.0, 1.0, 0.0)),
            ],
            false,
        ),
        (
            "lumetri",
            vec![
                ("temperature", fl(20.0)),
                ("tint", fl(-10.0)),
                ("exposure", fl(0.5)),
                ("contrast", fl(15.0)),
                ("highlights", fl(-20.0)),
                ("shadows", fl(25.0)),
                ("whites", fl(10.0)),
                ("blacks", fl(-15.0)),
                ("saturation", fl(110.0)),
                ("creative_on", ParamValue::Bool(false)),
                ("vignette_on", ParamValue::Bool(false)),
            ],
            false,
        ),
        (
            "lumetri",
            vec![
                ("basic_on", ParamValue::Bool(false)),
                ("creative_sat", fl(120.0)),
                ("vibrance", fl(30.0)),
                ("faded_film", fl(25.0)),
                ("shadow_tint", col(0.4, 0.45, 0.6)),
                ("highlight_tint", col(0.6, 0.55, 0.4)),
                ("vignette_on", ParamValue::Bool(false)),
            ],
            false,
        ),
        (
            "lumetri",
            vec![
                ("basic_on", ParamValue::Bool(false)),
                ("creative_on", ParamValue::Bool(false)),
                ("vignette_amount", fl(-3.0)),
                ("vignette_midpoint", fl(45.0)),
                ("vignette_roundness", fl(-30.0)),
                ("vignette_feather", fl(60.0)),
            ],
            false,
        ),
        (
            "lumetri",
            vec![
                ("temperature", fl(-15.0)),
                ("tint", fl(10.0)),
                ("exposure", fl(0.3)),
                ("contrast", fl(20.0)),
                ("highlights", fl(-10.0)),
                ("shadows", fl(15.0)),
                ("whites", fl(-5.0)),
                ("blacks", fl(5.0)),
                ("saturation", fl(105.0)),
                ("creative_sat", fl(110.0)),
                ("vibrance", fl(20.0)),
                ("faded_film", fl(15.0)),
                ("shadow_tint", col(0.48, 0.5, 0.55)),
                ("highlight_tint", col(0.52, 0.5, 0.45)),
                ("vignette_amount", fl(2.0)),
                ("vignette_midpoint", fl(50.0)),
                ("vignette_roundness", fl(20.0)),
                ("vignette_feather", fl(50.0)),
            ],
            false,
        ),
        (
            "lumetri",
            vec![
                ("exposure", fl(4.0)),
                ("contrast", fl(100.0)),
                ("temperature", fl(100.0)),
                ("tint", fl(100.0)),
                ("whites", fl(100.0)),
                ("blacks", fl(100.0)),
            ],
            false,
        ),
        (
            "lumetri",
            vec![
                ("exposure", fl(-4.0)),
                ("contrast", fl(-100.0)),
                ("temperature", fl(-100.0)),
                ("tint", fl(-100.0)),
                ("whites", fl(-100.0)),
                ("blacks", fl(-100.0)),
            ],
            false,
        ),
    ]
}

/// `ops` on the GPU, read back.
fn on_gpu(gpu: &'static Gpu, image: &Image, ops: &[FxOp]) -> Vec<f32> {
    let mut frame = gpu.frame();
    let picture = frame.image(image);
    let out = run(&mut frame, picture, ops).expect("the stage builds");
    frame.read_floats(&out).expect("read back")
}

#[test]
fn gpu_effects_match_cpu_exactly() {
    let Some(gpu) = kerf_gpu::shared() else {
        eprintln!("no GPU; skipping");
        return;
    };
    let (w, h) = (67usize, 41usize);
    let image = Image {
        w,
        h,
        px: picture(w as u32, h as u32),
    };
    let mut failures = Vec::new();
    for (id, params, steps) in cases() {
        let e = effect(id, &params);
        let op = FxOp::eval(&e, &cx(Tick::ZERO, 1.0), w, h)
            .unwrap_or_else(|| panic!("{id} has no GPU op"));
        assert!(op.gpu_ok(), "{id} {params:?}: not GPU-capable");
        let mut cpu = image.clone();
        op.apply(&mut cpu);
        let gpu_px = on_gpu(gpu, &image, std::slice::from_ref(&op));
        let (worst, flips) = compare(&cpu.px, &gpu_px);
        // A step function's pixel on a threshold moves a whole step, which at Posterize's finest
        // levels is under what counts as a flip.
        let exact = if steps { STEP } else { EXACT };
        if worst > exact || flips > if steps { FLIPS } else { 0.0 } {
            failures.push(format!("{id} {params:?}: {worst:.2e}, flips {flips:.4}"));
        }
    }
    assert!(
        failures.is_empty(),
        "GPU effects differ from the CPU:\n{}",
        failures.join("\n")
    );
}

/// Chains run in order, and Unsharp keeps its original while blurring, between other blurs.
#[test]
fn gpu_effect_chains_match_cpu() {
    let Some(gpu) = kerf_gpu::shared() else {
        eprintln!("no GPU; skipping");
        return;
    };
    let (w, h) = (96usize, 54usize);
    let image = Image {
        w,
        h,
        px: picture(w as u32, h as u32),
    };
    let chains: Vec<Vec<EffectInstance>> = vec![
        vec![
            effect(
                "brightness_contrast",
                &[("brightness", fl(20.0)), ("contrast", fl(30.0))],
            ),
            effect("gaussian_blur", &[("blurriness", fl(8.0))]),
            effect("tint", &[("amount", fl(50.0))]),
        ],
        vec![
            effect(
                "gaussian_blur",
                &[
                    ("blurriness", fl(90.0)),
                    ("repeat_edge", ParamValue::Bool(true)),
                ],
            ),
            effect(
                "unsharp_mask",
                &[("amount", fl(200.0)), ("radius", fl(4.0))],
            ),
            effect(
                "gaussian_blur",
                &[
                    ("blurriness", fl(3.0)),
                    ("dimensions", ParamValue::Choice(1)),
                ],
            ),
            effect("sharpen", &[("amount", fl(50.0))]),
        ],
        vec![
            effect(
                "crop",
                &[
                    ("left", fl(10.0)),
                    ("bottom", fl(20.0)),
                    ("feather", fl(4.0)),
                ],
            ),
            effect(
                "transform",
                &[("rotation", fl(-15.0)), ("scale_height", fl(110.0))],
            ),
            effect("horizontal_flip", &[]),
            effect("levels", &[("in_black", fl(15.0)), ("gamma", fl(80.0))]),
            effect("offset", &[("shift", pt(70.0, 10.0))]),
            effect("invert", &[("channel", ParamValue::Choice(4))]),
            effect(
                "directional_blur",
                &[("length", fl(6.0)), ("direction", fl(90.0))],
            ),
        ],
    ];
    for chain in chains {
        let cx = cx(Tick::ZERO, 1.0);
        let ops: Vec<FxOp> = chain
            .iter()
            .map(|e| FxOp::eval(e, &cx, w, h).expect("a GPU op"))
            .collect();
        let mut cpu = image.clone();
        for op in &ops {
            op.apply(&mut cpu);
        }
        let gpu_px = on_gpu(gpu, &image, &ops);
        let (worst, flips) = compare(&cpu.px, &gpu_px);
        assert!(
            worst <= EXACT * 4.0 && flips <= FLIPS,
            "{worst:.2e}, flips {flips:.4}"
        );
    }
}

/// The effects `FxOp` covers are the ones it draws: nothing `apply` runs ahead of it takes them.
#[test]
fn an_op_draws_what_the_effect_draws() {
    let (w, h) = (64usize, 40usize);
    let image = Image {
        w,
        h,
        px: picture(w as u32, h as u32),
    };
    let context = cx(Tick::ZERO, 1.0);
    let mut covered = 0;
    for (id, def) in crate::filmcraft::catalog::definitions() {
        let e = def.instance();
        let Some(op) = FxOp::eval(&e, &context, w, h).filter(FxOp::gpu_ok) else {
            continue;
        };
        covered += 1;
        let mut by_op = image.clone();
        op.apply(&mut by_op);
        let mut by_effect = image.clone();
        crate::filmcraft::effects::apply(&mut by_effect, &e, &context);
        assert_eq!(by_op.px, by_effect.px, "{id}");
    }
    assert!(covered >= 20, "{covered} effects have kernels");
}
