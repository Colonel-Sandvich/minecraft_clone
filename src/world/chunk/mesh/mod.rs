mod blocks;
pub mod vertex_pulling;

pub use blocks::ChunkMeshBlocks;
pub use vertex_pulling::{VertexPullingLight, VertexPullingMesh, VpAtlasState};

use std::sync::Arc;

use bevy::{camera::primitives::Aabb, platform::collections::HashMap, prelude::*, utils::Parallel};
use strum::{EnumCount, IntoEnumIterator};

use crate::block::{BlockMaterialLayer, BlockTextureMap, BlockType, block_to_colour};
use crate::quad::Direction;
use crate::textures::{BlockStandardMaterials, TextureState};

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkLight, ChunkNeedsLightUpload,
    ChunkNeedsMeshRebuild, ChunkPosition, ambient_occlusion::AmbientOcclusionSettings,
};

pub(crate) const PADDED_CHUNK_SIZE: usize = CHUNK_SIZE + 2;
pub(crate) const PADDED_CHUNK_VOLUME: usize =
    PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const PADDED_CHUNK_LAYER_SIZE: usize = PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const BLOCK_TYPE_COUNT: usize = BlockType::COUNT;
pub(crate) const DIRECTION_COUNT: usize = 6;
pub(crate) const BLOCK_FLAG_RENDERED: u8 = 1 << 0;
pub(crate) const BLOCK_FLAG_FULL_CUBE: u8 = 1 << 1;
pub(crate) const BLOCK_FLAG_EMITS_INTERNAL_FACES: u8 = 1 << 2;
pub(crate) const BLOCK_FLAG_CUTOUT: u8 = 1 << 3;
pub(crate) const DIRECTION_INDEX_OFFSETS: [isize; DIRECTION_COUNT] = [
    -1,
    1,
    -(PADDED_CHUNK_LAYER_SIZE as isize),
    PADDED_CHUNK_LAYER_SIZE as isize,
    -(PADDED_CHUNK_SIZE as isize),
    PADDED_CHUNK_SIZE as isize,
];
pub(crate) const BLOCK_MESH_FLAGS: [u8; BLOCK_TYPE_COUNT] = [
    0,
    BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE,
    BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE,
    BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE,
    BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE,
    BLOCK_FLAG_RENDERED | BLOCK_FLAG_CUTOUT,
    BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE,
    BLOCK_FLAG_RENDERED | BLOCK_FLAG_CUTOUT | BLOCK_FLAG_EMITS_INTERNAL_FACES,
    BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE,
];
pub(crate) const VERTEX_AO: [u32; 8] = [3, 2, 2, 0, 2, 1, 1, 0];

#[inline(always)]
pub(crate) fn block_mesh_flags(block: BlockType) -> u8 {
    unsafe { *BLOCK_MESH_FLAGS.get_unchecked(block as usize) }
}

#[inline(always)]
pub(crate) const fn material_layer_index_from_flags(flags: u8) -> usize {
    ((flags & BLOCK_FLAG_CUTOUT) != 0) as usize
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

pub struct ChunkMeshPlugin;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkMaterialLayerMarker(BlockMaterialLayer);

#[derive(Debug, Clone, Copy)]
pub(crate) struct BlockMeshTables {
    pub(crate) uvs: [[Rect; DIRECTION_COUNT]; BLOCK_TYPE_COUNT],
    pub(crate) colors: [[Vec4; DIRECTION_COUNT]; BLOCK_TYPE_COUNT],
}

impl BlockMeshTables {
    pub(crate) fn from_texture_map(block_texture_map: &BlockTextureMap) -> Self {
        let mut uvs = [[Rect::new(0.0, 0.0, 0.0, 0.0); DIRECTION_COUNT]; BLOCK_TYPE_COUNT];
        let mut colors = [[Vec4::ZERO; DIRECTION_COUNT]; BLOCK_TYPE_COUNT];

        for block in BlockType::iter() {
            let block_index = block as usize;
            if !block.is_rendered() {
                continue;
            };

            for (side_index, side) in Direction::iter().enumerate() {
                uvs[block_index][side_index] = block_texture_map.block_to_mesh(block, side);
                colors[block_index][side_index] = block_to_colour(block, side);
            }
        }

        Self { uvs, colors }
    }
}

impl Plugin for ChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedPreUpdate,
            (rebuild_chunk_meshes, upload_chunk_lights)
                .chain()
                .run_if(in_state(TextureState::Finished)),
        );
    }
}

fn rebuild_chunk_meshes(
    mut commands: Commands,
    block_materials: Res<BlockStandardMaterials>,
    block_texture_map: Res<BlockTextureMap>,
    ao_settings: Res<AmbientOcclusionSettings>,
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

    let ao_brightness = ao_settings.brightness_curve();
    let chunks_by_pos = all_chunks_q
        .iter()
        .map(|(pos, chunk)| (pos.0, chunk))
        .collect::<HashMap<_, _>>();

    let tables = BlockMeshTables::from_texture_map(&block_texture_map);
    let tile_offsets: Vec<[f32; 2]> = (0..BLOCK_TYPE_COUNT)
        .flat_map(|bt| {
            (0..DIRECTION_COUNT).map(move |dir| {
                let r = tables.uvs[bt][dir];
                [r.min.x, r.min.y]
            })
        })
        .collect();
    let tint_colors: Vec<[f32; 4]> = (0..BLOCK_TYPE_COUNT)
        .flat_map(|bt| {
            (0..DIRECTION_COUNT).map(move |dir| {
                let c = tables.colors[bt][dir];
                [c.x, c.y, c.z, c.w]
            })
        })
        .collect();
    let tile_size = {
        let stone = tables.uvs[BlockType::Stone as usize][0];
        Vec2::new(stone.width(), stone.height())
    };

    let atlas_state = VpAtlasState {
        atlas_handle: block_materials.atlas.clone(),
        tile_size,
        tile_offsets,
        tint_colors,
        ao_brightness,
    };
    commands.insert_resource(atlas_state);

    let lights_by_pos: HashMap<IVec3, &ChunkLight> =
        light_q.iter().map(|(pos, light)| (pos.0, light)).collect();

    let mut build_queue = Parallel::<Vec<VpChunkBuild>>::default();
    dirty_chunks_q.par_iter().for_each_init(
        || build_queue.borrow_local_mut(),
        |builds, (chunk_pos, chunk_entity)| {
            let blocks = ChunkMeshBlocks::from_chunks(chunk_pos.0, &chunks_by_pos);
            let layers = vertex_pulling::build_descriptors(&blocks);
            builds.push(VpChunkBuild {
                entity: chunk_entity,
                chunk_pos: chunk_pos.0,
                layers,
            });
        },
    );
    let mut builds = Vec::new();
    build_queue.drain_into(&mut builds);
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
}

struct VpChunkBuild {
    entity: Entity,
    chunk_pos: IVec3,
    layers: Vec<(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)>,
}

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
                mesh.descriptors = descriptors;
                mesh.face_count = face_count;
                mesh.material_layer = layer;
                mesh.chunk_origin = chunk_origin;
            }
            commands.entity(*entity).insert(chunk_render_aabb());
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
                descriptors,
                face_count,
                material_layer: layer,
                chunk_origin,
            },
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
    dirty_chunks_q: Query<(&ChunkPosition, Entity), With<ChunkNeedsLightUpload>>,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    children_q: Query<&Children>,
    mut vp_light_q: Query<&mut VertexPullingLight>,
) {
    if dirty_chunks_q.is_empty() {
        return;
    }

    let lights_by_pos: HashMap<IVec3, &ChunkLight> =
        light_q.iter().map(|(pos, light)| (pos.0, light)).collect();

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
    block: BlockType,
    block_flags: u8,
    neighbor: BlockType,
    neighbor_flags: u8,
) -> bool {
    if neighbor_flags & BLOCK_FLAG_RENDERED == 0 {
        return true;
    }

    if neighbor_flags & BLOCK_FLAG_FULL_CUBE != 0 {
        return false;
    }

    if block == neighbor && block_flags & BLOCK_FLAG_FULL_CUBE == 0 {
        return block_flags & BLOCK_FLAG_EMITS_INTERNAL_FACES != 0;
    }

    true
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
fn vertex_ao_key(s1: u32, s2: u32, corner: u32) -> u32 {
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
