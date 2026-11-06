// Kerf's drawing on the GPU: the arithmetic of `kerf_draw::image`, `kerf_draw::blend`, and
// `kerf_draw::color`, the CPU reference, on premultiplied linear-light RGBA f32. Each kernel reads
// `src` (and `aux`) and writes `dst`. The blend modes and the dissolve pattern are carried from
// FilmCraft's GPU compositor (`composite.wgsl`).

struct U {
    a: vec4<u32>,   // blend mode, the picture's transparency counts, unused, unused
    bounds: vec4<u32>, // x0, x1, y0, y1
    m0: vec4<f32>,  // inverse a, b, c, d
    m1: vec4<f32>,  // inverse e, f, across, down
    rect: vec4<f32>, // left, top, right, bottom
    range: vec4<f32>, // lo x, lo y, hi x, hi y
    p: vec4<f32>,   // opacity, or the color filled
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var aux: texture_2d<f32>;

fn srgb_to_linear(v: vec3<f32>) -> vec3<f32> {
    let hi = pow((max(v, vec3(0.0)) + 0.055) / 1.055, vec3(2.4));
    return select(hi, v / 12.92, v <= vec3(0.04045));
}

fn linear_to_srgb(v: vec3<f32>) -> vec3<f32> {
    let hi = 1.055 * pow(max(v, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return select(hi, v * 12.92, v <= vec3(0.0031308));
}

fn inside(id: vec3<u32>) -> bool {
    let size = textureDimensions(dst);
    return id.x < size.x && id.y < size.y;
}

// Straight sRGB-encoded bytes into the working picture (`Image::from_rgba8`).
@compute @workgroup_size(16, 16)
fn decode(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let a = select(1.0, c.a, u.a.y != 0u);
    textureStore(dst, p, vec4(srgb_to_linear(c.rgb) * a, a));
}

// A picture of one color.
@compute @workgroup_size(16, 16)
fn fill(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    textureStore(dst, vec2<i32>(id.xy), u.p);
}

// Every value by a coverage (`Image::mul_coverage`).
@compute @workgroup_size(16, 16)
fn multiply(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    textureStore(dst, p, textureLoad(src, p, 0) * textureLoad(aux, p, 0).r);
}

// The picture at half its size, each pixel the mean of four (`Image::downsample2`).
@compute @workgroup_size(16, 16)
fn halve(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy) * 2;
    let last = vec2<i32>(textureDimensions(src)) - 1;
    let a = textureLoad(src, min(p, last), 0);
    let b = textureLoad(src, min(p + vec2(1, 0), last), 0);
    let c = textureLoad(src, min(p + vec2(0, 1), last), 0);
    let d = textureLoad(src, min(p + vec2(1, 1), last), 0);
    textureStore(dst, vec2<i32>(id.xy), (a + b + c + d) * 0.25);
}

// Bilinear sample of `aux`, the nearest edge pixel outside it (`Image::sample_bilinear_clamped`).
fn sample(x: f32, y: f32) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(aux));
    let f = clamp(vec2(x, y) - 0.5, vec2(0.0), vec2<f32>(size - 1));
    let p0 = vec2<i32>(floor(f));
    let p1 = min(p0 + 1, size - 1);
    let t = f - vec2<f32>(p0);
    let a = textureLoad(aux, p0, 0);
    let b = textureLoad(aux, vec2(p1.x, p0.y), 0);
    let c = textureLoad(aux, vec2(p0.x, p1.y), 0);
    let d = textureLoad(aux, p1, 0);
    let top = a + (b - a) * t.x;
    let bottom = c + (d - c) * t.x;
    return top + (bottom - top) * t.y;
}

fn edge(distance: f32, scale: f32) -> f32 {
    return clamp(distance / scale + 0.5, 0.0, 1.0);
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

// Wrapping 64-bit multiply.
fn mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let p = mul32(a.x, b.x);
    return vec2(p.x, p.y + a.x * b.y + a.y * b.x);
}

// 64-bit shift right by 0 < n < 32.
fn shr64(a: vec2<u32>, n: u32) -> vec2<u32> {
    return vec2((a.x >> n) | (a.y << (32u - n)), a.y >> n);
}

fn hash2(x: u32, y: u32) -> f32 {
    var h = mul64(vec2(x, 0u), vec2(0x7F4A7C15u, 0x9E3779B9u)) ^ mul64(vec2(y, 0u), vec2(0x27D4EB4Fu, 0xC2B2AE3Du));
    h = h ^ shr64(h, 31u);
    h = mul64(h, vec2(0x1CE4E5B9u, 0xBF58476Du));
    h = h ^ shr64(h, 29u);
    return f32(h.y >> 8u) / 16777216.0;
}

fn screen(b: f32, s: f32) -> f32 { return b + s - b * s; }

fn color_burn(b: f32, s: f32) -> f32 {
    if b >= 1.0 { return 1.0; }
    if s <= 0.0 { return 0.0; }
    return 1.0 - min((1.0 - b) / s, 1.0);
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if b <= 0.0 { return 0.0; }
    if s >= 1.0 { return 1.0; }
    return min(b / (1.0 - s), 1.0);
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 { return b * 2.0 * s; }
    return screen(b, 2.0 * s - 1.0);
}

fn vivid_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 { return color_burn(b, 2.0 * s); }
    return color_dodge(b, 2.0 * s - 1.0);
}

fn sep(mode: u32, b: f32, s: f32) -> f32 {
    switch mode {
        case 2u: { return min(b, s); }                       // Darken
        case 3u: { return b * s; }                           // Multiply
        case 4u: { return color_burn(b, s); }
        case 5u: { return max(b + s - 1.0, 0.0); }           // Linear Burn
        case 7u: { return max(b, s); }                       // Lighten
        case 8u: { return screen(b, s); }
        case 9u: { return color_dodge(b, s); }
        case 10u: { return min(b + s, 1.0); }                // Linear Dodge (Add)
        case 12u: { return hard_light(s, b); }               // Overlay
        case 13u: {                                          // Soft Light
            if s <= 0.5 { return b - (1.0 - 2.0 * s) * b * (1.0 - b); }
            var d = sqrt(b);
            if b <= 0.25 { d = ((16.0 * b - 12.0) * b + 4.0) * b; }
            return b + (2.0 * s - 1.0) * (d - b);
        }
        case 14u: { return hard_light(b, s); }
        case 15u: { return vivid_light(b, s); }
        case 16u: { return clamp(b + 2.0 * s - 1.0, 0.0, 1.0); } // Linear Light
        case 17u: {                                          // Pin Light
            if s <= 0.5 { return min(b, 2.0 * s); }
            return max(b, 2.0 * s - 1.0);
        }
        case 18u: { return select(0.0, 1.0, vivid_light(b, s) >= 0.5); } // Hard Mix
        case 19u: { return abs(b - s); }                     // Difference
        case 20u: { return b + s - 2.0 * b * s; }            // Exclusion
        case 21u: { return max(b - s, 0.0); }                // Subtract
        case 22u: {                                          // Divide
            if s <= 0.0 { return 1.0; }
            return min(b / s, 1.0);
        }
        default: { return s; }
    }
}

fn lum(c: vec3<f32>) -> f32 { return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b; }

fn clip_color(c: vec3<f32>) -> vec3<f32> {
    let l = lum(c);
    let n = min(min(c.r, c.g), c.b);
    let x = max(max(c.r, c.g), c.b);
    var o = c;
    if n < 0.0 { o = l + (o - l) * l / max(l - n, 1e-6); }
    if x > 1.0 { o = l + (o - l) * (1.0 - l) / max(x - l, 1e-6); }
    return o;
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> { return clip_color(c + (l - lum(c))); }

fn sat(c: vec3<f32>) -> f32 { return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b); }

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let mx = max(max(c.r, c.g), c.b);
    let mn = min(min(c.r, c.g), c.b);
    if mx - mn <= 1e-6 { return vec3(0.0); }
    return (c - mn) * s / (mx - mn);
}

fn blend_rgb(mode: u32, b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 23u: { return set_lum(set_sat(s, sat(b)), lum(b)); } // Hue
        case 24u: { return set_lum(set_sat(b, sat(s)), lum(b)); } // Saturation
        case 25u: { return set_lum(s, lum(b)); }                  // Color
        case 26u: { return set_lum(b, lum(s)); }                  // Luminosity
        case 6u: { return select(b, s, lum(s) < lum(b)); }        // Darker Color
        case 11u: { return select(b, s, lum(s) > lum(b)); }       // Lighter Color
        default: { return vec3(sep(mode, b.r, s.r), sep(mode, b.g, s.g), sep(mode, b.b, s.b)); }
    }
}

// Mix the premultiplied color `color` into `d` at `id` (`blend::mix`).
fn mix(d: vec4<f32>, color: vec4<f32>, id: vec2<u32>, mode: u32) -> vec4<f32> {
    var sp = color;
    var sa = sp.a;
    if sa <= 0.0 { return d; }
    if mode == 1u {
        if hash2(id.x, id.y) >= sa { return d; }
        sp = vec4(sp.rgb / sa, 1.0);
        sa = 1.0;
    }
    let da = d.a;
    if mode <= 1u || da <= 0.0 {
        return sp + d * (1.0 - sa);
    }
    let cs = linear_to_srgb(sp.rgb / sa);
    let cb = linear_to_srgb(d.rgb / da);
    let mixed = srgb_to_linear(clamp(blend_rgb(mode, cb, cs), vec3(0.0), vec3(1.0)));
    return vec4(sp.rgb * (1.0 - da) + d.rgb * (1.0 - sa) + sa * da * mixed, sa + da - sa * da);
}

// `aux` placed over `src` (`blend::composite_placed`, `Placement::sample`).
@compute @workgroup_size(16, 16)
fn composite(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let d = textureLoad(src, p, 0);
    if id.x < u.bounds.x || id.x >= u.bounds.y || id.y < u.bounds.z || id.y >= u.bounds.w {
        textureStore(dst, p, d);
        return;
    }
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    let x = u.m0.x * px + u.m0.z * py + u.m1.x;
    let y = u.m0.y * px + u.m0.w * py + u.m1.y;
    let covered = edge(x - u.rect.x, u.m1.z)
        * edge(u.rect.z - x, u.m1.z)
        * edge(y - u.rect.y, u.m1.w)
        * edge(u.rect.w - y, u.m1.w);
    if covered <= 0.0 {
        textureStore(dst, p, d);
        return;
    }
    let color = sample(
        clamp(x, u.range.x + 0.5, max(u.range.z - 0.5, u.range.x + 0.5)),
        clamp(y, u.range.y + 0.5, max(u.range.w - 0.5, u.range.y + 0.5)),
    );
    textureStore(dst, p, mix(d, color * covered * u.p.x, id.xy, u.a.x));
}

// `color.basic` (`kerf_draw::correct::Correction`). p: gain, contrast, brightness, saturation;
// rect: gamma, highlights, vibrance; range: shadows; a.y: which parts run, 1 exposure, 2 eq,
// 4 balance, 8 vibrance.

fn eq_plane(code: f32, contrast: f32, brightness: f32, gamma: f32) -> f32 {
    if contrast == 1.0 && brightness == 0.0 && gamma == 1.0 {
        return code;
    }
    var out: f32;
    if gamma == 1.0 && abs(contrast) < 7.9 {
        let c = trunc(contrast * 4096.0);
        let b = trunc((trunc(100.0 * brightness + 100.0) * 511.0) / 200.0) - 128.0 - trunc(c / 32.0);
        out = code * c / 4096.0 + b;
    } else {
        let v = contrast * (code / 255.0 - 0.5) + 0.5 + brightness;
        if v <= 0.0 {
            out = 0.0;
        } else {
            out = min(pow(v, 1.0 / gamma) * 256.0, 255.0);
        }
    }
    return clamp(out, 0.0, 255.0);
}

fn equalize(c: vec3<f32>) -> vec3<f32> {
    let y0 = 16.0 + 219.0 * (0.299 * c.r + 0.587 * c.g + 0.114 * c.b);
    let cb0 = 128.0 + 224.0 * (-0.168736 * c.r - 0.331264 * c.g + 0.5 * c.b);
    let cr0 = 128.0 + 224.0 * (0.5 * c.r - 0.418688 * c.g - 0.081312 * c.b);
    let y = (eq_plane(y0, u.p.y, u.p.z, u.rect.x) - 16.0) / 219.0;
    let cb = (eq_plane(cb0, u.p.w, 0.0, 1.0) - 128.0) / 224.0;
    let cr = (eq_plane(cr0, u.p.w, 0.0, 1.0) - 128.0) / 224.0;
    return clamp(
        vec3(y + 1.402 * cr, y - 0.344136 * cb - 0.714136 * cr, y + 1.772 * cb),
        vec3(0.0),
        vec3(1.0),
    );
}

fn balance_component(v: f32, l: f32, shadows: f32, highlights: f32) -> f32 {
    let s = shadows * clamp((0.333 - l) * 4.0 + 0.5, 0.0, 1.0) * 0.7;
    let h = highlights * clamp((l + 0.333 - 1.0) * 4.0 + 0.5, 0.0, 1.0) * 0.7;
    return clamp(v + s + h, 0.0, 1.0);
}

fn lightness_channel(n: f32, hue: f32, a: f32, l: f32) -> f32 {
    let k = (n + hue / 30.0) % 12.0;
    return clamp(l - a * clamp(min(k - 3.0, 9.0 - k), -1.0, 1.0), 0.0, 1.0);
}

fn color_balance(c: vec3<f32>) -> vec3<f32> {
    let lightness = max(max(c.r, c.g), c.b) + min(min(c.r, c.g), c.b);
    let r = balance_component(c.r, lightness, u.range.x, u.rect.y);
    let g = balance_component(c.g, lightness, u.range.y, u.rect.y);
    let b = balance_component(c.b, lightness, u.range.z, u.rect.y);
    let mx = max(max(r, g), b);
    let mn = min(min(r, g), b);
    let l = lightness * 0.5;
    var hue = 0.0;
    if !(r == g && g == b) {
        if mx == r {
            hue = 60.0 * ((g - b) / (mx - mn));
        } else if mx == g {
            hue = 60.0 * (2.0 + (b - r) / (mx - mn));
        } else {
            hue = 60.0 * (4.0 + (r - g) / (mx - mn));
        }
    }
    if hue < 0.0 { hue += 360.0; }
    var saturation = 0.0;
    if !(mx == 1.0 || mn == 0.0) {
        saturation = (mx - mn) / (1.0 - abs(2.0 * l - 1.0));
    }
    let a = saturation * min(l, 1.0 - l);
    return vec3(
        lightness_channel(0.0, hue, a, l),
        lightness_channel(8.0, hue, a, l),
        lightness_channel(4.0, hue, a, l),
    );
}

fn vibrance(c: vec3<f32>, intensity: f32) -> vec3<f32> {
    let saturation = max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
    let luma = 0.212656 * c.r + 0.715158 * c.g + 0.072186 * c.b;
    let sign = select(-1.0, 1.0, intensity > 0.0);
    let scale = 1.0 + intensity * (1.0 + sign * saturation);
    return clamp(vec3(luma) + (c - vec3(luma)) * scale, vec3(0.0), vec3(1.0));
}

@compute @workgroup_size(16, 16)
fn correct(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let pixel = textureLoad(src, p, 0);
    let alpha = pixel.a;
    if alpha <= 1e-6 {
        textureStore(dst, p, pixel);
        return;
    }
    var c = linear_to_srgb(pixel.rgb / alpha);
    if (u.a.y & 1u) != 0u { c = clamp(c * u.p.x, vec3(0.0), vec3(1.0)); }
    if (u.a.y & 2u) != 0u { c = equalize(c); }
    if (u.a.y & 4u) != 0u { c = color_balance(c); }
    if (u.a.y & 8u) != 0u { c = vibrance(c, u.rect.z); }
    textureStore(dst, p, vec4(srgb_to_linear(c) * alpha, alpha));
}

// `src` by the alpha of `aux` and by p.x, or (when a.y) by what is left of that.
@compute @workgroup_size(16, 16)
fn by_weight(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let weight = textureLoad(aux, p, 0).a * u.p.x;
    let k = select(weight, 1.0 - weight, u.a.y != 0u);
    textureStore(dst, p, textureLoad(src, p, 0) * k);
}

// `src` plus `aux`.
@compute @workgroup_size(16, 16)
fn plus(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    textureStore(dst, p, textureLoad(src, p, 0) + textureLoad(aux, p, 0));
}
