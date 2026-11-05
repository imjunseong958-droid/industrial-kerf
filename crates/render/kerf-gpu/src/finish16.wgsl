// A finished picture over black, as 16-bit sRGB-encoded code values, for a master that keeps more
// than 8 bits (`Image::to_rgba16_over_black`).

@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16uint, write>;

@compute @workgroup_size(16, 16)
fn finish(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst);
    if id.x >= size.x || id.y >= size.y { return; }
    let p = vec2<i32>(id.xy);
    let c = clamp(textureLoad(src, p, 0).rgb, vec3(0.0), vec3(1.0));
    let hi = 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055;
    let encoded = select(hi, c * 12.92, c <= vec3(0.0031308));
    textureStore(dst, p, vec4<u32>(vec3<u32>(encoded * 65535.0 + 0.5), 65535u));
}
