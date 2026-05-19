#version 450

layout(location = 0) out vec3 out_color;
layout(location = 1) out vec3 out_normal;

layout(binding = 0) uniform CameraUniform {
    mat4 view_projection;
} camera;

layout(push_constant) uniform ObjectConstants {
    mat4 model;
} object_constants;

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec3 in_color;

void main() {
    gl_Position = camera.view_projection * object_constants.model * vec4(in_position, 1.0);
    out_normal = normalize(transpose(inverse(mat3(object_constants.model))) * in_normal);
    out_color = in_color;
}
