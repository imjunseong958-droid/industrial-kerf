//! Particles, worked out rather than simulated: each particle is born at a set moment and follows
//! a formula from there, so any frame can be drawn first and the same frame always looks the same,
//! which a simulation stepped from frame to frame could not promise.

use kerf_draw::Image;

/// The most particles alive at once.
const MOST_ALIVE: f64 = 100_000.0;

/// What an emitter does, in pixels and seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Emitter {
    /// Particles born a second.
    pub rate: f64,
    /// How long each lives.
    pub life: f64,
    /// How fast they leave, and how much of that varies, 0 to 1.
    pub velocity: f64,
    pub vary: f64,
    /// The way they leave, in degrees clockwise from straight right, and how wide a fan.
    pub direction: f64,
    pub spread: f64,
    /// Pulling down, in pixels a second a second.
    pub gravity: f64,
    /// Across, as they start and as they end.
    pub size: (f64, f64),
    /// Premultiplied linear light, as they start and as they end.
    pub color: ([f32; 4], [f32; 4]),
    /// How much of a particle's radius fades out, 0 for a hard dot, 1 for a soft one.
    pub soft: f64,
    /// Where they are born, and how wide and tall an area around it.
    pub origin: [f64; 2],
    pub area: [f64; 2],
    pub seed: u64,
}

/// A number from 0 to 1 that stands for `index` of what is drawn of particle `id`.
fn random(seed: u64, id: u64, index: u64) -> f64 {
    let mut z = seed
        .wrapping_add(id.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(index.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

impl Emitter {
    /// Where particle `id` is `age` seconds after it was born.
    fn place(&self, id: u64, age: f64) -> [f64; 2] {
        let turn = random(self.seed, id, 0) - 0.5;
        let angle = (self.direction + turn * self.spread).to_radians();
        let speed = self.velocity * (1.0 + self.vary * (random(self.seed, id, 1) - 0.5) * 2.0);
        let start = [
            self.origin[0] + (random(self.seed, id, 2) - 0.5) * self.area[0],
            self.origin[1] + (random(self.seed, id, 3) - 0.5) * self.area[1],
        ];
        [
            start[0] + angle.cos() * speed * age,
            start[1] + angle.sin() * speed * age + self.gravity * age * age / 2.0,
        ]
    }

    /// Those alive at `time` seconds after the emitter began, oldest first.
    fn alive(&self, time: f64) -> std::ops::Range<u64> {
        if self.rate <= 0.0 || self.life <= 0.0 || time < 0.0 {
            return 0..0;
        }
        let rate = self.rate.min(MOST_ALIVE / self.life);
        let first = ((time - self.life) * rate).floor().max(0.0) as u64;
        let last = (time * rate).floor() as u64 + 1;
        first..last
    }

    /// Draw it over `image` at `time` seconds after it began.
    pub fn draw(&self, image: &mut Image, time: f64) {
        let rate = self.rate.min(MOST_ALIVE / self.life.max(1e-6));
        let (w, h) = (image.w as i64, image.h as i64);
        for id in self.alive(time) {
            let age = time - id as f64 / rate;
            if age < 0.0 || age >= self.life {
                continue;
            }
            let along = age / self.life;
            let size = self.size.0 + (self.size.1 - self.size.0) * along;
            let radius = size / 2.0;
            if radius <= 0.0 {
                continue;
            }
            let paint: [f32; 4] = std::array::from_fn(|k| {
                self.color.0[k] + (self.color.1[k] - self.color.0[k]) * along as f32
            });
            if paint[3] <= 0.0 {
                continue;
            }
            let at = self.place(id, age);
            let reach = radius + 1.0;
            let (x0, x1) = (
                ((at[0] - reach).floor() as i64).max(0),
                ((at[0] + reach).ceil() as i64).min(w),
            );
            let (y0, y1) = (
                ((at[1] - reach).floor() as i64).max(0),
                ((at[1] + reach).ceil() as i64).min(h),
            );
            let inner = radius * (1.0 - self.soft.clamp(0.0, 1.0));
            for y in y0..y1 {
                for x in x0..x1 {
                    let d = (x as f64 + 0.5 - at[0]).hypot(y as f64 + 0.5 - at[1]);
                    // Edge antialiased; a soft one fades from `inner` out.
                    let edge = (radius - d + 0.5).clamp(0.0, 1.0);
                    let fade = if d <= inner || radius <= inner {
                        1.0
                    } else {
                        let t = ((d - inner) / (radius - inner)).clamp(0.0, 1.0);
                        1.0 - t * t * (3.0 - 2.0 * t)
                    };
                    let covered = (edge * fade) as f32;
                    if covered <= 0.0 {
                        continue;
                    }
                    let start = (y * image.w as i64 + x) as usize * 4;
                    let pixel = &mut image.px[start..start + 4];
                    let keep = 1.0 - paint[3] * covered;
                    for (value, paint) in pixel.iter_mut().zip(paint) {
                        *value = paint * covered + *value * keep;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emitter() -> Emitter {
        Emitter {
            rate: 100.0,
            life: 1.0,
            velocity: 100.0,
            vary: 0.0,
            direction: 0.0,
            spread: 0.0,
            gravity: 0.0,
            size: (4.0, 4.0),
            color: ([1.0; 4], [1.0; 4]),
            soft: 0.0,
            origin: [50.0, 100.0],
            area: [0.0, 0.0],
            seed: 1,
        }
    }

    fn drawn(emitter: &Emitter, time: f64) -> Image {
        let mut image = Image::new(300, 200);
        emitter.draw(&mut image, time);
        image
    }

    fn ink(image: &Image) -> f32 {
        image.px.chunks_exact(4).map(|pixel| pixel[3]).sum()
    }

    #[test]
    fn any_frame_can_come_first_and_is_the_same_every_time() {
        let e = Emitter {
            spread: 360.0,
            vary: 0.5,
            ..emitter()
        };
        let direct = drawn(&e, 2.5);
        for t in [0.1, 1.0, 2.0] {
            drawn(&e, t);
        }
        assert_eq!(drawn(&e, 2.5), direct);
        let other = Emitter { seed: 2, ..e };
        assert_ne!(drawn(&other, 2.5), direct);
    }

    #[test]
    fn they_travel_at_their_velocity_and_gravity_bends_them() {
        // Straight right from x = 50 at 100 pixels a second, for a second: as far as x = 150.
        let line = drawn(&emitter(), 3.0);
        assert!(line.get(149, 100)[3] > 0.5);
        assert_eq!(line.get(153, 100)[3], 0.0);
        assert_eq!(line.get(100, 103)[3], 0.0);
        // Half a second on, gravity has pulled it down g t² / 2.
        let e = Emitter {
            gravity: 160.0,
            ..emitter()
        };
        let bent = drawn(&e, 3.0);
        // At x = 100 it is half a second old, 20 pixels down.
        assert!(bent.get(100, 120)[3] > 0.5);
        assert_eq!(bent.get(100, 100)[3], 0.0);
    }

    #[test]
    fn none_before_the_start_and_as_many_as_their_life_allows() {
        assert_eq!(ink(&drawn(&emitter(), -0.5)), 0.0);
        assert_eq!(emitter().alive(0.5), 0..51);
        assert_eq!(emitter().alive(3.0), 200..301);
        assert_eq!(
            Emitter {
                rate: 0.0,
                ..emitter()
            }
            .alive(1.0),
            0..0
        );
    }

    #[test]
    fn they_fade_and_shrink_as_they_age() {
        let e = Emitter {
            rate: 0.1,
            life: 2.0,
            velocity: 0.0,
            size: (40.0, 0.0),
            color: ([1.0; 4], [0.0; 4]),
            origin: [150.0, 100.0],
            ..emitter()
        };
        let young = ink(&drawn(&e, 0.0));
        let old = ink(&drawn(&e, 1.0));
        assert!(old < young * 0.5, "{young} then {old}");
        // Half life: half the size across and half as opaque.
        let at_half = drawn(&e, 1.0).get(150, 100)[3];
        assert!(at_half > 0.4 && at_half < 0.8, "{at_half}");
    }

    #[test]
    fn a_soft_one_fades_to_its_edge_and_a_hard_one_does_not() {
        let e = Emitter {
            rate: 1.0,
            velocity: 0.0,
            size: (40.0, 40.0),
            origin: [150.0, 100.0],
            ..emitter()
        };
        let hard = drawn(&e, 0.0);
        let soft = drawn(&Emitter { soft: 1.0, ..e }, 0.0);
        assert_eq!(hard.get(150, 85)[3], 1.0);
        let near_edge = soft.get(150, 85)[3];
        assert!(near_edge > 0.0 && near_edge < 0.3, "{near_edge}");
        assert!(soft.get(150, 100)[3] > 0.95);
    }

    #[test]
    fn a_fan_goes_both_ways_and_an_area_spreads_their_birth() {
        let fan = Emitter {
            spread: 90.0,
            rate: 500.0,
            ..emitter()
        };
        let image = drawn(&fan, 3.0);
        let (mut above, mut below) = (0.0, 0.0);
        for y in 0..200 {
            for x in 0..300 {
                if y < 100 {
                    above += image.get(x, y)[3];
                } else {
                    below += image.get(x, y)[3];
                }
            }
        }
        assert!(above > 100.0 && below > 100.0, "{above} {below}");
        let area = Emitter {
            velocity: 0.0,
            area: [100.0, 0.0],
            ..emitter()
        };
        let spread = drawn(&area, 3.0);
        assert!(spread.get(10, 100)[3] > 0.0 || spread.get(90, 100)[3] > 0.0);
        assert_eq!(spread.get(150, 100)[3], 0.0);
    }

    #[test]
    fn too_many_are_held_to_a_number() {
        let e = Emitter {
            rate: 1e9,
            life: 100.0,
            ..emitter()
        };
        let alive = e.alive(1000.0);
        assert!(alive.end - alive.start <= 100_002);
    }
}
