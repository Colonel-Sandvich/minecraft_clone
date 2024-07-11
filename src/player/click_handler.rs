use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

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
    rapier_context: Res<RapierContext>,
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
                if rapier_context.intersection_with_shape(
                    pos.to_global().as_vec3() + 0.5,
                    Rot::IDENTITY,
                    &Collider::cuboid(0.5, 0.5, 0.5),
                    QueryFilter::exclude_fixed(),
                ) != None
                {
                    continue;
                };

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
