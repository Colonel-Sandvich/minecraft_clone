use std::collections::HashSet;

use super::{CHUNK_SIZE, Chunk, ChunkColumn, ChunkContentCounts, ChunkPos, ChunkPosition};
use crate::world::{
    WORLD_COLLISION_LAYERS,
    dimension::{Active, Dimension},
};
use avian3d::prelude::*;
use bevy::prelude::*;

const COLLIDER_REBUILD_WAKE_MARGIN: f32 = 0.5;

pub struct ChunkColliderPlugin;

impl Plugin for ChunkColliderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedPreUpdate, rebuild_chunk_colliders);
    }
}

fn insert_one(
    commands: &mut Commands,
    chunk: &Chunk,
    chunk_entity: Entity,
    meta: &ChunkContentCounts,
) {
    if meta.solid == 0 {
        return;
    }

    let mut voxels = Vec::with_capacity(meta.solid as usize);
    for (cell, local) in chunk.iter() {
        if !cell.is_solid() {
            continue;
        }

        voxels.push(local.as_uvec3().as_ivec3());
    }

    if voxels.is_empty() {
        return;
    }

    commands.spawn((
        ChildOf(chunk_entity),
        Collider::voxels(Vec3::ONE, &voxels),
        WORLD_COLLISION_LAYERS,
        RigidBody::Static,
    ));
}

pub(crate) fn column_colliders_ready(
    dimension: &Dimension,
    column: ChunkColumn,
    chunks: &Query<(&ChunkPosition, &ChunkContentCounts, Option<&Children>)>,
    colliders: &Query<(), With<Collider>>,
) -> bool {
    let Some(column_chunks) = dimension.complete_loaded_column(column) else {
        return false;
    };

    for (position, entity) in column_chunks {
        if dimension.published_chunk_entity(position) != Some(entity)
            || dimension.has_pending_collider_rebuild(position)
        {
            return false;
        }
        let Ok((actual_position, contents, children)) = chunks.get(entity) else {
            return false;
        };
        if actual_position.chunk_pos() != position {
            return false;
        }
        let collider_count = children.map_or(0, |children| colliders.iter_many(children).count());
        if (contents.solid == 0 && collider_count != 0)
            || (contents.solid > 0 && collider_count == 0)
        {
            return false;
        }
    }

    true
}

pub(crate) fn discard_chunk_collider_work(dimension: Option<Single<&mut Dimension, With<Active>>>) {
    let Some(mut dimension) = dimension else {
        return;
    };
    dimension.take_collider_rebuilds();
}

fn rebuild_chunk_colliders(
    mut commands: Commands,
    chunks_q: Query<(
        Entity,
        &ChunkPosition,
        &Chunk,
        &ChunkContentCounts,
        Option<&Children>,
    )>,
    collider_q: Query<Entity, With<Collider>>,
    sleeping_bodies: Query<(Entity, &ColliderAabb), (With<RigidBody>, With<Sleeping>)>,
    dimension: Option<Single<&mut Dimension, With<Active>>>,
) {
    let Some(mut dimension) = dimension else {
        return;
    };
    let pending = dimension.take_collider_rebuilds();
    if pending.is_empty() {
        return;
    }

    let mut bodies_to_wake = HashSet::<Entity>::new();

    for work in pending {
        let position = work.position();
        let expected_entity = work.expected_entity();
        if dimension.published_chunk_entity(position) != Some(expected_entity) {
            continue;
        }
        let Ok((entity, actual_position, chunk, contents, children)) =
            chunks_q.get(expected_entity)
        else {
            dimension.requeue_collider_rebuild(work);
            continue;
        };
        if entity != expected_entity || actual_position.chunk_pos() != position {
            dimension.requeue_collider_rebuild(work);
            continue;
        }

        let affected_aabb = chunk_collider_rebuild_aabb(position);
        for (body, body_aabb) in &sleeping_bodies {
            if body_aabb.intersects(&affected_aabb) {
                bodies_to_wake.insert(body);
            }
        }

        if let Some(children) = children {
            for collider_entity in collider_q.iter_many(children) {
                if let Ok(mut entity) = commands.get_entity(collider_entity) {
                    entity.despawn();
                }
            }
        }

        insert_one(&mut commands, chunk, expected_entity, contents);
    }

    // TODO: Check whether this manual wake can be removed after updating Avian.
    // Avian 0.7 does not wake the other body when a static collider is removed
    // from an existing contact. Removing Sleeping uses Avian's component hook
    // to wake the body's complete physics island.
    for body in bodies_to_wake {
        commands.entity(body).remove::<Sleeping>();
    }
}

fn chunk_collider_rebuild_aabb(position: ChunkPos) -> ColliderAabb {
    let min = position.origin_translation();
    let max = min + Vec3::splat(CHUNK_SIZE as f32);
    ColliderAabb::from_min_max(min, max).grow(Vec3::splat(COLLIDER_REBUILD_WAKE_MARGIN))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Item;
    use crate::world::{
        WorldHeight,
        chunk::{ChunkCell, ChunkPos},
    };

    #[derive(Resource)]
    struct TestDimension(Entity);

    #[derive(Resource, Default)]
    struct CenterColliderReady(bool);

    fn collider_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, rebuild_chunk_colliders);
        add_test_dimension(&mut app);
        app
    }

    fn collider_disabled_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, discard_chunk_collider_work);
        add_test_dimension(&mut app);
        app
    }

    fn fixed_collider_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<CenterColliderReady>()
            .add_systems(FixedPreUpdate, rebuild_chunk_colliders)
            .add_systems(Update, observe_center_collider_readiness);
        add_test_dimension(&mut app);
        app
    }

    fn add_test_dimension(app: &mut App) {
        let dimension = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(dimension).insert((
            Dimension::new_for_test(dimension, WorldHeight::new(1).unwrap()),
            Active,
        ));
        app.insert_resource(TestDimension(dimension));
    }

    fn observe_center_collider_readiness(
        dimension: Single<&Dimension, With<Active>>,
        chunks: Query<(&ChunkPosition, &ChunkContentCounts, Option<&Children>)>,
        colliders: Query<(), With<Collider>>,
        mut ready: ResMut<CenterColliderReady>,
    ) {
        ready.0 = column_colliders_ready(
            dimension.into_inner(),
            ChunkColumn::new(0, 0),
            &chunks,
            &colliders,
        );
    }

    fn register_active_chunk(app: &mut App, position: ChunkPos, chunk: Entity) {
        let dimension = app.world().resource::<TestDimension>().0;
        register_dimension_chunk(app, dimension, position, chunk);
    }

    fn enqueue_active_rebuild(app: &mut App, position: ChunkPos) {
        let dimension = app.world().resource::<TestDimension>().0;
        enqueue_dimension_rebuild(app, dimension, position);
    }

    fn enqueue_dimension_rebuild(app: &mut App, dimension: Entity, position: ChunkPos) {
        assert!(
            app.world_mut()
                .entity_mut(dimension)
                .get_mut::<Dimension>()
                .unwrap()
                .enqueue_collider_rebuild(position)
        );
    }

    fn register_dimension_chunk(
        app: &mut App,
        dimension: Entity,
        position: ChunkPos,
        chunk: Entity,
    ) {
        app.world_mut()
            .entity_mut(dimension)
            .get_mut::<Dimension>()
            .unwrap()
            .register_published_chunk(position, chunk);
    }

    fn collider_child_count(world: &World, chunk: Entity) -> usize {
        world
            .get::<Children>(chunk)
            .map(|children| {
                children
                    .iter()
                    .filter(|child| world.get::<Collider>(*child).is_some())
                    .count()
            })
            .unwrap_or_default()
    }

    #[test]
    fn queued_collider_rebuild_is_consumed_after_rebuild() {
        let mut app = collider_app();

        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, Item::Stone.into());
        let meta = chunk.compute_content_counts();
        let chunk_entity = app
            .world_mut()
            .spawn((ChunkPosition::from(ChunkPos::ZERO), chunk, meta))
            .id();
        register_active_chunk(&mut app, ChunkPos::ZERO, chunk_entity);
        enqueue_active_rebuild(&mut app, ChunkPos::ZERO);

        app.update();

        let world = app.world();
        let dimension = world
            .get::<Dimension>(world.resource::<TestDimension>().0)
            .unwrap();
        assert!(!dimension.has_pending_collider_rebuild(ChunkPos::ZERO));
        assert_eq!(collider_child_count(world, chunk_entity), 1);
    }

    #[test]
    fn disabled_runtime_discards_collider_work_without_building_colliders() {
        let mut app = collider_disabled_app();

        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, Item::Stone.into());
        let meta = chunk.compute_content_counts();
        let chunk_entity = app
            .world_mut()
            .spawn((ChunkPosition::from(ChunkPos::ZERO), chunk, meta))
            .id();
        register_active_chunk(&mut app, ChunkPos::ZERO, chunk_entity);
        enqueue_active_rebuild(&mut app, ChunkPos::ZERO);

        app.update();

        let world = app.world();
        let dimension = world
            .get::<Dimension>(world.resource::<TestDimension>().0)
            .unwrap();
        assert!(!dimension.has_pending_collider_rebuild(ChunkPos::ZERO));
        assert_eq!(collider_child_count(world, chunk_entity), 0);
    }

    #[test]
    fn collider_rebuild_includes_solid_ice_but_skips_water() {
        let mut app = collider_app();

        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, Item::Ice.into());
        chunk.set_cell_xyz(1, 0, 0, ChunkCell::water_source());
        let meta = chunk.compute_content_counts();
        let chunk_entity = app
            .world_mut()
            .spawn((ChunkPosition::from(ChunkPos::ZERO), chunk, meta))
            .id();
        register_active_chunk(&mut app, ChunkPos::ZERO, chunk_entity);
        enqueue_active_rebuild(&mut app, ChunkPos::ZERO);

        app.update();

        assert_eq!(collider_child_count(app.world(), chunk_entity), 1);
    }

    #[test]
    fn collider_rebuild_is_scoped_to_active_dimension_with_duplicate_coordinates() {
        let mut app = collider_app();
        let active_dimension = app.world().resource::<TestDimension>().0;
        let foreign_dimension = app.world_mut().spawn(Dimension::default()).id();
        let position = ChunkPos::new(-7, 0, 11);

        let mut active_chunk = Chunk::default();
        active_chunk.set_cell_xyz(0, 0, 0, Item::Stone.into());
        let active_meta = active_chunk.compute_content_counts();
        let active_entity = app
            .world_mut()
            .spawn((ChunkPosition::from(position), active_chunk, active_meta))
            .id();

        let mut foreign_chunk = Chunk::default();
        foreign_chunk.set_cell_xyz(0, 0, 0, Item::Stone.into());
        let foreign_meta = foreign_chunk.compute_content_counts();
        let foreign_entity = app
            .world_mut()
            .spawn((ChunkPosition::from(position), foreign_chunk, foreign_meta))
            .id();

        register_dimension_chunk(&mut app, active_dimension, position, active_entity);
        register_dimension_chunk(&mut app, foreign_dimension, position, foreign_entity);
        enqueue_dimension_rebuild(&mut app, active_dimension, position);
        enqueue_dimension_rebuild(&mut app, foreign_dimension, position);

        app.update();

        let world = app.world();
        assert!(
            !world
                .get::<Dimension>(active_dimension)
                .unwrap()
                .has_pending_collider_rebuild(position)
        );
        assert_eq!(collider_child_count(world, active_entity), 1);
        assert!(
            world
                .get::<Dimension>(foreign_dimension)
                .unwrap()
                .has_pending_collider_rebuild(position)
        );
        assert_eq!(collider_child_count(world, foreign_entity), 0);
    }

    #[test]
    fn replacement_chunk_rejects_stale_collider_work() {
        let mut app = collider_app();
        let position = ChunkPos::ZERO;

        let mut old_chunk = Chunk::default();
        old_chunk.set_cell_xyz(0, 0, 0, Item::Stone.into());
        let old_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(position),
                old_chunk.clone(),
                old_chunk.compute_content_counts(),
            ))
            .id();
        register_active_chunk(&mut app, position, old_entity);
        enqueue_active_rebuild(&mut app, position);

        let mut replacement = Chunk::default();
        replacement.set_cell_xyz(1, 0, 0, Item::Stone.into());
        let replacement_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(position),
                replacement.clone(),
                replacement.compute_content_counts(),
            ))
            .id();
        register_active_chunk(&mut app, position, replacement_entity);
        enqueue_active_rebuild(&mut app, position);

        app.update();

        assert_eq!(collider_child_count(app.world(), old_entity), 0);
        assert_eq!(collider_child_count(app.world(), replacement_entity), 1);
    }

    #[test]
    fn empty_chunk_completion_removes_existing_collider() {
        let mut app = collider_app();
        let entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(ChunkPos::ZERO),
                Chunk::default(),
                ChunkContentCounts::default(),
            ))
            .id();
        register_active_chunk(&mut app, ChunkPos::ZERO, entity);
        app.world_mut().spawn((
            ChildOf(entity),
            Collider::cuboid(1.0, 1.0, 1.0),
            RigidBody::Static,
        ));
        enqueue_active_rebuild(&mut app, ChunkPos::ZERO);

        app.update();

        assert_eq!(collider_child_count(app.world(), entity), 0);
        let dimension = app
            .world()
            .get::<Dimension>(app.world().resource::<TestDimension>().0)
            .unwrap();
        assert!(!dimension.has_pending_collider_rebuild(ChunkPos::ZERO));
    }

    #[test]
    fn collider_rebuild_wakes_sleeping_bodies_near_the_affected_chunk() {
        let mut app = collider_app();
        let chunk_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(ChunkPos::ZERO),
                Chunk::default(),
                ChunkContentCounts::default(),
            ))
            .id();
        register_active_chunk(&mut app, ChunkPos::ZERO, chunk_entity);
        enqueue_active_rebuild(&mut app, ChunkPos::ZERO);

        let nearby = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Sleeping,
                ColliderAabb::new(vec3(8.0, 16.25, 8.0), Vec3::splat(0.125)),
            ))
            .id();
        let far_away = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Sleeping,
                ColliderAabb::new(vec3(32.0, 16.25, 32.0), Vec3::splat(0.125)),
            ))
            .id();

        app.update();

        assert!(!app.world().entity(nearby).contains::<Sleeping>());
        assert!(app.world().entity(far_away).contains::<Sleeping>());
    }

    #[test]
    fn current_but_unreadable_chunk_requeues_collider_work() {
        let mut app = collider_app();
        let entity = app
            .world_mut()
            .spawn(ChunkPosition::from(ChunkPos::ZERO))
            .id();
        register_active_chunk(&mut app, ChunkPos::ZERO, entity);
        enqueue_active_rebuild(&mut app, ChunkPos::ZERO);

        app.update();

        let dimension = app
            .world()
            .get::<Dimension>(app.world().resource::<TestDimension>().0)
            .unwrap();
        assert!(dimension.has_pending_collider_rebuild(ChunkPos::ZERO));
    }

    #[test]
    fn published_column_readiness_waits_for_fixed_collider_rebuild() {
        let mut app = fixed_collider_app();
        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, Item::Stone.into());
        let contents = chunk.compute_content_counts();
        let entity = app
            .world_mut()
            .spawn((ChunkPosition::from(ChunkPos::ZERO), chunk, contents))
            .id();
        register_active_chunk(&mut app, ChunkPos::ZERO, entity);
        enqueue_active_rebuild(&mut app, ChunkPos::ZERO);

        app.world_mut().run_schedule(Update);
        assert!(!app.world().resource::<CenterColliderReady>().0);

        app.world_mut().run_schedule(FixedPreUpdate);
        app.world_mut().run_schedule(Update);

        assert!(app.world().resource::<CenterColliderReady>().0);
        assert_eq!(collider_child_count(app.world(), entity), 1);
    }
}
