// Guide passes on the GPU: ids, depth, normals, motion, hard edges, and motion finalization.
// Matches `kerf-compose/src/draw/{ids,output,three_d}.rs`, the CPU reference.

struct U {
    a: vec4<u32>,
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    p3: vec4<f32>,
    p4: vec4<f32>,
    p5: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var aux: texture_2d<f32>;

fn inside(id: vec3<u32>) -> bool {
    let size = textureDimensions(dst);
    return id.x < size.x && id.y < size.y;
}

fn srgb_to_linear_scalar(v: f32) -> f32 {
    if v <= 0.04045 {
        return v / 12.92;
    }
    return pow((max(v, 0.0) + 0.055) / 1.055, 2.4);
}

fn nearest_color(c: vec4<f32>) -> vec4<f32> {
    if u.a.x <= 1u {
        return u.p0;
    }
    let rgb = c.rgb / c.a;
    let d0 = rgb - u.p0.rgb;
    let d1 = rgb - u.p1.rgb;
    if dot(d1, d1) < dot(d0, d0) {
        return u.p1;
    }
    return u.p0;
}

// `ids::flat`, `three_d::shade_still`, `three_d::shade_by_facing`: replace RGB with `p0.rgb * a`.
@compute @workgroup_size(16, 16)
fn shade_flat(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    textureStore(dst, p, vec4(u.p0.rgb * c.a, c.a));
}

// `three_d::shade_by_depth`: p0 = (base, du, dv, unit).
@compute @workgroup_size(16, 16)
fn shade_depth(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let u_coord = f32(id.x) + 0.5;
    let v_coord = f32(id.y) + 0.5;
    let ahead = u.p0.x + u.p0.y * u_coord + u.p0.z * v_coord;
    let near = u.p0.w / (u.p0.w + max(ahead, 0.0));
    let grey = srgb_to_linear_scalar(near) * c.a;
    textureStore(dst, p, vec4(grey, grey, grey, c.a));
}

// `three_d::shade_by_motion_flat`: p0 = (dx_u, dx_v, dx_0, most), p1 = (dy_u, dy_v, dy_0, 0).
@compute @workgroup_size(16, 16)
fn shade_motion_flat(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let u_coord = f32(id.x) + 0.5;
    let v_coord = f32(id.y) + 0.5;
    let dx = u.p0.x * u_coord + u.p0.y * v_coord + u.p0.z;
    let dy = u.p1.x * u_coord + u.p1.y * v_coord + u.p1.z;
    let most = u.p0.w;
    let r = srgb_to_linear_scalar(0.5 + 0.5 * clamp(dx / most, -1.0, 1.0));
    let g = srgb_to_linear_scalar(0.5 + 0.5 * clamp(dy / most, -1.0, 1.0));
    textureStore(dst, p, vec4(r * c.a, g * c.a, c.a, c.a));
}

// `three_d::shade_by_motion`:
// p0 = (ahead_now_0, du, dv, most), p1 = (rx_now_0, du, dv, center_dx),
// p2 = (ry_now_0, du, dv, center_dy), p3 = (ahead_next_0, du, dv, 0),
// p4 = (rx_next_0, du, dv, 0), p5 = (ry_next_0, du, dv, 0).
@compute @workgroup_size(16, 16)
fn shade_motion_3d(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    let u_coord = f32(id.x) + 0.5;
    let v_coord = f32(id.y) + 0.5;
    let ahead_now = u.p0.x + u.p0.y * u_coord + u.p0.z * v_coord;
    let ahead_next = u.p3.x + u.p3.y * u_coord + u.p3.z * v_coord;
    var dx = 0.0;
    var dy = 0.0;
    if ahead_now > 1e-9 && ahead_next > 1e-9 {
        let rx_now = u.p1.x + u.p1.y * u_coord + u.p1.z * v_coord;
        let ry_now = u.p2.x + u.p2.y * u_coord + u.p2.z * v_coord;
        let rx_next = u.p4.x + u.p4.y * u_coord + u.p4.z * v_coord;
        let ry_next = u.p5.x + u.p5.y * u_coord + u.p5.z * v_coord;
        dx = u.p1.w + rx_next / ahead_next - rx_now / ahead_now;
        dy = u.p2.w + ry_next / ahead_next - ry_now / ahead_now;
    }
    let most = u.p0.w;
    let r = srgb_to_linear_scalar(0.5 + 0.5 * clamp(dx / most, -1.0, 1.0));
    let g = srgb_to_linear_scalar(0.5 + 0.5 * clamp(dy / most, -1.0, 1.0));
    textureStore(dst, p, vec4(r * c.a, g * c.a, c.a, c.a));
}

// `ids::harden`: p0/p1 = linear candidate colors, a.x = count (1 or 2), p2.x = opacity.
@compute @workgroup_size(16, 16)
fn harden(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if u.a.x > 0u && c.a * u.p2.x >= 0.5 {
        textureStore(dst, p, nearest_color(c));
    } else {
        textureStore(dst, p, vec4(0.0));
    }
}

// `ids::lay_hard`: `aux` laid hard over `src`, writing `dst`.
@compute @workgroup_size(16, 16)
fn lay_hard(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let layer = textureLoad(aux, p, 0);
    if u.a.x > 0u && layer.a >= 0.5 {
        textureStore(dst, p, nearest_color(layer));
    } else {
        textureStore(dst, p, textureLoad(src, p, 0));
    }
}

// `output::whole_or_nothing`: finalize a motion pass picture before encoding.
@compute @workgroup_size(16, 16)
fn whole_or_nothing(@builtin(global_invocation_id) id: vec3<u32>) {
    if !inside(id) { return; }
    let p = vec2<i32>(id.xy);
    let c = textureLoad(src, p, 0);
    if c.a >= 0.5 {
        textureStore(dst, p, vec4(c.rgb / c.a, 1.0));
    } else {
        textureStore(dst, p, vec4(0.0));
    }
}
