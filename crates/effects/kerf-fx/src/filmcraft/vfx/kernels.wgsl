// The GPU's side of the look-making effects in `vfx` and `effects` (Wonder Glow, Glint, RGB Split,
// Noise, Wave Warp), with the arithmetic of the CPU code they mirror, which is the reference.

struct U {
    a: vec4<u32>,
    b: vec4<u32>,
    p0: vec4<f32>,
    p1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var aux: texture_2d<f32>;

fn inside(id: vec3<u32>) -> bool {
    let size = textureDimensions(dst);
    return id.x < size.x && id.y < size.y;
}

fn srgb_to_linear(v: vec3<f32>) -> vec3<f32> {
    let hi = pow((max(v, vec3(0.0)) + 0.055) / 1.055, vec3(2.4));
    return select(hi, v / 12.92, v <= vec3(0.04045));
}

fn linear_to_srgb(v: vec3<f32>) -> vec3<f32> {
    let hi = 1.055 * pow(max(v, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return select(hi, v * 12.92, v <= vec3(0.0031308));
}

fn luma(c: vec3<f32>) -> f32 {
    return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
}

fn load_or_clear(p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(src));
    if p.x < 0 || p.y < 0 || p.x >= size.x || p.y >= size.y {
        return vec4(0.0);
    }
    return textureLoad(src, p, 0);
}

// Bilinear, transparent outside the picture (`Image::sample_bilinear`).
fn sample(x: f32, y: f32) -> vec4<f32> {
    let f = vec2(x, y) - 0.5;
    let p0 = floor(f);
    let t = f - p0;
    let p = vec2<i32>(p0);
    let a = load_or_clear(p);
    let b = load_or_clear(p + vec2(1, 0));
    let c = load_or_clear(p + vec2(0, 1));
    let d = load_or_clear(p + vec2(1, 1));
    let top = a + (b - a) * t.x;
    let bottom = c + (d - c) * t.x;
    return top + (bottom - top) * t.y;
}

// `vfx::highlights`. p0: threshold, softness.
@compute @workgroup_size(16, 16)
fn highlights(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a <= 1e-6 {
        textureStore(dst, p, vec4(0.0));
        return;
    }
    let l = luma(linear_to_srgb(c.rgb / c.a));
    let t = clamp((l - u.p0.x) / max(u.p0.y, 1e-6), 0.0, 1.0);
    textureStore(dst, p, c * (t * t * (3.0 - 2.0 * t)));
}

// `lights::saturate_image` when a.x, then `lights::tint` when a.y. p0: saturation, tint amount;
// p1: tint color.
@compute @workgroup_size(16, 16)
fn grade_light(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    var c = textureLoad(src, p, 0);
    if u.a.x != 0u {
        let l = luma(c.rgb);
        c = vec4(max(vec3(l) + (c.rgb - vec3(l)) * u.p0.x, vec3(0.0)), c.a);
    }
    if u.a.y != 0u {
        let l = luma(c.rgb);
        c = vec4(c.rgb + (l * u.p1.rgb * 1.5 - c.rgb) * u.p0.y, c.a);
    }
    textureStore(dst, p, c);
}

// `src` plus `aux` by a weight. p0.x: weight.
@compute @workgroup_size(16, 16)
fn accumulate(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    textureStore(dst, p, textureLoad(src, p, 0) + textureLoad(aux, p, 0) * u.p0.x);
}

// `lights::add_image`: the light `aux` on `src`. p0: gain; a.x: screen.
@compute @workgroup_size(16, 16)
fn add_light(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let add = textureLoad(aux, p, 0).rgb * u.p0.rgb;
    if u.a.x != 0u {
        let a = min(add, vec3(1.0));
        let rgb = c.rgb + a - min(c.rgb, vec3(1.0)) * a;
        textureStore(dst, p, vec4(rgb, c.a + (1.0 - c.a) * max(max(a.r, a.g), a.b)));
    } else {
        let m = clamp(max(max(add.r, add.g), add.b), 0.0, 1.0);
        textureStore(dst, p, vec4(c.rgb + add, c.a + (1.0 - c.a) * m));
    }
}

// One doubling of `lights::streak`. p0: dx, dy, weight.
@compute @workgroup_size(16, 16)
fn streak(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let q = sample(f32(id.x) + 0.5 - u.p0.x, f32(id.y) + 0.5 - u.p0.y);
    textureStore(dst, p, textureLoad(src, p, 0) + q * u.p0.z);
}

// Every value by p0.x.
@compute @workgroup_size(16, 16)
fn scale_all(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    textureStore(dst, p, textureLoad(src, p, 0) * u.p0.x);
}

// `lights::rgb_split`. a.x: radial; p0: offset x, y, blend; p1: center x, y, reach, amount.
@compute @workgroup_size(16, 16)
fn rgb_split(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let xy = vec2<f32>(id.xy) + 0.5;
    var offset = u.p0.xy;
    if u.a.x != 0u {
        offset = (xy - u.p1.xy) / u.p1.z * u.p1.w;
    }
    let r = sample(xy.x - offset.x, xy.y - offset.y);
    let g = textureLoad(src, p, 0);
    let b = sample(xy.x + offset.x, xy.y + offset.y);
    let o = vec4(r.r, g.g, b.b, max(max(r.a, g.a), b.a));
    textureStore(dst, p, o + (g - o) * u.p0.z);
}

// The wave warp in `effects`. p0: across x, y, height, width; p1.x: phase.
@compute @workgroup_size(16, 16)
fn wave_warp(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let xy = vec2<f32>(id.xy) + 0.5;
    let along = xy.x * u.p0.x + xy.y * u.p0.y;
    let off = u.p0.z * sin(along / u.p0.w * 6.283185307179586 - u.p1.x);
    textureStore(dst, vec2<i32>(id.xy), sample(xy.x - u.p0.y * off, xy.y + u.p0.x * off));
}

// 32-bit multiply with the high word.
fn mul32(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xffffu;
    let a1 = a >> 16u;
    let b0 = b & 0xffffu;
    let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);
    return vec2((mid << 16u) | (p00 & 0xffffu), p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u));
}

fn mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let p = mul32(a.x, b.x);
    return vec2(p.x, p.y + a.x * b.y + a.y * b.x);
}

fn shr64(a: vec2<u32>, n: u32) -> vec2<u32> {
    return vec2((a.x >> n) | (a.y << (32u - n)), a.y >> n);
}

// `effects::hash3`, with `z` already multiplied by its constant.
fn hash3(x: u32, y: u32, z: vec2<u32>) -> f32 {
    var h = mul64(vec2(x, 0u), vec2(0x7F4A7C15u, 0x9E3779B9u))
        ^ mul64(vec2(y, 0u), vec2(0x27D4EB4Fu, 0xC2B2AE3Du))
        ^ z;
    h = h ^ shr64(h, 31u);
    h = mul64(h, vec2(0x1CE4E5B9u, 0xBF58476Du));
    h = h ^ shr64(h, 29u);
    return f32(h.y >> 8u) / 16777216.0;
}

// `color::noise`. a.x: colored; a.y: clip; a.zw, b.xy, b.zw: the frame's three hashes; p0: amount,
// grain.
@compute @workgroup_size(16, 16)
fn noise(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a <= 0.0 {
        textureStore(dst, p, c);
        return;
    }
    let gx = u32(f32(id.x) / u.p0.y);
    let gy = u32(f32(id.y) / u.p0.y);
    let n0 = hash3(gx, gy, u.a.zw) - 0.5;
    var n = vec3(n0);
    if u.a.x != 0u {
        n = vec3(n0, hash3(gx, gy, u.b.xy) - 0.5, hash3(gx, gy, u.b.zw) - 0.5);
    }
    var o = linear_to_srgb(max(c.rgb / c.a, vec3(0.0))) + n * u.p0.x;
    if u.a.y != 0u {
        o = clamp(o, vec3(0.0), vec3(1.0));
    }
    textureStore(dst, p, vec4(srgb_to_linear(clamp(o, vec3(0.0), vec3(1.0))) * c.a, c.a));
}
