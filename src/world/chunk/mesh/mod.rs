mod adaptive;
mod direct;
mod greedy;
mod hybrid;
mod reference;
mod shell;
mod sweep;

use self::reference::make_layered_quad_groups_from_blocks;
pub use adaptive::AdaptiveChunkMesher;
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
    chunk_neighbor_offsets,
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

#[derive(Default)]
pub(crate) struct MeshBufferBuilder {
    indices: Vec<u32>,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    uv1s: Vec<[f32; 2]>,
    colours: Vec<[f32; 4]>,
}

impl MeshBufferBuilder {
    fn with_face_capacity(face_count: usize) -> Self {
        Self {
            indices: Vec::with_capacity(face_count * 6),
            positions: Vec::with_capacity(face_count * 4),
            normals: Vec::with_capacity(face_count * 4),
            uvs: Vec::with_capacity(face_count * 4),
            uv1s: Vec::with_capacity(face_count * 4),
            colours: Vec::with_capacity(face_count * 4),
        }
    }

    pub(crate) fn push_face(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        side_index: usize,
        uv: Rect,
        color: Vec4,
        ao: [u8; 4],
        ao_brightness: [f32; 4],
    ) {
        self.indices
            .extend_from_slice(&get_ao_indices(self.positions.len() as u32, ao));

        for offset in VERTEX_OFFSETS[side_index] {
            self.positions.push([
                x as f32 + offset.x as f32,
                y as f32 + offset.y as f32,
                z as f32 + offset.z as f32,
            ]);
        }

        self.normals.extend_from_slice(&NORMALS[side_index]);
        self.uvs
            .extend_from_slice(&uvs_for_rect(Rect::new(0.0, 0.0, 1.0, 1.0)));
        let tile_offset = [uv.min.x, uv.min.y];
        self.uv1s.extend_from_slice(&[tile_offset; 4]);

        let face_light = face_brightness(DIRECTIONS[side_index]);
        self.colours.extend(ao.map(|ao| {
            let brightness = face_light * ao_brightness[ao as usize];
            [
                color.x * brightness,
                color.y * brightness,
                color.z * brightness,
                color.w,
            ]
        }));
    }

    pub(crate) fn push_merged_face(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        w: usize,
        h: usize,
        side_index: usize,
        uv: Rect,
        color: Vec4,
        ao: [u8; 4],
        ao_brightness: [f32; 4],
    ) {
        let base = self.positions.len() as u32;
        self.indices.extend_from_slice(&get_ao_indices(base, ao));

        match side_index {
            0 => {
                self.positions.push([x as f32, y as f32, (z + w) as f32]);
                self.positions.push([x as f32, y as f32, z as f32]);
                self.positions
                    .push([x as f32, (y + h) as f32, (z + w) as f32]);
                self.positions.push([x as f32, (y + h) as f32, z as f32]);
            }
            1 => {
                self.positions.push([(x + 1) as f32, y as f32, z as f32]);
                self.positions
                    .push([(x + 1) as f32, y as f32, (z + w) as f32]);
                self.positions
                    .push([(x + 1) as f32, (y + h) as f32, z as f32]);
                self.positions
                    .push([(x + 1) as f32, (y + h) as f32, (z + w) as f32]);
            }
            2 => {
                self.positions.push([x as f32, y as f32, (z + h) as f32]);
                self.positions
                    .push([(x + w) as f32, y as f32, (z + h) as f32]);
                self.positions.push([x as f32, y as f32, z as f32]);
                self.positions.push([(x + w) as f32, y as f32, z as f32]);
            }
            3 => {
                self.positions
                    .push([x as f32, (y + 1) as f32, (z + h) as f32]);
                self.positions.push([x as f32, (y + 1) as f32, z as f32]);
                self.positions
                    .push([(x + w) as f32, (y + 1) as f32, (z + h) as f32]);
                self.positions
                    .push([(x + w) as f32, (y + 1) as f32, z as f32]);
            }
            4 => {
                self.positions.push([x as f32, y as f32, z as f32]);
                self.positions.push([(x + w) as f32, y as f32, z as f32]);
                self.positions.push([x as f32, (y + h) as f32, z as f32]);
                self.positions
                    .push([(x + w) as f32, (y + h) as f32, z as f32]);
            }
            5 => {
                self.positions
                    .push([(x + w) as f32, y as f32, (z + 1) as f32]);
                self.positions.push([x as f32, y as f32, (z + 1) as f32]);
                self.positions
                    .push([(x + w) as f32, (y + h) as f32, (z + 1) as f32]);
                self.positions
                    .push([x as f32, (y + h) as f32, (z + 1) as f32]);
            }
            _ => unreachable!(),
        }

        self.normals.extend_from_slice(&NORMALS[side_index]);
        self.uvs
            .extend_from_slice(&uvs_for_rect(Rect::new(0.0, 0.0, w as f32, h as f32)));
        let tile_offset = [uv.min.x, uv.min.y];
        self.uv1s.extend_from_slice(&[tile_offset; 4]);

        let face_light = face_brightness(DIRECTIONS[side_index]);
        self.colours.extend(ao.map(|ao| {
            let brightness = face_light * ao_brightness[ao as usize];
            [
                color.x * brightness,
                color.y * brightness,
                color.z * brightness,
                color.w,
            ]
        }));
    }

    fn into_mesh(self) -> Option<Mesh> {
        if self.positions.is_empty() {
            return None;
        }

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );

        mesh.insert_indices(Indices::U32(self.indices));
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.uv1s);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colours);

        Some(mesh)
    }
}

pub struct ChunkMeshBlocks {
    pub(crate) blocks: Box<[BlockType; PADDED_CHUNK_VOLUME]>,
    pub(crate) center_rendered_blocks: u16,
    pub(crate) center_full_cube_blocks: u16,
}

impl ChunkMeshBlocks {
    pub fn from_chunk(chunk: &Chunk) -> Self {
        let mut blocks = Self::empty();
        blocks.copy_center_chunk(chunk);
        blocks
    }

    pub fn from_chunks(center_pos: IVec3, chunks: &HashMap<IVec3, &Chunk>) -> Self {
        let mut blocks = Self::empty();

        for offset in std::iter::once(IVec3::ZERO).chain(chunk_neighbor_offsets()) {
            let Some(chunk) = chunks.get(&(center_pos + offset)).copied() else {
                continue;
            };

            if offset == IVec3::ZERO {
                blocks.copy_center_chunk(chunk);
            } else {
                blocks.copy_neighbor_chunk_region(offset, chunk);
            }
        }

        blocks
    }

    fn empty() -> Self {
        Self {
            blocks: Box::new([BlockType::Air; PADDED_CHUNK_VOLUME]),
            center_rendered_blocks: 0,
            center_full_cube_blocks: 0,
        }
    }

    fn copy_center_chunk(&mut self, chunk: &Chunk) {
        let mut rendered_blocks = 0;
        let mut full_cube_blocks = 0;

        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    let block = chunk.blocks[x][z][y];
                    self.set_block(x as i32, y as i32, z as i32, block);
                    rendered_blocks += block.is_rendered() as u16;
                    full_cube_blocks += block.is_full_cube() as u16;
                }
            }
        }

        self.center_rendered_blocks = rendered_blocks;
        self.center_full_cube_blocks = full_cube_blocks;
    }

    fn copy_neighbor_chunk_region(&mut self, offset: IVec3, chunk: &Chunk) {
        for x in source_range_for_neighbor_axis(offset.x) {
            for y in source_range_for_neighbor_axis(offset.y) {
                for z in source_range_for_neighbor_axis(offset.z) {
                    self.set_block(
                        x as i32 + offset.x * CHUNK_ISIZE,
                        y as i32 + offset.y * CHUNK_ISIZE,
                        z as i32 + offset.z * CHUNK_ISIZE,
                        chunk.blocks[x][z][y],
                    );
                }
            }
        }
    }

    fn set_block(&mut self, x: i32, y: i32, z: i32, block: BlockType) {
        debug_assert!(is_in_padded_chunk(x));
        debug_assert!(is_in_padded_chunk(y));
        debug_assert!(is_in_padded_chunk(z));

        let x = (x + 1) as usize;
        let y = (y + 1) as usize;
        let z = (z + 1) as usize;
        self.blocks[padded_chunk_index(x, y, z)] = block;
    }

    pub(crate) fn can_skip_mesh(&self) -> bool {
        self.center_rendered_blocks == 0
            || (self.center_is_all_full_cube() && self.neighbor_face_shells_are_full_cube())
    }

    pub(crate) fn center_is_all_full_cube(&self) -> bool {
        self.center_full_cube_blocks as usize == CHUNK_VOLUME
    }

    fn neighbor_face_shells_are_full_cube(&self) -> bool {
        for y in 1..=CHUNK_SIZE {
            for z in 1..=CHUNK_SIZE {
                if !self.blocks[padded_chunk_index(0, y, z)].is_full_cube()
                    || !self.blocks[padded_chunk_index(CHUNK_SIZE + 1, y, z)].is_full_cube()
                {
                    return false;
                }
            }
        }

        for x in 1..=CHUNK_SIZE {
            for z in 1..=CHUNK_SIZE {
                if !self.blocks[padded_chunk_index(x, 0, z)].is_full_cube()
                    || !self.blocks[padded_chunk_index(x, CHUNK_SIZE + 1, z)].is_full_cube()
                {
                    return false;
                }
            }
        }

        for x in 1..=CHUNK_SIZE {
            for y in 1..=CHUNK_SIZE {
                if !self.blocks[padded_chunk_index(x, y, 0)].is_full_cube()
                    || !self.blocks[padded_chunk_index(x, y, CHUNK_SIZE + 1)].is_full_cube()
                {
                    return false;
                }
            }
        }

        true
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
            let meshes = AdaptiveChunkMesher.mesh(ChunkMeshInput {
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

fn source_range_for_neighbor_axis(delta: i32) -> std::ops::Range<usize> {
    match delta {
        -1 => CHUNK_SIZE - 1..CHUNK_SIZE,
        0 => 0..CHUNK_SIZE,
        1 => 0..1,
        _ => unreachable!("invalid neighbor offset"),
    }
}

fn is_in_padded_chunk(value: i32) -> bool {
    (-1..=CHUNK_ISIZE).contains(&value)
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
