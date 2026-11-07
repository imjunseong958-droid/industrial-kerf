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
