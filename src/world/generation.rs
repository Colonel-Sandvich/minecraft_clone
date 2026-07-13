use bevy::prelude::*;

use crate::{
    block::BlockType,
    world::{
        chunk::{CHUNK_ISIZE, CHUNK_SIZE, Chunk, ChunkCell, ChunkPos, WorldBlockPos},
        definition::{DimensionDefinition, GeneratorProfile},
    },
};

// KEEP THIS AT 1. IF THERE ARE BREAKING CHANGES IN CHUNK FORMAT THAT'S FINE I WILL JUST DELETE THE dev save world and make a new one!
pub const CHUNK_FORMAT_VERSION: u32 = 1;
pub const WORLD_GENERATOR_VERSION: u32 = 1;
pub const DEFAULT_DIMENSION_HEIGHT_IN_SUB_CHUNKS: usize = 5;
pub const DEFAULT_DEV_WORLD_SEED: u64 = 0x11c7_7473_eead_0b0f;
pub const MIN_WORLD_HEIGHT_CHUNKS: usize = 1;
pub const MAX_WORLD_HEIGHT_CHUNKS: usize = (u8::MAX as usize + 1) / CHUNK_SIZE;

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
    height: WorldHeight,
}

/// A validated vertical world extent in chunk units.
///
/// The upper bound keeps absolute block heights representable by the persisted
/// `u8` chunk heightmap.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldHeight(u8);

impl WorldHeight {
    pub const DEFAULT: Self = match Self::new(DEFAULT_DIMENSION_HEIGHT_IN_SUB_CHUNKS) {
        Ok(height) => height,
        Err(_) => panic!("default world height must be valid"),
    };

    pub const fn new(chunks: usize) -> Result<Self, InvalidWorldHeight> {
        if chunks < MIN_WORLD_HEIGHT_CHUNKS || chunks > MAX_WORLD_HEIGHT_CHUNKS {
            return Err(InvalidWorldHeight { chunks });
        }
        Ok(Self(chunks as u8))
    }

    pub const fn chunks(self) -> usize {
        self.0 as usize
    }

    pub const fn chunks_i32(self) -> i32 {
        self.0 as i32
    }

    pub const fn blocks(self) -> i32 {
        self.chunks_i32() * CHUNK_ISIZE
    }

    pub const fn contains_chunk(self, position: ChunkPos) -> bool {
        position.y() >= 0 && position.y() < self.chunks_i32()
    }
}

impl Default for WorldHeight {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<usize> for WorldHeight {
    type Error = InvalidWorldHeight;

    fn try_from(chunks: usize) -> Result<Self, Self::Error> {
        Self::new(chunks)
    }
}

impl From<WorldHeight> for usize {
    fn from(height: WorldHeight) -> Self {
        height.chunks()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidWorldHeight {
    pub chunks: usize,
}

impl std::fmt::Display for InvalidWorldHeight {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "world height must be {MIN_WORLD_HEIGHT_CHUNKS}..={MAX_WORLD_HEIGHT_CHUNKS} chunks, got {}",
            self.chunks
        )
    }
}

impl std::error::Error for InvalidWorldHeight {}

impl WorldMetadata {
    pub const fn with_seed(seed: u64) -> Self {
        Self {
            seed,
            generator_version: WORLD_GENERATOR_VERSION,
            chunk_format_version: CHUNK_FORMAT_VERSION,
            height: WorldHeight::DEFAULT,
        }
    }

    pub fn with_height_chunks(mut self, chunks: usize) -> Result<Self, InvalidWorldHeight> {
        self.height = WorldHeight::new(chunks)?;
        Ok(self)
    }

    pub const fn height(&self) -> WorldHeight {
        self.height
    }

    pub const fn height_chunks(&self) -> usize {
        self.height.chunks()
    }

    pub fn world_height_blocks(&self) -> i32 {
        self.height.blocks()
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

    if chunk_pos == IVec3::ZERO {
        let surface_y = terrain_height(metadata, 8, 8);
        let local_y = surface_y + 1;
        if (0..CHUNK_ISIZE).contains(&local_y) {
            let y = local_y as usize;
            chunk.set_cell_xyz(8, y, 8, BlockType::Glass.into());
            chunk.set_cell_xyz(7, y, 8, BlockType::Glass.into());
            chunk.set_cell_xyz(9, y, 8, BlockType::Glass.into());
            chunk.set_cell_xyz(8, y, 7, BlockType::Glass.into());
            chunk.set_cell_xyz(8, y, 9, BlockType::Glass.into());
        }
    }

    chunk
}

/// Generates a chunk using an explicit, immutable generator profile.
///
/// Column loading captures this profile before starting asynchronous work, so
/// switching the active dimension cannot change a task's generation behavior.
pub fn generate_chunk_for_profile(
    metadata: &WorldMetadata,
    profile: GeneratorProfile,
    chunk_pos: ChunkPos,
) -> Chunk {
    match profile {
        GeneratorProfile::OverworldV1 => generate_chunk(metadata, chunk_pos.as_ivec3()),
        GeneratorProfile::GrassFloorV1 => generate_grass_floor_chunk(chunk_pos),
        GeneratorProfile::CenterGlassPlatformV1 => generate_center_glass_platform_chunk(chunk_pos),
    }
}

pub fn generate_dimension_chunk(
    metadata: &WorldMetadata,
    definition: &DimensionDefinition,
    chunk_pos: ChunkPos,
) -> Chunk {
    generate_chunk_for_profile(metadata, definition.generator(), chunk_pos)
}

fn generate_grass_floor_chunk(chunk_pos: ChunkPos) -> Chunk {
    if chunk_pos.y() != 0 {
        return Chunk::default();
    }

    Chunk::from_cell_fn(|_, y, _| {
        if y == 0 {
            BlockType::Grass.into()
        } else {
            ChunkCell::EMPTY
        }
    })
}

fn generate_center_glass_platform_chunk(chunk_pos: ChunkPos) -> Chunk {
    if chunk_pos != ChunkPos::ZERO {
        return Chunk::default();
    }

    Chunk::from_cell_fn(|_, y, _| {
        if y == 0 {
            BlockType::Glass.into()
        } else {
            ChunkCell::EMPTY
        }
    })
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
    let origin = ChunkPos::from_ivec3(target_chunk).origin().as_ivec3();
    let min_x = origin.x - OAK_TREE_MAX_CANOPY_RADIUS;
    let max_x = origin.x + CHUNK_ISIZE - 1 + OAK_TREE_MAX_CANOPY_RADIUS;
    let min_z = origin.z - OAK_TREE_MAX_CANOPY_RADIUS;
    let max_z = origin.z + CHUNK_ISIZE - 1 + OAK_TREE_MAX_CANOPY_RADIUS;

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

    for dy in 0..tree.trunk_height {
        blocks.push((tree.origin + IVec3::Y * dy, BlockType::OakLog));
    }

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
                    ivec3(dx, tree.trunk_height - 1 + dy, dz) + tree.origin,
                    BlockType::OakLeaves,
                ));
            }
        }
    }

    blocks
}

pub fn apply_oak_tree_to_chunk(tree: OakTree, chunk_pos: IVec3, chunk: &mut Chunk) {
    for (global_pos, block) in oak_tree_blocks(tree) {
        let Some(local_pos) =
            ChunkPos::from_ivec3(chunk_pos).local_of(WorldBlockPos::from_ivec3(global_pos))
        else {
            continue;
        };

        match block {
            BlockType::OakLog => {
                chunk.set_cell(local_pos.as_uvec3(), BlockType::OakLog.into());
            }
            BlockType::OakLeaves if chunk.cell(local_pos) == ChunkCell::EMPTY => {
                chunk.set_cell(local_pos.as_uvec3(), BlockType::OakLeaves.into());
            }
            BlockType::OakLeaves => {}
            _ => unreachable!("oak tree emitted non-oak block"),
        }
    }
}

fn generate_terrain_chunk(metadata: &WorldMetadata, chunk_pos: IVec3) -> Chunk {
    let mut surface_heights = [[0; CHUNK_SIZE]; CHUNK_SIZE];
    let origin = ChunkPos::from_ivec3(chunk_pos).origin().as_ivec3();

    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let world_x = origin.x + x as i32;
            let world_z = origin.z + z as i32;
            surface_heights[x][z] = terrain_height(metadata, world_x, world_z);
        }
    }

    Chunk::from_cell_fn(|x, y, z| {
        let world_y = origin.y + y as i32;
        terrain_cell_at(world_y, surface_heights[x][z])
    })
}

fn apply_oak_trees_for_chunk(metadata: &WorldMetadata, chunk_pos: IVec3, chunk: &mut Chunk) {
    for source_chunk in candidate_oak_tree_source_chunks(chunk_pos) {
        for tree in oak_tree_candidates_for_source_chunk(metadata, source_chunk) {
            apply_oak_tree_to_chunk(tree, chunk_pos, chunk);
        }
    }
}

fn terrain_cell_at(world_y: i32, surface_y: i32) -> ChunkCell {
    if world_y > surface_y {
        ChunkCell::EMPTY
    } else if world_y == surface_y {
        BlockType::Grass.into()
    } else if world_y >= surface_y - 3 {
        BlockType::Dirt.into()
    } else {
        BlockType::Stone.into()
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
    use crate::world::definition::{DimensionCatalog, DimensionId};

    fn assert_single_bottom_layer(chunk: &Chunk, block: BlockType) {
        for (cell, local) in chunk.iter() {
            let expected = if local.y() == 0 {
                block.into()
            } else {
                ChunkCell::EMPTY
            };
            assert_eq!(cell, expected, "unexpected cell at {local:?}");
        }
    }

    fn storage_fingerprint(chunk: &Chunk) -> (usize, u64) {
        let bytes = chunk.to_storage_bytes();
        let fingerprint = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
        });
        (bytes.len(), fingerprint)
    }

    #[test]
    fn world_height_accepts_persistable_bounds() {
        let minimum = WorldHeight::new(MIN_WORLD_HEIGHT_CHUNKS).unwrap();
        let maximum = WorldHeight::new(MAX_WORLD_HEIGHT_CHUNKS).unwrap();

        assert_eq!(minimum.chunks(), MIN_WORLD_HEIGHT_CHUNKS);
        assert_eq!(maximum.chunks(), MAX_WORLD_HEIGHT_CHUNKS);
        assert_eq!(maximum.blocks(), i32::from(u8::MAX) + 1);
    }

    #[test]
    fn world_height_rejects_values_outside_persistable_bounds() {
        assert_eq!(
            WorldHeight::new(MIN_WORLD_HEIGHT_CHUNKS - 1),
            Err(InvalidWorldHeight {
                chunks: MIN_WORLD_HEIGHT_CHUNKS - 1
            })
        );
        assert_eq!(
            WorldHeight::new(MAX_WORLD_HEIGHT_CHUNKS + 1),
            Err(InvalidWorldHeight {
                chunks: MAX_WORLD_HEIGHT_CHUNKS + 1
            })
        );
    }

    #[test]
    fn world_metadata_exposes_only_validated_height() {
        let metadata = WorldMetadata::with_seed(1)
            .with_height_chunks(MAX_WORLD_HEIGHT_CHUNKS)
            .unwrap();

        assert_eq!(
            metadata.height(),
            WorldHeight::new(MAX_WORLD_HEIGHT_CHUNKS).unwrap()
        );
        assert_eq!(metadata.height_chunks(), MAX_WORLD_HEIGHT_CHUNKS);
        assert_eq!(metadata.world_height_blocks(), metadata.height().blocks());
        assert!(
            WorldMetadata::with_seed(1)
                .with_height_chunks(MAX_WORLD_HEIGHT_CHUNKS + 1)
                .is_err()
        );
    }

    #[test]
    fn chunk_generation_is_deterministic_for_seed_and_position() {
        let metadata = WorldMetadata::with_seed(1234);

        assert_eq!(
            generate_chunk(&metadata, ivec3(2, 1, -3)),
            generate_chunk(&metadata, ivec3(2, 1, -3))
        );
    }

    #[test]
    fn overworld_profile_is_exactly_compatible_with_generate_chunk() {
        let cases = [
            (0, ChunkPos::ZERO),
            (1234, ChunkPos::new(2, 1, -3)),
            (DEFAULT_DEV_WORLD_SEED, ChunkPos::new(-1, 1, -1)),
            (u64::MAX, ChunkPos::new(7, 4, 9)),
        ];

        for (seed, position) in cases {
            let metadata = WorldMetadata::with_seed(seed);
            let compatibility = generate_chunk(&metadata, position.as_ivec3());
            let profiled =
                generate_chunk_for_profile(&metadata, GeneratorProfile::OverworldV1, position);

            assert_eq!(
                profiled.to_storage_bytes(),
                compatibility.to_storage_bytes()
            );
            assert_eq!(
                generate_chunk_for_profile(&metadata, GeneratorProfile::OverworldV1, position,)
                    .to_storage_bytes(),
                profiled.to_storage_bytes()
            );
        }
    }

    #[test]
    fn overworld_profile_retains_golden_chunks() {
        let cases = [
            (0, ChunkPos::ZERO, (526, 0xd7ba_7b6e_a672_1b8b)),
            (
                1234,
                ChunkPos::new(2, 1, -3),
                (1_053, 0x3603_bf74_e0b7_cd78),
            ),
            (
                DEFAULT_DEV_WORLD_SEED,
                ChunkPos::new(-1, 1, -1),
                (1_579, 0x24d7_a93b_6f03_f59b),
            ),
        ];

        for (seed, position, expected) in cases {
            let chunk = generate_chunk_for_profile(
                &WorldMetadata::with_seed(seed),
                GeneratorProfile::OverworldV1,
                position,
            );
            assert_eq!(
                storage_fingerprint(&chunk),
                expected,
                "seed={seed} position={position:?}"
            );
        }
    }

    #[test]
    fn grass_floor_is_one_grass_layer_in_every_bottom_chunk() {
        let metadata = WorldMetadata::with_seed(1);

        for position in [
            ChunkPos::ZERO,
            ChunkPos::new(-8, 0, 13),
            ChunkPos::new(21, 0, -34),
        ] {
            let chunk =
                generate_chunk_for_profile(&metadata, GeneratorProfile::GrassFloorV1, position);
            assert_single_bottom_layer(&chunk, BlockType::Grass);
        }
    }

    #[test]
    fn grass_floor_is_empty_above_and_below_chunk_y_zero() {
        let metadata = WorldMetadata::with_seed(1);

        for position in [ChunkPos::new(0, 1, 0), ChunkPos::new(-4, -1, 9)] {
            assert_eq!(
                generate_chunk_for_profile(&metadata, GeneratorProfile::GrassFloorV1, position,),
                Chunk::default()
            );
        }
    }

    #[test]
    fn center_glass_platform_is_one_layer_in_the_origin_chunk() {
        let chunk = generate_chunk_for_profile(
            &WorldMetadata::with_seed(1),
            GeneratorProfile::CenterGlassPlatformV1,
            ChunkPos::ZERO,
        );

        assert_single_bottom_layer(&chunk, BlockType::Glass);
    }

    #[test]
    fn center_glass_platform_is_empty_outside_the_origin_chunk() {
        let metadata = WorldMetadata::with_seed(1);

        for position in [
            ChunkPos::new(-1, 0, 0),
            ChunkPos::new(1, 0, 0),
            ChunkPos::new(0, 0, -1),
            ChunkPos::new(0, 0, 1),
            ChunkPos::new(0, 1, 0),
            ChunkPos::new(0, -1, 0),
        ] {
            assert_eq!(
                generate_chunk_for_profile(
                    &metadata,
                    GeneratorProfile::CenterGlassPlatformV1,
                    position,
                ),
                Chunk::default(),
                "expected {position:?} to be empty"
            );
        }
    }

    #[test]
    fn dimension_generation_dispatches_from_the_catalog_definition() {
        let metadata = WorldMetadata::with_seed(9876);
        let catalog = DimensionCatalog::for_world(&metadata);

        let overworld = catalog.get(DimensionId::OVERWORLD).unwrap();
        assert_eq!(
            generate_dimension_chunk(&metadata, overworld, ChunkPos::new(2, 1, -3)),
            generate_chunk(&metadata, ivec3(2, 1, -3))
        );

        let grass = catalog.get(DimensionId::GRASS_FLOOR).unwrap();
        assert_single_bottom_layer(
            &generate_dimension_chunk(&metadata, grass, ChunkPos::new(2, 0, -3)),
            BlockType::Grass,
        );

        let glass = catalog.get(DimensionId::CENTER_GLASS_PLATFORM).unwrap();
        assert_single_bottom_layer(
            &generate_dimension_chunk(&metadata, glass, ChunkPos::ZERO),
            BlockType::Glass,
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

        assert_eq!(
            neighbor.get_block(uvec3(0, 14, 8)),
            Some(BlockType::OakLeaves)
        );
        assert_eq!(
            neighbor.get_block(uvec3(1, 14, 8)),
            Some(BlockType::OakLeaves)
        );
        assert_eq!(neighbor.get_cell(uvec3(15, 14, 8)), ChunkCell::EMPTY);
    }

    #[test]
    fn negative_world_coordinates_use_floor_chunk_math() {
        let sources = candidate_oak_tree_source_chunks(ivec3(-1, 0, -1));

        assert!(sources.contains(&ivec2(-2, -1)));
        assert!(sources.contains(&ivec2(-1, -2)));
        assert!(sources.contains(&ivec2(0, -1)));
    }
}
