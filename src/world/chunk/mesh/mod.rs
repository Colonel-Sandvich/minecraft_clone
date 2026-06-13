mod blocks;
mod buffer;
mod direct;
mod greedy;
mod hybrid;
mod reference;
mod shell;
mod sweep;

use self::reference::make_layered_quad_groups_from_blocks;
pub use blocks::ChunkMeshBlocks;
pub(crate) use buffer::MeshBufferBuilder;
pub use direct::DirectChunkMesher;
pub use greedy::GreedyChunkMesher;
pub use hybrid::HybridChunkMesher;
pub use reference::ReferenceChunkMesher;
pub use shell::FullCubeShellChunkMesher;
pub use sweep::SweepChunkMesher;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::utils::Parallel;
use strum::IntoEnumIterator;

use crate::block::{BlockMaterialLayer, BlockTextureMap, BlockType, block_to_colour};
use crate::quad::{Direction, QuadGroups, get_normals, get_positions, urect_to_uvs};
use crate::textures::{BlockStandardMaterials, TextureState};

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkNeedsMeshRebuild, ChunkPosition,
    ambient_occlusion::{AO_BRIGHTNESS, AmbientOcclusionSettings},
};

pub(crate) const SKY_FACE_BRIGHTNESS: f32 = 1.0;
pub(crate) const HORIZON_FACE_BRIGHTNESS: f32 = 0.86;
pub(crate) const GROUND_BOUNCE_FACE_BRIGHTNESS: f32 = 0.68;
pub(crate) const PADDED_CHUNK_SIZE: usize = CHUNK_SIZE + 2;
pub(crate) const PADDED_CHUNK_VOLUME: usize =
    PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const PADDED_CHUNK_LAYER_SIZE: usize = PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const BLOCK_TYPE_COUNT: usize = 8;
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
pub(crate) const VERTEX_OFFSETS: [[IVec3; 4]; DIRECTION_COUNT] = [
    [
        IVec3::new(0, 0, 1),
        IVec3::new(0, 0, 0),
        IVec3::new(0, 1, 1),
        IVec3::new(0, 1, 0),
    ],
    [
        IVec3::new(1, 0, 0),
        IVec3::new(1, 0, 1),
        IVec3::new(1, 1, 0),
        IVec3::new(1, 1, 1),
    ],
    [
        IVec3::new(0, 0, 1),
        IVec3::new(1, 0, 1),
        IVec3::new(0, 0, 0),
        IVec3::new(1, 0, 0),
    ],
    [
        IVec3::new(0, 1, 1),
        IVec3::new(0, 1, 0),
        IVec3::new(1, 1, 1),
        IVec3::new(1, 1, 0),
    ],
    [
        IVec3::new(0, 0, 0),
        IVec3::new(1, 0, 0),
        IVec3::new(0, 1, 0),
        IVec3::new(1, 1, 0),
    ],
    [
        IVec3::new(1, 0, 1),
        IVec3::new(0, 0, 1),
        IVec3::new(1, 1, 1),
        IVec3::new(0, 1, 1),
    ],
];
pub(crate) const NORMALS: [[[f32; 3]; 4]; DIRECTION_COUNT] = [
    [[-1.0, 0.0, 0.0]; 4],
    [[1.0, 0.0, 0.0]; 4],
    [[0.0, -1.0, 0.0]; 4],
    [[0.0, 1.0, 0.0]; 4],
    [[0.0, 0.0, -1.0]; 4],
    [[0.0, 0.0, 1.0]; 4],
];
pub(crate) const DIRECTIONS: [Direction; DIRECTION_COUNT] = [
    Direction::Left,
    Direction::Right,
    Direction::Down,
    Direction::Up,
    Direction::Forward,
    Direction::Backward,
];

pub struct ChunkMeshPlugin;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkMaterialLayerMarker(BlockMaterialLayer);

#[derive(Debug, Default)]
pub struct LayeredQuadGroups {
    pub layers: [QuadGroups; BlockMaterialLayer::COUNT],
}

pub type ChunkLayerMeshes = Vec<(BlockMaterialLayer, Mesh)>;

#[derive(Clone, Copy)]
pub struct ChunkMeshInput<'a> {
    pub blocks: &'a ChunkMeshBlocks,
    pub block_texture_map: &'a BlockTextureMap,
    pub ao_brightness: [f32; 4],
}

pub trait ChunkMesher: Sync {
    fn name(&self) -> &'static str;
    fn mesh(&self, input: ChunkMeshInput<'_>) -> ChunkLayerMeshes;
}

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

            for side in DIRECTIONS {
                let side_index = side as usize;
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
    mut meshes: ResMut<Assets<Mesh>>,
    block_materials: Res<BlockStandardMaterials>,
    block_texture_map: Res<BlockTextureMap>,
    ao_settings: Res<AmbientOcclusionSettings>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>,
    all_chunks_q: Query<(&ChunkPosition, &Chunk)>,
    children_q: Query<&Children>,
    mesh_children_q: Query<(Entity, &ChunkMaterialLayerMarker), With<Mesh3d>>,
    mut mesh_q: Query<&mut Mesh3d>,
) {
    if dirty_chunks_q.is_empty() {
        return;
    }

    let ao_brightness = ao_settings.brightness_curve();
    let chunks_by_pos = all_chunks_q
        .iter()
        .map(|(pos, chunk)| (pos.0, chunk))
        .collect::<HashMap<_, _>>();
    let mut build_queue = Parallel::<Vec<ChunkMeshBuild>>::default();

    dirty_chunks_q.par_iter().for_each_init(
        || build_queue.borrow_local_mut(),
        |builds, (chunk_pos, chunk_entity)| {
            let blocks = ChunkMeshBlocks::from_chunks(chunk_pos.0, &chunks_by_pos);
            let meshes = GreedyChunkMesher.mesh(ChunkMeshInput {
                blocks: &blocks,
                block_texture_map: &block_texture_map,
                ao_brightness,
            });
            builds.push(ChunkMeshBuild {
                entity: chunk_entity,
                meshes,
            });
        },
    );

    let mut builds = Vec::new();
    build_queue.drain_into(&mut builds);

    for build in builds {
        update_chunk_mesh_children(
            &mut commands,
            &mut meshes,
            &block_materials,
            &mesh_children_q,
            &mut mesh_q,
            build.entity,
            children_q.get(build.entity).ok(),
            build.meshes,
        );
        commands
            .entity(build.entity)
            .remove::<ChunkNeedsMeshRebuild>();
    }
}

struct ChunkMeshBuild {
    entity: Entity,
    meshes: ChunkLayerMeshes,
}

fn update_chunk_mesh_children(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    block_materials: &BlockStandardMaterials,
    mesh_children_q: &Query<(Entity, &ChunkMaterialLayerMarker), With<Mesh3d>>,
    mesh_q: &mut Query<&mut Mesh3d>,
    chunk_entity: Entity,
    children: Option<&Children>,
    chunk_meshes: Vec<(BlockMaterialLayer, Mesh)>,
) {
    let existing_mesh_children = children
        .map(|children| {
            mesh_children_q
                .iter_many(children)
                .map(|(entity, marker)| (marker.0, entity))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut updated_layers = Vec::new();
    for (layer, mesh) in chunk_meshes {
        let mesh_handle = meshes.add(mesh);
        updated_layers.push(layer);

        if let Some(entity) = existing_mesh_children.get(&layer) {
            *mesh_q.get_mut(*entity).unwrap() = Mesh3d(mesh_handle);
            commands.entity(*entity).remove::<Aabb>();
            continue;
        }

        commands.spawn((
            ChildOf(chunk_entity),
            ChunkMaterialLayerMarker(layer),
            Mesh3d(mesh_handle),
            MeshMaterial3d(block_materials.get(layer)),
        ));
    }

    for (layer, entity) in existing_mesh_children {
        if updated_layers.contains(&layer) {
            continue;
        }

        commands.entity(entity).despawn();
    }
}

pub fn make_chunk_meshes(chunk: &Chunk, block_texture_map: &BlockTextureMap) -> ChunkLayerMeshes {
    make_chunk_meshes_with_ao_brightness(chunk, block_texture_map, AO_BRIGHTNESS)
}

fn make_chunk_meshes_with_ao_brightness(
    chunk: &Chunk,
    block_texture_map: &BlockTextureMap,
    ao_brightness: [f32; 4],
) -> ChunkLayerMeshes {
    let blocks = ChunkMeshBlocks::from_chunk(chunk);

    ReferenceChunkMesher.mesh(ChunkMeshInput {
        blocks: &blocks,
        block_texture_map,
        ao_brightness,
    })
}

pub fn make_layered_quad_groups(
    chunk: &Chunk,
    block_texture_map: &BlockTextureMap,
) -> LayeredQuadGroups {
    make_layered_quad_groups_from_blocks(chunk, block_texture_map)
}

pub fn make_reference_layered_quad_groups(
    blocks: &ChunkMeshBlocks,
    block_texture_map: &BlockTextureMap,
) -> LayeredQuadGroups {
    make_layered_quad_groups_from_blocks(blocks, block_texture_map)
}

pub fn make_mesh_from_quad_groups(quad_groups: &QuadGroups) -> Option<Mesh> {
    make_mesh_from_quad_groups_with_ao_brightness(quad_groups, AO_BRIGHTNESS)
}

pub(crate) fn make_mesh_from_quad_groups_with_ao_brightness(
    quad_groups: &QuadGroups,
    ao_brightness: [f32; 4],
) -> Option<Mesh> {
    let len: usize = quad_groups.groups.iter().map(|g| g.len()).sum();

    if len == 0 {
        return None;
    }

    let num_indices = len * 6;
    let num_vertices = len * 4;

    let mut indices = Vec::with_capacity(num_indices);
    let mut positions = Vec::with_capacity(num_vertices);
    let mut normals = Vec::with_capacity(num_vertices);
    let mut uvs = Vec::with_capacity(num_vertices);
    let mut colours = Vec::with_capacity(num_vertices);

    for (quads, side) in quad_groups.groups.iter().zip(Direction::iter()) {
        for quad in quads.iter() {
            indices.extend_from_slice(&get_ao_indices(positions.len() as u32, quad.ao));
            positions.extend_from_slice(&get_positions(quad, &side, 1.0));
            normals.extend_from_slice(&get_normals(side.into()));
            uvs.extend_from_slice(&urect_to_uvs(&quad.uv));
            colours.extend(
                quad.ao
                    .map(|ao| shaded_colour(quad.color, side, ao, ao_brightness)),
            );
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );

    mesh.insert_indices(Indices::U32(indices));

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);

    Some(mesh)
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
    AO_SAMPLE_INDEX_OFFSETS[side_index].map(|offsets| {
        let side1 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets[0]);
        let side2 = block_occludes_ambient_light_from_index(blocks, padded_index, offsets[1]);
        let corner = block_occludes_ambient_light_from_index(blocks, padded_index, offsets[2]);
        VERTEX_AO[side1 as usize | ((side2 as usize) << 1) | ((corner as usize) << 2)]
    })
}

#[inline(always)]
pub(crate) fn block_occludes_ambient_light_from_index(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    offset: isize,
) -> bool {
    blocks.blocks[(padded_index as isize + offset) as usize].is_full_cube()
}

#[inline(always)]
pub(crate) fn uvs_for_rect(rect: Rect) -> [[f32; 2]; 4] {
    [
        [rect.min.x, rect.max.y],
        [rect.max.x, rect.max.y],
        [rect.min.x, rect.min.y],
        [rect.max.x, rect.min.y],
    ]
}

pub(crate) fn padded_chunk_index(x: usize, y: usize, z: usize) -> usize {
    x + PADDED_CHUNK_SIZE * (z + PADDED_CHUNK_SIZE * y)
}

fn get_ao_indices(start: u32, ao: [u8; 4]) -> [u32; 6] {
    if ao[1] + ao[2] > ao[0] + ao[3] {
        [start, start + 2, start + 1, start + 1, start + 2, start + 3]
    } else {
        [start, start + 3, start + 1, start, start + 2, start + 3]
    }
}

fn face_brightness(side: Direction) -> f32 {
    match side {
        Direction::Up => SKY_FACE_BRIGHTNESS,
        Direction::Down => GROUND_BOUNCE_FACE_BRIGHTNESS,
        Direction::Left | Direction::Right | Direction::Forward | Direction::Backward => {
            HORIZON_FACE_BRIGHTNESS
        }
    }
}

fn shaded_colour(color: Vec4, side: Direction, ao: u8, ao_brightness: [f32; 4]) -> Vec4 {
    let brightness = face_brightness(side) * ao_brightness[ao as usize];
    Vec4::new(
        color.x * brightness,
        color.y * brightness,
        color.z * brightness,
        color.w,
    )
}

#[cfg(test)]
mod tests;
