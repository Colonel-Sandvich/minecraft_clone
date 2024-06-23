use std::ops::Mul;

use bevy::{prelude::*, utils::HashMap};

use crate::chunk::{generate_flat_chunk_data, Chunk, ChunkBundle, CHUNK_SIZE};

#[derive(Default, Component)]
pub struct Dimension {
    pub chunks: HashMap<IVec3, Entity>,
}

pub struct DimensionPlugin;

impl Plugin for DimensionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup, generate_chunks).chain());
        app.add_systems(Update, (place_block_in_chunks).chain());
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((Dimension::default(), SpatialBundle::default()));
}

fn generate_chunks(mut dimension: Query<(Entity, &mut Dimension)>, mut commands: Commands) {
    let chunks = generate_flat_chunks();
    let (dimension_entity, mut dim) = dimension.single_mut();

    for (pos, chunk) in chunks.into_iter() {
        let chunk_entity = commands
            .spawn(ChunkBundle {
                chunk,
                spatial: SpatialBundle {
                    transform: Transform::from_translation(pos.as_vec3().mul(CHUNK_SIZE as f32)),
                    ..default()
                },
            })
            .set_parent(dimension_entity)
            .id();
        dim.chunks.insert(pos, chunk_entity);
    }
}

pub const SPAWN_AREA: i32 = 1;

pub fn generate_flat_chunks() -> HashMap<IVec3, Chunk> {
    let mut chunks = HashMap::new();
    for x in -SPAWN_AREA..=SPAWN_AREA {
        for z in -SPAWN_AREA..=SPAWN_AREA {
            let position = IVec3::new(x, 0, z);

            chunks.insert(position, generate_flat_chunk_data(position));
        }
    }

    chunks
}

fn place_block_in_chunks(
    mut commands: Commands,
    dimension: Query<(&Dimension, &Children)>,
    mut chunks: Query<&mut Chunk>,
) {
    for (_, chunk_children) in dimension.iter() {
        for chunk_entity in chunk_children.iter() {
            let Some(mut chunk) = chunks.get_mut(*chunk_entity).ok() else {
                continue;
            };
            if let Some(x) = chunk.place_block() {
                commands.entity(*chunk_entity).insert(x);
            }
            if let Some(x) = chunk.place_block() {
                commands.entity(*chunk_entity).insert(x);
            }
        }
    }
}
