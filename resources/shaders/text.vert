#version 450

layout(location = 0) in vec2 position;
layout(location = 1) in vec2 tex_coord;
layout(location = 2) in vec4 color;

layout(location = 0) out vec2 frag_tex_coord;
layout(location = 1) out vec4 frag_color;

layout(push_constant) uniform PushConstants {
    mat4 transform;
} pc;

void main() {
    gl_Position = pc.transform * vec4(position, 0.0, 1.0);
    frag_tex_coord = tex_coord;
    frag_color = color;
}
