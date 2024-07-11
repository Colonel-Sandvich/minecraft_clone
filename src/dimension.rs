use std::ops::Mul;

use bevy::{prelude::*, utils::HashMap};

use crate::{
    block::{BlockPos, BlockUpdateEvent, BlockUpdateKind},
    chunk::{
        util::{generate_flat_chunk_data, generate_full_chunk_data},
        Chunk, ChunkBundle, CHUNK_SIZE,
    },
};

#[derive(Default, Component)]
pub struct Dimension {
    pub chunks: HashMap<IVec3, Entity>,
}

pub struct DimensionPlugin;

impl Plugin for DimensionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup, spawn_chunks).chain());
        // app.add_systems(Update, (place_block_in_chunks).chain());
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((Dimension::default(), SpatialBundle::default()));
}

fn spawn_chunks(mut commands: Commands, mut dimension: Query<(&mut Dimension, Entity)>) {
    for (mut dim, dimension_entity) in dimension.iter_mut() {
        let chunks = make_chunks();

        for (pos, chunk) in chunks.into_iter() {
            let chunk_entity = commands
                .spawn(ChunkBundle {
                    chunk,
                    spatial: SpatialBundle {
                        transform: Transform::from_translation(
                            pos.as_vec3().mul(CHUNK_SIZE as f32),
                        ),
                        ..default()
                    },
                    ..default()
                })
                .set_parent(dimension_entity)
                .id();

            dim.chunks.insert(pos, chunk_entity);
        }
    }
}

pub const SPAWN_AREA: i32 = 0;
pub const HEIGHT: i32 = 4;

pub fn make_chunks() -> HashMap<IVec3, Chunk> {
    let mut chunks = HashMap::new();
    for x in -SPAWN_AREA..=SPAWN_AREA {
        for z in -SPAWN_AREA..=SPAWN_AREA {
            for y in 0..HEIGHT {
                let position = IVec3::new(x, y, z);
                chunks.insert(
                    position,
                    if y == 0 {
                        generate_full_chunk_data()
                    } else {
                        Chunk::default()
                    },
                );
            }
        }
    }

    chunks
}

fn place_block_in_chunks(
    dimension: Query<(&Dimension, &Children)>,
    mut chunks: Query<&mut Chunk>,
    mut block_events: EventWriter<BlockUpdateEvent>,
) {
    for (_, chunk_children) in dimension.iter() {
        for chunk_entity in chunk_children.iter() {
            let Some(mut chunk) = chunks.get_mut(*chunk_entity).ok() else {
                continue;
            };

            let Some(block) = chunk.place_random_block() else {
                continue;
            };

            // block_events.send(BlockUpdateEvent {
            //     chunk: *chunk_entity,
            //     pos: BlockPos {
            //         // chunk: chunk_pos, ???
            //         chunk: IVec3::ZERO,
            //         block: block.pos.0,
            //     },
            //     kind: BlockUpdateKind::Place(block.kind),
            // });
        }
    }
}

fn place_block_in_chunks_2(
    dimension: Query<&Dimension>,
    mut chunks: Query<&mut Chunk>,
    mut block_events: EventWriter<BlockUpdateEvent>,
) {
    for dim in dimension.iter() {
        for (chunk_pos, chunk_entity) in dim.chunks.iter() {
            let Some(mut chunk) = chunks.get_mut(*chunk_entity).ok() else {
                continue;
            };

            let Some(block) = chunk.place_random_block() else {
                continue;
            };

            block_events.send(BlockUpdateEvent {
                chunk: *chunk_entity,
                pos: BlockPos {
                    chunk: chunk_pos.clone(),
                    block: block.1,
                },
                kind: BlockUpdateKind::Place(block.0),
            });
        }
    }
}
