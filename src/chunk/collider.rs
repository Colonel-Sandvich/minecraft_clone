use avian3d::prelude::*;
use bevy::math::vec3;
use bevy::prelude::*;
use itertools::Itertools;

use crate::{block::BlockUpdateEvent, chunk::Chunk};

use super::CHUNK_VOLUME;

pub struct ChunkColliderPlugin;

impl Plugin for ChunkColliderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, insert_colliders_naive);
        app.add_systems(PostUpdate, update_collider_naive);
    }
}

fn insert_colliders_naive(mut commands: Commands, chunks_q: Query<(&Chunk, Entity), Added<Chunk>>) {
    for (chunk, chunk_entity) in chunks_q.iter() {
        insert_one(&mut commands, chunk, chunk_entity);
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

    commands
        .spawn((
            SpatialBundle::default(),
            Collider::compound(cubes),
            RigidBody::Static,
        ))
        .set_parent(chunk_entity);
}

fn update_collider_naive(
    mut commands: Commands,
    mut block_updates: EventReader<BlockUpdateEvent>,
    chunks_q: Query<(&Chunk, Entity, Option<&Children>)>,
    collider_q: Query<Entity, With<Collider>>,
) {
    for (chunk, chunk_entity, children) in
        chunks_q.iter_many(block_updates.read().map(|u| u.chunk).unique())
    {
        if let Some(children) = children {
            let mut colliders = collider_q.iter_many(children);

            if let Some(collider_entity) = colliders.fetch_next() {
                commands
                    .get_entity(collider_entity)
                    .unwrap()
                    .despawn_recursive();
            };
        }

        insert_one(&mut commands, chunk, chunk_entity);
    }
}
