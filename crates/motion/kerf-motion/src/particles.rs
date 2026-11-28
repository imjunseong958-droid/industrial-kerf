//! Particles: closed-form ballistic trajectories by default, or deterministic `kerf-sim`
//! physics (air drag, curl-noise turbulence, and floor bounce) backed by a bounded temporal
//! checkpoint cache so any frame can still be drawn in arbitrary order with bit-identical results.

use kerf_draw::Image;
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::Time;
use kerf_sim::{ParticleFieldSpec, WorldSpec, WorldTimeline, DEFAULT_STEP_FLICKS};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// The most particles alive at once in closed-form mode.
const MOST_ALIVE: f64 = 100_000.0;

/// Maximum distinct simulated particle fields kept cached in memory.
const MAX_CACHED_FIELDS: usize = 8;

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
    /// Air drag coefficient in s⁻¹ (`0.0` disables drag).
    pub drag: f64,
    /// Floor bounce restitution in `[0, 1]` (`0.0` disables floor collision).
    pub bounce: f64,
    /// Optional floor y-coordinate in pixels (Kerf `+y` is downward).
    pub floor_y: Option<f64>,
    /// Divergence-free curl turbulence acceleration in pixels/s² (`0.0` disables wind).
    pub turbulence: f64,
    /// Spatial wavelength of curl turbulence in pixels.
    pub turbulence_scale: f64,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SimCacheKey {
    rate: u64,
    life: u64,
    velocity: u64,
    vary: u64,
    direction: u64,
    spread: u64,
    gravity: u64,
    drag: u64,
    bounce: u64,
    floor_y: Option<u64>,
    turbulence: u64,
    turbulence_scale: u64,
    origin: [u64; 2],
    area: [u64; 2],
    seed: u64,
}

impl SimCacheKey {
    fn from_spec(spec: &ParticleFieldSpec) -> Self {
        Self {
            rate: spec.rate.to_bits(),
            life: spec.life.to_bits(),
            velocity: spec.velocity.to_bits(),
            vary: spec.vary.to_bits(),
            direction: spec.direction_deg.to_bits(),
            spread: spec.spread_deg.to_bits(),
            gravity: spec.gravity.to_bits(),
            drag: spec.drag.to_bits(),
            bounce: spec.bounce.to_bits(),
            floor_y: spec.floor_y.map(f64::to_bits),
            turbulence: spec.turbulence.to_bits(),
            turbulence_scale: spec.turbulence_scale.to_bits(),
            origin: [spec.origin[0].to_bits(), spec.origin[1].to_bits()],
            area: [spec.area[0].to_bits(), spec.area[1].to_bits()],
            seed: spec.seed,
        }
    }
}

struct CachedTimeline {
    key: SimCacheKey,
    timeline: WorldTimeline,
}

fn sim_cache() -> &'static Mutex<VecDeque<CachedTimeline>> {
    static CACHE: OnceLock<Mutex<VecDeque<CachedTimeline>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_CACHED_FIELDS)))
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
    /// True when any stateful physics force (drag, floor bounce, or curl turbulence) is enabled.
    #[inline]
    pub fn uses_physics(&self) -> bool {
        self.drag > 0.0 || self.bounce > 0.0 || self.turbulence > 0.0
    }

    fn particle_field_spec(&self) -> ParticleFieldSpec {
        ParticleFieldSpec {
            rate: self.rate,
            life: self.life,
            velocity: self.velocity,
            vary: self.vary,
            direction_deg: self.direction,
            spread_deg: self.spread,
            gravity: self.gravity,
            drag: self.drag,
            bounce: self.bounce,
            floor_y: if self.bounce > 0.0 {
                self.floor_y
            } else {
                None
            },
            turbulence: self.turbulence,
            turbulence_scale: self.turbulence_scale.max(16.0),
            origin: self.origin,
            area: self.area,
            seed: self.seed,
        }
    }

    /// Where particle `id` is `age` seconds after it was born (closed-form ballistic mode).
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

    fn splat(&self, image: &mut Image, at: [f64; 2], age: f64) {
        if age < 0.0 || age >= self.life {
            return;
        }
        let along = age / self.life;
        let size = self.size.0 + (self.size.1 - self.size.0) * along;
        let radius = size / 2.0;
        if radius <= 0.0 {
            return;
        }
        let paint: [f32; 4] = std::array::from_fn(|k| {
            self.color.0[k] + (self.color.1[k] - self.color.0[k]) * along as f32
        });
        if paint[3] <= 0.0 {
            return;
        }
        let (w, h) = (image.w as i64, image.h as i64);
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

    fn draw_simulated(&self, image: &mut Image, time: Time) {
        if self.rate <= 0.0 || self.life <= 0.0 || time.flicks() < 0 {
            return;
        }
        let pf = self.particle_field_spec();
        let key = SimCacheKey::from_spec(&pf);
        let state = if let Ok(mut cache) = sim_cache().lock() {
            if let Some(pos) = cache.iter().position(|entry| entry.key == key) {
                let mut entry = cache.remove(pos).expect("index in bounds");
                let state = entry.timeline.state_at(time);
                cache.push_back(entry);
                state
            } else {
                let spec = WorldSpec {
                    gravity: [0.0, self.gravity, 0.0],
                    substeps: 2,
                    step_flicks: DEFAULT_STEP_FLICKS,
                    bodies: Vec::new(),
                    constraints: Vec::new(),
                    particles: Some(pf),
                };
                let mut timeline = WorldTimeline::new(spec, 12, 128);
                let state = timeline.state_at(time);
                if cache.len() >= MAX_CACHED_FIELDS {
                    cache.pop_front();
                }
                cache.push_back(CachedTimeline { key, timeline });
                state
            }
        } else {
            let spec = WorldSpec {
                gravity: [0.0, self.gravity, 0.0],
                substeps: 2,
                step_flicks: DEFAULT_STEP_FLICKS,
                bodies: Vec::new(),
                constraints: Vec::new(),
                particles: Some(pf),
            };
            let mut timeline = WorldTimeline::new(spec, 12, 32);
            timeline.state_at(time)
        };

        for p in &state.particles {
            self.splat(image, [p.pos[0], p.pos[1]], p.age);
        }
    }

    /// Draw it over `image` at exact [`Time`] after it began.
    pub fn draw_at(&self, image: &mut Image, time: Time) {
        if self.uses_physics() {
            self.draw_simulated(image, time);
        } else {
            self.draw_analytical(image, time.seconds());
        }
    }

    /// Draw it over `image` at `time` seconds after it began.
    pub fn draw(&self, image: &mut Image, time: f64) {
        if self.uses_physics() {
            let flicks = (time * FLICKS_PER_SECOND as f64).round() as i64;
            self.draw_simulated(image, Time::from_flicks(flicks));
        } else {
            self.draw_analytical(image, time);
        }
    }

    fn draw_analytical(&self, image: &mut Image, time: f64) {
        let rate = self.rate.min(MOST_ALIVE / self.life.max(1e-6));
        for id in self.alive(time) {
            let age = time - id as f64 / rate;
            if age < 0.0 || age >= self.life {
                continue;
            }
            let at = self.place(id, age);
            self.splat(image, at, age);
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
            drag: 0.0,
            bounce: 0.0,
            floor_y: None,
            turbulence: 0.0,
            turbulence_scale: 120.0,
            size: (4.0, 4.0),
            color: ([1.0; 4], [1.0; 4]),
            soft: 0.0,
            origin: [50.0, 100.0],
            area: [0.0, 0.0],
            seed: 1,
        }
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

    #[test]
    fn simulated_particles_bounce_deterministically_across_random_seek() {
        let e = Emitter {
            rate: 40.0,
            life: 2.0,
            velocity: 40.0,
            vary: 0.2,
            direction: 90.0, // straight down
            spread: 30.0,
            gravity: 240.0,
            drag: 0.5,
            bounce: 0.75,
            floor_y: Some(150.0),
            turbulence: 45.0,
            turbulence_scale: 64.0,
            origin: [100.0, 40.0],
            ..emitter()
        };
        let t_early = Time::from_flicks(FLICKS_PER_SECOND / 2);
        let t_late = Time::from_flicks(FLICKS_PER_SECOND * 3 / 2);

        // Draw late first, then early, then late again.
        let mut late_first = Image::new(200, 200);
        e.draw_at(&mut late_first, t_late);
        let mut early = Image::new(200, 200);
        e.draw_at(&mut early, t_early);
        let mut late_again = Image::new(200, 200);
        e.draw_at(&mut late_again, t_late);

        assert_eq!(
            late_first.px, late_again.px,
            "random-seek draw must be bit-identical"
        );

        // No pixel below floor_y + particle radius (150 + 3 = 153) should be lit.
        let mut lit_above_floor = 0usize;
        let mut lit_below_floor = 0usize;
        for y in 0..200 {
            for x in 0..200 {
                let a = late_first.px[(y * 200 + x) * 4 + 3];
                if a > 0.01 {
                    if y > 153 {
                        lit_below_floor += 1;
                    } else {
                        lit_above_floor += 1;
                    }
                }
            }
        }
        assert!(
            lit_above_floor > 20,
            "expected visible simulated particles above floor"
        );
        assert_eq!(
            lit_below_floor, 0,
            "bouncing particles must not pass beneath floor_y"
        );
    }
}
