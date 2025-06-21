#version 450

layout(location = 0) in vec2 frag_tex_coord;
layout(location = 1) in vec4 frag_color;

layout(location = 0) out vec4 out_color;

layout(set = 0, binding = 0) uniform sampler2D font_atlas;

void main() {
    float alpha = texture(font_atlas, frag_tex_coord).r;
    out_color = vec4(frag_color.rgb, frag_color.a * alpha);
}