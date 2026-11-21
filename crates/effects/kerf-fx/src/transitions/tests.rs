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
