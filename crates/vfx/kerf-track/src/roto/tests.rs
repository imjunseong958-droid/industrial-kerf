//! Following an object on synthetic sequences: a textured disk over a textured background.

use super::*;
use kerf_segment::ModelInfo;

const W: usize = 200;
const H: usize = 150;
const R: f64 = 32.0;

/// Smooth value noise in 0..1.
fn noise(x: f64, y: f64, seed: u32) -> f64 {
    fn h(i: i64, j: i64, s: u32) -> f64 {
        let mut v = (i.wrapping_mul(73856093)
            ^ j.wrapping_mul(19349663)
            ^ i64::from(s).wrapping_mul(83492791)) as u64;
        v ^= v >> 13;
        v = v.wrapping_mul(0x5bd1_e995);
        v ^= v >> 15;
        (v % 10_000) as f64 / 10_000.0
    }
    let mut out = 0.0;
    for (cell, amp) in [(9.0, 0.6), (3.5, 0.4)] {
        let (fx, fy) = (x / cell, y / cell);
        let (i, j) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let a = h(i, j, seed) + (h(i + 1, j, seed) - h(i, j, seed)) * sx;
        let b = h(i, j + 1, seed) + (h(i + 1, j + 1, seed) - h(i, j + 1, seed)) * sx;
        out += amp * (a + (b - a) * sy);
    }
    out
}

fn center(frame: usize) -> [f64; 2] {
    let t = frame as f64;
    [60.0 + 3.1 * t, 70.0 + 1.3 * t + 4.0 * (t * 0.3).sin()]
}

fn inside_disk(frame: usize, x: f64, y: f64) -> bool {
    let c = center(frame);
    (x - c[0]).hypot(y - c[1]) < R
}

/// Frame `frame`: the disk, its texture moving with it, over a still background. With `camouflaged`,
/// the disk has the background's colours, so nothing but a model could tell them apart.
fn scene(frame: usize, camouflaged: bool) -> (usize, usize, Vec<u8>) {
    let c = center(frame);
    let mut rgba = Vec::with_capacity(W * H * 4);
    for y in 0..H {
        for x in 0..W {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let color = if inside_disk(frame, px, py) && !camouflaged {
                let n = noise(px - c[0] + 500.0, py - c[1] + 500.0, 11);
                [0.75 + 0.25 * n, 0.35 + 0.3 * n, 0.1 + 0.15 * n]
            } else {
                let (n, m) = (noise(px, py, 5), noise(px * 1.7, py * 1.7, 9));
                [0.05 + 0.25 * m, 0.3 + 0.4 * n, 0.45 + 0.4 * m]
            };
            for value in color {
                rgba.push((value * 255.0) as u8);
            }
            rgba.push(255);
        }
    }
    (W, H, rgba)
}

/// Whether `point` is inside a closed polygon, by crossings.
fn in_polygon(polygon: &[[f64; 2]], point: [f64; 2]) -> bool {
    let mut inside = false;
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < a[0] + (point[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
        {
            inside = !inside;
        }
    }
    inside
}

/// How well an outline in fractions matches the disk on a frame, intersection over union.
fn overlap(outline: &[[f64; 2]], frame: usize) -> f64 {
    let polygon: Vec<[f64; 2]> = outline
        .iter()
        .map(|c| [c[0] * W as f64, c[1] * H as f64])
        .collect();
    let (mut both, mut either) = (0, 0);
    for y in 0..H {
        for x in 0..W {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let (a, b) = (in_polygon(&polygon, [px, py]), inside_disk(frame, px, py));
            both += usize::from(a && b);
            either += usize::from(a || b);
        }
    }
    both as f64 / either.max(1) as f64
}

fn hints_on_the_disk() -> Hints {
    let c = center(0);
    Hints {
        inside: vec![[c[0] / W as f64, c[1] / H as f64]],
        ..Hints::default()
    }
}

#[test]
fn an_object_is_cut_out_from_one_point() {
    let followed = follow_object(1, &hints_on_the_disk(), None, |n| Some(scene(n, false)));
    assert_eq!(followed.lost_at, None);
    let score = overlap(&followed.outlines[0], 0);
    assert!(score > 0.93, "{score}");
}

#[test]
fn a_box_is_enough_and_nothing_leaves_it() {
    let c = center(0);
    let hints = Hints {
        bbox: Some([
            (c[0] - R - 6.0) / W as f64,
            (c[1] - R - 6.0) / H as f64,
            (c[0] + R + 6.0) / W as f64,
            (c[1] + R + 6.0) / H as f64,
        ]),
        ..Hints::default()
    };
    let followed = follow_object(1, &hints, None, |n| Some(scene(n, false)));
    assert!(overlap(&followed.outlines[0], 0) > 0.9);
    let right = (c[0] + R + 6.0) / W as f64;
    assert!(followed.outlines[0].iter().all(|p| p[0] <= right + 0.01));
}

#[test]
fn an_object_is_followed_with_corners_that_move_one_for_one() {
    let frames = 24;
    let followed = follow_object(frames, &hints_on_the_disk(), None, |n| {
        Some(scene(n, false))
    });
    assert_eq!(followed.lost_at, None, "{:?}", followed.areas);
    assert_eq!(followed.outlines.len(), frames);
    let mut worst = 1.0f64;
    for (frame, outline) in followed.outlines.iter().enumerate() {
        assert_eq!(outline.len(), OUTLINE_CORNERS);
        worst = worst.min(overlap(outline, frame));
    }
    eprintln!("roto: worst overlap with the true disk over {frames} frames, {worst:.3}");
    assert!(worst > 0.9, "{worst}");
    // Corners travel with the disk rather than sliding around it: no corner moves far more than it.
    for pair in followed.outlines.windows(2) {
        for (a, b) in pair[0].iter().zip(&pair[1]) {
            let moved = ((b[0] - a[0]) * W as f64).hypot((b[1] - a[1]) * H as f64);
            assert!(moved < 12.0, "a corner slid {moved} pixels");
        }
    }
    // A steady move does not need every frame.
    assert!(simplified_outlines(&followed.outlines, 0.0).len() <= frames);
}

/// Stands in for a trained model: it knows the disk's place, which colours cannot say, and counts
/// being asked.
struct Knowing {
    frame: std::sync::atomic::AtomicUsize,
}

const STAND_IN: ModelInfo = ModelInfo {
    id: "stand-in",
    task: kerf_segment::Task::Mask,
    name: "Stand-in",
    description: "",
    authors: "",
    licence: "MIT",
    licence_url: "",
    homepage: "",
    url: "",
    file_name: "",
    size: 0,
    sha256: "",
};

impl MaskModel for Knowing {
    fn info(&self) -> &'static ModelInfo {
        &STAND_IN
    }

    fn segment(
        &self,
        _: &[[f32; 3]],
        w: usize,
        h: usize,
        prompt: &Prompt,
    ) -> kerf_segment::Result<Vec<f32>> {
        assert!(prompt.points.iter().any(|(_, on)| *on), "{prompt:?}");
        let frame = self
            .frame
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok((0..w * h)
            .map(|i| {
                let on = inside_disk(frame, (i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
                f32::from(u8::from(on))
            })
            .collect())
    }
}

#[test]
fn a_model_plugged_in_finds_what_colour_alone_cannot() {
    let c = center(0);
    let hints = Hints {
        inside: vec![[c[0] / W as f64, c[1] / H as f64]],
        outside: vec![[0.95, 0.95]],
        ..Hints::default()
    };
    // Without it, the disk cannot be told from what is around it.
    let alone = follow_object(1, &hints, None, |n| Some(scene(n, true)));
    let without = alone
        .outlines
        .first()
        .map_or(0.0, |outline| overlap(outline, 0));
    let model = Knowing {
        frame: std::sync::atomic::AtomicUsize::new(0),
    };
    let with = follow_object(4, &hints, Some(&model), |n| Some(scene(n, true)));
    assert_eq!(with.lost_at, None, "{:?}", with.areas);
    let found = overlap(&with.outlines[3], 3);
    eprintln!("camouflaged: {without:.3} alone, {found:.3} with a model");
    assert!(found > 0.9, "{found}");
    assert!(without < 0.8, "{without}");
    assert_eq!(model.frame.load(std::sync::atomic::Ordering::Relaxed), 4);
}

#[test]
fn a_model_that_disagrees_with_the_flow_is_left_out() {
    struct Wrong;
    impl MaskModel for Wrong {
        fn info(&self) -> &'static ModelInfo {
            &STAND_IN
        }
        fn segment(
            &self,
            _: &[[f32; 3]],
            w: usize,
            h: usize,
            prompt: &Prompt,
        ) -> kerf_segment::Result<Vec<f32>> {
            // Right when first asked, and then the wrong end of the picture.
            Ok((0..w * h)
                .map(|i| {
                    let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
                    let on = if prompt.mask.is_none() {
                        inside_disk(0, x, y)
                    } else {
                        x > 170.0 && y > 120.0
                    };
                    f32::from(u8::from(on))
                })
                .collect())
        }
    }
    let followed = follow_object(6, &hints_on_the_disk(), Some(&Wrong), |n| {
        Some(scene(n, false))
    });
    assert_eq!(followed.lost_at, None, "{:?}", followed.areas);
    assert!(overlap(&followed.outlines[5], 5) > 0.9);
}

#[test]
fn a_frame_that_loses_the_object_stops_it() {
    let followed = follow_object(6, &hints_on_the_disk(), None, |n| {
        if n < 3 {
            Some(scene(n, false))
        } else {
            let (w, h, mut rgba) = scene(0, true);
            rgba.iter_mut().for_each(|value| *value /= 2);
            Some((w, h, rgba))
        }
    });
    assert_eq!(followed.lost_at, Some(3), "{:?}", followed.areas);
}

#[test]
fn nothing_to_go_on_finds_nothing() {
    let followed = follow_object(3, &Hints::default(), None, |n| Some(scene(n, false)));
    assert_eq!(followed.lost_at, Some(0));
    assert!(followed.outlines.is_empty());
}

#[test]
fn the_edge_of_a_square_goes_all_the_way_around() {
    let (w, h) = (12, 10);
    let mut matte = vec![0u8; w * h];
    for y in 2..7 {
        for x in 3..9 {
            matte[y * w + x] = 1;
        }
    }
    // A speck that is not the largest piece.
    matte[9 * w + 11] = 1;
    let edge = edge_of(&matte, w, h);
    assert_eq!(edge.len(), 2 * (6 + 5));
    assert!(edge.contains(&[3.0, 2.0]));
    assert!(edge.contains(&[9.0, 7.0]));
    assert!(!edge.contains(&[11.0, 9.0]));
}
