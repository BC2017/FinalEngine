use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioBackend {
    Kira,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioConfig {
    pub backend: AudioBackend,
    pub spatial_audio_enabled: bool,
    pub master_volume: f32,
}

impl AudioConfig {
    pub fn kira_defaults() -> Self {
        Self {
            backend: AudioBackend::Kira,
            spatial_audio_enabled: true,
            master_volume: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kira_defaults_enable_spatial_audio() {
        let config = AudioConfig::kira_defaults();

        assert!(config.spatial_audio_enabled);
    }
}
