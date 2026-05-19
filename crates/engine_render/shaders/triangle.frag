#version 450

layout(location = 0) in vec3 in_color;
layout(location = 1) in vec3 in_normal;
layout(location = 0) out vec4 out_color;

void main() {
    vec3 normal = normalize(in_normal);
    vec3 light_direction = normalize(vec3(-0.45, -0.8, -0.35));
    float diffuse = max(dot(normal, -light_direction), 0.0);
    vec3 lit_color = in_color * (0.22 + diffuse * 0.78);
    out_color = vec4(lit_color, 1.0);
}
