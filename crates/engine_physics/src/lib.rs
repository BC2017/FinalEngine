use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsConfig {
    pub fixed_timestep_seconds: f32,
    pub gravity: [f32; 3],
}

impl PhysicsConfig {
    pub fn rapier_defaults() -> Self {
        Self {
            fixed_timestep_seconds: 1.0 / 60.0,
            gravity: [0.0, -9.81, 0.0],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhysicsBackend {
    Rapier,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapier_defaults_use_downward_gravity() {
        let config = PhysicsConfig::rapier_defaults();

        assert!(config.gravity[1] < 0.0);
    }
}
