use bevy::prelude::*;

use crate::world::chunk::CHUNK_ISIZE;

pub const VIEW_DISTANCE: i32 = 7;
pub(crate) const VERTICAL_LOAD_STRIPE_HEIGHT: i32 = 2;

pub fn chunk_positions_in_view(centre_translation: Vec3, height_chunks: usize) -> Vec<IVec3> {
    let centre_in_chunk_coords = (centre_translation / CHUNK_ISIZE as f32).with_y(0.0);
    let centre_chunk = centre_in_chunk_coords.floor().as_ivec3();

    let mut columns = (-VIEW_DISTANCE..=VIEW_DISTANCE)
        .flat_map(|z| (-VIEW_DISTANCE..=VIEW_DISTANCE).map(move |x| ivec2(x, z)))
        .filter(|p| p.length_squared() <= VIEW_DISTANCE * VIEW_DISTANCE)
        .collect::<Vec<_>>();
    columns.sort_by_key(|p| (p.length_squared(), p.y, p.x));

    let height = height_chunks as i32;
    let mut chunks = Vec::with_capacity(columns.len() * height_chunks);
    for band_start in (0..height).step_by(VERTICAL_LOAD_STRIPE_HEIGHT as usize) {
        let band_end = (band_start + VERTICAL_LOAD_STRIPE_HEIGHT).min(height);
        for column in &columns {
            for y in band_start..band_end {
                chunks.push(ivec3(
                    centre_chunk.x + column.x,
                    y,
                    centre_chunk.z + column.y,
                ));
            }
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{chunk::CHUNK_SIZE, generation::WorldMetadata};

    fn expected_chunk_count(metadata: &WorldMetadata) -> usize {
        (-VIEW_DISTANCE..=VIEW_DISTANCE)
            .flat_map(|z| (-VIEW_DISTANCE..=VIEW_DISTANCE).map(move |x| ivec2(x, z)))
            .filter(|p| p.length_squared() <= VIEW_DISTANCE * VIEW_DISTANCE)
            .count()
            * metadata.height_chunks
    }

    #[test]
    fn chunk_positions_in_view_are_bounded() {
        let metadata = WorldMetadata::with_seed(1);
        let origin = chunk_positions_in_view(Vec3::ZERO, metadata.height_chunks);
        let moved = chunk_positions_in_view(
            Vec3::new(CHUNK_SIZE as f32 * 10.0, 0.0, 0.0),
            metadata.height_chunks,
        );

        assert_eq!(origin.len(), expected_chunk_count(&metadata));
        assert_eq!(moved.len(), origin.len());
        assert_eq!(origin[0], IVec3::ZERO);
        assert_eq!(moved[0], ivec3(10, 0, 0));
        assert!(origin.contains(&IVec3::ZERO));
        assert!(!moved.contains(&IVec3::ZERO));
        assert!(moved.contains(&ivec3(10, 0, 0)));
    }

    #[test]
    fn chunk_positions_interleave_vertical_sections_across_horizontal_columns() {
        let metadata = WorldMetadata::with_seed(1);
        let origin = chunk_positions_in_view(Vec3::ZERO, metadata.height_chunks);

        assert_eq!(origin[0], ivec3(0, 0, 0));
        assert_eq!(origin[1], ivec3(0, 1, 0));
        assert_ne!(ivec2(origin[2].x, origin[2].z), IVec2::ZERO);
        assert_eq!(
            origin
                .iter()
                .take(5)
                .filter(|pos| pos.x == 0 && pos.z == 0)
                .count(),
            VERTICAL_LOAD_STRIPE_HEIGHT as usize
        );
    }
}
