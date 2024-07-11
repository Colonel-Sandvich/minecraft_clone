use avian3d::{
    collision::Collider,
    spatial_query::{SpatialQuery, SpatialQueryFilter},
};
use bevy::prelude::*;

use crate::{
    block::{BlockType, BlockUpdateEvent, BlockUpdateKind},
    chunk::Chunk,
    dimension::Dimension,
};

use super::laser::{BlockClickEvent, MouseButtonForBlock};

pub struct ClickHandlerPlugin;

impl Plugin for ClickHandlerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, process_clicks);
    }
}

fn process_clicks(
    mut click_events: EventReader<BlockClickEvent>,
    dimension: Query<&Dimension>,
    mut chunks: Query<&mut Chunk>,
    mut block_events: EventWriter<BlockUpdateEvent>,
    mut picked_block: Local<BlockType>,
    query: SpatialQuery,
) {
    for click in click_events.read() {
        // let dim = dimension.get(click.dimension);
        // TODO: Abstract over Dimension, Chunk, Block
        // Single struct?
        let dim = dimension.single();

        let pos = click.pos;

        let Some(chunk_entity) = dim.chunks.get(&pos.chunk) else {
            warn!("Clicked into missing chunk");
            continue;
        };

        let Some(mut chunk) = chunks.get_mut(*chunk_entity).ok() else {
            continue;
        };

        match click.button {
            MouseButtonForBlock::Left => {
                if !chunk.break_block(pos.block) {
                    continue;
                };

                block_events.send(BlockUpdateEvent {
                    chunk: *chunk_entity,
                    pos,
                    kind: BlockUpdateKind::Break,
                });
            }
            MouseButtonForBlock::Right => {
                if !query
                    .shape_intersections(
                        &Collider::cuboid(0.90, 0.90, 0.90),
                        pos.to_global().as_vec3() + 0.5,
                        Quat::IDENTITY,
                        SpatialQueryFilter::default(),
                    )
                    .is_empty()
                {
                    continue;
                }

                if !chunk.place_block(pos.block, picked_block.clone()) {
                    continue;
                };

                block_events.send(BlockUpdateEvent {
                    chunk: *chunk_entity,
                    pos,
                    kind: BlockUpdateKind::Place(*picked_block),
                });
            }
            MouseButtonForBlock::Middle => {
                *picked_block = chunk.get(pos.block);
            }
        }
    }
}
