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

/// One frame of a subject with a known soft edge, over the same background as ever, mixed in light as
/// a lens or a shutter mixes it, with `alpha` saying how much of a pixel is the subject.
fn soft_scene((w, h): (usize, usize), alpha: impl Fn(f64, f64) -> f64) -> (usize, usize, Vec<u8>) {
    let linear = |code: f64| {
        if code <= 0.04045 {
            code / 12.92
        } else {
            ((code + 0.055) / 1.055).powf(2.4)
        }
    };
    let code = |light: f64| {
        if light <= 0.0031308 {
            12.92 * light
        } else {
            1.055 * light.powf(1.0 / 2.4) - 0.055
        }
    };
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let (n, m) = (noise(px, py, 5), noise(px * 1.7, py * 1.7, 9));
            let back = [0.05 + 0.25 * m, 0.3 + 0.4 * n, 0.45 + 0.4 * m];
            let f = noise(px + 500.0, py + 500.0, 11);
            let fore = [0.75 + 0.25 * f, 0.35 + 0.3 * f, 0.1 + 0.15 * f];
            let a = alpha(px, py);
            for (fore, back) in fore.iter().zip(&back) {
                let mixed = linear(*fore) * a + linear(*back) * (1.0 - a);
                rgba.push((code(mixed) * 255.0).round() as u8);
            }
            rgba.push(255);
        }
    }
    (w, h, rgba)
}

/// How far a frame's mask is from a true alpha, as the mean difference over the pixels of the edge.
fn off_by((w, h): (usize, usize), covered: &[f32], alpha: &impl Fn(f64, f64) -> f64) -> f64 {
    let (mut sum, mut count) = (0.0, 0usize);
    for y in 0..h {
        for x in 0..w {
            let truth = alpha(x as f64 + 0.5, y as f64 + 0.5);
            let found = f64::from(covered[y * w + x]);
            if (0.02..=0.98).contains(&truth) || (0.02..=0.98).contains(&found) {
                sum += (truth - found).abs();
                count += 1;
            }
        }
    }
    sum / count.max(1) as f64
}

fn mask_of((w, h): (usize, usize), outline: &[[f64; 2]], soft: Vec<f32>, feather: f32) -> Vec<f32> {
    let mask = kerf_draw::mask::FlatMask {
        points: outline
            .iter()
            .map(|c| [(c[0] * w as f64) as f32, (c[1] * h as f64) as f32])
            .collect(),
        soft,
        feather,
        expansion: 0.0,
        opacity: 1.0,
        inverted: false,
        mode: kerf_model::MaskMode::Add,
    };
    kerf_draw::mask::coverage(&[mask], w, h).unwrap()
}

/// How close what was followed comes to `alpha` at `frame`, and how close the best one `feather=` for
/// the whole mask does.
fn against_the_truth(
    size: (usize, usize),
    followed: &FollowedObject,
    frame: usize,
    alpha: &impl Fn(f64, f64) -> f64,
) -> (f64, f64) {
    let outline = &followed.outlines[frame];
    let soft: Vec<f32> = followed.soft[frame]
        .iter()
        .map(|soft| (soft * size.0 as f64) as f32)
        .collect();
    let ours = off_by(size, &mask_of(size, outline, soft, 0.0), alpha);
    let plain = (0..=30)
        .map(|feather| {
            off_by(
                size,
                &mask_of(size, outline, Vec::new(), feather as f32),
                alpha,
            )
        })
        .fold(f64::INFINITY, f64::min);
    (ours, plain)
}

#[test]
fn an_edge_softer_on_one_side_is_measured_and_beats_one_feather() {
    // Sharp on the left of the disk and 16 pixels wide on its right, as focus falls off across it.
    let (frames, size) = (8, (W, H));
    let alpha_at = |frame: usize| {
        move |x: f64, y: f64| {
            let c = center(frame);
            let (dx, dy) = (x - c[0], y - c[1]);
            let angle = dy.atan2(dx);
            let wide = 1.0 + 15.0 * (0.5 + 0.5 * angle.cos());
            1.0 / (1.0 + ((dx.hypot(dy) - R) * 4.4 / wide).exp())
        }
    };
    let followed = follow_object(frames, &hints_on_the_disk(), None, |n| {
        Some(soft_scene(size, alpha_at(n)))
    });
    assert_eq!(followed.lost_at, None, "{:?}", followed.areas);
    let (mut worst, mut best_plain) = (0.0f64, f64::INFINITY);
    for frame in 0..frames {
        let (ours, plain) = against_the_truth(size, &followed, frame, &alpha_at(frame));
        eprintln!("focus {frame}: {ours:.4} against one feather's {plain:.4}");
        worst = worst.max(ours);
        best_plain = best_plain.min(plain);
    }
    // Right and left of the middle, it found the difference.
    let c = center(frames / 2);
    let outline = &followed.outlines[frames / 2];
    let soft = &followed.soft[frames / 2];
    let at = |side: f64| {
        (0..outline.len())
            .max_by(|a, b| {
                let along = |corner: usize| (outline[corner][0] * W as f64 - c[0]) * side;
                along(*a).total_cmp(&along(*b))
            })
            .map(|corner| soft[corner] * W as f64)
            .unwrap()
    };
    let (right, left) = (at(1.0), at(-1.0));
    eprintln!("focus: {right:.1} wide on the right, {left:.1} on the left");
    assert!(right > 10.0 && left < 5.0, "{right} {left}");
    assert!(worst < 0.06, "{worst}");
    assert!(worst < best_plain * 0.75, "{worst} {best_plain}");
}

#[test]
fn motion_blur_is_measured_the_way_the_subject_moves_and_beats_one_feather() {
    // On a bigger frame, a disk going 12 pixels to the right a frame with the shutter open all the way:
    // its left and right are 12 wide, and its top and bottom are sharp.
    let (frames, size, radius) = (10, (400, 300), 64.0);
    let at = |frame: usize, off: f64| [90.0 + 12.0 * (frame as f64 + off), 140.0];
    let alpha_at = |frame: usize| {
        move |x: f64, y: f64| {
            let inside = (0..32)
                .filter(|step| {
                    let c = at(frame, (f64::from(*step) + 0.5) / 32.0);
                    (x - c[0]).hypot(y - c[1]) < radius
                })
                .count();
            inside as f64 / 32.0
        }
    };
    let start = at(0, 0.5);
    let hints = Hints {
        inside: vec![[start[0] / size.0 as f64, start[1] / size.1 as f64]],
        ..Hints::default()
    };
    let followed = follow_object(frames, &hints, None, |n| {
        Some(soft_scene(size, alpha_at(n)))
    });
    assert_eq!(followed.lost_at, None, "{:?}", followed.areas);
    let (mut worst, mut best_plain) = (0.0f64, f64::INFINITY);
    for frame in 0..frames {
        let (ours, plain) = against_the_truth(size, &followed, frame, &alpha_at(frame));
        eprintln!("blur {frame}: {ours:.4} against one feather's {plain:.4}");
        worst = worst.max(ours);
        best_plain = best_plain.min(plain);
    }
    assert!(worst < 0.06, "{worst}");
    assert!(worst < best_plain * 0.8, "{worst} {best_plain}");
    // It says so the way it moved.
    let middle = frames / 2;
    let c = at(middle, 0.5);
    let outline = &followed.outlines[middle];
    let soft = &followed.soft[middle];
    let (mut across, mut along) = (Vec::new(), Vec::new());
    for (corner, wide) in outline.iter().zip(soft) {
        let (dx, dy) = (
            corner[0] * size.0 as f64 - c[0],
            corner[1] * size.1 as f64 - c[1],
        );
        let wide = wide * size.0 as f64;
        if dx.abs() > dy.abs() * 2.0 {
            along.push(wide);
        } else if dy.abs() > dx.abs() * 2.0 {
            across.push(wide);
        }
    }
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
    eprintln!(
        "blur: {:.1} wide along, {:.1} across",
        mean(&along),
        mean(&across)
    );
    assert!(mean(&along) > mean(&across) + 3.0, "{along:?} {across:?}");
}

#[test]
fn a_hard_edge_stays_hard() {
    let frames = 6;
    let followed = follow_object(frames, &hints_on_the_disk(), None, |n| {
        Some(scene(n, false))
    });
    let widest = followed
        .soft
        .iter()
        .flatten()
        .fold(0.0f64, |most, soft| most.max(soft * W as f64));
    eprintln!("hard: at most {widest:.2} wide");
    assert!(widest < 2.0, "{widest}");
}

#[test]
fn softness_changing_alone_is_kept_as_keyframes() {
    let outline = vec![[0.5, 0.5], [0.6, 0.5], [0.5, 0.6]];
    let followed = FollowedObject {
        outlines: vec![outline; 5],
        soft: [0.0, 0.0, 0.2, 0.0, 0.0]
            .iter()
            .map(|wide| vec![*wide; 3])
            .collect(),
        areas: vec![0.1; 5],
        lost_at: None,
    };
    assert_eq!(simplified_outlines(&followed.outlines, 0.01), [0, 4]);
    assert!(simplified_object(&followed, 0.01).contains(&2));
}

/// Forty corners evenly around a square 0.3 of the frame each way whose upper left is at `left`, 0.35,
/// starting `start` of the way along its top.
fn square_outline(left: f64, start: f64) -> Vec<[f64; 2]> {
    (0..OUTLINE_CORNERS)
        .map(|corner| {
            let around = (start + corner as f64 / OUTLINE_CORNERS as f64 * 1.2).rem_euclid(1.2);
            let (side, along) = ((around / 0.3) as usize, around % 0.3);
            match side {
                0 => [left + along, 0.35],
                1 => [left + 0.3, 0.35 + along],
                2 => [left + 0.3 - along, 0.65],
                _ => [left, 0.65 - along],
            }
        })
        .collect()
}

/// How far `point` is from the closed outline through `corners`.
fn off_outline(point: [f64; 2], corners: &[[f64; 2]]) -> f64 {
    (0..corners.len())
        .map(|index| {
            let (a, b) = (corners[index], corners[(index + 1) % corners.len()]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let length = dx * dx + dy * dy;
            let along = if length > 0.0 {
                (((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (a[0] + dx * along - point[0]).hypot(a[1] + dy * along - point[1])
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_square_keeps_the_corners_that_shape_it_the_same_ones_in_every_frame() {
    // A square going right, its forty corners a little way along its top from its own.
    let outlines: Vec<Vec<[f64; 2]>> = (0..10)
        .map(|frame| square_outline(0.15 + frame as f64 * 0.01, 0.004))
        .collect();
    let followed = FollowedObject {
        soft: vec![vec![0.0; OUTLINE_CORNERS]; outlines.len()],
        areas: vec![0.09; outlines.len()],
        outlines,
        lost_at: None,
    };
    let tolerance = 0.5 / 960.0;
    let kept = simplified_corners(&followed, tolerance);
    // Two by each of its corners, where forty evenly spaced cut it short.
    assert_eq!(kept.len(), 8, "{kept:?}");
    let shaped = followed.with_corners(&kept);
    assert_eq!(shaped.outlines.len(), followed.outlines.len());
    for (whole, fewer) in followed.outlines.iter().zip(&shaped.outlines) {
        assert_eq!(fewer.len(), kept.len());
        for corner in whole {
            let off = off_outline(*corner, fewer);
            assert!(off <= tolerance, "{corner:?} is {off} off {fewer:?}");
        }
    }
    // One that any frame needs is kept in all of them: a dent in the top, in one frame.
    let mut dented = followed.clone();
    dented.outlines[6][5][1] += 0.01;
    let kept = simplified_corners(&dented, tolerance);
    assert!(kept.contains(&5), "{kept:?}");
    assert!(kept.len() <= 11, "{kept:?}");
    // And one where the edge is softer than it is on either side.
    let mut softer = followed.clone();
    for soft in &mut softer.soft {
        soft[25] = 0.02;
    }
    let kept = simplified_corners(&softer, tolerance);
    assert!(kept.contains(&25), "{kept:?}");
    // A round outline needs most of its corners, and three are the least there are.
    let round: Vec<[f64; 2]> = (0..OUTLINE_CORNERS)
        .map(|corner| {
            let turn = corner as f64 / OUTLINE_CORNERS as f64 * std::f64::consts::TAU;
            [0.5 + 0.2 * turn.cos(), 0.5 + 0.2 * turn.sin()]
        })
        .collect();
    let disk = FollowedObject {
        outlines: vec![round],
        soft: vec![vec![0.0; OUTLINE_CORNERS]],
        areas: vec![0.1],
        lost_at: None,
    };
    assert!(simplified_corners(&disk, tolerance).len() >= 20);
    assert!(simplified_corners(&disk, 1.0).len() >= 2);
    assert_eq!(
        simplified_corners(&FollowedObject::default(), tolerance),
        [0usize; 0]
    );
}

/// A textured disk `radius` across going back and forth across a textured frame at `speed` pixels a
/// frame, curving a little as it goes: where it is on `frame`.
fn fast_center((w, h): (usize, usize), radius: f64, speed: f64, frame: usize) -> [f64; 2] {
    let room = w as f64 - 2.0 * radius - 8.0;
    let along = (speed * frame as f64) % (2.0 * room);
    let along = if along > room {
        2.0 * room - along
    } else {
        along
    };
    let t = frame as f64;
    [
        radius + 4.0 + along,
        h as f64 / 2.0 + (h as f64 / 2.0 - radius - 4.0) * 0.5 * (t * 0.4).sin(),
    ]
}

fn fast_scene(
    (w, h): (usize, usize),
    radius: f64,
    speed: f64,
    frame: usize,
    alike: bool,
) -> (usize, usize, Vec<u8>) {
    let c = fast_center((w, h), radius, speed, frame);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let color = if (px - c[0]).hypot(py - c[1]) < radius {
                let n = noise(px - c[0] + 500.0, py - c[1] + 500.0, 11);
                if alike {
                    [0.35 + 0.3 * n, 0.4 + 0.3 * n, 0.3 + 0.3 * n]
                } else {
                    [0.75 + 0.25 * n, 0.35 + 0.3 * n, 0.1 + 0.15 * n]
                }
            } else {
                let (n, m) = (noise(px, py, 5), noise(px * 1.7, py * 1.7, 9));
                if alike {
                    [0.1 + 0.4 * m, 0.2 + 0.4 * n, 0.15 + 0.4 * m]
                } else {
                    [0.05 + 0.25 * m, 0.3 + 0.4 * n, 0.45 + 0.4 * m]
                }
            };
            for value in color {
                rgba.push((value * 255.0) as u8);
            }
            rgba.push(255);
        }
    }
    (w, h, rgba)
}

/// How well it follows that disk: the mean and worst overlap with the truth, and how many frames it
/// found of `frames`.
fn fast_overlap(
    size: (usize, usize),
    radius: f64,
    speed: f64,
    frames: usize,
    alike: bool,
) -> (f64, f64, usize) {
    let (w, h) = (size.0 as f64, size.1 as f64);
    let start = fast_center(size, radius, speed, 0);
    let hints = Hints {
        inside: vec![[start[0] / w, start[1] / h]],
        ..Hints::default()
    };
    let followed = follow_object(frames, &hints, None, |frame| {
        Some(fast_scene(size, radius, speed, frame, alike))
    });
    let (mut sum, mut worst) = (0.0, 1.0f64);
    for (frame, outline) in followed.outlines.iter().enumerate() {
        let c = fast_center(size, radius, speed, frame);
        let polygon: Vec<[f64; 2]> = outline.iter().map(|at| [at[0] * w, at[1] * h]).collect();
        let (mut both, mut either) = (0usize, 0usize);
        for y in 0..size.1 {
            for x in 0..size.0 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let (a, b) = (
                    in_polygon(&polygon, [px, py]),
                    (px - c[0]).hypot(py - c[1]) < radius,
                );
                both += usize::from(a && b);
                either += usize::from(a || b);
            }
        }
        let overlap = both as f64 / either.max(1) as f64;
        sum += overlap;
        worst = worst.min(overlap);
    }
    // A frame it never got to counts as none of it.
    if followed.outlines.len() < frames {
        worst = 0.0;
    }
    (sum / frames as f64, worst, followed.outlines.len())
}

#[test]
fn a_subject_much_further_than_its_band_is_kept_up_with() {
    // 640x360 has a band of 16 pixels, and the flow alone lost this within a few frames, at 0.17 and
    // 0.04 on average.
    for speed in [64.0, 128.0] {
        for alike in [false, true] {
            let (mean, worst, found) = fast_overlap((640, 360), 60.0, speed, 10, alike);
            assert_eq!(found, 10, "{speed} {alike}");
            assert!(
                mean > 0.96 && worst > 0.94,
                "{speed} {alike}: {mean} {worst}"
            );
        }
    }
}

#[test]
fn its_corners_go_with_a_fast_subject_one_for_one() {
    let (size, speed) = ((640, 360), 64.0);
    let start = fast_center(size, 60.0, speed, 0);
    let hints = Hints {
        inside: vec![[start[0] / 640.0, start[1] / 360.0]],
        ..Hints::default()
    };
    let followed = follow_object(6, &hints, None, |frame| {
        Some(fast_scene(size, 60.0, speed, frame, false))
    });
    assert_eq!(followed.outlines.len(), 6);
    for (frame, pair) in followed.outlines.windows(2).enumerate() {
        let (from, to) = (
            fast_center(size, 60.0, speed, frame),
            fast_center(size, 60.0, speed, frame + 1),
        );
        let went = [to[0] - from[0], to[1] - from[1]];
        for (a, b) in pair[0].iter().zip(&pair[1]) {
            let moved = [(b[0] - a[0]) * 640.0, (b[1] - a[1]) * 360.0];
            let off = (moved[0] - went[0]).hypot(moved[1] - went[1]);
            assert!(
                off < 12.0,
                "a corner slid {off} pixels off on frame {frame}"
            );
        }
    }
}

#[test]
fn a_subject_is_found_again_by_its_own_pixels_and_only_where_it_went() {
    let size = (640, 360);
    let (_, _, rgba) = fast_scene(size, 60.0, 0.0, 0, false);
    let before = Picture::of(640, 360, &rgba);
    let c = fast_center(size, 60.0, 0.0, 0);
    let matte: Vec<u8> = (0..640 * 360)
        .map(|at| {
            let (x, y) = ((at % 640) as f64 + 0.5, (at / 640) as f64 + 0.5);
            u8::from((x - c[0]).hypot(y - c[1]) < 60.0)
        })
        .collect();
    // Where it stays, nothing.
    assert_eq!(moved_by(&before, &matte, &before), [0, 0]);
    // A frame later, wherever it has got to.
    let (_, _, rgba) = fast_scene(size, 60.0, 100.0, 1, false);
    let after = Picture::of(640, 360, &rgba);
    let there = fast_center(size, 60.0, 100.0, 1);
    let went = moved_by(&before, &matte, &after);
    assert!(
        (went[0] as f64 - (there[0] - c[0])).abs() <= 1.0,
        "{went:?}"
    );
    assert!(
        (went[1] as f64 - (there[1] - c[1])).abs() <= 1.0,
        "{went:?}"
    );
    // The band goes with how far, between one and its most.
    assert_eq!(band_of(size, [0, 0]), 16.0);
    assert_eq!(band_of(size, [100, 0]), 25.0);
    assert_eq!(band_of(size, [1000, 0]), 48.0);
}

#[test]
#[ignore]
fn measure_fast_subjects() {
    for (size, radius) in [((200, 150), 32.0), ((640, 360), 60.0)] {
        let band = (size.0 as f64 / 40.0).max(4.0);
        for alike in [false, true] {
            for by_band in [0.25, 0.5, 1.0, 2.0, 4.0, 8.0] {
                let speed = band * by_band;
                let started = std::time::Instant::now();
                let (mean, worst, found) = fast_overlap(size, radius, speed, 24, alike);
                eprintln!(
                    "{}x{} {} at {speed:.1} px a frame ({by_band} bands): mean {mean:.3}, worst {worst:.3}, {found} of 24 frames, {:.0} ms a frame",
                    size.0,
                    size.1,
                    if alike { "alike" } else { "apart" },
                    started.elapsed().as_secs_f64() * 1000.0 / 24.0
                );
            }
        }
    }
}

/// How a harder subject goes: a textured ellipse, twice as wide as it is tall at rest, that turns and
/// changes size as it crosses the frame, and that a shutter may stay open on.
#[derive(Clone, Copy, Debug)]
struct Hard {
    /// Its half width at rest.
    radius: f64,
    /// Pixels a frame.
    speed: f64,
    /// Degrees a frame.
    turn: f64,
    /// How far its size goes either way, a share of its size, over about ten frames.
    grow: f64,
    /// How much of a frame the shutter is open for, from 0 to 1.
    shutter: f64,
    alike: bool,
    /// Pixels a frame up and down, on top of how it wanders.
    rise: f64,
    /// A rounded box rather than an ellipse.
    boxy: bool,
}

impl Hard {
    fn center(&self, (w, h): (usize, usize), t: f64) -> [f64; 2] {
        let reach = self.radius * (1.0 + self.grow);
        let room = w as f64 - 2.0 * reach - 8.0;
        let along = (self.speed * t).rem_euclid(2.0 * room);
        let along = if along > room {
            2.0 * room - along
        } else {
            along
        };
        let tall = reach / 2.0;
        let up = h as f64 - 2.0 * tall - 8.0;
        if self.rise != 0.0 {
            let by = (self.rise * t).rem_euclid(2.0 * up);
            let by = if by > up { 2.0 * up - by } else { by };
            return [reach + 4.0 + along, tall + 4.0 + by];
        }
        [
            reach + 4.0 + along,
            h as f64 / 2.0 + (h as f64 / 2.0 - reach - 4.0) * 0.5 * (t * 0.4).sin(),
        ]
    }

    /// Where a point of the frame falls on the subject at `t`, as a point of it at rest, and whether that
    /// is on it.
    fn on_it(&self, size: (usize, usize), t: f64, (px, py): (f64, f64)) -> ([f64; 2], bool) {
        let c = self.center(size, t);
        let (s, co) = (self.turn * t).to_radians().sin_cos();
        let by = 1.0 + self.grow * (t * 0.6).sin();
        let (dx, dy) = ((px - c[0]) / by, (py - c[1]) / by);
        let (x, y) = (dx * co + dy * s, -dx * s + dy * co);
        let (a, b) = (self.radius, self.radius / 2.0);
        if self.boxy {
            return ([x, y], (x / a).powi(4) + (y / b).powi(4) < 1.0);
        }
        ([x, y], (x / a).powi(2) + (y / b).powi(2) < 1.0)
    }

    /// What is covered more than half of the time: with an odd number of moments, since at 8 what is
    /// covered for 4 would count, and that is a sixteenth of the blur further out than the true edge at
    /// both ends.
    fn half_covered(&self, (w, h): (usize, usize), frame: usize) -> Vec<bool> {
        let moments: Vec<f64> = (0..33)
            .map(|at| frame as f64 + self.shutter * 7.0 / 8.0 * f64::from(at) / 32.0)
            .collect();
        let at = |i: usize| {
            let (px, py) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            moments
                .iter()
                .filter(|t| self.on_it((w, h), **t, (px, py)).1)
                .count()
                * 2
                > moments.len()
        };
        (0..w * h).into_par_iter().map(at).collect()
    }

    /// The frame as a shutter sees it, and how much of each pixel the subject covers.
    fn scene(&self, (w, h): (usize, usize), frame: usize) -> (Vec<u8>, Vec<f32>) {
        let moments: Vec<f64> = if self.shutter > 0.0 {
            (0..8)
                .map(|at| frame as f64 + self.shutter * at as f64 / 8.0)
                .collect()
        } else {
            vec![frame as f64]
        };
        let mut rgba = Vec::with_capacity(w * h * 4);
        let mut covered = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let (n, m) = (noise(px, py, 5), noise(px * 1.7, py * 1.7, 9));
                let behind = if self.alike {
                    [0.1 + 0.4 * m, 0.2 + 0.4 * n, 0.15 + 0.4 * m]
                } else {
                    [0.05 + 0.25 * m, 0.3 + 0.4 * n, 0.45 + 0.4 * m]
                };
                let (mut sum, mut hits) = ([0.0; 3], 0.0);
                for t in &moments {
                    let (at, on) = self.on_it((w, h), *t, (px, py));
                    let color = if on {
                        hits += 1.0;
                        let n = noise(at[0] + 500.0, at[1] + 500.0, 11);
                        if self.alike {
                            [0.35 + 0.3 * n, 0.4 + 0.3 * n, 0.3 + 0.3 * n]
                        } else {
                            [0.75 + 0.25 * n, 0.35 + 0.3 * n, 0.1 + 0.15 * n]
                        }
                    } else {
                        behind
                    };
                    // A shutter adds up light, not code values.
                    for k in 0..3 {
                        sum[k] += f64::from(super::soft::linear(color[k] as f32));
                    }
                }
                let share = moments.len() as f64;
                for value in sum {
                    let light = value / share;
                    let code = if light <= 0.0031308 {
                        light * 12.92
                    } else {
                        1.055 * light.powf(1.0 / 2.4) - 0.055
                    };
                    rgba.push((code * 255.0) as u8);
                }
                rgba.push(255);
                covered.push((hits / share) as f32);
            }
        }
        (rgba, covered)
    }

    /// What is measured for the table in `docs/architecture.md`, by the measure alone and with the shutter
    /// when one is found: the overlap as [`Hard::overlap`] has it, the outline's area for the area of what
    /// is more than half covered, and how far off the true alpha a mask drawn from it is.
    fn measured(
        &self,
        size: (usize, usize),
        frames: usize,
    ) -> ([[f64; 4]; 2], Option<shutter::Shutter>) {
        let (w, h) = (size.0 as f64, size.1 as f64);
        let start = self.center(size, self.shutter / 2.0);
        let hints = Hints {
            inside: vec![[start[0] / w, start[1] / h]],
            ..Hints::default()
        };
        let traced = || {
            trace(frames, &hints, None, |frame| {
                Some((size.0, size.1, self.scene(size, frame).0))
            })
        };
        let of = |followed: &FollowedObject| {
            let (mut sum, mut worst, mut area, mut alpha, mut read) = (0.0, 1.0f64, 0.0, 0.0, 0.0);
            for (frame, outline) in followed.outlines.iter().enumerate() {
                let polygon: Vec<[f64; 2]> =
                    outline.iter().map(|at| [at[0] * w, at[1] * h]).collect();
                let covered = self.half_covered(size, frame);
                let (mut both, mut either, mut mine, mut truth) = (0usize, 0usize, 0usize, 0usize);
                for y in 0..size.1 {
                    for x in 0..size.0 {
                        let a = in_polygon(&polygon, [x as f64 + 0.5, y as f64 + 0.5]);
                        let b = covered[y * size.0 + x];
                        both += usize::from(a && b);
                        either += usize::from(a || b);
                        mine += usize::from(a);
                        truth += usize::from(b);
                    }
                }
                let overlap = both as f64 / either.max(1) as f64;
                sum += overlap;
                worst = worst.min(overlap);
                area += mine as f64 / truth.max(1) as f64;
                // Every fourth frame: the soft edge takes a while to read.
                if frame % 4 == 2 {
                    let true_alpha = self.scene(size, frame).1;
                    let at = |x: f64, y: f64| {
                        let (x, y) = (x as usize, y as usize);
                        f64::from(true_alpha[y.min(size.1 - 1) * size.0 + x.min(size.0 - 1)])
                    };
                    alpha += against_the_truth(size, followed, frame, &at).0;
                    read += 1.0;
                }
            }
            let n = followed.outlines.len().max(1) as f64;
            [sum / n, worst, area / n, alpha / f64::max(read, 1.0)]
        };
        let first = traced();
        let found = shutter::fit(&first.raw, &first.across, first.shortest_blur());
        let plain = of(&first.by_the_measure());
        let with = match found {
            Some(found) => of(&traced().under_a_shutter(found)),
            None => plain,
        };
        ([plain, with], found)
    }

    /// How well it is followed over `frames`: the mean and worst overlap with what is more than half
    /// covered, and how many frames it got to.
    fn overlap(&self, size: (usize, usize), frames: usize) -> (f64, f64, usize) {
        let (w, h) = (size.0 as f64, size.1 as f64);
        let start = self.center(size, self.shutter / 2.0);
        let hints = Hints {
            inside: vec![[start[0] / w, start[1] / h]],
            ..Hints::default()
        };
        let followed = follow_object(frames, &hints, None, |frame| {
            Some((size.0, size.1, self.scene(size, frame).0))
        });
        let (mut sum, mut worst) = (0.0, 1.0f64);
        for (frame, outline) in followed.outlines.iter().enumerate() {
            let polygon: Vec<[f64; 2]> = outline.iter().map(|at| [at[0] * w, at[1] * h]).collect();
            let covered = self.half_covered(size, frame);
            let (mut both, mut either) = (0usize, 0usize);
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let a = in_polygon(&polygon, [x as f64 + 0.5, y as f64 + 0.5]);
                    let b = covered[y * size.0 + x];
                    both += usize::from(a && b);
                    either += usize::from(a || b);
                }
            }
            let overlap = both as f64 / either.max(1) as f64;
            sum += overlap;
            worst = worst.min(overlap);
        }
        if followed.outlines.len() < frames {
            worst = 0.0;
        }
        (sum / frames as f64, worst, followed.outlines.len())
    }
}

#[test]
#[ignore]
fn measure_hard_subjects() {
    let size = (640, 360);
    let still = Hard {
        radius: 80.0,
        speed: 0.0,
        turn: 0.0,
        grow: 0.0,
        shutter: 0.0,
        alike: false,
        rise: 0.0,
        boxy: false,
    };
    let frames = 24;
    for alike in [false, true] {
        for (name, shape) in [
            (
                "rigid, slow",
                Hard {
                    speed: 8.0,
                    ..still
                },
            ),
            (
                "rigid, fast",
                Hard {
                    speed: 64.0,
                    ..still
                },
            ),
            (
                "turning 8, slow",
                Hard {
                    speed: 8.0,
                    turn: 8.0,
                    ..still
                },
            ),
            (
                "turning 8, fast",
                Hard {
                    speed: 64.0,
                    turn: 8.0,
                    ..still
                },
            ),
            (
                "turning 20, fast",
                Hard {
                    speed: 64.0,
                    turn: 20.0,
                    ..still
                },
            ),
            (
                "growing 20%, slow",
                Hard {
                    speed: 8.0,
                    grow: 0.2,
                    ..still
                },
            ),
            (
                "growing 20%, fast",
                Hard {
                    speed: 64.0,
                    grow: 0.2,
                    ..still
                },
            ),
            (
                "turning and growing, fast",
                Hard {
                    speed: 64.0,
                    turn: 12.0,
                    grow: 0.2,
                    ..still
                },
            ),
            (
                "turning and growing, faster",
                Hard {
                    speed: 128.0,
                    turn: 20.0,
                    grow: 0.3,
                    ..still
                },
            ),
            (
                "half shutter, slow",
                Hard {
                    speed: 8.0,
                    shutter: 0.5,
                    ..still
                },
            ),
            (
                "half shutter, fast",
                Hard {
                    speed: 64.0,
                    shutter: 0.5,
                    ..still
                },
            ),
            (
                "open shutter, fast",
                Hard {
                    speed: 64.0,
                    shutter: 1.0,
                    ..still
                },
            ),
            (
                "half shutter, faster",
                Hard {
                    speed: 128.0,
                    shutter: 0.5,
                    ..still
                },
            ),
            (
                "half shutter, turning, fast",
                Hard {
                    speed: 64.0,
                    turn: 12.0,
                    shutter: 0.5,
                    ..still
                },
            ),
        ] {
            let shape = Hard { alike, ..shape };
            let began = std::time::Instant::now();
            let (mean, worst, found) = shape.overlap(size, frames);
            eprintln!(
                "{:<5} {name:<30} mean {mean:.3}, worst {worst:.3}, {found} of {frames} frames, {:.0} s",
                if alike { "alike" } else { "apart" },
                began.elapsed().as_secs_f64()
            );
        }
    }
}

/// Moving the picture to where a subject went moves what stayed behind by as much, and blocks on the
/// subject's edge went with that, dragging its matte out by more than a band could bring back: one frame
/// of this came to 0.938 where the shifts were all right.
#[test]
fn a_subject_found_again_keeps_clear_of_where_it_was() {
    for speed in [10.0, 20.0, 40.0] {
        let (mean, worst, found) = fast_overlap((200, 150), 32.0, speed, 24, false);
        assert_eq!(found, 24, "{speed}");
        assert!(mean > 0.99 && worst > 0.98, "{speed}: {mean} {worst}");
    }
}

fn hard() -> Hard {
    Hard {
        radius: 80.0,
        speed: 0.0,
        turn: 0.0,
        grow: 0.0,
        shutter: 0.0,
        alike: false,
        rise: 0.0,
        boxy: false,
    }
}

#[test]
fn a_subject_that_turns_and_changes_size_at_speed_is_kept_up_with() {
    let size = (640, 360);
    for (shape, least) in [
        (
            Hard {
                speed: 64.0,
                turn: 12.0,
                grow: 0.2,
                ..hard()
            },
            0.98,
        ),
        (
            Hard {
                speed: 128.0,
                turn: 20.0,
                grow: 0.3,
                ..hard()
            },
            0.97,
        ),
        (
            Hard {
                speed: 64.0,
                turn: 12.0,
                grow: 0.2,
                alike: true,
                ..hard()
            },
            0.94,
        ),
    ] {
        let (mean, worst, found) = shape.overlap(size, 12);
        assert_eq!(found, 12, "{shape:?}");
        assert!(
            mean > least && worst > least - 0.02,
            "{shape:?}: {mean} {worst}"
        );
    }
}

/// Under an open shutter a cut takes in nearly all of the blur, and what is more than half covered is that
/// drawn in along the way the subject went: see `shutter.rs`. These say how far it goes, so that it cannot
/// go less far.
#[test]
fn a_subject_under_an_open_shutter_is_kept_up_with() {
    let size = (640, 360);
    for (shape, least) in [
        (
            Hard {
                speed: 8.0,
                shutter: 0.5,
                ..hard()
            },
            0.96,
        ),
        (
            Hard {
                speed: 64.0,
                shutter: 0.5,
                ..hard()
            },
            0.95,
        ),
        (
            Hard {
                speed: 64.0,
                shutter: 1.0,
                ..hard()
            },
            0.90,
        ),
        (
            Hard {
                speed: 64.0,
                shutter: 0.5,
                alike: true,
                ..hard()
            },
            0.93,
        ),
        (
            Hard {
                speed: 48.0,
                rise: 40.0,
                shutter: 1.0,
                ..hard()
            },
            0.91,
        ),
        (
            Hard {
                speed: 64.0,
                shutter: 1.0,
                boxy: true,
                ..hard()
            },
            0.93,
        ),
    ] {
        let (mean, worst, found) = shape.overlap(size, 12);
        assert_eq!(found, 12, "{shape:?}");
        assert!(
            mean > least && worst > least - 0.14,
            "{shape:?}: {mean} {worst}"
        );
    }
}

/// The cut takes in more than is half covered, and the measure alone brings it only so far back: 12% too
/// much at 64 pixels with the shutter open all the time.
#[test]
fn an_outline_under_an_open_shutter_is_the_size_of_what_is_half_covered() {
    let shape = Hard {
        speed: 64.0,
        shutter: 1.0,
        ..hard()
    };
    let ([plain, with], found) = shape.measured((640, 360), 12);
    assert!(found.is_some());
    assert!(plain[2] > 1.08, "{plain:?}");
    assert!((with[2] - 1.0).abs() < 0.06, "{with:?}");
    // And a mask from it comes nearer the true alpha.
    assert!(with[3] < plain[3] * 0.85, "{plain:?} {with:?}");
}

/// What has no long shutter is left to the measure, down to its bytes, at the cost of a sum over its corners.
#[test]
fn what_has_no_long_shutter_stays_as_it_was() {
    let size = (640, 360);
    let (w, h) = (size.0 as f64, size.1 as f64);
    for shape in [
        Hard {
            speed: 8.0,
            shutter: 0.5,
            ..hard()
        },
        Hard {
            speed: 8.0,
            shutter: 0.5,
            alike: true,
            ..hard()
        },
        Hard {
            speed: 64.0,
            ..hard()
        },
        hard(),
    ] {
        let start = shape.center(size, shape.shutter / 2.0);
        let hints = Hints {
            inside: vec![[start[0] / w, start[1] / h]],
            ..Hints::default()
        };
        let traced = trace(8, &hints, None, |frame| {
            Some((size.0, size.1, shape.scene(size, frame).0))
        });
        assert_eq!(
            shutter::fit(&traced.raw, &traced.across, traced.shortest_blur()),
            None,
            "{shape:?}"
        );
    }
}

/// A subject out of focus reads as wide whichever way its edge faces, however fast it goes.
#[test]
fn a_subject_out_of_focus_and_moving_has_no_shutter() {
    let (frames, size, radius) = (10, (400, 300), 64.0);
    let center = |frame: usize| [90.0 + 16.0 * frame as f64, 140.0];
    let alpha_at = |frame: usize| {
        move |x: f64, y: f64| {
            let c = center(frame);
            1.0 / (1.0 + (((x - c[0]).hypot(y - c[1]) - radius) * 4.4 / 16.0).exp())
        }
    };
    let start = center(0);
    let hints = Hints {
        inside: vec![[start[0] / size.0 as f64, start[1] / size.1 as f64]],
        ..Hints::default()
    };
    let traced = trace(frames, &hints, None, |n| {
        Some(soft_scene(size, alpha_at(n)))
    });
    assert_eq!(traced.followed.lost_at, None);
    assert_eq!(
        shutter::fit(&traced.raw, &traced.across, traced.shortest_blur()),
        None
    );
}

#[test]
#[ignore]
fn measure_shutters() {
    let size = (640, 360);
    let frames = 24;
    for alike in [false, true] {
        for (name, shape) in [
            (
                "half 8",
                Hard {
                    speed: 8.0,
                    shutter: 0.5,
                    ..hard()
                },
            ),
            (
                "half 64",
                Hard {
                    speed: 64.0,
                    shutter: 0.5,
                    ..hard()
                },
            ),
            (
                "half 128",
                Hard {
                    speed: 128.0,
                    shutter: 0.5,
                    ..hard()
                },
            ),
            (
                "half 64 turn 12",
                Hard {
                    speed: 64.0,
                    turn: 12.0,
                    shutter: 0.5,
                    ..hard()
                },
            ),
            (
                "open 64",
                Hard {
                    speed: 64.0,
                    shutter: 1.0,
                    ..hard()
                },
            ),
            (
                "open 32",
                Hard {
                    speed: 32.0,
                    shutter: 1.0,
                    ..hard()
                },
            ),
            (
                "open 128",
                Hard {
                    speed: 128.0,
                    shutter: 1.0,
                    ..hard()
                },
            ),
            (
                "half 64 diagonal",
                Hard {
                    speed: 48.0,
                    rise: 40.0,
                    shutter: 0.5,
                    ..hard()
                },
            ),
            (
                "open 64 diagonal",
                Hard {
                    speed: 48.0,
                    rise: 40.0,
                    shutter: 1.0,
                    ..hard()
                },
            ),
            (
                "open 64 small",
                Hard {
                    radius: 50.0,
                    speed: 64.0,
                    shutter: 1.0,
                    ..hard()
                },
            ),
            (
                "open 48 turn+grow",
                Hard {
                    speed: 48.0,
                    turn: 12.0,
                    grow: 0.2,
                    shutter: 1.0,
                    ..hard()
                },
            ),
            (
                "open 64 box",
                Hard {
                    speed: 64.0,
                    shutter: 1.0,
                    boxy: true,
                    ..hard()
                },
            ),
            (
                "rigid 64 no blur",
                Hard {
                    speed: 64.0,
                    ..hard()
                },
            ),
            ("still", Hard { ..hard() }),
        ] {
            let shape = Hard { alike, ..shape };
            let ([plain, with], found) = shape.measured(size, frames);
            eprintln!(
                "{:<5} {name:<20} {:.3} {:.3} {:.3} {:.3} | {:.3} {:.3} {:.3} {:.3} | {found:?}",
                if alike { "alike" } else { "apart" },
                plain[0],
                plain[1],
                plain[2],
                plain[3],
                with[0],
                with[1],
                with[2],
                with[3],
            );
        }
    }
}

/// The two pictures, the matte of the first, and its colors, for a frame of a disk going at `speed`.
fn a_step(
    size: (usize, usize),
    radius: f64,
    speed: f64,
    alike: bool,
) -> (Picture, Picture, Vec<u8>, Gmm, Gmm) {
    let (w, h) = size;
    let picture =
        |frame: usize| Picture::of(w, h, &fast_scene(size, radius, speed, frame, alike).2);
    let start = fast_center(size, radius, speed, 0);
    let hints = Hints {
        inside: vec![[start[0] / w as f64, start[1] / h as f64]],
        ..Hints::default()
    };
    let first = picture(0);
    let (matte, fg, bg) = first_matte(&first, &hints, None).expect("a matte");
    (first, picture(1), matte, fg, bg)
}

#[test]
fn the_search_for_a_subject_runs_only_where_the_flow_has_lost_it() {
    let (size, band) = ((640, 360), 16.0);
    for alike in [false, true] {
        for by_band in [0.25, 0.5, 1.0, 1.9, 4.0, 8.0] {
            let (first, next, matte, ..) = a_step(size, 60.0, band * by_band, alike);
            let flow = block_flow(&next.image, &first.image, 8, 4);
            assert_eq!(
                flow_lost(&first, &matte, &next, &flow),
                by_band > 2.0,
                "{by_band} bands, {alike}"
            );
        }
    }
}

/// Slow subjects are where they were: going by the flow alone, as now, comes to the same bytes as
/// searching first, as every frame used to.
#[test]
fn a_slow_subject_is_cut_as_it_was_with_or_without_the_search() {
    for alike in [false, true] {
        for by_band in [0.25, 0.5, 1.0] {
            let (first, next, matte, fg, bg) = a_step((640, 360), 60.0, 16.0 * by_band, alike);
            let colors = (&fg, &bg);
            let without = next_matte(&first, &matte, &next, colors, None, false);
            let with = next_matte(&first, &matte, &next, colors, None, true);
            assert_eq!(without.1, [0, 0], "{by_band} bands, {alike}");
            assert_eq!(without, with, "{by_band} bands, {alike}");
        }
    }
}

/// The frames of the real footage `scripts/fetch-real-footage.sh` fetches, `width` across, as 0 to 1
/// RGB, or `None` when `KERF_REAL_FOOTAGE` does not say where it is.
fn footage(names: impl Iterator<Item = String>, width: usize) -> Option<(usize, usize, Vec<Vec<[f32; 3]>>)> {
    let folder = std::path::PathBuf::from(std::env::var_os("KERF_REAL_FOOTAGE")?);
    let mut size = (width, 0);
    let mut frames = Vec::new();
    for name in names {
        let path = folder.join(name);
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-vf", &format!("scale={width}:-2:flags=area"), "-f", "rawvideo", "-pix_fmt", "rgb48le", "pipe:"])
            .output()
            .expect("ffmpeg");
        assert!(decoded.status.success(), "{}", path.display());
        let frame: Vec<[f32; 3]> = decoded
            .stdout
            .as_chunks::<6>().0.iter()
            .map(|c| [0, 2, 4].map(|i| f32::from(u16::from_le_bytes([c[i], c[i + 1]])) / 65535.0))
            .collect();
        size.1 = frame.len() / width;
        frames.push(frame);
    }
    Some((size.0, size.1, frames))
}

/// Roto on a real shutter: a white box truck crossing a locked-off freeway at about 50 pixels a
/// frame at 1024 x 540. The shot gives its own truth: the median of its frames at each pixel is what
/// lies behind the truck, so how far a pixel has gone from it towards the truck's white is how much
/// of the exposure the truck covered there; and how long the box's front edge is blurred, against how
/// far it goes in a frame, is how long the camera's shutter was open. `KERF_REAL_FOOTAGE=… cargo test
/// --release -p kerf-track real_footage -- --ignored --nocapture`.
#[test]
#[ignore = "needs scripts/fetch-real-footage.sh"]
fn real_footage_roto_under_a_shutter() {
    let Some((w, h, frames)) = footage((2874..=2897).map(|f| format!("traffic/1080/{f}.png")), 1024) else {
        eprintln!("KERF_REAL_FOOTAGE is not set");
        return;
    };
    let n = frames.len();
    let mut plate = vec![[0.0f32; 3]; w * h];
    for (i, p) in plate.iter_mut().enumerate() {
        for c in 0..3 {
            let mut values: Vec<f32> = frames.iter().map(|f| f[i][c]).collect();
            values.sort_by(f32::total_cmp);
            p[c] = values[n / 2];
        }
    }
    let luma = |p: [f32; 3]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    // The truck's white: the brightest of what differs most from the plate, mid-shot.
    let mid = &frames[n / 2];
    let mut lit: Vec<[f32; 3]> = (0..w * h)
        .filter(|&i| (luma(mid[i]) - luma(plate[i])).abs() > 0.12)
        .map(|i| mid[i])
        .collect();
    lit.sort_by(|a, b| luma(*b).total_cmp(&luma(*a)));
    let take = (lit.len() / 10).max(1);
    let white = lit[..take].iter().fold([0.0f32; 3], |s, p| [s[0] + p[0], s[1] + p[1], s[2] + p[2]]).map(|v| v / take as f32);
    // Coverage: how far from the plate towards the white, along the line between them.
    let coverage = |frame: &[[f32; 3]], i: usize| -> f64 {
        let (b, d) = (plate[i], [white[0] - plate[i][0], white[1] - plate[i][1], white[2] - plate[i][2]]);
        let len = d.iter().map(|v| v * v).sum::<f32>();
        if len < 0.02 * 0.02 {
            return 0.0;
        }
        let along = (0..3).map(|c| (frame[i][c] - b[c]) * d[c]).sum::<f32>() / len;
        f64::from(along.clamp(0.0, 1.0))
    };
    // The truck in each frame: the largest piece of what is more than half covered, its box.
    let mut truths = Vec::new();
    for frame in &frames {
        let half: Vec<bool> = (0..w * h).map(|i| coverage(frame, i) > 0.5).collect();
        let mut piece = vec![0u32; w * h];
        let (mut largest, mut largest_size, mut next) = (0u32, 0usize, 0u32);
        for start in 0..w * h {
            if !half[start] || piece[start] != 0 {
                continue;
            }
            next += 1;
            let (mut size, mut stack) = (0usize, vec![start]);
            piece[start] = next;
            while let Some(i) = stack.pop() {
                size += 1;
                let (x, y) = (i % w, i / w);
                let around = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
                for j in around.into_iter().flatten() {
                    if half[j] && piece[j] == 0 {
                        piece[j] = next;
                        stack.push(j);
                    }
                }
            }
            if size > largest_size {
                (largest, largest_size) = (next, size);
            }
        }
        truths.push(piece.iter().map(|p| *p == largest && largest > 0).collect::<Vec<bool>>());
    }
    // From the first frame the box is in, a twentieth of the frame or more.
    let first = truths.iter().position(|t| t.iter().filter(|v| **v).count() > w * h / 20).expect("the box");
    let (frames, truths) = (&frames[first..], &truths[first..]);
    let n = frames.len();
    // The camera's shutter, from the box's front edge against the sky: a row a fifth of the way down
    // the box, read where coverage passes 0.1, 0.5 and 0.9.
    let mut edges = Vec::new();
    for (frame, truth) in frames.iter().zip(truths) {
        let top = (0..h).find(|&y| (0..w).any(|x| truth[y * w + x])).unwrap_or(0);
        let bottom = (0..h).rev().find(|&y| (0..w).any(|x| truth[y * w + x])).unwrap_or(0);
        let y = top + (bottom - top) / 5;
        let Some(front) = (0..w).rev().find(|&x| truth[y * w + x]) else { continue };
        let crossing = |level: f64| (front.saturating_sub(40)..(front + 40).min(w)).rev().find(|&x| coverage(frame, y * w + x) >= level);
        if let (Some(a), Some(b), Some(c)) = (crossing(0.9), crossing(0.5), crossing(0.1)) {
            if front + 40 < w {
                edges.push((b as f64, (c - a) as f64 / 0.8));
            }
        }
    }
    let speeds: Vec<f64> = edges.windows(2).map(|pair| pair[1].0 - pair[0].0).filter(|v| *v > 0.0).collect();
    let mut opens: Vec<f64> = edges.iter().skip(1).zip(&speeds).map(|((_, blur), speed)| blur / speed).collect();
    opens.sort_by(f64::total_cmp);
    let speed = speeds.iter().sum::<f64>() / speeds.len().max(1) as f64;
    let open = opens.get(opens.len() / 2).copied().unwrap_or(f64::NAN);

    let start = truths[0].iter().enumerate().filter(|(_, t)| **t).fold((0.0, 0.0, 0.0), |(x, y, k), (i, _)| (x + (i % w) as f64, y + (i / w) as f64, k + 1.0));
    let hints = Hints { inside: vec![[start.0 / start.2 / w as f64, start.1 / start.2 / h as f64]], ..Hints::default() };
    let rgba = |frame: usize| -> Option<(usize, usize, Vec<u8>)> {
        let f = frames.get(frame)?;
        Some((w, h, f.iter().flat_map(|p| [p[0], p[1], p[2], 1.0].map(|v| (v * 255.0).round() as u8)).collect()))
    };
    let begun = std::time::Instant::now();
    let traced = trace(n, &hints, None, rgba);
    let took = begun.elapsed().as_secs_f64() / n as f64;
    let why = shutter::fit_or_why(&traced.raw, &traced.across, traced.shortest_blur());
    let found = why.as_ref().ok().copied();
    // What it would draw let in: fit with no least blur, so the slope's shutter is taken.
    let let_in = shutter::fit_or_why(&traced.raw, &traced.across, 0.0);
    let of = |followed: &FollowedObject| {
        let (mut sum, mut worst, mut area) = (0.0, 1.0f64, 0.0);
        for (outline, truth) in followed.outlines.iter().zip(truths) {
            let polygon: Vec<[f64; 2]> = outline.iter().map(|at| [at[0] * w as f64, at[1] * h as f64]).collect();
            let (mut both, mut either, mut mine, mut theirs) = (0usize, 0usize, 0usize, 0usize);
            for y in 0..h {
                for x in 0..w {
                    let (a, b) = (in_polygon(&polygon, [x as f64 + 0.5, y as f64 + 0.5]), truth[y * w + x]);
                    both += usize::from(a && b);
                    either += usize::from(a || b);
                    mine += usize::from(a);
                    theirs += usize::from(b);
                }
            }
            let overlap = both as f64 / either.max(1) as f64;
            sum += overlap;
            worst = worst.min(overlap);
            area += mine as f64 / theirs.max(1) as f64;
        }
        let k = followed.outlines.len().max(1) as f64;
        (sum / k, worst, area / k, followed.outlines.len())
    };
    let plain = of(&traced.by_the_measure());
    let with = found
        .or(let_in.as_ref().ok().copied())
        .map(|shutter| (shutter.open, of(&trace(n, &hints, None, rgba).under_a_shutter(shutter))));
    println!(
        "roto on a real shutter, {w}x{h}, from frame {first}: the box's front goes {speed:.1} px a frame, its blur says the shutter was open {open:.2} of a frame ({} edges); \
         {took:.3} s a frame; by the measure overlap {:.3}, worst {:.3}, area {:.2} of the truth's over {} frames; shutter found {:?}; let in, open and drawn {:?}",
        edges.len(),
        plain.0,
        plain.1,
        plain.2,
        plain.3,
        why,
        with,
    );
    // Let in, it draws no better than the measure.
    if let Some((_, drawn)) = with {
        assert!(drawn.0 <= plain.0 + 0.01, "{drawn:?} against {plain:?}");
    }
    // The measure keeps up with the truck; a shutter does not come in, as it would draw no better.
    assert!(plain.0 > 0.85 && plain.1 > 0.7 && plain.3 == n, "{plain:?}");
    assert!(found.is_none(), "{found:?}");
    assert!((0.5..1.0).contains(&open), "{open}");
}
