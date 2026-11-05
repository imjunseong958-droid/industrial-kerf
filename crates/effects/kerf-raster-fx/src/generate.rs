//! Generate and Noise effects that synthesise images.

// ---- value noise / fBm (our own lattice noise with quintic fade) ----

#[inline]
pub(crate) fn lattice(ix: i32, iy: i32, iz: i32, seed: u32) -> f32 {
    let mut h =
        (ix as u32).wrapping_mul(0x27d4_eb2d) ^ (iy as u32).wrapping_mul(0x1656_67b1) ^ (iz as u32).wrapping_mul(0x9e37_79b9) ^ seed.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

#[inline]
pub(crate) fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// 3D value noise in 0..1 (z = evolution).
pub fn value_noise(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let (x0, y0, z0) = (x.floor(), y.floor(), z.floor());
    let (fx, fy, fz) = (fade(x - x0), fade(y - y0), fade(z - z0));
    let (ix, iy, iz) = (x0 as i32, y0 as i32, z0 as i32);
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let c = |dx: i32, dy: i32, dz: i32| lattice(ix + dx, iy + dy, iz + dz, seed);
    let a = l(l(c(0, 0, 0), c(1, 0, 0), fx), l(c(0, 1, 0), c(1, 1, 0), fx), fy);
    let b = l(l(c(0, 0, 1), c(1, 0, 1), fx), l(c(0, 1, 1), c(1, 1, 1), fx), fy);
    l(a, b, fz)
}
