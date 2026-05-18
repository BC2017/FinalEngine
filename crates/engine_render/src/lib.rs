use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
