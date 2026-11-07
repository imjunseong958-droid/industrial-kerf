// A camera's blur of a clip in depth: the passes of `kerf-compose/src/draw/focus.rs`, the CPU reference.
// A picture is gathered at a few radii and each pixel mixes the two around its own, which it has from
// how far off its plane is there. What radii those are depends on what was drawn, and a kernel cannot
// make textures, so `find_reach` and `find_range` say what is needed for the frame to ask for it.

struct Span {
    dy: i32,
    // Where the span's ends are from the pixel, in whole sums past `room`, and the rest of the way.
    high: i32,
    low: i32,
    unused: i32,
    parts: vec4<f32>, // high's, low's
};

struct U {
    size: vec4<i32>,     // w, h of what is read; how many sums each row has either side; depth's first row
    area: vec4<i32>,     // left, top, right, bottom it is needed in, of this picture, all included
    focus: vec4<f32>,    // focus, aperture, most
    margin: vec4<f32>,   // a blur's reach past what it has: by the radius, and more
    gather: vec4<f32>,   // the disc's area, the least and most radius that read this level, how many spans
    levels: array<vec4<f32>, 7>, // radius, how much smaller it is gathered, w, h
    counts: vec4<i32>,   // how many levels, how much smaller this one is gathered
    spans: array<Span, 17>,
};

// As `found` has them: where what has alpha is, as far as its radius, and the radii around that.
const LEFT: u32 = 0u;
const TOP: u32 = 1u;
const RIGHT: u32 = 2u;
const BOTTOM: u32 = 3u;
const REACH: u32 = 4u;
const LEAST: u32 = 5u;
const MOST: u32 = 6u;

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var depths: texture_2d<f32>;
@group(0) @binding(4) var<storage, read_write> found: array<atomic<u32>>;
@group(0) @binding(5) var level0: texture_2d<f32>;
@group(0) @binding(6) var level1: texture_2d<f32>;
@group(0) @binding(7) var level2: texture_2d<f32>;
@group(0) @binding(8) var level3: texture_2d<f32>;
@group(0) @binding(9) var level4: texture_2d<f32>;
@group(0) @binding(10) var level5: texture_2d<f32>;
@group(0) @binding(11) var level6: texture_2d<f32>;

const TILE: i32 = 16;
// `INFINITY` on the CPU, as `depth.wgsl` writes it.
const NOT_THERE: f32 = 1e29;

// `focus::radius_at`.
fn radius_at(x: i32, y: i32) -> f32 {
    let depth = textureLoad(depths, vec2(x, y + u.size.w), 0).r;
    if depth >= NOT_THERE || depth <= 0.0 {
        return u.focus.z;
    }
    return min(u.focus.y * abs(depth - u.focus.x) / depth, u.focus.z);
}

// A tile at a time, one invocation to each, so `found` is touched once a tile and not once a pixel.
// A radius is never negative, so its bits are in the same order as it is.
@compute @workgroup_size(1)
fn find_reach(@builtin(global_invocation_id) id: vec3<u32>) {
    let start = vec2<i32>(id.xy) * TILE;
    let end = min(start + TILE, u.size.xy);
    var low = end;
    var high = start - 1;
    var reach = 0.0;
    for (var y = start.y; y < end.y; y++) {
        for (var x = start.x; x < end.x; x++) {
            if textureLoad(src, vec2(x, y), 0).a > 0.0 {
                low = min(low, vec2(x, y));
                high = max(high, vec2(x, y));
                reach = max(reach, radius_at(x, y));
            }
        }
    }
    if high.x < start.x {
        return;
    }
    atomicMin(&found[LEFT], u32(low.x));
    atomicMin(&found[TOP], u32(low.y));
    atomicMax(&found[RIGHT], u32(high.x));
    atomicMax(&found[BOTTOM], u32(high.y));
    atomicMax(&found[REACH], bitcast<u32>(reach));
}

@compute @workgroup_size(1)
fn find_range(@builtin(global_invocation_id) id: vec3<u32>) {
    let reach = bitcast<f32>(atomicLoad(&found[REACH]));
    let margin = i32(ceil(reach * u.margin.x) + u.margin.y);
    let area_low = max(vec2(i32(atomicLoad(&found[LEFT])), i32(atomicLoad(&found[TOP]))) - margin, vec2(0));
    let area_high = min(
        vec2(i32(atomicLoad(&found[RIGHT])), i32(atomicLoad(&found[BOTTOM]))) + margin,
        u.size.xy - 1,
    );
    let start = max(vec2<i32>(id.xy) * TILE, area_low);
    let end = min((vec2<i32>(id.xy) + 1) * TILE - 1, area_high);
    if end.x < start.x || end.y < start.y {
        return;
    }
    var least = 1e30;
    var most = 0.0;
    for (var y = start.y; y <= end.y; y++) {
        for (var x = start.x; x <= end.x; x++) {
            let radius = radius_at(x, y);
            least = min(least, radius);
            most = max(most, radius);
        }
    }
    atomicMin(&found[LEAST], bitcast<u32>(least));
    atomicMax(&found[MOST], bitcast<u32>(most));
}

// `focus::shrunk`, over what a gather reads.
@compute @workgroup_size(16, 16)
fn shrink(@builtin(global_invocation_id) id: vec3<u32>) {
    let by = u.counts.y;
    let at = vec2<i32>(id.xy);
    if any(at < u.area.xy) || any(at > u.area.zw) {
        return;
    }
    let end = min((at + 1) * by, u.size.xy);
    var sum = vec4(0.0);
    var count = 0.0;
    for (var y = at.y * by; y < end.y; y++) {
        for (var x = at.x * by; x < end.x; x++) {
            sum += textureLoad(src, vec2(x, y), 0);
            count += 1.0;
        }
    }
    textureStore(dst, at, sum / count);
}

// `focus::running_sums` over what a gather reads, a row to an invocation. A gather only takes one sum
// from another, so a row may start counting where it is first read.
@compute @workgroup_size(1)
fn row_sums(@builtin(global_invocation_id) id: vec3<u32>) {
    let y = i32(id.x) + u.area.y;
    let w = u.size.x;
    let room = u.size.z;
    let start = u.area.x;
    let end = u.area.z;
    var sum = vec4(0.0);
    textureStore(dst, vec2(room + start, y), sum);
    if start == 0 {
        let first = textureLoad(src, vec2(0, y), 0);
        for (var index = room - 1; index >= 0; index--) {
            sum -= first;
            textureStore(dst, vec2(index, y), sum);
        }
    }
    sum = vec4(0.0);
    for (var x = start; x <= end; x++) {
        sum += textureLoad(src, vec2(x, y), 0);
        textureStore(dst, vec2(room + x + 1, y), sum);
    }
    if end == w - 1 {
        let last = textureLoad(src, vec2(w - 1, y), 0);
        for (var past = room + w + 1; past < w + 2 * room + 1; past++) {
            sum += last;
            textureStore(dst, vec2(past, y), sum);
        }
    }
}

// `focus::gathered`, from `row_sums`. At its own size a pixel reads only the levels around its radius,
// so the rest of the frame need not have them.
@compute @workgroup_size(16, 16)
fn gather(@builtin(global_invocation_id) id: vec3<u32>) {
    let at = vec2<i32>(id.xy);
    if any(at < u.area.xy) || any(at > u.area.zw) {
        return;
    }
    if u.counts.y == 1 {
        let radius = radius_at(at.x, at.y);
        if radius < u.gather.y || radius > u.gather.z {
            return;
        }
    }
    let spans = i32(u.gather.w);
    var sum = vec4(0.0);
    for (var index = 0; index < spans; index++) {
        let span = u.spans[index];
        let line = clamp(at.y + span.dy, 0, u.size.y - 1);
        let a = textureLoad(src, vec2(span.high + at.x, line), 0);
        let b = textureLoad(src, vec2(span.high + at.x + 1, line), 0);
        let c = textureLoad(src, vec2(span.low + at.x, line), 0);
        let d = textureLoad(src, vec2(span.low + at.x + 1, line), 0);
        sum += a + (b - a) * span.parts.x - c - (d - c) * span.parts.y;
    }
    textureStore(dst, at, sum / u.gather.x);
}

fn level_at(level: i32, at: vec2<i32>) -> vec4<f32> {
    switch level {
        case 0: { return textureLoad(level0, at, 0); }
        case 1: { return textureLoad(level1, at, 0); }
        case 2: { return textureLoad(level2, at, 0); }
        case 3: { return textureLoad(level3, at, 0); }
        case 4: { return textureLoad(level4, at, 0); }
        case 5: { return textureLoad(level5, at, 0); }
        default: { return textureLoad(level6, at, 0); }
    }
}

// `Image::sample_bilinear_clamped` at a pixel of the frame, on what was gathered smaller.
fn read(level: i32, x: i32, y: i32) -> vec4<f32> {
    let info = u.levels[level];
    let by = info.y;
    if by == 1.0 {
        return level_at(level, vec2(x, y));
    }
    let size = info.zw;
    let f = clamp((vec2(f32(x), f32(y)) + 0.5) / by - 0.5, vec2(0.0), size - 1.0);
    let p0 = vec2<i32>(floor(f));
    let p1 = min(p0 + 1, vec2<i32>(size) - 1);
    let t = f - vec2<f32>(p0);
    let a = level_at(level, p0);
    let b = level_at(level, vec2(p1.x, p0.y));
    let c = level_at(level, vec2(p0.x, p1.y));
    let d = level_at(level, p1);
    let top = a + (b - a) * t.x;
    let bottom = c + (d - c) * t.x;
    return top + (bottom - top) * t.y;
}

// The end of `focus::blurred`.
@compute @workgroup_size(16, 16)
fn focused(@builtin(global_invocation_id) id: vec3<u32>) {
    let at = vec2<i32>(id.xy);
    if any(at >= u.size.xy) {
        return;
    }
    if any(at < u.area.xy) || any(at > u.area.zw) {
        textureStore(dst, at, vec4(0.0));
        return;
    }
    let own = radius_at(at.x, at.y);
    let count = u.counts.x;
    var next = 0;
    while next < count && u.levels[next].x < own {
        next++;
    }
    var pixel: vec4<f32>;
    if next == 0 {
        pixel = read(0, at.x, at.y);
    } else if next == count {
        pixel = read(count - 1, at.x, at.y);
    } else {
        let low = u.levels[next - 1].x;
        let along = (own - low) / (u.levels[next].x - low);
        let a = read(next - 1, at.x, at.y);
        pixel = a + (read(next, at.x, at.y) - a) * along;
    }
    textureStore(dst, at, pixel);
}
