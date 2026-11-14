#version 440

// The part of the frame a viewer's GPU picture shows, over all of it.
layout(location = 0) in vec2 position;
layout(location = 0) out vec2 v_uv;

layout(std140, binding = 0) uniform buf {
    mat4 mvp;
    vec4 source;
};

void main() {
    v_uv = mix(source.xy, source.zw, position);
    gl_Position = mvp * vec4(position, 0.0, 1.0);
}
