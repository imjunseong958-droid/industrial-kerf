//! The transitions' kernels against the CPU, which is the reference, value by value.

use super::gpu::{available, Kernel};
use super::tests::changed;
use super::*;
use kerf_model::{Catalog, Params};

const W: usize = 320;
const H: usize = 180;

fn gpu() -> Option<&'static kerf_gpu::Gpu> {
    kerf_gpu::shared().filter(|gpu| available(gpu))
}

/// Outgoing: ramps, a checker with hard edges, values over 1, and a band with partial alpha.
fn frame_a(w: usize, h: usize) -> Image {
    paint(w, h, |x, y| {
        let (u, v) = (x / w as f32, y / h as f32);
        let cell = w as f32 / 8.0;
        let chk = if ((x / cell).floor() as i32 + (y / cell).floor() as i32) % 2 == 0 {
            0.15
        } else {
            0.0
        };
        let a = if (0.45..0.55).contains(&v) { 0.5 } else { 1.0 };
        [
            (0.2 + 1.1 * u + chk) * a,
            (0.1 + 0.3 * v) * a,
            (0.05 + chk) * a,
            a,
        ]
    })
}

/// Incoming: a cool ramp with a bright disc, and transparent around a clip smaller than the frame.
fn frame_b(w: usize, h: usize) -> Image {
    paint(w, h, |x, y| {
        let (u, v) = (x / w as f32, y / h as f32);
        if !(0.03..0.97).contains(&u) {
            return [0.0; 4];
        }
        let d = ((x - w as f32 * 0.62).powi(2) + (y - h as f32 * 0.4).powi(2)).sqrt();
        let disc = if d < h as f32 * 0.22 { 0.6 } else { 0.0 };
        [0.05 + disc, 0.2 + 0.2 * v + disc, 0.4 + 0.5 * v, 1.0]
    })
}

/// How far `gpu` is from `cpu`: the worst value, and the share of values off by more than `by`.
fn apart(cpu: &[f32], gpu: &[f32], by: f32) -> (f32, f64) {
    assert_eq!(cpu.len(), gpu.len());
    let (mut worst, mut off) = (0f32, 0usize);
    for (cpu, gpu) in cpu.iter().zip(gpu) {
        assert!(gpu.is_finite(), "not a number from the kernel");
        let d = (cpu - gpu).abs() / (1.0 + cpu.abs());
        worst = worst.max(d);
        off += usize::from(d > by);
    }
    (worst, off as f64 / cpu.len() as f64)
}

fn on_gpu(gpu: &'static kerf_gpu::Gpu, kernel: &Kernel, a: &Image, b: &Image) -> Vec<f32> {
    let mut frame = gpu.frame();
    let (a, b) = (frame.image(a), frame.image(b));
    let out = kernel.run(&mut frame, &a, &b).expect("the kernels build");
    frame.read_floats(&out).expect("read back")
}

/// `def`'s parameters as they come, and with each of them changed in turn.
fn parameter_sets(def: &kerf_model::FxDef) -> Vec<Params> {
    let mut sets = vec![Params::new()];
    for param in &def.params {
        if let Some(value) = changed(param) {
            let mut params = Params::new();
            params.insert(param.name.clone(), value);
            sets.push(params);
        }
    }
    sets
}

fn transitions() -> impl Iterator<Item = &'static kerf_model::FxDef> {
    Catalog::builtin()
        .all()
        .filter(|def| filmcraft_id(def.id.as_str()).is_some())
}

/// Every value of every transition with a kernel, at each size and parameter a test frame can show.
/// Off by [`EXACT`] at most, as `kerf_fx`'s other kernels are held. A threshold landing on a value may
/// flip, which it does where a sprocket hole of Film Roll ends exactly at a pixel of this size; at most
/// [`FLIPS`] of the values may be further off.
#[test]
fn each_kernel_draws_what_the_cpu_draws() {
    const EXACT: f32 = 5e-4;
    const FLIPS: f64 = 0.005;
    let Some(gpu) = gpu() else {
        return;
    };
    let mut failures = Vec::new();
    let mut ran = 0;
    // Each parameter at one size, and as they come at half the size, and on its side.
    for (w, h, scale, every) in [
        (W, H, 1.0, true),
        (W / 2, H / 2, 0.5, false),
        (H, W, 1.0, false),
    ] {
        let (a, b) = (frame_a(w, h), frame_b(w, h));
        for def in transitions() {
            let sets = if every {
                parameter_sets(def)
            } else {
                vec![Params::new()]
            };
            for params in sets {
                let fx = Fx::new(def, &params).expect("drawn");
                for p in [0.13, 0.5, 0.71] {
                    let Some(kernel) = Kernel::of(&fx, (w, h), p, scale) else {
                        continue;
                    };
                    let cpu = apply_scaled(&fx, &a, &b, p, scale);
                    let found = on_gpu(gpu, &kernel, &a, &b);
                    let (worst, off) = apart(&cpu.px, &found, EXACT);
                    ran += 1;
                    if off > FLIPS {
                        failures.push(format!(
                            "{} {params:?} at {p} on {w}x{h}: worst {worst}, {:.3}% off",
                            def.id,
                            off * 100.0
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(ran > 1000, "only {ran} ran");
    // `read_floats` gives nothing back from a device that failed, which would stop the test above; and
    // one that fails at the very end should not pass.
    assert!(kerf_gpu::shared().is_some(), "the device failed");
}

/// Which have kernels and which do not, by name, so that neither list can drift.
#[test]
fn which_transitions_have_kernels() {
    const KERNELS: &[&str] = &[
        "additive-dissolve",
        "barn-doors",
        "block-motion",
        "blur-dissolve",
        "burn-alpha",
        "burn-chroma",
        "center-split",
        "chaos",
        "chroma-leak",
        "clock-wipe",
        "cross-zoom",
        "directional-blur",
        "earthquake",
        "film-dissolve",
        "film-roll",
        "flare",
        "flash",
        "flicker",
        "flip-motion",
        "fold-motion",
        "frame",
        "glass",
        "glitch",
        "glow",
        "grunge",
        "inset",
        "iris-box",
        "iris-cross",
        "iris-diamond",
        "iris-round",
        "kaleidoscope",
        "lens-blur",
        "light-leak",
        "light-sweep",
        "liquid-distortion",
        "louver",
        "luma-fade",
        "mirror",
        "mosaic",
        "neon-wipe",
        "page-peel",
        "panel-wipe",
        "phosphor",
        "plateau-wipe",
        "pop-motion",
        "pull-motion",
        "push",
        "radial-blur",
        "radial-wipe",
        "ray",
        "roll",
        "roll-3d",
        "slice",
        "slide",
        "soft-wipe",
        "solarize",
        "spin-3d",
        "spin-motion",
        "spinback-3d",
        "split",
        "spring-motion",
        "star-wipe",
        "stretch",
        "stretch-wipe",
        "stripe",
        "travel-motion",
        "tv-power",
        "vhs-damage",
        "wave",
        "whip",
        "zoom-blur",
    ];
    const ON_THE_CPU: &[&str] = &["morph-cut"];
    let params = Params::new();
    let (mut with, mut without) = (Vec::new(), Vec::new());
    for def in transitions() {
        let fx = Fx::new(def, &params).expect("drawn");
        if Kernel::of(&fx, (64, 36), 0.5, 1.0).is_some() {
            with.push(def.id.as_str());
        } else {
            without.push(def.id.as_str());
        }
    }
    with.sort_unstable();
    without.sort_unstable();
    assert_eq!(with, KERNELS);
    assert_eq!(without, ON_THE_CPU);
    assert_eq!(with.len() + without.len(), 72);
}

/// Its ends are the two clips, as on the CPU, with no arithmetic that could change a byte.
#[test]
fn a_transition_at_its_ends_is_its_clips() {
    let Some(gpu) = gpu() else {
        return;
    };
    let (a, b) = (frame_a(W, H), frame_b(W, H));
    let params = Params::new();
    let def = Catalog::builtin().fx("glass").expect("in the catalog");
    let fx = Fx::new(def, &params).expect("drawn");
    let start = Kernel::of(&fx, (W, H), 0.0, 1.0).expect("a kernel");
    assert_eq!(on_gpu(gpu, &start, &a, &b), a.px);
    let end = Kernel::of(&fx, (W, H), 1.0, 1.0).expect("a kernel");
    assert_eq!(on_gpu(gpu, &end, &a, &b), b.px);
}
