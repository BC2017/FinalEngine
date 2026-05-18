use engine_audio::AudioConfig;
use engine_core::{AppInfo, EngineApp};
use engine_editor::EditorShell;
use engine_physics::PhysicsConfig;
use engine_render::RendererConfig;
use engine_scripting::{CoreClrScriptHost, DotNetRuntimeConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut app = EngineApp::new(AppInfo::new("FinalEngine", env!("CARGO_PKG_VERSION")));
    app.start()?;

    let editor = EditorShell::full_editor_defaults();
    let renderer = RendererConfig::vulkan_high_end();
    let physics = PhysicsConfig::rapier_defaults();
    let audio = AudioConfig::kira_defaults();
    let mut scripts = CoreClrScriptHost::new(DotNetRuntimeConfig::dotnet_10_lts());
    scripts.load_runtime();

    println!(
        "FinalEngine scaffold started: panels={}, renderer={:?}, physics_dt={:.4}, audio_spatial={}, scripts={:?}",
        editor.panels.len(),
        renderer.choose_path(false),
        physics.fixed_timestep_seconds,
        audio.spatial_audio_enabled,
        scripts.state()
    );

    app.stop();
    Ok(())
}
