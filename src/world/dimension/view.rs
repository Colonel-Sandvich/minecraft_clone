use bevy::prelude::*;

use crate::world::chunk::CHUNK_ISIZE;

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewDistance {
    chunks: i32,
}

impl ViewDistance {
    pub fn new(chunks: i32) -> Self {
        Self {
            chunks: chunks.max(1),
        }
    }

    pub fn chunks(self) -> i32 {
        self.chunks
    }

    pub fn increase(&mut self) {
        self.chunks = self.chunks.saturating_add(1);
    }

    pub fn decrease(&mut self) {
        self.chunks = self.chunks.saturating_sub(1).max(1);
    }
}

impl Default for ViewDistance {
    fn default() -> Self {
        Self::new(40)
    }
}

pub fn chunk_positions_in_view(
    centre_translation: Vec3,
    height_chunks: usize,
    view_distance: i32,
) -> Vec<IVec3> {
    let centre_in_chunk_coords = (centre_translation / CHUNK_ISIZE as f32).with_y(0.0);
    let centre_chunk = centre_in_chunk_coords.floor().as_ivec3();
    let view_distance = view_distance.max(0);

    let mut columns = (-view_distance..=view_distance)
        .flat_map(|z| (-view_distance..=view_distance).map(move |x| ivec2(x, z)))
        .filter(|p| p.length_squared() <= view_distance * view_distance)
        .collect::<Vec<_>>();
    columns.sort_by_key(|p| (p.length_squared(), p.y, p.x));

    let height = height_chunks as i32;
    let mut chunks = Vec::with_capacity(columns.len() * height_chunks);
    for column in &columns {
        for y in 0..height {
            chunks.push(ivec3(
                centre_chunk.x + column.x,
                y,
                centre_chunk.z + column.y,
            ));
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{chunk::CHUNK_SIZE, generation::WorldMetadata};

    const TEST_VIEW_DISTANCE: i32 = 14;

    fn expected_chunk_count(metadata: &WorldMetadata) -> usize {
        (-TEST_VIEW_DISTANCE..=TEST_VIEW_DISTANCE)
            .flat_map(|z| (-TEST_VIEW_DISTANCE..=TEST_VIEW_DISTANCE).map(move |x| ivec2(x, z)))
            .filter(|p| p.length_squared() <= TEST_VIEW_DISTANCE * TEST_VIEW_DISTANCE)
            .count()
            * metadata.height_chunks
    }

    #[test]
    fn chunk_positions_in_view_are_bounded() {
        let metadata = WorldMetadata::with_seed(1);
        let moved_chunk_x = TEST_VIEW_DISTANCE + 2;
        let origin =
            chunk_positions_in_view(Vec3::ZERO, metadata.height_chunks, TEST_VIEW_DISTANCE);
        let moved = chunk_positions_in_view(
            Vec3::new(CHUNK_SIZE as f32 * moved_chunk_x as f32, 0.0, 0.0),
            metadata.height_chunks,
            TEST_VIEW_DISTANCE,
        );

        assert_eq!(origin.len(), expected_chunk_count(&metadata));
        assert_eq!(moved.len(), origin.len());
        assert_eq!(origin[0], IVec3::ZERO);
        assert_eq!(moved[0], ivec3(moved_chunk_x, 0, 0));
        assert!(origin.contains(&IVec3::ZERO));
        assert!(!moved.contains(&IVec3::ZERO));
        assert!(moved.contains(&ivec3(moved_chunk_x, 0, 0)));
    }

    #[test]
    fn chunk_positions_load_full_columns_before_moving_to_next_column() {
        let metadata = WorldMetadata::with_seed(1);
        let origin =
            chunk_positions_in_view(Vec3::ZERO, metadata.height_chunks, TEST_VIEW_DISTANCE);

        assert_eq!(origin[0], ivec3(0, 0, 0));
        assert_eq!(origin[1], ivec3(0, 1, 0));

        let origin_column_count = origin
            .iter()
            .take(metadata.height_chunks)
            .filter(|pos| pos.x == 0 && pos.z == 0)
            .count();
        assert_eq!(origin_column_count, metadata.height_chunks);

        assert_ne!(
            ivec2(
                origin[metadata.height_chunks].x,
                origin[metadata.height_chunks].z
            ),
            IVec2::ZERO
        );
    }
}
