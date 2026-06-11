use bevy::prelude::*;

use crate::{
    block::BlockType,
    world::chunk::{CHUNK_ISIZE, CHUNK_SIZE, Chunk},
};

pub const CHUNK_FORMAT_VERSION: u32 = 1;
pub const WORLD_GENERATOR_VERSION: u32 = 1;
pub const DEFAULT_DIMENSION_HEIGHT_IN_SUB_CHUNKS: usize = 5;
pub const DEFAULT_DEV_WORLD_SEED: u64 = 0x11c7_7473_eead_0b0f;

const TERRAIN_BASE_HEIGHT: i32 = 18;
const TERRAIN_MIN_HEIGHT: i32 = 4;
const TERRAIN_TOP_PADDING: i32 = 12;

const OAK_TREE_ATTEMPTS_PER_CHUNK: u32 = 4;
pub const OAK_TREE_MAX_CANOPY_RADIUS: i32 = 2;
pub const OAK_TREE_MAX_HEIGHT: i32 = 7;

#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct WorldMetadata {
    pub seed: u64,
    pub generator_version: u32,
    pub chunk_format_version: u32,
    pub height_chunks: usize,
}

impl WorldMetadata {
    pub const fn with_seed(seed: u64) -> Self {
        Self {
            seed,
            generator_version: WORLD_GENERATOR_VERSION,
            chunk_format_version: CHUNK_FORMAT_VERSION,
            height_chunks: DEFAULT_DIMENSION_HEIGHT_IN_SUB_CHUNKS,
        }
    }

    pub fn world_height_blocks(&self) -> i32 {
        (self.height_chunks * CHUNK_SIZE) as i32
    }
}

impl Default for WorldMetadata {
    fn default() -> Self {
        Self::with_seed(DEFAULT_DEV_WORLD_SEED)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OakTree {
    pub origin: IVec3,
    pub trunk_height: i32,
}

pub fn generate_chunk(metadata: &WorldMetadata, chunk_pos: IVec3) -> Chunk {
    let mut chunk = generate_terrain_chunk(metadata, chunk_pos);
    apply_oak_trees_for_chunk(metadata, chunk_pos, &mut chunk);
    chunk
}

pub fn terrain_height(metadata: &WorldMetadata, world_x: i32, world_z: i32) -> i32 {
    let broad = value_noise_2d(metadata.seed, world_x, world_z, 32);
    let detail = value_noise_2d(metadata.seed ^ 0x9e37_79b9_7f4a_7c15, world_x, world_z, 11);
    let height = (TERRAIN_BASE_HEIGHT as f32 + broad * 10.0 + detail * 3.0).round() as i32;

    height.clamp(
        TERRAIN_MIN_HEIGHT,
        metadata.world_height_blocks() - TERRAIN_TOP_PADDING,
    )
}

pub fn candidate_oak_tree_source_chunks(target_chunk: IVec3) -> Vec<IVec2> {
    let min_x = target_chunk.x * CHUNK_ISIZE - OAK_TREE_MAX_CANOPY_RADIUS;
    let max_x = target_chunk.x * CHUNK_ISIZE + CHUNK_ISIZE - 1 + OAK_TREE_MAX_CANOPY_RADIUS;
    let min_z = target_chunk.z * CHUNK_ISIZE - OAK_TREE_MAX_CANOPY_RADIUS;
    let max_z = target_chunk.z * CHUNK_ISIZE + CHUNK_ISIZE - 1 + OAK_TREE_MAX_CANOPY_RADIUS;

    let source_min_x = div_floor(min_x, CHUNK_ISIZE);
    let source_max_x = div_floor(max_x, CHUNK_ISIZE);
    let source_min_z = div_floor(min_z, CHUNK_ISIZE);
    let source_max_z = div_floor(max_z, CHUNK_ISIZE);

    let mut sources = Vec::new();
    for z in source_min_z..=source_max_z {
        for x in source_min_x..=source_max_x {
            sources.push(ivec2(x, z));
        }
    }
    sources
}

pub fn oak_tree_candidates_for_source_chunk(
    metadata: &WorldMetadata,
    source_chunk: IVec2,
) -> Vec<OakTree> {
    let mut trees = Vec::new();

    for attempt in 0..OAK_TREE_ATTEMPTS_PER_CHUNK {
        let hash = hash_coords(
            metadata.seed,
            source_chunk.x,
            attempt as i32,
            source_chunk.y,
            0x6f61_6b5f_7472_6565,
        );

        if hash % 100 >= 22 {
            continue;
        }

        let local_x = ((hash >> 8) % CHUNK_SIZE as u64) as i32;
        let local_z = ((hash >> 16) % CHUNK_SIZE as u64) as i32;
        let world_x = source_chunk.x * CHUNK_ISIZE + local_x;
        let world_z = source_chunk.y * CHUNK_ISIZE + local_z;
        let surface_y = terrain_height(metadata, world_x, world_z);
        let trunk_height = 4 + ((hash >> 24) % 3) as i32;

        if surface_y + trunk_height + 2 >= metadata.world_height_blocks() {
            continue;
        }

        trees.push(OakTree {
            origin: ivec3(world_x, surface_y + 1, world_z),
            trunk_height,
        });
    }

    trees
}

pub fn oak_tree_blocks(tree: OakTree) -> Vec<(IVec3, BlockType)> {
    let mut blocks = Vec::new();

    for y in tree.origin.y..tree.origin.y + tree.trunk_height {
        blocks.push((ivec3(tree.origin.x, y, tree.origin.z), BlockType::OakLog));
    }

    let canopy_centre_y = tree.origin.y + tree.trunk_height - 1;
    for dy in -2_i32..=2 {
        let radius = if dy.abs() == 2 {
            1
        } else {
            OAK_TREE_MAX_CANOPY_RADIUS
        };
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                if dx == 0 && dz == 0 && dy <= 0 {
                    continue;
                }
                if dx.abs() + dz.abs() > radius + 1 {
                    continue;
                }

                blocks.push((
                    ivec3(tree.origin.x + dx, canopy_centre_y + dy, tree.origin.z + dz),
                    BlockType::OakLeaves,
                ));
            }
        }
    }

    blocks
}

pub fn apply_oak_tree_to_chunk(tree: OakTree, chunk_pos: IVec3, chunk: &mut Chunk) {
    for (global_pos, block) in oak_tree_blocks(tree) {
        let Some(local_pos) = global_to_local_in_chunk(chunk_pos, global_pos) else {
            continue;
        };

        let target = chunk.get_mut_uvec(local_pos);
        match block {
            BlockType::OakLog => *target = BlockType::OakLog,
            BlockType::OakLeaves if *target == BlockType::Air => *target = BlockType::OakLeaves,
            BlockType::OakLeaves => {}
            _ => unreachable!("oak tree emitted non-oak block"),
        }
    }
}

fn generate_terrain_chunk(metadata: &WorldMetadata, chunk_pos: IVec3) -> Chunk {
    let mut chunk = Chunk::default();

    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let world_x = chunk_pos.x * CHUNK_ISIZE + x as i32;
            let world_z = chunk_pos.z * CHUNK_ISIZE + z as i32;
            let surface_y = terrain_height(metadata, world_x, world_z);

            for y in 0..CHUNK_SIZE {
                let world_y = chunk_pos.y * CHUNK_ISIZE + y as i32;
                chunk.blocks[x][z][y] = terrain_block_at(world_y, surface_y);
            }
        }
    }

    chunk
}

fn apply_oak_trees_for_chunk(metadata: &WorldMetadata, chunk_pos: IVec3, chunk: &mut Chunk) {
    for source_chunk in candidate_oak_tree_source_chunks(chunk_pos) {
        for tree in oak_tree_candidates_for_source_chunk(metadata, source_chunk) {
            apply_oak_tree_to_chunk(tree, chunk_pos, chunk);
        }
    }
}

fn terrain_block_at(world_y: i32, surface_y: i32) -> BlockType {
    if world_y > surface_y {
        BlockType::Air
    } else if world_y == surface_y {
        BlockType::Grass
    } else if world_y >= surface_y - 3 {
        BlockType::Dirt
    } else {
        BlockType::Stone
    }
}

fn global_to_local_in_chunk(chunk_pos: IVec3, global_pos: IVec3) -> Option<UVec3> {
    let local = global_pos - chunk_pos * CHUNK_ISIZE;
    let in_bounds = |value: i32| (0..CHUNK_ISIZE).contains(&value);

    if in_bounds(local.x) && in_bounds(local.y) && in_bounds(local.z) {
        Some(local.as_uvec3())
    } else {
        None
    }
}

fn value_noise_2d(seed: u64, x: i32, z: i32, cell_size: i32) -> f32 {
    let x0 = div_floor(x, cell_size);
    let z0 = div_floor(z, cell_size);
    let x1 = x0 + 1;
    let z1 = z0 + 1;

    let tx = rem_floor(x, cell_size) as f32 / cell_size as f32;
    let tz = rem_floor(z, cell_size) as f32 / cell_size as f32;
    let sx = smoothstep(tx);
    let sz = smoothstep(tz);

    let n00 = hash_unit_signed(seed, x0, z0);
    let n10 = hash_unit_signed(seed, x1, z0);
    let n01 = hash_unit_signed(seed, x0, z1);
    let n11 = hash_unit_signed(seed, x1, z1);

    let nx0 = lerp(n00, n10, sx);
    let nx1 = lerp(n01, n11, sx);
    lerp(nx0, nx1, sz)
}

fn hash_unit_signed(seed: u64, x: i32, z: i32) -> f32 {
    let hash = hash_coords(seed, x, 0, z, 0x7465_7272_6169_6e31);
    let unit = (hash >> 11) as f64 / ((1_u64 << 53) - 1) as f64;
    (unit as f32) * 2.0 - 1.0
}

fn hash_coords(seed: u64, x: i32, y: i32, z: i32, salt: u64) -> u64 {
    let mut value = seed ^ salt;
    value = mix_u64(value ^ (x as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    value = mix_u64(value ^ (y as i64 as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9));
    mix_u64(value ^ (z as i64 as u64).wrapping_mul(0x94d0_49bb_1331_11eb))
}

fn mix_u64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn div_floor(value: i32, divisor: i32) -> i32 {
    value.div_euclid(divisor)
}

fn rem_floor(value: i32, divisor: i32) -> i32 {
    value.rem_euclid(divisor)
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_generation_is_deterministic_for_seed_and_position() {
        let metadata = WorldMetadata::with_seed(1234);

        assert_eq!(
            generate_chunk(&metadata, ivec3(2, 1, -3)),
            generate_chunk(&metadata, ivec3(2, 1, -3))
        );
    }

    #[test]
    fn chunk_generation_changes_with_seed() {
        let a = generate_chunk(&WorldMetadata::with_seed(1), IVec3::ZERO);
        let b = generate_chunk(&WorldMetadata::with_seed(2), IVec3::ZERO);

        assert_ne!(a, b);
    }

    #[test]
    fn tree_pull_sources_include_bordering_chunks() {
        let sources = candidate_oak_tree_source_chunks(IVec3::ZERO);

        assert!(sources.contains(&IVec2::ZERO));
        assert!(sources.contains(&ivec2(-1, 0)));
        assert!(sources.contains(&ivec2(1, 0)));
        assert!(sources.contains(&ivec2(0, -1)));
        assert!(sources.contains(&ivec2(0, 1)));
    }

    #[test]
    fn tree_parts_can_be_pulled_into_neighbor_chunk_without_mutating_source() {
        let tree = OakTree {
            origin: ivec3(CHUNK_ISIZE - 1, 10, 8),
            trunk_height: 5,
        };
        let mut neighbor = Chunk::default();

        apply_oak_tree_to_chunk(tree, ivec3(1, 0, 0), &mut neighbor);

        assert_eq!(neighbor.get(uvec3(0, 14, 8)), BlockType::OakLeaves);
        assert_eq!(neighbor.get(uvec3(1, 14, 8)), BlockType::OakLeaves);
        assert_eq!(neighbor.get(uvec3(15, 14, 8)), BlockType::Air);
    }

    #[test]
    fn negative_world_coordinates_use_floor_chunk_math() {
        let sources = candidate_oak_tree_source_chunks(ivec3(-1, 0, -1));

        assert!(sources.contains(&ivec2(-2, -1)));
        assert!(sources.contains(&ivec2(-1, -2)));
        assert!(sources.contains(&ivec2(0, -1)));
    }
}
