#version 450

layout(location = 0) in vec3 in_color;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec2 in_texcoord;
layout(location = 0) out vec4 out_color;

layout(binding = 0, set = 1) uniform sampler2D base_color_texture;

layout(push_constant) uniform ObjectConstants {
    mat4 model;
    vec4 base_color_factor;
} object_constants;

void main() {
    vec3 normal = normalize(in_normal);
    vec3 light_direction = normalize(vec3(-0.45, -0.8, -0.35));
    float diffuse = max(dot(normal, -light_direction), 0.0);
    vec4 sampled_color = texture(base_color_texture, in_texcoord);
    vec3 base_color = in_color * object_constants.base_color_factor.rgb * sampled_color.rgb;
    vec3 lit_color = base_color * (0.22 + diffuse * 0.78);
    out_color = vec4(lit_color, 1.0);
}
