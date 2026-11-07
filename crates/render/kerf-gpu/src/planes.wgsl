// A decoded frame's planes of Y'CbCr into RGB, as libswscale makes them: into the working picture, by
// the way `decode` takes sRGB-encoded bytes, or into 16-bit code values for `input.wgsl`.

struct U {
    // How far down in size chroma is across and down, as a power of 2; whether chroma is filtered
    // going down; whether to go by whole numbers.
    a: vec4<u32>,
    // Where luma starts and what spans it, then the same of chroma.
    spans: vec4<f32>,
    // How much of Cr goes to red, how much of Cb and of Cr comes off green, and how much of Cb goes to blue.
    mix: vec4<f32>,
    // In whole numbers: the luma offset and scale, unused, unused.
    whole_luma: vec4<i32>,
    // Then what of Cr goes to red, of Cb and of Cr to green, and of Cb to blue.
    whole_mix: vec4<i32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var luma: texture_2d<u32>;
@group(0) @binding(2) var cb: texture_2d<u32>;
@group(0) @binding(3) var cr: texture_2d<u32>;
@group(0) @binding(4) var light: texture_storage_2d<rgba32float, write>;
@group(0) @binding(5) var codes: texture_storage_2d<rgba16uint, write>;

// A pixel's chroma: its sample's, across, as libswscale repeats it. Down, when it asks for it, by the
// bicubic filter libswscale goes by, at the two places a row of 4:2:0 falls between its samples.
fn chroma_at(plane: texture_2d<u32>, p: vec2<i32>) -> f32 {
    let at = vec2<i32>(p.x >> u.a.x, p.y >> u.a.y);
    if u.a.z == 0u || u.a.y == 0u {
        return f32(textureLoad(plane, at, 0).r);
    }
    let last = i32(textureDimensions(plane).y) - 1;
    let tap = taps(plane, at, last);
    if (p.y & 1) == 0 {
        return -0.028125 * tap.x + 0.240625 * tap.y + 0.871875 * tap.z - 0.084375 * tap.w;
    }
    let below = f32(textureLoad(plane, vec2(at.x, min(at.y + 2, last)), 0).r);
    return -0.084375 * tap.y + 0.871875 * tap.z + 0.240625 * tap.w - 0.028125 * below;
}

// The samples two rows above, one above, at, and one below this row of chroma, repeated past its ends.
fn taps(plane: texture_2d<u32>, at: vec2<i32>, last: i32) -> vec4<f32> {
    return vec4(
        f32(textureLoad(plane, vec2(at.x, max(at.y - 2, 0)), 0).r),
        f32(textureLoad(plane, vec2(at.x, max(at.y - 1, 0)), 0).r),
        f32(textureLoad(plane, at, 0).r),
        f32(textureLoad(plane, vec2(at.x, min(at.y + 1, last)), 0).r),
    );
}

fn rgb_at(p: vec2<i32>) -> vec3<f32> {
    let y = (f32(textureLoad(luma, p, 0).r) - u.spans.x) / u.spans.y;
    let b = (chroma_at(cb, p) - u.spans.z) / u.spans.w;
    let r = (chroma_at(cr, p) - u.spans.z) / u.spans.w;
    return clamp(
        vec3(y + u.mix.x * r, y - u.mix.y * b - u.mix.z * r, y + u.mix.w * b),
        vec3(0.0),
        vec3(1.0),
    );
}

// As libswscale's fast paths go to 8-bit RGBA, each product kept to 15 bits, and halved and rounded at
// the end: the same bytes where it takes one, and within a step where it does not.
fn whole_rgb_at(p: vec2<i32>) -> vec3<f32> {
    let at = vec2<i32>(p.x >> u.a.x, p.y >> u.a.y);
    let b = (i32(textureLoad(cb, at, 0).r) - 128) * 8;
    let r = (i32(textureLoad(cr, at, 0).r) - 128) * 8;
    let y = ((i32(textureLoad(luma, p, 0).r) * 8 - u.whole_luma.x) * u.whole_luma.y) >> 15u;
    let red = (r * u.whole_mix.x) >> 15u;
    let green = ((b * u.whole_mix.y) >> 15u) + ((r * u.whole_mix.z) >> 15u);
    let blue = (b * u.whole_mix.w) >> 15u;
    let bytes = clamp((vec3(y) + vec3(red, green, blue) + vec3(1)) >> vec3(1u), vec3(0), vec3(255));
    return vec3<f32>(bytes) / 255.0;
}

@compute @workgroup_size(16, 16)
fn planes_light(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(light);
    if id.x >= size.x || id.y >= size.y { return; }
    let p = vec2<i32>(id.xy);
    var rgb: vec3<f32>;
    if u.a.w != 0u {
        rgb = whole_rgb_at(p);
    } else {
        rgb = rgb_at(p);
    }
    let hi = pow((rgb + 0.055) / 1.055, vec3(2.4));
    textureStore(light, p, vec4(select(hi, rgb / 12.92, rgb <= vec3(0.04045)), 1.0));
}

@compute @workgroup_size(16, 16)
fn planes_codes(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(codes);
    if id.x >= size.x || id.y >= size.y { return; }
    let p = vec2<i32>(id.xy);
    let code = vec3<u32>(round(rgb_at(p) * 65535.0));
    textureStore(codes, p, vec4<u32>(code, 65535u));
}
