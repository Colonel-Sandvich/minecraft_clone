# Vertex Pulling — Evaluation Plan

## Summary

Replace per-vertex attribute buffers with a compact GPU face-descriptor buffer.
The vertex shader reconstructs position, normal, and AO from the descriptor
using `@builtin(vertex_index)`.  CPU→GPU transfer shrinks from ~2 MB to ~40 KB
per chunk.

We want smooth block lighting (0–15, interpolated across faces) only.

## Greedy meshing with smooth lighting

Greedy merges N×M faces into one quad with only 4 corner vertices.  The 3D
light texture provides correct per-vertex light values at those 4 corners.
The GPU bilinearly interpolates between them across the merged area.

Whether this is correct depends entirely on the merge condition:

### Per-face light as a merge condition (works)

Add the owning block's light level to the merge conditions alongside block
type and AO key.  Two faces only merge if all three match.

In a light gradient (torch at one end, light falling 14→4 across 16 blocks),
no two adjacent faces have the same per-face light → **no merge happens**.
The 16 faces render individually, each sampling the 3D texture at their 4
corners.  Completely accurate.

In uniform light (sunny plain, sky light 15 everywhere), all faces have the
same per-face light → **big merges happen**.  The merged quad's 4 corners all
sample light 15 from the 3D texture.  Bilinear interpolation of uniform 15
gives 15 everywhere.  Also completely accurate.

The merge only succeeds when light is uniform across the merge area.  In that
case interior variation can't exist, so bilinear interpolation can't be wrong.

Contrast with Strategy A (non-greedy + 3D texture): no merging, every face
renders individually with 4 independent light samples per face.  Always
accurate, but more GPU vertices.

This is worth benchmarking — the per-face-light merge condition naturally
gives big quads where geometry is uniform AND light is uniform, and individual
faces everywhere else.

## Current baseline (for comparison)

```
Per chunk (~5000 faces, current greedy):
  Vertex buffer:  5 attrs × 4 verts × 5000 faces ≈ 400 KB
  Index buffer:   6 indices × 4 bytes × 5000      ≈ 120 KB
  Total:          ~520 KB/chunk

  Lighting:       static (AO × face_brightness baked into vertex colours)
  Draw:           indexed, standard vertex pipeline
```

## Face descriptors

### Non-greedy (8 bytes)

```
u32 packed:   x(5)|z(5)|y(5)|face_dir(3)|pad(4) = 22 bits
u32 info:     block_type(8)|ao_key(8)|pad(16)
```

- 20 pad bits
- No width/height since no merging
- Light comes from 3D texture sampled per-vertex in the shader

### Greedy (8 bytes)

```
u32 packed:   x(5)|z(5)|y(5)|face_dir(3)|pad(4)     = 22 bits
u32 info:     block_type(8)|ao_key(8)|width(4)|height(4) = 24 bits
```

- `width, height`: merge dimensions 1..16 (4 bits each)
- Per-face light checked at mesh time as a merge condition; NOT stored in the
  descriptor (the 3D texture provides per-vertex light)
- Same 8 bytes as non-greedy — width/height fits in the info pad bits

A 5000-face chunk = 40 KB of descriptors in either case.  The `ao_key` is
geometry-based (neighbor opacity) — stays in the descriptor regardless of
lighting method.

## Three smooth-lighting strategies

All strategies use a 3D light texture for per-vertex smooth lighting.  They
differ in whether they greedy-merge faces.

### Strategy A: Non-greedy + 3D texture (the Sodium approach)

Per-chunk `texture_3d<r8uint>` (18³ × 1 byte = 5,832 bytes):

```
bits 0..3 = block_light (0..15)
bits 4..7 = sky_light   (0..15)
```

Vertex shader samples at each vertex corner's world position:

```wgsl
let texel = textureLoad(light_tex, local_pos, 0);
let block_light = f32(texel & 0xFu) / 15.0;
let sky_light   = f32((texel >> 4) & 0xFu) / 15.0;
let combined = sky_light * day_night_factor + block_light;
```

GPU interpolates combined light across the face (4 different values for 4
vertices → smooth gradient within each individual face).

**Why sky + block separately:** sky light is multiplied by the day/night
factor (0.0–1.0).  Block light is not.  A torch-lit cave must look the same
at noon and midnight.  `max(sky, block)` is for mob spawning logic — wrong for
rendering.

| Pros | Cons |
|------|------|
| Light updates don't touch meshes | 5.7 KB static per chunk |
| Vertex shader texture load is nearly free | 2.3 MB at 400 visible chunks |
| Sky light × day/night is trivial in shader | Separate wgpu resource per chunk |
| Each face's 4 vertices sample independently → accurate | |

### Strategy B: 4-corner light in descriptor

No 3D texture.  16 extra bits carry 4 corner light values:

```
u32 packed:   x(5)|z(5)|y(5)|face_dir(3)|pad(4)     = 22 bits
u32 info:     block_type(8)|ao_key(8)|light_corners(16) = 32 bits
```

10 bytes per face (block light only).  12 bytes if adding sky light (another
16 bits).  Vertex shader reads directly, no texture sample needed.

| Pros | Cons |
|------|------|
| No texture per chunk | Light updates require rewriting descriptor buffer |
| One fewer bind group | 10–12 bytes/face vs 8 + texture |
| | Crossover: texture wins at > 1,458 faces/chunk |

**Crossover:** 5,832 / (12 − 8) = 1,458 faces (for block + sky).  Typical
chunks have 3,000–5,000 faces → texture wins for nearly all non-empty chunks.
Only near-empty sky chunks come out ahead with descriptor packing.

### Strategy C: Greedy + 3D texture + per-face-light merge condition

Greedy meshes faces into merged quads, but only when **block type**, **AO key**,
and **per-face owning-block light** all match.  Per-vertex light still comes
from the 3D texture sampled at each corner.

The per-face light is the light level of the block the face belongs to (not a
per-vertex value).  This is computed at mesh time alongside the AO key:

```rust
// In the greedy extension loop:
if block_light_of_owning_block(np) != base_block_light { break; }
```

In a light gradient (torch 14→4 across 16 blocks), adjacent faces have
different owning-block lights → **no merge**.  Each face renders individually
with 4 correct corner samples from the 3D texture.  Accurate.

In uniform sky light (all blocks at 15), all faces have the same owning-block
light → **big merges**.  All 4 corners sample light 15 from the 3D texture.
Bilinear interpolation of uniform values is exact.  Also accurate.

The merge only succeeds when light and geometry are both uniform across the
merge area.  If either varies, merges break and faces render individually.

| Pros | Cons |
|------|------|
| Big quads where possible | Extra merge condition reduces merge rate |
| 8-byte descriptor (same as non-greedy) | Light updates still need remesh if per-face light changes |
| + width/height in pad bits — no size increase | Per-face light value NOT stored in descriptor (implied by 3D texture) |
| Accurate at all merged quad corners | Interior bilinear interpolation assumes uniformity (holds by merge condition) |

**Note:** light updates that change a block's light level will invalidate the
merge condition for any merged quad containing that block → remesh needed.
If a light update makes two previously-different blocks have the same light,
they could now merge, but that also requires remesh.  This is the same as
today's block-change remesh trigger.

Greedy descriptor (8 bytes, width/height in info pad):

```
u32 packed:   x(5)|z(5)|y(5)|face_dir(3)|pad(4)     = 22 bits
u32 info:     block_type(8)|ao_key(8)|width(4)|height(4) = 24 bits
```

### Recommendation

Start with Strategy A (non-greedy + 3D texture) — simplest, light updates
don't touch meshes.  Then benchmark Strategy C (greedy + per-face-light merge
condition) to see if the merge wins justify the extra complexity and the remesh
on light change.

Strategy B (4-corner descriptor) only if the 3D texture per-chunk overhead
proves problematic, which is unlikely.

## WGSL vertex shader

```wgsl
struct FaceDescriptor {
    packed: u32,
    info: u32,
}

@group(1) @binding(0) var<storage, read> faces: array<FaceDescriptor>;
@group(1) @binding(1) var light_tex: texture_3d<u32>;

// Map 6 triangle vertices → 4 quad corners (indices 0,1,2, 1,2,3)
const TRI_TO_QUAD: array<u32, 6> = array(0u, 2u, 1u, 1u, 2u, 3u);

// 6 faces × 4 corners × 3 axes — same table as VERTEX_OFFSETS in quad.rs
const CORNER_OFFSETS: array<array<vec3<f32>, 4>, 6> = array(
    /* Left    */ array(vec3(0,0,1), vec3(0,0,0), vec3(0,1,1), vec3(0,1,0)),
    /* Right   */ array(vec3(1,0,0), vec3(1,0,1), vec3(1,1,0), vec3(1,1,1)),
    /* Down    */ array(vec3(0,0,1), vec3(1,0,1), vec3(0,0,0), vec3(1,0,0)),
    /* Up      */ array(vec3(0,1,1), vec3(0,1,0), vec3(1,1,1), vec3(1,1,0)),
    /* Forward */ array(vec3(0,0,0), vec3(1,0,0), vec3(0,1,0), vec3(1,1,0)),
    /* Back    */ array(vec3(1,0,1), vec3(0,0,1), vec3(1,1,1), vec3(0,1,1)),
);

const NORMALS: array<vec3<f32>, 6> = array(
    vec3(-1,0,0), vec3(1,0,0), vec3(0,-1,0), vec3(0,1,0), vec3(0,0,-1), vec3(0,0,1),
);

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) block_type: u32,
    @location(3) ao: f32,      // single AO value for this vertex
    @location(4) light: f32,    // interpolated by GPU
}

// For greedy: which axis gets scaled by width/height per face direction
// face_dir: 0=Left,1=Right,2=Down,3=Up,4=Forward,5=Backward
// axis index: 0=X, 1=Y, 2=Z
const WIDTH_AXIS:  array<u32, 6> = array(2u, 2u, 0u, 0u, 0u, 0u);
const HEIGHT_AXIS: array<u32, 6> = array(1u, 1u, 2u, 2u, 1u, 1u);
const NORMAL_AXIS: array<u32, 6> = array(0u, 0u, 1u, 1u, 2u, 2u);

@vertex
fn vertex(@builtin(vertex_index) vid: u32) -> VertexOutput {
    let face_idx = vid / 6u;
    let corner_raw = vid % 6u;
    let qi = TRI_TO_QUAD[corner_raw];

    let desc = faces[face_idx];
    let x = (desc.packed >> 27) & 0x1Fu;
    let z = (desc.packed >> 22) & 0x1Fu;
    let y = (desc.packed >> 17) & 0x1Fu;
    let face_dir = (desc.packed >> 14) & 0x7u;

    let block_type = desc.info & 0xFFu;
    let ao_key = (desc.info >> 8) & 0xFFu;
    let width  = max(f32((desc.info >> 16) & 0xFu), 1.0);   // 0 → 1 for non-greedy
    let height = max(f32((desc.info >> 20) & 0xFu), 1.0);   // 0 → 1 for non-greedy

    let wa = WIDTH_AXIS[face_dir];
    let ha = HEIGHT_AXIS[face_dir];
    let na = NORMAL_AXIS[face_dir];

    let offset = CORNER_OFFSETS[face_dir][qi];
    // offset values are 0 or 1. Scale width/height axes by merge dimensions.
    var local_pos = vec3<f32>(f32(x), f32(y), f32(z));
    local_pos[wa] += offset[wa] * width;
    local_pos[ha] += offset[ha] * height;
    local_pos[na] += offset[na];  // normal axis: always 0 or 1, unscaled
    let world_pos = (model * vec4(local_pos, 1.0)).xyz;

    // Sample light texture at this vertex corner's padded chunk position
    let texel = textureLoad(light_tex, ivec3(local_pos), 0);
    let block_light = f32(texel & 0xFu);
    let sky_light = f32((texel >> 4) & 0xFu);
    let light = (sky_light * day_night_factor + block_light) / 15.0;

    // Unpack AO for this corner
    let ao_shift = qi * 2u;
    let ao_val = (ao_key >> ao_shift) & 0x3u;
    let ao = AO_BRIGHTNESS[ao_val];

    return VertexOutput(
        view_proj * vec4(world_pos, 1.0),
        world_pos,
        NORMALS[face_dir],
        block_type,
        ao,
        light,
    );
}
```

## Fragment shader (largely unchanged from today)

```wgsl
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // UV from world_pos + normal (same as current block_material.wgsl)
    let n = abs(in.world_normal);
    let wp = in.world_pos.xyz;
    var face_uv: vec2<f32>;
    if n.x > 0.5 { face_uv = vec2(wp.z, 1.0 - wp.y); }
    else if n.y > 0.5 { face_uv = vec2(wp.x, wp.z); }
    else { face_uv = vec2(wp.x, 1.0 - wp.y); }

    let block_uv = fract(face_uv);
    let atlas_uv = tile_offsets[in.block_type] + block_uv * material.tile_size;

    let tex_color = textureSample(atlas, sampler, atlas_uv);
    let tint = block_colors[in.block_type];

    let face_factor = face_brightness(in.world_normal);
    let final_brightness = in.light * in.ao * face_factor;

    let color = tex_color * tint * final_brightness;
    if color.a < 0.5 { discard; }
    return color;
}
```

All tables (`AO_BRIGHTNESS`, `tile_offsets`, `block_colors`, `face_brightness`,
`tile_size`) are uniforms or const arrays.

## Complex blocks (hybrid pipeline)

Stairs, doors, fences, water, slabs, torches, plants → not axis-aligned full
cubes.  These stay on the traditional `MeshBufferBuilder` path.

```rust
fn is_vertex_pullable(block: BlockType) -> bool {
    block.is_full_cube() && !block.is_water()
}
```

Result: **two draw calls per chunk** (three if water gets its own translucent
pass):

```
Draw 0: vertex-pulled   non-indexed  draw(face_count * 6)  descriptor SSBO
Draw 1: traditional      indexed      existing MeshBufferBuilder path
```

Both share the same fragment shader and texture atlas.  Split in the mesher
— pullable faces go into a descriptor `Vec`, non-pullable faces call
`push_face()` as today.

## Bevy integration

No raw wgpu needed.  Bevy 0.18 supports non-indexed draws with no vertex
buffers through `RenderCommand<P>` + `BinnedRenderPhaseType::NonMesh`.

### Recipe

1. **Custom `RenderCommand`:**
```rust
struct DrawVertexPulled;
impl<P: PhaseItem> RenderCommand<P> for DrawVertexPulled {
    type Param = SRes<VPState>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(_item: &P, _view: (), _entity: Option<()>,
                  state: SystemParamItem<'w, '_, Self::Param>,
                  pass: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        pass.draw(0..state.face_count * 6, 0..1);
        RenderCommandResult::Success
    }
}

type DrawVPCmd = (SetItemPipeline, SetMeshViewBindGroup<0>, DrawVertexPulled);
```

2. **Register:**
```rust
render_app.add_render_command::<Opaque3d, DrawVPCmd>();
```

3. **Queue with `NonMesh`:**
```rust
phase.add(
    Opaque3dBatchSetKey { .., vertex_slab: default(), index_slab: None },
    Opaque3dBinKey { asset_id: AssetId::<Mesh>::invalid().untyped() },
    entity,
    InputUniformIndex::default(),
    BinnedRenderPhaseType::NonMesh,  // skips all vertex buffer lookup
    change_tick,
);
```

The face descriptor buffer and light texture are bound via `AsBindGroup`:

```rust
#[derive(AsBindGroup)]
struct VertexPullingMaterial {
    #[storage(3, read_only)]
    face_descriptors: Handle<ShaderStorageBuffer>,
    #[texture(4)]
    light_texture: Handle<Image>,
    // ... common with BlockMaterial: atlas texture, tile_size
}
```

### Per-chunk render entities

Currently: each chunk spawns 2 child entities (opaque + cutout materials).
Under vertex pulling:
- Pullable opaque faces → 1 entity with `VertexPullingMaterial` (descriptor SSBO + light texture)
- Pullable cutout faces → 1 entity with `VertexPullingMaterial` (separate descriptor SSBO)
- Complex opaque → 1 entity with `BlockMaterial` (traditional mesh, unchanged)
- Complex cutout → 1 entity with `BlockMaterial` (traditional mesh, unchanged)

Up to 4 draw calls per chunk, but most chunks have zero complex blocks →
usually 2 draws.

## Implementation phases

### Phase 1 — Non-greedy vertex pulling, no lighting

- Goal: prove the vertex-pulled draw path works end-to-end.
- Single descriptor buffer per material layer, non-indexed draw.
- Fragment shader: texture lookup + AO from descriptor.  No light
  multiplication (use existing static face brightness or a constant).
- Benchmarks: GPU memory vs current, frame time at varying view distances.

### Phase 2 — 3D light texture + basic light propagation

- Per-chunk 18³ × 1-byte 3D texture (`r8uint`, block + sky packed).
- Simple flood-fill light propagation:
  - Sky light: vertical sweep top-down from heightmap.
  - Block light: BFS from torches / glowstone with max-distance cap.
- Vertex shader samples light texture, fragment shader multiplies it in.
- Benchmarks: visual quality, light-update cost (no remesh).

### Phase 3 — Hybrid pipeline

- `is_vertex_pullable()` classification.
- Patch the mesher to emit descriptors instead of calling `push_face()` for
  pullable blocks.
- Existing blocks that aren't pullable stay on `MeshBufferBuilder`.
- Benchmark: two-draw overhead vs single draw.

## What changes from today

| Aspect | Current | Vertex pulling |
|--------|---------|---------------|
| Per-face GPU memory | ~400 bytes (4 verts × 100 bytes) | 8 bytes |
| Per-chunk static GPU memory | 0 | 5.7 KB (light texture) |
| Chunk mesh rebuild | Recompute all vertex attributes | Append to descriptor vec |
| Light update | Full remesh required | Write 3D texture, no remesh |
| Draw type | Indexed | Non-indexed `draw(N * 6)` |
| AO | Baked into vertex colours | In descriptor, unpacked in shader |
| Greedy meshing | Yes (for solid blocks) | Dropped |
| Face brightness | Baked into vertex colours | Uniform table in shader |
| Vertex count / chunk | ~20K for 5000 faces | 0 (generated in shader from 5000 descriptors) |

## Benchmarking plan

Per configuration, measure at view distances 4, 8, 12, 16:

| Metric | How |
|--------|-----|
| Face descriptors / chunk | Count from mesher output |
| GPU memory / chunk | Descriptor buffer + light texture + traditional vertex buffers |
| Mesh build time | CPU wall time (mesher + buffer upload) |
| Frame time | GPU timer query or frame-time delta |
| Light update cost | Time to write light texture (Phase 2) vs remesh (current) |

## Open questions

1. **Light propagation.**  Flood-fill across chunk boundaries is the expensive
   part.  How do we bound it?  Sky light = vertical sweep from heightmap
   (cheap).  Block light = BFS with max-distance cap from each source.
   Cross-chunk propagation needs neighbor chunk access.

2. **Two material layers.**  Currently separate child entities with separate
   meshes.  With vertex pulling, each layer gets its own descriptor SSBO.
   Separate buffers is simpler — each draw call binds only its own faces.
   The opaque draw sorts before the cutout draw naturally.

3. **Chunk padding for AO.**  AO is baked into `ao_key` at mesh time from the
   padded block array (18³, includes neighbor chunks).  Faces at chunk edges
   get correct AO from neighbor data.  No change needed from today.

4. **Sky light day/night cycle.**  The `day_night_factor` uniform (0.0–1.0)
   modulates sky light in the vertex shader.  We'd drop the existing
   `DirectionalLight` and `GlobalAmbientLight` in favour of the per-vertex
   sky_light × day_night calculation, OR keep them for the traditional pass
   and add a uniform for the vertex-pulled pass.

5. **Chunk origin transform.**  Descriptors store chunk-local positions (0..16).
   The vertex shader computes `world_pos = model * vec4(local_pos, 1.0)`.
   The per-chunk model matrix comes from Bevy's standard `Transform` — no
   change needed.  The fragment shader receives `world_pos` for UV computation
   (same as today).
