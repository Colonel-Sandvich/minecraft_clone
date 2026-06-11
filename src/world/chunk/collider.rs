use super::{CHUNK_VOLUME, Chunk, ChunkNeedsColliderRebuild};
use crate::world::WORLD_COLLISION_LAYERS;
use avian3d::prelude::*;
use bevy::math::vec3;
use bevy::prelude::*;

pub struct ChunkColliderPlugin;

impl Plugin for ChunkColliderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedPreUpdate, rebuild_chunk_colliders);
    }
}

fn insert_one(commands: &mut Commands, chunk: &Chunk, chunk_entity: Entity) {
    let mut cubes = Vec::with_capacity(CHUNK_VOLUME);
    for (block, (x, y, z)) in chunk.iter() {
        if !block.is_solid() {
            continue;
        }

        cubes.push((
            vec3(x as f32, y as f32, z as f32) + Vec3::splat(0.5),
            Quat::IDENTITY,
            Collider::cuboid(1.0, 1.0, 1.0),
        ));
    }

    if cubes.is_empty() {
        return;
    }

    commands.spawn((
        ChildOf(chunk_entity),
        Collider::compound(cubes),
        WORLD_COLLISION_LAYERS,
        RigidBody::Static,
    ));
}

fn rebuild_chunk_colliders(
    mut commands: Commands,
    chunks_q: Query<(&Chunk, Entity, Option<&Children>), With<ChunkNeedsColliderRebuild>>,
    collider_q: Query<Entity, With<Collider>>,
) {
    for (chunk, chunk_entity, children) in chunks_q.iter() {
        if let Some(children) = children {
            for collider_entity in collider_q.iter_many(children) {
                commands.get_entity(collider_entity).unwrap().despawn();
            }
        }

        insert_one(&mut commands, chunk, chunk_entity);
        commands
            .entity(chunk_entity)
            .remove::<ChunkNeedsColliderRebuild>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;

    #[test]
    fn collider_rebuild_marker_is_removed_after_rebuild() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, rebuild_chunk_colliders);

        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Stone;
        let chunk_entity = app
            .world_mut()
            .spawn((chunk, ChunkNeedsColliderRebuild))
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
}
