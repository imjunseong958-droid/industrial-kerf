// Clips in depth: the arithmetic of `kerf-compose/src/draw/three_d.rs`, the CPU reference, with the
// world measured from the eye so that 32 bits are enough. Each pixel's ray meets a clip's plane, reads
// its picture there, and is lit, with what stands in a light's way read from buffers made ahead
// (`depth_prep.wgsl`), since a kernel cannot take a varying number of textures.

struct U {
    size: vec4<u32>,   // output w, h, picture w, h
    what: vec4<u32>,   // this clip's number, write depth, depth's first row, its own table is summed
    crop: vec4<f32>,   // left, top, right, bottom
    origin: vec4<f32>, // the plane's first pixel; w: how much of its size the picture is read at
    back: vec4<f32>,   // the same of the plane behind it, for a solid
    normal: vec4<f32>,
    across: vec4<f32>, // a unit long, for a solid's sides
    down: vec4<f32>,   // a unit long, for a solid's sides
    to_u: vec4<f32>,
    to_v: vec4<f32>,
    bevel: vec4<f32>,  // how wide its bevel is and how deep the solid, in the pixels the picture is read at
};

struct Light {
    place: vec4<f32>, // a point's place or the way back along a directional's light; w: shadow
    color: vec4<f32>, // w: a point's radius or a directional's spread
    what: vec4<u32>,  // kind, where its table is in `floats`, it has one
};

struct Scene {
    right: vec4<f32>,   // w: focal
    down: vec4<f32>,    // w: center x
    forward: vec4<f32>, // w: center y
    counts: vec4<u32>,  // lights, stand-ins
    near: vec4<f32>,    // how near is seen, each ray's share of a light
    spiral: array<vec2<f32>, 32>,
    lights: array<Light>,
};

struct Caster {
    what: vec4<u32>,   // clip, opaque all over, where its alpha is, where its table is
    size: vec4<u32>,   // picture w, h, it has a table
    crop: vec4<f32>,
    origin: vec4<f32>, // w: opacity
    normal: vec4<f32>,
    to_u: vec4<f32>,
    to_v: vec4<f32>,
    across: vec4<f32>, // a unit long; w: how many of its pixels a pixel of the world is
    down: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var depth_out: texture_storage_2d<r32float, write>;
@group(0) @binding(4) var<storage, read> scene: Scene;
@group(0) @binding(5) var<storage, read> casters: array<Caster>;
@group(0) @binding(6) var<storage, read> floats: array<f32>;
@group(0) @binding(7) var<storage, read> tables: array<u32>;
@group(0) @binding(8) var<storage, read> own_table: array<u32>;
@group(0) @binding(9) var distance: texture_2d<f32>;

const RAYS: u32 = 32u;
const MOST_STEPS: u32 = 1024u;
const SIDE_REACH: f32 = 2.0;
const FAR: f32 = 1e30;
// `INFINITY` on the CPU, which a kernel cannot be sure to compute with.
const NO_DEPTH: f32 = 1e30;
const TAU: f32 = 6.283185307179586;
const PI: f32 = 3.141592653589793;
const ACROSS: i32 = 32;
const DOWN: i32 = 16;

const AMBIENT: u32 = 0u;
const POINT: u32 = 1u;
const DIRECTIONAL: u32 = 2u;

const TAKES_NOTHING: f32 = 0.0;
const TAKES_SOME: f32 = 1.0;
const TAKES_ALL: f32 = 2.0;

fn unit(a: vec3<f32>) -> vec3<f32> {
    let length = sqrt(dot(a, a));
    return select(a, a / length, length > 1e-12);
}

struct Met {
    ok: bool,
    hit: vec3<f32>,
    ahead: f32,
};

// `Scene::meets`.
fn meets(origin: vec3<f32>, place: vec2<f32>) -> Met {
    let ray = scene.right.xyz * (place.x - scene.down.w)
        + scene.down.xyz * (place.y - scene.forward.w)
        + scene.forward.xyz * scene.right.w;
    let toward = dot(ray, u.normal.xyz);
    if abs(toward) < 1e-12 {
        return Met(false, vec3(0.0), 0.0);
    }
    let along = dot(origin, u.normal.xyz) / toward;
    let ahead = along * dot(ray, scene.forward.xyz);
    if ahead < scene.near.x {
        return Met(false, vec3(0.0), 0.0);
    }
    return Met(true, ray * along, ahead);
}

fn at_plane(origin: vec3<f32>, hit: vec3<f32>) -> vec2<f32> {
    let start = hit - origin;
    return vec2(dot(start, u.to_u.xyz), dot(start, u.to_v.xyz));
}

// A pixel of the picture, transparent outside it.
fn pixel(x: i32, y: i32) -> vec4<f32> {
    if x < 0 || y < 0 || x >= i32(u.size.z) || y >= i32(u.size.w) {
        return vec4(0.0);
    }
    return textureLoad(src, vec2(x, y), 0);
}

// `Image::sample_bilinear`.
fn sample(x: f32, y: f32) -> vec4<f32> {
    let f = vec2(x, y) - 0.5;
    let whole = floor(f);
    let t = f - whole;
    let x0 = i32(whole.x);
    let y0 = i32(whole.y);
    let a = pixel(x0, y0);
    let b = pixel(x0 + 1, y0);
    let c = pixel(x0, y0 + 1);
    let d = pixel(x0 + 1, y0 + 1);
    let top = a + (b - a) * t.x;
    let bottom = c + (d - c) * t.x;
    return top + (bottom - top) * t.y;
}

// `Held::over` on a caster's table.
fn caster_holds(index: u32, summed: bool, area: vec4<f32>) -> vec2<u32> {
    let w = casters[index].size.x;
    let h = casters[index].size.y;
    let x0 = min(u32(max(area.x, 0.0)), w);
    let x1 = min(u32(max(area.z, 0.0)), w);
    let y0 = min(u32(max(area.y, 0.0)), h);
    let y1 = min(u32(max(area.w, 0.0)), h);
    let whole = (x1 - min(x0, x1)) * (y1 - min(y0, y1)) * 255u;
    if !summed {
        return vec2(whole, whole);
    }
    let stride = w + 1u;
    let start = casters[index].what.w;
    let held = tables[start + y1 * stride + x1]
        + tables[start + y0 * stride + x0]
        - tables[start + y1 * stride + x0]
        - tables[start + y0 * stride + x1];
    return vec2(held, whole);
}

// Whether a caster goes by its table: not one found to be opaque all over.
fn is_summed(index: u32) -> bool {
    if casters[index].size.z == 0u {
        return false;
    }
    let w = casters[index].size.x;
    let h = casters[index].size.y;
    let last = casters[index].what.w + h * (w + 1u) + w;
    return tables[last] != w * h * 255u;
}

fn caster_alpha(index: u32, x: i32, y: i32) -> f32 {
    let w = i32(casters[index].size.x);
    let h = i32(casters[index].size.y);
    if x < 0 || y < 0 || x >= w || y >= h {
        return 0.0;
    }
    if casters[index].what.y != 0u {
        return 1.0;
    }
    return floats[casters[index].what.z + u32(y) * u32(w) + u32(x)];
}

// `Caster::lets_through`.
fn lets_through(index: u32, start: vec3<f32>, way: vec3<f32>, reach: f32) -> f32 {
    let normal = casters[index].normal.xyz;
    let toward = dot(way, normal);
    if abs(toward) < 1e-9 {
        return 1.0;
    }
    let to_it = casters[index].origin.xyz - start;
    let along = dot(to_it, normal) / toward;
    if along < 1.0 || along > reach - 1.0 {
        return 1.0;
    }
    let hit = way * along - to_it;
    let at = vec2(dot(hit, casters[index].to_u.xyz), dot(hit, casters[index].to_v.xyz));
    let crop = casters[index].crop;
    if at.x < crop.x || at.y < crop.y || at.x > crop.z || at.y > crop.w {
        return 1.0;
    }
    let f = at - 0.5;
    let whole = floor(f);
    let t = f - whole;
    let x0 = i32(whole.x);
    let y0 = i32(whole.y);
    let a = caster_alpha(index, x0, y0);
    let b = caster_alpha(index, x0 + 1, y0);
    let c = caster_alpha(index, x0, y0 + 1);
    let d = caster_alpha(index, x0 + 1, y0 + 1);
    let top = a + (b - a) * t.x;
    let bottom = c + (d - c) * t.x;
    return 1.0 - (top + (bottom - top) * t.y) * casters[index].origin.w;
}

// `Scene::gets_through`.
fn gets_through(at: vec3<f32>, way: vec3<f32>, reach: f32, shadow: f32) -> f32 {
    if shadow <= 0.0 {
        return 1.0;
    }
    var through = 1.0;
    for (var index = 0u; index < scene.counts.y; index++) {
        if casters[index].what.x != u.what.x {
            through *= lets_through(index, at, way, reach);
        }
    }
    return 1.0 - shadow * (1.0 - through);
}

// `Caster::takes`: x says what it does to the rays, and y how much of each it lets through if all.
fn takes(index: u32, summed: bool, start: vec3<f32>, around: array<vec4<f32>, 4>) -> vec2<f32> {
    let normal = casters[index].normal.xyz;
    let to_it = casters[index].origin.xyz - start;
    let off_plane = dot(to_it, normal);
    var left = FAR;
    var top = FAR;
    var right = -FAR;
    var bottom = -FAR;
    var crossing = 0;
    for (var ray = 0; ray < 4; ray++) {
        let way = around[ray].xyz;
        let toward = dot(way, normal);
        if toward == 0.0 {
            continue;
        }
        let along = off_plane / toward;
        if along > 0.0 && along <= around[ray].w {
            crossing += 1;
            let hit = way * along - to_it;
            let u_at = dot(hit, casters[index].to_u.xyz);
            let v_at = dot(hit, casters[index].to_v.xyz);
            left = min(left, u_at);
            right = max(right, u_at);
            top = min(top, v_at);
            bottom = max(bottom, v_at);
        }
    }
    if crossing == 0 {
        return vec2(TAKES_NOTHING, 0.0);
    }
    if crossing < 4 {
        return vec2(TAKES_SOME, 0.0);
    }
    let crop = casters[index].crop;
    let x0 = max(left, crop.x);
    let x1 = min(right, crop.z);
    let y0 = max(top, crop.y);
    let y1 = min(bottom, crop.w);
    if x0 > x1 || y0 > y1 {
        return vec2(TAKES_NOTHING, 0.0);
    }
    var hides = abs(off_plane) > 1.0
        && left >= crop.x + 1.0
        && right <= crop.z - 1.0
        && top >= crop.y + 1.0
        && bottom <= crop.w - 1.0;
    if summed {
        let area = vec4(floor(x0) - 1.0, floor(y0) - 1.0, ceil(x1) + 1.0, ceil(y1) + 1.0);
        let held = caster_holds(index, true, area);
        if held.x == 0u {
            return vec2(TAKES_NOTHING, 0.0);
        }
        hides = hides && held.x == held.y;
    }
    if hides {
        return vec2(TAKES_ALL, 1.0 - casters[index].origin.w);
    }
    return vec2(TAKES_SOME, 0.0);
}

// Half of how far a ray's share of a light goes along an axis, laid on a plane it leans to.
fn half_along(axis: vec4<f32>, way: vec3<f32>, toward: f32, spread: f32) -> f32 {
    let lean = dot(way, axis.xyz) / toward;
    return max(spread * axis.w * sqrt(1.0 + lean * lean), 0.5);
}

// `Caster::lets_some_through`, of a caster read once for all its rays: a read of a buffer is checked.
fn lets_some_through(c: Caster, summed: bool, index: u32, start: vec3<f32>, ray: vec4<f32>, grows: f32) -> f32 {
    let way = ray.xyz;
    let toward = dot(way, c.normal.xyz);
    if abs(toward) < 1e-9 {
        return 1.0;
    }
    let to_it = c.origin.xyz - start;
    let along = dot(to_it, c.normal.xyz) / toward;
    if along < 1.0 || along > ray.w - 1.0 {
        return 1.0;
    }
    let hit = way * along - to_it;
    let u_at = dot(hit, c.to_u.xyz);
    let v_at = dot(hit, c.to_v.xyz);
    let half_u = half_along(c.across, way, toward, grows * along);
    let half_v = half_along(c.down, way, toward, grows * along);
    let left = max(u_at - half_u, c.crop.x);
    let right = min(u_at + half_u, c.crop.z);
    let top = max(v_at - half_v, c.crop.y);
    let bottom = min(v_at + half_v, c.crop.w);
    if left >= right || top >= bottom {
        return 1.0;
    }
    var covered: f32;
    if !summed {
        covered = (right - left) * (bottom - top) / (4.0 * half_u * half_v);
    } else {
        // On whole pixels of a picture, those of all of it as well as those left within the crop.
        let width = max(floor(u_at + half_u + 0.5) - floor(u_at - half_u + 0.5), 1.0);
        let height = max(floor(v_at + half_v + 0.5) - floor(v_at - half_v + 0.5), 1.0);
        let area = vec4(floor(left + 0.5), floor(top + 0.5), floor(right + 0.5), floor(bottom + 0.5));
        covered = f32(caster_holds(index, true, area).x) / 255.0 / (width * height);
    }
    return 1.0 - min(covered, 1.0) * c.origin.w;
}

// `square_to`: the two ways square to `way`, each a unit long.
fn side_of(way: vec3<f32>) -> mat2x3<f32> {
    let x = -way.x;
    let y = -way.y;
    let z = -way.z;
    if z < -0.999999 {
        return mat2x3(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0));
    }
    let by = -1.0 / (1.0 + z);
    let both = x * y * by;
    return mat2x3(vec3(1.0 + x * x * by, both, -x), vec3(both, 1.0 + y * y * by, -y));
}

// A ray to a light from what lies toward it: a point's way ends at it, a directional's goes on for ever.
fn ray_to(point: bool, toward: vec3<f32>, off: vec3<f32>) -> vec4<f32> {
    let to = toward + off;
    if point {
        let reach = sqrt(dot(to, to));
        return vec4(to / max(reach, 1e-12), reach);
    }
    return vec4(unit(to), FAR);
}

// What goes to a light with a size, for `gets_through_across`: where the rays around it go from, how
// far they spread, and how each is made.
struct Across {
    at: vec3<f32>,
    side: vec3<f32>,
    other: vec3<f32>,
    toward: vec3<f32>,
    radius: f32,
    grows: f32,
    point: bool,
};

fn nth_ray(a: Across, ray: u32) -> vec4<f32> {
    let on = scene.spiral[ray];
    return ray_to(a.point, a.toward, a.side * (on.x * a.radius) + a.other * (on.y * a.radius));
}

// The mean of what one caster lets through of each ray, which is all there is to know where only one
// is in the way.
fn mean_through(a: Across, index: u32, summed: bool) -> f32 {
    let caster = casters[index];
    var sum = 0.0;
    for (var ray = 0u; ray < RAYS; ray++) {
        sum += lets_some_through(caster, summed, index, a.at, nth_ray(a, ray), a.grows);
    }
    return sum / f32(RAYS);
}

// The mean of what gets past several casters along each ray. The room for this many numbers costs
// enough at each pixel that it is made only where it is needed.
fn mean_through_all(a: Across, around: array<vec4<f32>, 4>) -> f32 {
    var through: array<f32, 32>;
    for (var ray = 0u; ray < RAYS; ray++) {
        through[ray] = 1.0;
    }
    for (var index = 0u; index < scene.counts.y; index++) {
        if casters[index].what.x == u.what.x {
            continue;
        }
        let summed = is_summed(index);
        if takes(index, summed, a.at, around).x != TAKES_SOME {
            continue;
        }
        let caster = casters[index];
        for (var ray = 0u; ray < RAYS; ray++) {
            through[ray] *= lets_some_through(caster, summed, index, a.at, nth_ray(a, ray), a.grows);
        }
    }
    var sum = 0.0;
    for (var ray = 0u; ray < RAYS; ray++) {
        sum += through[ray];
    }
    return sum / f32(RAYS);
}

// `Scene::gets_through_across`.
fn gets_through_across(
    at: vec3<f32>,
    way: vec3<f32>,
    radius: f32,
    spread: f32,
    shadow: f32,
    point: bool,
    toward: vec3<f32>,
) -> f32 {
    if shadow <= 0.0 {
        return 1.0;
    }
    let sides = side_of(way);
    let out = radius * (1.0 + scene.near.y / 2.0);
    let around = array<vec4<f32>, 4>(
        ray_to(point, toward, sides[0] * -out + sides[1] * -out),
        ray_to(point, toward, sides[0] * out + sides[1] * -out),
        ray_to(point, toward, sides[0] * out + sides[1] * out),
        ray_to(point, toward, sides[0] * -out + sides[1] * out),
    );
    var all_alike = 1.0;
    var some = 0u;
    var one = 0u;
    var one_summed = false;
    for (var index = 0u; index < scene.counts.y; index++) {
        if casters[index].what.x == u.what.x {
            continue;
        }
        let summed = is_summed(index);
        let what = takes(index, summed, at, around);
        if what.x == TAKES_ALL {
            all_alike *= what.y;
            if all_alike <= 0.0 {
                return 1.0 - shadow;
            }
        } else if what.x == TAKES_SOME {
            some += 1u;
            one = index;
            one_summed = summed;
        }
    }
    var through = 1.0;
    if some > 0u {
        let a = Across(at, sides[0], sides[1], toward, radius, spread * scene.near.y / 2.0, point);
        if some == 1u {
            through = mean_through(a, one, one_summed);
        } else {
            through = mean_through_all(a, around);
        }
    }
    return 1.0 - shadow * (1.0 - all_alike * through);
}

// `Irradiance::toward`.
fn table_at(start: u32, column: i32, row: i32) -> vec3<f32> {
    let c = ((column % ACROSS) + ACROSS) % ACROSS;
    let r = min(max(row, 0), DOWN - 1);
    let at = start + u32(r * ACROSS + c) * 4u;
    return vec3(floats[at], floats[at + 1u], floats[at + 2u]);
}

fn table_toward(start: u32, normal: vec3<f32>) -> vec3<f32> {
    // Straight up or down, a kernel's arc tangent of nothing may be anything.
    var around = 0.5;
    if normal.x != 0.0 || normal.z != 0.0 {
        around = atan2(normal.x, normal.z) / TAU + 0.5;
    }
    let from_up = acos(clamp(-normal.y, -1.0, 1.0)) / PI;
    let x = around * f32(ACROSS) - 0.5;
    let y = from_up * f32(DOWN) - 0.5;
    let left = floor(x);
    let up = floor(y);
    let right_part = x - left;
    let lower_part = y - up;
    let column = i32(left);
    let row = i32(up);
    let top = mix(table_at(start, column, row), table_at(start, column + 1, row), right_part);
    let bottom = mix(table_at(start, column, row + 1), table_at(start, column + 1, row + 1), right_part);
    return mix(top, bottom, lower_part);
}

// `Scene::lit_by`, with what comes of no lights at all left to whoever asks.
fn lit(at: vec3<f32>, normal: vec3<f32>, outside: bool) -> vec3<f32> {
    var sum = vec3(0.0);
    for (var index = 0u; index < scene.counts.x; index++) {
        let kind = scene.lights[index].what.x;
        let color = scene.lights[index].color.xyz;
        if kind == AMBIENT {
            sum += color;
        } else if kind == POINT {
            let place = scene.lights[index].place.xyz;
            let shadow = scene.lights[index].place.w;
            let radius = scene.lights[index].color.w;
            let to = place - at;
            let way = unit(to);
            let reach = sqrt(dot(to, to));
            var through: f32;
            if radius > 0.0 {
                through = gets_through_across(at, way, radius, radius / max(reach, 1e-12), shadow, true, to);
            } else {
                through = gets_through(at, way, reach, shadow);
            }
            let cosine = dot(way, normal);
            let facing = select(abs(cosine), max(cosine, 0.0), outside);
            sum += color * facing * through;
        } else if kind == DIRECTIONAL {
            let back = scene.lights[index].place.xyz;
            let shadow = scene.lights[index].place.w;
            let spread = scene.lights[index].color.w;
            var through: f32;
            if spread > 0.0 {
                through = gets_through_across(at, back, spread, spread, shadow, false, back);
            } else {
                through = gets_through(at, back, FAR, shadow);
            }
            let cosine = dot(back, normal);
            let facing = select(abs(cosine), max(cosine, 0.0), outside);
            sum += color * facing * through;
        } else if scene.lights[index].what.z != 0u {
            // A flat clip faces whichever way it is seen from.
            let seen = outside || dot(-at, normal) >= 0.0;
            let facing = select(-normal, normal, seen);
            sum += table_toward(scene.lights[index].what.y, facing) * color;
        }
    }
    return sum;
}

fn put(id: vec2<u32>, color: vec4<f32>, depth: f32) {
    let p = vec2<i32>(id);
    textureStore(dst, p, color);
    if u.what.y != 0u {
        textureStore(depth_out, p + vec2(0, i32(u.what.z)), vec4(depth, 0.0, 0.0, 0.0));
    }
}

// A clip with no depth (`three_d::projected`).
@compute @workgroup_size(16, 16)
fn flat(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.size.x || id.y >= u.size.y {
        return;
    }
    let met = meets(u.origin.xyz, vec2<f32>(id.xy) + 0.5);
    if !met.ok {
        put(id.xy, vec4(0.0), NO_DEPTH);
        return;
    }
    let at = at_plane(u.origin.xyz, met.hit);
    if at.x < u.crop.x || at.y < u.crop.y || at.x > u.crop.z || at.y > u.crop.w {
        put(id.xy, vec4(0.0), met.ahead);
        return;
    }
    var color = sample(at.x * u.origin.w, at.y * u.origin.w);
    if color.a <= 0.0 {
        put(id.xy, vec4(0.0), met.ahead);
        return;
    }
    if scene.counts.x > 0u {
        let light = lit(met.hit, u.normal.xyz, false);
        color = vec4(color.rgb * light, color.a);
    }
    put(id.xy, color, met.ahead);
}

// `Held::over` on this clip's own picture.
fn own_holds(area: vec4<f32>) -> u32 {
    let w = u.size.z;
    let h = u.size.w;
    let x0 = min(u32(max(area.x, 0.0)), w);
    let x1 = min(u32(max(area.z, 0.0)), w);
    let y0 = min(u32(max(area.y, 0.0)), h);
    let y1 = min(u32(max(area.w, 0.0)), h);
    if u.what.w == 0u {
        return (x1 - min(x0, x1)) * (y1 - min(y0, y1)) * 255u;
    }
    let stride = w + 1u;
    return own_table[y1 * stride + x1]
        + own_table[y0 * stride + x0]
        - own_table[y1 * stride + x0]
        - own_table[y0 * stride + x1];
}

// `Solid::side_facing`, with nothing to say as all zeros.
fn side_facing(at: vec2<f32>) -> vec3<f32> {
    let x = at.x * u.origin.w;
    let y = at.y * u.origin.w;
    let across = sample(x + SIDE_REACH, y).a - sample(x - SIDE_REACH, y).a;
    let down = sample(x, y + SIDE_REACH).a - sample(x, y - SIDE_REACH).a;
    if across == 0.0 && down == 0.0 {
        return vec3(0.0);
    }
    return unit(u.across.xyz * -across + u.down.xyz * -down);
}

// `bevel::distance_at`.
fn distance_at(x: f32, y: f32) -> f32 {
    let size = vec2<i32>(textureDimensions(distance));
    let f = clamp(vec2(x, y) - 0.5, vec2(0.0), vec2<f32>(size - 1));
    let p0 = vec2<i32>(floor(f));
    let p1 = min(p0 + 1, size - 1);
    let t = f - vec2<f32>(p0);
    let a = textureLoad(distance, p0, 0).r;
    let b = textureLoad(distance, vec2(p1.x, p0.y), 0).r;
    let c = textureLoad(distance, vec2(p0.x, p1.y), 0).r;
    let d = textureLoad(distance, p1, 0).r;
    let top = a + (b - a) * t.x;
    let bottom = c + (d - c) * t.x;
    return top + (bottom - top) * t.y;
}

// `Solid::bevel_facing`.
fn bevel_facing(at: vec2<f32>, from_front: bool) -> vec3<f32> {
    let x = at.x * u.origin.w;
    let y = at.y * u.origin.w;
    let across = distance_at(x + SIDE_REACH, y) - distance_at(x - SIDE_REACH, y);
    let down = distance_at(x, y + SIDE_REACH) - distance_at(x, y - SIDE_REACH);
    let out = unit(u.across.xyz * -across + u.down.xyz * -down);
    return unit(out + select(u.normal.xyz, -u.normal.xyz, from_front));
}

// A clip with a depth (`Solid::draw_row`).
@compute @workgroup_size(16, 16)
fn solid(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.size.x || id.y >= u.size.y {
        return;
    }
    let place = vec2<f32>(id.xy) + 0.5;
    let front = meets(u.origin.xyz, place);
    let back = meets(u.back.xyz, place);
    if !front.ok && !back.ok {
        put(id.xy, vec4(0.0), NO_DEPTH);
        return;
    }
    // Behind the camera or edge on, what is left of it is the plane that can be seen.
    var near = front;
    var far = back;
    var near_origin = u.origin.xyz;
    var far_origin = u.back.xyz;
    var near_front = true;
    if !back.ok {
        far = front;
        far_origin = u.origin.xyz;
    } else if !front.ok || front.ahead > back.ahead {
        near = back;
        near_origin = u.back.xyz;
        near_front = false;
        if front.ok {
            far = front;
            far_origin = u.origin.xyz;
        } else {
            far = back;
            far_origin = u.back.xyz;
        }
    }
    let start = at_plane(near_origin, near.hit);
    let to = at_plane(far_origin, far.hit);
    let left = max(min(start.x, to.x), u.crop.x);
    let right = min(max(start.x, to.x), u.crop.z);
    let top = max(min(start.y, to.y), u.crop.y);
    let bottom = min(max(start.y, to.y), u.crop.w);
    if left - 1.0 > right || top - 1.0 > bottom {
        put(id.xy, vec4(0.0), near.ahead);
        return;
    }
    let by = u.origin.w;
    let area = vec4(
        floor((left - 1.0) * by),
        floor((top - 1.0) * by),
        ceil((right + 1.0) * by) + 1.0,
        ceil((bottom + 1.0) * by) + 1.0,
    );
    if own_holds(area) == 0u {
        put(id.xy, vec4(0.0), near.ahead);
        return;
    }
    // A bevel changes as the ray goes in, however little it goes across.
    let beveled = u.bevel.x > 0.0;
    var across = max(abs(to.x - start.x), abs(to.y - start.y)) * by;
    if beveled {
        across = max(across, u.bevel.y);
    }
    let steps = clamp(u32(ceil(across * 2.0)), 1u, MOST_STEPS);
    // Out of the side it comes in by.
    let flat_way = select(u.normal.xyz, -u.normal.xyz, near_front);
    var sum = vec3(0.0);
    var covered = 0.0;
    var first = -1.0;
    for (var step = 0u; step <= steps; step++) {
        let along = f32(step) / f32(steps);
        let at = start + (to - start) * along;
        if at.x < u.crop.x || at.y < u.crop.y || at.x > u.crop.z || at.y > u.crop.w {
            continue;
        }
        let color = sample(at.x * by, at.y * by);
        // What a bevel leaves of it this far in, and whether the ray is on the bevel.
        var reached = color.a;
        var on_bevel = false;
        var from_front = false;
        if beveled {
            let in_depth = select(1.0 - along, along, near_front) * u.bevel.y;
            let inset = max(max(u.bevel.x - in_depth, u.bevel.x - (u.bevel.y - in_depth)), 0.0);
            let inside = distance_at(at.x * by, at.y * by);
            reached = min(reached, clamp(inside - inset + 0.5, 0.0, 1.0));
            on_bevel = inset > 0.0 && inside - inset < 1.5;
            from_front = in_depth < u.bevel.y / 2.0;
        }
        if reached <= covered {
            continue;
        }
        let more = reached - covered;
        let hit = near.hit + (far.hit - near.hit) * along;
        var facing = flat_way;
        if on_bevel {
            facing = bevel_facing(at, from_front);
        } else if step != 0u {
            let side = side_facing(at);
            if any(side != vec3(0.0)) {
                facing = side;
            }
        }
        var light = vec3(1.0);
        if scene.counts.x > 0u {
            light = lit(hit, facing, true);
        }
        sum += color.rgb / color.a * light * more;
        covered = reached;
        if first < 0.0 && covered >= 0.5 {
            first = near.ahead + (far.ahead - near.ahead) * along;
        }
        if covered >= 0.995 {
            break;
        }
    }
    if covered <= 0.0 {
        put(id.xy, vec4(0.0), near.ahead);
        return;
    }
    put(id.xy, vec4(sum, covered), select(near.ahead, first, first >= 0.0));
}
