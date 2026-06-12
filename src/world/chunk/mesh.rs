use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::math::UVec3;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::utils::Parallel;
use strum::IntoEnumIterator;

use crate::textures::{BlockStandardMaterials, TextureState};
use crate::{
    block::{
        BlockMaterialLayer, BlockTextureMap, BlockType, FaceOcclusion, FaceSidedness,
        block_to_colour,
    },
    quad::{
        Direction, Quad, QuadGroups, get_normals, get_positions, get_vertex_offsets, urect_to_uvs,
    },
};

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, Chunk, ChunkNeedsMeshRebuild, ChunkPosition,
    ambient_occlusion::{AO_BRIGHTNESS, AmbientOcclusionSettings},
    chunk_neighbor_offsets,
};

const SKY_FACE_BRIGHTNESS: f32 = 1.0;
const HORIZON_FACE_BRIGHTNESS: f32 = 0.86;
const GROUND_BOUNCE_FACE_BRIGHTNESS: f32 = 0.68;
const PADDED_CHUNK_SIZE: usize = CHUNK_SIZE + 2;
const PADDED_CHUNK_VOLUME: usize = PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;

pub struct ChunkMeshPlugin;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkMaterialLayerMarker(BlockMaterialLayer);

#[derive(Debug, Default)]
pub struct LayeredQuadGroups {
    pub layers: [QuadGroups; BlockMaterialLayer::COUNT],
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
            let padded_blocks = PaddedChunkBlocks::from_chunks(chunk_pos.0, &chunks_by_pos);
            let meshes = make_chunk_meshes_from_blocks_with_ao_brightness(
                &padded_blocks,
                &block_texture_map,
                ao_brightness,
            );
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
    meshes: Vec<(BlockMaterialLayer, Mesh)>,
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
            // Recompute Aabb after swapping mesh data.
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

pub fn make_chunk_meshes(
    chunk: &Chunk,
    block_texture_map: &BlockTextureMap,
) -> Vec<(BlockMaterialLayer, Mesh)> {
    make_chunk_meshes_with_ao_brightness(chunk, block_texture_map, AO_BRIGHTNESS)
}

fn make_chunk_meshes_with_ao_brightness(
    chunk: &Chunk,
    block_texture_map: &BlockTextureMap,
    ao_brightness: [f32; 4],
) -> Vec<(BlockMaterialLayer, Mesh)> {
    make_chunk_meshes_from_blocks_with_ao_brightness(chunk, block_texture_map, ao_brightness)
}

fn make_chunk_meshes_from_blocks_with_ao_brightness(
    blocks: &(impl BlockSampler + ?Sized),
    block_texture_map: &BlockTextureMap,
    ao_brightness: [f32; 4],
) -> Vec<(BlockMaterialLayer, Mesh)> {
    let quad_groups = make_layered_quad_groups_from_blocks(blocks, block_texture_map);
    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            make_mesh_from_quad_groups_with_ao_brightness(
                &quad_groups.layers[layer.index()],
                ao_brightness,
            )
            .map(|mesh| (layer, mesh))
        })
        .collect()
}

pub fn make_layered_quad_groups(
    chunk: &Chunk,
    block_texture_map: &BlockTextureMap,
) -> LayeredQuadGroups {
    make_layered_quad_groups_from_blocks(chunk, block_texture_map)
}

fn make_layered_quad_groups_from_blocks(
    blocks: &(impl BlockSampler + ?Sized),
    block_texture_map: &BlockTextureMap,
) -> LayeredQuadGroups {
    let mut buffer = LayeredQuadGroups::default();

    for x in 0..CHUNK_ISIZE {
        for y in 0..CHUNK_ISIZE {
            for z in 0..CHUNK_ISIZE {
                let block = blocks.get_block(x, y, z);

                let Some(profile) = block.render_profile() else {
                    continue;
                };

                let neighbors = [
                    blocks.get_block(x - 1, y, z),
                    blocks.get_block(x + 1, y, z),
                    blocks.get_block(x, y - 1, z),
                    blocks.get_block(x, y + 1, z),
                    blocks.get_block(x, y, z - 1),
                    blocks.get_block(x, y, z + 1),
                ];

                for (neighbor, side) in neighbors.into_iter().zip(Direction::iter()) {
                    if !should_emit_face(block, neighbor, side) {
                        continue;
                    }

                    let voxel = UVec3::from_slice(&[x, y, z].map(|u| u.try_into().unwrap()));

                    buffer.layers[profile.material_layer().index()].groups[side as usize].push(
                        Quad {
                            voxel,
                            uv: block_texture_map.block_to_mesh(block, side),
                            color: block_to_colour(block, side),
                            ao: face_ao(blocks, IVec3::new(x, y, z), side),
                        },
                    );
                }
            }
        }
    }

    buffer
}

fn should_emit_face(block: BlockType, neighbor: BlockType, side: Direction) -> bool {
    let Some(block_profile) = block.render_profile() else {
        return false;
    };
    let Some(neighbor_profile) = neighbor.render_profile() else {
        return true;
    };

    if neighbor_profile.occlusion == FaceOcclusion::FullCube {
        return false;
    }

    if block == neighbor
        && block_profile.occlusion == FaceOcclusion::None
        && neighbor_profile.occlusion == FaceOcclusion::None
    {
        return block_profile.sidedness == FaceSidedness::Double && is_positive_side(side);
    }

    true
}

trait BlockSampler: Sync {
    fn get_block(&self, x: i32, y: i32, z: i32) -> BlockType;
}

impl BlockSampler for Chunk {
    fn get_block(&self, x: i32, y: i32, z: i32) -> BlockType {
        self.get_i(x, y, z).unwrap_or(BlockType::Air)
    }
}

struct PaddedChunkBlocks {
    blocks: Box<[BlockType; PADDED_CHUNK_VOLUME]>,
}

impl PaddedChunkBlocks {
    fn from_chunks(center_pos: IVec3, chunks: &HashMap<IVec3, &Chunk>) -> Self {
        let mut padded_blocks = Self {
            blocks: Box::new([BlockType::Air; PADDED_CHUNK_VOLUME]),
        };

        for offset in std::iter::once(IVec3::ZERO).chain(chunk_neighbor_offsets()) {
            let Some(chunk) = chunks.get(&(center_pos + offset)).copied() else {
                continue;
            };

            for x in source_range_for_neighbor_axis(offset.x) {
                for y in source_range_for_neighbor_axis(offset.y) {
                    for z in source_range_for_neighbor_axis(offset.z) {
                        padded_blocks.set_block(
                            x as i32 + offset.x * CHUNK_ISIZE,
                            y as i32 + offset.y * CHUNK_ISIZE,
                            z as i32 + offset.z * CHUNK_ISIZE,
                            chunk.blocks[x][z][y],
                        );
                    }
                }
            }
        }

        padded_blocks
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
}

impl BlockSampler for PaddedChunkBlocks {
    fn get_block(&self, x: i32, y: i32, z: i32) -> BlockType {
        if !is_in_padded_chunk(x) || !is_in_padded_chunk(y) || !is_in_padded_chunk(z) {
            return BlockType::Air;
        }

        let x = (x + 1) as usize;
        let y = (y + 1) as usize;
        let z = (z + 1) as usize;
        self.blocks[padded_chunk_index(x, y, z)]
    }
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

fn padded_chunk_index(x: usize, y: usize, z: usize) -> usize {
    x + PADDED_CHUNK_SIZE * (z + PADDED_CHUNK_SIZE * y)
}

fn is_positive_side(side: Direction) -> bool {
    matches!(side, Direction::Right | Direction::Up | Direction::Backward)
}

fn face_ao(blocks: &(impl BlockSampler + ?Sized), voxel: IVec3, side: Direction) -> [u8; 4] {
    let normal = direction_offset(side);
    let tangent_axes = tangent_axes(side);

    get_vertex_offsets(side).map(|vertex_offset| {
        let side1 = sample_axis_offset(vertex_offset, tangent_axes[0]);
        let side2 = sample_axis_offset(vertex_offset, tangent_axes[1]);

        vertex_ao(
            ao_occludes(blocks, voxel + normal + side1),
            ao_occludes(blocks, voxel + normal + side2),
            ao_occludes(blocks, voxel + normal + side1 + side2),
        )
    })
}

fn vertex_ao(side1: bool, side2: bool, corner: bool) -> u8 {
    if side1 && side2 {
        0
    } else {
        3 - side1 as u8 - side2 as u8 - corner as u8
    }
}

fn ao_occludes(blocks: &(impl BlockSampler + ?Sized), pos: IVec3) -> bool {
    block_occludes_ambient_light(blocks.get_block(pos.x, pos.y, pos.z))
}

fn block_occludes_ambient_light(block: BlockType) -> bool {
    block
        .render_profile()
        .is_some_and(|profile| profile.occlusion == FaceOcclusion::FullCube)
}

fn direction_offset(side: Direction) -> IVec3 {
    match side {
        Direction::Left => IVec3::NEG_X,
        Direction::Right => IVec3::X,
        Direction::Down => IVec3::NEG_Y,
        Direction::Up => IVec3::Y,
        Direction::Forward => IVec3::NEG_Z,
        Direction::Backward => IVec3::Z,
    }
}

fn tangent_axes(side: Direction) -> [usize; 2] {
    match side {
        Direction::Left | Direction::Right => [1, 2],
        Direction::Down | Direction::Up => [0, 2],
        Direction::Forward | Direction::Backward => [0, 1],
    }
}

fn sample_axis_offset(vertex_offset: IVec3, axis: usize) -> IVec3 {
    let sign = if axis_component(vertex_offset, axis) == 0 {
        -1
    } else {
        1
    };

    match axis {
        0 => IVec3::new(sign, 0, 0),
        1 => IVec3::new(0, sign, 0),
        2 => IVec3::new(0, 0, sign),
        _ => unreachable!("invalid axis"),
    }
}

fn axis_component(offset: IVec3, axis: usize) -> i32 {
    match axis {
        0 => offset.x,
        1 => offset.y,
        2 => offset.z,
        _ => unreachable!("invalid axis"),
    }
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

pub fn make_mesh_from_quad_groups(quad_groups: &QuadGroups) -> Option<Mesh> {
    make_mesh_from_quad_groups_with_ao_brightness(quad_groups, AO_BRIGHTNESS)
}

fn make_mesh_from_quad_groups_with_ao_brightness(
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{math::Rect, mesh::VertexAttributeValues, platform::collections::HashMap};
    use strum::IntoEnumIterator;

    use crate::block::block_and_side_to_texture_path;

    fn test_texture_map() -> BlockTextureMap {
        let mut paths = HashMap::default();

        for block in BlockType::iter() {
            if block == BlockType::Air {
                continue;
            }

            for side in Direction::iter() {
                paths.insert(
                    block_and_side_to_texture_path(block, side).to_owned(),
                    Rect::new(0.0, 0.0, 1.0, 1.0),
                );
            }
        }

        BlockTextureMap(paths)
    }

    fn quad_count(groups: &QuadGroups) -> usize {
        groups.groups.iter().map(Vec::len).sum()
    }

    fn padded_chunk_blocks<'a>(
        chunks: impl IntoIterator<Item = (IVec3, &'a Chunk)>,
    ) -> PaddedChunkBlocks {
        let chunks = chunks.into_iter().collect::<HashMap<_, _>>();
        PaddedChunkBlocks::from_chunks(IVec3::ZERO, &chunks)
    }

    #[test]
    fn vertex_ao_uses_four_symmetric_levels() {
        let cases = [
            ((false, false, false), 3),
            ((true, false, false), 2),
            ((false, true, false), 2),
            ((false, false, true), 2),
            ((true, false, true), 1),
            ((false, true, true), 1),
            ((true, true, false), 0),
            ((true, true, true), 0),
        ];

        for ((side1, side2, corner), expected) in cases {
            assert_eq!(vertex_ao(side1, side2, corner), expected);
        }
    }

    #[test]
    fn face_ao_samples_adjacent_plane_and_only_full_cube_occluders() {
        let mut chunk = Chunk::default();
        chunk.blocks[1][1][1] = BlockType::Stone;

        chunk.blocks[0][1][2] = BlockType::Stone;
        chunk.blocks[1][2][2] = BlockType::Stone;
        chunk.blocks[0][2][2] = BlockType::Stone;
        chunk.blocks[2][1][2] = BlockType::Glass;
        chunk.blocks[2][2][2] = BlockType::OakLeaves;

        assert_eq!(
            face_ao(&chunk, IVec3::new(1, 1, 1), Direction::Up),
            [0, 2, 2, 3]
        );
    }

    #[test]
    fn face_ao_samples_loaded_face_neighbor_chunk() {
        let mut centre = Chunk::default();
        centre.blocks[1][1][15] = BlockType::Stone;

        let mut above = Chunk::default();
        above.blocks[0][1][0] = BlockType::Stone;
        above.blocks[1][2][0] = BlockType::Stone;
        above.blocks[0][2][0] = BlockType::Stone;

        let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (IVec3::Y, &above)]);

        assert_eq!(
            face_ao(&padded_blocks, IVec3::new(1, 15, 1), Direction::Up),
            [0, 2, 2, 3]
        );
    }

    #[test]
    fn face_ao_samples_loaded_edge_neighbor_chunk() {
        let mut centre = Chunk::default();
        centre.blocks[0][1][15] = BlockType::Stone;

        let mut edge = Chunk::default();
        edge.blocks[15][1][0] = BlockType::Stone;
        edge.blocks[15][2][0] = BlockType::Stone;

        let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (ivec3(-1, 1, 0), &edge)]);

        assert_eq!(
            face_ao(&padded_blocks, IVec3::new(0, 15, 1), Direction::Up),
            [1, 2, 3, 3]
        );
    }

    #[test]
    fn face_ao_samples_loaded_corner_neighbor_chunk() {
        let mut centre = Chunk::default();
        centre.blocks[0][15][15] = BlockType::Stone;

        let mut corner = Chunk::default();
        corner.blocks[15][0][0] = BlockType::Stone;

        let padded_blocks =
            padded_chunk_blocks([(IVec3::ZERO, &centre), (ivec3(-1, 1, 1), &corner)]);

        assert_eq!(
            face_ao(&padded_blocks, IVec3::new(0, 15, 15), Direction::Up),
            [2, 3, 3, 3]
        );
    }

    #[test]
    fn boundary_faces_are_culled_against_loaded_neighbor_chunks() {
        let texture_map = test_texture_map();
        let mut centre = Chunk::default();
        centre.blocks[15][0][0] = BlockType::Stone;

        let mut right = Chunk::default();
        right.blocks[0][0][0] = BlockType::Stone;

        let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (IVec3::X, &right)]);
        let groups = make_layered_quad_groups_from_blocks(&padded_blocks, &texture_map);
        let opaque_groups = &groups.layers[BlockMaterialLayer::Opaque.index()];

        assert_eq!(quad_count(opaque_groups), 5);
        assert_eq!(opaque_groups.groups[Direction::Right as usize].len(), 0);
    }

    #[test]
    fn mesh_bakes_ao_into_colours_and_chooses_less_biased_diagonal() {
        let mut groups = QuadGroups::default();
        let color = Vec4::new(0.8, 0.6, 0.4, 0.5);
        groups.groups[Direction::Up as usize].push(Quad {
            voxel: UVec3::ZERO,
            color,
            uv: Rect::new(0.0, 0.0, 1.0, 1.0),
            ao: [3, 0, 0, 3],
        });

        let mesh = make_mesh_from_quad_groups_with_ao_brightness(&groups, AO_BRIGHTNESS).unwrap();
        let Some(VertexAttributeValues::Float32x4(colours)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("missing colour attribute");
        };
        let dark = AO_BRIGHTNESS[0];
        assert_eq!(
            colours,
            &vec![
                [0.8, 0.6, 0.4, 0.5],
                [0.8 * dark, 0.6 * dark, 0.4 * dark, 0.5],
                [0.8 * dark, 0.6 * dark, 0.4 * dark, 0.5],
                [0.8, 0.6, 0.4, 0.5],
            ]
        );

        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("missing indices");
        };
        assert_eq!(indices, &[0, 3, 1, 0, 2, 3]);
    }

    #[test]
    fn face_lighting_uses_hemisphere_levels_not_horizontal_fake_sun() {
        assert_eq!(face_brightness(Direction::Up), SKY_FACE_BRIGHTNESS);
        assert_eq!(
            face_brightness(Direction::Down),
            GROUND_BOUNCE_FACE_BRIGHTNESS
        );

        for side in [
            Direction::Left,
            Direction::Right,
            Direction::Forward,
            Direction::Backward,
        ] {
            assert_eq!(face_brightness(side), HORIZON_FACE_BRIGHTNESS);
        }
    }

    #[test]
    fn mesh_bakes_face_lighting_and_ao_into_colours() {
        let mut groups = QuadGroups::default();
        let color = Vec4::new(1.0, 0.5, 0.25, 0.75);
        groups.groups[Direction::Right as usize].push(Quad {
            voxel: UVec3::ZERO,
            color,
            uv: Rect::new(0.0, 0.0, 1.0, 1.0),
            ao: [3, 2, 1, 0],
        });

        let ao_brightness = [0.25, 0.5, 0.75, 1.0];
        let mesh = make_mesh_from_quad_groups_with_ao_brightness(&groups, ao_brightness).unwrap();
        let Some(VertexAttributeValues::Float32x4(colours)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("missing colour attribute");
        };

        let face_light = HORIZON_FACE_BRIGHTNESS;
        assert_eq!(
            colours,
            &vec![
                [1.0 * face_light, 0.5 * face_light, 0.25 * face_light, 0.75],
                [
                    1.0 * face_light * 0.75,
                    0.5 * face_light * 0.75,
                    0.25 * face_light * 0.75,
                    0.75,
                ],
                [
                    1.0 * face_light * 0.5,
                    0.5 * face_light * 0.5,
                    0.25 * face_light * 0.5,
                    0.75,
                ],
                [
                    1.0 * face_light * 0.25,
                    0.5 * face_light * 0.25,
                    0.25 * face_light * 0.25,
                    0.75,
                ],
            ]
        );
    }

    #[test]
    fn mesh_rebuild_marker_is_removed_after_rebuild() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<AmbientOcclusionSettings>()
            .insert_resource(test_texture_map())
            .insert_resource(BlockStandardMaterials::test_handles())
            .add_systems(Update, rebuild_chunk_meshes);

        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Stone;
        let chunk_entity = app
            .world_mut()
            .spawn((ChunkPosition(IVec3::ZERO), chunk, ChunkNeedsMeshRebuild))
            .id();

        app.update();

        let world = app.world();
        assert!(world.get::<ChunkNeedsMeshRebuild>(chunk_entity).is_none());
        let children = world.get::<Children>(chunk_entity).unwrap();
        let mesh_child_count = children
            .iter()
            .filter(|child| world.get::<Mesh3d>(*child).is_some())
            .count();
        assert_eq!(mesh_child_count, 1);
    }

    #[test]
    fn adjacent_leaves_emit_one_shared_double_sided_face() {
        let texture_map = test_texture_map();
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::OakLeaves;
        chunk.blocks[1][0][0] = BlockType::OakLeaves;

        let groups = make_layered_quad_groups(&chunk, &texture_map);
        let leaf_groups = &groups.layers[BlockMaterialLayer::CutoutDoubleSided.index()];

        assert_eq!(quad_count(leaf_groups), 11);
        assert_eq!(
            groups.layers[BlockMaterialLayer::Opaque.index()]
                .groups
                .iter()
                .map(Vec::len)
                .sum::<usize>(),
            0
        );
        assert_eq!(leaf_groups.groups[Direction::Right as usize].len(), 2);
        assert_eq!(leaf_groups.groups[Direction::Left as usize].len(), 1);
    }

    #[test]
    fn leaves_do_not_occlude_opaque_faces() {
        let texture_map = test_texture_map();
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Stone;
        chunk.blocks[1][0][0] = BlockType::OakLeaves;

        let groups = make_layered_quad_groups(&chunk, &texture_map);
        let opaque_groups = &groups.layers[BlockMaterialLayer::Opaque.index()];
        let leaf_groups = &groups.layers[BlockMaterialLayer::CutoutDoubleSided.index()];

        assert_eq!(quad_count(opaque_groups), 6);
        assert_eq!(quad_count(leaf_groups), 5);
        assert_eq!(opaque_groups.groups[Direction::Right as usize].len(), 1);
        assert_eq!(leaf_groups.groups[Direction::Left as usize].len(), 0);
    }

    #[test]
    fn chunk_meshes_are_split_by_render_layer() {
        let texture_map = test_texture_map();
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Stone;
        chunk.blocks[3][0][0] = BlockType::OakLeaves;

        let layers = make_chunk_meshes(&chunk, &texture_map)
            .into_iter()
            .map(|(layer, _)| layer)
            .collect::<Vec<_>>();

        assert_eq!(
            layers,
            vec![
                BlockMaterialLayer::Opaque,
                BlockMaterialLayer::CutoutDoubleSided
            ]
        );
    }
}
