pub mod binary;
mod blocks;
pub mod vertex_pulling;

pub use blocks::ChunkMeshBlocks;
pub use vertex_pulling::{
    ChunkMeshDescriptors, VertexPullingLight, VertexPullingMesh, VpTextureState,
};

use std::sync::Arc;

use bevy::{camera::primitives::Aabb, platform::collections::HashMap, prelude::*, utils::Parallel};
use strum::IntoEnumIterator;

use crate::block::{
    BLOCK_FLAG_CUTOUT, BLOCK_FLAG_EMITS_INTERNAL_FACES, BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED,
    BLOCK_FLAG_TRANSLUCENT, BlockMaterialLayer, BlockTextureLayer, BlockTextureMap, BlockType,
    RENDER_ID_COUNT, WATER_RENDER_ID, from_render_id, render_id_for_block, render_id_to_colour,
};
use crate::quad::Direction;
use crate::textures::{BlockTextures, TextureState};

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkLight, ChunkNeedsLightUpload,
    ChunkNeedsMeshRebuild, ChunkPerfCounters, ChunkPosition, ambient_occlusion::AO_BRIGHTNESS,
};

pub(crate) const PADDED_CHUNK_SIZE: usize = CHUNK_SIZE + 2;
pub(crate) const PADDED_CHUNK_VOLUME: usize =
    PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const PADDED_CHUNK_LAYER_SIZE: usize = PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const DIRECTION_COUNT: usize = 6;
pub(crate) const DIRECTION_INDEX_OFFSETS: [isize; DIRECTION_COUNT] = [
    -1,
    1,
    -(PADDED_CHUNK_LAYER_SIZE as isize),
    PADDED_CHUNK_LAYER_SIZE as isize,
    -(PADDED_CHUNK_SIZE as isize),
    PADDED_CHUNK_SIZE as isize,
];
pub(crate) const VERTEX_AO: [u32; 8] = [3, 2, 2, 0, 2, 1, 1, 0];

#[inline(always)]
pub(crate) fn block_mesh_flags(rid: u16) -> u8 {
    match rid {
        0 => 0,
        WATER_RENDER_ID => BLOCK_FLAG_RENDERED | BLOCK_FLAG_TRANSLUCENT,
        _ => BlockType::from_repr(rid - 1).unwrap().mesh_flags(),
    }
}

#[inline(always)]
pub(crate) const fn material_layer_index_from_flags(flags: u8) -> usize {
    if flags & BLOCK_FLAG_TRANSLUCENT != 0 {
        BlockMaterialLayer::Translucent.index()
    } else if flags & BLOCK_FLAG_CUTOUT != 0 {
        BlockMaterialLayer::Cutout.index()
    } else {
        BlockMaterialLayer::Opaque.index()
    }
}

// Each face has 4 AO corners but only 8 unique sample cells. These macros encode
// the two corner-order layouts used by the six face directions and avoid the old
// 12 block-classification loads per face. `indexed_face_ao_matches_reference...`
// guards the numeric offsets and corner ordering.
macro_rules! ao_key_ab {
    ($blocks:expr, $padded_index:expr, $a0:expr, $a1:expr, $b0:expr, $b1:expr, $c00:expr, $c01:expr, $c10:expr, $c11:expr) => {{
        let a0 = block_occludes_ambient_light_bit($blocks, $padded_index, $a0);
        let a1 = block_occludes_ambient_light_bit($blocks, $padded_index, $a1);
        let b0 = block_occludes_ambient_light_bit($blocks, $padded_index, $b0);
        let b1 = block_occludes_ambient_light_bit($blocks, $padded_index, $b1);
        let c00 = block_occludes_ambient_light_bit($blocks, $padded_index, $c00);
        let c01 = block_occludes_ambient_light_bit($blocks, $padded_index, $c01);
        let c10 = block_occludes_ambient_light_bit($blocks, $padded_index, $c10);
        let c11 = block_occludes_ambient_light_bit($blocks, $padded_index, $c11);

        vertex_ao_key(a0, b0, c00)
            | (vertex_ao_key(a0, b1, c01) << 2)
            | (vertex_ao_key(a1, b0, c10) << 4)
            | (vertex_ao_key(a1, b1, c11) << 6)
    }};
}

macro_rules! ao_key_ba {
    ($blocks:expr, $padded_index:expr, $a0:expr, $a1:expr, $b0:expr, $b1:expr, $c00:expr, $c01:expr, $c10:expr, $c11:expr) => {{
        let a0 = block_occludes_ambient_light_bit($blocks, $padded_index, $a0);
        let a1 = block_occludes_ambient_light_bit($blocks, $padded_index, $a1);
        let b0 = block_occludes_ambient_light_bit($blocks, $padded_index, $b0);
        let b1 = block_occludes_ambient_light_bit($blocks, $padded_index, $b1);
        let c00 = block_occludes_ambient_light_bit($blocks, $padded_index, $c00);
        let c01 = block_occludes_ambient_light_bit($blocks, $padded_index, $c01);
        let c10 = block_occludes_ambient_light_bit($blocks, $padded_index, $c10);
        let c11 = block_occludes_ambient_light_bit($blocks, $padded_index, $c11);

        vertex_ao_key(a0, b0, c00)
            | (vertex_ao_key(a1, b0, c10) << 2)
            | (vertex_ao_key(a0, b1, c01) << 4)
            | (vertex_ao_key(a1, b1, c11) << 6)
    }};
}

fn drop_uploaded_descriptors(
    mut commands: Commands,
    descriptors: Query<(Entity, Ref<ChunkMeshDescriptors>)>,
) {
    for (entity, desc_ref) in &descriptors {
        if !desc_ref.is_added() {
            commands.entity(entity).remove::<ChunkMeshDescriptors>();
        }
    }
}

pub struct ChunkMeshPlugin;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkMaterialLayerMarker(BlockMaterialLayer);

#[derive(Debug, Clone, Copy)]
pub(crate) struct BlockMeshTables {
    pub(crate) texture_layers: [[BlockTextureLayer; DIRECTION_COUNT]; RENDER_ID_COUNT],
    pub(crate) texture_frame_counts: [[u32; DIRECTION_COUNT]; RENDER_ID_COUNT],
    pub(crate) colors: [[Vec4; DIRECTION_COUNT]; RENDER_ID_COUNT],
}

impl BlockMeshTables {
    pub(crate) fn from_texture_map(block_texture_map: &BlockTextureMap) -> Self {
        let mut texture_layers = [[BlockTextureLayer::default(); DIRECTION_COUNT]; RENDER_ID_COUNT];
        let mut texture_frame_counts = [[1u32; DIRECTION_COUNT]; RENDER_ID_COUNT];
        let mut colors = [[Vec4::ZERO; DIRECTION_COUNT]; RENDER_ID_COUNT];

        for block in BlockType::iter() {
            let rid = render_id_for_block(block) as usize;
            for (side_index, side) in Direction::iter().enumerate() {
                let animation = block_texture_map.block_to_texture_animation(block, side);
                texture_layers[rid][side_index] = animation.base_layer();
                texture_frame_counts[rid][side_index] = animation.frame_count();
                colors[rid][side_index] = render_id_to_colour(rid as u16, side);
            }
        }

        let water = WATER_RENDER_ID as usize;
        for (side_index, side) in Direction::iter().enumerate() {
            let animation = block_texture_map.render_id_to_texture_animation(WATER_RENDER_ID, side);
            texture_layers[water][side_index] = animation.base_layer();
            texture_frame_counts[water][side_index] = animation.frame_count();
            colors[water][side_index] = render_id_to_colour(WATER_RENDER_ID, side);
        }

        Self {
            texture_layers,
            texture_frame_counts,
            colors,
        }
    }
}

impl Plugin for ChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedPreUpdate,
            (rebuild_chunk_meshes, upload_chunk_lights)
                .chain()
                .run_if(in_state(TextureState::Finished)),
        )
        .add_systems(PostUpdate, drop_uploaded_descriptors);
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn rebuild_chunk_meshes(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    block_textures: Res<BlockTextures>,
    block_texture_map: Res<BlockTextureMap>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>,
    all_chunks_q: Query<(&ChunkPosition, &Chunk)>,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    children_q: Query<&Children>,
    vp_children_q: Query<(Entity, &ChunkMaterialLayerMarker), With<VertexPullingMesh>>,
    mut vp_mesh_q: Query<&mut VertexPullingMesh>,
    vp_light_q: Query<&VertexPullingLight>,
    chunk_transform_q: Query<&Transform>,
) {
    if dirty_chunks_q.is_empty() {
        return;
    }

    let mut chunks_by_pos = HashMap::with_capacity(all_chunks_q.iter().len());
    for (positions, chunks) in all_chunks_q
        .contiguous_iter()
        .expect("chunk mesh position map query should stay dense")
    {
        chunks_by_pos.extend(
            positions
                .iter()
                .zip(chunks.iter())
                .map(|(pos, chunk)| (pos.0, chunk)),
        );
    }

    let tables = BlockMeshTables::from_texture_map(&block_texture_map);
    let texture_layers: Vec<u32> = (0..RENDER_ID_COUNT)
        .flat_map(|bt| {
            (0..DIRECTION_COUNT).map(move |dir| {
                pack_texture_layer(
                    tables.texture_layers[bt][dir],
                    tables.texture_frame_counts[bt][dir],
                )
            })
        })
        .collect();
    let tint_colors: Vec<[f32; 4]> = (0..RENDER_ID_COUNT)
        .flat_map(|bt| {
            (0..DIRECTION_COUNT).map(move |dir| {
                let c = tables.colors[bt][dir];
                [c.x, c.y, c.z, c.w]
            })
        })
        .collect();
    let emission_factors: Vec<f32> = (0..RENDER_ID_COUNT)
        .flat_map(|rid| {
            let emission = if rid == 0 || rid == WATER_RENDER_ID as usize {
                0.0
            } else {
                f32::from(from_render_id(rid as u16).unwrap().light_emission()) / 15.0
            };
            (0..DIRECTION_COUNT).map(move |_| emission)
        })
        .collect();
    let texture_state = VpTextureState {
        terrain_texture_handle: block_textures.terrain.clone(),
        texture_layers,
        tint_colors,
        emission_factors,
        ao_brightness: AO_BRIGHTNESS,
    };
    commands.insert_resource(texture_state);

    let mut lights_by_pos: HashMap<IVec3, &ChunkLight> =
        HashMap::with_capacity(light_q.iter().len());
    for (positions, lights) in light_q
        .contiguous_iter()
        .expect("chunk mesh light map query should stay dense")
    {
        lights_by_pos.extend(
            positions
                .iter()
                .zip(lights.iter())
                .map(|(pos, light)| (pos.0, light)),
        );
    }

    let mut build_queue = Parallel::<Vec<VpChunkBuild>>::default();
    dirty_chunks_q.par_iter().for_each_init(
        || build_queue.borrow_local_mut(),
        |builds, (chunk_pos, chunk_entity)| {
            let blocks = ChunkMeshBlocks::from_chunks(chunk_pos.0, &chunks_by_pos);
            let layers = binary::build_descriptors_hybrid(&blocks);
            builds.push(VpChunkBuild {
                entity: chunk_entity,
                chunk_pos: chunk_pos.0,
                layers,
            });
        },
    );
    let mut builds = Vec::new();
    build_queue.drain_into(&mut builds);
    let rebuilt_count = builds.len();
    for build in builds {
        let origin = chunk_transform_q
            .get(build.entity)
            .map(|t| t.translation)
            .unwrap_or(Vec3::ZERO);
        update_chunk_vp_children(
            &mut commands,
            &vp_children_q,
            &mut vp_mesh_q,
            build.entity,
            build.chunk_pos,
            children_q.get(build.entity).ok(),
            build.layers,
            origin,
            &lights_by_pos,
            &vp_light_q,
        );
        commands
            .entity(build.entity)
            .remove::<ChunkNeedsMeshRebuild>();
    }
    if let Some(perf) = perf.as_deref_mut() {
        perf.mesh_rebuilds += rebuilt_count;
    }
}

fn pack_texture_layer(layer: BlockTextureLayer, frame_count: u32) -> u32 {
    layer.index() | (frame_count.min(255) << 24)
}

struct VpChunkBuild {
    entity: Entity,
    chunk_pos: IVec3,
    layers: Vec<(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)>,
}

#[allow(clippy::too_many_arguments)]
fn update_chunk_vp_children(
    commands: &mut Commands,
    vp_children_q: &Query<(Entity, &ChunkMaterialLayerMarker), With<VertexPullingMesh>>,
    vp_mesh_q: &mut Query<&mut VertexPullingMesh>,
    chunk_entity: Entity,
    chunk_pos: IVec3,
    children: Option<&Children>,
    layers: Vec<(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)>,
    chunk_origin: Vec3,
    lights_by_pos: &HashMap<IVec3, &ChunkLight>,
    vp_light_q: &Query<&VertexPullingLight>,
) {
    let existing = children
        .map(|children| {
            vp_children_q
                .iter_many(children)
                .map(|(entity, marker)| (marker.0, entity))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut updated = Vec::new();
    let mut shared_light_data = existing_child_light_data(&existing, vp_light_q);
    for (layer, descriptors) in layers {
        let face_count = descriptors.len() as u32;
        updated.push(layer);

        if let Some(entity) = existing.get(&layer) {
            if let Ok(mut mesh) = vp_mesh_q.get_mut(*entity) {
                mesh.face_count = face_count;
                mesh.material_layer = layer;
                mesh.chunk_origin = chunk_origin;
            }
            commands
                .entity(*entity)
                .insert((ChunkMeshDescriptors(descriptors), chunk_render_aabb()));
            continue;
        }

        let light_data =
            light_data_for_new_vp_child(&mut shared_light_data, chunk_pos, lights_by_pos);

        commands.spawn((
            ChildOf(chunk_entity),
            ChunkMaterialLayerMarker(layer),
            Transform::default(),
            Visibility::default(),
            chunk_render_aabb(),
            VertexPullingMesh {
                face_count,
                material_layer: layer,
                chunk_origin,
            },
            ChunkMeshDescriptors(descriptors),
            VertexPullingLight {
                light_data: light_data.clone(),
            },
        ));
    }

    for (layer, entity) in existing {
        if updated.contains(&layer) {
            continue;
        }
        commands.entity(entity).despawn();
    }
}

fn existing_child_light_data(
    existing: &HashMap<BlockMaterialLayer, Entity>,
    vp_light_q: &Query<&VertexPullingLight>,
) -> Option<Arc<[u32]>> {
    existing.values().find_map(|entity| {
        vp_light_q
            .get(*entity)
            .ok()
            .map(|light| light.light_data.clone())
    })
}

fn light_data_for_new_vp_child(
    shared_light_data: &mut Option<Arc<[u32]>>,
    chunk_pos: IVec3,
    lights_by_pos: &HashMap<IVec3, &ChunkLight>,
) -> Arc<[u32]> {
    shared_light_data
        .get_or_insert_with(|| {
            Arc::from(ChunkLight::build_padded_light_data(
                chunk_pos,
                lights_by_pos,
            ))
        })
        .clone()
}

fn upload_chunk_lights(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), With<ChunkNeedsLightUpload>>,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    children_q: Query<&Children>,
    mut vp_light_q: Query<&mut VertexPullingLight>,
) {
    if dirty_chunks_q.is_empty() {
        return;
    }
    let upload_count = dirty_chunks_q.iter().len();

    let mut lights_by_pos: HashMap<IVec3, &ChunkLight> =
        HashMap::with_capacity(light_q.iter().len());
    for (positions, lights) in light_q
        .contiguous_iter()
        .expect("chunk light upload map query should stay dense")
    {
        lights_by_pos.extend(
            positions
                .iter()
                .zip(lights.iter())
                .map(|(pos, light)| (pos.0, light)),
        );
    }

    for (chunk_pos, chunk_entity) in &dirty_chunks_q {
        let light_data: Arc<[u32]> =
            ChunkLight::build_padded_light_data(chunk_pos.0, &lights_by_pos).into();
        if let Ok(children) = children_q.get(chunk_entity) {
            for child in children {
                if let Ok(mut light) = vp_light_q.get_mut(*child) {
                    light.light_data = light_data.clone();
                }
            }
        }

        commands
            .entity(chunk_entity)
            .remove::<ChunkNeedsLightUpload>();
    }
    if let Some(perf) = perf.as_deref_mut() {
        perf.light_uploads += upload_count;
    }
}

const fn chunk_render_aabb() -> Aabb {
    let half = CHUNK_SIZE as f32 / 2.0;
    let v = vec3a(half, half, half);
    Aabb {
        center: v,
        half_extents: v,
    }
}

#[inline(always)]
pub(crate) fn should_emit_face_from_flags(
    cell: u16,
    block_flags: u8,
    neighbor: u16,
    neighbor_flags: u8,
) -> bool {
    if neighbor_flags & BLOCK_FLAG_RENDERED == 0 {
        return true;
    }

    if neighbor_flags & BLOCK_FLAG_FULL_CUBE != 0 && block_flags & BLOCK_FLAG_TRANSLUCENT == 0 {
        return false;
    }

    if cell == neighbor && block_flags & BLOCK_FLAG_FULL_CUBE == 0 {
        return block_flags & BLOCK_FLAG_EMITS_INTERNAL_FACES != 0;
    }

    true
}

/// Translucent-specific face culling (water, ice).
///
/// Same rules as the scalar pass except translucent blocks are culled by
/// full-cube face occluders (stone, dirt, grass, etc.). Same-fluid-type
/// faces are also culled.
#[inline(always)]
pub(crate) fn should_emit_translucent_face(
    cell: u16,
    _block_flags: u8,
    neighbor: u16,
    neighbor_flags: u8,
) -> bool {
    if neighbor_flags & BLOCK_FLAG_RENDERED == 0 {
        return true;
    }

    if neighbor_flags & BLOCK_FLAG_FULL_CUBE != 0 {
        return false;
    }

    if cell == neighbor {
        return false;
    }

    true
}

/// Compute the 4 corner water heights for a water cell's top surface.
///
/// Each corner height is the average of the water levels in the four cells
/// that meet at that corner (Minecraft-style corner interpolation). This
/// ensures adjacent water blocks compute the same height for shared vertices,
/// preventing surface gaps.
///
/// Returns (h00, h10, h01, h11) where:
///   h00 = corner at (x+0, z+0)  h10 = corner at (x+1, z+0)
///   h01 = corner at (x+0, z+1)  h11 = corner at (x+1, z+1)
pub(crate) fn water_corner_heights(
    self_level: u8,
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
) -> (u32, u32, u32, u32) {
    let base = padded_index as isize;
    let pcs = PADDED_CHUNK_SIZE as isize;
    let n1 = base - 1;
    let p1 = base + 1;
    let nz = base - pcs;
    let pz = base + pcs;
    let h00 = water_corner_4(self_level, blocks, n1, nz, n1 - pcs);
    let h10 = water_corner_4(self_level, blocks, p1, nz, p1 - pcs);
    let h01 = water_corner_4(self_level, blocks, n1, pz, n1 + pcs);
    let h11 = water_corner_4(self_level, blocks, p1, pz, p1 + pcs);
    (h00 as u32, h10 as u32, h01 as u32, h11 as u32)
}

/// Average of self_level + up to 3 other cells at the given offsets.
/// Only cells containing water contribute; non-water neighbours are skipped.
fn water_corner_4(self_level: u8, blocks: &ChunkMeshBlocks, o1: isize, o2: isize, o3: isize) -> u8 {
    let mut sum = self_level as u32;
    let mut count = 1u32;
    for &offset in &[o1, o2, o3] {
        let cell = unsafe { *blocks.blocks.get_unchecked(offset as usize) };
        if cell == WATER_RENDER_ID {
            sum += blocks.get_fluid_level(offset as usize) as u32;
            count += 1;
        }
    }
    ((sum + count / 2) / count) as u8
}

/// Return the (lo, hi) lower-water corner height pair for a given side-index
/// bottom vertex.  lo = corner at qi=0 in quad space, hi = corner at qi=1.
/// For the y-facing indices (down=2, up=3) both values are zero.
#[inline(always)]
pub(crate) fn water_below_pair(
    side_index: usize,
    bh00: u32,
    bh10: u32,
    bh01: u32,
    bh11: u32,
) -> (u32, u32) {
    match side_index {
        0 => (bh00, bh01), // -X / Left
        1 => (bh10, bh11), // +X / Right
        4 => (bh00, bh10), // -Z / Back
        5 => (bh01, bh11), // +Z / Forward
        _ => (0, 0),       // DOWN, UP
    }
}

#[inline(always)]
pub(crate) fn face_ao_key_from_indices(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    side_index: usize,
) -> u32 {
    match side_index {
        0 => ao_key_ab!(
            blocks,
            padded_index,
            -325,
            323,
            17,
            -19,
            -307,
            -343,
            341,
            305
        ),
        1 => ao_key_ab!(
            blocks,
            padded_index,
            -323,
            325,
            -17,
            19,
            -341,
            -305,
            307,
            343
        ),
        2 => ao_key_ba!(
            blocks,
            padded_index,
            -325,
            -323,
            -306,
            -342,
            -307,
            -343,
            -305,
            -341
        ),
        3 => ao_key_ab!(blocks, padded_index, 323, 325, 342, 306, 341, 305, 343, 307),
        4 => ao_key_ba!(
            blocks,
            padded_index,
            -19,
            -17,
            -342,
            306,
            -343,
            305,
            -341,
            307
        ),
        _ => ao_key_ba!(
            blocks,
            padded_index,
            19,
            17,
            -306,
            342,
            -305,
            343,
            -307,
            341
        ),
    }
}

#[inline(always)]
#[cfg(test)]
pub(crate) fn face_ao_from_indices(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    side_index: usize,
) -> [u8; 4] {
    let key = face_ao_key_from_indices(blocks, padded_index, side_index);
    [
        (key & 0x3) as u8,
        ((key >> 2) & 0x3) as u8,
        ((key >> 4) & 0x3) as u8,
        ((key >> 6) & 0x3) as u8,
    ]
}

#[inline(always)]
pub(crate) fn vertex_ao_key(s1: u32, s2: u32, corner: u32) -> u32 {
    VERTEX_AO[(s1 | (s2 << 1) | (corner << 2)) as usize]
}

#[inline(always)]
fn block_occludes_ambient_light_bit(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    offset: isize,
) -> u32 {
    let index = (padded_index as isize + offset) as usize;
    debug_assert!(index < PADDED_CHUNK_VOLUME);
    unsafe {
        ((block_mesh_flags(*blocks.blocks.get_unchecked(index)) & BLOCK_FLAG_FULL_CUBE) >> 1) as u32
    }
}

pub(crate) fn padded_chunk_index(x: usize, y: usize, z: usize) -> usize {
    x + PADDED_CHUNK_SIZE * (z + PADDED_CHUNK_SIZE * y)
}

#[cfg(test)]
mod tests;
