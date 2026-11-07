// What `depth.wgsl` reads of other pictures, made ahead into buffers: a picture's alpha, its summed
// table (`three_d::Held`), and how much light each way a surface can face gets from a picture all
// around (`three_d::Irradiance`).

struct P {
    size: vec4<u32>, // w, h, where in the buffer it goes
};

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> floats: array<f32>;
@group(0) @binding(3) var<storage, read_write> table: array<u32>;

const ACROSS: u32 = 32u;
const DOWN: u32 = 16u;
const TAU: f32 = 6.283185307179586;
const PI: f32 = 3.141592653589793;

@compute @workgroup_size(64)
fn alpha_of(@builtin(global_invocation_id) id: vec3<u32>) {
    let y = id.x;
    if y >= p.size.y {
        return;
    }
    let w = p.size.x;
    for (var x = 0u; x < w; x++) {
        floats[p.size.z + y * w + x] = textureLoad(src, vec2(i32(x), i32(y)), 0).a;
    }
}

// Each row's own running sums, with a row and a column of nothing before the first.
@compute @workgroup_size(64)
fn table_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let y = id.x;
    if y >= p.size.y {
        return;
    }
    let w = p.size.x;
    let stride = w + 1u;
    let row = p.size.z + (y + 1u) * stride;
    if y == 0u {
        for (var x = 0u; x < stride; x++) {
            table[p.size.z + x] = 0u;
        }
    }
    var line = 0u;
    table[row] = 0u;
    for (var x = 0u; x < w; x++) {
        let alpha = textureLoad(src, vec2(i32(x), i32(y)), 0).a;
        line += u32(max(alpha * 255.0 + 0.5, 0.0));
        table[row + x + 1u] = line;
    }
}

// Those summed down each column, wrapped as they may be.
@compute @workgroup_size(64)
fn table_columns(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    let stride = p.size.x + 1u;
    if x >= stride {
        return;
    }
    var line = 0u;
    for (var y = 1u; y <= p.size.y; y++) {
        let at = p.size.z + y * stride + x;
        line += table[at];
        table[at] = line;
    }
}

fn sample_clamped(x: f32, y: f32) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(src));
    let f = clamp(vec2(x, y) - 0.5, vec2(0.0), vec2<f32>(size - 1));
    let p0 = vec2<i32>(floor(f));
    let p1 = min(p0 + 1, size - 1);
    let t = f - vec2<f32>(p0);
    let a = textureLoad(src, p0, 0).rgb;
    let b = textureLoad(src, vec2(p1.x, p0.y), 0).rgb;
    let c = textureLoad(src, vec2(p0.x, p1.y), 0).rgb;
    let d = textureLoad(src, p1, 0).rgb;
    let top = a + (b - a) * t.x;
    let bottom = c + (d - c) * t.x;
    return top + (bottom - top) * t.y;
}

// What comes from each cell of a panorama, a few reads of the picture to a cell.
@compute @workgroup_size(64)
fn radiance(@builtin(global_invocation_id) id: vec3<u32>) {
    let cell = id.x;
    if cell >= ACROSS * DOWN {
        return;
    }
    let size = vec2<f32>(textureDimensions(src));
    let column = f32(cell % ACROSS);
    let row = f32(cell / ACROSS);
    var sum = vec3(0.0);
    for (var step_y = 0u; step_y < 4u; step_y++) {
        for (var step_x = 0u; step_x < 4u; step_x++) {
            let u = (column + (f32(step_x) + 0.5) / 4.0) / f32(ACROSS);
            let v = (row + (f32(step_y) + 0.5) / 4.0) / f32(DOWN);
            sum += sample_clamped(u * size.x, v * size.y);
        }
    }
    let at = p.size.z + cell * 4u;
    floats[at] = sum.x / 16.0;
    floats[at + 1u] = sum.y / 16.0;
    floats[at + 2u] = sum.z / 16.0;
    floats[at + 3u] = 1.0;
}

fn way_of(cell: u32) -> vec3<f32> {
    let around = ((f32(cell % ACROSS) + 0.5) / f32(ACROSS) - 0.5) * TAU;
    let down = (f32(cell / ACROSS) + 0.5) / f32(DOWN) * PI;
    let ring = sin(down);
    return vec3(ring * sin(around), -cos(down), ring * cos(around));
}

// How much of it reaches a surface facing each way, written just past what it reads.
@compute @workgroup_size(64)
fn irradiance(@builtin(global_invocation_id) id: vec3<u32>) {
    let cell = id.x;
    let cells = ACROSS * DOWN;
    if cell >= cells {
        return;
    }
    let facing = way_of(cell);
    var sum = vec3(0.0);
    for (var source = 0u; source < cells; source++) {
        let cosine = dot(facing, way_of(source));
        if cosine <= 0.0 {
            continue;
        }
        // Cells near the top and bottom of a panorama cover less of what is around.
        let row = f32(source / ACROSS);
        let solid_angle = (TAU / f32(ACROSS)) * (PI / f32(DOWN)) * sin((row + 0.5) / f32(DOWN) * PI);
        let at = p.size.z + source * 4u;
        sum += vec3(floats[at], floats[at + 1u], floats[at + 2u]) * cosine * solid_angle;
    }
    let at = p.size.z + (cells + cell) * 4u;
    floats[at] = sum.x / PI;
    floats[at + 1u] = sum.y / PI;
    floats[at + 2u] = sum.z / PI;
    floats[at + 3u] = 1.0;
}
