//! Keying effects, batch 2: Difference Matte (against another layer), Inner/Outer Key (mattes
//! from masks with colour-based edge estimation and decontamination), Unmult and CC Simple
//! Wire Removal.

use crate::ParamUi;
use crate::Value;
use rayon::prelude::*;

use crate::util::{premul, unpremul};
use crate::{Buf, EffectCtx, EffectSpec, num, p, popup, slider};

fn spec(id: &'static str, name: &'static str, params: Vec<crate::ParamSpec>, render: crate::RenderFn) -> EffectSpec {
    EffectSpec { id, name, category: "Keying", params, render, gpu: false, float: true }
}

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

// ---------------------------------------------------------------- Difference Matte

// ---------------------------------------------------------------- Inner/Outer Key

// ---------------------------------------------------------------- Unmult

/// Turns a black (or white) background into transparency. Alpha ramps up from the level (Black
/// Level: on the brightest channel; White Level: on the darkest channel, mirrored) over a
/// transition whose width is Softness (0 % = hard cutoff at the level, 100 % = the whole
/// remaining range, which with the default levels is the classic unmultiply: alpha = brightest
/// channel). Remove Color Matting takes the background colour back out of semi-transparent
/// pixels; Clip HDR Results keeps the recovered colours in 0..1.
fn unmult(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let pr = ctx.params;
    let white = pr.e("backgroundColor") == 1;
    let level = if white { pr.f("whiteLevel") } else { pr.f("blackLevel") }.clamp(0.0, 100.0) as f32 / 100.0;
    let soft = pr.f("softness").clamp(0.0, 100.0) as f32 / 100.0;
    let decon = pr.b("removeColorMatting");
    let clip = pr.b("clipHdrResults");
    b.img.data.par_iter_mut().for_each(|px| {
        let (c, a0) = unpremul(*px);
        // Distance from the background colour, and where the ramp starts on that scale.
        let (d, start) = if white { (1.0 - c[0].min(c[1]).min(c[2]), 1.0 - level) } else { (c[0].max(c[1]).max(c[2]), level) };
        let d = d.clamp(0.0, 1.0);
        let width = soft * (1.0 - start);
        let m = if width > 1e-6 {
            ((d - start) / width).clamp(0.0, 1.0)
        } else if d > start {
            1.0
        } else {
            0.0
        };
        if m <= 1e-6 {
            *px = [0.0; 4];
            return;
        }
        let bg = if white { 1.0 } else { 0.0 };
        let mut out = if decon { c.map(|v| (v - bg * (1.0 - m)) / m) } else { c };
        if clip {
            out = out.map(|v| v.clamp(0.0, 1.0));
        }
        *px = premul(out, a0 * m);
    });
    b
}

// ---------------------------------------------------------------- CC Simple Wire Removal

pub fn specs() -> Vec<EffectSpec> {
    vec![spec(
        "ec.key.unmult",
        "Unmult",
        vec![
            p("backgroundColor", "Background Color", Value::Enum(0), popup(&["Black", "White"])),
            p("blackLevel", "Black Level", num(0.0), pct()),
            p("whiteLevel", "White Level", num(100.0), pct()),
            p("softness", "Softness", num(100.0), pct()),
            p("removeColorMatting", "Remove Color Matting", Value::Bool(true), ParamUi::Checkbox),
            p("clipHdrResults", "Clip HDR Results", Value::Bool(true), ParamUi::Checkbox),
        ],
        unmult,
    )]
}
