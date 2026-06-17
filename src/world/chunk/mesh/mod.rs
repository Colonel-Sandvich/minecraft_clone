mod blocks;
pub mod vertex_pulling;

pub use blocks::ChunkMeshBlocks;
pub use vertex_pulling::{VertexPullingMesh, VpAtlasState};

use bevy::{camera::primitives::Aabb, platform::collections::HashMap, prelude::*, utils::Parallel};
use strum::{EnumCount, IntoEnumIterator};

use crate::block::{BlockMaterialLayer, BlockTextureMap, BlockType, block_to_colour};
use crate::quad::Direction;
use crate::textures::{BlockStandardMaterials, TextureState};

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkLight, ChunkNeedsMeshRebuild, ChunkPosition,
    ambient_occlusion::AmbientOcclusionSettings,
};

pub(crate) const PADDED_CHUNK_SIZE: usize = CHUNK_SIZE + 2;
pub(crate) const PADDED_CHUNK_VOLUME: usize =
    PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const PADDED_CHUNK_LAYER_SIZE: usize = PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const BLOCK_TYPE_COUNT: usize = BlockType::COUNT;
pub(crate) const DIRECTION_COUNT: usize = 6;
pub(crate) const DIRECTION_INDEX_OFFSETS: [isize; DIRECTION_COUNT] = [
    -1,
    1,
    -(PADDED_CHUNK_LAYER_SIZE as isize),
    PADDED_CHUNK_LAYER_SIZE as isize,
    -(PADDED_CHUNK_SIZE as isize),
    PADDED_CHUNK_SIZE as isize,
];
pub(crate) const VERTEX_AO: [u8; 8] = [3, 2, 2, 0, 2, 1, 1, 0];
pub(crate) const AO_SAMPLE_INDEX_OFFSETS: [[[isize; 3]; 4]; DIRECTION_COUNT] = [
    [
        [-325, 17, -307],
        [-325, -19, -343],
        [323, 17, 341],
        [323, -19, 305],
    ],
    [
        [-323, -17, -341],
        [-323, 19, -305],
        [325, -17, 307],
        [325, 19, 343],
    ],
    [
        [-325, -306, -307],
        [-323, -306, -305],
        [-325, -342, -343],
        [-323, -342, -341],
    ],
    [
        [323, 342, 341],
        [323, 306, 305],
        [325, 342, 343],
        [325, 306, 307],
    ],
    [
        [-19, -342, -343],
        [-17, -342, -341],
        [-19, 306, 305],
        [-17, 306, 307],
    ],
    [
        [19, -306, -305],
        [17, -306, -307],
        [19, 342, 343],
        [17, 342, 341],
    ],
];

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
            rebuild_chunk_meshes.run_if(in_state(TextureState::Finished)),
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
            let light_data = ChunkLight::build_padded_light_data(chunk_pos.0, &lights_by_pos);
            builds.push(VpChunkBuild {
                entity: chunk_entity,
                layers,
                light_data,
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
            children_q.get(build.entity).ok(),
            build.layers,
            origin,
            build.light_data,
        );
        commands
            .entity(build.entity)
            .remove::<ChunkNeedsMeshRebuild>();
    }
}

struct VpChunkBuild {
    entity: Entity,
    layers: Vec<(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)>,
    light_data: Box<[u32]>,
}

fn update_chunk_vp_children(
    commands: &mut Commands,
    vp_children_q: &Query<(Entity, &ChunkMaterialLayerMarker), With<VertexPullingMesh>>,
    vp_mesh_q: &mut Query<&mut VertexPullingMesh>,
    chunk_entity: Entity,
    children: Option<&Children>,
    layers: Vec<(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)>,
    chunk_origin: Vec3,
    light_data: Box<[u32]>,
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
    for (layer, descriptors) in layers {
        let face_count = descriptors.len() as u32;
        updated.push(layer);

        if let Some(entity) = existing.get(&layer) {
            if let Ok(mut mesh) = vp_mesh_q.get_mut(*entity) {
                mesh.descriptors = descriptors;
                mesh.face_count = face_count;
                mesh.material_layer = layer;
                mesh.chunk_origin = chunk_origin;
                mesh.light_data = light_data.clone();
            }
            commands.entity(*entity).insert(chunk_render_aabb());
            continue;
        }

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

const fn chunk_render_aabb() -> Aabb {
    let half = CHUNK_SIZE as f32 / 2.0;
    let v = vec3a(half, half, half);
    Aabb {
        center: v,
        half_extents: v,
    }
}

#[inline(always)]
pub(crate) fn should_emit_face_from_indices(block: BlockType, neighbor: BlockType) -> bool {
    if !neighbor.is_rendered() {
        return true;
    }

    if neighbor.is_full_cube() {
        return false;
    }

    if block == neighbor && !block.is_full_cube() && !neighbor.is_full_cube() {
        return block.emits_internal_faces();
    }

    true
}

#[inline(always)]
pub(crate) fn face_ao_from_indices(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    side_index: usize,
) -> [u8; 4] {
    let all = AO_SAMPLE_INDEX_OFFSETS[side_index];

    let offsets0 = all[0];
    let s10 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets0[0]);
    let s20 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets0[1]);
    let c0 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets0[2]);
    let ao0 = VERTEX_AO[s10 as usize | ((s20 as usize) << 1) | ((c0 as usize) << 2)];

    let offsets1 = all[1];
    let s11 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets1[0]);
    let s21 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets1[1]);
    let c1 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets1[2]);
    let ao1 = VERTEX_AO[s11 as usize | ((s21 as usize) << 1) | ((c1 as usize) << 2)];

    let offsets2 = all[2];
    let s12 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets2[0]);
    let s22 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets2[1]);
    let c2 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets2[2]);
    let ao2 = VERTEX_AO[s12 as usize | ((s22 as usize) << 1) | ((c2 as usize) << 2)];

    let offsets3 = all[3];
    let s13 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets3[0]);
    let s23 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets3[1]);
    let c3 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets3[2]);
    let ao3 = VERTEX_AO[s13 as usize | ((s23 as usize) << 1) | ((c3 as usize) << 2)];

    [ao0, ao1, ao2, ao3]
}

#[inline(always)]
pub(crate) fn block_occludes_ambient_light_from_index(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    offset: isize,
) -> bool {
    blocks.blocks[(padded_index as isize + offset) as usize].is_full_cube()
}

pub(crate) fn padded_chunk_index(x: usize, y: usize, z: usize) -> usize {
    x + PADDED_CHUNK_SIZE * (z + PADDED_CHUNK_SIZE * y)
}

#[cfg(test)]
mod tests;
