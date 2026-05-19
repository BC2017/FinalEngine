# FinalEngine Test Scene

Place a glTF 2.0 test scene at one of these paths:

```text
assets/test_scene/scene.glb
assets/test_scene/scene.gltf
```

The Vulkan renderer checks this folder at startup. It prefers `scene.glb`, then falls back to `scene.gltf`. If neither file exists, the renderer uses the built-in cube, pyramid, and ground-plane demo scene.

Supported in this first loader pass:

- `.glb` binary files.
- `.gltf` JSON files.
- Embedded base64 buffers.
- Relative external `.bin` buffers next to the `.gltf`.
- Triangle primitives with `POSITION`, optional `NORMAL`, optional `COLOR_0`, and `UNSIGNED_SHORT` or `UNSIGNED_INT` indices.

Textures, materials, animations, skins, cameras, and node transforms are not imported yet.
