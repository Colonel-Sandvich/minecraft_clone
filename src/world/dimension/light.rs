use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
};

use crate::world::{
    chunk::{
        Chunk, ChunkHeightmap, ChunkInvalidationPlan, ChunkLight, ChunkNeedsLightRebuild,
        ChunkPerfCounters, ChunkPos, ChunkPosition, chunk_neighbor_offsets,
        light::compute_light_region,
    },
    generation::WorldMetadata,
};

use super::{Active, Dimension, apply_chunk_invalidations};

pub(crate) fn rebuild_chunk_light(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    needs_rebuild: Query<(Entity, &ChunkPosition), With<ChunkNeedsLightRebuild>>,
    all_chunks: Query<(Entity, &ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
    dimension: Single<&Dimension, With<Active>>,
    metadata: Res<WorldMetadata>,
) {
    if needs_rebuild.is_empty() {
        return;
    }

    let dimension = dimension.into_inner();
    let dirty_chunks = dimension
        .iter_chunks()
        .filter_map(|(registered_position, entity)| {
            let (_, actual_position) = needs_rebuild.get(entity).ok()?;
            (actual_position.chunk_pos() == registered_position)
                .then_some((entity, registered_position.as_ivec3()))
        })
        .collect::<Vec<_>>();
    let dirty_positions = dirty_chunks
        .iter()
        .map(|(_, position)| *position)
        .collect::<Vec<_>>();
    let loaded_chunks = dimension.chunk_entities();

    let targets = light_rebuild_targets(
        &dirty_positions,
        loaded_chunks,
        metadata.height_chunks as i32,
    );
    if let Some(perf) = perf.as_deref_mut() {
        perf.light_rebuild_targets += targets.len();
    }

    if targets.is_empty() {
        for (entity, _) in dirty_chunks {
            commands.entity(entity).remove::<ChunkNeedsLightRebuild>();
        }
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
        metadata.height_chunks as i32,
    );

    let mut invalidations = ChunkInvalidationPlan::new();
    for &pos in &targets {
        let Some((entity, _, old_light, old_heightmap)) = chunk_map.get(&pos) else {
            continue;
        };
        let new_light = lights.get(&pos).cloned().unwrap_or_default();
        let new_heightmap = heightmaps.get(&pos).copied().unwrap_or_default();
        let light_changed = new_light != **old_light;
        let heightmap_changed = new_heightmap != **old_heightmap;

        if light_changed {
            commands.entity(*entity).insert(new_light);
            invalidations.record_render_light_changed(ChunkPos::from_ivec3(pos));
        }
        if heightmap_changed {
            commands.entity(*entity).insert(new_heightmap);
        }
        commands.entity(*entity).remove::<ChunkNeedsLightRebuild>();
    }

    apply_chunk_invalidations(&mut commands, dimension, &invalidations);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block::BlockType,
        world::chunk::{
            CHUNK_SIZE, ChunkCell, ChunkNeedsLightRebuild, ChunkNeedsMeshRebuild,
            ChunkNeedsRenderLightUpload,
        },
    };

    #[derive(Resource)]
    struct TestDimension(Entity);

    fn app_with_light_system(height_chunks: usize) -> App {
        let mut metadata = WorldMetadata::with_seed(1);
        metadata.height_chunks = height_chunks;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata)
            .add_systems(Update, rebuild_chunk_light);
        let dimension = app.world_mut().spawn((Dimension::default(), Active)).id();
        app.insert_resource(TestDimension(dimension));
        app
    }

    fn register_chunk(app: &mut App, position: IVec3, entity: Entity) {
        let dimension = app.world().resource::<TestDimension>().0;
        app.world_mut()
            .entity_mut(dimension)
            .get_mut::<Dimension>()
            .unwrap()
            .register_chunk(position, entity);
    }

    fn solid_chunk(block: BlockType) -> Chunk {
        Chunk::filled(block.into())
    }

    fn heightmap_with(value: u8) -> ChunkHeightmap {
        ChunkHeightmap {
            heights: [[value; CHUNK_SIZE]; CHUNK_SIZE],
        }
    }

    #[test]
    fn rebuild_light_resolves_vertical_sky_occlusion_across_loaded_column() {
        let mut app = app_with_light_system(2);
        let lower_pos = ivec3(0, 0, 0);
        let upper_pos = ivec3(0, 1, 0);
        let lower_entity = app
            .world_mut()
            .spawn((
                ChunkPosition(lower_pos),
                Chunk::default(),
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();

        let mut upper = Chunk::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                upper.set_cell_xyz(x, 0, z, BlockType::Stone.into());
            }
        }
        let upper_entity = app
            .world_mut()
            .spawn((
                ChunkPosition(upper_pos),
                upper,
                ChunkLight::default(),
                ChunkHeightmap::default(),
            ))
            .id();
        register_chunk(&mut app, lower_pos, lower_entity);
        register_chunk(&mut app, upper_pos, upper_entity);

        app.update();

        let world = app.world();
        assert_eq!(
            world
                .get::<ChunkLight>(upper_entity)
                .unwrap()
                .sky_light(uvec3(8, 1, 8)),
            15
        );
        assert_eq!(
            world
                .get::<ChunkLight>(lower_entity)
                .unwrap()
                .sky_light(uvec3(8, 15, 8)),
            0
        );
        assert!(world.get::<ChunkNeedsLightRebuild>(lower_entity).is_none());
    }

    #[test]
    fn changed_light_marks_padded_neighbor_light_upload_dirty() {
        let mut app = app_with_light_system(1);
        let center_entity = app
            .world_mut()
            .spawn((
                ChunkPosition(IVec3::ZERO),
                Chunk::default(),
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();
        let neighbor_entity = app
            .world_mut()
            .spawn((
                ChunkPosition(IVec3::X),
                solid_chunk(BlockType::Stone),
                ChunkLight::default(),
                heightmap_with(15),
            ))
            .id();
        register_chunk(&mut app, IVec3::ZERO, center_entity);
        register_chunk(&mut app, IVec3::X, neighbor_entity);

        app.update();

        let world = app.world();
        assert_eq!(
            world
                .get::<ChunkLight>(center_entity)
                .unwrap()
                .sky_light(uvec3(8, 8, 8)),
            15
        );
        assert!(
            world
                .get::<ChunkNeedsRenderLightUpload>(neighbor_entity)
                .is_some()
        );
        assert!(
            world
                .get::<ChunkNeedsRenderLightUpload>(center_entity)
                .is_some()
        );
        assert!(world.get::<ChunkNeedsMeshRebuild>(center_entity).is_none());
        assert!(
            world
                .get::<ChunkNeedsMeshRebuild>(neighbor_entity)
                .is_none()
        );
    }

    #[test]
    fn rebuild_light_clears_neighbor_block_light_after_emitter_removed() {
        let mut app = app_with_light_system(1);
        let left_pos = IVec3::ZERO;
        let right_pos = IVec3::X;
        let left_entity = app
            .world_mut()
            .spawn((
                ChunkPosition(left_pos),
                Chunk::default(),
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();

        let mut right_chunk = Chunk::default();
        right_chunk.set_cell_xyz(0, 8, 8, BlockType::Glowstone.into());
        let right_entity = app
            .world_mut()
            .spawn((
                ChunkPosition(right_pos),
                right_chunk,
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();
        register_chunk(&mut app, left_pos, left_entity);
        register_chunk(&mut app, right_pos, right_entity);

        app.update();

        assert_eq!(
            app.world()
                .get::<ChunkLight>(left_entity)
                .unwrap()
                .block_light(uvec3(15, 8, 8)),
            14
        );

        app.world_mut()
            .entity_mut(right_entity)
            .get_mut::<Chunk>()
            .unwrap()
            .set_cell_xyz(0, 8, 8, ChunkCell::EMPTY);
        app.world_mut()
            .entity_mut(left_entity)
            .insert(ChunkNeedsLightRebuild);
        app.world_mut()
            .entity_mut(right_entity)
            .insert(ChunkNeedsLightRebuild);

        app.update();

        assert_eq!(
            app.world()
                .get::<ChunkLight>(right_entity)
                .unwrap()
                .block_light(uvec3(0, 8, 8)),
            0
        );
        assert_eq!(
            app.world()
                .get::<ChunkLight>(left_entity)
                .unwrap()
                .block_light(uvec3(15, 8, 8)),
            0
        );
    }

    #[test]
    fn light_rebuild_does_not_consume_markers_from_another_dimension() {
        let mut app = app_with_light_system(1);
        let foreign_position = ivec3(12, 0, -8);
        let foreign_entity = app
            .world_mut()
            .spawn((
                ChunkPosition(foreign_position),
                Chunk::default(),
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();
        let mut foreign_dimension = Dimension::default();
        foreign_dimension.register_chunk(foreign_position, foreign_entity);
        app.world_mut().spawn(foreign_dimension);

        app.update();

        assert!(
            app.world()
                .get::<ChunkNeedsLightRebuild>(foreign_entity)
                .is_some()
        );
    }

    #[test]
    fn light_rebuild_targets_do_not_expand_dirty_column_set() {
        let mut loaded_chunks = HashMap::new();
        for x in -2..=2 {
            for z in -2..=2 {
                for y in 0..2 {
                    loaded_chunks.insert(ivec3(x, y, z), Entity::PLACEHOLDER);
                }
            }
        }

        let dirty_positions = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| ivec3(x, 0, z)))
            .collect::<Vec<_>>();
        let targets = light_rebuild_targets(&dirty_positions, &loaded_chunks, 2);

        assert_eq!(targets.len(), 18);
        for x in -1..=1 {
            for z in -1..=1 {
                for y in 0..2 {
                    assert!(targets.contains(&ivec3(x, y, z)));
                }
            }
        }
        assert!(!targets.contains(&ivec3(-2, 0, 0)));
        assert!(!targets.contains(&ivec3(2, 0, 0)));
    }
}
