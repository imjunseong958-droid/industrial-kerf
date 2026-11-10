//! Property and golden tests for every transition Kerf's renderer draws, carried from FilmCraft's.
//!
//! Golden fingerprints (`goldens.txt`, FilmCraft's, by FilmCraft's ids): a 4×4 grid of mean 8-bit
//! RGB values at p = 0.25 / 0.5 / 0.75 on two procedural test frames, so the transitions draw here
//! what they drew in FilmCraft. Re-bless after an intended change with
//! `KERF_BLESS=1 cargo test -p kerf-fx transitions::tests::golden_fingerprints` and review the
//! diff.

use kerf_model::{Catalog, FxDef, Param, ParamDef, ParamType, Params, Rgba, Value};

use super::*;

const W: usize = 128;
const H: usize = 72;

/// Outgoing test frame: warm horizontal ramp with a checker overlay.
fn frame_a() -> Image {
    paint(W, H, |x, y| {
        let u = x / W as f32;
        let cell = W as f32 / 8.0;
        let chk = if ((x / cell).floor() as i32 + (y / cell).floor() as i32) % 2 == 0 {
            0.15
        } else {
            0.0
        };
        [
            0.2 + 0.7 * u + chk,
            0.1 + 0.3 * (y / H as f32),
            0.05 + chk,
            1.0,
        ]
    })
}
/// Incoming test frame: cool vertical ramp with a bright disc.
fn frame_b() -> Image {
    paint(W, H, |x, y| {
        let v = y / H as f32;
        let d = ((x - W as f32 * 0.62).powi(2) + (y - H as f32 * 0.4).powi(2)).sqrt();
        let disc = if d < H as f32 * 0.22 { 0.6 } else { 0.0 };
        [0.05 + disc, 0.2 + 0.2 * v + disc, 0.4 + 0.5 * v, 1.0]
    })
}

fn transitions() -> impl Iterator<Item = &'static FxDef> {
    Catalog::builtin()
        .all()
        .filter(|d| filmcraft_id(d.id.as_str()).is_some())
}

fn def(id: &str) -> &'static FxDef {
    Catalog::builtin().fx(id).unwrap_or_else(|| panic!("{id}"))
}

/// `d` at progress `p` with `params` set.
fn draw(d: &FxDef, params: &Params, a: &Image, b: &Image, p: f32) -> Image {
    apply(
        &Fx::new(d, params).expect("Kerf's renderer draws it"),
        a,
        b,
        p,
    )
}

fn number(value: f64) -> Param {
    Param::Static(Value::Number(value))
}

fn mean_abs(a: &Image, b: &Image) -> f32 {
    a.px.iter()
        .zip(&b.px)
        .map(|(x, y)| (x - y).abs())
        .sum::<f32>()
        / a.px.len() as f32
}

#[test]
fn continuous_near_both_ends() {
    let (a, b) = (frame_a(), frame_b());
    for d in transitions() {
        let e = Params::new();
        let near_a = mean_abs(&draw(d, &e, &a, &b, 0.01), &a);
        let near_b = mean_abs(&draw(d, &e, &a, &b, 0.99), &b);
        assert!(
            near_a < 0.06,
            "{}: p=0.01 jumps away from A by {near_a}",
            d.id
        );
        assert!(
            near_b < 0.06,
            "{}: p=0.99 jumps away from B by {near_b}",
            d.id
        );
    }
}

#[test]
fn no_nan_on_degenerate_inputs() {
    let clear = Image::new(W, H);
    let (a, b) = (frame_a(), frame_b());
    let tiny_a = Image::filled(1, 1, [1.0, 0.0, 0.0, 1.0]);
    let tiny_b = Image::filled(1, 1, [0.0, 0.0, 1.0, 1.0]);
    let odd_a = Image::filled(3, 2, [0.5, 0.5, 0.0, 1.0]);
    let odd_b = Image::filled(3, 2, [0.0, 0.5, 0.5, 1.0]);
    for d in transitions() {
        let e = Params::new();
        for p in [f32::NAN, -1.0, 1e-6, 0.25, 0.5, 0.999_999, 2.0] {
            for (x, y) in [
                (&a, &clear),
                (&clear, &b),
                (&tiny_a, &tiny_b),
                (&odd_a, &odd_b),
            ] {
                let m = draw(d, &e, x, y, p);
                assert!(
                    m.px.iter().all(|v| v.is_finite()),
                    "{} p={p} {}x{}",
                    d.id,
                    x.w,
                    x.h
                );
            }
        }
        // a single-sided fade from nothing ends exactly on B
        assert_eq!(draw(d, &e, &clear, &b, 1.0), b);
    }
}

/// A non-default value for a parameter (to check every parameter is wired).
pub(super) fn changed(def: &ParamDef) -> Option<Param> {
    Some(match (&def.ty, &def.default) {
        (ParamType::Number, Value::Number(v)) => {
            let (min, max) = (def.min.unwrap_or(0.0), def.max.unwrap_or(v * 2.0 + 10.0));
            let unit = def.unit.as_deref().unwrap_or_default();
            number(match def.name.as_str() {
                "center_x" => 12.0 / W as f64,
                "center_y" => 9.0 / H as f64,
                "border_width" => 6.0,
                _ if unit == "degrees" => (v + 70.0).min(max),
                // pixel sizes are authored for 1080p; keep them meaningful on the small test frame
                _ if unit == "px" && *v > 20.0 => v * 0.25,
                _ => {
                    let hi = if max > 1e5 { v * 2.0 + 10.0 } else { max };
                    if (hi - v).abs() > (v - min).abs() {
                        v + (hi - v) * 0.5
                    } else {
                        v - (v - min) * 0.5
                    }
                }
            })
        }
        (ParamType::Enum(options), Value::Enum(chosen)) => {
            let at = options
                .iter()
                .position(|option| option == chosen)
                .unwrap_or(0);
            Param::Static(Value::Enum(options[(at + 1) % options.len()].clone()))
        }
        (_, Value::Bool(v)) => Param::Static(Value::Bool(!v)),
        (_, Value::Color(c)) => Param::Static(Value::Color(Rgba::rgb(
            255 - c.r,
            128,
            (f32::from(c.b) * 0.3) as u8,
        ))),
        _ => return None,
    })
}

#[test]
fn every_parameter_changes_the_picture() {
    let (a, b) = (frame_a(), frame_b());
    let mut failures = Vec::new();
    for d in transitions() {
        for pd in &d.params {
            let Some(v) = changed(pd) else { continue };
            let mut base = Params::new();
            if pd.name == "border_color" {
                base.insert("border_width".into(), number(6.0));
            }
            let mut e = base.clone();
            e.insert(pd.name.clone(), v);
            if pd.name == "border_width" {
                // a border only shows if it differs from the content
                e.insert(
                    "border_color".into(),
                    Param::Static(Value::Color(Rgba::rgb(0, 255, 0))),
                );
            }
            let differs = [0.2, 0.35, 0.5, 0.65, 0.8]
                .iter()
                .any(|&p| mean_abs(&draw(d, &base, &a, &b, p), &draw(d, &e, &a, &b, p)) > 1e-4);
            if !differs {
                failures.push(format!("{}.{}", d.id, pd.name));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "parameters with no visible effect: {failures:?}"
    );
}

#[test]
fn directions_differ() {
    let (a, b) = (frame_a(), frame_b());
    for d in transitions().filter(|d| d.find("direction").is_some()) {
        let render = |direction: &str| {
            let e: Params = [(
                "direction".to_owned(),
                Param::Static(Value::Enum(direction.into())),
            )]
            .into();
            draw(d, &e, &a, &b, 0.4)
        };
        assert!(
            mean_abs(&render("from-east"), &render("from-west")) > 1e-3,
            "{}: East vs West identical",
            d.id
        );
    }
}

#[test]
fn push_moves_frames_by_the_eased_distance() {
    let (a, b) = (frame_a(), frame_b());
    let e: Params = [("motion_blur".to_owned(), number(0.0))].into();
    let m = draw(def("push"), &e, &a, &b, 0.5); // eased 0.5 → half a frame, from the west
    for x in [4usize, 20, 28] {
        let want = b.get(x + W / 2, 12);
        let got = m.get(x, 12);
        assert!(
            want.iter().zip(got).all(|(p, q)| (p - q).abs() < 1e-3),
            "x={x}: {got:?} vs {want:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Golden fingerprints

fn fingerprint(img: &Image) -> String {
    let mut s = String::new();
    for by in 0..4 {
        for bx in 0..4 {
            let mut acc = [0.0f32; 3];
            let (x0, x1, y0, y1) = (bx * W / 4, (bx + 1) * W / 4, by * H / 4, (by + 1) * H / 4);
            for y in y0..y1 {
                for x in x0..x1 {
                    let c = img.get(x, y);
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                }
            }
            let n = ((x1 - x0) * (y1 - y0)) as f32;
            for v in acc {
                s.push_str(&format!(
                    "{:02x}",
                    ((v / n).clamp(0.0, 1.0) * 255.0).round() as u8
                ));
            }
        }
    }
    s
}

fn golden_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/transitions/goldens.txt")
}

#[test]
fn golden_fingerprints() {
    let (a, b) = (frame_a(), frame_b());
    let mut lines = Vec::new();
    for d in transitions() {
        let none = Params::new();
        let e = Fx::new(d, &none).expect("Kerf's renderer draws it");
        for p in [0.25f32, 0.5, 0.75] {
            lines.push(format!(
                "{} {p:.2} {}",
                e.id,
                fingerprint(&apply(&e, &a, &b, p))
            ));
        }
    }
    let path = golden_path();
    if std::env::var_os("KERF_BLESS").is_some() {
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("goldens.txt (bless with KERF_BLESS=1)");
    let want: std::collections::HashMap<(String, String), String> = want
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((
                (it.next()?.to_string(), it.next()?.to_string()),
                it.next()?.to_string(),
            ))
        })
        .collect();
    let mut bad = Vec::new();
    for l in &lines {
        let mut it = l.split_whitespace();
        let (id, p, got) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        let Some(exp) = want.get(&(id.to_string(), p.to_string())) else {
            bad.push(format!("{id} {p}: no golden"));
            continue;
        };
        let bytes = |s: &str| {
            (0..s.len() / 2)
                .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap_or(0) as i32)
                .collect::<Vec<_>>()
        };
        let (g, e) = (bytes(got), bytes(exp));
        let worst = g
            .iter()
            .zip(&e)
            .map(|(x, y)| (x - y).abs())
            .max()
            .unwrap_or(255);
        if g.len() != e.len() || worst > 4 {
            bad.push(format!("{id} {p}: max Δ {worst}"));
        }
    }
    assert!(bad.is_empty(), "transition goldens differ: {bad:?}");
}

/// The disc blur as it was before #1871: offsets worked out for every pixel.
fn disc_blur_per_pixel(img: &Image, x: f32, y: f32, r: f32, n: usize) -> [f32; 4] {
    if r < 0.5 || n <= 1 {
        return img.sample_bilinear_clamped(x, y);
    }
    let mut acc = [0.0; 4];
    for i in 0..n {
        let rr = r * ((i as f32 + 0.5) / n as f32).sqrt();
        let a = i as f32 * 2.399_963;
        let p = img.sample_bilinear_clamped(x + rr * a.cos(), y + rr * a.sin());
        for k in 0..4 {
            acc[k] += p[k];
        }
    }
    scale4(acc, 1.0 / n as f32)
}

/// #1871: offsets worked out once per frame give exactly the pixels the per-pixel version gave.
#[test]
fn disc_blur_with_shared_offsets_is_bit_identical() {
    let img = frame_b();
    for (r, n) in [
        (0.0, 24),
        (0.4, 24),
        (3.0, 8),
        (30.0, 24),
        (75.5, 48),
        (200.0, 48),
        (12.0, 1),
    ] {
        let offsets = disc_offsets(r, n);
        for y in (0..H).step_by(7) {
            for x in (0..W).step_by(5) {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let a = disc_blur_per_pixel(&img, fx, fy, r, n);
                let b = disc_blur_with(&img, fx, fy, &offsets);
                assert_eq!(
                    a.map(f32::to_bits),
                    b.map(f32::to_bits),
                    "r {r}, n {n}, at ({x}, {y})"
                );
            }
        }
    }
}
