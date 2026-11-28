//! Tests for `kerf-kernel`'s SSA graph IR, color-gauge cancellation, forward-mode AD (`Dual`),
//! and multi-op pointwise fusion compiler.

use kerf_kernel::dual::{derivative, gradient, jacobian, Dual};
use kerf_kernel::fusion::{compile_fused, eval_fused, OpKind, PixelOp};
use kerf_kernel::ir::Graph;
use kerf_kernel::{
    cineon_offset, cineon_px, color_balance, photo_filter, unmult, wipe, Balance, Cineon, Num,
    PhotoFilter, Unmult, WipeEdge,
};

#[test]
fn ssa_graph_simplifies_algebra_and_cancels_color_gauge_round_trips() {
    let kinds = [
        OpKind::BrightnessContrast,
        OpKind::Gamma,
        OpKind::Levels,
        OpKind::AscCdl,
        OpKind::ColorBalance,
    ];
    let compiled = compile_fused(&kinds);
    // 5 sRGB-space ops have 4 adjacent boundaries × 3 channels = 12 redundant sRGB↔linear
    // conversions that MUST cancel in the SSA graph!
    assert!(
        compiled.stats.gauge_cancellations >= 12,
        "expected >= 12 color-gauge cancellations, got {:?}",
        compiled.stats
    );
    // Only the initial entry `lin_to_srgb` (3 channels) and final exit `srgb_to_lin` (3 channels)
    // should remain in the emitted WGSL body (excluding the helper function definitions).
    let fn_start = compiled
        .wgsl
        .find("fn fused(")
        .expect("fused entry point present");
    let body = &compiled.wgsl[fn_start..];
    assert_eq!(
        body.matches("lin_to_srgb(").count(),
        3,
        "expected exactly 3 lin_to_srgb calls in fused body:\n{body}"
    );
    assert_eq!(
        body.matches("srgb_to_lin(").count(),
        3,
        "expected exactly 3 srgb_to_lin calls in fused body:\n{body}"
    );
    assert!(
        compiled.stats.cse_hits > 0 && compiled.stats.algebraic_rewrites > 0,
        "expected CSE and algebraic rewrites, got {:?}",
        compiled.stats
    );

    // Dead-code elimination & algebraic identities on a direct graph.
    let (graph, roots) = Graph::trace(|| {
        let x = Graph::input("x");
        let _dead = (x + Num::lit(7.0)) * Num::lit(9.0);
        let redundant = ((x + Num::lit(0.0)) * Num::lit(1.0) - Num::lit(0.0)).pow(1.0);
        let zero = x - x;
        let round_trip = redundant
            .max(Num::lit(0.0))
            .srgb_to_linear()
            .linear_to_srgb();
        vec![round_trip + zero]
    });
    let (wgsl_body, exprs, stats) = graph.emit_wgsl_body(&roots, "");
    assert_eq!(stats.gauge_cancellations, 1);
    assert_eq!(
        stats.live_nodes, 1,
        "only max(x, 0.0) should stay live:\n{wgsl_body}"
    );
    assert!(
        wgsl_body.contains(&format!("let {}: f32 = max(x, 0.0);", exprs[0]))
            || wgsl_body.contains(&format!("let {}: f32 = max(0.0, x);", exprs[0])),
        "unexpected simplified WGSL:\n{wgsl_body}"
    );
}

#[test]
fn dual_ad_matches_central_finite_differences_across_kernels() {
    let eps = 1e-3f32;
    let check_scalar = |name: &str, f: &dyn Fn(Dual) -> Dual, g: &dyn Fn(f32) -> f32, x: f32| {
        let (val, deriv) = derivative(f, x);
        let fd = (g(x + eps) - g(x - eps)) / (2.0 * eps);
        assert!(
            (val - g(x)).abs() < 1e-5,
            "{name} value mismatch: {val} vs {}",
            g(x)
        );
        assert!(
            (deriv - fd).abs() < 2e-2 * (1.0 + fd.abs()),
            "{name} derivative mismatch at {x}: dual={deriv} fd={fd}"
        );
    };

    // Core transcendental & transfer ops.
    check_scalar(
        "srgb_to_linear",
        &|x: Dual| x.srgb_to_linear(),
        &|x: f32| x.srgb_to_linear(),
        0.37,
    );
    check_scalar(
        "linear_to_srgb",
        &|x: Dual| x.linear_to_srgb(),
        &|x: f32| x.linear_to_srgb(),
        0.21,
    );
    check_scalar(
        "powf_exp_log10_hypot",
        &|x: Dual| (x.powf(x + Dual::lit(0.5)) + x.exp().log10()).hypot(x.sqrt()),
        &|x: f32| (x.powf(x + 0.5) + x.exp().log10()).hypot(x.sqrt()),
        0.85,
    );

    // Gradient of color_balance w.r.t. RGB pixel channels.
    let balance_f = Balance {
        shadows: [0.08, -0.04, 0.05],
        middles: [-0.03, 0.06, -0.02],
        highlights: [0.04, 0.02, -0.07],
        keep: 1.0,
    };
    let balance_d = Balance {
        shadows: balance_f.shadows.map(Dual::constant),
        middles: balance_f.middles.map(Dual::constant),
        highlights: balance_f.highlights.map(Dual::constant),
        keep: Dual::constant(1.0),
    };
    let at = [0.28f32, 0.42, 0.19];
    let (vals, jac) = jacobian(
        |[r, g, b]| {
            let out = color_balance([r, g, b, Dual::constant(0.85)], &balance_d);
            [out[0], out[1], out[2]]
        },
        at,
    );
    let ref_out = color_balance([at[0], at[1], at[2], 0.85], &balance_f);
    for c in 0..3 {
        assert!((vals[c] - ref_out[c]).abs() < 1e-6);
    }
    for col in 0..3 {
        let mut p_plus = at;
        let mut p_minus = at;
        p_plus[col] += eps;
        p_minus[col] -= eps;
        let o_plus = color_balance([p_plus[0], p_plus[1], p_plus[2], 0.85], &balance_f);
        let o_minus = color_balance([p_minus[0], p_minus[1], p_minus[2], 0.85], &balance_f);
        for row in 0..3 {
            let fd = (o_plus[row] - o_minus[row]) / (2.0 * eps);
            assert!(
                (jac[row][col] - fd).abs() < 5e-3,
                "color_balance J[{row}][{col}]: dual={} fd={fd}",
                jac[row][col]
            );
        }
    }

    // Gradient through a multi-op fused grading chain!
    let ops = [
        PixelOp::BrightnessContrast { br: 0.05, co: 1.15 },
        PixelOp::Gamma { g: 1.2 },
        PixelOp::Levels {
            ib: 0.05,
            iw: 0.95,
            ob: 0.02,
            ow: 0.98,
            g: 0.9,
        },
        PixelOp::AscCdl {
            slope: [1.05, 0.98, 1.02],
            offset: [0.01, -0.01, 0.02],
            power: [1.1, 0.95, 1.05],
            sat: 1.1,
        },
    ];
    let loss = |[r, g, b]: [Dual; 3]| -> Dual {
        let out = eval_fused([r, g, b, Dual::constant(1.0)], &ops);
        let target = [Dual::lit(0.5), Dual::lit(0.4), Dual::lit(0.3)];
        (out[0] - target[0]).pow(2.0)
            + (out[1] - target[1]).pow(2.0)
            + (out[2] - target[2]).pow(2.0)
    };
    let loss_f = |c: [f32; 3]| -> f32 {
        let out = eval_fused([c[0], c[1], c[2], 1.0], &ops);
        (out[0] - 0.5).powi(2) + (out[1] - 0.4).powi(2) + (out[2] - 0.3).powi(2)
    };
    let (v, grad) = gradient(loss, [0.35, 0.25, 0.45]);
    assert!((v - loss_f([0.35, 0.25, 0.45])).abs() < 1e-6);
    for k in 0..3 {
        let mut a = [0.35, 0.25, 0.45];
        let mut b = a;
        a[k] += eps;
        b[k] -= eps;
        let fd = (loss_f(a) - loss_f(b)) / (2.0 * eps);
        assert!(
            (grad[k] - fd).abs() < 5e-3,
            "fused loss grad[{k}]: dual={} fd={fd}",
            grad[k]
        );
    }

    // Sanity-check Dual on photo_filter, unmult, cineon_px, wipe.
    let pf_d = PhotoFilter {
        color: [Dual::lit(0.9), Dual::lit(0.6), Dual::lit(0.3)],
        density: Dual::variable(0.4),
        keep: Dual::lit(1.0),
    };
    let pf_out = photo_filter([0.3, 0.4, 0.5, 1.0].map(Dual::constant), &pf_d);
    assert!(pf_out
        .iter()
        .all(|d| d.value.is_finite() && d.deriv.is_finite()));

    let um_d = Unmult {
        white: Dual::lit(0.0),
        start: Dual::variable(0.1),
        width: Dual::lit(0.5),
        decontaminate: Dual::lit(1.0),
        clip: Dual::lit(1.0),
    };
    let um_out = unmult([0.3, 0.4, 0.5, 1.0].map(Dual::constant), &um_d);
    assert!(um_out
        .iter()
        .all(|d| d.value.is_finite() && d.deriv.is_finite()));

    let cin_d = Cineon {
        black: Dual::lit(95.0),
        white: Dual::lit(685.0),
        black_inside: Dual::lit(0.0),
        white_inside: Dual::lit(1.0),
        gamma: Dual::variable(0.6),
        rolloff: Dual::lit(0.1),
        offset: Dual::lit(cineon_offset(95.0, 685.0)),
    };
    let cin_out = cineon_px([0.3, 0.4, 0.5, 1.0].map(Dual::constant), 0, &cin_d);
    assert!(cin_out
        .iter()
        .all(|d| d.value.is_finite() && d.deriv.is_finite()));

    let edge = WipeEdge {
        feather: Dual::lit(4.0),
        border: Dual::lit(2.0),
        border_color: [Dual::lit(1.0); 4],
        aa: Dual::lit(1.0),
    };
    let wipe_out = wipe(
        &edge,
        [0.1, 0.2, 0.3, 1.0].map(Dual::constant),
        [0.8, 0.7, 0.6, 1.0].map(Dual::constant),
        Dual::variable(0.5),
    );
    assert!(wipe_out[0].deriv.is_finite() && wipe_out[0].deriv != 0.0);
}

#[test]
fn eval_fused_matches_sequential_evaluation() {
    let ops = [
        PixelOp::BrightnessContrast { br: 0.08, co: 1.2 },
        PixelOp::Tint {
            black: [0.05, 0.02, 0.1],
            white: [0.98, 0.95, 0.88],
            amount: 0.35,
        },
        PixelOp::ColorBalance {
            shadows: [0.05, -0.02, 0.03],
            middles: [0.01, 0.04, -0.02],
            highlights: [-0.03, 0.02, 0.04],
            preserve: true,
        },
        PixelOp::Gamma { g: 1.1 },
        PixelOp::Levels {
            ib: 0.04,
            iw: 0.96,
            ob: 0.02,
            ow: 0.98,
            g: 0.95,
        },
        PixelOp::AscCdl {
            slope: [1.02, 0.99, 1.04],
            offset: [0.01, 0.0, -0.01],
            power: [1.05, 0.98, 1.0],
            sat: 1.15,
        },
        PixelOp::AlphaAdjust {
            opacity: 0.8,
            ignore: false,
            invert: false,
            mask_only: false,
        },
        PixelOp::Invert {
            channel: 2,
            blend: 0.65,
        },
    ];
    for step in 0..200 {
        let a = match step % 5 {
            0 => 0.0,
            1 => 1e-7,
            2 => 0.4,
            _ => 1.0,
        };
        let p = [
            ((step * 37) % 101) as f32 / 100.0 * a,
            ((step * 53) % 101) as f32 / 100.0 * a,
            ((step * 71) % 101) as f32 / 100.0 * a,
            a,
        ];
        let fused = eval_fused(p, &ops);
        let mut seq = p;
        for op in &ops {
            seq = eval_fused(seq, &[*op]);
        }
        for c in 0..4 {
            assert!(
                (fused[c] - seq[c]).abs() < 1e-5,
                "step {step} channel {c}: fused={} seq={}",
                fused[c],
                seq[c]
            );
        }
    }
}
