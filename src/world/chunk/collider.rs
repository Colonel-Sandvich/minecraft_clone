use super::{Chunk, ChunkBlockCounts, ChunkNeedsColliderRebuild};
use crate::world::WORLD_COLLISION_LAYERS;
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
    meta: &ChunkBlockCounts,
) {
    if meta.rendered == 0 {
        return;
    }

    let mut voxels = Vec::with_capacity(meta.rendered as usize);
    for (cell, (x, y, z)) in chunk.iter() {
        if !cell.is_solid() {
            continue;
        }

        voxels.push(IVec3::new(x as i32, y as i32, z as i32));
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
        (&Chunk, &ChunkBlockCounts, Entity, Option<&Children>),
        With<ChunkNeedsColliderRebuild>,
    >,
    collider_q: Query<Entity, With<Collider>>,
) {
    for (chunk, meta, chunk_entity, children) in chunks_q.iter() {
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
    use crate::world::chunk::ChunkCell;

    #[test]
    fn collider_rebuild_marker_is_removed_after_rebuild() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, rebuild_chunk_colliders);

        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, BlockType::Stone.into());
        let meta = chunk.compute_block_counts();
        let chunk_entity = app
            .world_mut()
            .spawn((chunk, meta, ChunkNeedsColliderRebuild))
            .id();

        app.update();

        let world = app.world();
        assert!(
            world
                .get::<ChunkNeedsColliderRebuild>(chunk_entity)
                .is_none()
        );
        let children = world.get::<Children>(chunk_entity).unwrap();
        let collider_child_count = children
            .iter()
            .filter(|child| world.get::<Collider>(*child).is_some())
            .count();
        assert_eq!(collider_child_count, 1);
    }

    #[test]
    fn collider_rebuild_includes_solid_ice_but_skips_water() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, rebuild_chunk_colliders);

        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, BlockType::Ice.into());
        chunk.set_cell_xyz(1, 0, 0, ChunkCell::water_source());
        let meta = chunk.compute_block_counts();
        let chunk_entity = app
            .world_mut()
            .spawn((chunk, meta, ChunkNeedsColliderRebuild))
            .id();

        app.update();

        let world = app.world();
        let children = world.get::<Children>(chunk_entity).unwrap();
        let collider_child_count = children
            .iter()
            .filter(|child| world.get::<Collider>(*child).is_some())
            .count();
        assert_eq!(collider_child_count, 1);
    }
}
