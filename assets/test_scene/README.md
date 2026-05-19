# FinalEngine Test Scene

Place a glTF 2.0 test scene at:

```text
assets/test_scene/scene.gltf
```

The Vulkan renderer checks this path at startup. If `scene.gltf` exists, it is loaded and rendered by default. If it is missing, the renderer falls back to the built-in cube, pyramid, and ground-plane demo scene.

Supported in this first loader pass:

- `.gltf` JSON files.
- Embedded base64 buffers.
- Relative external `.bin` buffers next to the `.gltf`.
- Triangle primitives with `POSITION`, optional `NORMAL`, optional `COLOR_0`, and `UNSIGNED_SHORT` or `UNSIGNED_INT` indices.

Textures, materials, animations, skins, cameras, `.glb`, and node transforms are not imported yet.
