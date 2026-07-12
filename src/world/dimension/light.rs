use bevy::{platform::collections::HashSet, prelude::*};

use crate::world::{
    chunk::{
        Chunk, ChunkColumn, ChunkHeightmap, ChunkInvalidationPlan, ChunkLight,
        ChunkNeedsLightRebuild, ChunkPerfCounters, ChunkPos, ChunkPosition,
        light::ChunkLightRegion,
    },
    generation::WorldMetadata,
};

use super::{Active, Dimension, apply_chunk_invalidations};

pub(crate) fn rebuild_chunk_light(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    needs_rebuild: Query<(Entity, &ChunkPosition), With<ChunkNeedsLightRebuild>>,
    all_chunks: Query<(&ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
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
                .then_some((entity, registered_position))
        })
        .collect::<Vec<_>>();
    let dirty_positions = dirty_chunks
        .iter()
        .map(|(_, position)| *position)
        .collect::<Vec<_>>();

    let height_chunks = metadata.height_chunks();
    let mut region = ChunkLightRegion::new(height_chunks);
    let targets = light_rebuild_targets(&dirty_positions, dimension, height_chunks);
    if let Some(perf) = perf.as_deref_mut() {
        perf.light_rebuild_targets += targets.len();
    }

    if targets.is_empty() {
        for (entity, _) in dirty_chunks {
            commands.entity(entity).remove::<ChunkNeedsLightRebuild>();
        }
        return;
    }

    for &position in &targets {
        let entity = dimension
            .chunk_entity(position)
            .expect("light rebuild target must belong to the active dimension");
        let (actual_position, chunk, light, heightmap) = all_chunks
            .get(entity)
            .expect("registered light rebuild target must have chunk lighting components");
        assert_eq!(
            actual_position.chunk_pos(),
            position,
            "dimension registry and ChunkPosition must agree"
        );
        region.insert_target(position, chunk, light, heightmap);
    }

    for position in region.required_boundary_positions() {
        let Some(entity) = dimension.chunk_entity(position) else {
            continue;
        };
        let (actual_position, _, light, _) = all_chunks
            .get(entity)
            .expect("registered light boundary must have chunk lighting components");
        assert_eq!(
            actual_position.chunk_pos(),
            position,
            "dimension registry and ChunkPosition must agree"
        );
        region.insert_boundary_light(position, light);
    }

    let mut invalidations = ChunkInvalidationPlan::new();
    for rebuilt in region.rebuild() {
        let entity = dimension
            .chunk_entity(rebuilt.position)
            .expect("rebuilt light target must remain in the active dimension");
        let light_changed = rebuilt.light_changed();
        let heightmap_changed = rebuilt.heightmap_changed();

        if light_changed {
            commands.entity(entity).insert(rebuilt.light);
            invalidations.record_render_light_changed(rebuilt.position);
        }
        if heightmap_changed {
            commands.entity(entity).insert(rebuilt.heightmap);
        }
        commands.entity(entity).remove::<ChunkNeedsLightRebuild>();
    }

    apply_chunk_invalidations(&mut commands, dimension, &invalidations);
}

fn light_rebuild_targets(
    dirty_positions: &[ChunkPos],
    dimension: &Dimension,
    height_chunks: usize,
) -> HashSet<ChunkPos> {
    let columns = dirty_positions
        .iter()
        .copied()
        .map(ChunkColumn::from)
        .collect::<HashSet<_>>();

    let mut targets = HashSet::new();
    for column in columns {
        for y in 0..height_chunks as i32 {
            let position = column.chunk(y);
            if dimension.contains_chunk(position) {
                targets.insert(position);
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
            ChunkNeedsRenderLightUpload, LocalBlockPos,
        },
    };

    #[derive(Resource)]
    struct TestDimension(Entity);

    fn app_with_light_system(height_chunks: usize) -> App {
        let metadata = WorldMetadata::with_seed(1)
            .with_height_chunks(height_chunks)
            .unwrap();
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
                ChunkPosition::from(lower_pos),
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
                ChunkPosition::from(upper_pos),
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
                .sky_light(LocalBlockPos::new(8, 1, 8)),
            15
        );
        assert_eq!(
            world
                .get::<ChunkLight>(lower_entity)
                .unwrap()
                .sky_light(LocalBlockPos::new(8, 15, 8)),
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
                ChunkPosition::from(IVec3::ZERO),
                Chunk::default(),
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();
        let neighbor_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(IVec3::X),
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
                .sky_light(LocalBlockPos::new(8, 8, 8)),
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
                ChunkPosition::from(left_pos),
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
                ChunkPosition::from(right_pos),
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
                .block_light(LocalBlockPos::new(15, 8, 8)),
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
                .block_light(LocalBlockPos::new(0, 8, 8)),
            0
        );
        assert_eq!(
            app.world()
                .get::<ChunkLight>(left_entity)
                .unwrap()
                .block_light(LocalBlockPos::new(15, 8, 8)),
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
                ChunkPosition::from(foreign_position),
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
        let mut dimension = Dimension::default();
        for x in -2..=2 {
            for z in -2..=2 {
                for y in 0..2 {
                    dimension.register_chunk(ChunkPos::new(x, y, z), Entity::PLACEHOLDER);
                }
            }
        }

        let dirty_positions = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| ChunkPos::new(x, 0, z)))
            .collect::<Vec<_>>();
        let targets = light_rebuild_targets(&dirty_positions, &dimension, 2);

        assert_eq!(targets.len(), 18);
        for x in -1..=1 {
            for z in -1..=1 {
                for y in 0..2 {
                    assert!(targets.contains(&ChunkPos::new(x, y, z)));
                }
            }
        }
        assert!(!targets.contains(&ChunkPos::new(-2, 0, 0)));
        assert!(!targets.contains(&ChunkPos::new(2, 0, 0)));
    }

    #[test]
    fn light_rebuild_targets_keep_loaded_gaps_and_world_height_bounds() {
        let column = ChunkColumn::new(-8, 13);
        let mut dimension = Dimension::default();
        for y in [0, 2, 4] {
            dimension.register_chunk(column.chunk(y), Entity::PLACEHOLDER);
        }

        let targets = light_rebuild_targets(
            &[column.chunk(0), column.chunk(2), column.chunk(2)],
            &dimension,
            4,
        );

        assert_eq!(targets, HashSet::from([column.chunk(0), column.chunk(2)]));
    }
}
