use bevy::prelude::*;

use crate::{
    block::{BlockType, BlockUpdateEvent, BlockUpdateKind, LocalBlockPos},
    chunk::{global_pos_to_chunk_pos, Chunk, CHUNK_ISIZE},
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
) {
    for click in click_events.read() {
        // let dim = dimension.get(click.dimension);
        // TODO: Abstract over Dimension, Chunk, Block
        // Single struct?
        let dim = dimension.single();

        let chunk_pos = global_pos_to_chunk_pos(click.pos);

        let Some(chunk_entity) = dim.chunks.get(&chunk_pos) else {
            warn!("Clicked into missing chunk");
            continue;
        };

        let Some(mut chunk) = chunks.get_mut(*chunk_entity).ok() else {
            continue;
        };

        let pos: LocalBlockPos = (click.pos - chunk_pos * CHUNK_ISIZE).as_uvec3().into();

        match click.button {
            MouseButtonForBlock::Left => {
                if !chunk.break_block(&pos) {
                    continue;
                };

                block_events.send(BlockUpdateEvent {
                    chunk: *chunk_entity,
                    pos,
                    kind: BlockUpdateKind::Break,
                });
            }
            MouseButtonForBlock::Right => {
                let block = BlockType::Grass;
                if !chunk.place_block(&pos, block.clone()) {
                    continue;
                };

                block_events.send(BlockUpdateEvent {
                    chunk: *chunk_entity,
                    pos,
                    kind: BlockUpdateKind::Place(block),
                });
            }
            _ => {} // Move into hand
        }
    }
}
