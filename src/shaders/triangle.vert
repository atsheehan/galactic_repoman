#version 450

// Three triangle vertices baked into the shader, indexed by gl_VertexIndex.
// No vertex buffer is bound; the pipeline uses an empty vertex input state.

// Written inline into the command buffer by `vkCmdPushConstants` each frame, so
// there is no buffer to allocate, no descriptor set to bind, and nothing to
// synchronize against the previous frame still being read.
layout(push_constant) uniform Push {
    float angle; // radians
} pc;

layout(location = 0) out vec3 fragColor;

vec2 positions[3] = vec2[](
    vec2(0.0, -0.5),
    vec2(0.5, 0.5),
    vec2(-0.5, 0.5)
);

vec3 colors[3] = vec3[](
    vec3(1.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0),
    vec3(0.0, 0.0, 1.0)
);

void main() {
    // Clip space is square and the window is not, so the triangle stretches as it
    // turns. Deliberate: aspect correction belongs with a real 2D projection, not
    // bolted onto a diagnostic triangle.
    float s = sin(pc.angle);
    float c = cos(pc.angle);
    mat2 rotation = mat2(c, s, -s, c);

    gl_Position = vec4(rotation * positions[gl_VertexIndex], 0.0, 1.0);
    fragColor = colors[gl_VertexIndex];
}
