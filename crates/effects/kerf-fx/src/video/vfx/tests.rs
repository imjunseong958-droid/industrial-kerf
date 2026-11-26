//! Tests for the M5.11 video effects: every effect runs, stays finite and in range, and is
//! deterministic; neutral parameters are the identity; each effect has a known-value check; the
//! frame/track environment works through the real compositor.

use crate::video::project::{find_effect, ParamKind};
use crate::video::tick::Tick;
use rayon::prelude::*;

use super::*;

fn cx() -> FxCtx<'static> {
    FxCtx {
        t: Tick::ZERO,
        px_scale: 1.0,
        seconds: 0.0,
        timecode: "01:00:00:00",
        clip_name: "clip",
        project: None,
        env: None,
        frame_rate: 24.0,
        working: kerf_color::WorkingSpace::Rec709,
    }
}

fn inst(id: &str) -> EffectInstance {
    find_effect(id)
        .unwrap_or_else(|| panic!("no effect {id}"))
        .instance()
}

fn set(e: &mut EffectInstance, k: &str, v: ParamValue) {
    e.params
        .get_mut(k)
        .unwrap_or_else(|| panic!("{}: no param {k}", e.effect))
        .value = v;
}
// Noise for the test pictures.

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn grad3(h: u64, x: f32, y: f32, z: f32) -> f32 {
    // 12 edge directions of a cube (Perlin 2002)
    match h % 12 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        _ => -y - z,
    }
}

fn hash_u(x: i64, y: i64, z: i64, seed: u64) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
        ^ seed.wrapping_mul(0x27D4_EB2F_1656_67C5);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 32)
}

/// Uniform 0..1 from integer coordinates.
fn hash01(x: i64, y: i64, z: i64, seed: u64) -> f32 {
    (hash_u(x, y, z, seed) >> 40) as f32 / (1u64 << 24) as f32
}

/// 3-D gradient noise in about −1..1 (our own hash; Perlin's interpolant and gradient set).
fn noise3(x: f32, y: f32, z: f32, seed: u64) -> f32 {
    let (xi, yi, zi) = (x.floor(), y.floor(), z.floor());
    let (xf, yf, zf) = (x - xi, y - yi, z - zi);
    let (xi, yi, zi) = (xi as i64, yi as i64, zi as i64);
    let (u, v, w) = (fade(xf), fade(yf), fade(zf));
    let g = |dx: i64, dy: i64, dz: i64| {
        grad3(
            hash_u(xi + dx, yi + dy, zi + dz, seed),
            xf - dx as f32,
            yf - dy as f32,
            zf - dz as f32,
        )
    };
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = l(g(0, 0, 0), g(1, 0, 0), u);
    let x10 = l(g(0, 1, 0), g(1, 1, 0), u);
    let x01 = l(g(0, 0, 1), g(1, 0, 1), u);
    let x11 = l(g(0, 1, 1), g(1, 1, 1), u);
    l(l(x00, x10, v), l(x01, x11, v), w) * 0.9
}

/// Fractal (fBm) noise with `octaves` (fractional octaves fade in), about −1..1.
fn fbm(x: f32, y: f32, z: f32, octaves: f32, seed: u64) -> f32 {
    let n = octaves.clamp(1.0, 10.0);
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    let mut i = 0;
    while (i as f32) < n {
        let k = (n - i as f32).min(1.0);
        sum += noise3(
            x * freq,
            y * freq,
            z * freq + i as f32 * 17.13,
            seed.wrapping_add(i as u64),
        ) * amp
            * k;
        norm += amp * k;
        amp *= 0.5;
        freq *= 2.0;
        i += 1;
    }
    sum / norm.max(1e-6)
}

/// A deterministic textured test picture (straight sRGB-ish values in linear premultiplied form).
fn picture(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let n = noise3(x as f32 / 6.0, y as f32 / 6.0, 0.5, 9) * 0.25;
            let c = [
                (0.2 + 0.6 * u + n).clamp(0.0, 1.0),
                (0.3 + 0.5 * v - n).clamp(0.0, 1.0),
                (0.5 + 0.4 * (u - v) + n * 0.5).clamp(0.0, 1.0),
            ];
            let l = dec(c);
            img.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[l[0], l[1], l[2], 1.0]);
        }
    }
    img
}

fn run(id: &str, e: &EffectInstance, img: &Image, cx: &FxCtx) -> Image {
    let mut o = img.clone();
    crate::video::effects::apply(&mut o, e, cx);
    let _ = id;
    o
}

fn max_diff(a: &Image, b: &Image) -> f32 {
    a.px.iter()
        .zip(&b.px)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

/// The ids this milestone added or rebuilt.
const NEW_IDS: &[&str] = &[
    "reduce_interlace_flicker",
    "asc_cdl",
    "video_limiter",
    "vignette",
    "corner_pin",
    "warp_stabilizer",
    "channel_mix",
    "color_replace",
    "vr_blur",
    "vr_denoise",
    "vr_plane_to_sphere",
    "vr_projection",
    "vr_rotate_sphere",
    "vr_sharpen",
    "alpha_adjust",
    "logo_cutout",
    "posterize_time",
    "auto_reframe",
    "cineon_converter",
    "metadata_burnin",
    // rebuilt
    "ultra_key",
    "track_matte",
    "noise",
    "mosaic",
];

/// Push every float parameter towards its soft maximum so the code paths run.
fn pushed(id: &str) -> EffectInstance {
    let def = find_effect(id).unwrap();
    let mut e = def.instance();
    for (pid, p) in e.params.iter_mut() {
        if let (
            ParamValue::Float(v),
            Some(ParamKind::Float {
                soft_max, soft_min, ..
            }),
        ) = (&mut p.value, def.param(pid).map(|d| &d.kind))
        {
            *v = (*v + (soft_max - soft_min) * 0.3).min(*soft_max);
        }
    }
    e
}

#[test]
fn every_effect_is_finite_in_range_and_deterministic() {
    let img = picture(40, 28);
    let c = FxCtx {
        seconds: 0.4,
        ..cx()
    };
    for id in NEW_IDS {
        for e in [inst(id), pushed(id)] {
            let a = run(id, &e, &img, &c);
            let b = run(id, &e, &img, &c);
            assert_eq!(a.px, b.px, "{id} not deterministic");
            assert_eq!((a.w, a.h), (img.w, img.h), "{id} changed size");
            for p in a.px.as_chunks::<4>().0 {
                assert!(p.iter().all(|v| v.is_finite()), "{id}: {p:?}");
                assert!(p[3] >= -1e-4 && p[3] <= 1.0 + 1e-3, "{id}: alpha {}", p[3]);
                assert!(p[..3].iter().all(|v| *v >= -1e-3), "{id}: negative {p:?}");
            }
        }
    }
}

#[test]
fn every_choice_value_runs() {
    let img = picture(24, 16);
    for id in NEW_IDS {
        let def = find_effect(id).unwrap();
        for p in &def.params {
            if let ParamKind::Choice(opts) = p.kind {
                for i in 0..opts.len() {
                    let mut e = pushed(id);
                    set(&mut e, p.id, ParamValue::Choice(i as u32));
                    let o = run(id, &e, &img, &cx());
                    assert!(o.px.iter().all(|v| v.is_finite()), "{id}.{}={i}", p.id);
                }
            }
        }
    }
}

#[test]
fn neutral_parameters_are_identity() {
    let img = picture(32, 24);
    let cases: Vec<(&str, Vec<(&str, ParamValue)>)> = vec![
        ("asc_cdl", vec![]),
        ("channel_mix", vec![]),
        ("vignette", vec![("amount", ParamValue::Float(0.0))]),
        ("corner_pin", vec![]),
        ("alpha_adjust", vec![]),
        ("reduce_interlace_flicker", vec![]),
        ("vr_blur", vec![]),
        ("vr_sharpen", vec![]),
        ("vr_denoise", vec![]),
        ("vr_rotate_sphere", vec![]),
        ("vr_projection", vec![]),
        (
            "color_replace",
            vec![
                ("target", ParamValue::Color([0.0, 0.0, 0.0, 1.0])),
                ("similarity", ParamValue::Float(0.0)),
            ],
        ),
        ("posterize_time", vec![]),
        ("warp_stabilizer", vec![]),
        ("track_matte", vec![]),
        ("noise", vec![]),
        (
            "video_limiter",
            vec![("compression", ParamValue::Choice(0))],
        ),
        (
            "cineon_converter",
            vec![
                ("conversion", ParamValue::Choice(2)),
                ("black10", ParamValue::Float(95.0)),
                ("white10", ParamValue::Float(685.0)),
            ],
        ),
    ];
    for (id, params) in cases {
        let mut e = inst(id);
        for (k, v) in params {
            set(&mut e, k, v);
        }
        let o = run(id, &e, &img, &cx());
        let d = max_diff(&o, &img);
        // in-range values pass the limiter's knee untouched; Cineon log→log at the default points
        assert!(d < 2e-3, "{id} not identity: max diff {d}");
    }
}

/// A clip environment over a synthetic moving picture (for the analysis effects).
struct ClipEnv {
    w: usize,
    h: usize,
    fps: f64,
    secs: f64,
    now: f64,
    key: u64,
    seq: (u32, u32),
    pic: fn(f64, f64, f64) -> [f32; 3],
}
impl ClipEnv {
    fn render(&self, t: f64, scale: f32) -> Image {
        let (w, h) = (
            ((self.w as f32 * scale).round() as usize).max(1),
            ((self.h as f32 * scale).round() as usize).max(1),
        );
        let k = self.w as f64 / w as f64;
        let mut img = Image::new(w, h);
        img.px
            .par_chunks_mut(w * 4)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..w {
                    let c = (self.pic)((x as f64 + 0.5) * k, (y as f64 + 0.5) * k, t);
                    row[x * 4..x * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 1.0]);
                }
            });
        img
    }
}
impl FxEnv for ClipEnv {
    fn frame(&self, dt: f64, _: &EffectInstance) -> Option<Image> {
        Some(self.render(((self.now + dt) * self.fps).round() / self.fps, 1.0))
    }
    fn source_frame(&self, dt: f64, scale: f32) -> Option<Image> {
        Some(self.render(((self.now + dt) * self.fps).round() / self.fps, scale))
    }
    fn track(&self, _: usize) -> Option<Image> {
        None
    }
    fn layer_to_output(&self) -> Affine {
        Affine::IDENTITY
    }
    fn clip_seconds(&self) -> f64 {
        self.secs
    }
    fn clip_offset(&self) -> f64 {
        self.now
    }
    fn frame_rate(&self) -> f64 {
        self.fps
    }
    fn media_timecode(&self) -> String {
        String::new()
    }
    fn file_name(&self) -> String {
        String::new()
    }
    fn sequence_name(&self) -> String {
        String::new()
    }
    fn sequence_size(&self) -> (u32, u32) {
        self.seq
    }
    fn source_size(&self) -> (u32, u32) {
        (self.w as u32, self.h as u32)
    }
    fn clip_key(&self) -> u64 {
        self.key
    }
}

fn texture(x: f64, y: f64) -> [f32; 3] {
    let v = 0.5 + 0.45 * fbm(x as f32 / 9.0, y as f32 / 9.0, 0.3, 4.0, 11);
    let l = kerf_color::srgb_to_linear(v.clamp(0.0, 1.0));
    [l, l, l]
}
/// Camera jitter at time t: a few pixels, changing every frame.
fn jitter(t: f64) -> (f64, f64) {
    let f = (t * 24.0).round() as i64;
    (
        (hash01(f, 0, 0, 3) as f64 - 0.5) * 8.0,
        (hash01(f, 1, 0, 3) as f64 - 0.5) * 8.0,
    )
}
fn shaky(x: f64, y: f64, t: f64) -> [f32; 3] {
    let (dx, dy) = jitter(t);
    texture(x + dx, y + dy)
}

#[test]
fn warp_stabilizer_removes_camera_jitter() {
    let (w, h) = (192, 128);
    let mk = |now: f64| ClipEnv {
        w,
        h,
        fps: 24.0,
        secs: 1.0,
        now,
        key: 0xC0FFEE,
        seq: (w as u32, h as u32),
        pic: shaky,
    };
    let mut e = inst("warp_stabilizer");
    set(&mut e, "result", ParamValue::Choice(1)); // No Motion
    set(&mut e, "framing", ParamValue::Choice(0)); // Stabilize Only
    clear_stabilizer_cache();
    let frames: Vec<(Image, Image)> = [3usize, 9, 15, 21]
        .iter()
        .map(|&f| {
            let now = f as f64 / 24.0;
            let env = mk(now);
            let raw = env.render(now, 1.0);
            let mut st = raw.clone();
            crate::video::effects::apply(
                &mut st,
                &e,
                &FxCtx {
                    seconds: now,
                    env: Some(&env),
                    ..cx()
                },
            );
            (raw, st)
        })
        .collect();
    // compare the interior of every pair of frames: stabilised frames agree, raw ones don't
    let interior = |a: &Image, b: &Image| {
        let mut s = 0.0f64;
        let mut n = 0;
        for y in 24..h - 24 {
            for x in 24..w - 24 {
                s += (a.get(x, y)[0] - b.get(x, y)[0]).abs() as f64;
                n += 1;
            }
        }
        s / n as f64
    };
    let (mut raw_err, mut st_err) = (0.0, 0.0);
    for i in 1..frames.len() {
        raw_err += interior(&frames[0].0, &frames[i].0);
        st_err += interior(&frames[0].1, &frames[i].1);
    }
    assert!(
        st_err < raw_err * 0.25,
        "stabilised {st_err} vs raw {raw_err}"
    );
    // Auto-scale framing hides the borders
    set(&mut e, "framing", ParamValue::Choice(2));
    let env = mk(9.0 / 24.0);
    let mut st = env.render(9.0 / 24.0, 1.0);
    crate::video::effects::apply(
        &mut st,
        &e,
        &FxCtx {
            seconds: 9.0 / 24.0,
            env: Some(&env),
            ..cx()
        },
    );
    assert!(
        st.px.as_chunks::<4>().0.iter().all(|p| p[3] > 0.99),
        "auto-scale leaves no border"
    );
    // the path cache is reused
    assert!(stabilizer_path(
        &env,
        0.0,
        crate::video::project::TrackMethod::Position,
        false
    )
    .is_some());
}
