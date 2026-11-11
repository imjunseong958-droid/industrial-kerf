// The GPU's side of effects in `vfx` and `effects` (Wonder Glow, Glint, RGB Split, Noise, Wave Warp,
// the keys, Radial Zoom Blur, Corner Pin, Drop Shadow, Rounded Crop, Spacer, Mosaic), and of holding values over 1 for them, with the arithmetic of the CPU code they mirror, which is the reference.

struct U {
    a: vec4<u32>,
    b: vec4<u32>,
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    p3: vec4<f32>,
    p4: vec4<f32>,
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

// Bilinear, the nearest edge pixel outside the picture (`Image::sample_bilinear_clamped`).
fn bilinear_clamped(picture: texture_2d<f32>, x: f32, y: f32) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(picture));
    let f = clamp(vec2(x, y) - 0.5, vec2(0.0), vec2<f32>(size - 1));
    let p0 = vec2<i32>(f);
    let p1 = min(p0 + 1, size - 1);
    let t = f - vec2<f32>(p0);
    let a = textureLoad(picture, p0, 0);
    let b = textureLoad(picture, vec2(p1.x, p0.y), 0);
    let c = textureLoad(picture, vec2(p0.x, p1.y), 0);
    let d = textureLoad(picture, p1, 0);
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

fn dec3(v: vec3<f32>) -> vec3<f32> {
    return srgb_to_linear(clamp(v, vec3(0.0), vec3(1.0)));
}

fn enc3(v: vec3<f32>) -> vec3<f32> {
    return linear_to_srgb(max(v, vec3(0.0)));
}

// `Image::unpremul`.
fn straight(c: vec4<f32>) -> vec3<f32> {
    if c.a <= 1e-6 {
        return vec3(0.0);
    }
    return c.rgb / c.a;
}

fn ycbcr709(v: vec3<f32>) -> vec3<f32> {
    let kr = 0.2126;
    let kb = 0.0722;
    let y = kr * v.r + (1.0 - kr - kb) * v.g + kb * v.b;
    return vec3(y, (v.b - y) / (2.0 * (1.0 - kb)), (v.r - y) / (2.0 * (1.0 - kr)));
}

fn rgb_to_hsl(c: vec3<f32>) -> vec3<f32> {
    let mx = max(max(c.r, c.g), c.b);
    let mn = min(min(c.r, c.g), c.b);
    let l = (mx + mn) / 2.0;
    if abs(mx - mn) < 1e-7 {
        return vec3(0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = select(d / (mx + mn), d / (2.0 - mx - mn), l > 0.5);
    var h: f32;
    if mx == c.r {
        h = (c.g - c.b) / d + select(0.0, 6.0, c.g < c.b);
    } else if mx == c.g {
        h = (c.b - c.r) / d + 2.0;
    } else {
        h = (c.r - c.g) / d + 4.0;
    }
    return vec3(h / 6.0, s, l);
}

fn hsl_channel(p: f32, q: f32, t0: f32) -> f32 {
    var t = t0;
    if t < 0.0 { t += 1.0; }
    if t > 1.0 { t -= 1.0; }
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 0.5 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl_to_rgb(h: vec3<f32>) -> vec3<f32> {
    if h.y <= 0.0 {
        return vec3(h.z);
    }
    let q = select(h.z + h.y - h.z * h.y, h.z * (1.0 + h.y), h.z < 0.5);
    let p = 2.0 * h.z - q;
    return vec3(
        hsl_channel(p, q, h.x + 1.0 / 3.0),
        hsl_channel(p, q, h.x),
        hsl_channel(p, q, h.x - 1.0 / 3.0),
    );
}

// `effects::key` for Color Key. p0: the key's y, cb, cr, then tolerance; p1: softness.
@compute @workgroup_size(16, 16)
fn color_key(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a <= 0.0 {
        textureStore(dst, p, c);
        return;
    }
    let v = enc3(c.rgb / c.a);
    let y = ycbcr709(v);
    let d = length(y.yz - u.p0.yz) + abs(y.x - u.p0.x) * 0.15;
    let a = c.a * clamp((d - u.p0.w) / u.p1.x, 0.0, 1.0);
    textureStore(dst, p, vec4(dec3(v) * a, a));
}

// `effects::apply` for Luma Key. p0: threshold, cutoff.
@compute @workgroup_size(16, 16)
fn luma_key(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a <= 0.0 {
        textureStore(dst, p, c);
        return;
    }
    let th = u.p0.x;
    let cut = u.p0.y;
    let l = linear_to_srgb(vec3(max(luma(c.rgb / c.a), 0.0))).x;
    var a: f32;
    if l <= cut {
        a = 0.0;
    } else if l >= max(th, cut + 1e-3) {
        a = 1.0;
    } else {
        a = (l - cut) / max(th - cut, 1e-3);
    }
    textureStore(dst, p, c * a);
}

// `color::logo_cutout`. p0: background; p1: threshold, softness. a.x unmultiply, a.y invert.
@compute @workgroup_size(16, 16)
fn logo_cutout(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a <= 1e-6 {
        textureStore(dst, p, c);
        return;
    }
    let v = enc3(c.rgb / c.a);
    let bg = u.p0.rgb;
    var a0 = 0.0;
    for (var k = 0; k < 3; k++) {
        let d = v[k] - bg[k];
        let room = select(bg[k], 1.0 - bg[k], d >= 0.0);
        if room > 1e-6 {
            a0 = max(a0, abs(d) / room);
        }
    }
    a0 = clamp(a0, 0.0, 1.0);
    let th = u.p1.x;
    var a = clamp((a0 - th) / (max(1.0 - th, 1e-3) * u.p1.y), 0.0, 1.0);
    if u.a.y != 0u {
        a = 1.0 - a;
    }
    var col = v;
    if u.a.x != 0u && a0 > 1e-4 && u.a.y == 0u {
        col = clamp(bg + (v - bg) / a0, vec3(0.0), vec3(1.0));
    }
    let na = a * c.a;
    textureStore(dst, p, vec4(dec3(col) * na, na));
}

// `color::ultra_key`. p0: the key's y, cb, cr, |key|²; p1: gain, highlight, shadow, tolerance;
// p2: pedestal, contrast, mid point, spill; p3: desaturate, range, spill luma; p4: color correction's
// saturation, hue, luminance. a.x output, a.y the dominant channel, a.z whether to correct color.
fn ultra_matte_of(c: vec4<f32>, v: vec3<f32>) -> f32 {
    if c.a <= 1e-6 {
        return 0.0;
    }
    let k = u.p0.xyz;
    let kmag2 = u.p0.w;
    let y = ycbcr709(v);
    let proj = (y.y * k.y + y.z * k.z) / kmag2;
    let perp = length(y.yz - proj * k.yz) / sqrt(kmag2);
    let keyness = clamp(proj, 0.0, 1.5) * (1.0 - smoothstep(0.0, u.p1.w, perp));
    var m = clamp(1.0 - keyness * u.p1.x, 0.0, 1.0);
    m += max(y.x - k.x, 0.0) * u.p1.y * 2.0 * min(keyness, 1.0);
    m += max(k.x - y.x, 0.0) * u.p1.z * 2.0 * min(keyness, 1.0);
    let pedestal = u.p2.x;
    return clamp((clamp(m, 0.0, 1.0) - pedestal) / (1.0 - pedestal), 0.0, 1.0);
}

fn ultra_finish(p: vec2<i32>, c: vec4<f32>, encoded: vec3<f32>, matte: f32) {
    var v = encoded;
    var alpha = matte;
    let contrast = u.p2.y;
    if contrast > 0.0 {
        let g = 1.0 + contrast * 4.0;
        alpha = clamp((alpha - u.p2.z) * g + u.p2.z, 0.0, 1.0);
    }
    let dom = i32(u.a.y);
    let spill = u.p2.w;
    let desat = u.p3.x;
    let range = u.p3.y;
    let sluma = u.p3.z;
    let o1 = v[(dom + 1) % 3];
    let o2 = v[(dom + 2) % 3];
    let limit = max(o1, o2) * (1.0 - range) + (o1 + o2) * 0.5 * range;
    let excess = max(v[dom] - limit, 0.0);
    if excess > 0.0 && spill > 0.0 {
        let l0 = luma(v);
        v[dom] -= excess * spill;
        let l1 = luma(v);
        v = l1 + (v - l1) * (1.0 - desat * min(excess * 4.0, 1.0));
        v += (l0 - l1) * sluma;
    }
    if u.a.z != 0u {
        var h = rgb_to_hsl(clamp(v, vec3(0.0), vec3(1.0)));
        h.x = fract(h.x + u.p4.y);
        h.y = clamp(h.y * u.p4.x, 0.0, 1.0);
        h.z = clamp(h.z * u.p4.z, 0.0, 1.0);
        v = hsl_to_rgb(h);
    }
    let l = dec3(v);
    switch u.a.x {
        case 1u: {
            let g = srgb_to_linear(vec3(alpha)).x;
            textureStore(dst, p, vec4(vec3(g * c.a), c.a));
        }
        case 2u: {
            textureStore(dst, p, vec4(l * c.a, c.a));
        }
        default: {
            let na = alpha * c.a;
            textureStore(dst, p, vec4(l * na, na));
        }
    }
}

// All of it in a pass, when the matte is not cleaned up.
@compute @workgroup_size(16, 16)
fn ultra_key(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let v = enc3(straight(c));
    ultra_finish(p, c, v, ultra_matte_of(c, v));
}

// Its matte alone, in red, for its Choke and Soften to blur.
@compute @workgroup_size(16, 16)
fn ultra_matte(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    textureStore(dst, p, vec4(ultra_matte_of(c, enc3(straight(c))), 0.0, 0.0, 0.0));
}

// Choke: the matte and its blur (aux), by how far it chokes (p0.x).
@compute @workgroup_size(16, 16)
fn ultra_choke(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let at = u.p0.x;
    let blurred = textureLoad(aux, p, 0).x;
    let choked = clamp((blurred - at) / (1.0 - at), 0.0, 1.0);
    textureStore(dst, p, vec4(min(textureLoad(src, p, 0).x, choked), 0.0, 0.0, 0.0));
}

// The rest of it, with its matte cleaned up in aux.
@compute @workgroup_size(16, 16)
fn ultra_keyed(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    ultra_finish(p, c, enc3(straight(c)), textureLoad(aux, p, 0).x);
}

// `kept::encoded`: premultiplied and sRGB-encoded, as EffectCraft's effects read pictures.
@compute @workgroup_size(16, 16)
fn encode_kept(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a <= 1e-6 {
        textureStore(dst, p, vec4(0.0));
        return;
    }
    let v = linear_to_srgb(clamp(c.rgb / c.a, vec3(0.0), vec3(1.0)));
    textureStore(dst, p, vec4(v * c.a, c.a));
}

// `kept::decode_into`, the way back.
@compute @workgroup_size(16, 16)
fn decode_kept(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let a = clamp(c.a, 0.0, 1.0);
    if a <= 1e-6 {
        textureStore(dst, p, vec4(0.0));
        return;
    }
    let v = srgb_to_linear(clamp(c.rgb / a, vec3(0.0), vec3(1.0)));
    textureStore(dst, p, vec4(v * a, a));
}

// `blur3::cc_radial_blur`. a.x its type; p0: center, zoom, arc; p1: quality.
@compute @workgroup_size(16, 16)
fn radial_zoom(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let center = u.p0.xy;
    let zoom = u.p0.z;
    let arc = u.p0.w;
    let kind = u.a.x;
    let d = vec2<f32>(id.xy) + 0.5 - center;
    let r = length(d);
    let span = r * select(abs(zoom), abs(arc), kind >= 3u);
    let n = i32(clamp(ceil(span * u.p1.x / 50.0), 1.0, 256.0));
    if n <= 1 {
        textureStore(dst, p, textureLoad(src, p, 0));
        return;
    }
    var sum = vec4(0.0);
    var weights = 0.0;
    for (var i = 0; i < n; i++) {
        let t = f32(i) / f32(n - 1);
        var at: vec2<f32>;
        var w = 1.0;
        switch kind {
            case 0u: {
                at = center + d * (1.0 - zoom * t);
            }
            case 1u: {
                at = center + d * (1.0 - zoom * t);
                w = (1.0 - t) + 0.05;
            }
            case 2u: {
                at = center + d * (1.0 + zoom * (t - 0.5));
            }
            default: {
                let a = arc * (t - 0.5);
                let s = sin(a);
                let co = cos(a);
                at = center + vec2(d.x * co - d.y * s, d.x * s + d.y * co);
                if kind == 4u {
                    w = (1.0 - abs(2.0 * t - 1.0)) + 0.05;
                }
            }
        }
        sum += sample(at.x, at.y) * w;
        weights += w;
    }
    textureStore(dst, p, sum / weights);
}

// How much of a color is left when it is pulled inside BT.709 toward the gray of its own brightness:
// 1 for one it can hold, and 0 for one with no light in it to keep (`Image::to_gamut`).
fn color_left(c: vec3<f32>) -> f32 {
    let lowest = min(min(c.r, c.g), c.b);
    if lowest >= 0.0 { return 1.0; }
    let gray = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    if gray <= 0.0 { return 0.0; }
    return gray / (gray - lowest);
}

fn pulled_inside(c: vec3<f32>, left: f32) -> vec3<f32> {
    if left >= 1.0 { return c; }
    if left <= 0.0 { return vec3(0.0); }
    let gray = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    return vec3(gray) + (c - vec3(gray)) * left;
}

fn put_outside(c: vec3<f32>, left: f32) -> vec3<f32> {
    if left >= 1.0 || left <= 0.0 { return c; }
    let gray = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    return vec3(gray) + (c - vec3(gray)) / left;
}

// `Image::to_unit`: each pixel brought inside BT.709 and under 1.
@compute @workgroup_size(16, 16)
fn to_unit(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let held = pulled_inside(c.rgb, color_left(c.rgb));
    let gain = max(max(max(held.r, held.g), held.b), 1.0);
    textureStore(dst, p, vec4(held / gain, c.a));
}

// `Image::from_unit`: `src` put back by what `aux`, as it was before `to_unit`, says.
@compute @workgroup_size(16, 16)
fn from_unit(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let before = textureLoad(aux, p, 0);
    let left = color_left(before.rgb);
    let held = pulled_inside(before.rgb, left);
    let gain = max(max(max(held.r, held.g), held.b), 1.0);
    textureStore(dst, p, vec4(put_outside(c.rgb * gain, left), c.a));
}

// `Image::to_gamut`: each pixel brought inside BT.709.
@compute @workgroup_size(16, 16)
fn to_gamut(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    textureStore(dst, p, vec4(pulled_inside(c.rgb, color_left(c.rgb)), c.a));
}

// `Image::from_gamut`: `src` put back by what `aux`, as it was before `to_gamut`, says.
@compute @workgroup_size(16, 16)
fn from_gamut(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let before = textureLoad(aux, p, 0);
    textureStore(dst, p, vec4(put_outside(c.rgb, color_left(before.rgb)), c.a));
}

// ---------------------------------------------------------------------------------------------
// Lumetri past its plain numbers: `lumetri`, `lumetri_advanced`, and `hsl_secondary` in `effects.rs`.
// aux is a table: three rows of curves, when there are any (luma, red, green, blue; hue vs sat, hue vs
// hue, hue vs luma, luma vs sat; sat vs sat), then a LUT's cube below them, a row to each blue.

const CURVE_POINTS: i32 = 1024;

// `Lut3d::apply`, a cube of `size` from row `first` of aux, over its domain.
fn lut_at(r: i32, g: i32, b: i32, size: i32, first: i32) -> vec3<f32> {
    return textureLoad(aux, vec2(r + g * size, first + b), 0).xyz;
}

fn lut_apply(rgb: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>, size: i32, first: i32) -> vec3<f32> {
    let n = vec3<f32>(f32(size - 1));
    let at = clamp((rgb - lo) / (hi - lo), vec3(0.0), vec3(1.0)) * n;
    let r0 = min(i32(at.x), size - 2);
    let g0 = min(i32(at.y), size - 2);
    let b0 = min(i32(at.z), size - 2);
    let fr = at.x - f32(r0);
    let fg = at.y - f32(g0);
    let fb = at.z - f32(b0);
    let c000 = lut_at(r0, g0, b0, size, first);
    let c111 = lut_at(r0 + 1, g0 + 1, b0 + 1, size, first);
    var ca: vec3<f32>;
    var cb: vec3<f32>;
    var w: vec4<f32>;
    if fr > fg {
        if fg > fb {
            ca = lut_at(r0 + 1, g0, b0, size, first);
            cb = lut_at(r0 + 1, g0 + 1, b0, size, first);
            w = vec4(1.0 - fr, fr - fg, fg - fb, fb);
        } else if fr > fb {
            ca = lut_at(r0 + 1, g0, b0, size, first);
            cb = lut_at(r0 + 1, g0, b0 + 1, size, first);
            w = vec4(1.0 - fr, fr - fb, fb - fg, fg);
        } else {
            ca = lut_at(r0, g0, b0 + 1, size, first);
            cb = lut_at(r0 + 1, g0, b0 + 1, size, first);
            w = vec4(1.0 - fb, fb - fr, fr - fg, fg);
        }
    } else if fb > fg {
        ca = lut_at(r0, g0, b0 + 1, size, first);
        cb = lut_at(r0, g0 + 1, b0 + 1, size, first);
        w = vec4(1.0 - fb, fb - fg, fg - fr, fr);
    } else if fb > fr {
        ca = lut_at(r0, g0 + 1, b0, size, first);
        cb = lut_at(r0, g0 + 1, b0 + 1, size, first);
        w = vec4(1.0 - fg, fg - fb, fb - fr, fr);
    } else {
        ca = lut_at(r0, g0 + 1, b0, size, first);
        cb = lut_at(r0 + 1, g0 + 1, b0, size, first);
        w = vec4(1.0 - fg, fg - fr, fr - fb, fb);
    }
    return w.x * c000 + w.y * ca + w.z * cb + w.w * c111;
}

// Each pixel as `gs.encode`, `f`, and `gs.decode` have it, skipping what has no alpha.
// a.x the cube's size, a.y where it starts. p0: its domain's low, p1: its high.
@compute @workgroup_size(16, 16)
fn lumetri_input(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a <= 1e-6 {
        textureStore(dst, p, c);
        return;
    }
    let v = clamp(enc3(c.rgb / c.a), vec3(0.0), vec3(1.0));
    let out = lut_apply(v, u.p0.xyz, u.p1.xyz, i32(u.a.x), i32(u.a.y));
    textureStore(dst, p, vec4(dec3(out) * c.a, c.a));
}

const MORE_LUT: u32 = 1u;
const MORE_WHEELS: u32 = 2u;
const MORE_LUMA: u32 = 4u;
const MORE_RED: u32 = 8u;
const MORE_GREEN: u32 = 16u;
const MORE_BLUE: u32 = 32u;
const MORE_HUE_SAT: u32 = 64u;
const MORE_HUE_HUE: u32 = 128u;
const MORE_HUE_LUMA: u32 = 256u;
const MORE_LUMA_SAT: u32 = 512u;
const MORE_SAT_SAT: u32 = 1024u;
const MORE_HUES: u32 = 1984u;

fn curve_at(row: i32, channel: i32, x: f32) -> f32 {
    let at = clamp(x, 0.0, 1.0) * f32(CURVE_POINTS - 1);
    let i = i32(at);
    let j = min(i + 1, CURVE_POINTS - 1);
    let a = textureLoad(aux, vec2(i, row), 0)[channel];
    let b = textureLoad(aux, vec2(j, row), 0)[channel];
    return a + (b - a) * (at - f32(i));
}

// a.x which parts run, a.y the cube's size, a.z where it starts. p0: its domain's low, and how much
// of it; p1: its high; p2, p3, p4: the wheels' lift, mid, and gain.
@compute @workgroup_size(16, 16)
fn lumetri_more(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a <= 1e-6 {
        textureStore(dst, p, c);
        return;
    }
    let parts = u.a.x;
    var v = enc3(c.rgb / c.a);
    if (parts & MORE_LUT) != 0u {
        let unit = clamp(v, vec3(0.0), vec3(1.0));
        let looked = lut_apply(unit, u.p0.xyz, u.p1.xyz, i32(u.a.y), i32(u.a.z));
        v = v + (looked - v) * u.p0.w;
    }
    if (parts & MORE_WHEELS) != 0u {
        let l = clamp(luma(v), 0.0, 1.0);
        let wsh = (1.0 - l) * (1.0 - l);
        let whi = l * l;
        let wmid = max(1.0 - wsh - whi, 0.0);
        v = v + u.p2.xyz * wsh;
        v = v + u.p3.xyz * wmid;
        v = v * (vec3(1.0) + u.p4.xyz * whi);
    }
    if (parts & MORE_LUMA) != 0u {
        v = vec3(curve_at(0, 0, v.x), curve_at(0, 0, v.y), curve_at(0, 0, v.z));
    }
    if (parts & MORE_RED) != 0u {
        v.x = curve_at(0, 1, v.x);
    }
    if (parts & MORE_GREEN) != 0u {
        v.y = curve_at(0, 2, v.y);
    }
    if (parts & MORE_BLUE) != 0u {
        v.z = curve_at(0, 3, v.z);
    }
    if (parts & MORE_HUES) != 0u {
        var h = rgb_to_hsl(clamp(v, vec3(0.0), vec3(1.0)));
        let h0 = h.x;
        let s0 = h.y;
        let l0 = h.z;
        if (parts & MORE_HUE_HUE) != 0u {
            h.x = fract(h.x + (curve_at(1, 1, h0) - 0.5));
        }
        var sm = 1.0;
        if (parts & MORE_HUE_SAT) != 0u {
            sm *= curve_at(1, 0, h0) * 2.0;
        }
        if (parts & MORE_LUMA_SAT) != 0u {
            sm *= curve_at(1, 3, l0) * 2.0;
        }
        if (parts & MORE_SAT_SAT) != 0u {
            sm *= curve_at(2, 0, s0) * 2.0;
        }
        h.y = clamp(h.y * sm, 0.0, 1.0);
        if (parts & MORE_HUE_LUMA) != 0u {
            h.z = clamp(h.z + (curve_at(1, 2, h0) - 0.5) * 0.5, 0.0, 1.0);
        }
        v = hsl_to_rgb(h);
    }
    textureStore(dst, p, vec4(dec3(v) * c.a, c.a));
}

// The HSL Secondary's key, in red. p0: hue center, hue range, saturation min, luma min; p1: luma max,
// soften.
@compute @workgroup_size(16, 16)
fn hsl_key(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    var key = 0.0;
    if c.a > 1e-6 {
        let h = rgb_to_hsl(clamp(enc3(c.rgb / c.a), vec3(0.0), vec3(1.0)));
        let soft = u.p1.y;
        let dh = min(abs(h.x - u.p0.x), 1.0 - abs(h.x - u.p0.x));
        let mh = 1.0 - clamp((dh - u.p0.y / 2.0) / soft, 0.0, 1.0);
        let ms = clamp((h.y - u.p0.z) / soft, 0.0, 1.0);
        let ml = min(
            clamp((h.z - u.p0.w) / soft, 0.0, 1.0),
            clamp((u.p1.x - h.z) / soft, 0.0, 1.0),
        );
        key = mh * ms * ml;
    }
    textureStore(dst, p, vec4(key, 0.0, 0.0, 0.0));
}

// What it does inside its key (aux). a.x which mask it shows, 0 for none. p0: temperature, tint,
// saturation, hue shift.
@compute @workgroup_size(16, 16)
fn hsl_apply(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a <= 1e-6 {
        textureStore(dst, p, c);
        return;
    }
    let m = textureLoad(aux, p, 0).x;
    let v = clamp(enc3(c.rgb / c.a), vec3(0.0), vec3(1.0));
    var out: vec3<f32>;
    switch u.a.x {
        case 1u: {
            let g = rgb_to_hsl(v).z;
            out = g + (v - g) * m;
        }
        case 2u: {
            out = v * m;
        }
        case 3u: {
            out = vec3(m);
        }
        default: {
            var hh = rgb_to_hsl(v);
            hh.x = fract(hh.x + u.p0.w);
            hh.y = clamp(hh.y * u.p0.z, 0.0, 1.0);
            var moved = hsl_to_rgb(hh);
            moved.x *= 1.0 + 0.25 * u.p0.x;
            moved.z *= 1.0 - 0.25 * u.p0.x;
            moved.y *= 1.0 - 0.2 * u.p0.y;
            out = v + (moved - v) * m;
        }
    }
    textureStore(dst, p, vec4(dec3(out) * c.a, c.a));
}

// ---------------------------------------------------------------------------------------------
// `light_leaks`, on `noise3`, whose hash is 64 bits a step, as two words here: x the low, y the high.

fn high_of_product(a: u32, b: u32) -> u32 {
    let a0 = a & 0xffffu;
    let a1 = a >> 16u;
    let b0 = b & 0xffffu;
    let b1 = b >> 16u;
    let low = a0 * b0;
    let cross0 = a0 * b1;
    let cross1 = a1 * b0;
    let mid = (low >> 16u) + (cross0 & 0xffffu) + (cross1 & 0xffffu);
    return a1 * b1 + (cross0 >> 16u) + (cross1 >> 16u) + (mid >> 16u);
}

fn times64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    return vec2(a.x * b.x, high_of_product(a.x, b.x) + a.x * b.y + a.y * b.x);
}

// `x as u64` from what fits in an i32.
fn wide(x: i32) -> vec2<u32> {
    return vec2(u32(x), select(0u, 0xffffffffu, x < 0));
}

// `hash_u` with the seed already multiplied.
fn hash_u(x: i32, y: i32, z: i32, seed: vec2<u32>) -> vec2<u32> {
    var h = times64(wide(x), vec2(0x7F4A7C15u, 0x9E3779B9u))
        ^ times64(wide(y), vec2(0x27D4EB4Fu, 0xC2B2AE3Du))
        ^ times64(wide(z), vec2(0x9E3779F9u, 0x165667B1u))
        ^ seed;
    h = vec2(h.x ^ ((h.x >> 31u) | (h.y << 1u)), h.y ^ (h.y >> 31u));
    h = times64(h, vec2(0x1CE4E5B9u, 0xBF58476Du));
    h = vec2(h.x ^ ((h.x >> 29u) | (h.y << 3u)), h.y ^ (h.y >> 29u));
    h = times64(h, vec2(0x133111EBu, 0x94D049BBu));
    return vec2(h.x ^ h.y, h.y);
}

fn grad3(h: vec2<u32>, x: f32, y: f32, z: f32) -> f32 {
    // `h % 12`, 2^32 being 4 more than a multiple of 12.
    switch ((h.y % 12u) * 4u + (h.x % 12u)) % 12u {
        case 0u: { return x + y; }
        case 1u: { return -x + y; }
        case 2u: { return x - y; }
        case 3u: { return -x - y; }
        case 4u: { return x + z; }
        case 5u: { return -x + z; }
        case 6u: { return x - z; }
        case 7u: { return -x - z; }
        case 8u: { return y + z; }
        case 9u: { return -y + z; }
        case 10u: { return y - z; }
        default: { return -y - z; }
    }
}

fn fade(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn noise3(x: f32, y: f32, z: f32, seed: vec2<u32>) -> f32 {
    let xi = floor(x);
    let yi = floor(y);
    let zi = floor(z);
    let xf = x - xi;
    let yf = y - yi;
    let zf = z - zi;
    let ix = i32(xi);
    let iy = i32(yi);
    let iz = i32(zi);
    let ux = fade(xf);
    let vy = fade(yf);
    let wz = fade(zf);
    let g000 = grad3(hash_u(ix, iy, iz, seed), xf, yf, zf);
    let g100 = grad3(hash_u(ix + 1, iy, iz, seed), xf - 1.0, yf, zf);
    let g010 = grad3(hash_u(ix, iy + 1, iz, seed), xf, yf - 1.0, zf);
    let g110 = grad3(hash_u(ix + 1, iy + 1, iz, seed), xf - 1.0, yf - 1.0, zf);
    let g001 = grad3(hash_u(ix, iy, iz + 1, seed), xf, yf, zf - 1.0);
    let g101 = grad3(hash_u(ix + 1, iy, iz + 1, seed), xf - 1.0, yf, zf - 1.0);
    let g011 = grad3(hash_u(ix, iy + 1, iz + 1, seed), xf, yf - 1.0, zf - 1.0);
    let g111 = grad3(hash_u(ix + 1, iy + 1, iz + 1, seed), xf - 1.0, yf - 1.0, zf - 1.0);
    let x00 = g000 + (g100 - g000) * ux;
    let x10 = g010 + (g110 - g010) * ux;
    let x01 = g001 + (g101 - g001) * ux;
    let x11 = g011 + (g111 - g011) * ux;
    let y0 = x00 + (x10 - x00) * vy;
    let y1 = x01 + (x11 - x01) * vy;
    return (y0 + (y1 - y0) * wz) * 0.9;
}

// `fbm` at a whole number of octaves. Each octave's seed, already multiplied, is in a.
fn fbm_leak(x: f32, y: f32, z: f32, octaves: u32) -> f32 {
    var sum = 0.0;
    var amp = 1.0;
    var freq = 1.0;
    var norm = 0.0;
    for (var i = 0u; i < octaves; i++) {
        let seed = select(select(u.b.xy, u.b.zw, i == 2u), u.a.zw, i == 0u);
        sum += noise3(x * freq, y * freq, z * freq + f32(i) * 17.13, seed) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    return sum / max(norm, 1e-6);
}

fn leak_step(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / max(e1 - e0, 1e-6), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn screen_on(b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    return vec3(1.0) - (vec3(1.0) - min(b, vec3(1.0))) * (vec3(1.0) - min(s, vec3(1.0)));
}

fn overlay_on(b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    let dark = 2.0 * b * s;
    let light = vec3(1.0) - 2.0 * (vec3(1.0) - min(b, vec3(1.0))) * (vec3(1.0) - min(s, vec3(1.0)));
    return select(light, dark, b < vec3(0.5));
}

// a.x the mode, a.zw and b.xy and b.zw: the seeds of octaves 0, 1, and 2, each by `hash_u`'s constant.
// p0: first color, intensity; p1: second color, scale; p2: direction, time.
@compute @workgroup_size(16, 16)
fn light_leaks(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let at = vec2<i32>(id.xy);
    let c = textureLoad(src, at, 0);
    let size = vec2<f32>(textureDimensions(dst));
    let x = f32(id.x) + 0.5;
    let y = f32(id.y) + 0.5;
    let scale = u.p1.w;
    let dx = u.p2.x;
    let dy = u.p2.y;
    let t = u.p2.z;
    let up = x / scale;
    let vp = y / scale;
    let along = ((x / size.x - 0.5) * dx + (y / size.y - 0.5) * dy) * 2.0;
    let bias = leak_step(-0.2, 1.0, -along);
    let n = fbm_leak(up + dx * t * 0.3, vp + dy * t * 0.3, t * 0.15, 3u) * 0.5 + 0.5;
    let m = fbm_leak(up * 0.7 + 5.2, vp * 0.7, t * 0.1 + 9.0, 2u) * 0.5 + 0.5;
    let k = leak_step(0.35, 0.85, n) * bias * u.p0.w;
    let light = (u.p0.xyz + (u.p1.xyz - u.p0.xyz) * m) * k;
    let base = straight(c);
    var o: vec3<f32>;
    switch u.a.x {
        case 1u: {
            o = base + light;
        }
        case 2u: {
            let s = light + vec3(0.5);
            o = overlay_on(base, base + (s - base) * min(k, 1.0));
        }
        default: {
            o = screen_on(base, light);
        }
    }
    let a = c.a + (1.0 - c.a) * min(k, 1.0);
    textureStore(dst, at, vec4(o * a, a));
}

// ---------------------------------------------------------------------------------------------
// `add_grain` of `noise.rs`, on what `encode_kept` makes.

fn lattice(ix: i32, iy: i32, iz: i32, seed: u32) -> f32 {
    var h = (u32(ix) * 0x27d4eb2du) ^ (u32(iy) * 0x165667b1u) ^ (u32(iz) * 0x9e3779b9u)
        ^ (seed * 0x85ebca6bu);
    h ^= h >> 15u;
    h *= 0x2c1b3c6du;
    h ^= h >> 12u;
    h *= 0x297a2d39u;
    h ^= h >> 15u;
    return f32(h & 0xffffu) / 65535.0;
}

fn value_noise(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let x0 = floor(x);
    let y0 = floor(y);
    let z0 = floor(z);
    let fx = fade(x - x0);
    let fy = fade(y - y0);
    let fz = fade(z - z0);
    let ix = i32(x0);
    let iy = i32(y0);
    let iz = i32(z0);
    let c00 = lattice(ix, iy, iz, seed) + (lattice(ix + 1, iy, iz, seed) - lattice(ix, iy, iz, seed)) * fx;
    let c10 = lattice(ix, iy + 1, iz, seed)
        + (lattice(ix + 1, iy + 1, iz, seed) - lattice(ix, iy + 1, iz, seed)) * fx;
    let c01 = lattice(ix, iy, iz + 1, seed)
        + (lattice(ix + 1, iy, iz + 1, seed) - lattice(ix, iy, iz + 1, seed)) * fx;
    let c11 = lattice(ix, iy + 1, iz + 1, seed)
        + (lattice(ix + 1, iy + 1, iz + 1, seed) - lattice(ix, iy + 1, iz + 1, seed)) * fx;
    let a = c00 + (c10 - c00) * fy;
    let b = c01 + (c11 - c01) * fy;
    return a + (b - a) * fz;
}

fn grain_step(e0: f32, e1: f32, x: f32) -> f32 {
    if abs(e1 - e0) < 1e-9 {
        return select(1.0, 0.0, x < e0);
    }
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn grain_blend(c: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var o: vec3<f32>;
    switch u.a.x {
        case 1u: {
            o = c * (vec3(1.0) + n);
        }
        case 2u: {
            o = c + n;
        }
        case 3u: {
            o = vec3(1.0) - (vec3(1.0) - c) * (vec3(1.0) - abs(n));
        }
        case 4u: {
            o = c + n * 4.0 * c * (vec3(1.0) - c);
        }
        default: {
            o = c + n * (vec3(1.0) - 0.5 * c);
        }
    }
    return max(o, vec3(0.0));
}

// a.x the blending mode, a.y whether one plane does for all three; b.xyz each plane's seed. p0: what
// each channel's grain is multiplied by, and the aspect ratio; p1: each channel's grain size, and the
// grain's time; p2: shadows, midtones, highlights, midpoint; p3.x: saturation.
@compute @workgroup_size(16, 16)
fn add_grain(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let at = vec2<i32>(id.xy);
    let px = textureLoad(src, at, 0);
    if px.a <= 0.0 {
        textureStore(dst, at, px);
        return;
    }
    var c = vec3(0.0);
    if px.a > 1e-6 {
        c = px.rgb / px.a;
    }
    let l = clamp(0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b, 0.0, 1.0);
    let mid = u.p2.w;
    let s = 1.0 - grain_step(0.0, mid, l);
    let hi = grain_step(mid, 1.0, l);
    let weight = u.p2.x * s + u.p2.y * (1.0 - s - hi) + u.p2.z * hi;
    let x = f32(id.x);
    let y = f32(id.y);
    let aspect = u.p0.w;
    let frame = u.p1.w;
    var g: vec3<f32>;
    if u.a.y != 0u {
        g = vec3(value_noise(x / (u.p1.x * aspect), y / u.p1.x, frame, u.b.x) - 0.5);
    } else {
        g = vec3(
            value_noise(x / (u.p1.x * aspect), y / u.p1.x, frame, u.b.x) - 0.5,
            value_noise(x / (u.p1.y * aspect), y / u.p1.y, frame, u.b.y) - 0.5,
            value_noise(x / (u.p1.z * aspect), y / u.p1.z, frame, u.b.z) - 0.5,
        );
        let gm = (g.x + g.y + g.z) / 3.0;
        g = gm + (g - gm) * u.p3.x;
    }
    let o = grain_blend(c, g * u.p0.xyz * weight);
    textureStore(dst, at, vec4(o * px.a, px.a));
}

// ---------------------------------------------------------------------------------------------
// What moves a picture's pixels: `distort.rs`.

// `distort::corner_pin`. p0, p1, and p2.x: from a pixel to where it comes from across the picture,
// 0 to 1, row by row; p2.yz: the picture's size.
// By hand, not from `kerf_kernel`: the CPU maps each pixel in 64 bits, which WGSL has not got, so no
// one source could draw what the reference draws; nor does `Num` read a picture yet.
@compute @workgroup_size(16, 16)
fn corner_pin(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let xy = vec2<f32>(id.xy) + 0.5;
    let w = u.p1.z * xy.x + u.p1.w * xy.y + u.p2.x;
    var out = vec4(0.0);
    if abs(w) >= 1e-12 {
        let across = (u.p0.x * xy.x + u.p0.y * xy.y + u.p0.z) / w;
        let down = (u.p0.w * xy.x + u.p1.x * xy.y + u.p1.y) / w;
        if across >= -1e-3 && down >= -1e-3 && across <= 1.0 + 1e-3 && down <= 1.0 + 1e-3 {
            out = sample(across * u.p2.y, down * u.p2.z);
        }
    }
    textureStore(dst, vec2<i32>(id.xy), out);
}

// `distort::mosaic`, its blocks: a picture with a pixel to each, read from the middle of each in
// `src`. p0.xy: a block's width and height.
// By hand, not from `kerf_kernel`: it reads a second, smaller picture at whole-number places, a block to
// a pixel, which `Num` has no way to say.
@compute @workgroup_size(16, 16)
fn mosaic_blocks(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let at = (vec2<f32>(id.xy) + 0.5) * u.p0.xy;
    textureStore(dst, vec2<i32>(id.xy), bilinear_clamped(src, at.x, at.y));
}

// `distort::mosaic`, each pixel from its block in `aux`, as `mosaic_blocks` made them, and with
// softness toward what lies between the blocks. p0.xy: a block's width and height; p0.z: softness.
@compute @workgroup_size(16, 16)
fn mosaic(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let blocks = vec2<i32>(textureDimensions(aux));
    let xy = vec2<f32>(id.xy);
    let block = min(vec2<i32>(xy / u.p0.xy), blocks - 1);
    var out = textureLoad(aux, block, 0);
    if u.p0.z > 0.0 {
        let at = (xy + 0.5) / u.p0.xy;
        out += (bilinear_clamped(aux, at.x, at.y) - out) * u.p0.z;
    }
    textureStore(dst, vec2<i32>(id.xy), out);
}

// The three below take their arithmetic from `kerf_kernel`, which writes `rounded_crop_px`, `spacer_px`,
// and `shadow_px` out from what the CPU can run too; they go on the end of this.

// `distort::rounded_crop`. p0: its left, top, right, and bottom; p1: radius, feather, border; p2: the
// border's color.
@compute @workgroup_size(16, 16)
fn rounded_crop(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let xy = vec2<f32>(id.xy) + 0.5;
    textureStore(dst, p, rounded_crop_px(textureLoad(src, p, 0), xy.x, xy.y));
}

// What `distort::spacer` does to the picture once it is placed. p0: the placed picture's left, top,
// right, and bottom; p1: radius, and 1 to fill behind it; p2: the color behind it.
@compute @workgroup_size(16, 16)
fn spacer(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let xy = vec2<f32>(id.xy) + 0.5;
    textureStore(dst, p, spacer_px(textureLoad(src, p, 0), xy.x, xy.y));
}

// The shadow of `effects`' Drop Shadow, from the picture moved to where it falls. p0: its color, and
// how dark it is.
@compute @workgroup_size(16, 16)
fn shadow(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    textureStore(dst, p, shadow_px(textureLoad(src, p, 0), 0.0, 0.0));
}
