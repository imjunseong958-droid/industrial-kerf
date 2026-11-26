//! Moving-frame transitions: Animation, Slide and the camera-style Smart Tools.

use super::cards::{eye, focal_for, Card, V3};
use super::{
    bell, ease_in, ease_in_out, ease_out, exposure, following, line_blur, mid_mix, mix, over,
    paint, samples_over, scale4, temporal, zoom_blur, Tx, MOST_MOMENTS,
};
use kerf_draw::Image;

/// One textured card in a 3D scene.
#[derive(Clone, Copy)]
pub(crate) struct Layer<'a> {
    pub card: Card,
    pub front: &'a Image,
    /// Visible part of the card in image pixels (x0, y0, x1, y1).
    pub clip: [f32; 4],
    pub opacity: f32,
}

impl<'a> Layer<'a> {
    pub fn new(card: Card, front: &'a Image) -> Self {
        Layer {
            card,
            front,
            clip: [0.0, 0.0, front.w as f32, front.h as f32],
            opacity: 1.0,
        }
    }
    fn clipped(mut self, c: [f32; 4]) -> Self {
        self.clip = c;
        self
    }
    /// Where the corners of the card's visible part are on the screen, seen from `eye`.
    fn corners_seen(&self, eye: V3) -> [Option<[f32; 2]>; 4] {
        let [x0, y0, x1, y1] = self.clip;
        [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(lx, ly)| self.card.seen(eye, lx, ly))
    }
}

/// Composite the layers seen through screen pixel (x, y), nearest last. Those whose [`bounds`] it is
/// outside are not looked at, as nothing of them shows there.
fn composite(layers: &[Layer], bounds: &[[f32; 4]], eye: V3, x: f32, y: f32) -> [f32; 4] {
    const MAX: usize = 8;
    let mut hits = [(0.0f32, [0.0f32; 4]); MAX];
    let mut nh = 0;
    for (l, b) in layers.iter().zip(bounds) {
        if x < b[0] || y < b[1] || x > b[2] || y > b[3] {
            continue;
        }
        let Some((lx, ly, depth, front)) = l.card.hit(eye, x, y) else {
            continue;
        };
        let [x0, y0, x1, y1] = l.clip;
        let edge = (lx - x0).min(x1 - lx).min(ly - y0).min(y1 - ly);
        let cov = (edge + 0.5).clamp(0.0, 1.0);
        if cov <= 0.0 {
            continue;
        }
        // A card is one-sided: turned away, it is not seen.
        if !front {
            continue;
        }
        let c = l.front.sample_bilinear_clamped(lx, ly);
        let k = (cov * l.opacity).clamp(0.0, 1.0);
        if nh < MAX {
            hits[nh] = (depth, scale4(c, k));
            nh += 1;
        }
    }
    let hits = &mut hits[..nh];
    hits.sort_by(|a, b| b.0.total_cmp(&a.0));
    hits.iter().fold([0.0; 4], |acc, (_, c)| over(acc, *c))
}

/// The box on the screen outside which nothing of a layer shows, seen from `eye`: around the corners of
/// its visible part and the half pixel past them that its edges cover, and a pixel more. Everywhere
/// when some of that is not in front of the eye.
fn bounds(layer: &Layer, eye: V3) -> [f32; 4] {
    let everywhere = [
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::INFINITY,
    ];
    let [x0, y0, x1, y1] = layer.clip;
    let (x0, y0, x1, y1) = (x0 - 0.5, y0 - 0.5, x1 + 0.5, y1 + 0.5);
    let mut b = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for (lx, ly) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        let Some([sx, sy]) = layer.card.seen(eye, lx, ly) else {
            return everywhere;
        };
        b = [b[0].min(sx), b[1].min(sy), b[2].max(sx), b[3].max(sy)];
    }
    [b[0] - 1.0, b[1] - 1.0, b[2] + 1.0, b[3] + 1.0]
}

/// Paint a card scene: its cards at each of its moments, averaged (motion blur).
fn scene(t: &Tx, found: &CardScene) -> Image {
    let scenes: Vec<Vec<Layer>> = found.moments(t).iter().map(|&q| (found.build)(q)).collect();
    let e = eye(t.wf, t.hf, found.focal);
    let seen: Vec<Vec<[f32; 4]>> = scenes
        .iter()
        .map(|layers| layers.iter().map(|layer| bounds(layer, e)).collect())
        .collect();
    let inv = 1.0 / scenes.len() as f32;
    paint(t.w, t.h, |x, y| {
        let mut acc = [0.0; 4];
        for (s, b) in scenes.iter().zip(&seen) {
            let c = composite(s, b, e, x, y);
            for k in 0..4 {
                acc[k] += c[k];
            }
        }
        scale4(acc, inv)
    })
}

/// What a scene of cards takes: how far off the eye is, how long the shutter is open (in progress
/// units, 0 for no motion blur), and the cards at each moment.
pub(crate) struct CardScene<'a, 'b> {
    pub focal: f32,
    pub shutter: f32,
    pub build: Box<dyn Fn(f32) -> Vec<Layer<'a>> + 'b>,
}

impl CardScene<'_, '_> {
    /// The moments whose cards a pixel averages for its motion blur, evenly over the shutter around
    /// `t.p`: as many as keep the cards within [`BLUR_STEP`](super::BLUR_STEP) pixels on the screen
    /// from one moment to the next where they move fastest, and [`MOST_MOMENTS`] at most
    /// ([`samples_over`]). Just `t.p` with the shutter closed. The CPU and the GPU's kernel draw the cards at the same moments.
    pub fn moments(&self, t: &Tx) -> Vec<f32> {
        let n = self.samples(t);
        if n <= 1 {
            return vec![t.p];
        }
        (0..n)
            .map(|i| (t.p + (i as f32 / (n - 1) as f32 - 0.5) * self.shutter).clamp(0.0, 1.0))
            .collect()
    }

    /// How many moments the shutter takes: enough for how fast the corners of the cards' visible parts
    /// move on the screen while it is open.
    fn samples(&self, t: &Tx) -> usize {
        let eye = eye(t.wf, t.hf, self.focal);
        samples_over(t.p, self.shutter, MOST_MOMENTS, |q| {
            (self.build)(q)
                .iter()
                .flat_map(|layer| layer.corners_seen(eye))
                .collect()
        })
    }
}

fn boxed<'a, 'b>(
    focal: f32,
    shutter: f32,
    build: impl Fn(f32) -> Vec<Layer<'a>> + 'b,
) -> CardScene<'a, 'b> {
    CardScene {
        focal,
        shutter,
        build: Box::new(build),
    }
}

/// The cards of a transition that is a scene of them, which the CPU paints and the GPU's kernel reads.
pub(crate) fn card_scene<'a, 'b>(id: &str, t: &'b Tx<'a>) -> Option<CardScene<'a, 'b>> {
    let (w, h) = (t.wf, t.hf);
    let (dx, dy) = t.dir();
    let horiz = dx != 0.0;
    let travel = t.travel();
    let far = focal_for(w, h, 0.0);
    Some(match id {
        "push" => boxed(far, exposure(t, 0.12), move |q| {
            let e = ease_in_out(q);
            vec![
                Layer::new(
                    Card::flat().translate([dx * travel * e, dy * travel * e, 0.0]),
                    t.a,
                ),
                Layer::new(
                    Card::flat().translate([dx * travel * (e - 1.0), dy * travel * (e - 1.0), 0.0]),
                    t.b,
                ),
            ]
        }),
        "slide" => boxed(far, exposure(t, 0.12), move |q| {
            let e = ease_in_out(q);
            vec![
                Layer::new(Card::flat(), t.a),
                Layer::new(
                    Card::flat().translate([dx * travel * (e - 1.0), dy * travel * (e - 1.0), 0.0]),
                    t.b,
                ),
            ]
        }),
        "split" => {
            let vertical_cut = t.choice("orientation") == 0;
            boxed(far, exposure(t, 0.12), move |q| {
                let e = ease_in_out(q);
                let (half_l, half_r) = if vertical_cut {
                    ([0.0, 0.0, w / 2.0, h], [w / 2.0, 0.0, w, h])
                } else {
                    ([0.0, 0.0, w, h / 2.0], [0.0, h / 2.0, w, h])
                };
                let (ox, oy) = if vertical_cut {
                    (w / 2.0 * e, 0.0)
                } else {
                    (0.0, h / 2.0 * e)
                };
                vec![
                    Layer::new(Card::flat(), t.b),
                    Layer::new(Card::flat().translate([-ox, -oy, -0.01]), t.a).clipped(half_l),
                    Layer::new(Card::flat().translate([ox, oy, -0.01]), t.a).clipped(half_r),
                ]
            })
        }
        "roll" => {
            // A and B ride a big wheel whose hub sits below (or beside) the frame.
            let r = if horiz { h * 2.0 } else { w * 2.0 };
            let hub: V3 = if horiz {
                [w / 2.0, h / 2.0 + r, 0.0]
            } else {
                [w / 2.0 - r, h / 2.0, 0.0]
            };
            let gap = (if horiz { w } else { h }) * 1.15 / r;
            let sign = if horiz { dx.signum() } else { dy.signum() };
            boxed(far, exposure(t, 0.1), move |q| {
                let a = ease_in_out(q) * gap * sign;
                vec![
                    Layer::new(Card::flat().rot_z(hub, a), t.a),
                    Layer::new(Card::flat().rot_z(hub, a - gap * sign), t.b),
                ]
            })
        }
        _ => return None,
    })
}

/// The two clips where Whip has pushed them, fast in the middle: a steep sigmoid normalised to 0..1.
pub(crate) fn whip_layers<'a>(t: &Tx<'a>) -> Vec<Layer<'a>> {
    let (dx, dy) = t.dir();
    let travel = t.travel();
    let p = t.p;
    let s = |u: f32| 1.0 / (1.0 + (-(u - 0.5) * 14.0).exp());
    let e = (s(p) - s(0.0)) / (s(1.0) - s(0.0));
    vec![
        Layer::new(
            Card::flat().translate([dx * travel * e, dy * travel * e, 0.0]),
            t.a,
        ),
        Layer::new(
            Card::flat().translate([dx * travel * (e - 1.0), dy * travel * (e - 1.0), 0.0]),
            t.b,
        ),
    ]
}

/// Film Roll's motion blur: its frames roll `step` along the way it goes, eased.
pub(crate) fn film_roll_blur(t: &Tx, step: f32) -> (f32, usize) {
    following(t, 0.1, |q| vec![Some([step * ease_in_out(q), 0.0])])
}

pub(super) fn apply(id: &str, t: &Tx) -> Option<Image> {
    if let Some(found) = card_scene(id, t) {
        return Some(scene(t, &found));
    }
    let (w, h) = (t.wf, t.hf);
    let (dx, dy) = t.dir();
    let horiz = dx != 0.0;
    let travel = t.travel();
    let far = focal_for(w, h, 0.0);
    Some(match id {
        "whip" => {
            let pushed = scene(t, &boxed(far, 0.0, |_| whip_layers(t)));
            let len = t.frac("blur") * bell(t.p) * travel * 0.18;
            if len < 0.75 {
                pushed
            } else {
                let n = super::taps(len * 2.0);
                paint(t.w, t.h, |x, y| {
                    line_blur(&pushed, x, y, dx * len, dy * len, n)
                })
            }
        }
        "film_roll" => {
            let gap = t.px("gap");
            let gcol = t.color("gap_color");
            let step = travel + gap;
            let (sh, n) = film_roll_blur(t, step);
            let inside = move |u: f32, v: f32| u >= 0.0 && v >= 0.0 && u < w && v < h;
            paint(t.w, t.h, |x, y| {
                temporal(x, y, t.p, sh, n, &|x, y, q| {
                    let off = step * ease_in_out(q);
                    let (ax, ay) = (x - dx * off, y - dy * off);
                    if inside(ax, ay) {
                        return t.sac(ax, ay);
                    }
                    let (bx, by) = (ax + dx * step, ay + dy * step);
                    if inside(bx, by) {
                        return t.sbc(bx, by);
                    }
                    // the gap between the frames, with sprocket holes along both edges
                    let across = if horiz { y } else { x };
                    let across_len = if horiz { h } else { w };
                    let along = if horiz { ax } else { ay };
                    let edge_band = (across / across_len < 0.12) || (across / across_len > 0.88);
                    let pitch = across_len / 14.0;
                    let f = (across / pitch).fract();
                    let g = ((along.rem_euclid(step) - travel) / gap.max(1.0)).clamp(0.0, 1.0);
                    let hole = !edge_band && (0.25..0.75).contains(&f) && (0.3..0.7).contains(&g);
                    if hole {
                        [0.5, 0.5, 0.5, 1.0]
                    } else {
                        gcol
                    }
                })
            })
        }
        // ---------------- Smart Tools (camera-style) ----------------
        "motion_camera" => {
            let zoom = t.frac("zoom");
            let rot = t.num("rotation").to_radians();
            let mb = t.frac("motion_blur");
            let p = t.p;
            let k = mid_mix(p);
            // A pushes in and turns; B arrives from the same push, settling to rest
            let (za, ra) = (1.0 + zoom * ease_in(p.min(0.7) / 0.7), rot * ease_in(p));
            let (zb, rb) = (
                1.0 + zoom * (1.0 - ease_out(((p - 0.3) / 0.7).clamp(0.0, 1.0))),
                -rot * (1.0 - ease_out(p)),
            );
            let blur = mb * bell(p) * 0.35;
            let n = super::taps(blur * w * 0.3);
            paint(t.w, t.h, |x, y| {
                let sample = |img: &Image, z: f32, r: f32| {
                    let (s, co) = (-r).sin_cos();
                    let (ux, uy) = ((x - w / 2.0) / z, (y - h / 2.0) / z);
                    let (lx, ly) = (ux * co - uy * s + w / 2.0, ux * s + uy * co + h / 2.0);
                    zoom_blur(img, lx, ly, w / 2.0, h / 2.0, blur, n)
                };
                let a = if k < 1.0 {
                    sample(t.a, za, ra)
                } else {
                    [0.0; 4]
                };
                let b = if k > 0.0 {
                    sample(t.b, zb, rb)
                } else {
                    [0.0; 4]
                };
                mix(a, b, k)
            })
        }
        "motion_tween" => {
            let s = t.frac("scale");
            let p = t.p;
            let e = ease_in_out(p);
            let (za, zb) = (1.0 + s * e, 1.0 + s * (e - 1.0));
            paint(t.w, t.h, |x, y| {
                let a = t.sac((x - w / 2.0) / za + w / 2.0, (y - h / 2.0) / za + h / 2.0);
                let b = t.sbc((x - w / 2.0) / zb + w / 2.0, (y - h / 2.0) / zb + h / 2.0);
                mix(a, b, e)
            })
        }
        _ => return None,
    })
}
