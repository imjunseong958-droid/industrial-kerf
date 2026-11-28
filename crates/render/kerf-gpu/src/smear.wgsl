// A picture smeared over where it goes while a shutter is open: what
// `kerf_draw::blend::composite_smeared`, the CPU reference, adds up before it lays the smear down.
// A copy of the picture at each place it has is sampled as `composite` in `picture.wgsl` samples
// one, and each pixel's samples are added in the order of the places, as the CPU adds them. A pass
// for each copy would read and write the whole picture as often as there are copies.

struct Place {
    m0: vec4<f32>,     // inverse a, b, c, d
    m1: vec4<f32>,     // inverse e, f, across, down
    bounds: vec4<u32>, // x0, x1, y0, y1
};

struct U {
    a: vec4<u32>,     // how many places, `src` holds what the places before these came to
    rect: vec4<f32>,  // left, top, right, bottom
    range: vec4<f32>, // lo x, lo y, hi x, hi y
    p: vec4<f32>,     // what one place is of them all
    places: array<Place, 64>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var aux: texture_2d<f32>;

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

// `aux` at each of the places, added to what `src` holds of the places before them.
@compute @workgroup_size(16, 16)
fn smear(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst);
    if id.x >= size.x || id.y >= size.y { return; }
    let p = vec2<i32>(id.xy);
    var sum = vec4<f32>(0.0);
    if u.a.y != 0u {
        sum = textureLoad(src, p, 0);
    }
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    for (var n = 0u; n < u.a.x; n++) {
        let bounds = u.places[n].bounds;
        if id.x < bounds.x || id.x >= bounds.y || id.y < bounds.z || id.y >= bounds.w {
            continue;
        }
        let m0 = u.places[n].m0;
        let m1 = u.places[n].m1;
        let x = m0.x * px + m0.z * py + m1.x;
        let y = m0.y * px + m0.w * py + m1.y;
        let covered = edge(x - u.rect.x, m1.z)
            * edge(u.rect.z - x, m1.z)
            * edge(y - u.rect.y, m1.w)
            * edge(u.rect.w - y, m1.w);
        if covered <= 0.0 {
            continue;
        }
        let color = sample(
            clamp(x, u.range.x + 0.5, max(u.range.z - 0.5, u.range.x + 0.5)),
            clamp(y, u.range.y + 0.5, max(u.range.w - 0.5, u.range.y + 0.5)),
        );
        sum += color * covered * u.p.x;
    }
    textureStore(dst, p, sum);
}
