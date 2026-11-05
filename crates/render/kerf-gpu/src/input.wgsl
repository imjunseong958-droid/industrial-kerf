// Deep footage into the working picture: `kerf_color::InputTransform::convert`, the reference, on
// straight 16-bit code values. A table for the curve, then the HLG OOTF, the gamut matrix, tone
// mapping, and bringing the color into gamut, each only where the footage's color space asks.

struct U {
    // Which stages run: 1 HLG, 2 matrix, 4 tone map, 8 compress, 16 desaturate; the picture's
    // transparency counts; unused, unused.
    a: vec4<u32>,
    // The curve table's low and high ends, the tone map's source and target peaks in nits.
    range: vec4<f32>,
    // The HLG OOTF's luma weights and its scale to working units.
    hlg: vec4<f32>,
    // The matrix's rows; `m0.w` is the HLG OOTF's power.
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // The working gamut's luma weights.
    luma: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<u32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var curve: texture_2d<f32>;
@group(0) @binding(4) var tone_table: texture_2d<f32>;

const REFERENCE_WHITE_NITS: f32 = 203.0;

// `DecodeTable::lookup`.
fn lookup(table: texture_2d<f32>, low: f32, high: f32, v: f32) -> f32 {
    let last = i32(textureDimensions(table).x) - 1;
    let p = clamp((v - low) / (high - low), 0.0, 1.0) * f32(last);
    let i = min(i32(p), last - 1);
    let f = p - f32(i);
    let a = textureLoad(table, vec2(i, 0), 0).r;
    let b = textureLoad(table, vec2(i + 1, 0), 0).r;
    return a + (b - a) * f;
}

fn pq_inverse_eotf(y: f32) -> f32 {
    let p = pow(max(y, 0.0), 0.15930176);
    return pow((0.8359375 + 18.851563 * p) / (1.0 + 18.6875 * p), 78.84375);
}

// `ToneMap::apply`.
fn tone_map(c: vec3<f32>) -> vec3<f32> {
    let m = max(max(c.r, c.g), c.b);
    if m <= 0.0 { return c; }
    let nits = m * REFERENCE_WHITE_NITS;
    if nits >= u.range.z {
        return c * (u.range.w / REFERENCE_WHITE_NITS / m);
    }
    return c * lookup(tone_table, 0.0, 1.0, pq_inverse_eotf(nits / 10000.0));
}

// `transform::gamut_compress`: threshold 0.8, limit 1.25, power 1.2.
fn compress_channel(v: f32, achromatic: f32) -> f32 {
    let threshold = 0.8;
    let limit = 1.25;
    let power = 1.2;
    let d = (achromatic - v) / achromatic;
    if d < threshold { return v; }
    let s = (limit - threshold) / pow(pow((limit - threshold) / (1.0 - threshold), power) - 1.0, 1.0 / power);
    let x = (d - threshold) / s;
    let dc = threshold + s * x / pow(1.0 + pow(x, power), 1.0 / power);
    return achromatic - dc * achromatic;
}

fn gamut_compress(c: vec3<f32>) -> vec3<f32> {
    let achromatic = max(max(c.r, c.g), c.b);
    if achromatic <= 1e-9 { return max(c, vec3(0.0)); }
    return vec3(
        compress_channel(c.r, achromatic),
        compress_channel(c.g, achromatic),
        compress_channel(c.b, achromatic),
    );
}

// `transform::desaturate_into_gamut`.
fn desaturate(c: vec3<f32>) -> vec3<f32> {
    let lowest = min(min(c.r, c.g), c.b);
    if lowest >= 0.0 { return c; }
    let y = dot(u.luma.rgb, c);
    if y <= 0.0 { return vec3(0.0); }
    let t = y / (y - lowest);
    return max(vec3(y) + (c - vec3(y)) * t, vec3(0.0));
}

@compute @workgroup_size(16, 16)
fn input(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst);
    if id.x >= size.x || id.y >= size.y { return; }
    let p = vec2<i32>(id.xy);
    let code = vec4<f32>(textureLoad(src, p, 0)) / 65535.0;
    let a = select(1.0, code.a, u.a.y != 0u);
    var c = vec3(
        lookup(curve, u.range.x, u.range.y, code.r),
        lookup(curve, u.range.x, u.range.y, code.g),
        lookup(curve, u.range.x, u.range.y, code.b),
    );
    if (u.a.x & 1u) != 0u {
        let ys = max(dot(u.hlg.rgb, c), 0.0);
        c = c * (u.hlg.w * pow(ys, u.m0.w));
    }
    if (u.a.x & 2u) != 0u {
        c = vec3(dot(u.m0.xyz, c), dot(u.m1.xyz, c), dot(u.m2.xyz, c));
    }
    if (u.a.x & 4u) != 0u { c = tone_map(c); }
    if (u.a.x & 8u) != 0u { c = gamut_compress(c); }
    if (u.a.x & 16u) != 0u { c = desaturate(c); }
    textureStore(dst, p, vec4(c * a, a));
}
