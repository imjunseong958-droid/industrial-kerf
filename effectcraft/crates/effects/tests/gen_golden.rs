//! Golden hashes of the CPU effects that draw through a shared per-frame plan (M13.28).
//!
//! The GPU compositor rasterises these effects from geometry the CPU computes (bolts, strokes,
//! audio marks, wave lists, flood-fill regions, sampled colours); the CPU effects were
//! refactored to build the same plans. The hashes guard against unintended output changes;
//! intended behavior changes require a focused regression test before re-pinning. Each effect
//! renders on a generated layer with masks, another layer and an audio track, at default
//! and non-default parameters and two layer times, in an 8 bpc pipeline (input and output
//! clamped and quantised) and a 32 bpc one, at full and at half resolution. Each frame's pixels
//! are hashed (FNV-1a over the `f32` bits) and compared with the pinned hashes below.
//!
//! The hashes are exact `f32` bit patterns, so they depend on the platform's libm; they are
//! pinned for aarch64 macOS (where the gates run). Elsewhere the test only checks that every
//! case renders deterministically.
//!
//! `GEN_GOLDEN_PRINT=1` prints the table (to re-pin after an intended change).

use effectcraft_effects::{Buf, EffectCtx, EffectEnv, EffectHost, LayerPixels, MaskShape, Params, apply, default_value, find};
use effectcraft_effects::Value;
use effectcraft_raster::Image;

const W: u32 = 64;
const H: u32 = 48;
/// Layer id served by [`Host`]: a radial luminance map (also the audio layer).
const MAP: u64 = 8;

/// The effect's own layer: a colour ramp with a clear hole and a half-transparent band.
fn layer(hdr: bool) -> Image {
    let mut img = Image::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let (u, v) = (x as f32 / W as f32, y as f32 / H as f32);
            let (dx, dy) = (x as f32 - 40.0, y as f32 - 20.0);
            let a = if dx * dx + dy * dy < 64.0 {
                0.0
            } else if (30..36).contains(&y) {
                0.45
            } else {
                1.0
            };
            let gain = if hdr && x < 20 { 1.6 } else { 1.0 };
            let c = [u * gain, v * gain, (0.3 + 0.4 * (u * 9.0).sin().abs()) * gain];
            img.set(x, y, [c[0] * a, c[1] * a, c[2] * a, a]);
        }
    }
    img
}

fn map() -> Image {
    let mut img = Image::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let d = ((x as f32 - 30.0).powi(2) + (y as f32 - 22.0).powi(2)).sqrt() / 30.0;
            let l = (1.0 - d).clamp(0.0, 1.0);
            img.set(x, y, [l, l * 0.8, l * 0.5, 1.0]);
        }
    }
    img
}

struct Host;
impl EffectHost for Host {
    fn layer(&self, _: u64, _: bool) -> Option<LayerPixels> {
        Some(LayerPixels { buf: Buf { img: map(), offset: [0.0; 2], scale: 1.0 }, size: [W as f64, H as f64] })
    }
    fn audio(&self, _: u64, start: f64, frames: usize, rate: u32) -> Option<Vec<f32>> {
        // Two tones plus a chirp, stereo interleaved.
        Some(
            (0..frames)
                .flat_map(|i| {
                    let t = start + i as f64 / rate as f64;
                    let l = 0.5 * (t * 440.0 * std::f64::consts::TAU).sin() + 0.2 * (t * 1300.0 * std::f64::consts::TAU).sin();
                    let r = 0.4 * (t * (200.0 + 300.0 * t) * std::f64::consts::TAU).sin();
                    [l as f32, r as f32]
                })
                .collect(),
        )
    }
}

/// The layer's masks (layer pixels): a closed pentagon, a closed quad and an open curve.
fn masks() -> Vec<MaskShape> {
    let pent = (0..5)
        .map(|i| {
            let a = i as f64 * std::f64::consts::TAU / 5.0 - 1.3;
            [30.0 + a.cos() * 17.0, 24.0 + a.sin() * 15.0]
        })
        .collect();
    vec![
        MaskShape { name: "Mask 1".into(), points: pent, closed: true, inverted: false },
        MaskShape { name: "Mask 2".into(), points: vec![[6.3, 5.1], [58.2, 7.7], [55.4, 43.6], [9.1, 40.2]], closed: true, inverted: false },
        MaskShape { name: "Mask 3".into(), points: vec![[4.2, 30.3], [18.7, 12.4], [33.1, 28.9], [47.6, 9.8], [60.2, 25.5]], closed: false, inverted: false },
    ]
}

fn n(v: f64) -> Value {
    Value::Scalar(v)
}
fn e(v: u32) -> Value {
    Value::Enum(v)
}
fn b(v: bool) -> Value {
    Value::Bool(v)
}

type Set = Vec<(&'static str, Value)>;

/// (effect id, parameter sets) — the first set of each is the defaults.
fn cases() -> Vec<(&'static str, Vec<Set>)> {
    vec![
        (
            "ec.generate.audiospectrum",
            vec![
                vec![("audioLayer", Value::Layer(Some(MAP)))],
                vec![("audioLayer", Value::Layer(Some(MAP))), ("displayOptions", e(1)), ("sideOptions", e(2)), ("compositeOnOriginal", b(true))],
                vec![("audioLayer", Value::Layer(Some(MAP))), ("displayOptions", e(2)), ("path", n(3.0)), ("usePolarPath", b(false))],
            ],
        ),
        (
            "ec.generate.audiowaveform",
            vec![
                vec![("audioLayer", Value::Layer(Some(MAP)))],
                vec![("audioLayer", Value::Layer(Some(MAP))), ("displayOptions", e(0)), ("waveformOptions", e(2)), ("usePolarPath", b(true))],
                vec![("audioLayer", Value::Layer(Some(MAP))), ("displayOptions", e(2)), ("path", n(1.0)), ("compositeOnOriginal", b(true))],
            ],
        ),
    ]
}

/// Layer times.
const TIMES: [f64; 2] = [0.4, 1.3];

fn fnv(img: &Image) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in [img.width, img.height].iter().flat_map(|v| v.to_le_bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    for p in &img.data {
        for c in p {
            for b in c.to_bits().to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    h
}

fn render(id: &str, set: &Set, time: f64, bpc8: bool, half: bool) -> Image {
    let spec = find(id).unwrap_or_else(|| panic!("no effect {id}"));
    let size = [W as f64, H as f64];
    let mut params = Params { values: spec.params.iter().map(|p| (p.id.to_string(), default_value(p, size))).collect() };
    for (k, v) in set {
        assert!(params.values.contains_key(*k), "{id}: unknown param {k}");
        params.values.insert(k.to_string(), v.clone());
    }
    let mut img = layer(!bpc8);
    if bpc8 {
        img.clamp01();
        img.quantize(255.0);
    }
    let mut buf = Buf { img, offset: [0.0; 2], scale: 1.0 };
    if half {
        // A half-resolution preview buffer padded by 3 px.
        let (w2, h2) = (W / 2, H / 2);
        let mut small = Image::new(w2, h2);
        for y in 0..h2 {
            for x in 0..w2 {
                small.set(x, y, buf.img.sample_bilinear(x as f64 * 2.0 + 1.0, y as f64 * 2.0 + 1.0));
            }
        }
        buf = Buf { img: small, offset: [0.0; 2], scale: 0.5 };
        buf.pad(3);
    }
    let host = Host;
    let masks = masks();
    let env = EffectEnv { host: Some(&host), masks: &masks, frame_rate: 30.0, comp_time: time, ..Default::default() };
    let ctx = EffectCtx { params: &params, time, layer_size: size, seed: 1, adjustment: false, env };
    let mut out = apply(spec, &ctx, buf).img;
    if bpc8 {
        out.clamp01();
        out.quantize(255.0);
    }
    out
}

/// Every case's name and hash, in a fixed order.
fn hashes() -> Vec<(String, u64)> {
    let mut v = Vec::new();
    for (id, sets) in cases() {
        for (si, set) in sets.iter().enumerate() {
            for (bpc8, half) in [(true, false), (false, false), (false, true)] {
                for t in TIMES {
                    let img = render(id, set, t, bpc8, half);
                    let name = format!("{id}/{si}/{}{}/t{t}", if bpc8 { "8bpc" } else { "32bpc" }, if half { "/half" } else { "" });
                    v.push((name, fnv(&img)));
                }
            }
        }
    }
    v
}

/// Pin the intended linear spectrum behavior independently of the pixel hash table.

#[test]
fn plan_effects_golden_hashes() {
    let got = hashes();
    if std::env::var_os("GEN_GOLDEN_PRINT").is_some() {
        for (name, h) in &got {
            println!("    (\"{name}\", 0x{h:016x}),");
        }
    }
    assert_eq!(got, hashes(), "the effects render deterministically");
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        let bad: Vec<String> = got
            .iter()
            .zip(GOLDEN)
            .filter(|((n, h), (gn, gh))| n != gn || h != gh)
            .map(|((n, h), (_, gh))| format!("{n}: 0x{h:016x} (pinned 0x{gh:016x})"))
            .collect();
        assert_eq!(got.len(), GOLDEN.len(), "case count");
        assert!(bad.is_empty(), "CPU output changed:\n{}", bad.join("\n"));
    }
}

/// Pinned hashes (aarch64 macOS).
const GOLDEN: &[(&str, u64)] = &[
    // Linear frequency bands and amplitude from 581326ec; all other effects retain their hashes.
    ("ec.generate.audiospectrum/0/8bpc/t0.4", 0x07be49f76890493c),
    ("ec.generate.audiospectrum/0/8bpc/t1.3", 0xc0109e19c3958517),
    ("ec.generate.audiospectrum/0/32bpc/t0.4", 0x24fe2e48cc432c1f),
    ("ec.generate.audiospectrum/0/32bpc/t1.3", 0x9bdbf835e7d42f78),
    ("ec.generate.audiospectrum/0/32bpc/half/t0.4", 0x445e16a81e313a40),
    ("ec.generate.audiospectrum/0/32bpc/half/t1.3", 0x88849fde30e6b048),
    ("ec.generate.audiospectrum/1/8bpc/t0.4", 0xf95a769e610688cc),
    ("ec.generate.audiospectrum/1/8bpc/t1.3", 0x7df3a63a59a10c38),
    ("ec.generate.audiospectrum/1/32bpc/t0.4", 0x53f3e442e240599b),
    ("ec.generate.audiospectrum/1/32bpc/t1.3", 0x213066492a32f6a8),
    ("ec.generate.audiospectrum/1/32bpc/half/t0.4", 0x38ceb79d523c7402),
    ("ec.generate.audiospectrum/1/32bpc/half/t1.3", 0x7d63c37d260eb112),
    ("ec.generate.audiospectrum/2/8bpc/t0.4", 0x73c15c41ad557cec),
    ("ec.generate.audiospectrum/2/8bpc/t1.3", 0x0bb38170a3fbb6fd),
    ("ec.generate.audiospectrum/2/32bpc/t0.4", 0x2ac65e1ca97db691),
    ("ec.generate.audiospectrum/2/32bpc/t1.3", 0xa8be6d0571415ab0),
    ("ec.generate.audiospectrum/2/32bpc/half/t0.4", 0x318b82f955d58300),
    ("ec.generate.audiospectrum/2/32bpc/half/t1.3", 0x4775b23a8cc7c6c6),
    ("ec.generate.audiowaveform/0/8bpc/t0.4", 0x68a7733a023bc9f1),
    ("ec.generate.audiowaveform/0/8bpc/t1.3", 0x57e13a1aaa458b14),
    ("ec.generate.audiowaveform/0/32bpc/t0.4", 0xb78e946d794b5ade),
    ("ec.generate.audiowaveform/0/32bpc/t1.3", 0x2acdbc2a2558c847),
    ("ec.generate.audiowaveform/0/32bpc/half/t0.4", 0x99da8b92d697e171),
    ("ec.generate.audiowaveform/0/32bpc/half/t1.3", 0xdfe01c4d3368ea92),
    ("ec.generate.audiowaveform/1/8bpc/t0.4", 0x5f463552fd80eda6),
    ("ec.generate.audiowaveform/1/8bpc/t1.3", 0x4e724e55141cb0e2),
    ("ec.generate.audiowaveform/1/32bpc/t0.4", 0xcba797b9142cc9bc),
    ("ec.generate.audiowaveform/1/32bpc/t1.3", 0x1efc192f83ff114f),
    ("ec.generate.audiowaveform/1/32bpc/half/t0.4", 0x5f81670776ddb569),
    ("ec.generate.audiowaveform/1/32bpc/half/t1.3", 0xf56e5d8f8445125f),
    ("ec.generate.audiowaveform/2/8bpc/t0.4", 0xb9fb312b8553ee47),
    ("ec.generate.audiowaveform/2/8bpc/t1.3", 0x08640b88a8496eb1),
    ("ec.generate.audiowaveform/2/32bpc/t0.4", 0x12a7502643511c69),
    ("ec.generate.audiowaveform/2/32bpc/t1.3", 0xc5f3b7bc698f6328),
    ("ec.generate.audiowaveform/2/32bpc/half/t0.4", 0x2409e86ae0f5e841),
    ("ec.generate.audiowaveform/2/32bpc/half/t1.3", 0x5518084e651af2c6),
];
