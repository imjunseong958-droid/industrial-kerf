
// Clips in depth that cross, over `src` (`three_d::composite_by_depth`). Appended to `picture.wgsl`
// for its blend modes: `aux` holds the clips' pictures one below another, and `depths` how far along
// the camera's way each is at each pixel.

struct Layer {
    opacity: f32,
    blend: u32,
    unused: vec2<u32>,
};

@group(0) @binding(4) var depths: texture_2d<f32>;
@group(0) @binding(5) var<storage, read> layers: array<Layer>;

const SAME_DEPTH: f32 = 0.01;
const MOST_LAYERS: u32 = 15u;
const NOT_THERE: f32 = 1e29;

fn depth_of(layer: u32, x: i32, y: i32, h: i32) -> f32 {
    return textureLoad(depths, vec2(x, y + i32(layer) * h), 0).r;
}

// `cover`: how much of a pixel the nearer of two planes covers in front of the farther.
fn cover(far: u32, near: u32, x: i32, y: i32, size: vec2<i32>) -> f32 {
    let here = depth_of(far, x, y, size.y) - depth_of(near, x, y, size.y);
    let w = size.x;
    let h = size.y;
    var across = here;
    if x + 1 < w {
        across = depth_of(far, x + 1, y, h) - depth_of(near, x + 1, y, h);
    } else if x > 0 {
        across = depth_of(far, x - 1, y, h) - depth_of(near, x - 1, y, h);
    }
    var down = here;
    if y + 1 < h {
        down = depth_of(far, x, y + 1, h) - depth_of(near, x, y + 1, h);
    } else if y > 0 {
        down = depth_of(far, x, y - 1, h) - depth_of(near, x, y - 1, h);
    }
    var change_across = abs(across - here);
    if change_across > NOT_THERE {
        change_across = 0.0;
    }
    var change_down = abs(down - here);
    if change_down > NOT_THERE {
        change_down = 0.0;
    }
    let change = change_across + change_down;
    if change < 1e-3 {
        return 1.0;
    }
    return min(0.5 + max(here, 0.0) / change, 1.0);
}

@compute @workgroup_size(16, 16)
fn by_depth(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let size = vec2<i32>(textureDimensions(dst));
    let x = i32(id.x);
    let y = i32(id.y);
    let count = min(arrayLength(&layers), MOST_LAYERS);
    var here: array<u32, 15>;
    var held = 0u;
    for (var index = 0u; index < count; index++) {
        if textureLoad(aux, vec2(x, y + i32(index) * size.y), 0).a <= 0.0 {
            continue;
        }
        // Past all that is farther, and at the same depth past those too, so that clips that do not
        // differ stack as their tracks do.
        let depth = depth_of(index, x, y, size.y);
        var place = held;
        for (var other = 0u; other < held; other++) {
            if depth_of(here[other], x, y, size.y) < depth - SAME_DEPTH {
                place = other;
                break;
            }
        }
        for (var moved = held; moved > place; moved--) {
            here[moved] = here[moved - 1u];
        }
        here[place] = index;
        held += 1u;
    }
    var out = textureLoad(src, vec2(x, y), 0);
    for (var place = 0u; place < held; place++) {
        let index = here[place];
        var shared_by = 1.0;
        if place > 0u {
            shared_by = cover(here[place - 1u], index, x, y, size);
        }
        let color = textureLoad(aux, vec2(x, y + i32(index) * size.y), 0);
        out = mix(out, color * (layers[index].opacity * shared_by), id.xy, layers[index].blend);
    }
    textureStore(dst, vec2(x, y), out);
}
