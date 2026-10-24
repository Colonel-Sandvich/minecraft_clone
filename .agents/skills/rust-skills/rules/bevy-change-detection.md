# bevy-change-detection

> Mutate ECS data only when the value actually changes.

## Why It Matters

`ResMut<T>` and `Mut<T>` mark the item as changed when dereferenced mutably, even if you write the same value back. `Changed<T>` and `Added<T>` filters, render extraction, and this repo's mesh preparation all rely on those change ticks. Unconditional writes wake downstream systems and can force remeshing or GPU uploads every frame for data that did not change.

## Bad

```rust
fn sync_visuals(mut visuals: ResMut<TerrainVisualSettings>, ui: Res<UiSettings>) {
    // DerefMut marks the resource changed every frame.
    visuals.fog_distance = ui.fog_distance;
}
```

## Good

```rust
fn sync_visuals(mut visuals: ResMut<TerrainVisualSettings>, ui: Res<UiSettings>) {
    if visuals.fog_distance != ui.fog_distance {
        visuals.fog_distance = ui.fog_distance;
    }
}
```

Check changes without taking a mutable borrow:

```rust
fn prepare(meshes: Query<&ChunkMeshLayer>) {
    for layer in &meshes {
        if layer.is_changed() {
            // Only work for fresh layers.
        }
    }
}
```

Use `Added<T>` for one-shot setup and `Mut::bypass_change_detection()` only for bookkeeping fields that must not wake consumers, such as counters or timing stats. Never bypass change detection for gameplay-visible state.

## Project Notes

- `src/world/chunk/mesh/render/vertex_pulling/prepare.rs` uses `Ref<ChunkMeshLayer>` plus `Changed<ChunkMeshLayer>`; follow that combination for conditional render work.
- Prefer explicit dirty markers (`ChunkNeedsFluidStep`, `ChunkNeedsSave`, `ChunkNeedsMesh`) when the origin of a change is known; they document intent better than a blanket `Changed` filter.
- Beware `*value = *value;` and read-modify-write helpers that always assign.

## See Also

- [bevy-query-disjoint](bevy-query-disjoint.md) - `Ref` and `Mut` interact with query access
- [perf-profile-first](perf-profile-first.md) - verify that the extra work is real
- [anti-premature-optimize](anti-premature-optimize.md) - do not bypass change detection without measuring
