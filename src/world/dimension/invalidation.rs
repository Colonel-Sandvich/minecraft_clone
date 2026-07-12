use bevy::{platform::collections::HashSet, prelude::*};

use crate::world::chunk::{
    ChunkColumn, ChunkInvalidationPlan, ChunkNeedsColliderRebuild, ChunkNeedsFluidStep,
    ChunkNeedsLightRebuild, ChunkNeedsMeshRebuild, ChunkNeedsRenderLightUpload, ChunkNeedsSave,
};

use super::Dimension;

/// Applies coalesced chunk work only to entities owned by `dimension`.
pub(crate) fn apply_chunk_invalidations(
    commands: &mut Commands,
    dimension: &mut Dimension,
    plan: &ChunkInvalidationPlan,
) {
    for column in plan.light_columns() {
        dimension.mark_column_light_pending(column);
    }

    for (position, effects) in plan.chunks() {
        let Some(entity) = dimension.published_chunk_entity(position) else {
            continue;
        };
        let mut entity = commands.entity(entity);

        if effects.needs_save() {
            entity.insert(ChunkNeedsSave);
        }
        if effects.needs_mesh_rebuild() {
            entity.insert(ChunkNeedsMeshRebuild);
        }
        if effects.needs_collider_rebuild() {
            entity.insert(ChunkNeedsColliderRebuild);
        }
        if effects.needs_light_rebuild() {
            entity.insert(ChunkNeedsLightRebuild);
        }
        if effects.needs_fluid_step() {
            entity.insert(ChunkNeedsFluidStep);
        }
        if effects.needs_render_light_upload() {
            entity.insert(ChunkNeedsRenderLightUpload);
        }
    }

    let dirty_columns = plan.light_columns().collect::<HashSet<_>>();
    if dirty_columns.is_empty() {
        return;
    }

    for (position, entity) in dimension.iter_published_chunks() {
        if dirty_columns.contains(&ChunkColumn::from(position)) {
            commands.entity(entity).insert(ChunkNeedsLightRebuild);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block::BlockType,
        world::{
            chunk::{CellDelta, ChunkCell, ChunkPos, LocalBlockPos},
            dimension::Active,
        },
    };

    #[derive(Resource)]
    struct TestPlan(ChunkInvalidationPlan);

    fn apply_test_plan(
        mut commands: Commands,
        dimension: Single<&mut Dimension, With<Active>>,
        plan: Res<TestPlan>,
    ) {
        let mut dimension = dimension.into_inner();
        apply_chunk_invalidations(&mut commands, &mut dimension, &plan.0);
    }

    #[test]
    fn applying_a_plan_is_scoped_to_one_dimension() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, apply_test_plan);

        let origin = ChunkPos::new(4, 2, -7);
        let own = app.world_mut().spawn_empty().id();
        let upper = app.world_mut().spawn_empty().id();
        let adjacent_column = app.world_mut().spawn_empty().id();
        let foreign_same_position = app.world_mut().spawn_empty().id();

        let mut dimension = Dimension::default();
        dimension.register_published_chunk(origin, own);
        dimension.register_published_chunk(origin.offset(IVec3::Y), upper);
        dimension.register_published_chunk(origin.offset(IVec3::X), adjacent_column);
        app.world_mut().spawn((dimension, Active));

        let mut other_dimension = Dimension::default();
        other_dimension.register_published_chunk(origin, foreign_same_position);
        app.world_mut().spawn(other_dimension);

        let mut plan = ChunkInvalidationPlan::new();
        plan.record_cell_delta(
            origin,
            LocalBlockPos::new(1, 2, 3),
            CellDelta {
                old: ChunkCell::EMPTY,
                new: BlockType::Stone.into(),
            },
        );
        app.insert_resource(TestPlan(plan));
        app.update();

        let world = app.world();
        assert!(world.get::<ChunkNeedsSave>(own).is_some());
        assert!(world.get::<ChunkNeedsMeshRebuild>(own).is_some());
        assert!(world.get::<ChunkNeedsColliderRebuild>(own).is_some());
        assert!(world.get::<ChunkNeedsLightRebuild>(own).is_some());
        assert!(world.get::<ChunkNeedsFluidStep>(own).is_some());

        for entity in [upper, adjacent_column] {
            assert!(world.get::<ChunkNeedsLightRebuild>(entity).is_some());
            assert!(world.get::<ChunkNeedsSave>(entity).is_none());
            assert!(world.get::<ChunkNeedsMeshRebuild>(entity).is_none());
        }

        assert!(world.get::<ChunkNeedsSave>(foreign_same_position).is_none());
        assert!(
            world
                .get::<ChunkNeedsLightRebuild>(foreign_same_position)
                .is_none()
        );
    }
}
