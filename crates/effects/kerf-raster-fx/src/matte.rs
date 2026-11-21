//! Matte effects: chokers (grey-scale morphology and blur/threshold stages) and edge-aware matte
//! refinement (guided filter, He et al. 2010).

use crate::ParamUi;
use crate::Value;
use rayon::prelude::*;

use crate::util::{morph_frac, set_alpha, Plane};
use crate::{num, p, popup, slider, Buf, EffectCtx, EffectSpec};

fn spec(
    id: &'static str,
    name: &'static str,
    params: Vec<crate::ParamSpec>,
    render: crate::RenderFn,
) -> EffectSpec {
    EffectSpec {
        id,
        name,
        category: "Matte",
        params,
        render,
        gpu: false,
        float: true,
    }
}

fn matte_view(b: &mut Buf) {
    b.img.data.par_iter_mut().for_each(|px| {
        let a = px[3].clamp(0.0, 1.0);
        *px = [a, a, a, 1.0];
    });
}

fn simple_choker(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = ctx.params.f("chokeMatte") * b.scale;
    if c.abs() > 1e-6 {
        if c < 0.0 && !ctx.adjustment {
            b.pad((-c).ceil() as u32 + 1);
        }
        let a = Plane::alpha(&b.img);
        let na = if c > 0.0 {
            morph_frac(&a, c, false)
        } else {
            morph_frac(&a, -c, true)
        };
        set_alpha(&mut b.img, &na);
    }
    if ctx.params.e("view") == 1 {
        matte_view(&mut b);
    }
    b
}

pub fn specs() -> Vec<EffectSpec> {
    let pct0 = || slider(0.0, 100.0, 0.0, 100.0, 0);
    let check = |id: &'static str, name: &'static str, on: bool| {
        p(id, name, Value::Bool(on), ParamUi::Checkbox)
    };
    let decon = |pre: &'static [&'static str; 4]| {
        vec![
            p(pre[0], "Decontamination Amount", num(100.0), pct0()),
            check(pre[1], "Extend Where Smoothed", true),
            p(
                pre[2],
                "Increase Decontamination Radius",
                num(0.0),
                slider(0.0, 100.0, 0.0, 20.0, 1),
            ),
            check(pre[3], "View Decontamination Map", false),
        ]
    };
    // Reduce Chatter and the motion-blur settings.
    let temporal = || {
        vec![
            p("reduceChatter", "Reduce Chatter", num(0.0), pct0()),
            check("useMotionBlur", "Use Motion Blur", false),
            p(
                "motionBlur/motionBlurSamples",
                "Motion Blur Samples",
                num(16.0),
                slider(1.0, 64.0, 1.0, 64.0, 0),
            ),
            p(
                "motionBlur/shutterAngle",
                "Shutter Angle",
                num(180.0),
                slider(0.0, 720.0, 0.0, 360.0, 1),
            ),
            check("motionBlur/higherQuality", "Higher Quality", false),
        ]
    };
    let radius = |d: f64| {
        p(
            "radius",
            "Additional Edge Radius",
            num(d),
            slider(1.0, 200.0, 1.0, 50.0, 1),
        )
    };
    let mut soft_params = vec![
        check("calculateEdgeDetails", "Calculate Edge Details", true),
        radius(10.0),
        check("viewEdgeRegion", "View Edge Region", false),
        p("smooth", "Smooth", num(0.0), pct0()),
        p("feather", "Feather", num(10.0), pct0()),
        p("contrast", "Contrast", num(0.0), pct0()),
        p(
            "shiftEdge",
            "Shift Edge",
            num(0.0),
            slider(-100.0, 100.0, -100.0, 100.0, 0),
        ),
    ];
    soft_params.extend(temporal());
    soft_params.push(check("decontaminate", "Decontaminate Edge Colors", true));
    soft_params.extend(decon(&[
        "decontamination/decontaminationAmount",
        "decontamination/extendWhereSmoothed",
        "decontamination/increaseDecontaminationRadius",
        "decontamination/viewDecontaminationMap",
    ]));
    soft_params.push(check("invert", "Invert", false));
    // Superseded by Shift Edge (opposite sign); kept so older projects render as before.
    soft_params.push(p("choke", "Choke", num(0.0), ParamUi::Hidden));
    let mut hard_params = vec![
        p("smooth", "Smooth", num(20.0), pct0()),
        p("feather", "Feather", num(0.0), pct0()),
        p(
            "choke",
            "Choke",
            num(0.0),
            slider(-100.0, 100.0, -100.0, 100.0, 0),
        ),
    ];
    hard_params.extend(temporal());
    hard_params.push(check("decontaminate", "Decontaminate Edge Colors", true));
    hard_params.extend(decon(&[
        "decontaminationAmount",
        "extendWhereSmoothed",
        "increaseDecontaminationRadius",
        "viewDecontaminationMap",
    ]));
    // Our extras: the analysed edge band's radius, alpha contrast and inversion.
    hard_params.push(radius(3.0));
    hard_params.push(p(
        "alphaContrast",
        "Alpha Contrast",
        num(300.0),
        slider(100.0, 1000.0, 100.0, 500.0, 0),
    ));
    hard_params.push(check("invert", "Invert", false));
    vec![spec(
        "ec.matte.simplechoker",
        "Simple Choker",
        vec![
            p(
                "view",
                "View",
                Value::Enum(0),
                popup(&["Final Output", "Matte"]),
            ),
            p(
                "chokeMatte",
                "Choke Matte",
                num(0.0),
                slider(-100.0, 100.0, -10.0, 10.0, 2),
            ),
        ],
        simple_choker,
    )]
}
