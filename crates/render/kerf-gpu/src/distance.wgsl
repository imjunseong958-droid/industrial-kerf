// How far each pixel of a picture is from the outline of its alpha, by jump flooding, for a solid's
// bevel: the passes of `kerf-compose/src/draw/bevel.rs`, the CPU reference. A kernel cannot read what it
// writes, so each pass reads the last one's places.

struct U {
    size: vec4<i32>, // w, h, how far this pass jumps
    crop: vec4<f32>, // left, top, right, bottom
    far: vec4<f32>,  // how far counts; what is left of the outline taken in by this
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var read_in: texture_2d<f32>;
@group(0) @binding(3) var places: texture_storage_2d<rg32float, write>;
@group(0) @binding(4) var distance: texture_storage_2d<r32float, write>;
@group(0) @binding(5) var taken_in: texture_storage_2d<rgba32float, write>;

const NOWHERE: vec2<f32> = vec2(-1e6, -1e6);

fn alpha(x: i32, y: i32) -> f32 {
    if x < 0 || y < 0 || x >= u.size.x || y >= u.size.y {
        return 0.0;
    }
    let cx = f32(x) + 0.5;
    let cy = f32(y) + 0.5;
    if cx < u.crop.x || cy < u.crop.y || cx > u.crop.z || cy > u.crop.w {
        return 0.0;
    }
    return textureLoad(picture, vec2(x, y), 0).a;
}

fn gap(a: vec2<f32>, b: vec2<f32>) -> f32 {
    let d = a - b;
    return dot(d, d);
}

@compute @workgroup_size(16, 16)
fn outline(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = i32(id.x);
    let y = i32(id.y);
    if x >= u.size.x || y >= u.size.y {
        return;
    }
    let at = vec2<f32>(id.xy) + 0.5;
    let here = alpha(x, y);
    var seed = NOWHERE;
    let ways = array<vec2<i32>, 4>(vec2(1, 0), vec2(-1, 0), vec2(0, 1), vec2(0, -1));
    for (var way = 0; way < 4; way++) {
        let there = alpha(x + ways[way].x, y + ways[way].y);
        if (here >= 0.5) == (there >= 0.5) {
            continue;
        }
        let along = (here - 0.5) / (here - there);
        let met = at + along * vec2<f32>(ways[way]);
        if gap(met, at) < gap(seed, at) {
            seed = met;
        }
    }
    textureStore(places, vec2(x, y), vec4(seed, 0.0, 0.0));
}

@compute @workgroup_size(16, 16)
fn jump(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = i32(id.x);
    let y = i32(id.y);
    if x >= u.size.x || y >= u.size.y {
        return;
    }
    let at = vec2<f32>(id.xy) + 0.5;
    let step = u.size.z;
    var best = textureLoad(read_in, vec2(x, y), 0).rg;
    for (var dy = -step; dy <= step; dy += step) {
        for (var dx = -step; dx <= step; dx += step) {
            let nx = x + dx;
            let ny = y + dy;
            if nx < 0 || ny < 0 || nx >= u.size.x || ny >= u.size.y {
                continue;
            }
            let there = textureLoad(read_in, vec2(nx, ny), 0).rg;
            if gap(there, at) < gap(best, at) {
                best = there;
            }
        }
    }
    textureStore(places, vec2(x, y), vec4(best, 0.0, 0.0));
}

@compute @workgroup_size(16, 16)
fn signed_distance(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = i32(id.x);
    let y = i32(id.y);
    if x >= u.size.x || y >= u.size.y {
        return;
    }
    let at = vec2<f32>(id.xy) + 0.5;
    let seed = textureLoad(read_in, vec2(x, y), 0).rg;
    let reach = min(sqrt(gap(seed, at)), u.far.x);
    let value = select(-reach, reach, alpha(x, y) >= 0.5);
    textureStore(distance, vec2(x, y), vec4(value, 0.0, 0.0, 0.0));
}

// A picture with its outline taken in (`bevel::eroded`).
@compute @workgroup_size(16, 16)
fn eroded(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = i32(id.x);
    let y = i32(id.y);
    if x >= u.size.x || y >= u.size.y {
        return;
    }
    let left = clamp(textureLoad(read_in, vec2(x, y), 0).r - u.far.y + 0.5, 0.0, 1.0);
    textureStore(taken_in, vec2(x, y), textureLoad(picture, vec2(x, y), 0) * left);
}
