use std::collections::BTreeMap;
use std::fs;
use std::mem::size_of;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("asset path is already registered: {0}")]
    DuplicatePath(PathBuf),
    #[error("asset id is already registered: {0}")]
    DuplicateId(AssetId),
    #[error("failed to serialize asset metadata: {0}")]
    Serialize(#[from] ron::Error),
    #[error("failed to deserialize asset metadata: {0}")]
    Deserialize(#[from] ron::error::SpannedError),
    #[error("asset IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse glTF JSON: {0}")]
    GltfJson(#[from] serde_json::Error),
    #[error("failed to decode glTF image: {0}")]
    GltfImage(#[from] image::ImageError),
    #[error("glTF asset is missing {0}")]
    MissingGltfField(&'static str),
    #[error("unsupported glTF feature: {0}")]
    UnsupportedGltfFeature(String),
    #[error("invalid glTF mesh data: {0}")]
    InvalidGltfMesh(String),
    #[error("invalid GLB file: {0}")]
    InvalidGlb(String),
    #[error("failed to decode embedded glTF buffer: {0}")]
    DecodeGltfBuffer(#[from] base64::DecodeError),
}

pub type AssetResult<T> = Result<T, AssetError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AssetId(Uuid);

impl AssetId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for AssetId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AssetId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Scene,
    Prefab,
    Mesh,
    Texture,
    Material,
    Shader,
    Script,
    Audio,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetMetadata {
    pub schema_version: u32,
    pub id: AssetId,
    pub kind: AssetKind,
    pub source_path: PathBuf,
    pub imported_path: Option<PathBuf>,
    pub dependencies: Vec<AssetId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticMeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    pub texcoord: [f32; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticMeshAsset {
    pub name: String,
    pub vertices: Vec<StaticMeshVertex>,
    pub indices: Vec<u16>,
    pub transform: StaticMeshTransform,
    pub material: StaticMeshMaterialAsset,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticMeshMaterialAsset {
    pub base_color_factor: [f32; 4],
    pub base_color_texture: Option<StaticMeshTextureAsset>,
}

impl Default for StaticMeshMaterialAsset {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            base_color_texture: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticMeshTextureAsset {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub sampler: StaticMeshTextureSampler,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StaticMeshTextureSampler {
    pub mag_filter: StaticMeshTextureFilter,
    pub min_filter: StaticMeshTextureMinFilter,
    pub wrap_s: StaticMeshTextureWrap,
    pub wrap_t: StaticMeshTextureWrap,
}

impl Default for StaticMeshTextureSampler {
    fn default() -> Self {
        Self {
            mag_filter: StaticMeshTextureFilter::Linear,
            min_filter: StaticMeshTextureMinFilter::LinearMipmapLinear,
            wrap_s: StaticMeshTextureWrap::Repeat,
            wrap_t: StaticMeshTextureWrap::Repeat,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StaticMeshTextureFilter {
    Nearest,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StaticMeshTextureMinFilter {
    Nearest,
    Linear,
    NearestMipmapNearest,
    LinearMipmapNearest,
    NearestMipmapLinear,
    LinearMipmapLinear,
}

impl StaticMeshTextureMinFilter {
    pub fn uses_mipmaps(self) -> bool {
        matches!(
            self,
            Self::NearestMipmapNearest
                | Self::LinearMipmapNearest
                | Self::NearestMipmapLinear
                | Self::LinearMipmapLinear
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StaticMeshTextureWrap {
    ClampToEdge,
    MirroredRepeat,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StaticMeshTransform {
    pub matrix: [[f32; 4]; 4],
}

impl StaticMeshTransform {
    pub const IDENTITY: Self = Self {
        matrix: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
}

impl Default for StaticMeshTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticMeshSceneAsset {
    pub name: String,
    pub meshes: Vec<StaticMeshAsset>,
}

impl StaticMeshSceneAsset {
    pub fn from_gltf_path(path: impl AsRef<Path>) -> AssetResult<Self> {
        let path = path.as_ref();
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let (document, buffers) = if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("glb"))
        {
            load_glb_document_and_buffers(path)?
        } else {
            let source = fs::read_to_string(path)?;
            let document: Value = serde_json::from_str(&source)?;
            let buffers = decode_gltf_buffers(&document, Some(base_dir), None)?;
            (document, buffers)
        };
        let meshes = static_meshes_from_gltf_document(&document, &buffers, Some(base_dir))?;
        if meshes.is_empty() {
            return Err(AssetError::InvalidGltfMesh(format!(
                "{} does not contain any supported mesh primitives",
                path.display()
            )));
        }
        Ok(Self {
            name: path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("glTF Test Scene")
                .to_string(),
            meshes,
        })
    }
}

impl StaticMeshAsset {
    pub fn new(
        name: impl Into<String>,
        vertices: Vec<StaticMeshVertex>,
        indices: Vec<u16>,
    ) -> AssetResult<Self> {
        Self::with_material(name, vertices, indices, StaticMeshMaterialAsset::default())
    }

    pub fn with_material(
        name: impl Into<String>,
        vertices: Vec<StaticMeshVertex>,
        indices: Vec<u16>,
        material: StaticMeshMaterialAsset,
    ) -> AssetResult<Self> {
        if vertices.is_empty() {
            return Err(AssetError::InvalidGltfMesh(
                "static mesh must contain at least one vertex".to_string(),
            ));
        }

        if indices.is_empty() {
            return Err(AssetError::InvalidGltfMesh(
                "static mesh must contain at least one index".to_string(),
            ));
        }

        if !indices.len().is_multiple_of(3) {
            return Err(AssetError::InvalidGltfMesh(
                "static mesh index count must be a multiple of three".to_string(),
            ));
        }

        if let Some(index) = indices
            .iter()
            .find(|index| usize::from(**index) >= vertices.len())
        {
            return Err(AssetError::InvalidGltfMesh(format!(
                "index {index} references only {} vertices",
                vertices.len()
            )));
        }

        Ok(Self {
            name: name.into(),
            vertices,
            indices,
            transform: StaticMeshTransform::default(),
            material,
        })
    }

    pub fn from_embedded_gltf_json(source: &str) -> AssetResult<Self> {
        let document: Value = serde_json::from_str(source)?;
        let buffers = decode_gltf_buffers(&document, None, None)?;
        static_meshes_from_gltf_document(&document, &buffers, None)?
            .into_iter()
            .next()
            .ok_or(AssetError::MissingGltfField("meshes[0]"))
    }

    pub fn demo_pyramid_from_embedded_gltf() -> AssetResult<Self> {
        let positions = [
            [-0.6, -0.5, 0.6],
            [0.6, -0.5, 0.6],
            [0.6, -0.5, -0.6],
            [-0.6, -0.5, -0.6],
            [0.0, 0.75, 0.0],
        ];
        let colors = [
            [0.95, 0.50, 0.18],
            [0.95, 0.78, 0.20],
            [0.32, 0.72, 0.95],
            [0.45, 0.35, 0.95],
            [0.95, 0.92, 0.78],
        ];
        let indices = [0_u16, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0, 4, 3, 2, 1, 1, 0, 3];

        let mut buffer = Vec::new();
        let position_offset = buffer.len();
        write_vec3_f32(&mut buffer, &positions);
        let color_offset = buffer.len();
        write_vec3_f32(&mut buffer, &colors);
        let index_offset = buffer.len();
        write_u16(&mut buffer, &indices);
        let encoded = BASE64_STANDARD.encode(&buffer);

        let source = format!(
            r#"{{
  "asset": {{ "version": "2.0", "generator": "FinalEngine demo static mesh" }},
  "buffers": [
    {{ "byteLength": {buffer_len}, "uri": "data:application/octet-stream;base64,{encoded}" }}
  ],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": {position_offset}, "byteLength": {position_bytes} }},
    {{ "buffer": 0, "byteOffset": {color_offset}, "byteLength": {color_bytes} }},
    {{ "buffer": 0, "byteOffset": {index_offset}, "byteLength": {index_bytes} }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": {vertex_count}, "type": "VEC3" }},
    {{ "bufferView": 1, "componentType": 5126, "count": {vertex_count}, "type": "VEC3" }},
    {{ "bufferView": 2, "componentType": 5123, "count": {index_count}, "type": "SCALAR" }}
  ],
  "meshes": [
    {{
      "name": "Embedded GLTF Pyramid",
      "primitives": [
        {{ "attributes": {{ "POSITION": 0, "COLOR_0": 1 }}, "indices": 2, "mode": 4 }}
      ]
    }}
  ]
}}"#,
            buffer_len = buffer.len(),
            position_bytes = positions.len() * 3 * size_of::<f32>(),
            color_bytes = colors.len() * 3 * size_of::<f32>(),
            index_bytes = indices.len() * size_of::<u16>(),
            vertex_count = positions.len(),
            index_count = indices.len(),
        );

        Self::from_embedded_gltf_json(&source)
    }
}

impl AssetMetadata {
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn new(kind: AssetKind, source_path: impl Into<PathBuf>) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            id: AssetId::new(),
            kind,
            source_path: source_path.into(),
            imported_path: None,
            dependencies: Vec::new(),
        }
    }

    pub fn to_ron(&self) -> AssetResult<String> {
        Ok(ron::ser::to_string_pretty(
            self,
            ron::ser::PrettyConfig::default(),
        )?)
    }

    pub fn from_ron(source: &str) -> AssetResult<Self> {
        Ok(ron::from_str(source)?)
    }
}

fn static_meshes_from_gltf_document(
    document: &Value,
    buffers: &[Vec<u8>],
    base_dir: Option<&Path>,
) -> AssetResult<Vec<StaticMeshAsset>> {
    let mut static_meshes = Vec::new();

    if let Some(scene_nodes) = default_scene_node_indices(document)? {
        for node_index in scene_nodes {
            collect_static_meshes_from_node(
                document,
                buffers,
                base_dir,
                node_index,
                StaticMeshTransform::IDENTITY.matrix,
                &mut static_meshes,
            )?;
        }

        return Ok(static_meshes);
    }

    let meshes = document
        .get("meshes")
        .and_then(Value::as_array)
        .ok_or(AssetError::MissingGltfField("meshes"))?;
    for (mesh_index, mesh) in meshes.iter().enumerate() {
        let mesh_name = mesh
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("glTF Mesh {mesh_index}"));
        let primitives = mesh
            .get("primitives")
            .and_then(Value::as_array)
            .ok_or(AssetError::MissingGltfField("meshes[].primitives"))?;

        for (primitive_index, primitive) in primitives.iter().enumerate() {
            let name = if primitives.len() == 1 {
                mesh_name.clone()
            } else {
                format!("{mesh_name} primitive {primitive_index}")
            };
            static_meshes.push(static_mesh_from_gltf_primitive(
                document, buffers, base_dir, primitive, name,
            )?);
        }
    }

    Ok(static_meshes)
}

fn default_scene_node_indices(document: &Value) -> AssetResult<Option<Vec<usize>>> {
    let Some(scenes) = document.get("scenes").and_then(Value::as_array) else {
        return Ok(None);
    };
    if scenes.is_empty() {
        return Ok(None);
    }

    let scene_index = document.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize;
    let scene = scenes.get(scene_index).ok_or_else(|| {
        AssetError::InvalidGltfMesh(format!("default scene index {scene_index} is out of range"))
    })?;
    let Some(nodes) = scene.get("nodes").and_then(Value::as_array) else {
        return Ok(None);
    };

    nodes
        .iter()
        .map(|node| {
            node.as_u64().map(|index| index as usize).ok_or_else(|| {
                AssetError::InvalidGltfMesh(
                    "scene node index must be an unsigned integer".to_string(),
                )
            })
        })
        .collect::<AssetResult<Vec<_>>>()
        .map(Some)
}

fn collect_static_meshes_from_node(
    document: &Value,
    buffers: &[Vec<u8>],
    base_dir: Option<&Path>,
    node_index: usize,
    parent_transform: [[f32; 4]; 4],
    static_meshes: &mut Vec<StaticMeshAsset>,
) -> AssetResult<()> {
    let node = gltf_array_item(document, "nodes", node_index)?;
    let node_transform = multiply_mat4(parent_transform, gltf_node_transform(node)?);
    if let Some(mesh_index) = node.get("mesh").and_then(Value::as_u64) {
        let node_name = node.get("name").and_then(Value::as_str).map(str::to_string);
        push_static_mesh_primitives(
            document,
            buffers,
            base_dir,
            mesh_index as usize,
            node_name.as_deref(),
            node_transform,
            static_meshes,
        )?;
    }

    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            let child_index = child.as_u64().ok_or_else(|| {
                AssetError::InvalidGltfMesh(
                    "node child index must be an unsigned integer".to_string(),
                )
            })? as usize;
            collect_static_meshes_from_node(
                document,
                buffers,
                base_dir,
                child_index,
                node_transform,
                static_meshes,
            )?;
        }
    }

    Ok(())
}

fn push_static_mesh_primitives(
    document: &Value,
    buffers: &[Vec<u8>],
    base_dir: Option<&Path>,
    mesh_index: usize,
    node_name: Option<&str>,
    transform: [[f32; 4]; 4],
    static_meshes: &mut Vec<StaticMeshAsset>,
) -> AssetResult<()> {
    let mesh = gltf_array_item(document, "meshes", mesh_index)?;
    let mesh_name = mesh
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("glTF Mesh {mesh_index}"));
    let primitives = mesh
        .get("primitives")
        .and_then(Value::as_array)
        .ok_or(AssetError::MissingGltfField("meshes[].primitives"))?;
    let instance_name = node_name.unwrap_or(&mesh_name);

    for (primitive_index, primitive) in primitives.iter().enumerate() {
        let primitive_name = if primitives.len() == 1 {
            instance_name.to_string()
        } else {
            format!("{instance_name} primitive {primitive_index}")
        };
        let mut mesh = static_mesh_from_gltf_primitive(
            document,
            buffers,
            base_dir,
            primitive,
            primitive_name,
        )?;
        mesh.transform = StaticMeshTransform { matrix: transform };
        static_meshes.push(mesh);
    }

    Ok(())
}

fn static_mesh_from_gltf_primitive(
    document: &Value,
    buffers: &[Vec<u8>],
    base_dir: Option<&Path>,
    primitive: &Value,
    name: String,
) -> AssetResult<StaticMeshAsset> {
    let attributes = primitive
        .get("attributes")
        .and_then(Value::as_object)
        .ok_or(AssetError::MissingGltfField(
            "meshes[].primitives[].attributes",
        ))?;
    let position_accessor = attributes
        .get("POSITION")
        .and_then(Value::as_u64)
        .ok_or(AssetError::MissingGltfField("attributes.POSITION"))?
        as usize;
    let normal_accessor = attributes.get("NORMAL").and_then(Value::as_u64);
    let color_accessor = attributes.get("COLOR_0").and_then(Value::as_u64);
    let texcoord_accessor = attributes.get("TEXCOORD_0").and_then(Value::as_u64);
    let index_accessor =
        primitive
            .get("indices")
            .and_then(Value::as_u64)
            .ok_or(AssetError::MissingGltfField(
                "meshes[].primitives[].indices",
            ))? as usize;

    if primitive
        .get("mode")
        .and_then(Value::as_u64)
        .is_some_and(|mode| mode != 4)
    {
        return Err(AssetError::UnsupportedGltfFeature(
            "only TRIANGLES primitive mode is supported".to_string(),
        ));
    }

    let positions = read_accessor_vec3(document, buffers, position_accessor, "POSITION")?;
    let material_base_color = gltf_primitive_base_color_factor(document, primitive)?;
    let base_color_texture =
        gltf_primitive_base_color_texture(document, buffers, base_dir, primitive)?;
    let normals = match normal_accessor {
        Some(accessor) => Some(read_accessor_vec3(
            document,
            buffers,
            accessor as usize,
            "NORMAL",
        )?),
        None => None,
    };
    let colors = match color_accessor {
        Some(accessor) => Some(read_accessor_color(
            document,
            buffers,
            accessor as usize,
            "COLOR_0",
        )?),
        None => None,
    };
    let texcoords = match texcoord_accessor {
        Some(accessor) => Some(read_accessor_vec2(
            document,
            buffers,
            accessor as usize,
            "TEXCOORD_0",
        )?),
        None => None,
    };
    let indices = read_accessor_indices(document, buffers, index_accessor)?;

    let normals = match normals {
        Some(normals) => normals,
        None => generate_smooth_normals(&positions, &indices)?,
    };
    let material_affects_color = material_base_color.is_some() || base_color_texture.is_some();
    let colors = match colors {
        Some(colors) => colors,
        None if material_affects_color => vec![[1.0, 1.0, 1.0]; positions.len()],
        None => vec![[0.75, 0.75, 0.75]; positions.len()],
    };
    if texcoords
        .as_ref()
        .is_some_and(|texcoords| texcoords.len() != positions.len())
    {
        return Err(AssetError::InvalidGltfMesh(
            "POSITION and TEXCOORD_0 accessor counts must match".to_string(),
        ));
    }

    if normals.len() != positions.len() || colors.len() != positions.len() {
        return Err(AssetError::InvalidGltfMesh(
            "POSITION, NORMAL, and COLOR_0 accessor counts must match".to_string(),
        ));
    }

    let texcoords = texcoords.unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
    let vertices = positions
        .into_iter()
        .zip(normals)
        .zip(colors)
        .zip(texcoords)
        .map(|(((position, normal), color), texcoord)| StaticMeshVertex {
            position,
            normal,
            color,
            texcoord,
        })
        .collect();

    StaticMeshAsset::with_material(
        name,
        vertices,
        indices,
        StaticMeshMaterialAsset {
            base_color_factor: material_base_color.unwrap_or([1.0, 1.0, 1.0, 1.0]),
            base_color_texture,
        },
    )
}

fn gltf_primitive_base_color_factor(
    document: &Value,
    primitive: &Value,
) -> AssetResult<Option<[f32; 4]>> {
    let Some(material_index) = primitive.get("material").and_then(Value::as_u64) else {
        return Ok(None);
    };

    let material = gltf_array_item(document, "materials", material_index as usize)?;
    let Some(pbr) = material.get("pbrMetallicRoughness") else {
        return Ok(Some([1.0, 1.0, 1.0, 1.0]));
    };

    let base_color = gltf_optional_vec4(pbr, "baseColorFactor", [1.0, 1.0, 1.0, 1.0])?;
    Ok(Some(base_color))
}

fn gltf_primitive_base_color_texture(
    document: &Value,
    buffers: &[Vec<u8>],
    base_dir: Option<&Path>,
    primitive: &Value,
) -> AssetResult<Option<StaticMeshTextureAsset>> {
    let Some(material_index) = primitive.get("material").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let material = gltf_array_item(document, "materials", material_index as usize)?;
    let Some(texture_index) = material
        .get("pbrMetallicRoughness")
        .and_then(|pbr| pbr.get("baseColorTexture"))
        .and_then(|texture| texture.get("index"))
        .and_then(Value::as_u64)
    else {
        return Ok(None);
    };

    let texture = gltf_array_item(document, "textures", texture_index as usize)?;
    let sampler = gltf_texture_sampler(document, texture)?;
    let source_index = texture
        .get("source")
        .and_then(Value::as_u64)
        .ok_or(AssetError::MissingGltfField("textures[].source"))? as usize;
    let image_info = gltf_array_item(document, "images", source_index)?;
    let texture_name = image_info
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("glTF base color texture")
        .to_string();
    let image_bytes = gltf_image_bytes(document, buffers, base_dir, image_info)?;
    let image = image::load_from_memory(&image_bytes)?.to_rgba8();
    let (width, height) = image.dimensions();
    Ok(Some(StaticMeshTextureAsset {
        name: texture_name,
        width,
        height,
        rgba: image.into_raw(),
        sampler,
    }))
}

fn gltf_texture_sampler(
    document: &Value,
    texture: &Value,
) -> AssetResult<StaticMeshTextureSampler> {
    let Some(sampler_index) = texture.get("sampler").and_then(Value::as_u64) else {
        return Ok(StaticMeshTextureSampler::default());
    };
    let sampler = gltf_array_item(document, "samplers", sampler_index as usize)?;

    Ok(StaticMeshTextureSampler {
        mag_filter: sampler
            .get("magFilter")
            .and_then(Value::as_u64)
            .map(gltf_texture_filter)
            .transpose()?
            .unwrap_or(StaticMeshTextureSampler::default().mag_filter),
        min_filter: sampler
            .get("minFilter")
            .and_then(Value::as_u64)
            .map(gltf_texture_min_filter)
            .transpose()?
            .unwrap_or(StaticMeshTextureSampler::default().min_filter),
        wrap_s: sampler
            .get("wrapS")
            .and_then(Value::as_u64)
            .map(gltf_texture_wrap)
            .transpose()?
            .unwrap_or(StaticMeshTextureSampler::default().wrap_s),
        wrap_t: sampler
            .get("wrapT")
            .and_then(Value::as_u64)
            .map(gltf_texture_wrap)
            .transpose()?
            .unwrap_or(StaticMeshTextureSampler::default().wrap_t),
    })
}

fn gltf_texture_filter(value: u64) -> AssetResult<StaticMeshTextureFilter> {
    match value {
        9728 => Ok(StaticMeshTextureFilter::Nearest),
        9729 => Ok(StaticMeshTextureFilter::Linear),
        _ => Err(AssetError::UnsupportedGltfFeature(format!(
            "unsupported glTF texture filter value {value}"
        ))),
    }
}

fn gltf_texture_min_filter(value: u64) -> AssetResult<StaticMeshTextureMinFilter> {
    match value {
        9728 => Ok(StaticMeshTextureMinFilter::Nearest),
        9729 => Ok(StaticMeshTextureMinFilter::Linear),
        9984 => Ok(StaticMeshTextureMinFilter::NearestMipmapNearest),
        9985 => Ok(StaticMeshTextureMinFilter::LinearMipmapNearest),
        9986 => Ok(StaticMeshTextureMinFilter::NearestMipmapLinear),
        9987 => Ok(StaticMeshTextureMinFilter::LinearMipmapLinear),
        _ => Err(AssetError::UnsupportedGltfFeature(format!(
            "unsupported glTF texture minFilter value {value}"
        ))),
    }
}

fn gltf_texture_wrap(value: u64) -> AssetResult<StaticMeshTextureWrap> {
    match value {
        33071 => Ok(StaticMeshTextureWrap::ClampToEdge),
        33648 => Ok(StaticMeshTextureWrap::MirroredRepeat),
        10497 => Ok(StaticMeshTextureWrap::Repeat),
        _ => Err(AssetError::UnsupportedGltfFeature(format!(
            "unsupported glTF texture wrap value {value}"
        ))),
    }
}

fn gltf_image_bytes(
    document: &Value,
    buffers: &[Vec<u8>],
    base_dir: Option<&Path>,
    image: &Value,
) -> AssetResult<Vec<u8>> {
    if let Some(uri) = image.get("uri").and_then(Value::as_str) {
        if let Some(encoded) = data_uri_base64_payload(uri) {
            return Ok(BASE64_STANDARD.decode(encoded)?);
        }
        let base_dir = base_dir.ok_or_else(|| {
            AssetError::UnsupportedGltfFeature(format!(
                "image URI {uri:?} is external, but no base directory is available"
            ))
        })?;
        if uri.contains(':') {
            return Err(AssetError::UnsupportedGltfFeature(format!(
                "image URI {uri:?} is not a relative file path"
            )));
        }
        return Ok(fs::read(base_dir.join(uri))?);
    }

    let buffer_view_index =
        image
            .get("bufferView")
            .and_then(Value::as_u64)
            .ok_or(AssetError::MissingGltfField(
                "images[].uri or images[].bufferView",
            ))? as usize;
    Ok(buffer_view_bytes(document, buffers, buffer_view_index)?.to_vec())
}

fn data_uri_base64_payload(uri: &str) -> Option<&str> {
    let (metadata, payload) = uri.strip_prefix("data:")?.split_once(',')?;
    metadata.ends_with(";base64").then_some(payload)
}

fn load_glb_document_and_buffers(path: &Path) -> AssetResult<(Value, Vec<Vec<u8>>)> {
    let bytes = fs::read(path)?;
    if bytes.len() < 12 {
        return Err(AssetError::InvalidGlb(
            "file is shorter than the 12-byte GLB header".to_string(),
        ));
    }

    let magic = read_u32_le(&bytes, 0)?;
    let version = read_u32_le(&bytes, 4)?;
    let declared_length = read_u32_le(&bytes, 8)? as usize;
    if magic != 0x4654_6C67 {
        return Err(AssetError::InvalidGlb(
            "magic header is not glTF".to_string(),
        ));
    }
    if version != 2 {
        return Err(AssetError::UnsupportedGltfFeature(format!(
            "GLB version {version} is not supported"
        )));
    }
    if declared_length != bytes.len() {
        return Err(AssetError::InvalidGlb(format!(
            "declared length {declared_length} does not match file length {}",
            bytes.len()
        )));
    }

    let mut cursor = 12;
    let mut json_chunk = None;
    let mut bin_chunk = None;
    while cursor < bytes.len() {
        if cursor + 8 > bytes.len() {
            return Err(AssetError::InvalidGlb(
                "chunk header extends past end of file".to_string(),
            ));
        }
        let chunk_length = read_u32_le(&bytes, cursor)? as usize;
        let chunk_type = read_u32_le(&bytes, cursor + 4)?;
        cursor += 8;
        let chunk_end = cursor.checked_add(chunk_length).ok_or_else(|| {
            AssetError::InvalidGlb("chunk byte range overflows usize".to_string())
        })?;
        if chunk_end > bytes.len() {
            return Err(AssetError::InvalidGlb(
                "chunk payload extends past end of file".to_string(),
            ));
        }
        match chunk_type {
            0x4E4F_534A => json_chunk = Some(bytes[cursor..chunk_end].to_vec()),
            0x004E_4942 => bin_chunk = Some(bytes[cursor..chunk_end].to_vec()),
            _ => {}
        }
        cursor = chunk_end;
    }

    let json_chunk =
        json_chunk.ok_or_else(|| AssetError::InvalidGlb("missing JSON chunk".to_string()))?;
    let json_source = std::str::from_utf8(&json_chunk)
        .map_err(|error| AssetError::InvalidGlb(format!("JSON chunk is not UTF-8: {error}")))?
        .trim_end_matches([' ', '\0']);
    let document: Value = serde_json::from_str(json_source)?;
    let buffers = decode_gltf_buffers(&document, path.parent(), bin_chunk.as_deref())?;
    Ok((document, buffers))
}

fn decode_gltf_buffers(
    document: &Value,
    base_dir: Option<&Path>,
    glb_binary_chunk: Option<&[u8]>,
) -> AssetResult<Vec<Vec<u8>>> {
    let buffers = document
        .get("buffers")
        .and_then(Value::as_array)
        .ok_or(AssetError::MissingGltfField("buffers"))?;

    buffers
        .iter()
        .enumerate()
        .map(|(index, buffer)| {
            let uri = buffer.get("uri").and_then(Value::as_str);
            let decoded = if let Some(encoded) = uri
                .and_then(|uri| uri.strip_prefix("data:application/octet-stream;base64,"))
                .or_else(|| {
                    uri.and_then(|uri| uri.strip_prefix("data:application/gltf-buffer;base64,"))
                })
            {
                BASE64_STANDARD.decode(encoded)?
            } else if let Some(uri) = uri {
                let base_dir = base_dir.ok_or_else(|| {
                    AssetError::UnsupportedGltfFeature(format!(
                        "buffer[{index}] uses external URI {uri:?}, but no base directory is available"
                    ))
                })?;
                if uri.contains(':') {
                    return Err(AssetError::UnsupportedGltfFeature(format!(
                        "buffer[{index}] URI {uri:?} is not a relative file path"
                    )));
                }
                fs::read(base_dir.join(uri))?
            } else if index == 0 {
                glb_binary_chunk
                    .ok_or(AssetError::MissingGltfField("buffers[0].uri or GLB BIN chunk"))?
                    .to_vec()
            } else {
                return Err(AssetError::UnsupportedGltfFeature(format!(
                    "buffer[{index}] has no URI and only the first GLB buffer may omit URI"
                )));
            };
            let expected_len = buffer
                .get("byteLength")
                .and_then(Value::as_u64)
                .ok_or(AssetError::MissingGltfField("buffers[].byteLength"))?
                as usize;
            if decoded.len() < expected_len {
                return Err(AssetError::InvalidGltfMesh(format!(
                    "buffer[{index}] byteLength expected at least {expected_len}, decoded {}",
                    decoded.len()
                )));
            }
            Ok(decoded[..expected_len].to_vec())
        })
        .collect()
}

fn read_accessor_vec3(
    document: &Value,
    buffers: &[Vec<u8>],
    accessor_index: usize,
    label: &'static str,
) -> AssetResult<Vec<[f32; 3]>> {
    let accessor = gltf_array_item(document, "accessors", accessor_index)?;
    let component_type = gltf_u64(accessor, "componentType")?;
    let accessor_type = gltf_str(accessor, "type")?;
    if component_type != 5126 || accessor_type != "VEC3" {
        return Err(AssetError::UnsupportedGltfFeature(format!(
            "{label} must be FLOAT VEC3"
        )));
    }

    let count = gltf_u64(accessor, "count")? as usize;
    let (bytes, offset, stride) = accessor_buffer_span(document, buffers, accessor)?;
    let element_size = 3 * size_of::<f32>();
    if stride < element_size {
        return Err(AssetError::InvalidGltfMesh(format!(
            "{label} stride {stride} is smaller than element size {element_size}"
        )));
    }

    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let start = offset + index * stride;
        let end = start + element_size;
        let element = bytes.get(start..end).ok_or_else(|| {
            AssetError::InvalidGltfMesh(format!(
                "{label} accessor reads past buffer bounds at element {index}"
            ))
        })?;
        values.push([
            read_f32(element, 0)?,
            read_f32(element, 4)?,
            read_f32(element, 8)?,
        ]);
    }
    Ok(values)
}

fn read_accessor_color(
    document: &Value,
    buffers: &[Vec<u8>],
    accessor_index: usize,
    label: &'static str,
) -> AssetResult<Vec<[f32; 3]>> {
    let accessor = gltf_array_item(document, "accessors", accessor_index)?;
    let component_type = gltf_u64(accessor, "componentType")?;
    let accessor_type = gltf_str(accessor, "type")?;
    if component_type != 5126 || !matches!(accessor_type, "VEC3" | "VEC4") {
        return Err(AssetError::UnsupportedGltfFeature(format!(
            "{label} must be FLOAT VEC3 or FLOAT VEC4"
        )));
    }

    let count = gltf_u64(accessor, "count")? as usize;
    let components = if accessor_type == "VEC4" { 4 } else { 3 };
    let (bytes, offset, stride) = accessor_buffer_span(document, buffers, accessor)?;
    let element_size = components * size_of::<f32>();
    if stride < element_size {
        return Err(AssetError::InvalidGltfMesh(format!(
            "{label} stride {stride} is smaller than element size {element_size}"
        )));
    }

    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let start = offset + index * stride;
        let end = start + element_size;
        let element = bytes.get(start..end).ok_or_else(|| {
            AssetError::InvalidGltfMesh(format!(
                "{label} accessor reads past buffer bounds at element {index}"
            ))
        })?;
        values.push([
            read_f32(element, 0)?,
            read_f32(element, 4)?,
            read_f32(element, 8)?,
        ]);
    }
    Ok(values)
}

fn read_accessor_vec2(
    document: &Value,
    buffers: &[Vec<u8>],
    accessor_index: usize,
    label: &'static str,
) -> AssetResult<Vec<[f32; 2]>> {
    let accessor = gltf_array_item(document, "accessors", accessor_index)?;
    let component_type = gltf_u64(accessor, "componentType")?;
    let accessor_type = gltf_str(accessor, "type")?;
    if component_type != 5126 || accessor_type != "VEC2" {
        return Err(AssetError::UnsupportedGltfFeature(format!(
            "{label} must be FLOAT VEC2"
        )));
    }

    let count = gltf_u64(accessor, "count")? as usize;
    let (bytes, offset, stride) = accessor_buffer_span(document, buffers, accessor)?;
    let element_size = 2 * size_of::<f32>();
    if stride < element_size {
        return Err(AssetError::InvalidGltfMesh(format!(
            "{label} stride {stride} is smaller than element size {element_size}"
        )));
    }

    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let start = offset + index * stride;
        let end = start + element_size;
        let element = bytes.get(start..end).ok_or_else(|| {
            AssetError::InvalidGltfMesh(format!(
                "{label} accessor reads past buffer bounds at element {index}"
            ))
        })?;
        values.push([read_f32(element, 0)?, read_f32(element, 4)?]);
    }
    Ok(values)
}

fn read_accessor_indices(
    document: &Value,
    buffers: &[Vec<u8>],
    accessor_index: usize,
) -> AssetResult<Vec<u16>> {
    let accessor = gltf_array_item(document, "accessors", accessor_index)?;
    let component_type = gltf_u64(accessor, "componentType")?;
    let accessor_type = gltf_str(accessor, "type")?;
    if accessor_type != "SCALAR" || !matches!(component_type, 5123 | 5125) {
        return Err(AssetError::UnsupportedGltfFeature(
            "indices must be UNSIGNED_SHORT or UNSIGNED_INT SCALAR".to_string(),
        ));
    }

    let count = gltf_u64(accessor, "count")? as usize;
    let (bytes, offset, stride) = accessor_buffer_span(document, buffers, accessor)?;
    let element_size = if component_type == 5125 {
        size_of::<u32>()
    } else {
        size_of::<u16>()
    };
    if stride < element_size {
        return Err(AssetError::InvalidGltfMesh(format!(
            "index stride {stride} is smaller than element size {element_size}"
        )));
    }

    let mut indices = Vec::with_capacity(count);
    for index in 0..count {
        let start = offset + index * stride;
        let element = bytes.get(start..start + element_size).ok_or_else(|| {
            AssetError::InvalidGltfMesh(format!(
                "index accessor reads past buffer bounds at element {index}"
            ))
        })?;
        let value = if component_type == 5125 {
            let value = u32::from_le_bytes([element[0], element[1], element[2], element[3]]);
            u16::try_from(value).map_err(|_| {
                AssetError::UnsupportedGltfFeature(format!(
                    "index value {value} exceeds Vulkan UINT16 path"
                ))
            })?
        } else {
            u16::from_le_bytes([element[0], element[1]])
        };
        indices.push(value);
    }
    Ok(indices)
}

fn accessor_buffer_span<'a>(
    document: &'a Value,
    buffers: &'a [Vec<u8>],
    accessor: &Value,
) -> AssetResult<(&'a [u8], usize, usize)> {
    let buffer_view_index = gltf_u64(accessor, "bufferView")? as usize;
    let buffer_view = gltf_array_item(document, "bufferViews", buffer_view_index)?;
    let buffer_index = gltf_u64(buffer_view, "buffer")? as usize;
    let buffer = buffers.get(buffer_index).ok_or_else(|| {
        AssetError::InvalidGltfMesh(format!(
            "bufferView references missing buffer {buffer_index}"
        ))
    })?;
    let view_offset = gltf_optional_u64(buffer_view, "byteOffset") as usize;
    let accessor_offset = gltf_optional_u64(accessor, "byteOffset") as usize;
    let view_length = gltf_u64(buffer_view, "byteLength")? as usize;
    let stride = gltf_optional_u64(buffer_view, "byteStride") as usize;
    let view_end = view_offset.checked_add(view_length).ok_or_else(|| {
        AssetError::InvalidGltfMesh("bufferView byte range overflows usize".to_string())
    })?;
    if view_end > buffer.len() {
        return Err(AssetError::InvalidGltfMesh(format!(
            "bufferView range {view_offset}..{view_end} exceeds buffer length {}",
            buffer.len()
        )));
    }
    Ok((
        buffer,
        view_offset + accessor_offset,
        stride.max(accessor_component_size(accessor)?),
    ))
}

fn buffer_view_bytes<'a>(
    document: &'a Value,
    buffers: &'a [Vec<u8>],
    buffer_view_index: usize,
) -> AssetResult<&'a [u8]> {
    let buffer_view = gltf_array_item(document, "bufferViews", buffer_view_index)?;
    let buffer_index = gltf_u64(buffer_view, "buffer")? as usize;
    let buffer = buffers.get(buffer_index).ok_or_else(|| {
        AssetError::InvalidGltfMesh(format!(
            "bufferView references missing buffer {buffer_index}"
        ))
    })?;
    let view_offset = gltf_optional_u64(buffer_view, "byteOffset") as usize;
    let view_length = gltf_u64(buffer_view, "byteLength")? as usize;
    let view_end = view_offset.checked_add(view_length).ok_or_else(|| {
        AssetError::InvalidGltfMesh("bufferView byte range overflows usize".to_string())
    })?;
    buffer.get(view_offset..view_end).ok_or_else(|| {
        AssetError::InvalidGltfMesh(format!(
            "bufferView range {view_offset}..{view_end} exceeds buffer length {}",
            buffer.len()
        ))
    })
}

fn accessor_component_size(accessor: &Value) -> AssetResult<usize> {
    let component_size = match gltf_u64(accessor, "componentType")? {
        5123 => size_of::<u16>(),
        5125 => size_of::<u32>(),
        5126 => size_of::<f32>(),
        component_type => {
            return Err(AssetError::UnsupportedGltfFeature(format!(
                "component type {component_type} is not supported"
            )));
        }
    };
    let components = match gltf_str(accessor, "type")? {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        accessor_type => {
            return Err(AssetError::UnsupportedGltfFeature(format!(
                "accessor type {accessor_type} is not supported"
            )));
        }
    };
    Ok(component_size * components)
}

fn generate_smooth_normals(positions: &[[f32; 3]], indices: &[u16]) -> AssetResult<Vec<[f32; 3]>> {
    let mut normals = vec![[0.0, 0.0, 0.0]; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let a_index = usize::from(triangle[0]);
        let b_index = usize::from(triangle[1]);
        let c_index = usize::from(triangle[2]);
        let a = *positions.get(a_index).ok_or_else(|| {
            AssetError::InvalidGltfMesh(format!("index {a_index} references missing position"))
        })?;
        let b = *positions.get(b_index).ok_or_else(|| {
            AssetError::InvalidGltfMesh(format!("index {b_index} references missing position"))
        })?;
        let c = *positions.get(c_index).ok_or_else(|| {
            AssetError::InvalidGltfMesh(format!("index {c_index} references missing position"))
        })?;
        let normal = cross3(sub3(b, a), sub3(c, a));
        normals[a_index] = add3(normals[a_index], normal);
        normals[b_index] = add3(normals[b_index], normal);
        normals[c_index] = add3(normals[c_index], normal);
    }
    Ok(normals.into_iter().map(normalize3).collect())
}

fn read_f32(bytes: &[u8], offset: usize) -> AssetResult<f32> {
    let value = bytes
        .get(offset..offset + size_of::<f32>())
        .ok_or_else(|| {
            AssetError::InvalidGltfMesh("f32 read exceeds accessor element bounds".to_string())
        })?;
    Ok(f32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> AssetResult<u32> {
    let value = bytes
        .get(offset..offset + size_of::<u32>())
        .ok_or_else(|| AssetError::InvalidGlb("u32 read exceeds GLB byte bounds".to_string()))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn write_vec3_f32(buffer: &mut Vec<u8>, values: &[[f32; 3]]) {
    for value in values {
        for component in value {
            buffer.extend_from_slice(&component.to_le_bytes());
        }
    }
}

#[cfg(test)]
fn write_vec2_f32(buffer: &mut Vec<u8>, values: &[[f32; 2]]) {
    for value in values {
        for component in value {
            buffer.extend_from_slice(&component.to_le_bytes());
        }
    }
}

fn write_u16(buffer: &mut Vec<u8>, values: &[u16]) {
    for value in values {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
}

fn gltf_array_item<'a>(
    document: &'a Value,
    name: &'static str,
    index: usize,
) -> AssetResult<&'a Value> {
    document
        .get(name)
        .and_then(Value::as_array)
        .and_then(|items| items.get(index))
        .ok_or(AssetError::MissingGltfField(name))
}

fn gltf_u64(value: &Value, name: &'static str) -> AssetResult<u64> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .ok_or(AssetError::MissingGltfField(name))
}

fn gltf_optional_u64(value: &Value, name: &'static str) -> u64 {
    value.get(name).and_then(Value::as_u64).unwrap_or_default()
}

fn gltf_str<'a>(value: &'a Value, name: &'static str) -> AssetResult<&'a str> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or(AssetError::MissingGltfField(name))
}

fn gltf_node_transform(node: &Value) -> AssetResult<[[f32; 4]; 4]> {
    if node.get("matrix").is_some() {
        return gltf_mat4(node, "matrix");
    }

    let translation = gltf_optional_vec3(node, "translation", [0.0, 0.0, 0.0])?;
    let rotation = gltf_optional_quat(node, "rotation", [0.0, 0.0, 0.0, 1.0])?;
    let scale = gltf_optional_vec3(node, "scale", [1.0, 1.0, 1.0])?;

    Ok(multiply_mat4(
        translation_matrix(translation),
        multiply_mat4(quaternion_matrix(rotation), scale_matrix(scale)),
    ))
}

fn gltf_mat4(value: &Value, name: &'static str) -> AssetResult<[[f32; 4]; 4]> {
    let values = value
        .get(name)
        .and_then(Value::as_array)
        .ok_or(AssetError::MissingGltfField(name))?;
    if values.len() != 16 {
        return Err(AssetError::InvalidGltfMesh(format!(
            "{name} matrix must contain 16 numbers"
        )));
    }

    let mut matrix = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            matrix[column][row] = values[column * 4 + row].as_f64().ok_or_else(|| {
                AssetError::InvalidGltfMesh(format!("{name} matrix values must be numeric"))
            })? as f32;
        }
    }
    Ok(matrix)
}

fn gltf_optional_vec3(
    value: &Value,
    name: &'static str,
    default: [f32; 3],
) -> AssetResult<[f32; 3]> {
    let Some(values) = value.get(name) else {
        return Ok(default);
    };
    let values = values
        .as_array()
        .ok_or(AssetError::MissingGltfField(name))?;
    if values.len() != 3 {
        return Err(AssetError::InvalidGltfMesh(format!(
            "{name} must contain 3 numbers"
        )));
    }
    Ok([
        values[0]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
        values[1]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
        values[2]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
    ])
}

fn gltf_optional_quat(
    value: &Value,
    name: &'static str,
    default: [f32; 4],
) -> AssetResult<[f32; 4]> {
    let Some(values) = value.get(name) else {
        return Ok(default);
    };
    let values = values
        .as_array()
        .ok_or(AssetError::MissingGltfField(name))?;
    if values.len() != 4 {
        return Err(AssetError::InvalidGltfMesh(format!(
            "{name} must contain 4 numbers"
        )));
    }
    Ok([
        values[0]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
        values[1]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
        values[2]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
        values[3]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
    ])
}

fn gltf_optional_vec4(
    value: &Value,
    name: &'static str,
    default: [f32; 4],
) -> AssetResult<[f32; 4]> {
    let Some(values) = value.get(name) else {
        return Ok(default);
    };
    let values = values
        .as_array()
        .ok_or(AssetError::MissingGltfField(name))?;
    if values.len() != 4 {
        return Err(AssetError::InvalidGltfMesh(format!(
            "{name} must contain 4 numbers"
        )));
    }
    Ok([
        values[0]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
        values[1]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
        values[2]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
        values[3]
            .as_f64()
            .ok_or_else(|| AssetError::InvalidGltfMesh(format!("{name} values must be numeric")))?
            as f32,
    ])
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

fn quaternion_matrix(rotation: [f32; 4]) -> [[f32; 4]; 4] {
    let [x, y, z, w] = normalize4(rotation);
    let x2 = x + x;
    let y2 = y + y;
    let z2 = z + z;
    let xx = x * x2;
    let xy = x * y2;
    let xz = x * z2;
    let yy = y * y2;
    let yz = y * z2;
    let zz = z * z2;
    let wx = w * x2;
    let wy = w * y2;
    let wz = w * z2;

    [
        [1.0 - (yy + zz), xy + wz, xz - wy, 0.0],
        [xy - wz, 1.0 - (xx + zz), yz + wx, 0.0],
        [xz + wy, yz - wx, 1.0 - (xx + yy), 0.0],
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

fn add3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn sub3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn cross3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn dot3(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn normalize3(value: [f32; 3]) -> [f32; 3] {
    let length = dot3(value, value).sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [value[0] / length, value[1] / length, value[2] / length]
}

fn normalize4(value: [f32; 4]) -> [f32; 4] {
    let length =
        (value[0] * value[0] + value[1] * value[1] + value[2] * value[2] + value[3] * value[3])
            .sqrt();
    if length <= f32::EPSILON {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [
        value[0] / length,
        value[1] / length,
        value[2] / length,
        value[3] / length,
    ]
}

#[derive(Debug, Default)]
pub struct AssetDatabase {
    by_id: BTreeMap<AssetId, AssetMetadata>,
    by_path: BTreeMap<PathBuf, AssetId>,
}

impl AssetDatabase {
    pub fn register(&mut self, metadata: AssetMetadata) -> AssetResult<AssetId> {
        if self.by_id.contains_key(&metadata.id) {
            return Err(AssetError::DuplicateId(metadata.id));
        }

        if self.by_path.contains_key(&metadata.source_path) {
            return Err(AssetError::DuplicatePath(metadata.source_path));
        }

        let id = metadata.id;
        self.by_path.insert(metadata.source_path.clone(), id);
        self.by_id.insert(id, metadata);
        Ok(id)
    }

    pub fn get(&self, id: AssetId) -> Option<&AssetMetadata> {
        self.by_id.get(&id)
    }

    pub fn find_by_path(&self, path: impl AsRef<Path>) -> Option<&AssetMetadata> {
        self.by_path
            .get(path.as_ref())
            .and_then(|id| self.by_id.get(id))
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_data_uri(width: u32, height: u32, rgba: &[u8]) -> String {
        let mut png = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png);
        image::ImageEncoder::write_image(
            encoder,
            rgba,
            width,
            height,
            image::ColorType::Rgba8.into(),
        )
        .unwrap();
        format!("data:image/png;base64,{}", BASE64_STANDARD.encode(png))
    }

    #[test]
    fn metadata_round_trips_through_ron() {
        let metadata = AssetMetadata::new(AssetKind::Scene, "scenes/main.ron");

        let serialized = metadata.to_ron().unwrap();
        let deserialized = AssetMetadata::from_ron(&serialized).unwrap();

        assert_eq!(metadata, deserialized);
    }

    #[test]
    fn database_rejects_duplicate_source_paths() {
        let mut database = AssetDatabase::default();
        let first = AssetMetadata::new(AssetKind::Texture, "textures/albedo.png");
        let second = AssetMetadata::new(AssetKind::Texture, "textures/albedo.png");

        database.register(first).unwrap();

        assert!(matches!(
            database.register(second),
            Err(AssetError::DuplicatePath(_))
        ));
    }

    #[test]
    fn embedded_gltf_pyramid_loads_static_mesh() {
        let mesh = StaticMeshAsset::demo_pyramid_from_embedded_gltf().unwrap();

        assert_eq!(mesh.name, "Embedded GLTF Pyramid");
        assert_eq!(mesh.vertices.len(), 5);
        assert_eq!(mesh.indices.len(), 18);
        assert!(mesh.indices.iter().all(|index| usize::from(*index) < 5));
        assert!(mesh.vertices.iter().all(|vertex| {
            let length_squared = dot3(vertex.normal, vertex.normal);
            (length_squared - 1.0).abs() < 0.0001
        }));
    }

    #[test]
    fn external_gltf_scene_loads_relative_bin_buffer() {
        let directory = std::env::temp_dir().join(format!("finalengine-gltf-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let bin_path = directory.join("triangle.bin");
        let gltf_path = directory.join("scene.gltf");

        let positions = [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let colors = [[1.0_f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let indices = [0_u16, 1, 2];
        let mut buffer = Vec::new();
        let position_offset = buffer.len();
        write_vec3_f32(&mut buffer, &positions);
        let color_offset = buffer.len();
        write_vec3_f32(&mut buffer, &colors);
        let index_offset = buffer.len();
        write_u16(&mut buffer, &indices);
        std::fs::write(&bin_path, &buffer).unwrap();

        std::fs::write(
            &gltf_path,
            format!(
                r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": {buffer_len}, "uri": "triangle.bin" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": {position_offset}, "byteLength": {position_bytes} }},
    {{ "buffer": 0, "byteOffset": {color_offset}, "byteLength": {color_bytes} }},
    {{ "buffer": 0, "byteOffset": {index_offset}, "byteLength": {index_bytes} }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "meshes": [
    {{
      "name": "External Triangle",
      "primitives": [
        {{ "attributes": {{ "POSITION": 0, "COLOR_0": 1 }}, "indices": 2 }}
      ]
    }}
  ]
}}"#,
                buffer_len = buffer.len(),
                position_bytes = positions.len() * 3 * size_of::<f32>(),
                color_bytes = colors.len() * 3 * size_of::<f32>(),
                index_bytes = indices.len() * size_of::<u16>(),
            ),
        )
        .unwrap();

        let scene = StaticMeshSceneAsset::from_gltf_path(&gltf_path).unwrap();

        assert_eq!(scene.name, "scene");
        assert_eq!(scene.meshes.len(), 1);
        assert_eq!(scene.meshes[0].name, "External Triangle");
        assert_eq!(scene.meshes[0].vertices.len(), 3);
        assert_eq!(scene.meshes[0].indices, indices);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn gltf_scene_nodes_apply_hierarchical_mesh_transforms() {
        let positions = [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let colors = [[1.0_f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let indices = [0_u16, 1, 2];
        let mut buffer = Vec::new();
        let position_offset = buffer.len();
        write_vec3_f32(&mut buffer, &positions);
        let color_offset = buffer.len();
        write_vec3_f32(&mut buffer, &colors);
        let index_offset = buffer.len();
        write_u16(&mut buffer, &indices);
        let encoded = BASE64_STANDARD.encode(&buffer);

        let source = format!(
            r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0] }}],
  "nodes": [
    {{ "name": "Parent", "translation": [3.0, 0.0, 0.0], "children": [1] }},
    {{ "name": "Child Instance", "mesh": 0, "translation": [0.0, 5.0, 0.0], "scale": [2.0, 1.0, 1.0] }}
  ],
  "buffers": [{{ "byteLength": {buffer_len}, "uri": "data:application/octet-stream;base64,{encoded}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": {position_offset}, "byteLength": {position_bytes} }},
    {{ "buffer": 0, "byteOffset": {color_offset}, "byteLength": {color_bytes} }},
    {{ "buffer": 0, "byteOffset": {index_offset}, "byteLength": {index_bytes} }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "meshes": [
    {{
      "name": "Reusable Triangle",
      "primitives": [
        {{ "attributes": {{ "POSITION": 0, "COLOR_0": 1 }}, "indices": 2 }}
      ]
    }}
  ]
}}"#,
            buffer_len = buffer.len(),
            position_bytes = positions.len() * 3 * size_of::<f32>(),
            color_bytes = colors.len() * 3 * size_of::<f32>(),
            index_bytes = indices.len() * size_of::<u16>(),
        );

        let mesh = StaticMeshAsset::from_embedded_gltf_json(&source).unwrap();

        assert_eq!(mesh.name, "Child Instance");
        assert_eq!(mesh.transform.matrix[0], [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(mesh.transform.matrix[1], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(mesh.transform.matrix[2], [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(mesh.transform.matrix[3], [3.0, 5.0, 0.0, 1.0]);
    }

    #[test]
    fn gltf_material_base_color_tints_vertices_without_color_accessor() {
        let positions = [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let indices = [0_u16, 1, 2];
        let mut buffer = Vec::new();
        let position_offset = buffer.len();
        write_vec3_f32(&mut buffer, &positions);
        let index_offset = buffer.len();
        write_u16(&mut buffer, &indices);
        let encoded = BASE64_STANDARD.encode(&buffer);

        let source = format!(
            r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": {buffer_len}, "uri": "data:application/octet-stream;base64,{encoded}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": {position_offset}, "byteLength": {position_bytes} }},
    {{ "buffer": 0, "byteOffset": {index_offset}, "byteLength": {index_bytes} }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "materials": [
    {{ "name": "Blue Material", "pbrMetallicRoughness": {{ "baseColorFactor": [0.25, 0.5, 0.75, 0.4] }} }}
  ],
  "meshes": [
    {{
      "name": "Material Triangle",
      "primitives": [
        {{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "material": 0 }}
      ]
    }}
  ]
}}"#,
            buffer_len = buffer.len(),
            position_bytes = positions.len() * 3 * size_of::<f32>(),
            index_bytes = indices.len() * size_of::<u16>(),
        );

        let mesh = StaticMeshAsset::from_embedded_gltf_json(&source).unwrap();

        assert_eq!(mesh.name, "Material Triangle");
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| vertex.color == [1.0, 1.0, 1.0])
        );
        assert_eq!(mesh.material.base_color_factor, [0.25, 0.5, 0.75, 0.4]);
        assert!(mesh.material.base_color_texture.is_none());
    }

    #[test]
    fn gltf_material_base_color_multiplies_color_accessor() {
        let positions = [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let colors = [[0.5_f32, 1.0, 0.25], [1.0, 0.5, 0.5], [0.25, 0.25, 1.0]];
        let indices = [0_u16, 1, 2];
        let mut buffer = Vec::new();
        let position_offset = buffer.len();
        write_vec3_f32(&mut buffer, &positions);
        let color_offset = buffer.len();
        write_vec3_f32(&mut buffer, &colors);
        let index_offset = buffer.len();
        write_u16(&mut buffer, &indices);
        let encoded = BASE64_STANDARD.encode(&buffer);

        let source = format!(
            r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": {buffer_len}, "uri": "data:application/octet-stream;base64,{encoded}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": {position_offset}, "byteLength": {position_bytes} }},
    {{ "buffer": 0, "byteOffset": {color_offset}, "byteLength": {color_bytes} }},
    {{ "buffer": 0, "byteOffset": {index_offset}, "byteLength": {index_bytes} }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "materials": [
    {{ "name": "Tint Material", "pbrMetallicRoughness": {{ "baseColorFactor": [0.2, 0.5, 0.75, 1.0] }} }}
  ],
  "meshes": [
    {{
      "name": "Tinted Vertex Color Triangle",
      "primitives": [
        {{ "attributes": {{ "POSITION": 0, "COLOR_0": 1 }}, "indices": 2, "material": 0 }}
      ]
    }}
  ]
}}"#,
            buffer_len = buffer.len(),
            position_bytes = positions.len() * 3 * size_of::<f32>(),
            color_bytes = colors.len() * 3 * size_of::<f32>(),
            index_bytes = indices.len() * size_of::<u16>(),
        );

        let mesh = StaticMeshAsset::from_embedded_gltf_json(&source).unwrap();

        assert_eq!(mesh.name, "Tinted Vertex Color Triangle");
        assert_eq!(mesh.vertices[0].color, [0.5, 1.0, 0.25]);
        assert_eq!(mesh.vertices[1].color, [1.0, 0.5, 0.5]);
        assert_eq!(mesh.vertices[2].color, [0.25, 0.25, 1.0]);
        assert_eq!(mesh.material.base_color_factor, [0.2, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn gltf_base_color_texture_tints_vertices_from_texcoords() {
        let positions = [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let texcoords = [[0.0_f32, 0.0], [0.25, 0.75], [0.9, 0.1]];
        let indices = [0_u16, 1, 2];
        let mut buffer = Vec::new();
        let position_offset = buffer.len();
        write_vec3_f32(&mut buffer, &positions);
        let texcoord_offset = buffer.len();
        write_vec2_f32(&mut buffer, &texcoords);
        let index_offset = buffer.len();
        write_u16(&mut buffer, &indices);
        let encoded = BASE64_STANDARD.encode(&buffer);
        let texture_uri = png_data_uri(1, 1, &[128, 64, 255, 255]);

        let source = format!(
            r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": {buffer_len}, "uri": "data:application/octet-stream;base64,{encoded}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": {position_offset}, "byteLength": {position_bytes} }},
    {{ "buffer": 0, "byteOffset": {texcoord_offset}, "byteLength": {texcoord_bytes} }},
    {{ "buffer": 0, "byteOffset": {index_offset}, "byteLength": {index_bytes} }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2" }},
    {{ "bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "images": [
    {{ "uri": "{texture_uri}" }}
  ],
  "samplers": [
    {{ "magFilter": 9728, "minFilter": 9985, "wrapS": 33071, "wrapT": 33648 }}
  ],
  "textures": [
    {{ "source": 0, "sampler": 0 }}
  ],
  "materials": [
    {{ "name": "Textured Material", "pbrMetallicRoughness": {{ "baseColorFactor": [0.5, 1.0, 0.25, 1.0], "baseColorTexture": {{ "index": 0 }} }} }}
  ],
  "meshes": [
    {{
      "name": "Textured Triangle",
      "primitives": [
        {{ "attributes": {{ "POSITION": 0, "TEXCOORD_0": 1 }}, "indices": 2, "material": 0 }}
      ]
    }}
  ]
}}"#,
            buffer_len = buffer.len(),
            position_bytes = positions.len() * 3 * size_of::<f32>(),
            texcoord_bytes = texcoords.len() * 2 * size_of::<f32>(),
            index_bytes = indices.len() * size_of::<u16>(),
        );

        let mesh = StaticMeshAsset::from_embedded_gltf_json(&source).unwrap();

        assert_eq!(mesh.name, "Textured Triangle");
        assert_eq!(mesh.material.base_color_factor, [0.5, 1.0, 0.25, 1.0]);
        assert_eq!(
            mesh.vertices
                .iter()
                .map(|vertex| vertex.texcoord)
                .collect::<Vec<_>>(),
            texcoords
        );
        let texture = mesh.material.base_color_texture.as_ref().unwrap();
        assert_eq!(texture.width, 1);
        assert_eq!(texture.height, 1);
        assert_eq!(texture.rgba, vec![128, 64, 255, 255]);
        assert_eq!(texture.sampler.mag_filter, StaticMeshTextureFilter::Nearest);
        assert_eq!(
            texture.sampler.min_filter,
            StaticMeshTextureMinFilter::LinearMipmapNearest
        );
        assert_eq!(texture.sampler.wrap_s, StaticMeshTextureWrap::ClampToEdge);
        assert_eq!(
            texture.sampler.wrap_t,
            StaticMeshTextureWrap::MirroredRepeat
        );
    }

    #[test]
    fn binary_glb_scene_loads_embedded_bin_chunk() {
        let directory = std::env::temp_dir().join(format!("finalengine-glb-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let glb_path = directory.join("scene.glb");

        let positions = [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let colors = [[1.0_f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let indices = [0_u16, 1, 2];
        let mut buffer = Vec::new();
        let position_offset = buffer.len();
        write_vec3_f32(&mut buffer, &positions);
        let color_offset = buffer.len();
        write_vec3_f32(&mut buffer, &colors);
        let index_offset = buffer.len();
        write_u16(&mut buffer, &indices);

        let json_source = format!(
            r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": {buffer_len} }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": {position_offset}, "byteLength": {position_bytes} }},
    {{ "buffer": 0, "byteOffset": {color_offset}, "byteLength": {color_bytes} }},
    {{ "buffer": 0, "byteOffset": {index_offset}, "byteLength": {index_bytes} }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "meshes": [
    {{
      "name": "Binary Triangle",
      "primitives": [
        {{ "attributes": {{ "POSITION": 0, "COLOR_0": 1 }}, "indices": 2 }}
      ]
    }}
  ]
}}"#,
            buffer_len = buffer.len(),
            position_bytes = positions.len() * 3 * size_of::<f32>(),
            color_bytes = colors.len() * 3 * size_of::<f32>(),
            index_bytes = indices.len() * size_of::<u16>(),
        );

        let mut json_chunk = json_source.into_bytes();
        while !json_chunk.len().is_multiple_of(4) {
            json_chunk.push(b' ');
        }
        let mut bin_chunk = buffer;
        while !bin_chunk.len().is_multiple_of(4) {
            bin_chunk.push(0);
        }

        let total_length = 12 + 8 + json_chunk.len() + 8 + bin_chunk.len();
        let mut glb = Vec::new();
        glb.extend_from_slice(&0x4654_6C67_u32.to_le_bytes());
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&(total_length as u32).to_le_bytes());
        glb.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
        glb.extend_from_slice(&json_chunk);
        glb.extend_from_slice(&(bin_chunk.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
        glb.extend_from_slice(&bin_chunk);
        std::fs::write(&glb_path, glb).unwrap();

        let scene = StaticMeshSceneAsset::from_gltf_path(&glb_path).unwrap();

        assert_eq!(scene.name, "scene");
        assert_eq!(scene.meshes.len(), 1);
        assert_eq!(scene.meshes[0].name, "Binary Triangle");
        assert_eq!(scene.meshes[0].vertices.len(), 3);
        assert_eq!(scene.meshes[0].indices, indices);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn static_mesh_rejects_out_of_range_indices() {
        let vertices = vec![StaticMeshVertex {
            position: [0.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            color: [1.0, 1.0, 1.0],
            texcoord: [0.0, 0.0],
        }];

        assert!(matches!(
            StaticMeshAsset::new("bad mesh", vertices, vec![0, 1, 2]),
            Err(AssetError::InvalidGltfMesh(_))
        ));
    }
}
