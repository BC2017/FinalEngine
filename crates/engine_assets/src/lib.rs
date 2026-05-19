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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticMeshAsset {
    pub name: String,
    pub vertices: Vec<StaticMeshVertex>,
    pub indices: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticMeshSceneAsset {
    pub name: String,
    pub meshes: Vec<StaticMeshAsset>,
}

impl StaticMeshSceneAsset {
    pub fn from_gltf_path(path: impl AsRef<Path>) -> AssetResult<Self> {
        let path = path.as_ref();
        let (document, buffers) = if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("glb"))
        {
            load_glb_document_and_buffers(path)?
        } else {
            let source = fs::read_to_string(path)?;
            let document: Value = serde_json::from_str(&source)?;
            let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
            let buffers = decode_gltf_buffers(&document, Some(base_dir), None)?;
            (document, buffers)
        };
        let meshes = static_meshes_from_gltf_document(&document, &buffers)?;
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
        })
    }

    pub fn from_embedded_gltf_json(source: &str) -> AssetResult<Self> {
        let document: Value = serde_json::from_str(source)?;
        let buffers = decode_gltf_buffers(&document, None, None)?;
        static_meshes_from_gltf_document(&document, &buffers)?
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
) -> AssetResult<Vec<StaticMeshAsset>> {
    let meshes = document
        .get("meshes")
        .and_then(Value::as_array)
        .ok_or(AssetError::MissingGltfField("meshes"))?;
    let mut static_meshes = Vec::new();

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
                document, buffers, primitive, name,
            )?);
        }
    }

    Ok(static_meshes)
}

fn static_mesh_from_gltf_primitive(
    document: &Value,
    buffers: &[Vec<u8>],
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
    let indices = read_accessor_indices(document, buffers, index_accessor)?;

    let normals = match normals {
        Some(normals) => normals,
        None => generate_smooth_normals(&positions, &indices)?,
    };
    let colors = colors.unwrap_or_else(|| vec![[0.75, 0.75, 0.75]; positions.len()]);

    if normals.len() != positions.len() || colors.len() != positions.len() {
        return Err(AssetError::InvalidGltfMesh(
            "POSITION, NORMAL, and COLOR_0 accessor counts must match".to_string(),
        ));
    }

    let vertices = positions
        .into_iter()
        .zip(normals)
        .zip(colors)
        .map(|((position, normal), color)| StaticMeshVertex {
            position,
            normal,
            color,
        })
        .collect();

    StaticMeshAsset::new(name, vertices, indices)
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
        }];

        assert!(matches!(
            StaticMeshAsset::new("bad mesh", vertices, vec![0, 1, 2]),
            Err(AssetError::InvalidGltfMesh(_))
        ));
    }
}
