// The GPU's side of effects in `vfx` and `effects` (Wonder Glow, Glint, RGB Split, Noise, Wave Warp,
// the keys, Radial Zoom Blur), and of holding values over 1 for them, with the arithmetic of the CPU code they mirror, which is the reference.

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

// `color::ultra_key` without its matte cleanup's blurs. p0: the key's y, cb, cr, |key|²;
// p1: gain, highlight, shadow, tolerance; p2: pedestal, contrast, mid point, spill;
// p3: desaturate, range, spill luma; p4: color correction's saturation, hue, luminance.
// a.x output, a.y the dominant channel, a.z whether to correct color.
@compute @workgroup_size(16, 16)
fn ultra_key(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    var v = enc3(straight(c));
    var alpha = 0.0;
    if c.a > 1e-6 {
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
        alpha = clamp((clamp(m, 0.0, 1.0) - pedestal) / (1.0 - pedestal), 0.0, 1.0);
    }
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
