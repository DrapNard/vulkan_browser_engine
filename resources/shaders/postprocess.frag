#version 450

layout(location = 0) in vec2 frag_tex_coord;
layout(location = 0) out vec4 out_color;

layout(set = 0, binding = 0) uniform sampler2D scene_texture;

layout(push_constant) uniform PostProcessParams {
    float gamma;
    float exposure;
    float contrast;
    float brightness;
} params;

vec3 tonemap(vec3 color) {
    color *= params.exposure;
    color = color / (color + vec3(1.0));
    return pow(color, vec3(1.0 / params.gamma));
}

void main() {
    vec3 color = texture(scene_texture, frag_tex_coord).rgb;
    
    color = color * params.contrast + params.brightness;
    color = tonemap(color);
    
    out_color = vec4(color, 1.0);
}