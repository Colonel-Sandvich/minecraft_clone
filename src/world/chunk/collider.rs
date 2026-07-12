use super::{Chunk, ChunkContentCounts, ChunkNeedsColliderRebuild, ChunkPosition};
use crate::world::{
    WORLD_COLLISION_LAYERS,
    dimension::{Active, Dimension},
};
use avian3d::prelude::*;
use bevy::prelude::*;

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

fn rebuild_chunk_colliders(
    mut commands: Commands,
    chunks_q: Query<
        (
            &ChunkPosition,
            &Chunk,
            &ChunkContentCounts,
            Entity,
            Option<&Children>,
        ),
        With<ChunkNeedsColliderRebuild>,
    >,
    collider_q: Query<Entity, With<Collider>>,
    dimension: Option<Single<&Dimension, With<Active>>>,
) {
    if chunks_q.is_empty() {
        return;
    }

    let Some(dimension) = dimension else {
        return;
    };
    for (position, chunk, meta, chunk_entity, children) in &chunks_q {
        if dimension.chunk_entity(position.chunk_pos()) != Some(chunk_entity) {
            continue;
        }

        if let Some(children) = children {
            for collider_entity in collider_q.iter_many(children) {
                commands.get_entity(collider_entity).unwrap().despawn();
            }
        }

        insert_one(&mut commands, chunk, chunk_entity, meta);
        commands
            .entity(chunk_entity)
            .remove::<ChunkNeedsColliderRebuild>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;
    use crate::world::chunk::{ChunkCell, ChunkPos};

    #[derive(Resource)]
    struct TestDimension(Entity);

    fn collider_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, rebuild_chunk_colliders);
        let dimension = app.world_mut().spawn((Dimension::default(), Active)).id();
        app.insert_resource(TestDimension(dimension));
        app
    }

    fn register_active_chunk(app: &mut App, position: ChunkPos, chunk: Entity) {
        let dimension = app.world().resource::<TestDimension>().0;
        register_dimension_chunk(app, dimension, position, chunk);
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
            .register_chunk(position, chunk);
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
    fn collider_rebuild_marker_is_removed_after_rebuild() {
        let mut app = collider_app();

        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, BlockType::Stone.into());
        let meta = chunk.compute_content_counts();
        let chunk_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(ChunkPos::ZERO),
                chunk,
                meta,
                ChunkNeedsColliderRebuild,
            ))
            .id();
        register_active_chunk(&mut app, ChunkPos::ZERO, chunk_entity);

        app.update();

        let world = app.world();
        assert!(
            world
                .get::<ChunkNeedsColliderRebuild>(chunk_entity)
                .is_none()
        );
        assert_eq!(collider_child_count(world, chunk_entity), 1);
    }

    #[test]
    fn collider_rebuild_includes_solid_ice_but_skips_water() {
        let mut app = collider_app();

        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, BlockType::Ice.into());
        chunk.set_cell_xyz(1, 0, 0, ChunkCell::water_source());
        let meta = chunk.compute_content_counts();
        let chunk_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(ChunkPos::ZERO),
                chunk,
                meta,
                ChunkNeedsColliderRebuild,
            ))
            .id();
        register_active_chunk(&mut app, ChunkPos::ZERO, chunk_entity);

        app.update();

        assert_eq!(collider_child_count(app.world(), chunk_entity), 1);
    }

    #[test]
    fn collider_rebuild_is_scoped_to_active_dimension_with_duplicate_coordinates() {
        let mut app = collider_app();
        let active_dimension = app.world().resource::<TestDimension>().0;
        let foreign_dimension = app.world_mut().spawn(Dimension::default()).id();
        let position = ChunkPos::new(-7, 2, 11);

        let mut active_chunk = Chunk::default();
        active_chunk.set_cell_xyz(0, 0, 0, BlockType::Stone.into());
        let active_meta = active_chunk.compute_content_counts();
        let active_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(position),
                active_chunk,
                active_meta,
                ChunkNeedsColliderRebuild,
            ))
            .id();

        let mut foreign_chunk = Chunk::default();
        foreign_chunk.set_cell_xyz(0, 0, 0, BlockType::Stone.into());
        let foreign_meta = foreign_chunk.compute_content_counts();
        let foreign_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(position),
                foreign_chunk,
                foreign_meta,
                ChunkNeedsColliderRebuild,
            ))
            .id();

        register_dimension_chunk(&mut app, active_dimension, position, active_entity);
        register_dimension_chunk(&mut app, foreign_dimension, position, foreign_entity);

        app.update();

        let world = app.world();
        assert!(
            world
                .get::<ChunkNeedsColliderRebuild>(active_entity)
                .is_none()
        );
        assert_eq!(collider_child_count(world, active_entity), 1);
        assert!(
            world
                .get::<ChunkNeedsColliderRebuild>(foreign_entity)
                .is_some()
        );
        assert_eq!(collider_child_count(world, foreign_entity), 0);
    }
}
