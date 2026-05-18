use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("{0}")]
    Message(String),
}

pub type EngineResult<T> = Result<T, EngineError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnginePhase {
    Created,
    Running,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

impl AppInfo {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

#[derive(Debug)]
pub struct EngineApp {
    info: AppInfo,
    phase: EnginePhase,
    clock: EngineClock,
}

impl EngineApp {
    pub fn new(info: AppInfo) -> Self {
        Self {
            info,
            phase: EnginePhase::Created,
            clock: EngineClock::new(),
        }
    }

    pub fn start(&mut self) -> EngineResult<()> {
        if self.phase == EnginePhase::Running {
            return Err(EngineError::Message(
                "engine is already running".to_string(),
            ));
        }

        self.phase = EnginePhase::Running;
        self.clock.reset();
        tracing::info!(name = %self.info.name, version = %self.info.version, "engine started");
        Ok(())
    }

    pub fn stop(&mut self) {
        self.phase = EnginePhase::Stopped;
        tracing::info!("engine stopped");
    }

    pub fn tick(&mut self) -> FrameTime {
        self.clock.tick()
    }

    pub fn info(&self) -> &AppInfo {
        &self.info
    }

    pub fn phase(&self) -> EnginePhase {
        self.phase
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FrameTime {
    pub delta_seconds: f32,
    pub total_seconds: f64,
    pub frame_index: u64,
}

#[derive(Debug)]
pub struct EngineClock {
    started_at: Instant,
    last_frame_at: Instant,
    frame_index: u64,
}

impl EngineClock {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started_at: now,
            last_frame_at: now,
            frame_index: 0,
        }
    }

    pub fn reset(&mut self) {
        let now = Instant::now();
        self.started_at = now;
        self.last_frame_at = now;
        self.frame_index = 0;
    }

    pub fn tick(&mut self) -> FrameTime {
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame_at);
        let total = now.duration_since(self.started_at);
        self.last_frame_at = now;
        self.frame_index += 1;

        FrameTime {
            delta_seconds: duration_to_seconds_f32(delta),
            total_seconds: total.as_secs_f64(),
            frame_index: self.frame_index,
        }
    }
}

impl Default for EngineClock {
    fn default() -> Self {
        Self::new()
    }
}

fn duration_to_seconds_f32(duration: Duration) -> f32 {
    duration.as_secs_f32().max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_start_transitions_to_running() {
        let mut app = EngineApp::new(AppInfo::new("test", "0.1.0"));

        app.start().unwrap();

        assert_eq!(app.phase(), EnginePhase::Running);
    }

    #[test]
    fn double_start_is_rejected() {
        let mut app = EngineApp::new(AppInfo::new("test", "0.1.0"));

        app.start().unwrap();

        assert!(app.start().is_err());
    }
}
