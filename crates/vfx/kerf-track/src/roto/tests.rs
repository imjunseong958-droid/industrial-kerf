//! Following an object on synthetic sequences: a textured disk over a textured background.
#![expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]

use super::*;

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
/// the disk has the background's colours, so nothing in the picture tells them apart.
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
    let followed = follow_object(1, &hints_on_the_disk(), |n| Some(scene(n, false)));
    assert_eq!(followed.lost_at, None);
    let score = overlap(&followed.outlines[0], 0);
    assert!(score > 0.93, "{score}");
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

    /// How well it is followed over `frames`: the mean and worst overlap with what is more than half
    /// covered, and how many frames it got to.
    fn overlap(&self, size: (usize, usize), frames: usize) -> (f64, f64, usize) {
        let (w, h) = (size.0 as f64, size.1 as f64);
        let start = self.center(size, self.shutter / 2.0);
        let hints = Hints {
            inside: vec![[start[0] / w, start[1] / h]],
            ..Hints::default()
        };
        let followed = follow_object(frames, &hints, |frame| {
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

/// The frames of the real footage `scripts/fetch-real-footage.sh` fetches, `width` across, as 0 to 1
/// RGB, or `None` when `KERF_REAL_FOOTAGE` does not say where it is.
#[expect(
    clippy::type_complexity,
    reason = "carried from EffectCraft, whose tests spell out nested types"
)]
fn footage(
    names: impl Iterator<Item = String>,
    width: usize,
) -> Option<(usize, usize, Vec<Vec<[f32; 3]>>)> {
    let folder = std::path::PathBuf::from(std::env::var_os("KERF_REAL_FOOTAGE")?);
    let mut size = (width, 0);
    let mut frames = Vec::new();
    for name in names {
        let path = folder.join(name);
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args([
                "-vf",
                &format!("scale={width}:-2:flags=area"),
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb48le",
                "pipe:",
            ])
            .output()
            .expect("ffmpeg");
        assert!(decoded.status.success(), "{}", path.display());
        let frame: Vec<[f32; 3]> = decoded
            .stdout
            .as_chunks::<6>()
            .0
            .iter()
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
    let Some((w, h, frames)) =
        footage((2874..=2897).map(|f| format!("traffic/1080/{f}.png")), 1024)
    else {
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
    let white = lit[..take]
        .iter()
        .fold([0.0f32; 3], |s, p| [s[0] + p[0], s[1] + p[1], s[2] + p[2]])
        .map(|v| v / take as f32);
    // Coverage: how far from the plate towards the white, along the line between them.
    let coverage = |frame: &[[f32; 3]], i: usize| -> f64 {
        let (b, d) = (
            plate[i],
            [
                white[0] - plate[i][0],
                white[1] - plate[i][1],
                white[2] - plate[i][2],
            ],
        );
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
                let around = [
                    (x > 0).then(|| i - 1),
                    (x + 1 < w).then(|| i + 1),
                    (y > 0).then(|| i - w),
                    (y + 1 < h).then(|| i + w),
                ];
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
        truths.push(
            piece
                .iter()
                .map(|p| *p == largest && largest > 0)
                .collect::<Vec<bool>>(),
        );
    }
    // From the first frame the box is in, a twentieth of the frame or more.
    let first = truths
        .iter()
        .position(|t| t.iter().filter(|v| **v).count() > w * h / 20)
        .expect("the box");
    let (frames, truths) = (&frames[first..], &truths[first..]);
    let n = frames.len();
    // The camera's shutter, from the box's front edge against the sky: a row a fifth of the way down
    // the box, read where coverage passes 0.1, 0.5 and 0.9.
    let mut edges = Vec::new();
    for (frame, truth) in frames.iter().zip(truths) {
        let top = (0..h)
            .find(|&y| (0..w).any(|x| truth[y * w + x]))
            .unwrap_or(0);
        let bottom = (0..h)
            .rev()
            .find(|&y| (0..w).any(|x| truth[y * w + x]))
            .unwrap_or(0);
        let y = top + (bottom - top) / 5;
        let Some(front) = (0..w).rev().find(|&x| truth[y * w + x]) else {
            continue;
        };
        let crossing = |level: f64| {
            (front.saturating_sub(40)..(front + 40).min(w))
                .rev()
                .find(|&x| coverage(frame, y * w + x) >= level)
        };
        if let (Some(a), Some(b), Some(c)) = (crossing(0.9), crossing(0.5), crossing(0.1))
            && front + 40 < w
        {
            edges.push((b as f64, (c - a) as f64 / 0.8));
        }
    }
    let speeds: Vec<f64> = edges
        .windows(2)
        .map(|pair| pair[1].0 - pair[0].0)
        .filter(|v| *v > 0.0)
        .collect();
    let mut opens: Vec<f64> = edges
        .iter()
        .skip(1)
        .zip(&speeds)
        .map(|((_, blur), speed)| blur / speed)
        .collect();
    opens.sort_by(f64::total_cmp);
    let speed = speeds.iter().sum::<f64>() / speeds.len().max(1) as f64;
    let open = opens.get(opens.len() / 2).copied().unwrap_or(f64::NAN);

    let start = truths[0]
        .iter()
        .enumerate()
        .filter(|(_, t)| **t)
        .fold((0.0, 0.0, 0.0), |(x, y, k), (i, _)| {
            (x + (i % w) as f64, y + (i / w) as f64, k + 1.0)
        });
    let hints = Hints {
        inside: vec![[start.0 / start.2 / w as f64, start.1 / start.2 / h as f64]],
        ..Hints::default()
    };
    let rgba = |frame: usize| -> Option<(usize, usize, Vec<u8>)> {
        let f = frames.get(frame)?;
        Some((
            w,
            h,
            f.iter()
                .flat_map(|p| [p[0], p[1], p[2], 1.0].map(|v| (v * 255.0).round() as u8))
                .collect(),
        ))
    };
    let begun = std::time::Instant::now();
    let traced = trace(n, &hints, rgba);
    let took = begun.elapsed().as_secs_f64() / n as f64;
    let why = shutter::fit_or_why(&traced.raw, &traced.across, traced.shortest_blur());
    let found = why.as_ref().ok().copied();
    // What it would draw let in: fit with no least blur, so the slope's shutter is taken.
    let let_in = shutter::fit_or_why(&traced.raw, &traced.across, 0.0);
    let of = |followed: &FollowedObject| {
        let (mut sum, mut worst, mut area) = (0.0, 1.0f64, 0.0);
        for (outline, truth) in followed.outlines.iter().zip(truths) {
            let polygon: Vec<[f64; 2]> = outline
                .iter()
                .map(|at| [at[0] * w as f64, at[1] * h as f64])
                .collect();
            let (mut both, mut either, mut mine, mut theirs) = (0usize, 0usize, 0usize, 0usize);
            for y in 0..h {
                for x in 0..w {
                    let (a, b) = (
                        in_polygon(&polygon, [x as f64 + 0.5, y as f64 + 0.5]),
                        truth[y * w + x],
                    );
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
    let with = found.or(let_in.as_ref().ok().copied()).map(|shutter| {
        (
            shutter.open,
            of(&trace(n, &hints, rgba).under_a_shutter(shutter)),
        )
    });
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

/// The warp as it was worked out before only the pixels the matte can come to were: every pixel.
fn warped_everywhere(
    flow: &Flow,
    matte: &[u8],
    (w, h): (usize, usize),
    moved: [i64; 2],
) -> Vec<f32> {
    let source: Vec<f32> = matte.iter().map(|v| f32::from(*v)).collect();
    let (went_x, went_y) = (moved[0] as f64, moved[1] as f64);
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            let v = flow.at(x, y);
            let (vx, vy) = (f64::from(v[0]), f64::from(v[1]));
            let (vx, vy) = if vx.hypot(vy) > (vx - went_x).hypot(vy - went_y) {
                (0.0, 0.0)
            } else {
                (vx, vy)
            };
            matting::sample(
                &source,
                w,
                h,
                x + vx - moved[0] as f64,
                y + vy - moved[1] as f64,
            )
        })
        .collect()
}

/// The same numbers bit for bit, a `NaN` or a zero's sign included.
fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}

#[test]
fn a_distance_measured_near_the_matte_is_the_distance_measured_everywhere() {
    let (w, h) = (97, 61);
    let disk = |cx: f64, cy: f64, r: f64| -> Vec<u8> {
        (0..w * h)
            .map(|i| u8::from(((i % w) as f64 - cx).hypot((i / w) as f64 - cy) < r))
            .collect()
    };
    let both =
        |a: Vec<u8>, b: Vec<u8>| -> Vec<u8> { a.iter().zip(&b).map(|(a, b)| a | b).collect() };
    let mut holed = disk(40.0, 30.0, 18.0);
    for (at, inner) in disk(43.0, 28.0, 5.0).iter().enumerate() {
        holed[at] &= 1 - inner;
    }
    let mattes = [
        ("a disk", disk(40.0, 30.0, 12.0)),
        ("a disk with a hole", holed),
        (
            "a disk and a speck far from it",
            both(disk(20.0, 20.0, 9.0), disk(90.0, 55.0, 1.5)),
        ),
        ("one over the frame's corner", disk(0.0, 0.0, 14.0)),
        ("one over its far corner", disk(96.0, 60.0, 14.0)),
        ("one pixel", disk(50.0, 30.0, 0.5)),
        ("nothing", vec![0; w * h]),
        ("everything", vec![1; w * h]),
    ];
    for (name, matte) in mattes {
        let everywhere = matting::signed_distance(&matte, w, h);
        for reach in [0.0, 3.0, 7.5, 24.0, 200.0] {
            let near = matting::signed_distance_near(&matte, w, h, reach);
            for (at, (near, all)) in near.iter().zip(&everywhere).enumerate() {
                if *near == matting::FAR_OUTSIDE {
                    // Not measured: outside, and further than was asked for.
                    assert!(
                        f64::from(*all) < -reach,
                        "{name}, reach {reach}, pixel {at}: {all}"
                    );
                } else {
                    assert_eq!(
                        near.to_bits(),
                        all.to_bits(),
                        "{name}, reach {reach}, pixel {at}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_matte_cleaned_in_its_box_is_the_matte_cleaned_everywhere() {
    let (w, h) = (64, 48);
    let mut seen = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        seen ^= seen << 13;
        seen ^= seen >> 7;
        seen ^= seen << 17;
        seen
    };
    for round in 0..400 {
        // Blobs, holes in them, specks, and seeds of both kinds, anywhere, the frame's edge included.
        let mut labels = vec![0u8; w * h];
        let mut paint = |value: u8, most: u64, labels: &mut Vec<u8>| {
            let (cx, cy, r) = (
                (next() % w as u64) as i64,
                (next() % h as u64) as i64,
                (next() % most) as i64,
            );
            for y in (cy - r).max(0)..(cy + r + 1).min(h as i64) {
                for x in (cx - r).max(0)..(cx + r + 1).min(w as i64) {
                    if (x - cx).pow(2) + (y - cy).pow(2) <= r * r {
                        labels[y as usize * w + x as usize] = value;
                    }
                }
            }
        };
        for _ in 0..(round % 4) {
            paint(1, 14, &mut labels);
        }
        for _ in 0..(round % 5) {
            paint(0, 4, &mut labels);
        }
        for _ in 0..(round % 3) {
            paint(1, 2, &mut labels);
        }
        let mut seeds = vec![false; w * h];
        let mut bg_seeds = vec![false; w * h];
        for _ in 0..(round % 6) {
            let at = (next() % (w * h) as u64) as usize;
            seeds[at] = true;
            bg_seeds[(next() % (w * h) as u64) as usize] = round % 2 == 0;
        }
        let (mut boxed, mut every) = (labels.clone(), labels.clone());
        clean(&mut boxed, &seeds, &bg_seeds, w, h);
        clean_all(&mut every, &seeds, &bg_seeds, w, h);
        assert_eq!(boxed, every, "round {round}");
    }
}

#[test]
fn a_frame_is_cut_as_it_was_before_less_of_it_was_worked_out() {
    // What each step gives, against the step as it was: the warp over every pixel, the distance over
    // the whole frame twice, and the picture put in light a pixel at a time.
    let size = (320, 200);
    let radius = 40.0;
    for (speed, alike) in [(3.0, false), (14.0, false), (40.0, false), (6.0, true)] {
        let picture = |n: usize| {
            let (w, h, rgba) = fast_scene(size, radius, speed, n, alike);
            Picture::of(w, h, &rgba)
        };
        let center = fast_center(size, radius, speed, 0);
        let hints = Hints {
            inside: vec![[center[0] / size.0 as f64, center[1] / size.1 as f64]],
            ..Hints::default()
        };
        let (w, h) = size;
        let mut previous = picture(0);
        let (mut matte, fg, bg) = first_matte(&previous, &hints).unwrap();
        for frame in 1..6 {
            let next = picture(frame);
            let flow = block_flow(&next.image, &previous.image, 8, 4);
            for moved in [[0, 0], [9, -4], [-30, 17]] {
                assert!(
                    same_bits(
                        &warped_by(&flow, &matte, size, moved),
                        &warped_everywhere(&flow, &matte, size, moved)
                    ),
                    "the warp at {speed} px a frame, frame {frame}, moved {moved:?}"
                );
            }
            // The core, from the one distance, against one measured over the whole frame for it.
            let soft = warped_by(&flow, &matte, size, [0, 0]);
            let flowed: Vec<u8> = soft.iter().map(|v| u8::from(*v >= 0.5)).collect();
            let reach = band_of(size, [0, 0]);
            let everywhere = matting::signed_distance(&flowed, w, h);
            let near = matting::signed_distance_near(&flowed, w, h, reach);
            let free = |signed: &[f32]| -> Vec<bool> {
                signed.iter().map(|d| f64::from(d.abs()) <= reach).collect()
            };
            assert_eq!(
                free(&near),
                free(&everywhere),
                "where the cut is free, frame {frame}"
            );
            assert_eq!(
                core_of(&flowed, &near, reach),
                core_of(&flowed, &everywhere, reach),
                "what is held to, frame {frame}"
            );
            // The cut cleaned in the box around it, against every pixel gone through.
            let blurred = matting::gauss(&soft, w, h, (reach / 4.0).max(1.0));
            let mut uncleaned = flowed.clone();
            cut(
                &next.rgb,
                w,
                h,
                &mut uncleaned,
                &free(&near),
                &vec![0u8; w * h],
                &fg,
                &bg,
                Some((&blurred, PRIOR)),
            );
            let seeds = core_of(&flowed, &near, reach);
            let (mut boxed, mut every) = (uncleaned.clone(), uncleaned.clone());
            clean(&mut boxed, &seeds, &vec![false; w * h], w, h);
            clean_all(&mut every, &seeds, &vec![false; w * h], w, h);
            assert_eq!(
                boxed, every,
                "the cleaning at {speed} px a frame, frame {frame}"
            );
            let (labels, _) = next_matte(&previous, &matte, &next, (&fg, &bg), false);
            if moved_by(&previous, &matte, &next) == [0, 0] {
                assert_eq!(
                    labels, every,
                    "the matte at {speed} px a frame, frame {frame}"
                );
            }
            // The softness, against the picture in light pixel by pixel and the distance everywhere.
            let edge = edge_of(&labels, w, h);
            let traced = Edge::of(&edge, OUTLINE_CORNERS).unwrap();
            let corners = Places::of(&traced, OUTLINE_CORNERS).first(&traced);
            assert_eq!(
                soft::measured(&next, &labels, &corners),
                soft::measured_everywhere(&next, &labels, &corners),
                "the softness at {speed} px a frame, frame {frame}"
            );
            matte = labels;
            previous = next;
        }
    }
}
