use engine_core::{AppInfo, EngineApp};
use engine_render::{
    RendererConfig, VulkanWindowConfig, run_vulkan_renderer, run_vulkan_renderer_with_window_config,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            std::env::var("FINALENGINE_LOG").unwrap_or_else(|_| "trace".to_string()),
        ))
        .init();

    let mut app = EngineApp::new(AppInfo::new("FinalEngine", env!("CARGO_PKG_VERSION")));
    app.start()?;

    let renderer = RendererConfig::vulkan_high_end();
    if std::env::args().any(|arg| arg == "--renderer-smoke-test") {
        let window_config = VulkanWindowConfig {
            max_frames: Some(3),
            ..Default::default()
        };
        run_vulkan_renderer_with_window_config(renderer, window_config)?;
    } else {
        run_vulkan_renderer(renderer)?;
    }

    app.stop();
    Ok(())
}
