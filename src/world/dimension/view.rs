use bevy::{platform::collections::HashSet, prelude::*};
use bevy_settings::{ReflectSettingsGroup, SettingsGroup};

use crate::world::{
    chunk::{ChunkColumn, ChunkPos},
    generation::WorldHeight,
};

#[derive(Resource, SettingsGroup, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource, SettingsGroup, Default)]
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
        self.chunks.max(1)
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
        Self::new(24)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DesiredColumnViewKey {
    center: ChunkColumn,
    radius: i32,
    height: WorldHeight,
}

/// Cached, nearest-first columns requested by the active view.
#[derive(Resource, Debug, Default)]
pub struct DesiredColumnView {
    key: Option<DesiredColumnViewKey>,
    revision: u64,
    columns: Vec<ChunkColumn>,
    column_set: HashSet<ChunkColumn>,
}

impl DesiredColumnView {
    /// Refreshes the cache and returns whether its inputs changed.
    pub fn refresh(
        &mut self,
        center: ChunkColumn,
        view_distance: ViewDistance,
        height: WorldHeight,
    ) -> bool {
        let key = DesiredColumnViewKey {
            center,
            radius: view_distance.chunks(),
            height,
        };
        if self.key == Some(key) {
            return false;
        }

        self.columns = columns_in_radius(center, key.radius);
        self.column_set.clear();
        self.column_set.extend(self.columns.iter().copied());
        self.key = Some(key);
        self.revision = self
            .revision
            .checked_add(1)
            .expect("desired column view revision overflowed");
        true
    }

    /// Increases whenever the cached view is rebuilt.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn center(&self) -> Option<ChunkColumn> {
        self.key.map(|key| key.center)
    }

    pub fn height(&self) -> Option<WorldHeight> {
        self.key.map(|key| key.height)
    }

    pub fn columns(&self) -> &[ChunkColumn] {
        &self.columns
    }

    pub fn contains_column(&self, column: ChunkColumn) -> bool {
        self.column_set.contains(&column)
    }

    pub fn contains_chunk(&self, position: ChunkPos) -> bool {
        let Some(key) = self.key else {
            return false;
        };
        let y = position.as_ivec3().y;
        (0..key.height.chunks_i32()).contains(&y)
            && self.contains_column(ChunkColumn::from(position))
    }

    pub fn chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        let height = self.key.map_or(0, |key| key.height.chunks_i32());
        self.columns
            .iter()
            .flat_map(move |column| (0..height).map(|y| column.chunk(y)))
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    pub fn chunk_count(&self) -> usize {
        self.key
            .map_or(0, |key| self.columns.len() * key.height.chunks())
    }
}

fn columns_in_radius(center: ChunkColumn, radius: i32) -> Vec<ChunkColumn> {
    let radius = radius.max(0);
    let radius_squared = i64::from(radius) * i64::from(radius);
    let mut offsets = (-radius..=radius)
        .flat_map(|z| (-radius..=radius).map(move |x| (x, z)))
        .filter(|&(x, z)| {
            i64::from(x) * i64::from(x) + i64::from(z) * i64::from(z) <= radius_squared
        })
        .collect::<Vec<_>>();
    offsets.sort_by_key(|&(x, z)| {
        (
            i64::from(x) * i64::from(x) + i64::from(z) * i64::from(z),
            z,
            x,
        )
    });
    offsets
        .into_iter()
        .map(|(x, z)| ChunkColumn::new(center.x() + x, center.z() + z))
        .collect()
}

/// Compatibility helper for callers that need an owned vector.
pub fn chunk_positions_in_view(
    centre_translation: Vec3,
    height_chunks: usize,
    view_distance: i32,
) -> Vec<IVec3> {
    let center = ChunkColumn::from(ChunkPos::containing_translation(centre_translation));
    let height = WorldHeight::new(height_chunks)
        .unwrap_or_else(|error| panic!("invalid desired chunk view: {error}"));
    let mut view = DesiredColumnView::default();
    view.refresh(center, ViewDistance::new(view_distance), height);
    view.chunks().map(ChunkPos::as_ivec3).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::chunk::CHUNK_SIZE;

    const TEST_VIEW_DISTANCE: i32 = 14;

    fn expected_column_count(radius: i32) -> usize {
        (-radius..=radius)
            .flat_map(|z| (-radius..=radius).map(move |x| (x, z)))
            .filter(|(x, z)| x * x + z * z <= radius * radius)
            .count()
    }

    #[test]
    fn desired_columns_are_typed_bounded_and_nearest_first() {
        let center = ChunkColumn::new(-3, 7);
        let height = WorldHeight::new(5).unwrap();
        let mut view = DesiredColumnView::default();

        assert!(view.refresh(center, ViewDistance::new(TEST_VIEW_DISTANCE), height));

        assert_eq!(view.columns()[0], center);
        assert_eq!(
            view.column_count(),
            expected_column_count(TEST_VIEW_DISTANCE)
        );
        assert_eq!(view.chunk_count(), view.column_count() * height.chunks());
        assert!(view.contains_column(ChunkColumn::new(-3 + TEST_VIEW_DISTANCE, 7)));
        assert!(!view.contains_column(ChunkColumn::new(-3 + TEST_VIEW_DISTANCE + 1, 7)));
    }

    #[test]
    fn view_cache_changes_only_for_center_radius_or_height_inputs() {
        let center = ChunkColumn::new(2, -5);
        let radius = ViewDistance::new(4);
        let height = WorldHeight::new(5).unwrap();
        let mut view = DesiredColumnView::default();

        assert_eq!(view.revision(), 0);
        assert!(view.refresh(center, radius, height));
        assert_eq!(view.revision(), 1);
        assert_eq!(view.center(), Some(center));
        assert_eq!(view.height(), Some(height));
        assert!(!view.refresh(center, radius, height));
        assert_eq!(view.revision(), 1);
        assert!(view.refresh(ChunkColumn::new(3, -5), radius, height));
        assert_eq!(view.revision(), 2);
        assert!(view.refresh(ChunkColumn::new(3, -5), ViewDistance::new(5), height));
        assert_eq!(view.revision(), 3);
        assert!(view.refresh(
            ChunkColumn::new(3, -5),
            ViewDistance::new(5),
            WorldHeight::new(6).unwrap()
        ));
        assert_eq!(view.revision(), 4);
    }

    #[test]
    fn chunk_expansion_keeps_each_column_contiguous() {
        let height = WorldHeight::new(5).unwrap();
        let mut view = DesiredColumnView::default();
        view.refresh(
            ChunkColumn::new(0, 0),
            ViewDistance::new(TEST_VIEW_DISTANCE),
            height,
        );
        let chunks = view.chunks().collect::<Vec<_>>();

        assert_eq!(chunks[0], ChunkPos::new(0, 0, 0));
        assert_eq!(chunks[1], ChunkPos::new(0, 1, 0));
        assert!(
            chunks
                .iter()
                .take(height.chunks())
                .all(|position| ChunkColumn::from(*position) == ChunkColumn::new(0, 0))
        );
        assert_ne!(
            ChunkColumn::from(chunks[height.chunks()]),
            ChunkColumn::new(0, 0)
        );
    }

    #[test]
    fn player_translation_changes_cache_only_across_column_boundaries() {
        let height = WorldHeight::new(5).unwrap();
        let radius = ViewDistance::new(2);
        let mut view = DesiredColumnView::default();
        let origin = ChunkColumn::from(ChunkPos::containing_translation(Vec3::ZERO));
        assert!(view.refresh(origin, radius, height));

        let same_column = ChunkColumn::from(ChunkPos::containing_translation(Vec3::new(
            CHUNK_SIZE as f32 - 0.5,
            100.0,
            0.0,
        )));
        assert!(!view.refresh(same_column, radius, height));

        let next_column = ChunkColumn::from(ChunkPos::containing_translation(Vec3::new(
            CHUNK_SIZE as f32,
            100.0,
            0.0,
        )));
        assert!(view.refresh(next_column, radius, height));
    }
}
