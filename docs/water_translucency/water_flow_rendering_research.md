# Water Flow And Rendering Research

This document summarizes the research pass for rewriting water propagation and rendering. It is intentionally broader than the original first-pass water roadmap because the current implementation has moved past simple full-block visuals.

## Short Version

- Minecraft water is not a per-chunk uniform flood fill. It is a scheduled fluid tick system driven by block updates and neighbor updates.
- The current implementation re-emits water from existing fluid cells into a next buffer, flows down first, then spreads to every horizontal neighbor uniformly. That explains the too-symmetric spread.
- Minecraft horizontal spread prefers the nearest downward exit. It assigns each candidate horizontal direction a cost, searches for a reachable drop, and only spreads into directions with the lowest cost.
- Water state is more than occupied/not occupied plus source/not source. Level, falling/downward state, still-vs-flowing state, and queued updates affect behavior and rendering.
- Flow direction for rendering should be derived from the final fluid state and neighboring fluid states, not hard-coded per face or per texture.
- Cross-chunk flow needs world-coordinate fluid ticks and cross-chunk neighbor notifications. One-shot boundary writes are not enough.
- Water rendering needs its own mesh rules, direction-aware top UVs, translucent ordering, and a camera submersion path. Underwater fog/overlay are separate from the water mesh.
- A block update system is the right foundation for Minecraft-like water. It does not need to be as broad as Minecraft's full redstone/block-update machinery at first, but all block/fluid edits need to route through it.

## Current Repo Snapshot

Propagation:

- `src/world/chunk/mod.rs:894` implements `Chunk::step_fluids` by copying the whole chunk, clearing old fluid cells, then writing the next state from the old state.
- `src/world/chunk/mod.rs:924` flows downward first. If water can occupy the cell below, it writes falling water and skips horizontal spread for that source cell.
- `src/world/chunk/mod.rs:929` then decays level and spreads to every local horizontal neighbor that can be occupied.
- `src/world/chunk/mod.rs:963` runs source creation after the bulk propagation pass. This pass is chunk-local.
- `src/world/chunk/fluid.rs:50` steps fluids every 5 fixed ticks, matching the Minecraft water spread rate of 1 block per 5 game ticks.
- `src/world/chunk/fluid.rs:78` only collects cross-chunk boundary flows when a local boundary cell changed in that step.
- `src/world/chunk/fluid.rs:164` applies boundary writes only to already-loaded target chunks. If the neighbor chunk is missing, the boundary flow is dropped.

Rendering:

- `src/block/mod.rs:318` maps water top/bottom to `water_still.png` and sides to `water_flow.png`.
- `src/block/mod.rs:385` tints water `#55B8FF` with alpha `0.62`.
- `src/world/chunk/mesh/mod.rs:486` culls translucent same-cell faces, including water-water faces.
- `src/world/chunk/mesh/mod.rs:517` computes water top corner heights by averaging this cell with water neighbors around each corner.
- `src/world/chunk/mesh/binary.rs:592` marks sloped top water as using the flow texture.
- `src/world/chunk/mesh/vertex_pulling.rs:82` packs water corner heights into face descriptors, but there is no flow vector or UV orientation data.
- `assets/shaders/vertex_pulling.wgsl:251` uses world-aligned UVs. Top-water flow texture selection exists, but rotation/scroll is not direction-aware.
- `assets/shaders/vertex_pulling.wgsl:285` discards any texel with alpha below `0.5`, even in the translucent pipeline. That is correct for cutout materials but risky for blended water if the water texture contains alpha variation.
- `src/world/chunk/mesh/vertex_pulling.rs:692` creates a translucent pipeline with alpha blending and depth writes disabled.
- `src/world/chunk/mesh/vertex_pulling.rs:1332` sorts translucent chunks by chunk center, not by face or material depth.

Likely causes of the reported issues:

- Clear gaps can come from stale or missing neighbor chunk halos, same-water face culling combined with mismatched corner heights, alpha discard in the water shader, or translucent sorting artifacts.
- The flowing texture is static because no flow direction is stored or derived for the shader. The shader samples the same world-aligned UV frame regardless of current.
- Cross-chunk flow is flaky because boundary propagation is conditional on a local boundary change, dropped if the neighbor is unloaded, and not replayed when the neighbor loads.
- Uniform spread is a direct result of the current `for horizontal_neighbors` fan-out. Minecraft does not do that when a nearby drop exists.

## Minecraft Water Propagation

### State Model

Modern Java water has a fluid state, not just a block id.

- There are still/source and flowing fluid types.
- Flowable fluids have a `LEVEL` property and a `FALLING` boolean property.
- A source/still water cell is full for simulation purposes.
- Flowing water has levels. In the user-facing blockstate, `level=0` is source, `level=1..7` are increasing distance/emptiness, and `level=8` is falling. Internally, Yarn's `FlowableFluid` exposes amount-like levels where higher is fuller.
- `FALLING=true` matters. It marks downward/falling flow and contributes to current/rendering behavior.
- Source layout alone is not enough to reproduce behavior. The hidden fluid state and scheduled ticks matter.

For this codebase, pick one internal convention and stick to it. The current `FluidState` uses source/full as level `8` and lower values as weaker flow. That can work if we add an explicit `falling` flag and avoid mixing it up with Minecraft's visible blockstate `level` naming.

### Tick And Update Model

Minecraft fluids update through scheduled fluid ticks and block/neighbor updates.

- Water spreads at 1 block per 5 game ticks.
- A fluid tick recomputes the fluid state at a position from current neighbors.
- If the state changes, the block/fluid state is written, neighboring blocks are notified, and more fluid ticks are scheduled.
- Block placement/breaking near fluid causes neighbor updates, which schedule affected fluid cells.
- Generated structures can leave water unupdated because generation does not always send normal neighbor updates. This is why Minecraft can have water glitches until something updates the area.

This means a correct architecture should not scan and rewrite every active fluid cell as the main algorithm. It should schedule fluid work where something changed or where a fluid tick says more spreading is needed.

### Downward Priority

Downward flow has priority.

- On a fluid tick, Minecraft first checks whether water can flow into the block below.
- If it can, the water creates falling water below.
- Horizontal spreading is considered when downward flow is blocked or after the current falling update rules allow it.
- Falling water can continue downward indefinitely until blocked.
- Flat horizontal spread is limited to 7 blocks from a source.

The current implementation already has a rough downward-first rule, but the rest of the update model is too broad and chunk-local.

### Nearest-Drop Horizontal Spread

Minecraft does not spread horizontally to all open sides when a nearby drop exists.

For each horizontal candidate direction:

- Start with a high cost, commonly described as `1000`.
- If the candidate itself can flow downward, its cost is `0`.
- Otherwise, search outward from that candidate for a downward path.
- Water searches up to 4 additional horizontal steps from the candidate. Since the candidate is already one step away from the origin, the visible preference reaches drops up to about 5 blocks from the source.
- Only directions with the lowest cost receive horizontal spread.
- If no direction finds a drop, all valid directions tie at the high cost and water spreads outward normally.

Pseudocode for the horizontal direction choice:

```rust
fn spread_dirs(origin: IVec3, state: FluidState, world: &WorldView) -> Vec<Direction> {
    let mut best_cost = INF;
    let mut dirs = Vec::new();

    for dir in HORIZONTAL_DIRS {
        let candidate = origin + dir;
        if !can_flow_into(origin, candidate, dir, world) {
            continue;
        }

        let cost = if can_flow_down_from(candidate, world) {
            0
        } else {
            min_drop_distance(candidate, world, 1, dir.opposite(), MAX_DROP_SEARCH)
        };

        if cost < best_cost {
            best_cost = cost;
            dirs.clear();
        }
        if cost == best_cost {
            dirs.push(dir);
        }
    }

    dirs
}
```

Important implementation details:

- The search must use world coordinates and be allowed to cross chunk boundaries.
- The search must treat blocks that can be waterlogged/replaced/destroyed according to the block's flow/collision shape. For our first rewrite, air/non-solid vs solid may be enough.
- The search should avoid immediately going back through the direction it came from.
- Deterministic direction ordering matters for transient behavior and test reproducibility.

### Source Creation

Java water source conversion is controlled by the `waterSourceConversion` gamerule, enabled by default.

The relevant rules:

- A flowing water block can become a source if it is horizontally adjacent to two or more water sources and has a solid block or water source below.
- A flowing water block can also become a source with one horizontal source and one source above, again with acceptable support below.
- Waterlogged sources can count if their block exposes water flow on that side. This can be ignored until waterlogging exists here.

The current implementation has a similar source creation pass but it only checks local chunk neighbors and support. The rewrite should evaluate the same rule through a world-neighbor query so it works on chunk borders.

### Same Layout, Different Behavior

The user's suspicion is correct if "layout" means block occupancy and source positions only.

Two worlds can have the same visible occupied water blocks and source positions but differ because of:

- Flowing `LEVEL` values.
- `FALLING` state.
- Still vs flowing fluid type.
- Scheduled ticks that have not fired yet.
- Generated/unupdated fluid states.
- Update order during transient competition between multiple flows.

However, if every block state, fluid state, and scheduled tick is identical, and all relevant chunks are loaded, modern Java behavior should be deterministic and usually converge for simple layouts.

### Cross-Chunk Requirements

Minecraft fluid logic is local, but it is not chunk-local.

- Neighbor checks cross chunk borders.
- Source creation can depend on source neighbors across a border.
- The nearest-drop search can inspect up to about 5 horizontal blocks from the origin, crossing chunk borders.
- Scheduled ticks are associated with loaded chunks. If a target chunk is unloaded or outside simulation distance, flow may stall until the chunk loads.
- On chunk load, borders need reconciliation so missed updates are scheduled instead of forgotten.

For this engine, never drop a cross-chunk fluid event silently. If the target chunk is unavailable, store a pending border notification or dirty edge marker and replay/reconcile on load.

## Minecraft Water Rendering

### Mesh Shape And Face Rules

Vanilla-style water is rendered as a fluid mesh, not as a full cube.

- Top surface height comes from fluid level and neighbor samples.
- If the same fluid exists above, the local height is full.
- Otherwise source/still water renders just below full block height. Vanilla uses a maximum visible top height around `8/9` for fluid without same-fluid above.
- Corner heights are smoothed from nearby fluid heights. The important property is that both chunks compute the same height for a shared edge.
- Internal faces between same fluid are culled.
- Faces against solid/full occluding blocks are culled.
- Faces against air, different fluids, or different translucent materials are emitted.
- Same-fluid side faces are normally culled even when adjacent fluid levels differ, so the top surfaces must share edge heights or cracks appear.
- Water is effectively visible from both inside and outside on top/sides, so do not rely blindly on backface culling for water surfaces.

The current corner height averaging is in the right family, but it is fragile if neighbor chunk halos are missing or stale. The rewrite should make mesh input snapshots include a reliable one-block halo, and should force both chunks to rebuild when border fluid changes.

### Still Vs Flowing Texture

Vanilla chooses texture and UV behavior from current/velocity.

- Still top water uses the still sprite.
- Flowing top water uses the flowing sprite.
- Side faces generally use the flowing sprite.
- Flowing top UVs are rotated/oriented by the horizontal flow vector.
- The flow texture is animated. Texture animation alone is not enough; top UVs also need direction.

For this engine, the mesh or shader needs enough data to orient water UVs. Options:

- Pack a quantized flow direction/angle into `FaceDescriptor` for water faces.
- Pack a 2D signed flow vector in a water-specific descriptor buffer.
- Derive flow in the shader from neighboring fluid levels stored in a texture/SSBO. This is more complex and probably not worth it now.

The minimal practical choice is to derive flow during meshing from the fluid snapshot and pack a quantized direction plus a still/flowing bit.

### Flow Vector

Water current and water texture orientation are derived from neighboring fluid states.

Minecraft current is not just a cosmetic value. It also pushes entities, and the visible flowing texture is aligned from the same kind of directional information. The current can differ between worlds that have the same visible water occupancy if their hidden fluid levels, falling states, or update history differ.

Approximate model for this engine:

- Convert each fluid cell to a visible/physical height from its level and falling/source state.
- Compare the current cell against the four horizontal neighbors.
- Add a vector toward lower fluid height or empty flowable space.
- Include incoming higher neighbors when deriving entity current if/when gameplay current is implemented.
- If the cell is falling, mark downward current. For top texture orientation, use only the horizontal part.
- If horizontal vector length is near zero, use the still texture and unrotated UVs.

This is not a byte-for-byte clone of `FlowableFluid.getVelocity`, but it fixes the current one-direction rendering issue and can be made closer later.

### Translucent Pipeline

Opaque terrain behind water should be visible because opaque terrain renders first, writes color/depth, then water renders with blending and depth test on.

Baseline water pass:

- Render opaque blocks first with depth writes on.
- Render cutout blocks with depth writes on and discard/alpha test as needed.
- Render water/translucent after opaque with depth test on and depth writes off.
- Use alpha blending, usually `SRC_ALPHA, ONE_MINUS_SRC_ALPHA` for straight alpha or `ONE, ONE_MINUS_SRC_ALPHA` for premultiplied alpha.
- Sort translucent draw packets back-to-front. Chunk-center sorting is a first approximation, but large water surfaces and adjacent chunk overlaps will still artifact.
- Do not apply a cutout alpha discard to water except perhaps for nearly-zero alpha. The current global `tex_color.a < 0.5` discard can turn texture alpha into holes.

If we want water to distort or refract meshes behind it, alpha blending is not enough. A screen-space water pass needs:

- An opaque scene color texture copied/resolved before water draws.
- The opaque depth texture or a depth copy.
- Water shader sampling the scene color with a small normal/UV distortion.
- Optional depth-based absorption by comparing water surface depth to scene depth.
- Care to avoid sampling from the same color target currently being rendered into.

Minecraft Java's classic water look is mostly animated texture, tint, translucency, light/fog, and underwater overlay/fog. Full refraction is optional for this project rather than required for Minecraft-like behavior.

### Camera Submersion

Underwater rendering should be driven by camera/eye submersion, not by whether a water face happens to be between the camera and terrain.

Detection:

- Sample the fluid cell containing the camera eye position.
- Compute that cell's visible fluid height using the same fluid height function as the mesh.
- The camera is submerged if `eye_y` is below the fluid surface height in that cell.
- Falling columns and water above the camera should count correctly.

Rendering effects:

- Apply underwater fog to all terrain when the camera is submerged.
- Use water fog color, ideally biome-tinted later. Java default water color is `#3F76E4`; default water fog color is much darker.
- Vanilla has underwater fog behavior that changes over time and is affected by effects such as Night Vision/Conduit Power. We only need a simple static fog first.
- Optionally draw an underwater overlay/noise/wave texture in screen space.
- Render water surfaces two-sided or with inside-visible faces so looking out of water does not produce missing planes.

This should be a render-view state/uniform, for example `terrain_visuals.camera_submersion = None | Water`, plus water fog parameters. Do not rely on the water mesh shader alone to fog opaque terrain behind the water.

## Recommended Rewrite Architecture

### World Write Path

Create one authoritative block/fluid write path, even if it is thin at first.

Required behavior:

- `set_cell(world_pos, new_cell, flags)` writes block/fluid state in world coordinates.
- If the state changes, enqueue neighbor notifications for the six adjacent cells in fixed order.
- If the position is on a chunk edge, notify/dirty the adjacent chunk too.
- Schedule fluid ticks for the changed cell, adjacent fluid cells, and adjacent cells that may accept fluid.
- Mark render mesh and lighting dirty for the changed chunk/section and any neighbor section sharing a face.

This is the smallest useful "block update system" for water. It can be extended later for redstone, plants, physics blocks, and waterlogging.

### Scheduled Fluid Ticks

Use a deterministic scheduled tick queue.

Suggested tick record:

```rust
struct ScheduledFluidTick {
    time: u64,
    pos: IVec3,
    fluid_type: FluidType,
    priority: TickPriority,
    sequence: u64,
}
```

Queue rules:

- Sort by `time`, `priority`, `sequence`, then world position if needed.
- Dedupe pending ticks by `(pos, fluid_type)` only if dedupe does not suppress needed rechecks.
- Revalidate current state when the tick fires. Stale ticks should do nothing, not overwrite newer player edits.
- Persist queued ticks if practical. If not, rebuild/reseed fluid ticks by scanning loaded fluid cells and chunk borders on load.
- Keep a per-frame/per-fixed-tick budget, but defer overflow without changing order.

### Fluid Tick Algorithm

A practical rewrite can use this sequence:

```rust
fn fluid_tick(pos: IVec3, ty: FluidType, world: &mut World) {
    let old = world.fluid(pos);
    let updated = compute_updated_fluid_state(pos, ty, world);

    if updated != old {
        world.set_fluid(pos, updated);
        world.notify_neighbors(pos);
        world.schedule_fluid_neighbors(pos, ty, WATER_TICK_DELAY);
    }

    if updated.is_empty() {
        return;
    }

    if try_flow_down(pos, updated, world) {
        world.schedule_fluid_tick(pos.below(), ty, WATER_TICK_DELAY);
        return;
    }

    for dir in spread_dirs(pos, updated, world) {
        if try_flow_side(pos, dir, updated, world) {
            world.schedule_fluid_tick(pos + dir, ty, WATER_TICK_DELAY);
        }
    }
}
```

Key points:

- `compute_updated_fluid_state` should handle source creation, losing support/replenishment, and level decay based on neighbors.
- `try_flow_down` writes a falling state below if possible.
- `spread_dirs` implements nearest-drop preference.
- Writes should call the same world write/update path as player edits.
- The algorithm must query neighbors through a world/chunk API, not local chunk arrays.

### Cross-Chunk Flow

Replace one-shot boundary writes with normal world-coordinate ticks.

Rules:

- A fluid tick may read and write any loaded neighboring chunk.
- If a target chunk is unloaded, record a pending border notification for that chunk edge.
- On chunk load, reconcile the one-block border against loaded neighbors and schedule ticks for both sides where fluid could change.
- When a chunk unloads, persist its fluid state and either persist scheduled ticks or mark its borders for reseed on reload.
- Mesh dirtying must affect both chunks when a fluid change touches a boundary.
- World-to-chunk conversion must use correct floor division for negative coordinates.

This directly addresses the current dropped-outflow failure mode.

### Mesh Snapshots

Mesh jobs should read immutable chunk snapshots with halos.

Requirements:

- Include the center chunk plus a one-block neighbor halo for fluid level, falling/source, block opacity, and render id.
- If any halo chunk is missing, choose a conservative temporary behavior and force rebuild when it appears.
- Drop stale mesh jobs if the chunk version changes while building.
- Dirty adjacent chunks/sections when border fluid changes.
- Compute water corner heights using the same samples on both sides of a boundary.

### Water Mesh Descriptor Data

The current descriptor packs corner heights and a top-flowing bit. The rewrite likely needs:

- Four corner heights, preferably still packed in 4-bit values if using 0..=8 or 0..=15.
- A still/flow texture bit per face.
- A quantized top-flow direction, such as 16 directions plus `none`.
- Optional downward/falling flag for side texture behavior.
- Optional water material/tint index for biome water later.

If descriptor bits are too tight, split water into a water-specific descriptor path instead of forcing all water data into the general block face descriptor.

### Shader Changes

Minimum shader work for direction-aware water:

- Make alpha discard material/layer-specific. Cutout can discard at `0.5`; water should not.
- Add water flow direction data to the vertex/fragment path.
- For top faces, rotate or transform UVs around the block center according to flow direction.
- Animate water textures by frame metadata as now, or add UV scrolling if desired.
- Keep depth writes disabled for water.
- Add camera-submersion uniforms for fog color/range and overlay state.

Suggested simple top-UV behavior:

```wgsl
let centered = fract(world_xz) - vec2(0.5, 0.5);
let rotated = mat2x2(cos_a, -sin_a, sin_a, cos_a) * centered;
let water_uv = rotated + vec2(0.5, 0.5);
```

Later, a screen-space refraction pass can sample opaque scene color/depth, but do not block the propagation rewrite on that.

## Suggested Rewrite Order

1. Add/reshape fluid state so water has `type`, `level/amount`, `source`, and `falling` semantics with one clear internal convention.
2. Add a world-coordinate block/fluid write path that emits neighbor updates and mesh dirty marks.
3. Add deterministic scheduled fluid ticks and use them for water instead of chunk-wide active fluid scanning.
4. Implement Minecraft-like water tick rules: downward first, nearest-drop horizontal spread, source conversion, and stale tick revalidation.
5. Replace boundary outflow code with cross-chunk world-coordinate tick/query/write logic plus pending border reconciliation.
6. Update water meshing to use reliable fluid snapshots/halos and rebuild both chunks on border changes.
7. Pack/derive flow direction and rotate flowing top UVs in the shader.
8. Split cutout alpha discard from blended water alpha.
9. Add camera submersion detection and underwater fog/overlay uniforms.
10. Only then consider screen-space refraction or better transparent ordering if visual artifacts remain.

## Sources

- Minecraft Wiki, Water: `https://minecraft.wiki/w/Water`
- Minecraft Wiki, Fluid: `https://minecraft.wiki/w/Fluid`
- Yarn `FlowableFluid`: `https://maven.fabricmc.net/docs/yarn-1.21.5+build.1/net/minecraft/fluid/FlowableFluid.html`
- Yarn `WaterFluid`: `https://maven.fabricmc.net/docs/yarn-1.21.5+build.1/net/minecraft/fluid/WaterFluid.html`
- Yarn `FluidRenderer`: `https://maven.fabricmc.net/docs/yarn-1.21.4+build.8/net/minecraft/client/render/block/FluidRenderer.html`
- Yarn `FluidState`: `https://maven.fabricmc.net/docs/yarn-1.21.4+build.8/net/minecraft/fluid/FluidState.html`
- Yarn `RenderLayer`: `https://maven.fabricmc.net/docs/yarn-1.21.4+build.8/net/minecraft/client/render/RenderLayer.html`
- Yarn `BackgroundRenderer`: `https://maven.fabricmc.net/docs/yarn-1.21.4+build.8/net/minecraft/client/render/BackgroundRenderer.html`
- Yarn `CameraSubmersionType`: `https://maven.fabricmc.net/docs/yarn-1.21.4+build.8/net/minecraft/block/enums/CameraSubmersionType.html`
- Yarn `InGameOverlayRenderer`: `https://maven.fabricmc.net/docs/yarn-1.21.4+build.8/net/minecraft/client/gui/hud/InGameOverlayRenderer.html`
- Yarn `WorldTickScheduler`: `https://maven.fabricmc.net/docs/yarn-1.21.4+build.8/net/minecraft/world/tick/WorldTickScheduler.html`
- Yarn `ChunkTickScheduler`: `https://maven.fabricmc.net/docs/yarn-1.21.4+build.8/net/minecraft/world/tick/ChunkTickScheduler.html`
- LearnOpenGL, Blending: `https://learnopengl.com/Advanced-OpenGL/Blending`
- LearnOpenGL, Weighted Blended OIT: `https://learnopengl.com/Guest-Articles/2020/OIT/Weighted-Blended`
