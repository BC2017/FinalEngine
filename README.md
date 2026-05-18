# FinalEngine

FinalEngine is a Windows-first Rust 3D game engine scaffold with an ECS runtime, Vulkan renderer boundary, C# scripting host boundary, editor boundary, and import-oriented asset model.

This repository currently implements the foundation crates and verification tests for the first milestone:

- `engine_core`: app lifecycle, clock, engine errors.
- `engine_ecs`: Shipyard-backed world boundary, stable entity IDs, transform data.
- `engine_assets`: GUID-based asset database metadata and RON round trips.
- `engine_scripting`: CoreCLR/.NET script host configuration and hot-reload session model.
- `engine_render`: Vulkan renderer configuration and feature negotiation model.
- `engine_editor`: editor shell state and panel registry.
- `engine_physics`: physics configuration boundary.
- `engine_audio`: audio configuration boundary.
- `finalengine`: CLI entry point for the scaffold runtime.

## Verification

```powershell
cargo fmt --check
cargo test --workspace
cargo check --workspace
```

