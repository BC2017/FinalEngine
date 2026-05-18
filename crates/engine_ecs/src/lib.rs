use serde::{Deserialize, Serialize};
use shipyard::World;
use uuid::Uuid;

#[derive(Debug)]
pub struct EcsWorld {
    world: World,
}

impl EcsWorld {
    pub fn new() -> Self {
        Self {
            world: World::new(),
        }
    }

    pub fn raw(&self) -> &World {
        &self.world
    }

    pub fn raw_mut(&mut self) -> &mut World {
        &mut self.world
    }
}

impl Default for EcsWorld {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StableEntityId(Uuid);

impl StableEntityId {
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

impl Default for StableEntityId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityName {
    pub value: String,
}

impl EntityName {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: [0.0, 0.0, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    };
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneEntity {
    pub id: StableEntityId,
    pub name: EntityName,
    pub transform: Transform,
}

impl SceneEntity {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: StableEntityId::new(),
            name: EntityName::new(name),
            transform: Transform::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_are_unique() {
        let first = StableEntityId::new();
        let second = StableEntityId::new();

        assert_ne!(first, second);
    }

    #[test]
    fn scene_entity_defaults_to_identity_transform() {
        let entity = SceneEntity::new("Camera");

        assert_eq!(entity.transform, Transform::IDENTITY);
    }
}
