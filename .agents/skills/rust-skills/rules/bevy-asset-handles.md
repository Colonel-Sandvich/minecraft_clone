# bevy-asset-handles

> Keep `Handle<T>` values in long-lived state; do not reload assets per frame.

## Why It Matters

`Handle<T>` is a strong reference. When the last clone drops, Bevy unloads the asset. Repeated `AssetServer::load` calls return cached handles but keep adding references and bookkeeping, and a handle returned from a system and immediately dropped can unload the asset before anything uses it. Runtime-generated meshes and images must keep their handle on the entity or resource that consumes them.

## Bad

```rust
fn spawn_icon(mut commands: Commands, asset_server: Res<AssetServer>) {
    // New strong handle every time this runs.
    let icon = asset_server.load("textures/icon.png");
    commands.spawn(Sprite::from_image(icon));
}
```

## Good

Load once and store the handle:

```rust
#[derive(Resource)]
struct UiTextures {
    icon: Handle<Image>,
}

fn load_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(UiTextures {
        icon: asset_server.load("textures/icon.png"),
    });
}
```

For generated data, keep the handle where it is consumed:

```rust
let handle = meshes.add(mesh);
commands.entity(chunk_entity).insert(Mesh3d(handle));
```

Dropping the owning component is the intended way to free a per-chunk mesh. Do not keep a global cache of every generated handle unless you actually need to reuse it.

## Project Notes

- Existing patterns: `PauseMenuTextures` and `SoundBank` hold long-lived handles; `PreparedChunkMesh` owns generated mesh handles.
- UI textures load with `load_builder().with_settings(...)`; keep the sampler settings attached to the same load.
- A steadily growing handle count over a long session is a leak signal, not a loading-strategy detail.

## See Also

- [own-arc-shared](own-arc-shared.md) - handle reference counting follows the same ownership discipline
- [mem-reuse-collections](mem-reuse-collections.md) - reuse buffers instead of rebuilding assets
- [bevy-change-detection](bevy-change-detection.md) - avoid re-preparing assets that did not change
