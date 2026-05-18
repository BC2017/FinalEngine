use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("script assembly has no path")]
    MissingAssemblyPath,
    #[error("script hot reload is not enabled")]
    HotReloadDisabled,
}

pub type ScriptResult<T> = Result<T, ScriptError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DotNetRuntimeConfig {
    pub target_framework: String,
    pub runtime_config_path: Option<PathBuf>,
    pub enable_hot_reload: bool,
}

impl DotNetRuntimeConfig {
    pub fn dotnet_10_lts() -> Self {
        Self {
            target_framework: "net10.0".to_string(),
            runtime_config_path: None,
            enable_hot_reload: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptAssembly {
    pub name: String,
    pub path: PathBuf,
}

impl ScriptAssembly {
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> ScriptResult<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(ScriptError::MissingAssemblyPath);
        }

        Ok(Self {
            name: name.into(),
            path,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptHostState {
    Created,
    RuntimeLoaded,
    AssemblyLoaded(ScriptAssembly),
}

#[derive(Debug)]
pub struct CoreClrScriptHost {
    config: DotNetRuntimeConfig,
    state: ScriptHostState,
    reload_generation: u64,
}

impl CoreClrScriptHost {
    pub fn new(config: DotNetRuntimeConfig) -> Self {
        Self {
            config,
            state: ScriptHostState::Created,
            reload_generation: 0,
        }
    }

    pub fn load_runtime(&mut self) {
        self.state = ScriptHostState::RuntimeLoaded;
    }

    pub fn load_assembly(&mut self, assembly: ScriptAssembly) {
        self.state = ScriptHostState::AssemblyLoaded(assembly);
    }

    pub fn begin_hot_reload(&mut self) -> ScriptResult<HotReloadToken> {
        if !self.config.enable_hot_reload {
            return Err(ScriptError::HotReloadDisabled);
        }

        self.reload_generation += 1;
        Ok(HotReloadToken {
            generation: self.reload_generation,
        })
    }

    pub fn state(&self) -> &ScriptHostState {
        &self.state
    }

    pub fn reload_generation(&self) -> u64 {
        self.reload_generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotReloadToken {
    pub generation: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_targets_dotnet_10() {
        let config = DotNetRuntimeConfig::dotnet_10_lts();

        assert_eq!(config.target_framework, "net10.0");
        assert!(config.enable_hot_reload);
    }

    #[test]
    fn hot_reload_advances_generation() {
        let mut host = CoreClrScriptHost::new(DotNetRuntimeConfig::dotnet_10_lts());

        let first = host.begin_hot_reload().unwrap();
        let second = host.begin_hot_reload().unwrap();

        assert_eq!(first.generation, 1);
        assert_eq!(second.generation, 2);
    }
}
