# FinalEngine Test Scene

Place glTF 2.0 test scenes in this folder:

```text
assets/test_scene/
```

The Vulkan renderer checks this folder at startup and loads the first `.glb` or `.gltf` file it finds. Selection is deterministic: `.glb` files are preferred over `.gltf` files, and files within each extension group are sorted by name. If no supported file exists, the renderer uses the built-in cube, pyramid, and ground-plane demo scene.

Supported in this first loader pass:

- `.glb` binary files.
- `.gltf` JSON files.
- Embedded base64 buffers.
- Relative external `.bin` buffers next to the `.gltf`.
- Triangle primitives with `POSITION`, optional `NORMAL`, optional `COLOR_0`, and `UNSIGNED_SHORT` or `UNSIGNED_INT` indices.
- Material base color factors.
- Base color textures from data URIs, relative image files, or buffer views. Textures are decoded as PNG/JPEG, uploaded to Vulkan images with mipmaps, and sampled with `TEXCOORD_0`.
- glTF texture sampler filters and `wrapS`/`wrapT` modes for base color textures.
- Default glTF scene traversal with hierarchical node transforms.

Animations, skins, cameras, alpha blending, metallic/roughness textures, normal maps, and texture deduplication are not imported yet.
