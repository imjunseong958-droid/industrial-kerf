// The GPU's side of the transitions in this folder, with the arithmetic of the CPU code they mirror,
// which is the reference. `gpu.rs` works out every number that does not change across a frame, with
// the CPU's own functions where it can, and says here which of them each kernel reads.
//
// A picture is `tex_a`, the outgoing clip, and `tex_b`, the incoming one; a kernel of a later pass
// reads what earlier passes made in those places.

struct U {
    kind: u32,
    count: u32,
    layers: u32,
    seed: u32,
    size: vec2<f32>,
    p: f32,
    shutter: f32,
    v: array<vec4<f32>, 200>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var tex_a: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var tex_b: texture_2d<f32>;

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;
const FRAC_PI_2: f32 = 1.5707963267949;

fn n(i: u32) -> f32 {
    return u.v[i / 4u][i % 4u];
}

fn n4(i: u32) -> vec4<f32> {
    return u.v[i / 4u];
}

fn on(i: u32) -> bool {
    return n(i) != 0.0;
}

fn inside_dst(id: vec3<u32>) -> bool {
    let size = textureDimensions(dst);
    return id.x < size.x && id.y < size.y;
}

// `Image::get_clamped`, from where `as isize` takes a position.
fn pixel(t: texture_2d<f32>, x: f32, y: f32) -> vec4<f32> {
    let top = vec2<i32>(textureDimensions(t)) - 1;
    return textureLoad(t, clamp(vec2<i32>(vec2(x, y)), vec2(0), top), 0);
}

fn pa(x: f32, y: f32) -> vec4<f32> {
    return pixel(tex_a, x, y);
}

fn pb(x: f32, y: f32) -> vec4<f32> {
    return pixel(tex_b, x, y);
}

// `Image::sample_bilinear_clamped`.
fn clamped(t: texture_2d<f32>, x: f32, y: f32) -> vec4<f32> {
    let top = vec2<f32>(textureDimensions(t)) - 1.0;
    let f = clamp(vec2(x, y) - 0.5, vec2(0.0), top);
    let p0 = floor(f);
    let d = f - p0;
    let at = vec2<i32>(p0);
    let last = vec2<i32>(top);
    let far = min(at + 1, last);
    let c00 = textureLoad(t, at, 0);
    let c10 = textureLoad(t, vec2(far.x, at.y), 0);
    let c01 = textureLoad(t, vec2(at.x, far.y), 0);
    let c11 = textureLoad(t, far, 0);
    let upper = c00 + (c10 - c00) * d.x;
    let lower = c01 + (c11 - c01) * d.x;
    return upper + (lower - upper) * d.y;
}

fn sac(x: f32, y: f32) -> vec4<f32> {
    return clamped(tex_a, x, y);
}

fn sbc(x: f32, y: f32) -> vec4<f32> {
    return clamped(tex_b, x, y);
}

fn lerp4(a: vec4<f32>, b: vec4<f32>, k: f32) -> vec4<f32> {
    return a + (b - a) * k;
}

fn over(under: vec4<f32>, top: vec4<f32>) -> vec4<f32> {
    return top + under * (1.0 - top.w);
}

fn add_light(c: vec4<f32>, rgb: vec4<f32>, k: f32) -> vec4<f32> {
    return vec4(c.xyz + rgb.xyz * k, min(c.w + max(k, 0.0), 1.0));
}

fn luma(c: vec4<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

fn perceptual_luma(c: vec4<f32>) -> f32 {
    return pow(clamp(luma(c), 0.0, 1.0), 1.0 / 2.2);
}

// `smoothstep` in `mod.rs`, which is defined where the two edges meet.
fn sstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 <= e0 {
        return select(1.0, 0.0, x < e0);
    }
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn bell(p: f32) -> f32 {
    let t = 4.0 * p * (1.0 - p);
    return t * t * (3.0 - 2.0 * t);
}

fn mid_mix(p: f32) -> f32 {
    return sstep(0.3, 0.7, p);
}

fn ease_in_out(v: f32) -> f32 {
    let t = clamp(v, 0.0, 1.0);
    if t < 0.5 {
        return 4.0 * t * t * t;
    }
    let far = -2.0 * t + 2.0;
    return 1.0 - far * far * far / 2.0;
}

fn ease_in(v: f32) -> f32 {
    let t = clamp(v, 0.0, 1.0);
    return t * t * t;
}

fn ease_out(v: f32) -> f32 {
    let t = 1.0 - clamp(v, 0.0, 1.0);
    return 1.0 - t * t * t;
}

// `f32::rem_euclid`.
fn rem_euclid(x: f32, m: f32) -> f32 {
    let r = x - m * trunc(x / m);
    return select(r, r + abs(m), r < 0.0);
}

fn hash(x: i32, y: i32, s: u32) -> f32 {
    var h = (u32(x) * 0x8DA6B343u) ^ (u32(y) * 0xD8163841u) ^ (s * 0xCB1AB31Fu) ^ 0x9E3779B9u;
    h ^= h >> 15u;
    h *= 0x2C1B3C6Du;
    h ^= h >> 12u;
    h *= 0x297A2D39u;
    h ^= h >> 15u;
    return f32(h >> 8u) / 16777216.0;
}

fn vnoise(x: f32, y: f32, s: u32) -> f32 {
    let xi = floor(x);
    let yi = floor(y);
    let fx = x - xi;
    let fy = y - yi;
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    let i = i32(xi);
    let j = i32(yi);
    let a = hash(i, j, s);
    let b = hash(i + 1, j, s);
    let c = hash(i, j + 1, s);
    let d = hash(i + 1, j + 1, s);
    let upper = a + (b - a) * ux;
    let lower = c + (d - c) * ux;
    return upper + (lower - upper) * uy;
}

fn fbm(x: f32, y: f32, s: u32) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var f = 1.0;
    var norm = 0.0;
    for (var o = 0u; o < 4u; o++) {
        v += vnoise(x * f, y * f, s + o * 101u) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    return v / norm;
}

fn line_blur(t: texture_2d<f32>, x: f32, y: f32, dx: f32, dy: f32, count: u32) -> vec4<f32> {
    if count <= 1u || (abs(dx) + abs(dy)) < 0.5 {
        return clamped(t, x, y);
    }
    var acc = vec4(0.0);
    for (var i = 0u; i < count; i++) {
        let along = f32(i) / f32(count - 1u) * 2.0 - 1.0;
        acc += clamped(t, x + dx * along, y + dy * along);
    }
    return acc * (1.0 / f32(count));
}

fn zoom_blur(t: texture_2d<f32>, x: f32, y: f32, cx: f32, cy: f32, k: f32, count: u32) -> vec4<f32> {
    let dx = (x - cx) * k * 0.5;
    let dy = (y - cy) * k * 0.5;
    return line_blur(t, x - dx, y - dy, dx, dy, count);
}

fn spin_blur(t: texture_2d<f32>, x: f32, y: f32, cx: f32, cy: f32, ang: f32, count: u32) -> vec4<f32> {
    if count <= 1u || abs(ang) < 1e-4 {
        return clamped(t, x, y);
    }
    let rx = x - cx;
    let ry = y - cy;
    var acc = vec4(0.0);
    for (var i = 0u; i < count; i++) {
        let a = (f32(i) / f32(count - 1u) - 0.5) * ang;
        let s = sin(a);
        let c = cos(a);
        acc += clamped(t, cx + rx * c - ry * s, cy + rx * s + ry * c);
    }
    return acc * (1.0 / f32(count));
}

// Off the edges of the frame: the slides and the film roll's gaps.
fn within(x: f32, y: f32) -> bool {
    return x >= 0.0 && y >= 0.0 && x < u.size.x && y < u.size.y;
}

// ---------------------------------------------------------------------------------------------
// Wipes: `field_wipe`. n: feather, border, anti-aliasing, the sweep's threshold; border color; then
// the field's own numbers from 8.

const W_SOFT: u32 = 0u;
const W_CLOCK: u32 = 1u;
const W_RADIAL: u32 = 2u;
const W_STAR: u32 = 3u;
const W_PANEL: u32 = 4u;
const W_PLATEAU: u32 = 5u;
const W_IRIS_ROUND: u32 = 6u;
const W_IRIS_BOX: u32 = 7u;
const W_IRIS_DIAMOND: u32 = 8u;
const W_IRIS_CROSS: u32 = 9u;
const W_BARN_DOORS: u32 = 10u;
const W_INSET: u32 = 11u;
const W_GRUNGE: u32 = 12u;

fn star_dist(x: f32, y: f32, points: f32, inner: f32) -> f32 {
    let r = sqrt(x * x + y * y);
    if r < 1e-6 {
        return 0.0;
    }
    let th = atan2(-y, x) - FRAC_PI_2;
    let seg = TAU / max(points, 2.0);
    let a = seg / 2.0 - abs(rem_euclid(th, seg) - seg / 2.0);
    let ri = clamp(inner, 0.05, 1.0);
    let p1x = ri * cos(seg / 2.0);
    let p1y = ri * sin(seg / 2.0);
    let ex = p1x - 1.0;
    let ey = p1y;
    let denom = cos(a) * ey - sin(a) * ex;
    let tr = select(ey / denom, 1.0, abs(denom) < 1e-9);
    return r / max(tr, 0.05);
}

fn field(x: f32, y: f32) -> f32 {
    switch u.kind {
        case W_SOFT: {
            return x * n(8) + y * n(9);
        }
        case W_CLOCK: {
            let a = atan2(x - n(8), -(y - n(9))) - n(10);
            return rem_euclid(a, TAU) / TAU * n(11);
        }
        case W_RADIAL: {
            let a = rem_euclid(atan2(y - n(9), x - n(8)) - n(10), TAU);
            let b = select(min(a, FRAC_PI_2), 0.0, a > PI * 1.5);
            return b / FRAC_PI_2 * n(11);
        }
        case W_STAR: {
            let rx = x - n(8);
            let ry = y - n(9);
            let s = n(10);
            let c = n(11);
            return star_dist(rx * c + ry * s, -rx * s + ry * c, n(12), n(13));
        }
        case W_PANEL: {
            let along = x * n(8) + y * n(9);
            let across = select(x, y, n(8) != 0.0);
            let count = n(10);
            let idx = clamp(floor(across / n(12) * count), 0.0, count - 1.0);
            return along + idx * n(11) * 0.6 / count;
        }
        case W_PLATEAU: {
            let dx = n(8);
            let dy = n(9);
            let count = n(10);
            let along = x * dx + y * dy;
            let across = ((-x * dy + y * dx) - n(11)) / n(12);
            let idx = clamp(floor(across * count), 0.0, count - 1.0);
            return along + abs(idx - (count - 1.0) / 2.0) * n(13);
        }
        case W_IRIS_ROUND: {
            return iris_round(x, y, n(8), n(9));
        }
        case W_IRIS_BOX: {
            return max(abs(x - n(8)), abs(y - n(9)) * n(10));
        }
        case W_IRIS_DIAMOND: {
            return abs(x - n(8)) + abs(y - n(9)) * n(10);
        }
        case W_IRIS_CROSS: {
            let a = abs(x - n(8));
            let b = abs(y - n(9)) * n(10);
            let k = 0.3;
            return min(max(a / k, b), max(a, b / k));
        }
        case W_BARN_DOORS: {
            return select(abs(y - n(9)) * n(10), abs(x - n(8)), on(11));
        }
        case W_INSET: {
            return max(abs(x - n(8)), abs(y - n(9)) * n(10));
        }
        default: {
            let scale = n(8);
            let noise = fbm(x / scale, y / scale, u.seed) * 0.8
                + vnoise(x / (scale * 0.15), y / (scale * 0.15), u.seed + 9u) * 0.2;
            return noise * scale * 3.0;
        }
    }
}

// `wipe_px`, with its style in the first numbers, and `iris_round` are written out from `kerf_kernel`
// and go on the end of this.

@compute @workgroup_size(16, 16)
fn wipes(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside_dst(id) {
        return;
    }
    let x = f32(id.x) + 0.5;
    let y = f32(id.y) + 0.5;
    let out = wipe_px(pa(x, y), pb(x, y), field(x, y) - n(3));
    textureStore(dst, vec2<i32>(id.xy), out);
}

// ---------------------------------------------------------------------------------------------
// Cards under a camera: `motion::scene` and `motion::composite`. n: the eye; from 4, `layers` cards
// for each of `count` moments, in four rows: origin and opacity, ex and flags, ey, what shows of it.

const CARD_FRONT_B: u32 = 1u;
const CARD_BACK: u32 = 2u;
const CARD_BACK_B: u32 = 4u;
const CARD_MIRROR_Y: u32 = 8u;
const CARD_SHADE: u32 = 16u;

fn cross3(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return vec3(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x);
}

fn dot3(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return a.x * b.x + a.y * b.y + a.z * b.z;
}

fn card_read(is_b: bool, x: f32, y: f32) -> vec4<f32> {
    if is_b {
        return sbc(x, y);
    }
    return sac(x, y);
}

fn cards_at(first: u32, x: f32, y: f32) -> vec4<f32> {
    let eye = n4(0).xyz;
    var depths: array<f32, 8>;
    var colors: array<vec4<f32>, 8>;
    var hits = 0u;
    for (var l = 0u; l < u.layers && hits < 8u; l++) {
        let row = 4u * (first + l) + 1u;
        let origin = u.v[row];
        let ex = u.v[row + 1u].xyz;
        let flags = u32(u.v[row + 1u].w);
        let ey = u.v[row + 2u].xyz;
        let clip = u.v[row + 3u];
        let d = vec3(x, y, 0.0) - eye;
        let rhs = eye - origin.xyz;
        let nd = -d;
        let ey_nd = cross3(ey, nd);
        let det = dot3(ex, ey_nd);
        if abs(det) < 1e-9 {
            continue;
        }
        let lx = dot3(rhs, ey_nd) / det;
        let ly = dot3(ex, cross3(rhs, nd)) / det;
        let depth = dot3(ex, cross3(ey, rhs)) / det;
        if depth <= 1e-4 {
            continue;
        }
        let normal = cross3(ex, ey);
        let front = dot3(normal, d) > 0.0;
        let edge = min(min(lx - clip.x, clip.z - lx), min(ly - clip.y, clip.w - ly));
        let cov = clamp(edge + 0.5, 0.0, 1.0);
        if cov <= 0.0 {
            continue;
        }
        var c: vec4<f32>;
        if front {
            c = card_read((flags & CARD_FRONT_B) != 0u, lx, ly);
        } else if (flags & CARD_BACK) != 0u {
            let is_b = (flags & CARD_BACK_B) != 0u;
            if (flags & CARD_MIRROR_Y) != 0u {
                c = card_read(is_b, lx, u.size.y - ly);
            } else {
                c = card_read(is_b, u.size.x - lx, ly);
            }
        } else {
            continue;
        }
        if (flags & CARD_SHADE) != 0u {
            let len = max(sqrt(dot3(normal, normal)), 1e-9);
            let facing = abs(normal.z / len);
            let light = 0.45 + 0.55 * facing;
            c = vec4(c.xyz * light, c.w);
        }
        let k = clamp(cov * origin.w, 0.0, 1.0);
        // Furthest first, those at one depth as they came, as a stable sort leaves them.
        var at = hits;
        while at > 0u && depths[at - 1u] < depth {
            depths[at] = depths[at - 1u];
            colors[at] = colors[at - 1u];
            at -= 1u;
        }
        depths[at] = depth;
        colors[at] = c * k;
        hits += 1u;
    }
    var acc = vec4(0.0);
    for (var i = 0u; i < hits; i++) {
        acc = over(acc, colors[i]);
    }
    return acc;
}

@compute @workgroup_size(16, 16)
fn scene(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside_dst(id) {
        return;
    }
    let x = f32(id.x) + 0.5;
    let y = f32(id.y) + 0.5;
    var acc = vec4(0.0);
    for (var s = 0u; s < u.count; s++) {
        acc += cards_at(s * u.layers, x, y);
    }
    textureStore(dst, vec2<i32>(id.xy), acc * (1.0 / f32(u.count)));
}

// ---------------------------------------------------------------------------------------------
// What reads each clip at the pixel it is, or close to it.

const M_LERP: u32 = 0u;
const M_ADDITIVE: u32 = 1u;
const M_FILM: u32 = 2u;
const M_BURN_ALPHA: u32 = 3u;
const M_LUMA_FADE: u32 = 4u;
const M_MOSAIC: u32 = 5u;
const M_BURN_CHROMA: u32 = 6u;
const M_LEAK: u32 = 7u;
const M_FLARE: u32 = 8u;
const M_FLASH: u32 = 9u;
const M_PHOSPHOR: u32 = 10u;
const M_SOLARIZE: u32 = 11u;
const M_STRIPE: u32 = 12u;
const M_LIGHT_SWEEP: u32 = 13u;
const M_NEON: u32 = 14u;
const M_QUARTER: u32 = 15u;
const M_BRIGHT: u32 = 16u;

// `base` in `lights.rs`.
fn base(x: f32, y: f32, k: f32) -> vec4<f32> {
    if k <= 0.0 {
        return pa(x, y);
    }
    if k >= 1.0 {
        return pb(x, y);
    }
    return lerp4(pa(x, y), pb(x, y), k);
}

fn blob(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> f32 {
    let rx = x - cx;
    let ry = y - cy;
    return exp(-((rx * rx + ry * ry) / max(r * r, 1e-6)));
}

fn squared(v: f32) -> f32 {
    return v * v;
}

fn burn_alpha(x: f32, y: f32) -> vec4<f32> {
    let soft = n(4);
    let scale = n(5);
    let a = pa(x, y);
    let g = 0.55 * fbm(x / scale, y / scale, u.seed) + 0.45 * (1.0 - perceptual_luma(a));
    let d = g - n(6);
    let k = 1.0 - sstep(-soft * 0.3, soft * 0.3, d);
    let mixed = lerp4(a, pb(x, y), k);
    let edge = exp(-squared(d / (soft * 0.6))) * n(7);
    var charred = 1.0;
    if d > 0.0 {
        charred = 1.0 - 0.6 * exp(-squared(d / soft)) * n(7);
    }
    return add_light(vec4(mixed.xyz * charred, mixed.w), n4(0), edge * 2.0);
}

fn mosaic(x: f32, y: f32) -> vec4<f32> {
    let bs = n(0);
    let k = n(1);
    if bs < 1.5 {
        return lerp4(pa(x, y), pb(x, y), k);
    }
    let bx = floor(x / bs);
    let by = floor(y / bs);
    let sx = (bx + 0.5) * bs;
    let sy = (by + 0.5) * bs;
    var kk = k;
    if on(2) {
        let r = hash(i32(bx), i32(by), 7u);
        kk = sstep(r - 0.15, r + 0.15, k * 1.3 - 0.15);
    }
    return lerp4(sac(sx, sy), sbc(sx, sy), kk);
}

fn flare(x: f32, y: f32) -> vec4<f32> {
    let c = base(x, y, n(4));
    let dx = n(7);
    let dy = n(8);
    let diag = n(9);
    let fx = n(10);
    let fy = n(11);
    let unit = n(12);
    let rx = x - fx;
    let ry = y - fy;
    let along = rx * dx + ry * dy;
    let across = -rx * dy + ry * dx;
    let core = blob(x, y, fx, fy, unit * 0.06) * 3.0 + blob(x, y, fx, fy, unit * 0.35) * 0.6;
    let streak = exp(-(abs(across) / (unit * 0.01))) * exp(-(abs(along) / (diag * 0.35))) * 1.5;
    let gx = u.size.x - fx;
    let gy = u.size.y - fy;
    let ghost = blob(x, y, gx, gy, unit * 0.08) * 0.4
        + blob(x, y, (gx + fx) / 2.0, (gy + fy) / 2.0, unit * 0.05) * 0.3;
    return add_light(c, n4(0), (core + streak + ghost) * n(5) * n(6));
}

fn solar(v: f32) -> f32 {
    let g = min(pow(max(v, 0.0), 1.0 / 2.2), 1.0);
    let s = select(g, 1.0 - g, g > 0.5) * 2.0;
    return pow(min(s, 1.0), 2.2);
}

fn stripe(x: f32, y: f32) -> vec4<f32> {
    let count = n(4);
    let dx = n(5);
    let dy = n(6);
    let clo = n(10);
    let chi = n(11);
    let span = n(12);
    let bar = n(13);
    let i = clamp(floor(((x * n(7) + y * n(8)) - clo) / max(chi - clo, 1.0) * count), 0.0, count - 1.0);
    let qi = clamp((u.p - 0.4 * i / count) / 0.6, 0.0, 1.0);
    let e = n(9) - bar + qi * (span + 2.0 * bar);
    let d = x * dx + y * dy - e;
    let c = lerp4(pa(x, y), pb(x, y), 1.0 - sstep(-1.0, 1.0, d));
    var light = 0.0;
    if d >= 0.0 && d < bar {
        light = squared(1.0 - d / bar) * sstep(0.0, 0.1, qi) * (1.0 - sstep(0.9, 1.0, qi));
    }
    return add_light(c, n4(0), light * 1.5);
}

fn neon(x: f32, y: f32) -> vec4<f32> {
    let glow = n(6);
    let d = x * n(4) + y * n(5) - n(8);
    let s = n(10) * 0.5;
    let kb = 1.0 - sstep(-s, s, d);
    let mixed = pa(x, y) * (1.0 - kb) + pb(x, y) * kb;
    let core = exp(-squared(d / (glow * 0.08)));
    let halo = exp(-abs(d) / (glow * 0.5));
    let k = (core * 1.5 + halo * 0.8) * n(7) * n(9);
    return add_light(mixed, n4(0), k);
}

fn mixed_at(x: f32, y: f32) -> vec4<f32> {
    switch u.kind {
        case M_LERP: {
            return lerp4(pa(x, y), pb(x, y), n(0));
        }
        case M_ADDITIVE: {
            return min(pa(x, y) * n(0) + pb(x, y) * n(1), vec4(1.0));
        }
        case M_FILM: {
            let g = 1.0 / 2.2;
            let a = pow(max(pa(x, y), vec4(0.0)), vec4(g));
            let b = pow(max(pb(x, y), vec4(0.0)), vec4(g));
            return pow(a * (1.0 - u.p) + b * u.p, vec4(2.2));
        }
        case M_BURN_ALPHA: {
            return burn_alpha(x, y);
        }
        case M_LUMA_FADE: {
            let soft = n(0);
            let a = pa(x, y);
            let b = pb(x, y);
            let l = perceptual_luma(select(a, b, on(1)));
            let g = select(1.0 - l, l, on(2));
            return lerp4(a, b, sstep(-soft, soft, n(3) - g));
        }
        case M_MOSAIC: {
            return mosaic(x, y);
        }
        case M_BURN_CHROMA: {
            let c = base(x, y, n(0));
            let g = n(1);
            let l = luma(c);
            return vec4(
                c.x * (1.0 + g * n(2)) + l * g * 0.3,
                c.y * (1.0 + g * n(3)) + l * g * 0.2,
                c.z * (1.0 + g * n(4)) + l * g * 0.1,
                c.w
            );
        }
        case M_LEAK: {
            var c = base(x, y, n(0));
            for (var i = 0u; i < 3u; i++) {
                let at = 4u + i * 8u;
                let k = blob(x, y, n(at), n(at + 1u), n(at + 2u)) * n(1) * n(2) * 0.7;
                c = add_light(c, n4(at + 4u), k);
            }
            return c;
        }
        case M_FLARE: {
            return flare(x, y);
        }
        case M_FLASH: {
            return lerp4(base(x, y, n(4)), n4(0), n(5));
        }
        case M_PHOSPHOR: {
            let a = pa(x, y);
            let c = lerp4(a, pb(x, y), n(4));
            let l = squared(perceptual_luma(a));
            return add_light(c, n4(0), l * n(5) * 1.5);
        }
        case M_SOLARIZE: {
            let c = base(x, y, n(0));
            let s = vec4(solar(c.x) * c.w, solar(c.y) * c.w, solar(c.z) * c.w, c.w);
            return lerp4(c, s, n(1));
        }
        case M_STRIPE: {
            return stripe(x, y);
        }
        case M_LIGHT_SWEEP: {
            let bw = n(6);
            let d = x * n(4) + y * n(5) - n(8);
            let c = lerp4(pa(x, y), pb(x, y), 1.0 - sstep(-bw * 0.3, bw * 0.3, d));
            return add_light(c, n4(0), exp(-squared(d / (bw * 0.35))) * n(7) * 2.0);
        }
        case M_NEON: {
            return neon(x, y);
        }
        case M_QUARTER: {
            let w = u.size.x;
            let h = u.size.y;
            let a = x / w;
            let b = y / h;
            if a >= n(0) && a < n(0) + 0.5 && b >= n(1) && b < n(1) + 0.5 {
                return pa(x, y);
            }
            return vec4(0.0);
        }
        default: {
            let c = pa(x, y);
            let l = max(perceptual_luma(c) - 0.55, 0.0) * 2.2;
            return c * l;
        }
    }
}

@compute @workgroup_size(16, 16)
fn mixes(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside_dst(id) {
        return;
    }
    textureStore(dst, vec2<i32>(id.xy), mixed_at(f32(id.x) + 0.5, f32(id.y) + 0.5));
}

// ---------------------------------------------------------------------------------------------
// Those that smear what they read, `count` samples a pixel.

const B_ZOOM: u32 = 0u;
const B_DIRECTIONAL: u32 = 1u;
const B_LENS: u32 = 2u;
const B_RADIAL: u32 = 3u;
const B_RAY: u32 = 4u;
const B_LINE: u32 = 5u;
const B_GLOW: u32 = 6u;

fn disc(t: texture_2d<f32>, x: f32, y: f32, first: u32, count: u32) -> vec4<f32> {
    if count == 0u {
        return clamped(t, x, y);
    }
    var acc = vec4(0.0);
    for (var i = 0u; i < count; i++) {
        let at = first + i * 2u;
        acc += clamped(t, x + n(at), y + n(at + 1u));
    }
    return acc * (1.0 / f32(count));
}

fn turned(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> vec2<f32> {
    let s = sin(-r);
    let c = cos(-r);
    let ux = x - cx;
    let uy = y - cy;
    return vec2(cx + ux * c - uy * s, cy + ux * s + uy * c);
}

fn smeared_at(x: f32, y: f32) -> vec4<f32> {
    var a = vec4(0.0);
    var b = vec4(0.0);
    switch u.kind {
        case B_ZOOM: {
            let cx = n(0);
            let cy = n(1);
            let k = n(5);
            if k < 1.0 {
                let z = n(2);
                a = zoom_blur(tex_a, (x - cx) / z + cx, (y - cy) / z + cy, cx, cy, n(4), u.count);
            }
            if k > 0.0 {
                let z = n(3);
                b = zoom_blur(tex_b, (x - cx) / z + cx, (y - cy) / z + cy, cx, cy, n(4), u.count);
            }
            return lerp4(a, b, k);
        }
        case B_DIRECTIONAL: {
            let dx = n(0);
            let dy = n(1);
            let len = n(2);
            let shift = n(3);
            let k = n(4);
            if k < 1.0 {
                a = line_blur(tex_a, x - dx * shift * u.p, y - dy * shift * u.p, dx * len, dy * len, u.count);
            }
            if k > 0.0 {
                let left = 1.0 - u.p;
                b = line_blur(tex_b, x + dx * shift * left, y + dy * shift * left, dx * len, dy * len, u.count);
            }
            return lerp4(a, b, k);
        }
        case B_LENS: {
            let k = n(0);
            let from_a = u32(n(1));
            let from_b = u32(n(2));
            if k < 1.0 {
                a = disc(tex_a, x, y, 4u, from_a);
            }
            if k > 0.0 {
                b = disc(tex_b, x, y, 4u + from_a * 2u, from_b);
            }
            return lerp4(a, b, k);
        }
        case B_RADIAL: {
            let cx = n(0);
            let cy = n(1);
            let ang = n(4);
            let k = n(5);
            if k < 1.0 {
                let at = turned(x, y, cx, cy, n(2));
                a = spin_blur(tex_a, at.x, at.y, cx, cy, ang, u.count);
            }
            if k > 0.0 {
                let at = turned(x, y, cx, cy, n(3));
                b = spin_blur(tex_b, at.x, at.y, cx, cy, ang, u.count);
            }
            return lerp4(a, b, k);
        }
        case B_RAY: {
            let c = pa(x, y);
            let g = n(3);
            let r = zoom_blur(tex_b, x, y, n(0), n(1), n(2), 24u);
            return vec4(c.xyz + r.xyz * g, max(c.w, r.w * min(g, 1.0)));
        }
        case B_LINE: {
            return line_blur(tex_a, x, y, n(0), n(1), u.count);
        }
        default: {
            let c = pa(x, y);
            let light = pb(x, y);
            let gain = n(0);
            return vec4(c.xyz + light.xyz * gain, max(c.w, light.w * min(gain, 1.0)));
        }
    }
}

@compute @workgroup_size(16, 16)
fn blurs(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside_dst(id) {
        return;
    }
    textureStore(dst, vec2<i32>(id.xy), smeared_at(f32(id.x) + 0.5, f32(id.y) + 0.5));
}

// ---------------------------------------------------------------------------------------------
// What moves the frames by arithmetic of its own, at each moment of `temporal`.

const V_FILM_ROLL: u32 = 0u;
const V_STRETCH: u32 = 1u;
const V_BLOCK: u32 = 2u;
const V_FRAME: u32 = 3u;
const V_LOUVER: u32 = 4u;
const V_MIRROR: u32 = 5u;
const V_PEEL: u32 = 6u;
const V_SLICE: u32 = 7u;
const V_WAVE: u32 = 8u;
const V_STRETCH_WIPE: u32 = 9u;
const V_EARTHQUAKE: u32 = 10u;

fn film_roll(x: f32, y: f32, q: f32) -> vec4<f32> {
    let dx = n(4);
    let dy = n(5);
    let step = n(6);
    let travel = n(7);
    let gap = n(8);
    let horiz = on(9);
    let off = step * ease_in_out(q);
    let ax = x - dx * off;
    let ay = y - dy * off;
    if within(ax, ay) {
        return sac(ax, ay);
    }
    let bx = ax + dx * step;
    let by = ay + dy * step;
    if within(bx, by) {
        return sbc(bx, by);
    }
    let across = select(x, y, horiz);
    let across_len = select(u.size.x, u.size.y, horiz);
    let along = select(ay, ax, horiz);
    let share = across / across_len;
    let edge_band = share < 0.12 || share > 0.88;
    let pitch = across_len / 14.0;
    let turns = across / pitch;
    let f = turns - trunc(turns);
    let g = clamp((rem_euclid(along, step) - travel) / max(gap, 1.0), 0.0, 1.0);
    if !edge_band && f >= 0.25 && f < 0.75 && g >= 0.3 && g < 0.7 {
        return vec4(0.5, 0.5, 0.5, 1.0);
    }
    return n4(0);
}

fn stretch(x: f32, y: f32) -> vec4<f32> {
    let len = n(0);
    let split = n(1);
    let axis_x = on(2);
    let s = select(y, x, axis_x);
    let before = s / max(split, 1e-3) * len;
    let beyond = (s - split) / max(len - split, 1e-3) * len;
    var in_b: bool;
    var local: f32;
    if on(3) {
        in_b = s < split;
        local = select(beyond, before, in_b);
    } else {
        in_b = s >= split;
        local = select(before, beyond, in_b);
    }
    let sx = select(x, local, axis_x);
    let sy = select(local, y, axis_x);
    if in_b {
        return sbc(sx, sy);
    }
    return sac(sx, sy);
}

fn block_motion(x: f32, y: f32, q: f32) -> vec4<f32> {
    let lanes = n(0);
    let cells = n(1);
    let sgn = n(2);
    let horiz = on(3);
    let along = select(y, x, horiz);
    let across = select(x, y, horiz);
    let len_al = select(u.size.y, u.size.x, horiz);
    let len_ac = select(u.size.x, u.size.y, horiz);
    let lane = clamp(floor(across / len_ac * lanes), 0.0, lanes - 1.0);
    let cell_len = len_al / cells;
    var col = vec4(0.0);
    var got_a = false;
    var got_b = vec4(0.0);
    var has_b = false;
    let count = i32(cells);
    for (var ci = 0; ci < count; ci++) {
        let order = f32(select(ci, count - 1 - ci, sgn > 0.0)) / cells;
        let delay = 0.6 * (0.6 * order + 0.4 * hash(ci, i32(lane), 17u));
        let qi = clamp((q - delay) / (1.0 - 0.6), 0.0, 1.0);
        let e = ease_in_out(qi);
        let off_a = sgn * len_al * e;
        let off_b = sgn * len_al * (e - 1.0);
        let lo = f32(ci) * cell_len;
        let hi = f32(ci + 1) * cell_len;
        let at_a = along - off_a;
        if !got_a && at_a >= lo && at_a < hi {
            col = sac(select(x, at_a, horiz), select(at_a, y, horiz));
            got_a = true;
        }
        let at_b = along - off_b;
        if !has_b && at_b >= lo && at_b < hi {
            got_b = sbc(select(x, at_b, horiz), select(at_b, y, horiz));
            has_b = true;
        }
    }
    if got_a {
        return over(got_b, col);
    }
    return got_b;
}

fn frame_side(out: vec4<f32>, x: f32, y: f32, is_b: bool, off: f32) -> vec4<f32> {
    let w = u.size.x;
    let h = u.size.y;
    let s = n(4);
    let ux = x - n(6) * off;
    let uy = y - n(7) * off;
    let lx = (ux - w / 2.0) / s + w / 2.0;
    let ly = (uy - h / 2.0) / s + h / 2.0;
    let bpx = n(5) / s;
    let shows = lx >= 0.0 && lx < w && ly >= 0.0 && ly < h;
    let in_frame = lx >= -bpx && lx < w + bpx && ly >= -bpx && ly < h + bpx;
    if shows {
        if is_b {
            return over(out, sbc(lx, ly));
        }
        return over(out, sac(lx, ly));
    }
    if in_frame && bpx > 0.0 {
        return over(out, n4(0));
    }
    return out;
}

fn louver(x: f32, y: f32) -> vec4<f32> {
    let count = n(0);
    let horizontal = on(1);
    let pos = select(x, y, horizontal);
    let len = select(u.size.x, u.size.y, horizontal);
    let sl = len / count;
    let i = clamp(floor(pos / sl), 0.0, count - 1.0);
    let mid = (i + 0.5) * sl;
    let stagger = 0.4 * i / max(count, 1.0);
    let q = clamp((u.p - stagger) / 0.6, 0.0, 1.0);
    let co = cos(ease_in_out(q) * PI);
    if abs(co) < 1e-3 {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    let local = (pos - mid) / co;
    if abs(local) > sl / 2.0 {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    let read = mid + select(-local, local, co > 0.0);
    let sx = select(read, x, horizontal);
    let sy = select(y, read, horizontal);
    var c: vec4<f32>;
    if co > 0.0 {
        c = sac(sx, sy);
    } else {
        c = sbc(sx, sy);
    }
    return vec4(c.xyz * (0.5 + 0.5 * abs(co)), c.w);
}

// How far along the frame from its entry side, for a mirror and a wave.
fn from_entry(x: f32, y: f32, horiz: bool, neg: bool) -> f32 {
    if horiz {
        return select(x, u.size.x - x, neg);
    }
    return select(y, u.size.y - y, neg);
}

fn mirror(x: f32, y: f32) -> vec4<f32> {
    let horiz = on(0);
    let neg = on(1);
    let refl = n(2);
    let e = n(3);
    let s = from_entry(x, y, horiz, neg);
    if s < e {
        return pb(x, y);
    }
    let a = pa(x, y);
    let d = s - e;
    if d < refl {
        let m = e - d;
        var r: vec4<f32>;
        if horiz {
            r = sbc(select(m, u.size.x - m, neg), y);
        } else {
            r = sbc(x, select(m, u.size.y - m, neg));
        }
        return lerp4(a, r, 0.75 * (1.0 - d / refl));
    }
    return a;
}

fn page_back(c: vec4<f32>, light: f32) -> vec4<f32> {
    let m = lerp4(c, vec4(c.w), 0.6);
    return vec4(m.xyz * light, m.w);
}

fn page_peel(x: f32, y: f32) -> vec4<f32> {
    let r = n(0);
    let nx = n(4);
    let ny = n(5);
    let f = n(6);
    let s = (x - n(2)) * nx + (y - n(3)) * ny;
    if s >= f {
        let so = 2.0 * f - s - PI * r;
        let px = x - nx * (s - so);
        let py = y - ny * (s - so);
        if within(px, py) {
            return page_back(sac(px, py), 0.95);
        }
    }
    let d = f - s;
    if d >= 0.0 && d < r {
        let upper = PI - asin(d / r);
        let upper_so = f - r * upper;
        let ux = x - nx * (s - upper_so);
        let uy = y - ny * (s - upper_so);
        if within(ux, uy) {
            return page_back(sac(ux, uy), 0.75 + 0.25 * sin(upper));
        }
        let lower = asin(d / r);
        let lower_so = f - r * lower;
        let lx = x - nx * (s - lower_so);
        let ly = y - ny * (s - lower_so);
        if within(lx, ly) {
            let c = sac(lx, ly);
            let l = 1.0 - 0.45 * (lower / FRAC_PI_2);
            return vec4(c.xyz * l, c.w);
        }
    }
    if s >= f {
        return pa(x, y);
    }
    let b = pb(x, y);
    if on(1) {
        let k = 1.0
            - 0.45
                * exp(-max(d - r, 0.0) / (r * 1.5))
                * sstep(0.0, 0.05, u.p)
                * (1.0 - sstep(0.9, 1.0, u.p));
        return vec4(b.xyz * k, b.w);
    }
    return b;
}

fn slice(x: f32, y: f32, q: f32) -> vec4<f32> {
    let count = n(0);
    let ax = n(1);
    let ay = n(2);
    let px = n(3);
    let py = n(4);
    let lo = n(5);
    let span = n(6);
    let dist = n(7);
    let w = u.size.x;
    let h = u.size.y;
    let i = clamp(floor(((x * px + y * py) - lo) / span * count), 0.0, count - 1.0);
    let sign_of = select(-1.0, 1.0, i32(i) % 2 == 0);
    let qi = clamp((q - 0.3 * i / max(count, 1.0)) / 0.7, 0.0, 1.0);
    let off = sign_of * dist * ease_in(qi);
    let sx = x - ax * off;
    let sy = y - ay * off;
    let zb = 1.0 + 0.05 * (1.0 - q);
    let b = sbc((x - w / 2.0) / zb + w / 2.0, (y - h / 2.0) / zb + h / 2.0);
    if within(sx, sy) {
        let j = clamp(floor(((sx * px + sy * py) - lo) / span * count), 0.0, count - 1.0);
        if j == i {
            return over(b, sac(sx, sy));
        }
    }
    return b;
}

fn wave(x: f32, y: f32) -> vec4<f32> {
    let horiz = on(3);
    let band = n(4);
    let e = n(5);
    let s = from_entry(x, y, horiz, on(2));
    let across = select(x, y, horiz);
    let near = exp(-squared((s - e) / band));
    let disp = n(0) * n(6) * near * sin(2.0 * PI * (across / n(1)) - u.p * 6.0);
    let sx = select(x + disp * 0.3, x + disp, horiz);
    let sy = select(y + disp, y + disp * 0.3, horiz);
    let k = 1.0 - sstep(-band * 0.3, band * 0.3, s - e);
    return lerp4(sac(sx, sy), sbc(sx, sy), k);
}

fn stretch_wipe(x: f32, y: f32) -> vec4<f32> {
    let dx = n(0);
    let dy = n(1);
    let s_len = n(2);
    let e = n(3);
    let s = x * dx + y * dy;
    if s < e {
        return pb(x, y);
    }
    if s < e + s_len {
        let back = s - e;
        let smear = sbc(x - dx * back, y - dy * back);
        return lerp4(smear, pa(x, y), sstep(0.0, 1.0, back / s_len));
    }
    return pa(x, y);
}

// `ab` in `grunge.rs`.
fn ab(x: f32, y: f32, k: f32) -> vec4<f32> {
    if k <= 0.0 {
        return sac(x, y);
    }
    if k >= 1.0 {
        return sbc(x, y);
    }
    return lerp4(sac(x, y), sbc(x, y), k);
}

fn earthquake(x: f32, y: f32, q: f32) -> vec4<f32> {
    let w = u.size.x;
    let h = u.size.y;
    let shake = n(0);
    let s = u.seed;
    let e = bell(q);
    let ox = (vnoise(q * 45.0, 0.5, s) - 0.5) * 2.0 * shake * e;
    let oy = (vnoise(q * 45.0, 7.5, s + 1u) - 0.5) * 2.0 * shake * e;
    let r = (vnoise(q * 30.0, 3.5, s + 2u) - 0.5) * n(1) * e;
    let zoom = 1.0 + 0.08 * e;
    let sn = sin(r);
    let cs = cos(r);
    let ux = (x - w / 2.0) / zoom;
    let uy = (y - h / 2.0) / zoom;
    return ab(ux * cs - uy * sn + w / 2.0 - ox, ux * sn + uy * cs + h / 2.0 - oy, mid_mix(q));
}

fn moved_at(x: f32, y: f32, q: f32) -> vec4<f32> {
    switch u.kind {
        case V_FILM_ROLL: {
            return film_roll(x, y, q);
        }
        case V_STRETCH: {
            return stretch(x, y);
        }
        case V_BLOCK: {
            return block_motion(x, y, q);
        }
        case V_FRAME: {
            let out = frame_side(vec4(0.0), x, y, false, n(8));
            return frame_side(out, x, y, true, n(9));
        }
        case V_LOUVER: {
            return louver(x, y);
        }
        case V_MIRROR: {
            return mirror(x, y);
        }
        case V_PEEL: {
            return page_peel(x, y);
        }
        case V_SLICE: {
            return slice(x, y, q);
        }
        case V_WAVE: {
            return wave(x, y);
        }
        case V_STRETCH_WIPE: {
            return stretch_wipe(x, y);
        }
        default: {
            return earthquake(x, y, q);
        }
    }
}

@compute @workgroup_size(16, 16)
fn moves(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside_dst(id) {
        return;
    }
    let x = f32(id.x) + 0.5;
    let y = f32(id.y) + 0.5;
    var out: vec4<f32>;
    if u.count <= 1u || u.shutter <= 1e-5 {
        out = moved_at(x, y, u.p);
    } else {
        var acc = vec4(0.0);
        for (var i = 0u; i < u.count; i++) {
            let q = clamp(u.p + (f32(i) / f32(u.count - 1u) - 0.5) * u.shutter, 0.0, 1.0);
            acc += moved_at(x, y, q);
        }
        out = acc * (1.0 / f32(u.count));
    }
    textureStore(dst, vec2<i32>(id.xy), out);
}

// ---------------------------------------------------------------------------------------------
// Grunge & Distort. n: the bell, the midpoint's mix, the size of a pixel of 1080p; then their own.

const G_CHAOS: u32 = 0u;
const G_FLICKER: u32 = 1u;
const G_GLASS: u32 = 2u;
const G_GLITCH: u32 = 3u;
const G_KALEIDOSCOPE: u32 = 4u;
const G_LIQUID: u32 = 5u;
const G_TV_POWER: u32 = 6u;
const G_VHS: u32 = 7u;

// `split` in `grunge.rs`.
fn rgb_split(x: f32, y: f32, d: f32, k: f32) -> vec4<f32> {
    let c = ab(x, y, k);
    if abs(d) < 0.25 {
        return c;
    }
    let r = ab(x + d, y, k);
    let b = ab(x - d, y, k);
    return vec4(r.x, c.y, b.z, max(max(c.w, r.w), b.w));
}

fn chaos(x: f32, y: f32) -> vec4<f32> {
    let w = u.size.x;
    let h = u.size.y;
    let env = n(0);
    let k = n(1);
    let amt = n(4);
    let frame = u32(n(5));
    let amp = n(6);
    let s = u.seed;
    let bx = i32(floor(x / (w / 8.0)));
    let by = i32(floor(y / (h / 6.0)));
    let cx = i32(floor(x / (w / 23.0)));
    let cy = i32(floor(y / (h / 13.0)));
    let r = hash(bx, by, s + frame);
    let fine = hash(cx, cy, s + frame * 7u + 1u);
    var ox = 0.0;
    var oy = 0.0;
    if r > 0.55 {
        ox = (hash(bx, by, s + 99u) - 0.5) * amp;
        oy = (hash(bx, by, s + 7u) - 0.5) * amp * 0.3;
    }
    let swapped = select(0.0, 1.0, fine < k);
    let kk = swapped + (k - swapped) * (1.0 - env);
    return rgb_split(x - ox, y - oy, (fine - 0.5) * amt * env * 30.0 * n(2), kk);
}

fn glass(x: f32, y: f32) -> vec4<f32> {
    let env = n(0);
    let cell = n(4);
    let s = u.seed;
    let gx = i32(floor(x / cell));
    let gy = i32(floor(y / cell));
    var best = 1e30;
    var second = 1e30;
    var at = vec2(0);
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let cx = gx + i;
            let cy = gy + j;
            let px = (f32(cx) + hash(cx, cy, s)) * cell;
            let py = (f32(cy) + hash(cx, cy, s + 1u)) * cell;
            let rx = x - px;
            let ry = y - py;
            let d = sqrt(rx * rx + ry * ry);
            if d < best {
                second = best;
                best = d;
                at = vec2(cx, cy);
            } else if d < second {
                second = d;
            }
        }
    }
    let tau = 0.3 + 0.4 * hash(at.x, at.y, s + 2u);
    let kk = sstep(tau - 0.1, tau + 0.1, u.p);
    let ang = hash(at.x, at.y, s + 3u) * TAU;
    let off = n(5) * env * hash(at.x, at.y, s + 4u);
    let c = ab(x + cos(ang) * off, y + sin(ang) * off, kk);
    let edge = exp(-(second - best) / (1.5 * max(n(2), 0.5))) * env;
    let facet = 1.0 + (hash(at.x, at.y, s + 5u) - 0.5) * 0.4 * env;
    return add_light(vec4(c.xyz * facet, c.w), vec4(1.0), edge * 0.8);
}

fn glitch(x: f32, y: f32) -> vec4<f32> {
    let w = u.size.x;
    let h = u.size.y;
    let env = n(0);
    let k = n(1);
    let unit = n(2);
    let amt = n(4);
    let frame = u32(n(5));
    let s = u.seed;
    let band_h = h / (6.0 + 18.0 * hash(i32(frame), 0, s));
    let band = i32(floor(y / max(band_h, 1.0)));
    let r = hash(band, i32(frame), s);
    var ox = 0.0;
    if r > 0.6 {
        ox = (hash(band, i32(frame), s + 1u) - 0.5) * amt * env * w * 0.25;
    }
    let tau = 0.4 + 0.2 * hash(band, 9, s);
    let kk = select(0.0, 1.0, u.p > tau) * env + k * (1.0 - env);
    let c = rgb_split(x - ox, y, amt * env * 18.0 * unit * max(r - 0.3, 0.0) * 2.0, kk);
    let bx = i32(floor(x / (24.0 * max(unit, 0.2))));
    let by = i32(floor(y / (12.0 * max(unit, 0.2))));
    if hash(bx, by, s + frame * 3u) > 1.0 - 0.04 * amt * env {
        let v = hash(bx, by, s + 77u);
        return lerp4(c, vec4(v, 1.0 - v, v * 0.5, 1.0), 0.8);
    }
    return c;
}

fn kaleidoscope(x: f32, y: f32) -> vec4<f32> {
    let w = u.size.x;
    let h = u.size.y;
    let k = n(1);
    let seg = n(4);
    let rx = x - w / 2.0;
    let ry = y - h / 2.0;
    let r = sqrt(rx * rx + ry * ry) / n(5);
    let th = atan2(ry, rx) + u.p * PI * 0.5;
    let a = abs(rem_euclid(th, seg) - seg / 2.0);
    let kal = ab(w / 2.0 + r * cos(a), h / 2.0 + r * sin(a), k);
    let plain = ab(x, y, k);
    return lerp4(plain, kal, sstep(0.0, 0.6, n(0)));
}

fn tv_power(x: f32, y: f32) -> vec4<f32> {
    let w = u.size.x;
    let h = u.size.y;
    let glow = n4(4);
    let sy = n(9);
    let sx = n(10);
    let hy = n(11);
    let hx = n(12);
    let white = n(13);
    let rx = x - w / 2.0;
    let ry = y - h / 2.0;
    var c = vec4(0.0, 0.0, 0.0, 1.0);
    if abs(rx) <= hx && abs(ry) <= hy && hx > 0.0 {
        let lx = w / 2.0 + rx / max(sx, 1e-3);
        let ly = clamp(h / 2.0 + ry / max(sy, 1e-3), 0.0, h);
        var read: vec4<f32>;
        if on(8) {
            read = sbc(lx, ly);
        } else {
            read = sac(lx, ly);
        }
        c = over(c, lerp4(read, glow, white));
    }
    let dy = max(abs(ry) - hy, 0.0);
    let dx = max(abs(rx) - hx, 0.0);
    let halo = exp(-sqrt(dx * dx + dy * dy) / (n(14) * 3.0)) * white;
    return add_light(c, glow, halo * 0.8);
}

fn vhs(x: f32, y: f32) -> vec4<f32> {
    let w = u.size.x;
    let h = u.size.y;
    let env = n(0);
    let unit = n(2);
    let amt = n(4);
    let frame = u32(n(5));
    let s = u.seed;
    let row = i32(floor(y / (2.0 * max(unit, 0.5))));
    let jitter = (hash(row, i32(frame), s) - 0.5) * amt * env * w * 0.02;
    let in_band = exp(-squared((y - n(6)) / (h * 0.06)));
    let ox = jitter
        + in_band * amt * env * w * 0.08 * (vnoise(y * 0.05, u.p * 20.0, s) - 0.5) * 2.0;
    let c = rgb_split(x - ox, y, amt * env * 10.0 * unit, n(1));
    let grain = (hash(i32(x), i32(y), s + frame) - 0.5) * 0.25 * amt * env;
    let band_noise = in_band * env * amt * 0.5 * hash(i32(x) / 3, row, frame);
    let l = luma(c);
    let desat = lerp4(c, vec4(l, l, l, c.w), 0.4 * amt * env);
    return vec4(desat.xyz + grain + band_noise, desat.w);
}

fn damaged_at(x: f32, y: f32) -> vec4<f32> {
    switch u.kind {
        case G_CHAOS: {
            return chaos(x, y);
        }
        case G_FLICKER: {
            let c = select(pa(x, y), pb(x, y), on(4));
            return vec4(c.xyz * n(5), c.w);
        }
        case G_GLASS: {
            return glass(x, y);
        }
        case G_GLITCH: {
            return glitch(x, y);
        }
        case G_KALEIDOSCOPE: {
            return kaleidoscope(x, y);
        }
        case G_LIQUID: {
            let env = n(0);
            let amt = n(4);
            let sc = n(5);
            let s = u.seed;
            let ox = (fbm(x / sc + u.p * 2.0, y / sc, s) - 0.5) * 2.0 * amt * env;
            let oy = (fbm(x / sc, y / sc - u.p * 2.0, s + 5u) - 0.5) * 2.0 * amt * env;
            return ab(x + ox, y + oy, n(1));
        }
        case G_TV_POWER: {
            return tv_power(x, y);
        }
        default: {
            return vhs(x, y);
        }
    }
}

@compute @workgroup_size(16, 16)
fn grunge(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside_dst(id) {
        return;
    }
    textureStore(dst, vec2<i32>(id.xy), damaged_at(f32(id.x) + 0.5, f32(id.y) + 0.5));
}
