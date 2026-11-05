// A picture that is already a signal, from `output.wgsl`, as 16-bit code values, for an HDR master
// (`Image::to_signal16_over_black`).

@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16uint, write>;

@compute @workgroup_size(16, 16)
fn finish(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst);
    if id.x >= size.x || id.y >= size.y { return; }
    let p = vec2<i32>(id.xy);
    let c = clamp(textureLoad(src, p, 0).rgb, vec3(0.0), vec3(1.0));
    textureStore(dst, p, vec4<u32>(vec3<u32>(c * 65535.0 + 0.5), 65535u));
}
