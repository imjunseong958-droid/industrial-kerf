// The working picture of an HDR sequence on its way out: `kerf_color::HdrOutput`, the reference,
// to linear SDR or to its signal.

struct U {
    // 0 linear SDR, 1 the PQ signal, 2 the HLG signal; unused, unused, unused.
    a: vec4<u32>,
    // The tone map's source and target peaks in nits, the HLG OOTF's scale and gamma.
    b: vec4<f32>,
    // BT.709 primaries to BT.2020.
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    bt2020: vec4<f32>,
    bt709: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var tone_table: texture_2d<f32>;

const REFERENCE_WHITE_NITS: f32 = 203.0;

fn pq_inverse_eotf(y: f32) -> f32 {
    let p = pow(max(y, 0.0), 0.15930176);
    return pow((0.8359375 + 18.851563 * p) / (1.0 + 18.6875 * p), 78.84375);
}

fn hlg_oetf(l: f32) -> f32 {
    if l <= 1.0 / 12.0 { return sqrt(3.0 * max(l, 0.0)); }
    return 0.17883277 * log(12.0 * l - 0.28466892) + 0.5599107;
}

// `ToneMap::apply`.
fn tone_map(c: vec3<f32>) -> vec3<f32> {
    let m = max(max(c.r, c.g), c.b);
    if m <= 0.0 { return c; }
    let nits = m * REFERENCE_WHITE_NITS;
    if nits >= u.b.x {
        return c * (u.b.y / REFERENCE_WHITE_NITS / m);
    }
    let last = i32(textureDimensions(tone_table).x) - 1;
    let p = clamp(pq_inverse_eotf(nits / 10000.0), 0.0, 1.0) * f32(last);
    let i = min(i32(p), last - 1);
    let a = textureLoad(tone_table, vec2(i, 0), 0).r;
    let b = textureLoad(tone_table, vec2(i + 1, 0), 0).r;
    return c * (a + (b - a) * (p - f32(i)));
}

// `transform::desaturate_into_gamut`.
fn desaturate(c: vec3<f32>, luma: vec3<f32>) -> vec3<f32> {
    let lowest = min(min(c.r, c.g), c.b);
    if lowest >= 0.0 { return c; }
    let y = dot(luma, c);
    if y <= 0.0 { return vec3(0.0); }
    let t = y / (y - lowest);
    return max(vec3(y) + (c - vec3(y)) * t, vec3(0.0));
}

fn signal(rgb: vec3<f32>) -> vec3<f32> {
    let m = vec3(dot(u.m0.xyz, rgb), dot(u.m1.xyz, rgb), dot(u.m2.xyz, rgb));
    let c = desaturate(m, u.bt2020.rgb);
    if u.a.x == 1u {
        let k = REFERENCE_WHITE_NITS / 10000.0;
        return vec3(pq_inverse_eotf(c.r * k), pq_inverse_eotf(c.g * k), pq_inverse_eotf(c.b * k));
    }
    let luma = dot(u.bt2020.rgb, c);
    if luma <= 0.0 { return vec3(0.0); }
    let scene = pow(luma / u.b.z, 1.0 / u.b.w);
    let scale = pow(scene, 1.0 - u.b.w) / u.b.z;
    let e = min(c * scale, vec3(1.0));
    return vec3(hlg_oetf(e.r), hlg_oetf(e.g), hlg_oetf(e.b));
}

@compute @workgroup_size(16, 16)
fn output(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst);
    if id.x >= size.x || id.y >= size.y { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    var rgb: vec3<f32>;
    if u.a.x == 0u {
        rgb = desaturate(tone_map(c.rgb), u.bt709.rgb);
    } else {
        rgb = signal(c.rgb);
    }
    textureStore(dst, p, vec4(rgb, c.a));
}
