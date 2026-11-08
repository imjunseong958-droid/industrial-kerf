//! A scene to film, for checking a solve against what it should find: textured surfaces at known
//! places, seen by a camera at known places. It is also read by `kerf-mcp`'s tests, to make footage.

/// A camera: its rows are its right, its down, and its forward, in a world where x runs right, y down,
/// and z away.
#[derive(Clone, Copy, Debug)]
pub struct Shot {
    pub turn: [[f64; 3]; 3],
    pub place: [f64; 3],
    pub focal: f64,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn minus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let long = dot(a, a).sqrt();
    [a[0] / long, a[1] / long, a[2] / long]
}

impl Shot {
    /// At `place`, looking at `target`, with `roll` radians about its own line.
    pub fn looking(place: [f64; 3], target: [f64; 3], roll: f64, focal: f64) -> Self {
        let forward = unit(minus(target, place));
        let right = unit(cross([0.0, 1.0, 0.0], forward));
        let down = cross(forward, right);
        let (s, c) = roll.sin_cos();
        let turn = [
            [
                right[0] * c + down[0] * s,
                right[1] * c + down[1] * s,
                right[2] * c + down[2] * s,
            ],
            [
                down[0] * c - right[0] * s,
                down[1] * c - right[1] * s,
                down[2] * c - right[2] * s,
            ],
            forward,
        ];
        Self { turn, place, focal }
    }

    /// Where a point of the world shows, in pixels from the middle of the frame.
    pub fn shows(&self, point: [f64; 3]) -> Option<[f64; 2]> {
        let from = minus(point, self.place);
        let z = dot(self.turn[2], from);
        (z > 1e-6).then(|| {
            [
                self.focal * dot(self.turn[0], from) / z,
                self.focal * dot(self.turn[1], from) / z,
            ]
        })
    }

    fn ray(&self, x: f64, y: f64) -> [f64; 3] {
        let (x, y) = (x / self.focal, y / self.focal);
        [
            self.turn[0][0] * x + self.turn[1][0] * y + self.turn[2][0],
            self.turn[0][1] * x + self.turn[1][1] * y + self.turn[2][1],
            self.turn[0][2] * x + self.turn[1][2] * y + self.turn[2][2],
        ]
    }
}

/// A camera going across a [`room`], turning to keep what is in it in view.
pub fn across_the_room(frames: usize, focal: f64) -> Vec<Shot> {
    (0..frames)
        .map(|frame| {
            let t = frame as f64 / (frames - 1) as f64;
            let place = [-2.5 + 5.0 * t, -0.5 + 0.3 * t, -2.0 + 1.2 * t];
            let target = [0.5 - t, 0.1, 9.0];
            Shot::looking(place, target, 0.03 * (t * 3.0).sin(), focal)
        })
        .collect()
}

/// A flat surface: `axis_u` and `axis_v` run along it from `origin`, as far as `reach` either way.
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub origin: [f64; 3],
    pub axis_u: [f64; 3],
    pub axis_v: [f64; 3],
    pub reach: [f64; 2],
    pub seed: f64,
}

/// A room with two cards standing in it, far enough apart in depth to show a camera's move.
pub fn room() -> Vec<Surface> {
    let wide = 100.0;
    let flat = |origin, axis_u, axis_v, seed| Surface {
        origin,
        axis_u,
        axis_v,
        reach: [wide, wide],
        seed,
    };
    vec![
        // The floor, the ceiling, the walls.
        flat([0.0, 2.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 1.0),
        flat([0.0, -4.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0),
        flat([0.0, 0.0, 14.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 3.0),
        flat([-9.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], 4.0),
        flat([9.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], 5.0),
        // Two cards.
        Surface {
            origin: [-2.2, 0.2, 6.0],
            axis_u: [1.0, 0.0, 0.0],
            axis_v: [0.0, 1.0, 0.0],
            reach: [1.2, 1.6],
            seed: 6.0,
        },
        Surface {
            origin: [2.4, -0.4, 8.5],
            axis_u: [0.9, 0.0, 0.436],
            axis_v: [0.0, 1.0, 0.0],
            reach: [1.5, 1.8],
            seed: 7.0,
        },
    ]
}

/// A plain wall, for a scene that is flat.
pub fn wall() -> Vec<Surface> {
    vec![Surface {
        origin: [0.0, 0.0, 10.0],
        axis_u: [1.0, 0.0, 0.0],
        axis_v: [0.0, 1.0, 0.0],
        reach: [100.0, 100.0],
        seed: 8.0,
    }]
}

/// Which surface a ray from `from` going `way` meets first, how far along, and where on it.
fn first_hit(
    surfaces: &[Surface],
    from: [f64; 3],
    way: [f64; 3],
) -> Option<(usize, f64, [f64; 2])> {
    let mut best = None;
    for (index, surface) in surfaces.iter().enumerate() {
        let normal = cross(surface.axis_u, surface.axis_v);
        let toward = dot(normal, way);
        if toward.abs() < 1e-9 {
            continue;
        }
        let along = dot(normal, minus(surface.origin, from)) / toward;
        if along <= 0.01 || best.is_some_and(|(_, nearest, _)| along >= nearest) {
            continue;
        }
        let hit = [
            from[0] + way[0] * along - surface.origin[0],
            from[1] + way[1] * along - surface.origin[1],
            from[2] + way[2] * along - surface.origin[2],
        ];
        let (u, v) = (dot(hit, surface.axis_u), dot(hit, surface.axis_v));
        if u.abs() <= surface.reach[0] && v.abs() <= surface.reach[1] {
            best = Some((index, along, [u, v]));
        }
    }
    best
}

/// The point of the scene that shows at `at`, in pixels from the middle of the frame.
pub fn point_at(surfaces: &[Surface], shot: &Shot, at: [f64; 2]) -> Option<[f64; 3]> {
    let way = shot.ray(at[0], at[1]);
    let (_, along, _) = first_hit(surfaces, shot.place, way)?;
    Some([
        shot.place[0] + way[0] * along,
        shot.place[1] + way[1] * along,
        shot.place[2] + way[2] * along,
    ])
}

fn hash(i: i64, j: i64) -> f64 {
    let mut x = (i.wrapping_mul(73_856_093) ^ j.wrapping_mul(19_349_663)) as u64;
    x ^= x >> 13;
    x = x.wrapping_mul(0x5bd1_e995);
    x ^= x >> 15;
    (x % 1000) as f64 / 1000.0
}

fn noise(x: f64, y: f64) -> f64 {
    let (i, j) = (x.floor(), y.floor());
    let (tx, ty) = (x - i, y - j);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (i, j) = (i as i64, j as i64);
    let a = hash(i, j) + (hash(i + 1, j) - hash(i, j)) * sx;
    let b = hash(i, j + 1) + (hash(i + 1, j + 1) - hash(i, j + 1)) * sx;
    a + (b - a) * sy
}

fn texture(u: f64, v: f64, seed: f64) -> f64 {
    0.12 + 0.45 * noise(u * 3.0 + seed * 7.0, v * 3.0)
        + 0.3 * noise(u * 8.0, v * 8.0 + seed * 11.0)
        + 0.15 * noise(u * 19.0 + seed, v * 19.0)
}

/// `shot`'s picture of `surfaces`, as RGBA.
pub fn picture(surfaces: &[Surface], shot: &Shot, width: usize, height: usize) -> Vec<u8> {
    let mut rgba = vec![255u8; width * height * 4];
    let (cx, cy) = (width as f64 / 2.0, height as f64 / 2.0);
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get());
    let rows_each = height.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (part, rows) in rgba.chunks_mut(rows_each * width * 4).enumerate() {
            scope.spawn(move || {
                for (row, pixels) in rows.chunks_exact_mut(width * 4).enumerate() {
                    let y = part * rows_each + row;
                    for (x, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let way = shot.ray(x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                        let value = first_hit(surfaces, shot.place, way)
                            .map_or(0.0, |(index, _, at)| {
                                texture(at[0], at[1], surfaces[index].seed)
                            });
                        let byte = (value.clamp(0.0, 1.0) * 255.0) as u8;
                        pixel[..3].fill(byte);
                    }
                }
            });
        }
    });
    rgba
}
