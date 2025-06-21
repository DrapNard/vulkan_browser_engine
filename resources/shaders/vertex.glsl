#version 450

layout(location = 0) in vec3 position;
layout(location = 1) in vec2 tex_coord;
layout(location = 2) in vec4 color;

layout(location = 0) out vec2 frag_tex_coord;
layout(location = 1) out vec4 frag_color;

layout(set = 0, binding = 0) uniform UniformBufferObject {
    mat4 model;
    mat4 view;
    mat4 proj;
} ubo;

void main() {
    gl_Position = ubo.proj * ubo.view * ubo.model * vec4(position, 1.0);
    frag_tex_coord = tex_coord;
    frag_color = color;
}