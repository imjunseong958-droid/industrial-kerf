#version 440

// The frame as it is: opaque, sampled between its pixels.
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 fragColor;

layout(binding = 1) uniform sampler2D frame;

void main() {
    fragColor = vec4(texture(frame, v_uv).rgb, 1.0);
}
