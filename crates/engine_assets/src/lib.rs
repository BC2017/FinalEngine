use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
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
}
