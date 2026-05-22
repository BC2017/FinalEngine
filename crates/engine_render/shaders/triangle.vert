#version 450

layout(location = 0) out vec3 out_color;
layout(location = 1) out vec3 out_normal;
layout(location = 2) out vec2 out_texcoord;
layout(location = 3) out vec4 out_tangent;

layout(binding = 0) uniform CameraUniform {
    mat4 view_projection;
} camera;

layout(push_constant) uniform ObjectConstants {
    mat4 model;
    vec4 base_color_factor;
    vec4 material_factors;
    vec4 normal_factors;
    vec4 debug_factors;
} object_constants;

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec3 in_color;
layout(location = 3) in vec2 in_texcoord;
layout(location = 4) in vec4 in_tangent;

void main() {
    gl_Position = camera.view_projection * object_constants.model * vec4(in_position, 1.0);
    mat3 normal_matrix = transpose(inverse(mat3(object_constants.model)));
    out_normal = normalize(normal_matrix * in_normal);
    out_color = in_color;
    out_texcoord = in_texcoord;
    out_tangent = vec4(normalize(mat3(object_constants.model) * in_tangent.xyz), in_tangent.w);
}
