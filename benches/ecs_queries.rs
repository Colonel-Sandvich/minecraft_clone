use std::{hint::black_box, sync::Arc, time::Duration};

use avian3d::prelude::{Collider, RigidBody};
use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
};
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use minecraft_clone::{
    block::BlockType,
    world::{
        WORLD_COLLISION_LAYERS, WorldMetadata,
        chunk::{
            CHUNK_SIZE, Chunk, ChunkCell, ChunkContentCounts, ChunkHeightmap, ChunkLight,
            ChunkNeedsColliderRebuild, ChunkNeedsFluidStep, ChunkNeedsLightRebuild,
            ChunkNeedsMeshRebuild, ChunkNeedsRenderLightUpload, ChunkNeedsSave, ChunkPosition,
            FluidProfile, FluidState,
            light::compute_light_region,
            mesh::{ChunkMeshBlocks, ChunkMeshLight, mesher::build},
        },
        dimension::{Active, Dimension},
        generation::generate_chunk,
    },
};

const MESH_REBUILD_SIDE: i32 = 16;
const MESH_REBUILD_HEIGHT: i32 = 4;
const LIGHT_UPLOAD_WORLD_CHUNKS: usize = 4096;
const LIGHT_UPLOAD_CHILDREN_PER_CHUNK: usize = 3;
const LIGHT_REBUILD_SIDE: i32 = 32;
const LIGHT_REBUILD_HEIGHT: i32 = 4;
const FLUID_SIDE: i32 = 32;
const COLLIDER_WORLD_CHUNKS: usize = 4096;

#[derive(Resource, Default)]
struct LightUploadBenchStats {
    dirty_chunks: usize,
    child_updates: usize,
}

#[derive(Resource, Default)]
struct MeshRebuildBenchStats {
    dirty_chunks: usize,
    faces: usize,
}

#[derive(Resource, Default)]
struct LightRebuildBenchStats {
    dirty_positions: usize,
    targets: usize,
    changed_lights: usize,
}

#[derive(Resource, Default)]
struct FluidBenchStats {
    stepped_chunks: usize,
    boundary_flows: usize,
}

#[derive(Resource, Default)]
struct ColliderBenchStats {
    dirty_chunks: usize,
    voxels: usize,
}

#[derive(Resource)]
struct BenchFluidStepBudget(usize);

struct BoundaryFlow {
    target_pos: IVec3,
    x: usize,
    y: usize,
    z: usize,
    fluid: FluidState,
}

fn chunk_pos(index: usize) -> IVec3 {
    let x = index as i32 % 64;
    let y = index as i32 / (64 * 64);
    let z = index as i32 / 64 % 64;
    ivec3(x, y, z)
}

fn build_mesh_rebuild_world(dirty_chunks: usize) -> World {
    let mut world = World::new();
    world.insert_resource(MeshRebuildBenchStats::default());

    let mut metadata = WorldMetadata::with_seed(1);
    metadata.height_chunks = MESH_REBUILD_HEIGHT as usize;

    let mut index = 0usize;
    for x in 0..MESH_REBUILD_SIDE {
        for z in 0..MESH_REBUILD_SIDE {
            for y in 0..MESH_REBUILD_HEIGHT {
                let pos = ivec3(x, y, z);
                let mut entity = world.spawn((
                    ChunkPosition(pos),
                    generate_chunk(&metadata, pos),
                    ChunkLight::default(),
                ));
                if index < dirty_chunks {
                    entity.insert(ChunkNeedsMeshRebuild);
                }
                index += 1;
            }
        }
    }

    world
}

fn mesh_rebuild_iter_system(
    all_chunks_q: Query<(&ChunkPosition, &Chunk)>,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>,
    mut stats: ResMut<MeshRebuildBenchStats>,
) {
    if dirty_chunks_q.is_empty() {
        return;
    }

    let chunks_by_pos = all_chunks_q
        .iter()
        .map(|(pos, chunk)| (pos.0, chunk))
        .collect::<HashMap<_, _>>();
    let lights_by_pos = light_q
        .iter()
        .map(|(pos, light)| (pos.0, light))
        .collect::<HashMap<_, _>>();

    let mut dirty_chunks = 0;
    let mut faces = 0;
    for (chunk_pos, _) in &dirty_chunks_q {
        dirty_chunks += 1;
        let blocks = ChunkMeshBlocks::from_chunks(chunk_pos.0, &chunks_by_pos);
        let layers = build(&blocks);
        faces += layers.iter().map(|layer| layer.faces.len()).sum::<usize>();
        black_box(ChunkLight::build_padded_light_data(
            chunk_pos.0,
            &lights_by_pos,
        ));
    }

    stats.dirty_chunks = dirty_chunks;
    stats.faces = faces;
}

fn mesh_rebuild_contiguous_system(
    all_chunks_q: Query<(&ChunkPosition, &Chunk)>,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>,
    mut stats: ResMut<MeshRebuildBenchStats>,
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

    let mut lights_by_pos = HashMap::with_capacity(light_q.iter().len());
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

    let mut dirty_chunks = 0;
    let mut faces = 0;
    for (chunk_pos, _) in &dirty_chunks_q {
        dirty_chunks += 1;
        let blocks = ChunkMeshBlocks::from_chunks(chunk_pos.0, &chunks_by_pos);
        let layers = build(&blocks);
        faces += layers.iter().map(|layer| layer.faces.len()).sum::<usize>();
        black_box(ChunkLight::build_padded_light_data(
            chunk_pos.0,
            &lights_by_pos,
        ));
    }

    stats.dirty_chunks = dirty_chunks;
    stats.faces = faces;
}

fn build_light_upload_world(dirty_chunks: usize) -> World {
    let mut world = World::new();
    world.insert_resource(LightUploadBenchStats::default());
    let light_data: Arc<[u32]> = Arc::from(vec![0u32; 18 * 18 * 18 / 4]);

    for index in 0..LIGHT_UPLOAD_WORLD_CHUNKS {
        let pos = chunk_pos(index);
        let mut entity = world.spawn((ChunkPosition(pos), ChunkLight::default()));
        if index < dirty_chunks {
            entity.insert(ChunkNeedsRenderLightUpload);
        }
        let parent = entity.id();

        for _ in 0..LIGHT_UPLOAD_CHILDREN_PER_CHUNK {
            world.spawn((ChildOf(parent), ChunkMeshLight::new(light_data.clone())));
        }
    }

    world
}

fn build_lights_by_pos<'w>(
    light_q: &'w Query<'w, '_, (&ChunkPosition, &ChunkLight)>,
) -> HashMap<IVec3, &'w ChunkLight> {
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

    lights_by_pos
}

fn build_lights_by_pos_iter<'w>(
    light_q: &'w Query<'w, '_, (&ChunkPosition, &ChunkLight)>,
) -> HashMap<IVec3, &'w ChunkLight> {
    light_q.iter().map(|(pos, light)| (pos.0, light)).collect()
}

fn light_upload_map_iter_system(
    mut commands: Commands,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), With<ChunkNeedsRenderLightUpload>>,
    children_q: Query<&Children>,
    mut mesh_light_q: Query<&mut ChunkMeshLight>,
    mut stats: ResMut<LightUploadBenchStats>,
) {
    let lights_by_pos = build_lights_by_pos_iter(&light_q);
    run_light_upload_dirty_loop(
        &mut commands,
        &dirty_chunks_q,
        &children_q,
        &mut mesh_light_q,
        &lights_by_pos,
        &mut stats,
    );
}

fn light_upload_map_contiguous_system(
    mut commands: Commands,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), With<ChunkNeedsRenderLightUpload>>,
    children_q: Query<&Children>,
    mut mesh_light_q: Query<&mut ChunkMeshLight>,
    mut stats: ResMut<LightUploadBenchStats>,
) {
    let lights_by_pos = build_lights_by_pos(&light_q);
    run_light_upload_dirty_loop(
        &mut commands,
        &dirty_chunks_q,
        &children_q,
        &mut mesh_light_q,
        &lights_by_pos,
        &mut stats,
    );
}

fn run_light_upload_dirty_loop(
    commands: &mut Commands,
    dirty_chunks_q: &Query<(&ChunkPosition, Entity), With<ChunkNeedsRenderLightUpload>>,
    children_q: &Query<&Children>,
    mesh_light_q: &mut Query<&mut ChunkMeshLight>,
    lights_by_pos: &HashMap<IVec3, &ChunkLight>,
    stats: &mut LightUploadBenchStats,
) {
    let mut dirty_chunks = 0;
    let mut child_updates = 0;

    for (chunk_pos, chunk_entity) in dirty_chunks_q {
        dirty_chunks += 1;
        let light_data: Arc<[u32]> =
            ChunkLight::build_padded_light_data(chunk_pos.0, lights_by_pos).into();
        if let Ok(children) = children_q.get(chunk_entity) {
            for child in children {
                if let Ok(mut light) = mesh_light_q.get_mut(*child) {
                    light.replace(light_data.clone());
                    child_updates += 1;
                }
            }
        }

        commands
            .entity(chunk_entity)
            .remove::<ChunkNeedsRenderLightUpload>();
    }

    stats.dirty_chunks = dirty_chunks;
    stats.child_updates = child_updates;
}

fn light_upload_dirty_iter_system(
    mut commands: Commands,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), With<ChunkNeedsRenderLightUpload>>,
    children_q: Query<&Children>,
    mut mesh_light_q: Query<&mut ChunkMeshLight>,
    mut stats: ResMut<LightUploadBenchStats>,
) {
    let lights_by_pos = build_lights_by_pos(&light_q);
    run_light_upload_dirty_loop(
        &mut commands,
        &dirty_chunks_q,
        &children_q,
        &mut mesh_light_q,
        &lights_by_pos,
        &mut stats,
    );
}

fn light_upload_dirty_contiguous_system(
    mut commands: Commands,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), With<ChunkNeedsRenderLightUpload>>,
    children_q: Query<&Children>,
    mut mesh_light_q: Query<&mut ChunkMeshLight>,
    mut stats: ResMut<LightUploadBenchStats>,
) {
    let lights_by_pos = build_lights_by_pos(&light_q);
    let mut dirty_chunks = 0;
    let mut child_updates = 0;

    for (positions, entities) in dirty_chunks_q
        .contiguous_iter()
        .expect("chunk light dirty upload query should stay dense")
    {
        dirty_chunks += positions.len();
        for (chunk_pos, chunk_entity) in positions.iter().zip(entities.iter().copied()) {
            let light_data: Arc<[u32]> =
                ChunkLight::build_padded_light_data(chunk_pos.0, &lights_by_pos).into();
            if let Ok(children) = children_q.get(chunk_entity) {
                for child in children {
                    if let Ok(mut light) = mesh_light_q.get_mut(*child) {
                        light.replace(light_data.clone());
                        child_updates += 1;
                    }
                }
            }

            commands
                .entity(chunk_entity)
                .remove::<ChunkNeedsRenderLightUpload>();
        }
    }

    stats.dirty_chunks = dirty_chunks;
    stats.child_updates = child_updates;
}

fn build_light_rebuild_world(dirty_columns: usize, with_active_dimension: bool) -> World {
    let mut world = World::new();
    let mut metadata = WorldMetadata::with_seed(1);
    metadata.height_chunks = LIGHT_REBUILD_HEIGHT as usize;
    world.insert_resource(metadata);
    world.insert_resource(LightRebuildBenchStats::default());

    let mut loaded_chunks = HashMap::new();
    let mut column_index = 0usize;
    for x in 0..LIGHT_REBUILD_SIDE {
        for z in 0..LIGHT_REBUILD_SIDE {
            let dirty = column_index < dirty_columns;
            column_index += 1;

            for y in 0..LIGHT_REBUILD_HEIGHT {
                let pos = ivec3(x, y, z);
                let mut entity = world.spawn((
                    ChunkPosition(pos),
                    Chunk::default(),
                    ChunkLight::default(),
                    ChunkHeightmap::default(),
                ));
                if dirty && y == 0 {
                    entity.insert(ChunkNeedsLightRebuild);
                }
                loaded_chunks.insert(pos, entity.id());
            }
        }
    }

    if with_active_dimension {
        world.spawn((
            Dimension {
                chunks: loaded_chunks,
            },
            Active,
        ));
    }

    world
}

fn active_fluid_positions(active_chunks: usize) -> Vec<IVec3> {
    let mut positions = Vec::with_capacity(active_chunks);
    for z in 1..FLUID_SIDE - 1 {
        for x in 1..FLUID_SIDE - 1 {
            positions.push(ivec3(x, 0, z));
            if positions.len() == active_chunks {
                return positions;
            }
        }
    }

    positions
}

fn fluid_bench_chunk(active: bool) -> Chunk {
    let mut chunk = Chunk::default();
    if active {
        for z in 1..CHUNK_SIZE - 1 {
            chunk.set_block(uvec3(1, 0, z as u32), BlockType::Stone);
            chunk.set_cell(uvec3(1, 1, z as u32), ChunkCell::water_source());
        }
    }

    chunk
}

fn build_fluid_world(active_chunks: usize) -> World {
    let mut world = World::new();
    world.insert_resource(BenchFluidStepBudget(active_chunks));
    world.insert_resource(FluidBenchStats::default());
    let active_positions = active_fluid_positions(active_chunks)
        .into_iter()
        .collect::<HashSet<_>>();

    for x in 0..FLUID_SIDE {
        for z in 0..FLUID_SIDE {
            let pos = ivec3(x, 0, z);
            let active = active_positions.contains(&pos);
            let chunk = fluid_bench_chunk(active);
            let counts = chunk.compute_content_counts();
            let mut entity = world.spawn((ChunkPosition(pos), chunk, counts));
            if active {
                entity.insert(ChunkNeedsFluidStep);
            }
        }
    }

    world
}

fn collider_bench_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..8 {
                if (x + y + z) % 3 != 0 {
                    chunk.set_cell_xyz(x, y, z, BlockType::Stone.into());
                }
            }
        }
    }

    chunk
}

fn build_collider_world(dirty_chunks: usize) -> World {
    let mut world = World::new();
    world.insert_resource(ColliderBenchStats::default());
    let chunk = collider_bench_chunk();
    let counts = chunk.compute_content_counts();

    for index in 0..COLLIDER_WORLD_CHUNKS {
        let mut entity = world.spawn((chunk.clone(), counts));
        if index < dirty_chunks {
            entity.insert(ChunkNeedsColliderRebuild);
            let parent = entity.id();
            world.spawn((
                ChildOf(parent),
                Collider::cuboid(1.0, 1.0, 1.0),
                WORLD_COLLISION_LAYERS,
                RigidBody::Static,
            ));
        }
    }

    world
}

fn collider_voxels(chunk: &Chunk, meta: &ChunkContentCounts) -> Vec<IVec3> {
    let mut voxels = Vec::with_capacity(meta.solid as usize);
    for (cell, (x, y, z)) in chunk.iter() {
        if cell.is_solid() {
            voxels.push(IVec3::new(x as i32, y as i32, z as i32));
        }
    }

    voxels
}

fn rebuild_one_collider(
    commands: &mut Commands,
    chunk: &Chunk,
    meta: &ChunkContentCounts,
    chunk_entity: Entity,
) -> usize {
    if meta.solid == 0 {
        return 0;
    }

    let voxels = collider_voxels(chunk, meta);
    if voxels.is_empty() {
        return 0;
    }

    let voxel_count = voxels.len();
    commands.spawn((
        ChildOf(chunk_entity),
        Collider::voxels(Vec3::ONE, &voxels),
        WORLD_COLLISION_LAYERS,
        RigidBody::Static,
    ));
    voxel_count
}

fn collider_rebuild_iter_system(
    mut commands: Commands,
    chunks_q: Query<
        (&Chunk, &ChunkContentCounts, Entity, Option<&Children>),
        With<ChunkNeedsColliderRebuild>,
    >,
    collider_q: Query<Entity, With<Collider>>,
    mut stats: ResMut<ColliderBenchStats>,
) {
    let mut dirty_chunks = 0;
    let mut voxels = 0;
    for (chunk, meta, chunk_entity, children) in chunks_q.iter() {
        dirty_chunks += 1;
        if let Some(children) = children {
            for collider_entity in collider_q.iter_many(children) {
                commands.get_entity(collider_entity).unwrap().despawn();
            }
        }

        voxels += rebuild_one_collider(&mut commands, chunk, meta, chunk_entity);
        commands
            .entity(chunk_entity)
            .remove::<ChunkNeedsColliderRebuild>();
    }

    stats.dirty_chunks = dirty_chunks;
    stats.voxels = voxels;
}

fn collider_rebuild_contiguous_system(
    mut commands: Commands,
    chunks_q: Query<(&Chunk, &ChunkContentCounts, Entity), With<ChunkNeedsColliderRebuild>>,
    children_q: Query<&Children>,
    collider_q: Query<Entity, With<Collider>>,
    mut stats: ResMut<ColliderBenchStats>,
) {
    let mut dirty_chunks = 0;
    let mut voxels = 0;
    for (chunks, counts, entities) in chunks_q
        .contiguous_iter()
        .expect("chunk collider rebuild query should stay dense")
    {
        dirty_chunks += chunks.len();
        for ((chunk, meta), chunk_entity) in chunks.iter().zip(counts.iter()).zip(entities.iter()) {
            if let Ok(children) = children_q.get(*chunk_entity) {
                for collider_entity in collider_q.iter_many(children) {
                    commands.get_entity(collider_entity).unwrap().despawn();
                }
            }

            voxels += rebuild_one_collider(&mut commands, chunk, meta, *chunk_entity);
            commands
                .entity(*chunk_entity)
                .remove::<ChunkNeedsColliderRebuild>();
        }
    }

    stats.dirty_chunks = dirty_chunks;
    stats.voxels = voxels;
}

fn collect_boundary_flows(
    chunk: &Chunk,
    chunk_pos: IVec3,
    profile: &FluidProfile,
) -> Vec<BoundaryFlow> {
    let mut boundary_flows = Vec::new();
    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let Some(fluid) = chunk.cell_xyz(x, y, z).as_fluid() else {
                    continue;
                };
                if fluid.ty() != profile.ty {
                    continue;
                }

                if x == 0 {
                    if let Some(next_fluid) = profile.decayed_flow(fluid) {
                        boundary_flows.push(BoundaryFlow {
                            target_pos: chunk_pos + IVec3::NEG_X,
                            x: CHUNK_SIZE - 1,
                            y,
                            z,
                            fluid: next_fluid,
                        });
                    }
                }
                if x == CHUNK_SIZE - 1 {
                    if let Some(next_fluid) = profile.decayed_flow(fluid) {
                        boundary_flows.push(BoundaryFlow {
                            target_pos: chunk_pos + IVec3::X,
                            x: 0,
                            y,
                            z,
                            fluid: next_fluid,
                        });
                    }
                }
                if z == 0 {
                    if let Some(next_fluid) = profile.decayed_flow(fluid) {
                        boundary_flows.push(BoundaryFlow {
                            target_pos: chunk_pos + IVec3::NEG_Z,
                            x,
                            y,
                            z: CHUNK_SIZE - 1,
                            fluid: next_fluid,
                        });
                    }
                }
                if z == CHUNK_SIZE - 1 {
                    if let Some(next_fluid) = profile.decayed_flow(fluid) {
                        boundary_flows.push(BoundaryFlow {
                            target_pos: chunk_pos + IVec3::Z,
                            x,
                            y,
                            z: 0,
                            fluid: next_fluid,
                        });
                    }
                }
                if y == 0 {
                    boundary_flows.push(BoundaryFlow {
                        target_pos: chunk_pos + IVec3::NEG_Y,
                        x,
                        y: CHUNK_SIZE - 1,
                        z,
                        fluid: profile.falling(),
                    });
                }
            }
        }
    }

    boundary_flows
}

fn chunk_neighbor_offsets() -> impl Iterator<Item = IVec3> {
    (-1..=1).flat_map(|x| {
        (-1..=1).flat_map(move |y| {
            (-1..=1).filter_map(move |z| {
                let offset = ivec3(x, y, z);
                (offset != IVec3::ZERO).then_some(offset)
            })
        })
    })
}

fn light_rebuild_targets(
    dirty_positions: &[IVec3],
    loaded_chunks: &HashMap<IVec3, Entity>,
    height_chunks: i32,
) -> HashSet<IVec3> {
    let columns = dirty_positions
        .iter()
        .map(|pos| ivec2(pos.x, pos.z))
        .collect::<HashSet<_>>();

    let mut targets = HashSet::new();
    for column in columns {
        for y in 0..height_chunks {
            let pos = ivec3(column.x, y, column.y);
            if loaded_chunks.contains_key(&pos) {
                targets.insert(pos);
            }
        }
    }

    targets
}

fn rebuild_chunk_light_iter_system(
    mut commands: Commands,
    needs_rebuild: Query<(Entity, &ChunkPosition), With<ChunkNeedsLightRebuild>>,
    all_chunks: Query<(Entity, &ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
    dimension: Option<Single<&Dimension, With<Active>>>,
    metadata: Res<WorldMetadata>,
    mut stats: ResMut<LightRebuildBenchStats>,
) {
    if needs_rebuild.is_empty() {
        return;
    }

    let dirty_positions = needs_rebuild
        .iter()
        .map(|(_, pos)| pos.0)
        .collect::<Vec<_>>();

    let fallback_loaded_chunks;
    let loaded_chunks = if let Some(dimension) = dimension.as_ref() {
        &dimension.chunks
    } else {
        fallback_loaded_chunks = all_chunks
            .iter()
            .map(|(entity, pos, _, _, _)| (pos.0, entity))
            .collect::<HashMap<_, _>>();
        &fallback_loaded_chunks
    };

    rebuild_chunk_light_rest(
        &mut commands,
        &needs_rebuild,
        &all_chunks,
        loaded_chunks,
        &dirty_positions,
        metadata.height_chunks as i32,
        &mut stats,
    );
}

fn rebuild_chunk_light_contiguous_system(
    mut commands: Commands,
    needs_rebuild: Query<(Entity, &ChunkPosition), With<ChunkNeedsLightRebuild>>,
    all_chunks: Query<(Entity, &ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
    dimension: Option<Single<&Dimension, With<Active>>>,
    metadata: Res<WorldMetadata>,
    mut stats: ResMut<LightRebuildBenchStats>,
) {
    if needs_rebuild.is_empty() {
        return;
    }

    let mut dirty_positions = Vec::with_capacity(needs_rebuild.iter().len());
    for (_, positions) in needs_rebuild
        .contiguous_iter()
        .expect("chunk light rebuild dirty query should stay dense")
    {
        dirty_positions.extend(positions.iter().map(|pos| pos.0));
    }

    let fallback_loaded_chunks;
    let loaded_chunks = if let Some(dimension) = dimension.as_ref() {
        &dimension.chunks
    } else {
        fallback_loaded_chunks = build_loaded_chunk_map_contiguous(&all_chunks);
        &fallback_loaded_chunks
    };

    rebuild_chunk_light_rest(
        &mut commands,
        &needs_rebuild,
        &all_chunks,
        loaded_chunks,
        &dirty_positions,
        metadata.height_chunks as i32,
        &mut stats,
    );
}

fn build_loaded_chunk_map_contiguous(
    all_chunks: &Query<(Entity, &ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
) -> HashMap<IVec3, Entity> {
    let mut loaded_chunks = HashMap::with_capacity(all_chunks.iter().len());
    for (entities, positions, _, _, _) in all_chunks
        .contiguous_iter()
        .expect("loaded chunk fallback map query should stay dense")
    {
        loaded_chunks.extend(
            positions
                .iter()
                .zip(entities.iter().copied())
                .map(|(pos, entity)| (pos.0, entity)),
        );
    }

    loaded_chunks
}

fn rebuild_chunk_light_rest(
    commands: &mut Commands,
    needs_rebuild: &Query<(Entity, &ChunkPosition), With<ChunkNeedsLightRebuild>>,
    all_chunks: &Query<(Entity, &ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
    loaded_chunks: &HashMap<IVec3, Entity>,
    dirty_positions: &[IVec3],
    height_chunks: i32,
    stats: &mut LightRebuildBenchStats,
) {
    let targets = light_rebuild_targets(dirty_positions, loaded_chunks, height_chunks);

    if targets.is_empty() {
        for (entity, _) in needs_rebuild.iter() {
            commands.entity(entity).remove::<ChunkNeedsLightRebuild>();
        }
        stats.dirty_positions = dirty_positions.len();
        stats.targets = 0;
        stats.changed_lights = 0;
        return;
    }

    let mut light_context = targets.clone();
    for &pos in &targets {
        for offset in chunk_neighbor_offsets() {
            light_context.insert(pos + offset);
        }
    }

    let chunk_map: HashMap<IVec3, (Entity, &Chunk, &ChunkLight, &ChunkHeightmap)> = light_context
        .iter()
        .filter_map(|pos| {
            let entity = *loaded_chunks.get(pos)?;
            let Ok((entity, actual_pos, chunk, light, heightmap)) = all_chunks.get(entity) else {
                return None;
            };

            (actual_pos.0 == *pos).then_some((*pos, (entity, chunk, light, heightmap)))
        })
        .collect();

    let chunks = targets
        .iter()
        .filter_map(|pos| chunk_map.get(pos).map(|(_, chunk, _, _)| (*pos, *chunk)))
        .collect::<HashMap<_, _>>();
    let mut lights = light_context
        .iter()
        .filter_map(|pos| {
            chunk_map
                .get(pos)
                .map(|(_, _, light, _)| (*pos, (*light).clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut heightmaps = targets
        .iter()
        .filter_map(|pos| {
            chunk_map
                .get(pos)
                .map(|(_, _, _, heightmap)| (*pos, **heightmap))
        })
        .collect::<HashMap<_, _>>();

    compute_light_region(
        &chunks,
        &mut lights,
        &mut heightmaps,
        &targets,
        height_chunks,
    );

    let mut changed_light_positions = HashSet::new();
    for &pos in &targets {
        let Some((entity, _, old_light, old_heightmap)) = chunk_map.get(&pos) else {
            continue;
        };
        let new_light = lights.get(&pos).cloned().unwrap_or_default();
        let new_heightmap = heightmaps.get(&pos).copied().unwrap_or_default();
        let light_changed = new_light != **old_light;
        let heightmap_changed = new_heightmap != **old_heightmap;

        if light_changed {
            commands
                .entity(*entity)
                .insert((new_light, ChunkNeedsRenderLightUpload));
            changed_light_positions.insert(pos);
        }
        if heightmap_changed {
            commands.entity(*entity).insert(new_heightmap);
        }
        commands.entity(*entity).remove::<ChunkNeedsLightRebuild>();
    }

    for pos in &changed_light_positions {
        for offset in chunk_neighbor_offsets() {
            let Some(entity) = loaded_chunks.get(&(*pos + offset)) else {
                continue;
            };
            commands.entity(*entity).insert(ChunkNeedsRenderLightUpload);
        }
    }

    stats.dirty_positions = dirty_positions.len();
    stats.targets = targets.len();
    stats.changed_lights = changed_light_positions.len();
}

fn step_active_fluids(
    commands: &mut Commands,
    budget: usize,
    param_set: &mut ParamSet<(
        Query<
            (Entity, &ChunkPosition, &mut Chunk, &mut ChunkContentCounts),
            With<ChunkNeedsFluidStep>,
        >,
        Query<(Entity, &ChunkPosition, &mut Chunk, &mut ChunkContentCounts)>,
    )>,
) -> (usize, Vec<BoundaryFlow>) {
    let mut boundary_flows = Vec::new();
    let mut stepped = 0;
    for (entity, pos, mut chunk, mut counts) in &mut param_set.p0() {
        if stepped >= budget {
            break;
        }
        stepped += 1;

        let profile = FluidProfile::WATER;
        let result = chunk.step_fluids(&profile);
        if !result.changed {
            commands.entity(entity).remove::<ChunkNeedsFluidStep>();
            continue;
        }

        *counts = chunk.compute_content_counts();
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((ChunkNeedsSave, ChunkNeedsMeshRebuild));
        if !chunk.has_fluids() {
            entity_commands.remove::<ChunkNeedsFluidStep>();
        }

        if result.boundary_changed {
            boundary_flows.extend(collect_boundary_flows(&chunk, pos.0, &profile));
        }
    }

    (stepped, boundary_flows)
}

fn apply_boundary_flow(
    commands: &mut Commands,
    entity: Entity,
    chunk: &mut Chunk,
    counts: &mut ChunkContentCounts,
    flow: BoundaryFlow,
) {
    let cell = chunk.cell_xyz(flow.x, flow.y, flow.z);
    if !cell.is_block()
        && cell
            .as_fluid()
            .is_none_or(|f| !f.is_source() && flow.fluid.level() > f.level())
    {
        chunk.set_cell_xyz(flow.x, flow.y, flow.z, ChunkCell::fluid(flow.fluid));
        *counts = chunk.compute_content_counts();
        commands.entity(entity).insert((
            ChunkNeedsFluidStep,
            ChunkNeedsSave,
            ChunkNeedsMeshRebuild,
        ));
    }
}

fn fluid_boundary_scan_system(
    mut commands: Commands,
    budget: Res<BenchFluidStepBudget>,
    mut stats: ResMut<FluidBenchStats>,
    mut param_set: ParamSet<(
        Query<
            (Entity, &ChunkPosition, &mut Chunk, &mut ChunkContentCounts),
            With<ChunkNeedsFluidStep>,
        >,
        Query<(Entity, &ChunkPosition, &mut Chunk, &mut ChunkContentCounts)>,
    )>,
) {
    let (stepped, boundary_flows) = step_active_fluids(&mut commands, budget.0, &mut param_set);
    let boundary_flow_count = boundary_flows.len();

    for flow in boundary_flows {
        for (entity, cpos, mut chunk, mut counts) in &mut param_set.p1() {
            if cpos.0 == flow.target_pos {
                apply_boundary_flow(&mut commands, entity, &mut chunk, &mut counts, flow);
                break;
            }
        }
    }

    stats.stepped_chunks = stepped;
    stats.boundary_flows = boundary_flow_count;
}

fn fluid_boundary_lookup_system(
    mut commands: Commands,
    budget: Res<BenchFluidStepBudget>,
    mut stats: ResMut<FluidBenchStats>,
    mut param_set: ParamSet<(
        Query<
            (Entity, &ChunkPosition, &mut Chunk, &mut ChunkContentCounts),
            With<ChunkNeedsFluidStep>,
        >,
        Query<(Entity, &ChunkPosition, &mut Chunk, &mut ChunkContentCounts)>,
    )>,
) {
    let (stepped, boundary_flows) = step_active_fluids(&mut commands, budget.0, &mut param_set);
    let boundary_flow_count = boundary_flows.len();
    let chunks_by_pos = {
        let mut chunks_by_pos = HashMap::with_capacity(param_set.p1().iter().len());
        chunks_by_pos.extend(
            param_set
                .p1()
                .iter()
                .map(|(entity, pos, _, _)| (pos.0, entity)),
        );
        chunks_by_pos
    };

    for flow in boundary_flows {
        let Some(entity) = chunks_by_pos.get(&flow.target_pos).copied() else {
            continue;
        };
        {
            let mut chunks_q = param_set.p1();
            let Ok((entity, _, mut chunk, mut counts)) = chunks_q.get_mut(entity) else {
                continue;
            };
            apply_boundary_flow(&mut commands, entity, &mut chunk, &mut counts, flow);
        };
    }

    stats.stepped_chunks = stepped;
    stats.boundary_flows = boundary_flow_count;
}

fn bench_light_upload_dirty_loop(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecs_system_light_upload_dirty_loop");
    for dirty_chunks in [1usize, 4, 16, 64, 256] {
        group.throughput(Throughput::Elements(dirty_chunks as u64));

        group.bench_function(BenchmarkId::new("iter", dirty_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_light_upload_world(dirty_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(light_upload_dirty_iter_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<LightUploadBenchStats>().child_updates)
                },
                BatchSize::SmallInput,
            )
        });

        group.bench_function(BenchmarkId::new("contiguous", dirty_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_light_upload_world(dirty_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(light_upload_dirty_contiguous_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<LightUploadBenchStats>().child_updates)
                },
                BatchSize::SmallInput,
            )
        });
    }

    group.finish();
}

fn bench_mesh_rebuild_maps(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecs_system_mesh_rebuild_maps");
    for dirty_chunks in [1usize, 4, 16, 64] {
        group.throughput(Throughput::Elements(dirty_chunks as u64));

        group.bench_function(BenchmarkId::new("iter", dirty_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_mesh_rebuild_world(dirty_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(mesh_rebuild_iter_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<MeshRebuildBenchStats>().faces)
                },
                BatchSize::SmallInput,
            )
        });

        group.bench_function(BenchmarkId::new("contiguous", dirty_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_mesh_rebuild_world(dirty_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(mesh_rebuild_contiguous_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<MeshRebuildBenchStats>().faces)
                },
                BatchSize::SmallInput,
            )
        });
    }

    group.finish();
}

fn bench_light_upload_map_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecs_system_light_upload_map_build");
    for dirty_chunks in [1usize, 4, 16, 64, 256] {
        group.throughput(Throughput::Elements(dirty_chunks as u64));

        group.bench_function(BenchmarkId::new("iter", dirty_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_light_upload_world(dirty_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(light_upload_map_iter_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<LightUploadBenchStats>().child_updates)
                },
                BatchSize::SmallInput,
            )
        });

        group.bench_function(BenchmarkId::new("contiguous", dirty_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_light_upload_world(dirty_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(light_upload_map_contiguous_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<LightUploadBenchStats>().child_updates)
                },
                BatchSize::SmallInput,
            )
        });
    }

    group.finish();
}

fn bench_light_rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecs_system_light_rebuild");
    for with_active_dimension in [true, false] {
        let scenario = if with_active_dimension {
            "active_dimension"
        } else {
            "fallback_map"
        };

        for dirty_columns in [1usize, 4, 16, 64] {
            group.throughput(Throughput::Elements(
                (dirty_columns * LIGHT_REBUILD_HEIGHT as usize) as u64,
            ));

            group.bench_function(
                BenchmarkId::new(format!("{scenario}/iter"), dirty_columns),
                |b| {
                    b.iter_batched(
                        || {
                            let mut world =
                                build_light_rebuild_world(dirty_columns, with_active_dimension);
                            let mut schedule = Schedule::default();
                            schedule.add_systems(rebuild_chunk_light_iter_system);
                            schedule.initialize(&mut world).unwrap();
                            (world, schedule)
                        },
                        |(mut world, mut schedule)| {
                            schedule.run(&mut world);
                            black_box(world.resource::<LightRebuildBenchStats>().targets)
                        },
                        BatchSize::SmallInput,
                    )
                },
            );

            group.bench_function(
                BenchmarkId::new(format!("{scenario}/contiguous"), dirty_columns),
                |b| {
                    b.iter_batched(
                        || {
                            let mut world =
                                build_light_rebuild_world(dirty_columns, with_active_dimension);
                            let mut schedule = Schedule::default();
                            schedule.add_systems(rebuild_chunk_light_contiguous_system);
                            schedule.initialize(&mut world).unwrap();
                            (world, schedule)
                        },
                        |(mut world, mut schedule)| {
                            schedule.run(&mut world);
                            black_box(world.resource::<LightRebuildBenchStats>().targets)
                        },
                        BatchSize::SmallInput,
                    )
                },
            );
        }
    }

    group.finish();
}

fn bench_fluid_boundary_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecs_system_fluid_boundary_lookup");
    for active_chunks in [1usize, 4, 16, 64] {
        group.throughput(Throughput::Elements(
            (active_chunks * (CHUNK_SIZE - 2)) as u64,
        ));

        group.bench_function(BenchmarkId::new("scan", active_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_fluid_world(active_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(fluid_boundary_scan_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<FluidBenchStats>().boundary_flows)
                },
                BatchSize::SmallInput,
            )
        });

        group.bench_function(BenchmarkId::new("lookup", active_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_fluid_world(active_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(fluid_boundary_lookup_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<FluidBenchStats>().boundary_flows)
                },
                BatchSize::SmallInput,
            )
        });
    }

    group.finish();
}

fn bench_collider_rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("ecs_system_collider_rebuild");
    for dirty_chunks in [1usize, 4, 16, 64] {
        group.throughput(Throughput::Elements(dirty_chunks as u64));

        group.bench_function(BenchmarkId::new("iter", dirty_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_collider_world(dirty_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(collider_rebuild_iter_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<ColliderBenchStats>().voxels)
                },
                BatchSize::SmallInput,
            )
        });

        group.bench_function(BenchmarkId::new("contiguous", dirty_chunks), |b| {
            b.iter_batched(
                || {
                    let mut world = build_collider_world(dirty_chunks);
                    let mut schedule = Schedule::default();
                    schedule.add_systems(collider_rebuild_contiguous_system);
                    schedule.initialize(&mut world).unwrap();
                    (world, schedule)
                },
                |(mut world, mut schedule)| {
                    schedule.run(&mut world);
                    black_box(world.resource::<ColliderBenchStats>().voxels)
                },
                BatchSize::SmallInput,
            )
        });
    }

    group.finish();
}

criterion_group! {
    name = ecs_query_benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(10);
    targets = bench_mesh_rebuild_maps, bench_light_upload_map_build, bench_light_upload_dirty_loop,
        bench_light_rebuild, bench_fluid_boundary_lookup, bench_collider_rebuild
}
criterion_main!(ecs_query_benches);
