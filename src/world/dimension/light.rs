use bevy::{platform::collections::HashMap, prelude::*};

use crate::world::chunk::{
    Chunk, ChunkBlockCounts, ChunkHeightmap, ChunkLight, ChunkNeedsLightRebuild, ChunkNeedsSave,
    ChunkPosition, CHUNK_ISIZE, chunk_neighbor_offsets, compute_light,
    light::offset_to_bit_index,
};

pub(crate) fn rebuild_chunk_light(
    mut commands: Commands,
    needs_rebuild: Query<
        (Entity, &ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap, &ChunkBlockCounts),
        With<ChunkNeedsLightRebuild>,
    >,
    all_chunks: Query<(Entity, &ChunkPosition, &Chunk, &ChunkLight)>,
) {
    let chunk_map: HashMap<IVec3, (Entity, &Chunk, &ChunkLight)> = all_chunks
        .iter()
        .map(|(entity, pos, chunk, light)| (pos.0, (entity, chunk, light)))
        .collect();

    for (entity, pos, chunk, _, heightmap, block_counts) in needs_rebuild.iter() {
        let center_pos = pos.0;

        let mut blocks = HashMap::new();
        let mut light_copies: HashMap<IVec3, (Entity, ChunkLight)> = HashMap::new();

        let center_light = chunk_map
            .get(&center_pos)
            .map(|(_, _, l)| (**l).clone())
            .unwrap_or_default();
        light_copies.insert(IVec3::ZERO, (entity, center_light));

        for offset in chunk_neighbor_offsets() {
            let neighbor_pos = center_pos + offset;
            if let Some(&(neighbor_entity, neighbor_chunk, neighbor_light)) =
                chunk_map.get(&neighbor_pos)
            {
                blocks.insert(offset, neighbor_chunk);
                light_copies.insert(offset, (neighbor_entity, (*neighbor_light).clone()));
            }
        }

        let mut center_light_copy = light_copies
            .remove(&IVec3::ZERO)
            .expect("center light must be seeded at IVec3::ZERO")
            .1;
        let mut heightmap = *heightmap;
        let mut dirty_neighbors = 0u32;

        {
            let mut lights_mut: HashMap<IVec3, &mut ChunkLight> = light_copies
                .iter_mut()
                .map(|(k, v)| (*k, &mut v.1))
                .collect();
            let column_y = (center_pos.y * CHUNK_ISIZE) as u32;
            compute_light(
                chunk,
                &mut center_light_copy,
                &mut heightmap,
                &blocks,
                &mut lights_mut,
                &mut dirty_neighbors,
                block_counts.rendered,
                column_y,
                true,
            );
        }

        commands.entity(entity).insert((center_light_copy, heightmap));

        for (offset, (neighbor_entity, updated_light)) in light_copies {
            if offset != IVec3::ZERO && dirty_neighbors & (1 << offset_to_bit_index(offset)) != 0 {
                commands.entity(neighbor_entity).insert((
                    updated_light,
                    ChunkNeedsSave,
                ));
            }
        }

        commands.entity(entity).remove::<ChunkNeedsLightRebuild>();
    }
}
