#version 450

layout(location = 0) in vec3 in_color;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec2 in_texcoord;
layout(location = 0) out vec4 out_color;

layout(binding = 0, set = 1) uniform sampler2D base_color_texture;
layout(binding = 1, set = 1) uniform sampler2D metallic_roughness_texture;

layout(push_constant) uniform ObjectConstants {
    mat4 model;
    vec4 base_color_factor;
    vec4 material_factors;
} object_constants;

void main() {
    vec3 normal = normalize(in_normal);
    vec3 light_direction = normalize(vec3(-0.45, -0.8, -0.35));
    float diffuse = max(dot(normal, -light_direction), 0.0);
    vec4 sampled_color = texture(base_color_texture, in_texcoord);
    float alpha = object_constants.base_color_factor.a * sampled_color.a;
    float alpha_cutoff = object_constants.material_factors.x;
    float alpha_mode = object_constants.material_factors.y;
    if (alpha_mode == 1.0 && alpha < alpha_cutoff) {
        discard;
    }

    vec4 sampled_metallic_roughness = texture(metallic_roughness_texture, in_texcoord);
    float metallic = object_constants.material_factors.z * sampled_metallic_roughness.b;
    float roughness = object_constants.material_factors.w * sampled_metallic_roughness.g;
    vec3 base_color = in_color * object_constants.base_color_factor.rgb * sampled_color.rgb;
    float ambient = mix(0.28, 0.18, clamp(metallic, 0.0, 1.0));
    float diffuse_weight = mix(0.86, 0.58, clamp(metallic, 0.0, 1.0));
    float roughness_lift = mix(1.08, 0.82, clamp(roughness, 0.0, 1.0));
    vec3 lit_color = base_color * (ambient + diffuse * diffuse_weight) * roughness_lift;
    out_color = vec4(lit_color, alpha_mode == 2.0 ? alpha : 1.0);
}
