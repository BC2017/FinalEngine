use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ffi::{CStr, CString};
use std::fs;
use std::io::Cursor;
use std::mem::{offset_of, size_of};
use std::os::raw::c_void;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ash::{Device, Entry, Instance, vk};
use engine_assets::{
    StaticMeshAsset, StaticMeshMaterialAsset, StaticMeshSceneAsset, StaticMeshTextureAsset,
    StaticMeshTextureFilter, StaticMeshTextureMinFilter, StaticMeshTextureSampler,
    StaticMeshTextureWrap, StaticMeshTransform,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, error, info, trace, warn};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";
const WINDOW_TITLE: &str = "FinalEngine Vulkan Renderer";
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const TRIANGLE_VERTEX_SHADER: &[u8] = include_bytes!("../shaders/triangle.vert.spv");
const TRIANGLE_FRAGMENT_SHADER: &[u8] = include_bytes!("../shaders/triangle.frag.spv");
const TRIANGLE_FRONT_FACE: vk::FrontFace = vk::FrontFace::COUNTER_CLOCKWISE;
const DEFAULT_TEST_SCENE_DIRECTORY: &str = "assets/test_scene";
const GLB_EXTENSION: &str = "glb";
const GLTF_EXTENSION: &str = "gltf";
const IMPORTED_SCENE_CAMERA_MARGIN: f32 = 1.35;
const CAMERA_ORBIT_RADIANS_PER_PIXEL: f32 = 0.008;
const CAMERA_ZOOM_LINE_FACTOR: f32 = 0.88;
const CAMERA_MIN_DISTANCE: f32 = 0.05;
const CAMERA_MAX_DISTANCE: f32 = 100_000.0;
const CAMERA_PAN_MIN_VIEWPORT_HEIGHT: f32 = 1.0;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    texcoord: [f32; 2],
}

impl Vertex {
    fn binding_description() -> vk::VertexInputBindingDescription {
        vk::VertexInputBindingDescription {
            binding: 0,
            stride: size_of::<Vertex>() as u32,
            input_rate: vk::VertexInputRate::VERTEX,
        }
    }

    fn attribute_descriptions() -> [vk::VertexInputAttributeDescription; 4] {
        [
            vk::VertexInputAttributeDescription {
                binding: 0,
                location: 0,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: offset_of!(Vertex, position) as u32,
            },
            vk::VertexInputAttributeDescription {
                binding: 0,
                location: 1,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: offset_of!(Vertex, normal) as u32,
            },
            vk::VertexInputAttributeDescription {
                binding: 0,
                location: 2,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: offset_of!(Vertex, color) as u32,
            },
            vk::VertexInputAttributeDescription {
                binding: 0,
                location: 3,
                format: vk::Format::R32G32_SFLOAT,
                offset: offset_of!(Vertex, texcoord) as u32,
            },
        ]
    }
}

const fn vertex(position: [f32; 3], normal: [f32; 3], color: [f32; 3]) -> Vertex {
    Vertex {
        position,
        normal,
        color,
        texcoord: [0.0, 0.0],
    }
}

fn sort_paths_for_default_scene_selection(paths: &mut [PathBuf]) {
    paths.sort_by_cached_key(|path| {
        path.file_name()
            .map(|file_name| file_name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    });
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct ObjectPushConstants {
    model: [[f32; 4]; 4],
    base_color_factor: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderScene {
    pub camera: RenderCamera,
    pub objects: Vec<RenderObject>,
}

impl RenderScene {
    pub fn default_or_test_scene() -> RenderResult<Self> {
        let paths = Self::default_test_scene_paths();
        if let Some(path) = paths.first() {
            info!("Loading default glTF test scene from {}", path.display());
            let scene = StaticMeshSceneAsset::from_gltf_path(path)?;
            return Self::from_static_mesh_scene_asset(scene);
        }

        info!(
            "No .glb or .gltf test scene found in {}; using built-in renderer demo scene",
            Self::default_test_scene_directory().display()
        );
        Ok(Self::demo_scene())
    }

    pub fn default_test_scene_path() -> PathBuf {
        Self::default_test_scene_paths()
            .into_iter()
            .next()
            .unwrap_or_else(|| {
                Self::default_test_scene_directory().join(format!("scene.{GLB_EXTENSION}"))
            })
    }

    pub fn default_test_scene_directory() -> PathBuf {
        PathBuf::from(DEFAULT_TEST_SCENE_DIRECTORY)
    }

    pub fn default_test_scene_paths() -> Vec<PathBuf> {
        Self::discover_test_scene_paths(&Self::default_test_scene_directory())
    }

    fn discover_test_scene_paths(directory: &Path) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(directory) else {
            return Vec::new();
        };

        let mut glb_paths = Vec::new();
        let mut gltf_paths = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };

            if extension.eq_ignore_ascii_case(GLB_EXTENSION) {
                glb_paths.push(path);
            } else if extension.eq_ignore_ascii_case(GLTF_EXTENSION) {
                gltf_paths.push(path);
            }
        }

        sort_paths_for_default_scene_selection(&mut glb_paths);
        sort_paths_for_default_scene_selection(&mut gltf_paths);
        glb_paths.extend(gltf_paths);
        glb_paths
    }

    pub fn from_default_test_scene_path(path: &Path) -> RenderResult<Self> {
        if path.exists() {
            info!("Loading default glTF test scene from {}", path.display());
            let scene = StaticMeshSceneAsset::from_gltf_path(path)?;
            Self::from_static_mesh_scene_asset(scene)
        } else {
            info!(
                "No default glTF test scene found at {}; using built-in renderer demo scene",
                path.display()
            );
            Ok(Self::demo_scene())
        }
    }

    pub fn demo_cube() -> Self {
        Self {
            camera: RenderCamera::default(),
            objects: vec![Self::demo_cube_object()],
        }
    }

    pub fn demo_scene() -> Self {
        Self {
            camera: RenderCamera::default(),
            objects: vec![
                Self::demo_cube_object(),
                Self::demo_loaded_pyramid_object(),
                Self::demo_ground_plane_object(),
            ],
        }
    }

    pub fn from_static_mesh_scene_asset(scene: StaticMeshSceneAsset) -> RenderResult<Self> {
        if scene.meshes.is_empty() {
            return Err(RenderError::Message(format!(
                "static mesh scene {:?} does not contain renderable meshes",
                scene.name
            )));
        }

        let scene_name = scene.name.clone();
        let objects: Vec<_> = scene
            .meshes
            .into_iter()
            .enumerate()
            .map(|(index, mesh)| {
                let model_matrix = (mesh.transform.matrix != StaticMeshTransform::IDENTITY.matrix)
                    .then_some(mesh.transform.matrix);
                RenderObject {
                    name: if mesh.name.is_empty() {
                        format!("{scene_name} mesh {index}")
                    } else {
                        mesh.name.clone()
                    },
                    transform: RenderTransform::default(),
                    model_matrix,
                    animation: None,
                    mesh: RenderMesh::Static(Box::new(mesh)),
                }
            })
            .collect();
        let camera = RenderCamera::framing_objects(&objects).unwrap_or_default();

        Ok(Self { camera, objects })
    }

    fn demo_cube_object() -> RenderObject {
        RenderObject {
            name: "Demo Cube".to_string(),
            transform: RenderTransform {
                translation: [0.0, 0.0, 0.0],
                rotation_euler_degrees: [-18.0, 35.0, 0.0],
                scale: [1.0, 1.0, 1.0],
            },
            model_matrix: None,
            animation: Some(RenderAnimation {
                rotation_degrees_per_second: [12.0, 45.0, 0.0],
            }),
            mesh: RenderMesh::DemoCube,
        }
    }

    fn demo_ground_plane_object() -> RenderObject {
        RenderObject {
            name: "Ground Plane".to_string(),
            transform: RenderTransform {
                translation: [0.0, -0.75, 0.0],
                rotation_euler_degrees: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
            },
            model_matrix: None,
            animation: None,
            mesh: RenderMesh::DemoGroundPlane,
        }
    }

    fn demo_loaded_pyramid_object() -> RenderObject {
        let mesh = StaticMeshAsset::demo_pyramid_from_embedded_gltf()
            .expect("embedded demo glTF pyramid must parse");
        RenderObject {
            name: mesh.name.clone(),
            transform: RenderTransform {
                translation: [1.35, -0.05, 0.0],
                rotation_euler_degrees: [0.0, -25.0, 0.0],
                scale: [0.9, 0.9, 0.9],
            },
            model_matrix: None,
            animation: Some(RenderAnimation {
                rotation_degrees_per_second: [0.0, -28.0, 0.0],
            }),
            mesh: RenderMesh::Static(Box::new(mesh)),
        }
    }

    pub fn primary_object(&self) -> RenderResult<&RenderObject> {
        self.objects.first().ok_or_else(|| {
            RenderError::Message("render scene must contain at least one object".to_string())
        })
    }
}

impl Default for RenderScene {
    fn default() -> Self {
        Self::demo_scene()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RenderCamera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub vertical_fov_degrees: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for RenderCamera {
    fn default() -> Self {
        Self {
            eye: [2.4, 1.7, 3.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            vertical_fov_degrees: 60.0,
            near: 0.1,
            far: 100.0,
        }
    }
}

impl RenderCamera {
    fn framing_objects(objects: &[RenderObject]) -> Option<Self> {
        let bounds = scene_bounds_for_objects(objects)?;
        let center = bounds.center();
        let radius = bounds.radius().max(0.5);
        let vertical_fov_degrees = Self::default().vertical_fov_degrees;
        let half_fov = (vertical_fov_degrees.to_radians() * 0.5).max(0.01);
        let distance = (radius / half_fov.sin()) * IMPORTED_SCENE_CAMERA_MARGIN;
        let default_camera = Self::default();
        let view_direction = normalize3(sub3(default_camera.eye, default_camera.target));
        let eye = add3(center, mul3(view_direction, distance));
        let near = (distance - radius * IMPORTED_SCENE_CAMERA_MARGIN).max(0.01);
        let far = (distance + radius * IMPORTED_SCENE_CAMERA_MARGIN * 2.0).max(near + 1.0);

        Some(Self {
            eye,
            target: center,
            up: default_camera.up,
            vertical_fov_degrees,
            near,
            far,
        })
    }

    fn orbit(&mut self, delta_pixels: [f32; 2]) {
        let target_to_eye = sub3(self.eye, self.target);
        let distance = length3(target_to_eye).max(CAMERA_MIN_DISTANCE);
        let world_up = normalize3(self.up);
        let yaw_radians = -delta_pixels[0] * CAMERA_ORBIT_RADIANS_PER_PIXEL;
        let pitch_radians = -delta_pixels[1] * CAMERA_ORBIT_RADIANS_PER_PIXEL;
        let yawed = rotate_vector_around_axis(target_to_eye, world_up, yaw_radians);
        let right = normalize3(cross3(world_up, normalize3(yawed)));
        if length3(right) <= f32::EPSILON {
            self.eye = add3(self.target, mul3(normalize3(yawed), distance));
            return;
        }

        let pitched = rotate_vector_around_axis(yawed, right, pitch_radians);
        let direction = normalize3(pitched);
        let vertical_alignment = dot3(direction, world_up).abs();
        let final_offset = if vertical_alignment > 0.98 {
            yawed
        } else {
            pitched
        };

        self.eye = add3(self.target, mul3(normalize3(final_offset), distance));
    }

    fn pan(&mut self, delta_pixels: [f32; 2], viewport_size: PhysicalSize<u32>) {
        let distance = length3(sub3(self.eye, self.target)).max(CAMERA_MIN_DISTANCE);
        let viewport_height = (viewport_size.height as f32).max(CAMERA_PAN_MIN_VIEWPORT_HEIGHT);
        let world_units_per_pixel =
            2.0 * distance * (self.vertical_fov_degrees.to_radians() * 0.5).tan() / viewport_height;
        let forward = normalize3(sub3(self.target, self.eye));
        let right = normalize3(cross3(forward, self.up));
        let up = normalize3(cross3(right, forward));
        let pan = add3(
            mul3(right, -delta_pixels[0] * world_units_per_pixel),
            mul3(up, delta_pixels[1] * world_units_per_pixel),
        );
        self.eye = add3(self.eye, pan);
        self.target = add3(self.target, pan);
    }

    fn zoom(&mut self, scroll_lines: f32) {
        let offset = sub3(self.eye, self.target);
        let distance = length3(offset).max(CAMERA_MIN_DISTANCE);
        let zoom_factor = CAMERA_ZOOM_LINE_FACTOR.powf(scroll_lines);
        let new_distance = (distance * zoom_factor).clamp(CAMERA_MIN_DISTANCE, CAMERA_MAX_DISTANCE);
        self.eye = add3(self.target, mul3(normalize3(offset), new_distance));
        self.near = self.near.min((new_distance * 0.01).max(0.001));
        self.far = self.far.max(new_distance * 4.0);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderObject {
    pub name: String,
    pub transform: RenderTransform,
    pub model_matrix: Option<[[f32; 4]; 4]>,
    pub animation: Option<RenderAnimation>,
    pub mesh: RenderMesh,
}

impl RenderObject {
    fn model_matrix(&self, elapsed_seconds: f32) -> [[f32; 4]; 4] {
        self.model_matrix
            .unwrap_or_else(|| self.transform.model_matrix(elapsed_seconds, self.animation))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RenderAnimation {
    pub rotation_degrees_per_second: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RenderTransform {
    pub translation: [f32; 3],
    pub rotation_euler_degrees: [f32; 3],
    pub scale: [f32; 3],
}

impl RenderTransform {
    pub const IDENTITY: Self = Self {
        translation: [0.0, 0.0, 0.0],
        rotation_euler_degrees: [0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
    };

    fn model_matrix(
        self,
        elapsed_seconds: f32,
        animation: Option<RenderAnimation>,
    ) -> [[f32; 4]; 4] {
        let animated_rotation = if let Some(animation) = animation {
            [
                self.rotation_euler_degrees[0]
                    + animation.rotation_degrees_per_second[0] * elapsed_seconds,
                self.rotation_euler_degrees[1]
                    + animation.rotation_degrees_per_second[1] * elapsed_seconds,
                self.rotation_euler_degrees[2]
                    + animation.rotation_degrees_per_second[2] * elapsed_seconds,
            ]
        } else {
            self.rotation_euler_degrees
        };
        let scale = scale_matrix(self.scale);
        let rotation = multiply_mat4(
            rotation_z(animated_rotation[2].to_radians()),
            multiply_mat4(
                rotation_y(animated_rotation[1].to_radians()),
                rotation_x(animated_rotation[0].to_radians()),
            ),
        );
        multiply_mat4(
            translation_matrix(self.translation),
            multiply_mat4(rotation, scale),
        )
    }
}

impl Default for RenderTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SceneBounds {
    min: [f32; 3],
    max: [f32; 3],
}

impl SceneBounds {
    fn empty() -> Self {
        Self {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        }
    }

    fn include_point(&mut self, point: [f32; 3]) {
        for (axis, value) in point.into_iter().enumerate() {
            self.min[axis] = self.min[axis].min(value);
            self.max[axis] = self.max[axis].max(value);
        }
    }

    fn is_valid(self) -> bool {
        self.min
            .into_iter()
            .chain(self.max)
            .all(|value| value.is_finite())
            && self
                .min
                .into_iter()
                .zip(self.max)
                .all(|(min, max)| min <= max)
    }

    fn center(self) -> [f32; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }

    fn radius(self) -> f32 {
        length3(sub3(self.max, self.min)) * 0.5
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub enum RenderMesh {
    #[default]
    DemoCube,
    DemoGroundPlane,
    Static(Box<StaticMeshAsset>),
}

impl RenderMesh {
    fn log_label(&self) -> String {
        match self {
            Self::DemoCube => "DemoCube".to_string(),
            Self::DemoGroundPlane => "DemoGroundPlane".to_string(),
            Self::Static(mesh) => format!(
                "Static(name={:?}, vertices={}, indices={})",
                mesh.name,
                mesh.vertices.len(),
                mesh.indices.len()
            ),
        }
    }
}

const DEMO_CUBE_VERTICES: [Vertex; 24] = [
    // Front face: 0..4
    vertex([-0.5, -0.5, 0.5], [0.0, 0.0, 1.0], [0.95, 0.20, 0.20]),
    vertex([0.5, -0.5, 0.5], [0.0, 0.0, 1.0], [0.95, 0.20, 0.20]),
    vertex([0.5, 0.5, 0.5], [0.0, 0.0, 1.0], [0.95, 0.20, 0.20]),
    vertex([-0.5, 0.5, 0.5], [0.0, 0.0, 1.0], [0.95, 0.20, 0.20]),
    // Back face: 4..8
    vertex([0.5, -0.5, -0.5], [0.0, 0.0, -1.0], [0.20, 0.85, 0.35]),
    vertex([-0.5, -0.5, -0.5], [0.0, 0.0, -1.0], [0.20, 0.85, 0.35]),
    vertex([-0.5, 0.5, -0.5], [0.0, 0.0, -1.0], [0.20, 0.85, 0.35]),
    vertex([0.5, 0.5, -0.5], [0.0, 0.0, -1.0], [0.20, 0.85, 0.35]),
    // Left face: 8..12
    vertex([-0.5, -0.5, -0.5], [-1.0, 0.0, 0.0], [0.25, 0.45, 1.00]),
    vertex([-0.5, -0.5, 0.5], [-1.0, 0.0, 0.0], [0.25, 0.45, 1.00]),
    vertex([-0.5, 0.5, 0.5], [-1.0, 0.0, 0.0], [0.25, 0.45, 1.00]),
    vertex([-0.5, 0.5, -0.5], [-1.0, 0.0, 0.0], [0.25, 0.45, 1.00]),
    // Right face: 12..16
    vertex([0.5, -0.5, 0.5], [1.0, 0.0, 0.0], [1.00, 0.78, 0.20]),
    vertex([0.5, -0.5, -0.5], [1.0, 0.0, 0.0], [1.00, 0.78, 0.20]),
    vertex([0.5, 0.5, -0.5], [1.0, 0.0, 0.0], [1.00, 0.78, 0.20]),
    vertex([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], [1.00, 0.78, 0.20]),
    // Top face: 16..20
    vertex([-0.5, 0.5, 0.5], [0.0, 1.0, 0.0], [0.85, 0.35, 0.95]),
    vertex([0.5, 0.5, 0.5], [0.0, 1.0, 0.0], [0.85, 0.35, 0.95]),
    vertex([0.5, 0.5, -0.5], [0.0, 1.0, 0.0], [0.85, 0.35, 0.95]),
    vertex([-0.5, 0.5, -0.5], [0.0, 1.0, 0.0], [0.85, 0.35, 0.95]),
    // Bottom face: 20..24
    vertex([-0.5, -0.5, -0.5], [0.0, -1.0, 0.0], [0.15, 0.80, 0.90]),
    vertex([0.5, -0.5, -0.5], [0.0, -1.0, 0.0], [0.15, 0.80, 0.90]),
    vertex([0.5, -0.5, 0.5], [0.0, -1.0, 0.0], [0.15, 0.80, 0.90]),
    vertex([-0.5, -0.5, 0.5], [0.0, -1.0, 0.0], [0.15, 0.80, 0.90]),
];

type MeshIndex = u16;

const DEMO_CUBE_INDICES: [MeshIndex; 36] = [
    0, 1, 2, 2, 3, 0, // Front
    4, 5, 6, 6, 7, 4, // Back
    8, 9, 10, 10, 11, 8, // Left
    12, 13, 14, 14, 15, 12, // Right
    16, 17, 18, 18, 19, 16, // Top
    20, 21, 22, 22, 23, 20, // Bottom
];

const DEMO_GROUND_PLANE_VERTICES: [Vertex; 16] = [
    // Back-left quad
    vertex([-4.0, 0.0, -4.0], [0.0, 1.0, 0.0], [0.18, 0.22, 0.24]),
    vertex([0.0, 0.0, -4.0], [0.0, 1.0, 0.0], [0.18, 0.22, 0.24]),
    vertex([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.18, 0.22, 0.24]),
    vertex([-4.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.18, 0.22, 0.24]),
    // Back-right quad
    vertex([0.0, 0.0, -4.0], [0.0, 1.0, 0.0], [0.42, 0.45, 0.41]),
    vertex([4.0, 0.0, -4.0], [0.0, 1.0, 0.0], [0.42, 0.45, 0.41]),
    vertex([4.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.42, 0.45, 0.41]),
    vertex([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.42, 0.45, 0.41]),
    // Front-left quad
    vertex([-4.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.42, 0.45, 0.41]),
    vertex([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.42, 0.45, 0.41]),
    vertex([0.0, 0.0, 4.0], [0.0, 1.0, 0.0], [0.42, 0.45, 0.41]),
    vertex([-4.0, 0.0, 4.0], [0.0, 1.0, 0.0], [0.42, 0.45, 0.41]),
    // Front-right quad
    vertex([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.18, 0.22, 0.24]),
    vertex([4.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.18, 0.22, 0.24]),
    vertex([4.0, 0.0, 4.0], [0.0, 1.0, 0.0], [0.18, 0.22, 0.24]),
    vertex([0.0, 0.0, 4.0], [0.0, 1.0, 0.0], [0.18, 0.22, 0.24]),
];

const DEMO_GROUND_PLANE_INDICES: [MeshIndex; 48] = [
    0, 1, 2, 2, 3, 0, 2, 1, 0, 0, 3, 2, // Back-left, double-sided
    4, 5, 6, 6, 7, 4, 6, 5, 4, 4, 7, 6, // Back-right, double-sided
    8, 9, 10, 10, 11, 8, 10, 9, 8, 8, 11, 10, // Front-left, double-sided
    12, 13, 14, 14, 15, 12, 14, 13, 12, 12, 15, 14, // Front-right, double-sided
];

struct MeshGeometry<'a> {
    vertices: Cow<'a, [Vertex]>,
    indices: Cow<'a, [MeshIndex]>,
    material: Cow<'a, StaticMeshMaterialAsset>,
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("failed to load Vulkan entry points: {0}")]
    LoadVulkan(#[from] ash::LoadingError),
    #[error("Vulkan call failed: {0:?}")]
    Vulkan(#[from] vk::Result),
    #[error("window handle error: {0}")]
    WindowHandle(#[from] raw_window_handle::HandleError),
    #[error("winit event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("winit OS error: {0}")]
    Os(#[from] winit::error::OsError),
    #[error("asset error: {0}")]
    Asset(#[from] engine_assets::AssetError),
    #[error("{0}")]
    Message(String),
}

pub type RenderResult<T> = Result<T, RenderError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderBackend {
    Vulkan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VulkanFeaturePolicy {
    pub require_vulkan_1_3: bool,
    pub prefer_ray_tracing: bool,
    pub allow_raster_fallback: bool,
    pub enable_validation_layers: bool,
}

impl Default for VulkanFeaturePolicy {
    fn default() -> Self {
        Self {
            require_vulkan_1_3: true,
            prefer_ray_tracing: true,
            allow_raster_fallback: true,
            enable_validation_layers: cfg!(debug_assertions),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RendererPath {
    Raster,
    RayTracing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererConfig {
    pub backend: RenderBackend,
    pub features: VulkanFeaturePolicy,
    pub shader_source: ShaderSource,
}

impl RendererConfig {
    pub fn vulkan_high_end() -> Self {
        Self {
            backend: RenderBackend::Vulkan,
            features: VulkanFeaturePolicy::default(),
            shader_source: ShaderSource::Slang,
        }
    }

    pub fn choose_path(&self, ray_tracing_available: bool) -> RendererPath {
        if self.features.prefer_ray_tracing && ray_tracing_available {
            RendererPath::RayTracing
        } else {
            RendererPath::Raster
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShaderSource {
    Slang,
}

#[derive(Debug, Clone)]
pub struct VulkanWindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub clear_color: [f32; 4],
    pub max_frames: Option<u64>,
}

impl Default for VulkanWindowConfig {
    fn default() -> Self {
        Self {
            title: WINDOW_TITLE.to_string(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            clear_color: [0.02, 0.04, 0.08, 1.0],
            max_frames: None,
        }
    }
}

pub fn run_vulkan_renderer(config: RendererConfig) -> RenderResult<()> {
    run_vulkan_renderer_with_window_config(config, VulkanWindowConfig::default())
}

pub fn run_vulkan_renderer_with_window_config(
    config: RendererConfig,
    window_config: VulkanWindowConfig,
) -> RenderResult<()> {
    run_vulkan_renderer_with_scene(config, window_config, RenderScene::default_or_test_scene()?)
}

pub fn run_vulkan_renderer_with_scene(
    config: RendererConfig,
    window_config: VulkanWindowConfig,
    scene: RenderScene,
) -> RenderResult<()> {
    info!("Starting FinalEngine Vulkan renderer");
    info!("Renderer config: {config:#?}");
    info!("Window config: {window_config:#?}");
    log_render_scene_submission(&scene);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = VulkanApp::new(config, window_config, scene);
    event_loop.run_app(&mut app)?;

    if let Some(error) = app.fatal_error {
        return Err(error);
    }

    info!("Vulkan renderer shut down cleanly");
    Ok(())
}

fn log_render_scene_submission(scene: &RenderScene) {
    info!(
        "Render scene submission: objects={} camera_eye=({:.3}, {:.3}, {:.3}) camera_target=({:.3}, {:.3}, {:.3}) fov_y={:.3} near={:.3} far={:.3}",
        scene.objects.len(),
        scene.camera.eye[0],
        scene.camera.eye[1],
        scene.camera.eye[2],
        scene.camera.target[0],
        scene.camera.target[1],
        scene.camera.target[2],
        scene.camera.vertical_fov_degrees,
        scene.camera.near,
        scene.camera.far
    );
    for (index, object) in scene.objects.iter().enumerate() {
        info!(
            "  object[{index}] name={:?} mesh={} translation=({:.3}, {:.3}, {:.3}) rotation_euler_degrees=({:.3}, {:.3}, {:.3}) scale=({:.3}, {:.3}, {:.3}) animation={:?}",
            object.name,
            object.mesh.log_label(),
            object.transform.translation[0],
            object.transform.translation[1],
            object.transform.translation[2],
            object.transform.rotation_euler_degrees[0],
            object.transform.rotation_euler_degrees[1],
            object.transform.rotation_euler_degrees[2],
            object.transform.scale[0],
            object.transform.scale[1],
            object.transform.scale[2],
            object.animation
        );
    }
}

struct VulkanApp {
    renderer_config: RendererConfig,
    window_config: VulkanWindowConfig,
    scene: RenderScene,
    renderer: Option<VulkanRenderer>,
    camera_controls: CameraMouseControls,
    fatal_error: Option<RenderError>,
    presented_frames: u64,
}

impl VulkanApp {
    fn new(
        renderer_config: RendererConfig,
        window_config: VulkanWindowConfig,
        scene: RenderScene,
    ) -> Self {
        Self {
            renderer_config,
            window_config,
            scene,
            renderer: None,
            camera_controls: CameraMouseControls::default(),
            fatal_error: None,
            presented_frames: 0,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: RenderError) {
        error!("Fatal renderer error: {error}");
        self.fatal_error = Some(error);
        event_loop.exit();
    }
}

#[derive(Debug, Default)]
struct CameraMouseControls {
    last_cursor_position: Option<PhysicalPosition<f64>>,
    orbiting: bool,
    panning: bool,
}

impl CameraMouseControls {
    fn handle_button(&mut self, button: MouseButton, state: ElementState) {
        let pressed = state.is_pressed();
        match button {
            MouseButton::Left => {
                self.orbiting = pressed;
                info!(
                    "Camera orbit {} with left mouse button",
                    if pressed { "started" } else { "stopped" }
                );
            }
            MouseButton::Middle | MouseButton::Right => {
                self.panning = pressed;
                info!(
                    "Camera pan {} with {button:?} mouse button",
                    if pressed { "started" } else { "stopped" }
                );
            }
            _ => {}
        }
    }

    fn handle_cursor_moved(
        &mut self,
        renderer: &mut VulkanRenderer,
        position: PhysicalPosition<f64>,
    ) {
        let Some(previous_position) = self.last_cursor_position.replace(position) else {
            return;
        };

        let delta_pixels = [
            (position.x - previous_position.x) as f32,
            (position.y - previous_position.y) as f32,
        ];
        if self.panning {
            renderer.pan_camera(delta_pixels);
        } else if self.orbiting {
            renderer.orbit_camera(delta_pixels);
        }
    }

    fn handle_wheel(&mut self, renderer: &mut VulkanRenderer, delta: MouseScrollDelta) {
        let scroll_lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(position) => position.y as f32 / 120.0,
        };

        if scroll_lines.abs() <= f32::EPSILON {
            return;
        }

        renderer.zoom_camera(scroll_lines);
        info!("Camera zoom changed from mouse wheel: scroll_lines={scroll_lines:.3}");
    }
}

impl ApplicationHandler for VulkanApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        info!("winit resumed; creating Vulkan window and renderer");
        if self.renderer.is_some() {
            info!("Renderer already exists after resume; no new renderer needed");
            return;
        }

        let window_attributes = Window::default_attributes()
            .with_title(self.window_config.title.clone())
            .with_inner_size(PhysicalSize::new(
                self.window_config.width,
                self.window_config.height,
            ));

        let window = match event_loop.create_window(window_attributes) {
            Ok(window) => window,
            Err(error) => return self.fail(event_loop, error.into()),
        };

        match VulkanRenderer::new(
            window,
            self.renderer_config.clone(),
            self.window_config.clone(),
            self.scene.clone(),
        ) {
            Ok(renderer) => {
                info!("Vulkan renderer is ready; requesting first redraw");
                info!("Camera mouse controls: left-drag orbit, middle/right-drag pan, wheel zoom");
                renderer.window.request_redraw();
                self.renderer = Some(renderer);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        if renderer.window.id() != window_id {
            warn!("Ignoring event for unknown window {window_id:?}");
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                info!("Close requested; exiting renderer loop");
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state.is_pressed()
                    && matches!(event.logical_key, Key::Named(NamedKey::Escape)) =>
            {
                info!("Escape pressed; exiting renderer loop");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                info!("Window resized to {}x{}", size.width, size.height);
                renderer.resize(size);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                info!("Window scale factor changed to {scale_factor}");
                renderer.mark_swapchain_dirty();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.camera_controls.handle_button(button, state);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.camera_controls.handle_cursor_moved(renderer, position);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.camera_controls.handle_wheel(renderer, delta);
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = renderer.draw_frame() {
                    self.fail(event_loop, error);
                    return;
                }
                self.presented_frames += 1;
                if let Some(max_frames) = self.window_config.max_frames
                    && self.presented_frames >= max_frames
                {
                    info!(
                        "Reached configured max frame count ({max_frames}); exiting renderer loop"
                    );
                    event_loop.exit();
                    return;
                }
                renderer.window.request_redraw();
            }
            WindowEvent::Occluded(occluded) => {
                info!("Window occlusion changed: {occluded}");
                renderer.occluded = occluded;
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(renderer) = self.renderer.as_ref() {
            renderer.window.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        info!("winit exiting; dropping Vulkan renderer");
        self.renderer.take();
    }
}

struct VulkanRenderer {
    window: Window,
    scene: RenderScene,
    started_at: Instant,
    _entry: Entry,
    instance: Instance,
    debug_utils: Option<ash::ext::debug_utils::Instance>,
    debug_messenger: Option<vk::DebugUtilsMessengerEXT>,
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    device: Device,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    queue_family_indices: QueueFamilyIndices,
    swapchain_loader: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_image_views: Vec<vk::ImageView>,
    swapchain_format: vk::Format,
    swapchain_extent: vk::Extent2D,
    depth_image: GpuImage,
    depth_format: vk::Format,
    render_pass: vk::RenderPass,
    camera_descriptor_set_layout: vk::DescriptorSetLayout,
    texture_descriptor_set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    graphics_pipeline: vk::Pipeline,
    framebuffers: Vec<vk::Framebuffer>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    render_objects: Vec<GpuRenderObject>,
    camera_uniform_buffers: Vec<GpuBuffer>,
    camera_descriptor_pool: vk::DescriptorPool,
    camera_descriptor_sets: Vec<vk::DescriptorSet>,
    image_available: vk::Semaphore,
    render_finished: Vec<vk::Semaphore>,
    in_flight: vk::Fence,
    clear_color: vk::ClearValue,
    framebuffer_resized: bool,
    occluded: bool,
}

impl VulkanRenderer {
    fn new(
        window: Window,
        renderer_config: RendererConfig,
        window_config: VulkanWindowConfig,
        scene: RenderScene,
    ) -> RenderResult<Self> {
        info!("Loading Vulkan loader");
        let entry = unsafe { Entry::load()? };
        log_instance_layers_and_extensions(&entry)?;

        let validation_enabled = renderer_config.features.enable_validation_layers
            && validation_layer_available(&entry)?;
        if renderer_config.features.enable_validation_layers && !validation_enabled {
            warn!("{VALIDATION_LAYER:?} is not available; continuing without validation layers");
        }
        info!("Validation layers enabled: {validation_enabled}");

        let display_handle = window.display_handle()?.as_raw();
        let required_extensions = ash_window::enumerate_required_extensions(display_handle)?;
        let mut extension_names = required_extensions.to_vec();
        if validation_enabled {
            extension_names.push(ash::ext::debug_utils::NAME.as_ptr());
        }
        log_enabled_extensions("Instance extensions", &extension_names);

        let layer_names = if validation_enabled {
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };
        log_enabled_extensions("Instance layers", &layer_names);

        let app_name = CString::new("FinalEngine").expect("static app name is valid");
        let engine_name = CString::new("FinalEngine").expect("static engine name is valid");
        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&engine_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_3);

        let mut debug_create_info = debug_messenger_create_info();
        let mut instance_create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extension_names)
            .enabled_layer_names(&layer_names);

        if validation_enabled {
            instance_create_info = instance_create_info.push_next(&mut debug_create_info);
        }

        info!("Creating Vulkan instance");
        let instance = unsafe { entry.create_instance(&instance_create_info, None)? };
        let debug_utils =
            validation_enabled.then(|| ash::ext::debug_utils::Instance::new(&entry, &instance));
        let debug_messenger = if let Some(debug_utils) = debug_utils.as_ref() {
            info!("Creating Vulkan debug messenger");
            Some(unsafe { debug_utils.create_debug_utils_messenger(&debug_create_info, None)? })
        } else {
            None
        };

        info!("Creating window surface");
        let surface = unsafe {
            ash_window::create_surface(
                &entry,
                &instance,
                window.display_handle()?.as_raw(),
                window.window_handle()?.as_raw(),
                None,
            )?
        };
        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);

        let physical_device = pick_physical_device(
            &instance,
            &surface_loader,
            surface,
            &renderer_config.features,
        )?;
        let queue_family_indices =
            find_queue_families(&instance, &surface_loader, surface, physical_device)?.ok_or_else(
                || RenderError::Message("selected device lost queue support".to_string()),
            )?;

        info!(
            "Selected queue families: graphics={}, present={}",
            queue_family_indices.graphics, queue_family_indices.present
        );

        let unique_queue_families = queue_family_indices.unique();
        let queue_priority = [1.0_f32];
        let queue_create_infos = unique_queue_families
            .iter()
            .map(|family| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(*family)
                    .queue_priorities(&queue_priority)
            })
            .collect::<Vec<_>>();

        let device_extensions = [ash::khr::swapchain::NAME.as_ptr()];
        log_enabled_extensions("Device extensions", &device_extensions);
        let device_features = vk::PhysicalDeviceFeatures::default();
        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extensions)
            .enabled_features(&device_features);

        info!("Creating Vulkan logical device");
        let device = unsafe { instance.create_device(physical_device, &device_create_info, None)? };
        let graphics_queue = unsafe { device.get_device_queue(queue_family_indices.graphics, 0) };
        let present_queue = unsafe { device.get_device_queue(queue_family_indices.present, 0) };
        info!("Retrieved graphics and present queues");

        let camera_descriptor_set_layout = create_camera_descriptor_set_layout(&device)?;
        let texture_descriptor_set_layout = create_texture_descriptor_set_layout(&device)?;
        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };
        let swapchain_loader = ash::khr::swapchain::Device::new(&instance, &device);
        let swapchain_support = query_swapchain_support(physical_device, &surface_loader, surface)?;
        log_swapchain_support(&swapchain_support);

        let window_size = window.inner_size();
        let swapchain_bundle = create_swapchain_bundle(SwapchainCreateContext {
            instance: &instance,
            device: &device,
            physical_device,
            swapchain_loader: &swapchain_loader,
            surface,
            support: &swapchain_support,
            queue_family_indices,
            window_size,
            camera_descriptor_set_layout,
            texture_descriptor_set_layout,
        })?;

        let command_pool_create_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_indices.graphics)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        info!("Creating command pool");
        let command_pool = unsafe { device.create_command_pool(&command_pool_create_info, None)? };

        scene.primary_object()?;
        let render_objects = create_gpu_render_objects(
            &instance,
            &device,
            physical_device,
            command_pool,
            graphics_queue,
            texture_descriptor_set_layout,
            &scene,
        )?;
        let camera_uniform_buffers = create_camera_uniform_buffers(
            &device,
            &memory_properties,
            swapchain_bundle.framebuffers.len(),
        )?;
        let (camera_descriptor_pool, camera_descriptor_sets) = create_camera_descriptor_sets(
            &device,
            camera_descriptor_set_layout,
            &camera_uniform_buffers,
        )?;

        let command_buffers =
            allocate_command_buffers(&device, command_pool, swapchain_bundle.framebuffers.len())?;
        let semaphore_create_info = vk::SemaphoreCreateInfo::default();
        let fence_create_info =
            vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        info!("Creating frame synchronization primitives");
        let image_available = unsafe { device.create_semaphore(&semaphore_create_info, None)? };
        let render_finished =
            create_render_finished_semaphores(&device, swapchain_bundle.framebuffers.len())?;
        let in_flight = unsafe { device.create_fence(&fence_create_info, None)? };

        info!(
            "Renderer initialized: swapchain_images={}, format={:?}, extent={}x{}",
            swapchain_bundle.images.len(),
            swapchain_bundle.format,
            swapchain_bundle.extent.width,
            swapchain_bundle.extent.height
        );

        Ok(Self {
            window,
            scene,
            started_at: Instant::now(),
            _entry: entry,
            instance,
            debug_utils,
            debug_messenger,
            surface_loader,
            surface,
            physical_device,
            device,
            graphics_queue,
            present_queue,
            queue_family_indices,
            swapchain_loader,
            swapchain: swapchain_bundle.swapchain,
            swapchain_images: swapchain_bundle.images,
            swapchain_image_views: swapchain_bundle.image_views,
            swapchain_format: swapchain_bundle.format,
            swapchain_extent: swapchain_bundle.extent,
            depth_image: swapchain_bundle.depth_image,
            depth_format: swapchain_bundle.depth_format,
            render_pass: swapchain_bundle.render_pass,
            camera_descriptor_set_layout,
            texture_descriptor_set_layout,
            pipeline_layout: swapchain_bundle.pipeline_layout,
            graphics_pipeline: swapchain_bundle.graphics_pipeline,
            framebuffers: swapchain_bundle.framebuffers,
            command_pool,
            command_buffers,
            render_objects,
            camera_uniform_buffers,
            camera_descriptor_pool,
            camera_descriptor_sets,
            image_available,
            render_finished,
            in_flight,
            clear_color: vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: window_config.clear_color,
                },
            },
            framebuffer_resized: false,
            occluded: false,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            info!("Window minimized or zero-sized; deferring swapchain recreation");
            return;
        }
        self.framebuffer_resized = true;
    }

    fn orbit_camera(&mut self, delta_pixels: [f32; 2]) {
        self.scene.camera.orbit(delta_pixels);
        self.window.request_redraw();
    }

    fn pan_camera(&mut self, delta_pixels: [f32; 2]) {
        self.scene
            .camera
            .pan(delta_pixels, self.window.inner_size());
        self.window.request_redraw();
    }

    fn zoom_camera(&mut self, scroll_lines: f32) {
        self.scene.camera.zoom(scroll_lines);
        self.window.request_redraw();
    }

    fn mark_swapchain_dirty(&mut self) {
        self.framebuffer_resized = true;
    }

    fn draw_frame(&mut self) -> RenderResult<()> {
        if self.occluded {
            trace!("Skipping draw for occluded window");
            return Ok(());
        }

        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            trace!("Skipping draw for zero-sized window");
            return Ok(());
        }

        if self.framebuffer_resized {
            info!("Swapchain marked dirty before draw; recreating");
            self.recreate_swapchain()?;
        }

        unsafe {
            self.device
                .wait_for_fences(&[self.in_flight], true, u64::MAX)?;
        }

        let image_index = match unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                self.image_available,
                vk::Fence::null(),
            )
        } {
            Ok((image_index, suboptimal)) => {
                if suboptimal {
                    info!("Swapchain acquire reported suboptimal; will recreate after this frame");
                    self.framebuffer_resized = true;
                }
                image_index
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                info!("Swapchain out of date during acquire; recreating");
                self.recreate_swapchain()?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };

        unsafe {
            self.device.reset_fences(&[self.in_flight])?;
            self.device.reset_command_buffer(
                self.command_buffers[image_index as usize],
                vk::CommandBufferResetFlags::empty(),
            )?;
        }

        update_camera_uniform(
            &self.device,
            &self.camera_uniform_buffers[image_index as usize],
            self.swapchain_extent,
            self.scene.camera,
        )?;

        let elapsed_seconds = self.started_at.elapsed().as_secs_f32();
        let render_draws = self
            .render_objects
            .iter()
            .map(|render_object| {
                let object = &self.scene.objects[render_object.scene_object_index];
                RenderDraw {
                    vertex_buffer: render_object.mesh.vertex_buffer.buffer,
                    index_buffer: render_object.mesh.index_buffer.buffer,
                    index_count: render_object.mesh.index_count,
                    texture_descriptor_set: render_object.mesh.material.texture.descriptor_set,
                    object_constants: ObjectPushConstants {
                        model: object.model_matrix(elapsed_seconds),
                        base_color_factor: render_object.mesh.material.base_color_factor,
                    },
                }
            })
            .collect::<Vec<_>>();

        record_render_commands(
            &self.device,
            RenderCommandParams {
                command_buffer: self.command_buffers[image_index as usize],
                render_pass: self.render_pass,
                framebuffer: self.framebuffers[image_index as usize],
                pipeline_layout: self.pipeline_layout,
                graphics_pipeline: self.graphics_pipeline,
                render_draws: &render_draws,
                camera_descriptor_set: self.camera_descriptor_sets[image_index as usize],
                extent: self.swapchain_extent,
                clear_color: self.clear_color,
            },
        )?;

        let wait_semaphores = [self.image_available];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let command_buffers = [self.command_buffers[image_index as usize]];
        let signal_semaphores = [self.render_finished[image_index as usize]];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores);

        unsafe {
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], self.in_flight)?;
        }

        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        match unsafe {
            self.swapchain_loader
                .queue_present(self.present_queue, &present_info)
        } {
            Ok(suboptimal) => {
                if suboptimal || self.framebuffer_resized {
                    info!("Swapchain present reported suboptimal or resize pending; recreating");
                    self.recreate_swapchain()?;
                }
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                info!("Swapchain out of date during present; recreating");
                self.recreate_swapchain()?;
            }
            Err(error) => return Err(error.into()),
        }

        Ok(())
    }

    fn recreate_swapchain(&mut self) -> RenderResult<()> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            info!("Swapchain recreation skipped for zero-sized window");
            return Ok(());
        }

        info!("Waiting for device idle before swapchain recreation");
        unsafe {
            self.device.device_wait_idle()?;
        }

        self.destroy_swapchain_resources();
        let support =
            query_swapchain_support(self.physical_device, &self.surface_loader, self.surface)?;
        log_swapchain_support(&support);
        let bundle = create_swapchain_bundle(SwapchainCreateContext {
            instance: &self.instance,
            device: &self.device,
            physical_device: self.physical_device,
            swapchain_loader: &self.swapchain_loader,
            surface: self.surface,
            support: &support,
            queue_family_indices: self.queue_family_indices,
            window_size: size,
            camera_descriptor_set_layout: self.camera_descriptor_set_layout,
            texture_descriptor_set_layout: self.texture_descriptor_set_layout,
        })?;
        self.swapchain = bundle.swapchain;
        self.swapchain_images = bundle.images;
        self.swapchain_image_views = bundle.image_views;
        self.swapchain_format = bundle.format;
        self.swapchain_extent = bundle.extent;
        self.depth_image = bundle.depth_image;
        self.depth_format = bundle.depth_format;
        self.render_pass = bundle.render_pass;
        self.pipeline_layout = bundle.pipeline_layout;
        self.graphics_pipeline = bundle.graphics_pipeline;
        self.framebuffers = bundle.framebuffers;
        let memory_properties = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        self.camera_uniform_buffers = create_camera_uniform_buffers(
            &self.device,
            &memory_properties,
            self.framebuffers.len(),
        )?;
        (self.camera_descriptor_pool, self.camera_descriptor_sets) = create_camera_descriptor_sets(
            &self.device,
            self.camera_descriptor_set_layout,
            &self.camera_uniform_buffers,
        )?;
        self.command_buffers =
            allocate_command_buffers(&self.device, self.command_pool, self.framebuffers.len())?;
        self.render_finished =
            create_render_finished_semaphores(&self.device, self.framebuffers.len())?;
        self.framebuffer_resized = false;

        info!(
            "Swapchain recreated: images={}, format={:?}, extent={}x{}",
            self.swapchain_images.len(),
            self.swapchain_format,
            self.swapchain_extent.width,
            self.swapchain_extent.height
        );
        Ok(())
    }

    fn destroy_swapchain_resources(&mut self) {
        info!("Destroying swapchain-dependent resources");
        unsafe {
            if !self.command_buffers.is_empty() {
                self.device
                    .free_command_buffers(self.command_pool, &self.command_buffers);
            }
            self.command_buffers.clear();
            for semaphore in self.render_finished.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
            for buffer in &mut self.camera_uniform_buffers {
                destroy_gpu_buffer(&self.device, buffer, "camera uniform buffer");
            }
            self.camera_uniform_buffers.clear();
            self.camera_descriptor_sets.clear();
            if self.camera_descriptor_pool != vk::DescriptorPool::null() {
                info!(
                    "Destroying camera descriptor pool {:?}",
                    self.camera_descriptor_pool
                );
                self.device
                    .destroy_descriptor_pool(self.camera_descriptor_pool, None);
                self.camera_descriptor_pool = vk::DescriptorPool::null();
            }

            for framebuffer in self.framebuffers.drain(..) {
                self.device.destroy_framebuffer(framebuffer, None);
            }
            destroy_gpu_image(&self.device, &mut self.depth_image, "swapchain depth image");
            if self.graphics_pipeline != vk::Pipeline::null() {
                self.device.destroy_pipeline(self.graphics_pipeline, None);
                self.graphics_pipeline = vk::Pipeline::null();
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device
                    .destroy_pipeline_layout(self.pipeline_layout, None);
                self.pipeline_layout = vk::PipelineLayout::null();
            }
            if self.render_pass != vk::RenderPass::null() {
                self.device.destroy_render_pass(self.render_pass, None);
                self.render_pass = vk::RenderPass::null();
            }
            for image_view in self.swapchain_image_views.drain(..) {
                self.device.destroy_image_view(image_view, None);
            }
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_loader
                    .destroy_swapchain(self.swapchain, None);
                self.swapchain = vk::SwapchainKHR::null();
            }
        }
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        info!("Dropping Vulkan renderer and destroying GPU resources");
        unsafe {
            if let Err(error) = self.device.device_wait_idle() {
                warn!("device_wait_idle failed during drop: {error:?}");
            }
            self.destroy_swapchain_resources();
            destroy_gpu_render_objects(&self.device, &mut self.render_objects);
            self.device
                .destroy_descriptor_set_layout(self.camera_descriptor_set_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.texture_descriptor_set_layout, None);
            self.device.destroy_fence(self.in_flight, None);
            self.device.destroy_semaphore(self.image_available, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);
            if let (Some(debug_utils), Some(debug_messenger)) =
                (self.debug_utils.as_ref(), self.debug_messenger)
            {
                debug_utils.destroy_debug_utils_messenger(debug_messenger, None);
            }
            self.instance.destroy_instance(None);
        }
        info!("Vulkan resources destroyed");
    }
}

#[derive(Debug, Clone, Copy)]
struct QueueFamilyIndices {
    graphics: u32,
    present: u32,
}

impl QueueFamilyIndices {
    fn unique(self) -> Vec<u32> {
        BTreeSet::from([self.graphics, self.present])
            .into_iter()
            .collect()
    }
}

#[derive(Debug)]
struct SwapchainSupport {
    capabilities: vk::SurfaceCapabilitiesKHR,
    formats: Vec<vk::SurfaceFormatKHR>,
    present_modes: Vec<vk::PresentModeKHR>,
}

struct SwapchainBundle {
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    image_views: Vec<vk::ImageView>,
    format: vk::Format,
    extent: vk::Extent2D,
    depth_image: GpuImage,
    depth_format: vk::Format,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    graphics_pipeline: vk::Pipeline,
    framebuffers: Vec<vk::Framebuffer>,
}

#[derive(Debug)]
struct GpuBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: vk::DeviceSize,
}

#[derive(Debug)]
struct GpuMesh {
    vertex_buffer: GpuBuffer,
    index_buffer: GpuBuffer,
    index_count: u32,
    material: GpuMaterial,
}

#[derive(Debug)]
struct GpuMaterial {
    base_color_factor: [f32; 4],
    texture: GpuTexture,
}

#[derive(Debug)]
struct GpuTexture {
    image: GpuImage,
    sampler: vk::Sampler,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set: vk::DescriptorSet,
    width: u32,
    height: u32,
    mip_levels: u32,
    is_fallback: bool,
}

#[derive(Debug)]
struct GpuRenderObject {
    scene_object_index: usize,
    mesh: GpuMesh,
}

#[derive(Debug)]
struct GpuImage {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    mip_levels: u32,
}

struct SwapchainCreateContext<'a> {
    instance: &'a Instance,
    device: &'a Device,
    physical_device: vk::PhysicalDevice,
    swapchain_loader: &'a ash::khr::swapchain::Device,
    surface: vk::SurfaceKHR,
    support: &'a SwapchainSupport,
    queue_family_indices: QueueFamilyIndices,
    window_size: PhysicalSize<u32>,
    camera_descriptor_set_layout: vk::DescriptorSetLayout,
    texture_descriptor_set_layout: vk::DescriptorSetLayout,
}

fn log_instance_layers_and_extensions(entry: &Entry) -> RenderResult<()> {
    let layers = unsafe { entry.enumerate_instance_layer_properties()? };
    info!("Available Vulkan instance layers: {}", layers.len());
    for layer in layers {
        info!(
            "  layer={} spec={} implementation={} description={}",
            vk_string(&layer.layer_name),
            layer.spec_version,
            layer.implementation_version,
            vk_string(&layer.description)
        );
    }

    let extensions = unsafe { entry.enumerate_instance_extension_properties(None)? };
    info!("Available Vulkan instance extensions: {}", extensions.len());
    for extension in extensions {
        info!(
            "  extension={} spec={}",
            vk_string(&extension.extension_name),
            extension.spec_version
        );
    }
    Ok(())
}

fn validation_layer_available(entry: &Entry) -> RenderResult<bool> {
    let layers = unsafe { entry.enumerate_instance_layer_properties()? };
    Ok(layers
        .iter()
        .any(|layer| unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) } == VALIDATION_LAYER))
}

fn log_enabled_extensions(label: &str, names: &[*const i8]) {
    info!("{label}: {}", names.len());
    for name in names {
        let name = unsafe { CStr::from_ptr(*name) };
        info!("  {}", name.to_string_lossy());
    }
}

fn pick_physical_device(
    instance: &Instance,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    policy: &VulkanFeaturePolicy,
) -> RenderResult<vk::PhysicalDevice> {
    let devices = unsafe { instance.enumerate_physical_devices()? };
    info!("Enumerating Vulkan physical devices: {}", devices.len());
    let mut best: Option<(i32, vk::PhysicalDevice)> = None;

    for device in devices {
        let properties = unsafe { instance.get_physical_device_properties(device) };
        let features = unsafe { instance.get_physical_device_features(device) };
        let api = properties.api_version;
        let name = vk_string(&properties.device_name);
        let device_type = properties.device_type;
        info!("Physical device: {name}");
        info!("  type={device_type:?}");
        info!(
            "  api={}.{}.{}",
            vk::api_version_major(api),
            vk::api_version_minor(api),
            vk::api_version_patch(api)
        );
        info!(
            "  vendor_id=0x{:x} device_id=0x{:x}",
            properties.vendor_id, properties.device_id
        );
        info!("  geometry_shader={}", features.geometry_shader == vk::TRUE);
        info!(
            "  sampler_anisotropy={}",
            features.sampler_anisotropy == vk::TRUE
        );

        let extensions = unsafe { instance.enumerate_device_extension_properties(device)? };
        info!("  device extensions={}", extensions.len());
        for extension in &extensions {
            info!(
                "    {} spec={}",
                vk_string(&extension.extension_name),
                extension.spec_version
            );
        }

        let has_swapchain = extensions.iter().any(|extension| {
            let extension_name = unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) };
            extension_name == ash::khr::swapchain::NAME
        });
        let queue_families = find_queue_families(instance, surface_loader, surface, device)?;
        let swapchain_support = query_swapchain_support(device, surface_loader, surface)?;
        let supports_vulkan_1_3 = vk::api_version_major(api) > 1 || vk::api_version_minor(api) >= 3;
        let usable = has_swapchain
            && queue_families.is_some()
            && !swapchain_support.formats.is_empty()
            && !swapchain_support.present_modes.is_empty()
            && (!policy.require_vulkan_1_3 || supports_vulkan_1_3);

        info!("  has_swapchain_extension={has_swapchain}");
        info!("  has_required_queues={}", queue_families.is_some());
        info!("  surface_formats={}", swapchain_support.formats.len());
        info!("  present_modes={}", swapchain_support.present_modes.len());
        info!("  supports_vulkan_1_3={supports_vulkan_1_3}");
        info!("  usable={usable}");

        if !usable {
            continue;
        }

        let score = match device_type {
            vk::PhysicalDeviceType::DISCRETE_GPU => 10_000,
            vk::PhysicalDeviceType::INTEGRATED_GPU => 5_000,
            vk::PhysicalDeviceType::VIRTUAL_GPU => 2_500,
            vk::PhysicalDeviceType::CPU => 500,
            _ => 100,
        } + properties.limits.max_image_dimension2_d as i32;

        info!("  selection_score={score}");
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, device));
        }
    }

    best.map(|(_, device)| device)
        .ok_or_else(|| RenderError::Message("no suitable Vulkan physical device found".to_string()))
}

fn find_queue_families(
    instance: &Instance,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    device: vk::PhysicalDevice,
) -> RenderResult<Option<QueueFamilyIndices>> {
    let queue_families = unsafe { instance.get_physical_device_queue_family_properties(device) };
    let mut graphics = None;
    let mut present = None;

    for (index, family) in queue_families.iter().enumerate() {
        let index = index as u32;
        let supports_graphics = family.queue_flags.contains(vk::QueueFlags::GRAPHICS);
        let supports_present =
            unsafe { surface_loader.get_physical_device_surface_support(device, index, surface)? };
        info!(
            "  queue_family[{index}]: count={} flags={:?} graphics={} present={}",
            family.queue_count, family.queue_flags, supports_graphics, supports_present
        );

        if supports_graphics && graphics.is_none() {
            graphics = Some(index);
        }
        if supports_present && present.is_none() {
            present = Some(index);
        }
    }

    Ok(match (graphics, present) {
        (Some(graphics), Some(present)) => Some(QueueFamilyIndices { graphics, present }),
        _ => None,
    })
}

fn query_swapchain_support(
    device: vk::PhysicalDevice,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
) -> RenderResult<SwapchainSupport> {
    Ok(SwapchainSupport {
        capabilities: unsafe {
            surface_loader.get_physical_device_surface_capabilities(device, surface)?
        },
        formats: unsafe { surface_loader.get_physical_device_surface_formats(device, surface)? },
        present_modes: unsafe {
            surface_loader.get_physical_device_surface_present_modes(device, surface)?
        },
    })
}

fn log_swapchain_support(support: &SwapchainSupport) {
    let caps = support.capabilities;
    info!("Swapchain capabilities:");
    info!(
        "  min_images={} max_images={} current_extent={}x{} min_extent={}x{} max_extent={}x{} current_transform={:?}",
        caps.min_image_count,
        caps.max_image_count,
        caps.current_extent.width,
        caps.current_extent.height,
        caps.min_image_extent.width,
        caps.min_image_extent.height,
        caps.max_image_extent.width,
        caps.max_image_extent.height,
        caps.current_transform
    );
    info!("  formats={}", support.formats.len());
    for format in &support.formats {
        info!(
            "    format={:?} color_space={:?}",
            format.format, format.color_space
        );
    }
    info!("  present_modes={}", support.present_modes.len());
    for present_mode in &support.present_modes {
        info!("    {present_mode:?}");
    }
}

fn create_swapchain_bundle(context: SwapchainCreateContext<'_>) -> RenderResult<SwapchainBundle> {
    let surface_format = choose_surface_format(&context.support.formats);
    let present_mode = choose_present_mode(&context.support.present_modes);
    let extent = choose_swap_extent(&context.support.capabilities, context.window_size);
    let mut image_count = context.support.capabilities.min_image_count + 1;
    if context.support.capabilities.max_image_count > 0 {
        image_count = image_count.min(context.support.capabilities.max_image_count);
    }

    let unique_families = context.queue_family_indices.unique();
    let sharing_mode = if unique_families.len() > 1 {
        vk::SharingMode::CONCURRENT
    } else {
        vk::SharingMode::EXCLUSIVE
    };

    info!(
        "Creating swapchain: images={} format={:?} color_space={:?} present_mode={:?} extent={}x{} sharing={:?}",
        image_count,
        surface_format.format,
        surface_format.color_space,
        present_mode,
        extent.width,
        extent.height,
        sharing_mode
    );

    let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
        .surface(context.surface)
        .min_image_count(image_count)
        .image_format(surface_format.format)
        .image_color_space(surface_format.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(sharing_mode)
        .queue_family_indices(&unique_families)
        .pre_transform(context.support.capabilities.current_transform)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
        .present_mode(present_mode)
        .clipped(true);

    let swapchain = unsafe {
        context
            .swapchain_loader
            .create_swapchain(&swapchain_create_info, None)?
    };
    let images = unsafe { context.swapchain_loader.get_swapchain_images(swapchain)? };
    info!("Swapchain returned {} images", images.len());

    let image_views = images
        .iter()
        .map(|image| {
            create_image_view(
                context.device,
                *image,
                surface_format.format,
                vk::ImageAspectFlags::COLOR,
                1,
                "swapchain color image view",
            )
        })
        .collect::<RenderResult<Vec<_>>>()?;
    let depth_format = choose_depth_format(context.instance, context.physical_device)?;
    let memory_properties = unsafe {
        context
            .instance
            .get_physical_device_memory_properties(context.physical_device)
    };
    let depth_image = create_depth_image(context.device, &memory_properties, extent, depth_format)?;
    let render_pass = create_render_pass(context.device, surface_format.format, depth_format)?;
    let (pipeline_layout, graphics_pipeline) = create_triangle_pipeline(
        context.device,
        render_pass,
        extent,
        context.camera_descriptor_set_layout,
        context.texture_descriptor_set_layout,
    )?;
    let framebuffers = image_views
        .iter()
        .map(|image_view| {
            create_framebuffer(
                context.device,
                render_pass,
                *image_view,
                depth_image.view,
                extent,
            )
        })
        .collect::<RenderResult<Vec<_>>>()?;

    Ok(SwapchainBundle {
        swapchain,
        images,
        image_views,
        format: surface_format.format,
        extent,
        depth_image,
        depth_format,
        render_pass,
        pipeline_layout,
        graphics_pipeline,
        framebuffers,
    })
}

fn choose_surface_format(formats: &[vk::SurfaceFormatKHR]) -> vk::SurfaceFormatKHR {
    let selected = formats
        .iter()
        .copied()
        .find(|format| {
            format.format == vk::Format::B8G8R8A8_SRGB
                && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .unwrap_or_else(|| formats[0]);
    info!(
        "Selected surface format: {:?} / {:?}",
        selected.format, selected.color_space
    );
    selected
}

fn choose_present_mode(present_modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
    let selected = present_modes
        .iter()
        .copied()
        .find(|mode| *mode == vk::PresentModeKHR::MAILBOX)
        .unwrap_or(vk::PresentModeKHR::FIFO);
    info!("Selected present mode: {selected:?}");
    selected
}

fn choose_swap_extent(
    capabilities: &vk::SurfaceCapabilitiesKHR,
    window_size: PhysicalSize<u32>,
) -> vk::Extent2D {
    if capabilities.current_extent.width != u32::MAX {
        return capabilities.current_extent;
    }

    vk::Extent2D {
        width: window_size.width.clamp(
            capabilities.min_image_extent.width,
            capabilities.max_image_extent.width,
        ),
        height: window_size.height.clamp(
            capabilities.min_image_extent.height,
            capabilities.max_image_extent.height,
        ),
    }
}

fn choose_depth_format(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
) -> RenderResult<vk::Format> {
    let candidates = [
        vk::Format::D32_SFLOAT,
        vk::Format::D32_SFLOAT_S8_UINT,
        vk::Format::D24_UNORM_S8_UINT,
    ];
    info!("Choosing depth format from candidates: {candidates:?}");
    let selected = find_supported_format(
        instance,
        physical_device,
        &candidates,
        vk::ImageTiling::OPTIMAL,
        vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT,
    )?;
    info!("Selected depth format: {selected:?}");
    Ok(selected)
}

fn find_supported_format(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    candidates: &[vk::Format],
    tiling: vk::ImageTiling,
    features: vk::FormatFeatureFlags,
) -> RenderResult<vk::Format> {
    for candidate in candidates {
        let properties =
            unsafe { instance.get_physical_device_format_properties(physical_device, *candidate) };
        let supported = match tiling {
            vk::ImageTiling::LINEAR => properties.linear_tiling_features.contains(features),
            vk::ImageTiling::OPTIMAL => properties.optimal_tiling_features.contains(features),
            _ => false,
        };
        info!(
            "  format={candidate:?} linear={:?} optimal={:?} required={features:?} supported={supported}",
            properties.linear_tiling_features, properties.optimal_tiling_features
        );
        if supported {
            return Ok(*candidate);
        }
    }

    Err(RenderError::Message(format!(
        "no supported format found for tiling {tiling:?} and features {features:?}"
    )))
}

fn depth_aspect_mask(format: vk::Format) -> vk::ImageAspectFlags {
    let mut aspect = vk::ImageAspectFlags::DEPTH;
    if matches!(
        format,
        vk::Format::D16_UNORM_S8_UINT
            | vk::Format::D24_UNORM_S8_UINT
            | vk::Format::D32_SFLOAT_S8_UINT
    ) {
        aspect |= vk::ImageAspectFlags::STENCIL;
    }
    aspect
}

fn create_image_view(
    device: &Device,
    image: vk::Image,
    format: vk::Format,
    aspect_mask: vk::ImageAspectFlags,
    mip_levels: u32,
    label: &str,
) -> RenderResult<vk::ImageView> {
    info!(
        "Creating {label}: image={image:?} format={format:?} aspect={aspect_mask:?} mip_levels={mip_levels}"
    );
    let subresource_range = vk::ImageSubresourceRange::default()
        .aspect_mask(aspect_mask)
        .base_mip_level(0)
        .level_count(mip_levels)
        .base_array_layer(0)
        .layer_count(1);
    let create_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .components(vk::ComponentMapping::default())
        .subresource_range(subresource_range);
    Ok(unsafe { device.create_image_view(&create_info, None)? })
}

fn create_render_pass(
    device: &Device,
    color_format: vk::Format,
    depth_format: vk::Format,
) -> RenderResult<vk::RenderPass> {
    let color_attachment = vk::AttachmentDescription::default()
        .format(color_format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
    let color_attachment_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
    let depth_attachment = vk::AttachmentDescription::default()
        .format(depth_format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::DONT_CARE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
    let depth_attachment_ref = vk::AttachmentReference::default()
        .attachment(1)
        .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
    let color_attachments = [color_attachment_ref];
    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_attachments)
        .depth_stencil_attachment(&depth_attachment_ref);
    let dependency = vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
        )
        .src_access_mask(vk::AccessFlags::empty())
        .dst_stage_mask(
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
        )
        .dst_access_mask(
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
        );
    let attachments = [color_attachment, depth_attachment];
    let subpasses = [subpass];
    let dependencies = [dependency];
    let render_pass_info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
        .subpasses(&subpasses)
        .dependencies(&dependencies);
    info!("Creating render pass for color_format={color_format:?} depth_format={depth_format:?}");
    Ok(unsafe { device.create_render_pass(&render_pass_info, None)? })
}

fn create_triangle_pipeline(
    device: &Device,
    render_pass: vk::RenderPass,
    extent: vk::Extent2D,
    camera_descriptor_set_layout: vk::DescriptorSetLayout,
    texture_descriptor_set_layout: vk::DescriptorSetLayout,
) -> RenderResult<(vk::PipelineLayout, vk::Pipeline)> {
    info!(
        "Creating triangle graphics pipeline for extent {}x{}",
        extent.width, extent.height
    );
    info!(
        "Embedded triangle shaders: vertex={} bytes, fragment={} bytes",
        TRIANGLE_VERTEX_SHADER.len(),
        TRIANGLE_FRAGMENT_SHADER.len()
    );

    let vertex_shader = create_shader_module(device, TRIANGLE_VERTEX_SHADER, "triangle.vert")?;
    let fragment_shader = create_shader_module(device, TRIANGLE_FRAGMENT_SHADER, "triangle.frag")?;
    let entry_point = c"main";
    let shader_stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex_shader)
            .name(entry_point),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment_shader)
            .name(entry_point),
    ];

    let vertex_binding_descriptions = [Vertex::binding_description()];
    let vertex_attribute_descriptions = Vertex::attribute_descriptions();
    info!(
        "Triangle vertex layout: bindings={vertex_binding_descriptions:?} attributes={vertex_attribute_descriptions:?}"
    );
    let vertex_input_info = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&vertex_binding_descriptions)
        .vertex_attribute_descriptions(&vertex_attribute_descriptions);
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
        .primitive_restart_enable(false);
    let viewport = vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    let scissor = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent,
    };
    let viewports = [viewport];
    let scissors = [scissor];
    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewports(&viewports)
        .scissors(&scissors);
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .depth_clamp_enable(false)
        .rasterizer_discard_enable(false)
        .polygon_mode(vk::PolygonMode::FILL)
        .line_width(1.0)
        .cull_mode(vk::CullModeFlags::BACK)
        .front_face(TRIANGLE_FRONT_FACE)
        .depth_bias_enable(false);
    info!(
        "Rasterizer configured: polygon_mode=FILL cull_mode=BACK front_face={TRIANGLE_FRONT_FACE:?} depth_clamp=false depth_bias=false"
    );
    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .sample_shading_enable(false)
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::LESS)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);
    info!("Depth testing enabled: compare=LESS write=true bounds=false stencil=false");
    let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(
            vk::ColorComponentFlags::R
                | vk::ColorComponentFlags::G
                | vk::ColorComponentFlags::B
                | vk::ColorComponentFlags::A,
        )
        .blend_enable(false);
    let color_blend_attachments = [color_blend_attachment];
    let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
        .logic_op_enable(false)
        .attachments(&color_blend_attachments);
    let descriptor_set_layouts = [camera_descriptor_set_layout, texture_descriptor_set_layout];
    let push_constant_ranges = [vk::PushConstantRange::default()
        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
        .offset(0)
        .size(size_of::<ObjectPushConstants>() as u32)];
    info!(
        "Object push constants enabled: stages=VERTEX|FRAGMENT size={} bytes",
        size_of::<ObjectPushConstants>()
    );
    let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&descriptor_set_layouts)
        .push_constant_ranges(&push_constant_ranges);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&pipeline_layout_info, None)? };
    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
        .vertex_input_state(&vertex_input_info)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisampling)
        .depth_stencil_state(&depth_stencil)
        .color_blend_state(&color_blending)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let pipeline_result = unsafe {
        device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
    };

    unsafe {
        device.destroy_shader_module(fragment_shader, None);
        device.destroy_shader_module(vertex_shader, None);
    }

    match pipeline_result {
        Ok(mut pipelines) => {
            let pipeline = pipelines.pop().ok_or_else(|| {
                RenderError::Message("Vulkan returned no graphics pipeline".to_string())
            })?;
            info!("Triangle graphics pipeline created");
            Ok((pipeline_layout, pipeline))
        }
        Err((pipelines, error)) => {
            unsafe {
                for pipeline in pipelines {
                    device.destroy_pipeline(pipeline, None);
                }
                device.destroy_pipeline_layout(pipeline_layout, None);
            }
            Err(error.into())
        }
    }
}

fn create_shader_module(
    device: &Device,
    bytes: &[u8],
    label: &str,
) -> RenderResult<vk::ShaderModule> {
    info!("Creating shader module {label} from {} bytes", bytes.len());
    let mut cursor = Cursor::new(bytes);
    let code = ash::util::read_spv(&mut cursor)
        .map_err(|error| RenderError::Message(format!("failed to read {label} SPIR-V: {error}")))?;
    let create_info = vk::ShaderModuleCreateInfo::default().code(&code);
    Ok(unsafe { device.create_shader_module(&create_info, None)? })
}

fn create_depth_image(
    device: &Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    extent: vk::Extent2D,
    format: vk::Format,
) -> RenderResult<GpuImage> {
    info!(
        "Creating depth image: format={format:?} extent={}x{} aspect={:?}",
        extent.width,
        extent.height,
        depth_aspect_mask(format)
    );
    let mut image = create_image(
        device,
        memory_properties,
        ImageCreateParams {
            width: extent.width,
            height: extent.height,
            mip_levels: 1,
            format,
            tiling: vk::ImageTiling::OPTIMAL,
            usage: vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            required_properties: vk::MemoryPropertyFlags::DEVICE_LOCAL,
            label: "swapchain depth image",
        },
    )?;

    let view = match create_image_view(
        device,
        image.image,
        format,
        depth_aspect_mask(format),
        1,
        "swapchain depth image view",
    ) {
        Ok(view) => view,
        Err(error) => {
            destroy_gpu_image(device, &mut image, "swapchain depth image");
            return Err(error);
        }
    };
    image.view = view;
    info!(
        "Depth image ready: image={:?} view={:?}",
        image.image, image.view
    );
    Ok(image)
}

struct ImageCreateParams {
    width: u32,
    height: u32,
    mip_levels: u32,
    format: vk::Format,
    tiling: vk::ImageTiling,
    usage: vk::ImageUsageFlags,
    required_properties: vk::MemoryPropertyFlags,
    label: &'static str,
}

fn create_image(
    device: &Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    params: ImageCreateParams,
) -> RenderResult<GpuImage> {
    info!(
        "Creating {}: {}x{} mip_levels={} format={:?} tiling={:?} usage={:?} required_memory={:?}",
        params.label,
        params.width,
        params.height,
        params.mip_levels,
        params.format,
        params.tiling,
        params.usage,
        params.required_properties
    );
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width: params.width,
            height: params.height,
            depth: 1,
        })
        .mip_levels(params.mip_levels)
        .array_layers(1)
        .format(params.format)
        .tiling(params.tiling)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(params.usage)
        .samples(vk::SampleCountFlags::TYPE_1)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let image = unsafe { device.create_image(&image_info, None)? };
    let requirements = unsafe { device.get_image_memory_requirements(image) };
    info!(
        "  {} memory requirements: size={} alignment={} type_bits=0x{:x}",
        params.label, requirements.size, requirements.alignment, requirements.memory_type_bits
    );

    let memory_type_index = match find_memory_type(
        memory_properties,
        requirements.memory_type_bits,
        params.required_properties,
    ) {
        Some(index) => index,
        None => {
            unsafe {
                device.destroy_image(image, None);
            }
            return Err(RenderError::Message(format!(
                "no compatible memory type found for {}",
                params.label
            )));
        }
    };
    info!(
        "  {} selected memory_type_index={memory_type_index}",
        params.label
    );

    let allocate_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let memory = match unsafe { device.allocate_memory(&allocate_info, None) } {
        Ok(memory) => memory,
        Err(error) => {
            unsafe {
                device.destroy_image(image, None);
            }
            return Err(error.into());
        }
    };

    if let Err(error) = unsafe { device.bind_image_memory(image, memory, 0) } {
        unsafe {
            device.free_memory(memory, None);
            device.destroy_image(image, None);
        }
        return Err(error.into());
    }

    info!(
        "{} created: image={image:?} memory={memory:?}",
        params.label
    );
    Ok(GpuImage {
        image,
        memory,
        view: vk::ImageView::null(),
        mip_levels: params.mip_levels,
    })
}

fn create_framebuffer(
    device: &Device,
    render_pass: vk::RenderPass,
    color_view: vk::ImageView,
    depth_view: vk::ImageView,
    extent: vk::Extent2D,
) -> RenderResult<vk::Framebuffer> {
    info!(
        "Creating framebuffer: color_view={color_view:?} depth_view={depth_view:?} extent={}x{}",
        extent.width, extent.height
    );
    let attachments = [color_view, depth_view];
    let framebuffer_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(&attachments)
        .width(extent.width)
        .height(extent.height)
        .layers(1);
    Ok(unsafe { device.create_framebuffer(&framebuffer_info, None)? })
}

fn allocate_command_buffers(
    device: &Device,
    command_pool: vk::CommandPool,
    count: usize,
) -> RenderResult<Vec<vk::CommandBuffer>> {
    info!("Allocating {count} command buffers");
    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(count as u32);
    Ok(unsafe { device.allocate_command_buffers(&allocate_info)? })
}

fn create_render_finished_semaphores(
    device: &Device,
    count: usize,
) -> RenderResult<Vec<vk::Semaphore>> {
    info!("Creating {count} render-finished semaphores, one per swapchain image");
    let create_info = vk::SemaphoreCreateInfo::default();
    (0..count)
        .map(|_| Ok(unsafe { device.create_semaphore(&create_info, None)? }))
        .collect()
}

fn create_camera_descriptor_set_layout(device: &Device) -> RenderResult<vk::DescriptorSetLayout> {
    info!("Creating camera descriptor set layout with binding 0 uniform buffer");
    let binding = vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::VERTEX);
    let bindings = [binding];
    let create_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    Ok(unsafe { device.create_descriptor_set_layout(&create_info, None)? })
}

fn create_texture_descriptor_set_layout(device: &Device) -> RenderResult<vk::DescriptorSetLayout> {
    info!("Creating texture descriptor set layout with binding 0 combined image sampler");
    let binding = vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::FRAGMENT);
    let bindings = [binding];
    let create_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    Ok(unsafe { device.create_descriptor_set_layout(&create_info, None)? })
}

fn create_camera_uniform_buffers(
    device: &Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    count: usize,
) -> RenderResult<Vec<GpuBuffer>> {
    let uniform_size = size_of::<CameraUniform>() as vk::DeviceSize;
    info!("Creating {count} camera uniform buffers of {uniform_size} bytes");
    (0..count)
        .map(|index| {
            create_buffer(
                device,
                memory_properties,
                uniform_size,
                vk::BufferUsageFlags::UNIFORM_BUFFER,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                match index {
                    0 => "camera uniform buffer[0]",
                    1 => "camera uniform buffer[1]",
                    2 => "camera uniform buffer[2]",
                    _ => "camera uniform buffer",
                },
            )
        })
        .collect()
}

fn create_camera_descriptor_sets(
    device: &Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
    uniform_buffers: &[GpuBuffer],
) -> RenderResult<(vk::DescriptorPool, Vec<vk::DescriptorSet>)> {
    info!(
        "Creating camera descriptor pool and {} descriptor sets",
        uniform_buffers.len()
    );
    let pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::UNIFORM_BUFFER)
        .descriptor_count(uniform_buffers.len() as u32);
    let pool_sizes = [pool_size];
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .pool_sizes(&pool_sizes)
        .max_sets(uniform_buffers.len() as u32);
    let descriptor_pool = unsafe { device.create_descriptor_pool(&pool_info, None)? };

    let layouts = vec![descriptor_set_layout; uniform_buffers.len()];
    let allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(&layouts);
    let descriptor_sets = match unsafe { device.allocate_descriptor_sets(&allocate_info) } {
        Ok(descriptor_sets) => descriptor_sets,
        Err(error) => {
            unsafe {
                device.destroy_descriptor_pool(descriptor_pool, None);
            }
            return Err(error.into());
        }
    };

    for (index, (descriptor_set, uniform_buffer)) in
        descriptor_sets.iter().zip(uniform_buffers).enumerate()
    {
        info!(
            "Writing camera descriptor set[{index}]: set={descriptor_set:?} buffer={:?} size={}",
            uniform_buffer.buffer, uniform_buffer.size
        );
        let buffer_info = [vk::DescriptorBufferInfo::default()
            .buffer(uniform_buffer.buffer)
            .offset(0)
            .range(size_of::<CameraUniform>() as vk::DeviceSize)];
        let descriptor_write = [vk::WriteDescriptorSet::default()
            .dst_set(*descriptor_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .buffer_info(&buffer_info)];
        unsafe {
            device.update_descriptor_sets(&descriptor_write, &[]);
        }
    }

    Ok((descriptor_pool, descriptor_sets))
}

fn create_gpu_material(
    context: BufferUploadContext<'_>,
    texture_descriptor_set_layout: vk::DescriptorSetLayout,
    texture_format_properties: vk::FormatProperties,
    material: &StaticMeshMaterialAsset,
    mesh_label: &str,
) -> RenderResult<GpuMaterial> {
    let texture_asset = material
        .base_color_texture
        .as_ref()
        .map_or_else(fallback_white_texture_asset, Clone::clone);
    info!(
        "{mesh_label} material: base_color_factor={:?} texture_present={} texture_name={:?} texture_size={}x{} bytes={}",
        material.base_color_factor,
        material.base_color_texture.is_some(),
        texture_asset.name,
        texture_asset.width,
        texture_asset.height,
        texture_asset.rgba.len()
    );
    let texture = create_gpu_texture(
        context,
        texture_descriptor_set_layout,
        texture_format_properties,
        &texture_asset,
        material.base_color_texture.is_none(),
    )?;

    Ok(GpuMaterial {
        base_color_factor: material.base_color_factor,
        texture,
    })
}

fn fallback_white_texture_asset() -> StaticMeshTextureAsset {
    StaticMeshTextureAsset {
        name: "FinalEngine fallback white texture".to_string(),
        width: 1,
        height: 1,
        rgba: vec![255, 255, 255, 255],
        sampler: StaticMeshTextureSampler::default(),
    }
}

fn create_gpu_texture(
    context: BufferUploadContext<'_>,
    texture_descriptor_set_layout: vk::DescriptorSetLayout,
    texture_format_properties: vk::FormatProperties,
    texture: &StaticMeshTextureAsset,
    is_fallback: bool,
) -> RenderResult<GpuTexture> {
    if texture.width == 0 || texture.height == 0 {
        return Err(RenderError::Message(format!(
            "texture {:?} has invalid dimensions {}x{}",
            texture.name, texture.width, texture.height
        )));
    }

    let expected_bytes = texture.width as usize * texture.height as usize * 4;
    if texture.rgba.len() != expected_bytes {
        return Err(RenderError::Message(format!(
            "texture {:?} expected {expected_bytes} RGBA bytes, got {}",
            texture.name,
            texture.rgba.len()
        )));
    }

    let requested_mip_levels = texture_mip_level_count(texture.width, texture.height);
    let mip_levels = if requested_mip_levels > 1
        && !texture_format_supports_linear_mipmap_generation(texture_format_properties)
    {
        warn!(
            "Texture {:?} requested {requested_mip_levels} mip levels, but R8G8B8A8_SRGB linear blit support is unavailable; using one mip level",
            texture.name
        );
        1
    } else {
        requested_mip_levels
    };
    info!(
        "Uploading texture {:?}: {}x{} rgba_bytes={} mip_levels={} sampler={:?} fallback={is_fallback}",
        texture.name,
        texture.width,
        texture.height,
        texture.rgba.len(),
        mip_levels,
        texture.sampler
    );
    let mut staging_buffer = create_buffer(
        context.device,
        context.memory_properties,
        texture.rgba.len() as vk::DeviceSize,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        "texture staging buffer",
    )?;
    if let Err(error) = write_buffer_data(
        context.device,
        &staging_buffer,
        texture.rgba.as_slice(),
        "texture staging buffer",
    ) {
        destroy_gpu_buffer(
            context.device,
            &mut staging_buffer,
            "texture staging buffer",
        );
        return Err(error);
    }

    let mut image = match create_image(
        context.device,
        context.memory_properties,
        ImageCreateParams {
            width: texture.width,
            height: texture.height,
            mip_levels,
            format: vk::Format::R8G8B8A8_SRGB,
            tiling: vk::ImageTiling::OPTIMAL,
            usage: vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::SAMPLED,
            required_properties: vk::MemoryPropertyFlags::DEVICE_LOCAL,
            label: "base color texture image",
        },
    ) {
        Ok(image) => image,
        Err(error) => {
            destroy_gpu_buffer(
                context.device,
                &mut staging_buffer,
                "texture staging buffer",
            );
            return Err(error);
        }
    };

    let upload_result = (|| {
        transition_image_layout(
            context.device,
            context.command_pool,
            context.graphics_queue,
            ImageLayoutTransition {
                image: image.image,
                old_layout: vk::ImageLayout::UNDEFINED,
                new_layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                mip_levels,
                label: "texture undefined -> transfer dst",
            },
        )?;
        copy_buffer_to_image(
            context.device,
            context.command_pool,
            context.graphics_queue,
            staging_buffer.buffer,
            image.image,
            vk::Extent2D {
                width: texture.width,
                height: texture.height,
            },
            "texture buffer-to-image upload",
        )?;
        generate_texture_mipmaps(
            context.device,
            context.command_pool,
            context.graphics_queue,
            image.image,
            vk::Extent2D {
                width: texture.width,
                height: texture.height,
            },
            mip_levels,
            "texture mipmap generation",
        )
    })();
    destroy_gpu_buffer(
        context.device,
        &mut staging_buffer,
        "texture staging buffer",
    );
    if let Err(error) = upload_result {
        destroy_gpu_image(context.device, &mut image, "base color texture image");
        return Err(error);
    }

    image.view = match create_image_view(
        context.device,
        image.image,
        vk::Format::R8G8B8A8_SRGB,
        vk::ImageAspectFlags::COLOR,
        image.mip_levels,
        "base color texture image view",
    ) {
        Ok(view) => view,
        Err(error) => {
            destroy_gpu_image(context.device, &mut image, "base color texture image");
            return Err(error);
        }
    };
    let sampler = match create_texture_sampler(context.device, texture.sampler, mip_levels) {
        Ok(sampler) => sampler,
        Err(error) => {
            destroy_gpu_image(context.device, &mut image, "base color texture image");
            return Err(error);
        }
    };
    let (descriptor_pool, descriptor_set) = match create_texture_descriptor_set(
        context.device,
        texture_descriptor_set_layout,
        image.view,
        sampler,
    ) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            unsafe {
                context.device.destroy_sampler(sampler, None);
            }
            destroy_gpu_image(context.device, &mut image, "base color texture image");
            return Err(error);
        }
    };

    info!(
        "Texture {:?} ready: image={:?} view={:?} sampler={sampler:?} descriptor_set={descriptor_set:?}",
        texture.name, image.image, image.view
    );
    Ok(GpuTexture {
        image,
        sampler,
        descriptor_pool,
        descriptor_set,
        width: texture.width,
        height: texture.height,
        mip_levels,
        is_fallback,
    })
}

fn texture_mip_level_count(width: u32, height: u32) -> u32 {
    width.max(height).ilog2() + 1
}

fn texture_format_supports_linear_mipmap_generation(properties: vk::FormatProperties) -> bool {
    let required = vk::FormatFeatureFlags::BLIT_SRC
        | vk::FormatFeatureFlags::BLIT_DST
        | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
    let supported = properties.optimal_tiling_features.contains(required);
    info!(
        "Texture mipmap format support for R8G8B8A8_SRGB: optimal_features={:?} required={required:?} supported={supported}",
        properties.optimal_tiling_features
    );
    supported
}

fn create_texture_sampler(
    device: &Device,
    sampler: StaticMeshTextureSampler,
    mip_levels: u32,
) -> RenderResult<vk::Sampler> {
    let mag_filter = vulkan_texture_filter(sampler.mag_filter);
    let min_filter = vulkan_texture_min_filter(sampler.min_filter);
    let mipmap_mode = vulkan_texture_mipmap_mode(sampler.min_filter);
    let address_mode_u = vulkan_texture_wrap(sampler.wrap_s);
    let address_mode_v = vulkan_texture_wrap(sampler.wrap_t);
    let max_lod = if sampler.min_filter.uses_mipmaps() {
        (mip_levels.saturating_sub(1)) as f32
    } else {
        0.0
    };
    info!(
        "Creating texture sampler: mag={mag_filter:?} min={min_filter:?} mipmap={mipmap_mode:?} wrap_u={address_mode_u:?} wrap_v={address_mode_v:?} max_lod={max_lod} anisotropy=false"
    );
    let create_info = vk::SamplerCreateInfo::default()
        .mag_filter(mag_filter)
        .min_filter(min_filter)
        .mipmap_mode(mipmap_mode)
        .address_mode_u(address_mode_u)
        .address_mode_v(address_mode_v)
        .address_mode_w(vk::SamplerAddressMode::REPEAT)
        .mip_lod_bias(0.0)
        .anisotropy_enable(false)
        .max_anisotropy(1.0)
        .compare_enable(false)
        .compare_op(vk::CompareOp::ALWAYS)
        .min_lod(0.0)
        .max_lod(max_lod)
        .border_color(vk::BorderColor::INT_OPAQUE_BLACK)
        .unnormalized_coordinates(false);
    Ok(unsafe { device.create_sampler(&create_info, None)? })
}

fn vulkan_texture_filter(filter: StaticMeshTextureFilter) -> vk::Filter {
    match filter {
        StaticMeshTextureFilter::Nearest => vk::Filter::NEAREST,
        StaticMeshTextureFilter::Linear => vk::Filter::LINEAR,
    }
}

fn vulkan_texture_min_filter(filter: StaticMeshTextureMinFilter) -> vk::Filter {
    match filter {
        StaticMeshTextureMinFilter::Nearest
        | StaticMeshTextureMinFilter::NearestMipmapNearest
        | StaticMeshTextureMinFilter::NearestMipmapLinear => vk::Filter::NEAREST,
        StaticMeshTextureMinFilter::Linear
        | StaticMeshTextureMinFilter::LinearMipmapNearest
        | StaticMeshTextureMinFilter::LinearMipmapLinear => vk::Filter::LINEAR,
    }
}

fn vulkan_texture_mipmap_mode(filter: StaticMeshTextureMinFilter) -> vk::SamplerMipmapMode {
    match filter {
        StaticMeshTextureMinFilter::NearestMipmapNearest
        | StaticMeshTextureMinFilter::LinearMipmapNearest => vk::SamplerMipmapMode::NEAREST,
        StaticMeshTextureMinFilter::Nearest
        | StaticMeshTextureMinFilter::Linear
        | StaticMeshTextureMinFilter::NearestMipmapLinear
        | StaticMeshTextureMinFilter::LinearMipmapLinear => vk::SamplerMipmapMode::LINEAR,
    }
}

fn vulkan_texture_wrap(wrap: StaticMeshTextureWrap) -> vk::SamplerAddressMode {
    match wrap {
        StaticMeshTextureWrap::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        StaticMeshTextureWrap::MirroredRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
        StaticMeshTextureWrap::Repeat => vk::SamplerAddressMode::REPEAT,
    }
}

fn create_texture_descriptor_set(
    device: &Device,
    descriptor_set_layout: vk::DescriptorSetLayout,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
) -> RenderResult<(vk::DescriptorPool, vk::DescriptorSet)> {
    info!(
        "Creating texture descriptor pool and set: image_view={image_view:?} sampler={sampler:?}"
    );
    let pool_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1);
    let pool_sizes = [pool_size];
    let pool_info = vk::DescriptorPoolCreateInfo::default()
        .pool_sizes(&pool_sizes)
        .max_sets(1);
    let descriptor_pool = unsafe { device.create_descriptor_pool(&pool_info, None)? };
    let layouts = [descriptor_set_layout];
    let allocate_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(&layouts);
    let descriptor_set = match unsafe { device.allocate_descriptor_sets(&allocate_info) } {
        Ok(mut descriptor_sets) => match descriptor_sets.pop() {
            Some(descriptor_set) => descriptor_set,
            None => {
                unsafe {
                    device.destroy_descriptor_pool(descriptor_pool, None);
                }
                return Err(RenderError::Message(
                    "Vulkan returned no texture descriptor set".to_string(),
                ));
            }
        },
        Err(error) => {
            unsafe {
                device.destroy_descriptor_pool(descriptor_pool, None);
            }
            return Err(error.into());
        }
    };
    let image_info = [vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .image_view(image_view)
        .sampler(sampler)];
    let descriptor_write = [vk::WriteDescriptorSet::default()
        .dst_set(descriptor_set)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .image_info(&image_info)];
    unsafe {
        device.update_descriptor_sets(&descriptor_write, &[]);
    }
    info!("Texture descriptor set written: set={descriptor_set:?}");
    Ok((descriptor_pool, descriptor_set))
}

fn create_gpu_render_objects(
    instance: &Instance,
    device: &Device,
    physical_device: vk::PhysicalDevice,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,
    texture_descriptor_set_layout: vk::DescriptorSetLayout,
    scene: &RenderScene,
) -> RenderResult<Vec<GpuRenderObject>> {
    info!(
        "Creating GPU render objects for {} submitted scene objects",
        scene.objects.len()
    );
    let mut render_objects = Vec::with_capacity(scene.objects.len());
    for (scene_object_index, object) in scene.objects.iter().enumerate() {
        info!(
            "Creating GPU render object[{scene_object_index}] name={:?} mesh={}",
            object.name,
            object.mesh.log_label()
        );
        let mesh = match create_gpu_mesh(
            instance,
            device,
            physical_device,
            command_pool,
            graphics_queue,
            texture_descriptor_set_layout,
            &object.mesh,
        ) {
            Ok(mesh) => mesh,
            Err(error) => {
                destroy_gpu_render_objects(device, &mut render_objects);
                return Err(error);
            }
        };
        render_objects.push(GpuRenderObject {
            scene_object_index,
            mesh,
        });
    }
    Ok(render_objects)
}

fn create_gpu_mesh(
    instance: &Instance,
    device: &Device,
    physical_device: vk::PhysicalDevice,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,
    texture_descriptor_set_layout: vk::DescriptorSetLayout,
    mesh: &RenderMesh,
) -> RenderResult<GpuMesh> {
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    let texture_format_properties = unsafe {
        instance.get_physical_device_format_properties(physical_device, vk::Format::R8G8B8A8_SRGB)
    };
    let geometry = geometry_for_mesh(mesh)?;
    let vertex_bytes = std::mem::size_of_val(geometry.vertices.as_ref()) as vk::DeviceSize;
    let index_bytes = std::mem::size_of_val(geometry.indices.as_ref()) as vk::DeviceSize;
    let mesh_label = mesh.log_label();
    info!(
        "Creating {mesh_label} GPU mesh: vertices={} vertex_stride={} vertex_bytes={vertex_bytes} indices={} index_stride={} index_bytes={index_bytes} index_type=UINT16",
        geometry.vertices.len(),
        size_of::<Vertex>(),
        geometry.indices.len(),
        size_of::<MeshIndex>()
    );

    let upload_context = BufferUploadContext {
        device,
        memory_properties: &memory_properties,
        command_pool,
        graphics_queue,
    };
    let vertex_buffer = create_uploaded_buffer(
        upload_context,
        geometry.vertices.as_ref(),
        vk::BufferUsageFlags::VERTEX_BUFFER,
        BufferUploadLabels {
            final_label: "mesh vertex buffer",
            staging_label: "mesh vertex staging buffer",
            upload_label: "mesh vertex upload",
        },
    )?;
    let index_buffer = match create_uploaded_buffer(
        upload_context,
        geometry.indices.as_ref(),
        vk::BufferUsageFlags::INDEX_BUFFER,
        BufferUploadLabels {
            final_label: "mesh index buffer",
            staging_label: "mesh index staging buffer",
            upload_label: "mesh index upload",
        },
    ) {
        Ok(index_buffer) => index_buffer,
        Err(error) => {
            let mut vertex_buffer = vertex_buffer;
            destroy_gpu_buffer(device, &mut vertex_buffer, "mesh vertex buffer");
            return Err(error);
        }
    };
    let material = match create_gpu_material(
        upload_context,
        texture_descriptor_set_layout,
        texture_format_properties,
        geometry.material.as_ref(),
        &mesh_label,
    ) {
        Ok(material) => material,
        Err(error) => {
            let mut index_buffer = index_buffer;
            let mut vertex_buffer = vertex_buffer;
            destroy_gpu_buffer(device, &mut index_buffer, "mesh index buffer");
            destroy_gpu_buffer(device, &mut vertex_buffer, "mesh vertex buffer");
            return Err(error);
        }
    };

    info!("{mesh_label} GPU mesh uploaded and ready");
    Ok(GpuMesh {
        vertex_buffer,
        index_buffer,
        index_count: geometry.indices.len() as u32,
        material,
    })
}

fn geometry_for_mesh(mesh: &RenderMesh) -> RenderResult<MeshGeometry<'_>> {
    match mesh {
        RenderMesh::DemoCube => Ok(MeshGeometry {
            vertices: Cow::Borrowed(&DEMO_CUBE_VERTICES),
            indices: Cow::Borrowed(&DEMO_CUBE_INDICES),
            material: Cow::Owned(StaticMeshMaterialAsset::default()),
        }),
        RenderMesh::DemoGroundPlane => Ok(MeshGeometry {
            vertices: Cow::Borrowed(&DEMO_GROUND_PLANE_VERTICES),
            indices: Cow::Borrowed(&DEMO_GROUND_PLANE_INDICES),
            material: Cow::Owned(StaticMeshMaterialAsset::default()),
        }),
        RenderMesh::Static(mesh) => {
            let vertices = mesh
                .vertices
                .iter()
                .map(|vertex| Vertex {
                    position: vertex.position,
                    normal: vertex.normal,
                    color: vertex.color,
                    texcoord: vertex.texcoord,
                })
                .collect::<Vec<_>>();
            Ok(MeshGeometry {
                vertices: Cow::Owned(vertices),
                indices: Cow::Borrowed(&mesh.indices),
                material: Cow::Borrowed(&mesh.material),
            })
        }
    }
}

fn scene_bounds_for_objects(objects: &[RenderObject]) -> Option<SceneBounds> {
    let mut bounds = SceneBounds::empty();
    for object in objects {
        let geometry = geometry_for_mesh(&object.mesh).ok()?;
        let model = object.model_matrix(0.0);
        for vertex in geometry.vertices.iter() {
            bounds.include_point(transform_point(model, vertex.position));
        }
    }
    bounds.is_valid().then_some(bounds)
}

fn transform_point(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * point[0] + matrix[1][0] * point[1] + matrix[2][0] * point[2] + matrix[3][0],
        matrix[0][1] * point[0] + matrix[1][1] * point[1] + matrix[2][1] * point[2] + matrix[3][1],
        matrix[0][2] * point[0] + matrix[1][2] * point[1] + matrix[2][2] * point[2] + matrix[3][2],
    ]
}

#[derive(Clone, Copy)]
struct BufferUploadContext<'a> {
    device: &'a Device,
    memory_properties: &'a vk::PhysicalDeviceMemoryProperties,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,
}

#[derive(Clone, Copy)]
struct BufferUploadLabels<'a> {
    final_label: &'a str,
    staging_label: &'a str,
    upload_label: &'a str,
}

fn create_uploaded_buffer<T>(
    context: BufferUploadContext<'_>,
    data: &[T],
    usage: vk::BufferUsageFlags,
    labels: BufferUploadLabels<'_>,
) -> RenderResult<GpuBuffer> {
    let byte_len = std::mem::size_of_val(data) as vk::DeviceSize;
    let mut staging_buffer = create_buffer(
        context.device,
        context.memory_properties,
        byte_len,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        labels.staging_label,
    )?;

    if let Err(error) =
        write_buffer_data(context.device, &staging_buffer, data, labels.staging_label)
    {
        destroy_gpu_buffer(context.device, &mut staging_buffer, labels.staging_label);
        return Err(error);
    }

    let mut gpu_buffer = match create_buffer(
        context.device,
        context.memory_properties,
        byte_len,
        vk::BufferUsageFlags::TRANSFER_DST | usage,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
        labels.final_label,
    ) {
        Ok(buffer) => buffer,
        Err(error) => {
            destroy_gpu_buffer(context.device, &mut staging_buffer, labels.staging_label);
            return Err(error);
        }
    };

    if let Err(error) = copy_buffer(
        context.device,
        context.command_pool,
        context.graphics_queue,
        staging_buffer.buffer,
        gpu_buffer.buffer,
        byte_len,
        labels.upload_label,
    ) {
        destroy_gpu_buffer(context.device, &mut gpu_buffer, labels.final_label);
        destroy_gpu_buffer(context.device, &mut staging_buffer, labels.staging_label);
        return Err(error);
    }

    destroy_gpu_buffer(context.device, &mut staging_buffer, labels.staging_label);
    Ok(gpu_buffer)
}

fn create_buffer(
    device: &Device,
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
    required_properties: vk::MemoryPropertyFlags,
    label: &str,
) -> RenderResult<GpuBuffer> {
    info!("Creating {label}: size={size} usage={usage:?} required_memory={required_properties:?}");
    let buffer_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = unsafe { device.create_buffer(&buffer_info, None)? };
    let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
    info!(
        "  {label} memory requirements: size={} alignment={} type_bits=0x{:x}",
        requirements.size, requirements.alignment, requirements.memory_type_bits
    );
    let memory_type_index = match find_memory_type(
        memory_properties,
        requirements.memory_type_bits,
        required_properties,
    ) {
        Some(index) => index,
        None => {
            unsafe {
                device.destroy_buffer(buffer, None);
            }
            return Err(RenderError::Message(format!(
                "no compatible memory type found for {label}"
            )));
        }
    };
    info!("  {label} selected memory_type_index={memory_type_index}");

    let allocate_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let memory = match unsafe { device.allocate_memory(&allocate_info, None) } {
        Ok(memory) => memory,
        Err(error) => {
            unsafe {
                device.destroy_buffer(buffer, None);
            }
            return Err(error.into());
        }
    };

    if let Err(error) = unsafe { device.bind_buffer_memory(buffer, memory, 0) } {
        unsafe {
            device.free_memory(memory, None);
            device.destroy_buffer(buffer, None);
        }
        return Err(error.into());
    }

    info!("{label} created: buffer={buffer:?} memory={memory:?}");
    Ok(GpuBuffer {
        buffer,
        memory,
        size,
    })
}

fn find_memory_type(
    memory_properties: &vk::PhysicalDeviceMemoryProperties,
    type_filter: u32,
    required_properties: vk::MemoryPropertyFlags,
) -> Option<u32> {
    info!("Searching memory types: type_filter=0x{type_filter:x} required={required_properties:?}");
    for index in 0..memory_properties.memory_type_count {
        let supported = (type_filter & (1 << index)) != 0;
        let memory_type = memory_properties.memory_types[index as usize];
        let has_properties = memory_type.property_flags.contains(required_properties);
        info!(
            "  memory_type[{index}]: heap={} flags={:?} supported={} compatible={}",
            memory_type.heap_index, memory_type.property_flags, supported, has_properties
        );
        if supported && has_properties {
            return Some(index);
        }
    }
    None
}

fn write_buffer_data<T>(
    device: &Device,
    buffer: &GpuBuffer,
    data: &[T],
    label: &str,
) -> RenderResult<()> {
    let byte_len = std::mem::size_of_val(data) as vk::DeviceSize;
    info!("Writing {byte_len} bytes into {label}");
    if byte_len > buffer.size {
        return Err(RenderError::Message(format!(
            "{label} write of {byte_len} bytes exceeds buffer size {}",
            buffer.size
        )));
    }

    unsafe {
        let mapped = device.map_memory(buffer.memory, 0, byte_len, vk::MemoryMapFlags::empty())?;
        std::ptr::copy_nonoverlapping(
            data.as_ptr().cast::<u8>(),
            mapped.cast::<u8>(),
            byte_len as usize,
        );
        device.unmap_memory(buffer.memory);
    }
    info!("{label} write complete");
    Ok(())
}

fn copy_buffer(
    device: &Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    source: vk::Buffer,
    destination: vk::Buffer,
    size: vk::DeviceSize,
    label: &str,
) -> RenderResult<()> {
    info!("Copying {size} bytes for {label}: source={source:?} destination={destination:?}");
    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_pool(command_pool)
        .command_buffer_count(1);
    let command_buffer = unsafe { device.allocate_command_buffers(&allocate_info)?[0] };
    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    let region = vk::BufferCopy::default().size(size);
    let regions = [region];
    unsafe {
        device.begin_command_buffer(command_buffer, &begin_info)?;
        device.cmd_copy_buffer(command_buffer, source, destination, &regions);
        device.end_command_buffer(command_buffer)?;

        let command_buffers = [command_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        device.queue_submit(queue, &[submit_info], vk::Fence::null())?;
        device.queue_wait_idle(queue)?;
        device.free_command_buffers(command_pool, &[command_buffer]);
    }
    info!("{label} copy complete");
    Ok(())
}

fn begin_one_time_commands(
    device: &Device,
    command_pool: vk::CommandPool,
    label: &str,
) -> RenderResult<vk::CommandBuffer> {
    info!("Beginning one-time command buffer for {label}");
    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_pool(command_pool)
        .command_buffer_count(1);
    let command_buffer = unsafe { device.allocate_command_buffers(&allocate_info)?[0] };
    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    unsafe {
        device.begin_command_buffer(command_buffer, &begin_info)?;
    }
    Ok(command_buffer)
}

fn end_one_time_commands(
    device: &Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    command_buffer: vk::CommandBuffer,
    label: &str,
) -> RenderResult<()> {
    info!("Submitting one-time command buffer for {label}: command_buffer={command_buffer:?}");
    unsafe {
        device.end_command_buffer(command_buffer)?;
        let command_buffers = [command_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        device.queue_submit(queue, &[submit_info], vk::Fence::null())?;
        device.queue_wait_idle(queue)?;
        device.free_command_buffers(command_pool, &[command_buffer]);
    }
    info!("One-time command buffer complete for {label}");
    Ok(())
}

#[derive(Clone, Copy)]
struct ImageLayoutTransition<'a> {
    image: vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
    mip_levels: u32,
    label: &'a str,
}

fn transition_image_layout(
    device: &Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    transition: ImageLayoutTransition<'_>,
) -> RenderResult<()> {
    info!(
        "Transitioning image layout for {}: image={:?} {:?}->{:?} mip_levels={}",
        transition.label,
        transition.image,
        transition.old_layout,
        transition.new_layout,
        transition.mip_levels
    );
    let (src_access_mask, dst_access_mask, src_stage_mask, dst_stage_mask) =
        match (transition.old_layout, transition.new_layout) {
            (vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL) => (
                vk::AccessFlags::empty(),
                vk::AccessFlags::TRANSFER_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
            ),
            (vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL) => (
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
            ),
            _ => {
                return Err(RenderError::Message(format!(
                    "unsupported image layout transition for {}: {:?}->{:?}",
                    transition.label, transition.old_layout, transition.new_layout
                )));
            }
        };

    let command_buffer = begin_one_time_commands(device, command_pool, transition.label)?;
    let subresource_range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(transition.mip_levels)
        .base_array_layer(0)
        .layer_count(1);
    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(transition.old_layout)
        .new_layout(transition.new_layout)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(transition.image)
        .subresource_range(subresource_range)
        .src_access_mask(src_access_mask)
        .dst_access_mask(dst_access_mask);
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            src_stage_mask,
            dst_stage_mask,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }
    end_one_time_commands(
        device,
        command_pool,
        queue,
        command_buffer,
        transition.label,
    )
}

fn generate_texture_mipmaps(
    device: &Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    image: vk::Image,
    extent: vk::Extent2D,
    mip_levels: u32,
    label: &str,
) -> RenderResult<()> {
    if mip_levels <= 1 {
        return transition_image_layout(
            device,
            command_pool,
            queue,
            ImageLayoutTransition {
                image,
                old_layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                new_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                mip_levels: 1,
                label,
            },
        );
    }

    info!(
        "Generating {mip_levels} texture mip levels for {label}: image={image:?} base_extent={}x{}",
        extent.width, extent.height
    );
    let command_buffer = begin_one_time_commands(device, command_pool, label)?;
    let mut mip_width = extent.width as i32;
    let mut mip_height = extent.height as i32;

    for mip_level in 1..mip_levels {
        let previous_mip_level = mip_level - 1;
        let previous_to_src_barrier = vk::ImageMemoryBarrier::default()
            .image(image)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(previous_mip_level)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        unsafe {
            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[previous_to_src_barrier],
            );
        }

        let next_width = if mip_width > 1 { mip_width / 2 } else { 1 };
        let next_height = if mip_height > 1 { mip_height / 2 } else { 1 };
        let source_subresource = vk::ImageSubresourceLayers::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .mip_level(previous_mip_level)
            .base_array_layer(0)
            .layer_count(1);
        let destination_subresource = vk::ImageSubresourceLayers::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .mip_level(mip_level)
            .base_array_layer(0)
            .layer_count(1);
        let blit = vk::ImageBlit::default()
            .src_offsets([
                vk::Offset3D { x: 0, y: 0, z: 0 },
                vk::Offset3D {
                    x: mip_width,
                    y: mip_height,
                    z: 1,
                },
            ])
            .src_subresource(source_subresource)
            .dst_offsets([
                vk::Offset3D { x: 0, y: 0, z: 0 },
                vk::Offset3D {
                    x: next_width,
                    y: next_height,
                    z: 1,
                },
            ])
            .dst_subresource(destination_subresource);
        unsafe {
            device.cmd_blit_image(
                command_buffer,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[blit],
                vk::Filter::LINEAR,
            );
        }

        let previous_to_shader_barrier = vk::ImageMemoryBarrier::default()
            .image(image)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(previous_mip_level)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        unsafe {
            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[previous_to_shader_barrier],
            );
        }

        info!(
            "  generated mip_level={mip_level} extent={}x{} from previous_extent={}x{}",
            next_width, next_height, mip_width, mip_height
        );
        mip_width = next_width;
        mip_height = next_height;
    }

    let final_mip_barrier = vk::ImageMemoryBarrier::default()
        .image(image)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(mip_levels - 1)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1),
        );
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[final_mip_barrier],
        );
    }

    end_one_time_commands(device, command_pool, queue, command_buffer, label)
}

fn copy_buffer_to_image(
    device: &Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    buffer: vk::Buffer,
    image: vk::Image,
    extent: vk::Extent2D,
    label: &str,
) -> RenderResult<()> {
    info!(
        "Copying texture buffer to image for {label}: buffer={buffer:?} image={image:?} extent={}x{}",
        extent.width, extent.height
    );
    let command_buffer = begin_one_time_commands(device, command_pool, label)?;
    let subresource = vk::ImageSubresourceLayers::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .mip_level(0)
        .base_array_layer(0)
        .layer_count(1);
    let region = vk::BufferImageCopy::default()
        .buffer_offset(0)
        .buffer_row_length(0)
        .buffer_image_height(0)
        .image_subresource(subresource)
        .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
        .image_extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        });
    unsafe {
        device.cmd_copy_buffer_to_image(
            command_buffer,
            buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[region],
        );
    }
    end_one_time_commands(device, command_pool, queue, command_buffer, label)
}

fn destroy_gpu_buffer(device: &Device, buffer: &mut GpuBuffer, label: &str) {
    unsafe {
        if buffer.buffer != vk::Buffer::null() {
            info!("Destroying {label} buffer {:?}", buffer.buffer);
            device.destroy_buffer(buffer.buffer, None);
            buffer.buffer = vk::Buffer::null();
        }
        if buffer.memory != vk::DeviceMemory::null() {
            info!("Freeing {label} memory {:?}", buffer.memory);
            device.free_memory(buffer.memory, None);
            buffer.memory = vk::DeviceMemory::null();
        }
        buffer.size = 0;
    }
}

fn destroy_gpu_mesh(device: &Device, mesh: &mut GpuMesh) {
    destroy_gpu_material(device, &mut mesh.material);
    destroy_gpu_buffer(device, &mut mesh.index_buffer, "mesh index buffer");
    destroy_gpu_buffer(device, &mut mesh.vertex_buffer, "mesh vertex buffer");
    mesh.index_count = 0;
}

fn destroy_gpu_material(device: &Device, material: &mut GpuMaterial) {
    destroy_gpu_texture(device, &mut material.texture);
    material.base_color_factor = [1.0, 1.0, 1.0, 1.0];
}

fn destroy_gpu_texture(device: &Device, texture: &mut GpuTexture) {
    unsafe {
        if texture.descriptor_pool != vk::DescriptorPool::null() {
            info!(
                "Destroying texture descriptor pool {:?} (descriptor_set={:?})",
                texture.descriptor_pool, texture.descriptor_set
            );
            device.destroy_descriptor_pool(texture.descriptor_pool, None);
            texture.descriptor_pool = vk::DescriptorPool::null();
            texture.descriptor_set = vk::DescriptorSet::null();
        }
        if texture.sampler != vk::Sampler::null() {
            info!("Destroying texture sampler {:?}", texture.sampler);
            device.destroy_sampler(texture.sampler, None);
            texture.sampler = vk::Sampler::null();
        }
    }
    destroy_gpu_image(device, &mut texture.image, "base color texture image");
    texture.width = 0;
    texture.height = 0;
    texture.mip_levels = 0;
    texture.is_fallback = true;
}

fn destroy_gpu_render_objects(device: &Device, render_objects: &mut Vec<GpuRenderObject>) {
    for render_object in render_objects.iter_mut() {
        destroy_gpu_mesh(device, &mut render_object.mesh);
    }
    render_objects.clear();
}

fn destroy_gpu_image(device: &Device, image: &mut GpuImage, label: &str) {
    unsafe {
        if image.view != vk::ImageView::null() {
            info!("Destroying {label} view {:?}", image.view);
            device.destroy_image_view(image.view, None);
            image.view = vk::ImageView::null();
        }
        if image.image != vk::Image::null() {
            info!("Destroying {label} image {:?}", image.image);
            device.destroy_image(image.image, None);
            image.image = vk::Image::null();
        }
        if image.memory != vk::DeviceMemory::null() {
            info!("Freeing {label} memory {:?}", image.memory);
            device.free_memory(image.memory, None);
            image.memory = vk::DeviceMemory::null();
        }
        image.mip_levels = 0;
    }
}

fn update_camera_uniform(
    device: &Device,
    uniform_buffer: &GpuBuffer,
    extent: vk::Extent2D,
    camera: RenderCamera,
) -> RenderResult<()> {
    let uniform = camera_uniform_for_extent(extent, camera);
    unsafe {
        let mapped = device.map_memory(
            uniform_buffer.memory,
            0,
            size_of::<CameraUniform>() as vk::DeviceSize,
            vk::MemoryMapFlags::empty(),
        )?;
        std::ptr::copy_nonoverlapping(
            (&uniform as *const CameraUniform).cast::<u8>(),
            mapped.cast::<u8>(),
            size_of::<CameraUniform>(),
        );
        device.unmap_memory(uniform_buffer.memory);
    }
    Ok(())
}

fn camera_uniform_for_extent(extent: vk::Extent2D, camera: RenderCamera) -> CameraUniform {
    let aspect = if extent.height == 0 {
        1.0
    } else {
        extent.width as f32 / extent.height as f32
    };
    let projection = perspective_vulkan_rh(
        camera.vertical_fov_degrees.to_radians(),
        aspect,
        camera.near,
        camera.far,
    );
    let view = look_at_rh(camera.eye, camera.target, camera.up);
    CameraUniform {
        view_projection: multiply_mat4(projection, view),
    }
}

fn translation_matrix(translation: [f32; 3]) -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [translation[0], translation[1], translation[2], 1.0],
    ]
}

fn scale_matrix(scale: [f32; 3]) -> [[f32; 4]; 4] {
    [
        [scale[0], 0.0, 0.0, 0.0],
        [0.0, scale[1], 0.0, 0.0],
        [0.0, 0.0, scale[2], 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn rotation_z(radians: f32) -> [[f32; 4]; 4] {
    let (sin, cos) = radians.sin_cos();
    [
        [cos, sin, 0.0, 0.0],
        [-sin, cos, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn perspective_vulkan_rh(fovy_radians: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fovy_radians * 0.5).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, -f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, (far * near) / (near - far), 0.0],
    ]
}

fn look_at_rh(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let forward = normalize3(sub3(eye, target));
    let right = normalize3(cross3(up, forward));
    let up = cross3(forward, right);

    [
        [right[0], up[0], forward[0], 0.0],
        [right[1], up[1], forward[1], 0.0],
        [right[2], up[2], forward[2], 0.0],
        [-dot3(right, eye), -dot3(up, eye), -dot3(forward, eye), 1.0],
    ]
}

fn rotation_x(radians: f32) -> [[f32; 4]; 4] {
    let (sin, cos) = radians.sin_cos();
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, cos, sin, 0.0],
        [0.0, -sin, cos, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn rotation_y(radians: f32) -> [[f32; 4]; 4] {
    let (sin, cos) = radians.sin_cos();
    [
        [cos, 0.0, -sin, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [sin, 0.0, cos, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn multiply_mat4(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            result[column][row] = left[0][row] * right[column][0]
                + left[1][row] * right[column][1]
                + left[2][row] * right[column][2]
                + left[3][row] * right[column][3];
        }
    }
    result
}

fn sub3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn add3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn mul3(value: [f32; 3], scalar: f32) -> [f32; 3] {
    [value[0] * scalar, value[1] * scalar, value[2] * scalar]
}

fn length3(value: [f32; 3]) -> f32 {
    dot3(value, value).sqrt()
}

fn dot3(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn rotate_vector_around_axis(vector: [f32; 3], axis: [f32; 3], radians: f32) -> [f32; 3] {
    let axis = normalize3(axis);
    if length3(axis) <= f32::EPSILON {
        return vector;
    }
    let (sin, cos) = radians.sin_cos();
    add3(
        add3(mul3(vector, cos), mul3(cross3(axis, vector), sin)),
        mul3(axis, dot3(axis, vector) * (1.0 - cos)),
    )
}

fn normalize3(value: [f32; 3]) -> [f32; 3] {
    let length = length3(value);
    if length <= f32::EPSILON {
        return [0.0, 0.0, 0.0];
    }
    [value[0] / length, value[1] / length, value[2] / length]
}

struct RenderDraw {
    vertex_buffer: vk::Buffer,
    index_buffer: vk::Buffer,
    index_count: u32,
    texture_descriptor_set: vk::DescriptorSet,
    object_constants: ObjectPushConstants,
}

struct RenderCommandParams<'a> {
    command_buffer: vk::CommandBuffer,
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,
    pipeline_layout: vk::PipelineLayout,
    graphics_pipeline: vk::Pipeline,
    render_draws: &'a [RenderDraw],
    camera_descriptor_set: vk::DescriptorSet,
    extent: vk::Extent2D,
    clear_color: vk::ClearValue,
}

fn record_render_commands(device: &Device, params: RenderCommandParams<'_>) -> RenderResult<()> {
    let begin_info = vk::CommandBufferBeginInfo::default();
    let render_area = vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: params.extent,
    };
    let clear_values = [
        params.clear_color,
        vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        },
    ];
    let render_pass_info = vk::RenderPassBeginInfo::default()
        .render_pass(params.render_pass)
        .framebuffer(params.framebuffer)
        .render_area(render_area)
        .clear_values(&clear_values);

    unsafe {
        device.begin_command_buffer(params.command_buffer, &begin_info)?;
        device.cmd_begin_render_pass(
            params.command_buffer,
            &render_pass_info,
            vk::SubpassContents::INLINE,
        );
        device.cmd_bind_pipeline(
            params.command_buffer,
            vk::PipelineBindPoint::GRAPHICS,
            params.graphics_pipeline,
        );
        device.cmd_bind_descriptor_sets(
            params.command_buffer,
            vk::PipelineBindPoint::GRAPHICS,
            params.pipeline_layout,
            0,
            &[params.camera_descriptor_set],
            &[],
        );
        for draw in params.render_draws {
            let push_constant_bytes = std::slice::from_raw_parts(
                (&draw.object_constants as *const ObjectPushConstants).cast::<u8>(),
                size_of::<ObjectPushConstants>(),
            );
            device.cmd_push_constants(
                params.command_buffer,
                params.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                push_constant_bytes,
            );
            device.cmd_bind_descriptor_sets(
                params.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                params.pipeline_layout,
                1,
                &[draw.texture_descriptor_set],
                &[],
            );
            device.cmd_bind_vertex_buffers(params.command_buffer, 0, &[draw.vertex_buffer], &[0]);
            device.cmd_bind_index_buffer(
                params.command_buffer,
                draw.index_buffer,
                0,
                vk::IndexType::UINT16,
            );
            device.cmd_draw_indexed(params.command_buffer, draw.index_count, 1, 0, 0, 0);
        }
        device.cmd_end_render_pass(params.command_buffer);
        device.end_command_buffer(params.command_buffer)?;
    }
    Ok(())
}

fn debug_messenger_create_info() -> vk::DebugUtilsMessengerCreateInfoEXT<'static> {
    vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE
                | vk::DebugUtilsMessageSeverityFlagsEXT::INFO
                | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .pfn_user_callback(Some(vulkan_debug_callback))
}

unsafe extern "system" fn vulkan_debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user_data: *mut c_void,
) -> vk::Bool32 {
    let message = if callback_data.is_null() {
        "<null callback data>".into()
    } else {
        let raw_message = unsafe { (*callback_data).p_message };
        if raw_message.is_null() {
            "<null message>".into()
        } else {
            unsafe { CStr::from_ptr(raw_message) }.to_string_lossy()
        }
    };

    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        error!("Vulkan validation [{message_type:?}]: {message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        warn!("Vulkan validation [{message_type:?}]: {message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) {
        info!("Vulkan validation [{message_type:?}]: {message}");
    } else {
        debug!("Vulkan validation [{message_type:?}]: {message}");
    }

    vk::FALSE
}

fn vk_string(raw: &[i8]) -> String {
    unsafe { CStr::from_ptr(raw.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_assets::StaticMeshVertex;

    fn unique_test_scene_directory() -> PathBuf {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "finalengine_test_scene_paths_{}_{}",
            std::process::id(),
            unique_suffix
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn renderer_uses_raster_fallback_without_rt() {
        let config = RendererConfig::vulkan_high_end();

        assert_eq!(config.choose_path(false), RendererPath::Raster);
    }

    #[test]
    fn renderer_prefers_rt_when_available() {
        let config = RendererConfig::vulkan_high_end();

        assert_eq!(config.choose_path(true), RendererPath::RayTracing);
    }

    #[test]
    fn swap_extent_clamps_to_surface_limits() {
        let capabilities = vk::SurfaceCapabilitiesKHR {
            current_extent: vk::Extent2D {
                width: u32::MAX,
                height: u32::MAX,
            },
            min_image_extent: vk::Extent2D {
                width: 640,
                height: 480,
            },
            max_image_extent: vk::Extent2D {
                width: 1920,
                height: 1080,
            },
            ..Default::default()
        };

        let extent = choose_swap_extent(&capabilities, PhysicalSize::new(4096, 64));

        assert_eq!(extent.width, 1920);
        assert_eq!(extent.height, 480);
    }

    #[test]
    fn triangle_vertex_layout_matches_shader_inputs() {
        let binding = Vertex::binding_description();
        let attributes = Vertex::attribute_descriptions();

        assert_eq!(binding.stride, 44);
        assert_eq!(attributes[0].location, 0);
        assert_eq!(attributes[0].format, vk::Format::R32G32B32_SFLOAT);
        assert_eq!(attributes[0].offset, 0);
        assert_eq!(attributes[1].location, 1);
        assert_eq!(attributes[1].format, vk::Format::R32G32B32_SFLOAT);
        assert_eq!(attributes[1].offset, 12);
        assert_eq!(attributes[2].location, 2);
        assert_eq!(attributes[2].format, vk::Format::R32G32B32_SFLOAT);
        assert_eq!(attributes[2].offset, 24);
        assert_eq!(attributes[3].location, 3);
        assert_eq!(attributes[3].format, vk::Format::R32G32_SFLOAT);
        assert_eq!(attributes[3].offset, 36);
    }

    #[test]
    fn camera_uniform_is_std140_mat4_sized() {
        assert_eq!(size_of::<CameraUniform>(), 64);
    }

    #[test]
    fn object_push_constants_include_model_and_base_color_factor() {
        assert_eq!(size_of::<ObjectPushConstants>(), 80);
    }

    #[test]
    fn texture_mip_level_count_uses_largest_dimension() {
        assert_eq!(texture_mip_level_count(1, 1), 1);
        assert_eq!(texture_mip_level_count(2, 1), 2);
        assert_eq!(texture_mip_level_count(256, 128), 9);
    }

    #[test]
    fn gltf_sampler_settings_map_to_vulkan_values() {
        assert_eq!(
            vulkan_texture_filter(StaticMeshTextureFilter::Nearest),
            vk::Filter::NEAREST
        );
        assert_eq!(
            vulkan_texture_min_filter(StaticMeshTextureMinFilter::LinearMipmapNearest),
            vk::Filter::LINEAR
        );
        assert_eq!(
            vulkan_texture_mipmap_mode(StaticMeshTextureMinFilter::LinearMipmapNearest),
            vk::SamplerMipmapMode::NEAREST
        );
        assert_eq!(
            vulkan_texture_wrap(StaticMeshTextureWrap::MirroredRepeat),
            vk::SamplerAddressMode::MIRRORED_REPEAT
        );
    }

    #[test]
    fn camera_projection_changes_with_aspect_ratio() {
        let wide = camera_uniform_for_extent(
            vk::Extent2D {
                width: 1920,
                height: 1080,
            },
            RenderCamera::default(),
        );
        let square = camera_uniform_for_extent(
            vk::Extent2D {
                width: 1024,
                height: 1024,
            },
            RenderCamera::default(),
        );

        assert_ne!(wide.view_projection[0][0], square.view_projection[0][0]);
    }

    #[test]
    fn camera_orbit_preserves_target_distance() {
        let mut camera = RenderCamera::default();
        let original_target = camera.target;
        let original_distance = length3(sub3(camera.eye, camera.target));

        camera.orbit([80.0, -35.0]);

        let new_distance = length3(sub3(camera.eye, camera.target));
        assert_eq!(camera.target, original_target);
        assert_ne!(camera.eye, RenderCamera::default().eye);
        assert!((new_distance - original_distance).abs() < 0.0001);
    }

    #[test]
    fn camera_pan_moves_eye_and_target_without_changing_distance() {
        let mut camera = RenderCamera::default();
        let original_eye = camera.eye;
        let original_target = camera.target;
        let original_distance = length3(sub3(camera.eye, camera.target));

        camera.pan(
            [120.0, -60.0],
            PhysicalSize {
                width: 1280,
                height: 720,
            },
        );

        let new_distance = length3(sub3(camera.eye, camera.target));
        assert_ne!(camera.eye, original_eye);
        assert_ne!(camera.target, original_target);
        assert!((new_distance - original_distance).abs() < 0.0001);
    }

    #[test]
    fn camera_zoom_moves_eye_toward_target() {
        let mut camera = RenderCamera::default();
        let original_distance = length3(sub3(camera.eye, camera.target));

        camera.zoom(1.0);

        let new_distance = length3(sub3(camera.eye, camera.target));
        assert!(new_distance < original_distance);
        assert!(camera.far >= 100.0);
    }

    #[test]
    fn default_render_scene_submits_cube_loaded_mesh_and_ground_plane() {
        let scene = RenderScene::default();
        let cube = scene.primary_object().unwrap();
        let loaded = &scene.objects[1];
        let ground = &scene.objects[2];

        assert_eq!(scene.objects.len(), 3);
        assert_eq!(cube.name, "Demo Cube");
        assert_eq!(cube.mesh, RenderMesh::DemoCube);
        assert_eq!(
            cube.animation.unwrap().rotation_degrees_per_second,
            [12.0, 45.0, 0.0]
        );
        assert_eq!(loaded.name, "Embedded GLTF Pyramid");
        let RenderMesh::Static(mesh) = &loaded.mesh else {
            panic!("loaded object should use a static mesh asset");
        };
        assert_eq!(mesh.vertices.len(), 5);
        assert_eq!(mesh.indices.len(), 18);
        assert_eq!(ground.name, "Ground Plane");
        assert_eq!(ground.mesh, RenderMesh::DemoGroundPlane);
        assert_eq!(ground.transform.translation, [0.0, -0.75, 0.0]);
        assert!(ground.animation.is_none());
    }

    #[test]
    fn default_test_scene_directory_points_to_assets_folder() {
        assert_eq!(
            RenderScene::default_test_scene_directory(),
            PathBuf::from("assets/test_scene")
        );
    }

    #[test]
    fn default_test_scene_paths_discovers_supported_files_in_stable_order() {
        let directory = unique_test_scene_directory();
        std::fs::write(directory.join("z_scene.gltf"), b"{}").unwrap();
        std::fs::write(directory.join("b_scene.glb"), b"").unwrap();
        std::fs::write(directory.join("A_scene.GLB"), b"").unwrap();
        std::fs::write(directory.join("notes.txt"), b"").unwrap();
        std::fs::create_dir(directory.join("nested.glb")).unwrap();

        assert_eq!(
            RenderScene::discover_test_scene_paths(&directory),
            vec![
                directory.join("A_scene.GLB"),
                directory.join("b_scene.glb"),
                directory.join("z_scene.gltf")
            ]
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn static_mesh_scene_asset_converts_to_render_scene_objects() {
        let mesh = StaticMeshAsset::demo_pyramid_from_embedded_gltf().unwrap();
        let scene = RenderScene::from_static_mesh_scene_asset(StaticMeshSceneAsset {
            name: "test scene".to_string(),
            meshes: vec![mesh],
        })
        .unwrap();

        assert_eq!(scene.objects.len(), 1);
        assert_eq!(scene.objects[0].name, "Embedded GLTF Pyramid");
        assert!(matches!(scene.objects[0].mesh, RenderMesh::Static(_)));
    }

    #[test]
    fn imported_static_mesh_scene_camera_frames_mesh_bounds() {
        let vertices = vec![
            StaticMeshVertex {
                position: [-10.0, -2.0, -4.0],
                normal: [0.0, 1.0, 0.0],
                color: [1.0, 1.0, 1.0],
                texcoord: [0.0, 0.0],
            },
            StaticMeshVertex {
                position: [10.0, -2.0, -4.0],
                normal: [0.0, 1.0, 0.0],
                color: [1.0, 1.0, 1.0],
                texcoord: [1.0, 0.0],
            },
            StaticMeshVertex {
                position: [0.0, 8.0, 4.0],
                normal: [0.0, 1.0, 0.0],
                color: [1.0, 1.0, 1.0],
                texcoord: [0.5, 1.0],
            },
        ];
        let mesh = StaticMeshAsset::new("large triangle", vertices, vec![0, 1, 2]).unwrap();
        let scene = RenderScene::from_static_mesh_scene_asset(StaticMeshSceneAsset {
            name: "large scene".to_string(),
            meshes: vec![mesh],
        })
        .unwrap();
        let expected_center = [0.0, 3.0, 0.0];
        let radius = length3([20.0, 10.0, 8.0]) * 0.5;
        let distance = length3(sub3(scene.camera.eye, scene.camera.target));
        let minimum_distance =
            radius / (scene.camera.vertical_fov_degrees.to_radians() * 0.5).sin();

        assert_eq!(scene.camera.target, expected_center);
        assert!(distance > minimum_distance);
        assert!(scene.camera.near > 0.0);
        assert!(scene.camera.far > distance + radius);
    }

    #[test]
    fn scene_bounds_apply_object_transform() {
        let object = RenderObject {
            name: "offset cube".to_string(),
            transform: RenderTransform {
                translation: [4.0, 5.0, 6.0],
                rotation_euler_degrees: [0.0, 0.0, 0.0],
                scale: [2.0, 3.0, 4.0],
            },
            model_matrix: None,
            animation: None,
            mesh: RenderMesh::DemoCube,
        };

        let bounds = scene_bounds_for_objects(&[object]).unwrap();

        assert_eq!(bounds.min, [3.0, 3.5, 4.0]);
        assert_eq!(bounds.max, [5.0, 6.5, 8.0]);
    }

    #[test]
    fn demo_mesh_indices_reference_existing_vertices() {
        for mesh in [RenderMesh::DemoCube, RenderMesh::DemoGroundPlane] {
            let geometry = geometry_for_mesh(&mesh).unwrap();
            let vertex_count = geometry.vertices.len();

            assert_eq!(geometry.indices.len() % 3, 0);
            assert!(
                geometry
                    .indices
                    .iter()
                    .all(|index| usize::from(*index) < vertex_count)
            );
        }
    }

    #[test]
    fn demo_mesh_vertices_have_unit_normals() {
        for mesh in [RenderMesh::DemoCube, RenderMesh::DemoGroundPlane] {
            let geometry = geometry_for_mesh(&mesh).unwrap();
            for vertex in geometry.vertices.iter() {
                let length_squared = dot3(vertex.normal, vertex.normal);
                assert!(
                    (length_squared - 1.0).abs() < 0.0001,
                    "{mesh:?} vertex normal {:?} is not unit length",
                    vertex.normal
                );
            }
        }
    }

    #[test]
    fn rasterizer_front_face_matches_vulkan_y_corrected_projection() {
        assert_eq!(TRIANGLE_FRONT_FACE, vk::FrontFace::COUNTER_CLOCKWISE);
    }

    #[test]
    fn demo_cube_indices_use_counter_clockwise_outward_winding() {
        let geometry = geometry_for_mesh(&RenderMesh::DemoCube).unwrap();
        let expected_normals = [
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [-1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, -1.0, 0.0],
        ];

        for (triangle, expected_normal) in geometry.indices.chunks_exact(3).zip(expected_normals) {
            let a = geometry.vertices[usize::from(triangle[0])].position;
            let b = geometry.vertices[usize::from(triangle[1])].position;
            let c = geometry.vertices[usize::from(triangle[2])].position;
            let normal = cross3(sub3(b, a), sub3(c, a));

            assert!(
                dot3(normal, expected_normal) > 0.0,
                "triangle {triangle:?} is not wound counter-clockwise for outward normal {expected_normal:?}"
            );
        }
    }

    #[test]
    fn animated_demo_cube_changes_model_over_time() {
        let scene = RenderScene::demo_cube();
        let cube = scene.primary_object().unwrap();
        let first = cube.transform.model_matrix(0.0, cube.animation);
        let later = cube.transform.model_matrix(1.0, cube.animation);

        assert_ne!(first, later);
    }

    #[test]
    fn empty_render_scene_is_rejected() {
        let scene = RenderScene {
            camera: RenderCamera::default(),
            objects: Vec::new(),
        };

        assert!(scene.primary_object().is_err());
    }

    #[test]
    fn render_transform_model_matrix_applies_translation() {
        let matrix = RenderTransform {
            translation: [1.0, 2.0, 3.0],
            ..Default::default()
        }
        .model_matrix(0.0, None);

        assert_eq!(matrix[3], [1.0, 2.0, 3.0, 1.0]);
    }

    #[test]
    fn depth_aspect_includes_stencil_only_for_stencil_formats() {
        assert_eq!(
            depth_aspect_mask(vk::Format::D32_SFLOAT),
            vk::ImageAspectFlags::DEPTH
        );
        assert_eq!(
            depth_aspect_mask(vk::Format::D24_UNORM_S8_UINT),
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
        );
    }
}
