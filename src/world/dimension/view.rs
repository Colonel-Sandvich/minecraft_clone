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

#[derive(Debug, Default)]
struct OrderedColumnSet {
    columns: Vec<ChunkColumn>,
    set: HashSet<ChunkColumn>,
}

impl OrderedColumnSet {
    fn from_nearest_first(columns: Vec<ChunkColumn>) -> Self {
        let set = columns.iter().copied().collect::<HashSet<_>>();
        assert_eq!(
            columns.len(),
            set.len(),
            "ordered column set cannot contain duplicates"
        );
        Self { columns, set }
    }

    fn columns(&self) -> &[ChunkColumn] {
        &self.columns
    }

    fn contains(&self, column: ChunkColumn) -> bool {
        self.set.contains(&column)
    }

    fn len(&self) -> usize {
        self.columns.len()
    }
}

/// Cached visible columns and their resident lighting dependency closure.
#[derive(Resource, Debug, Default)]
pub struct DesiredColumnView {
    key: Option<DesiredColumnViewKey>,
    revision: u64,
    visible: OrderedColumnSet,
    resident: OrderedColumnSet,
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

        let visible = OrderedColumnSet::from_nearest_first(columns_in_radius(center, key.radius));
        let resident = OrderedColumnSet::from_nearest_first(resident_closure(&visible, center));
        self.visible = visible;
        self.resident = resident;
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

    pub fn visible_columns(&self) -> &[ChunkColumn] {
        self.visible.columns()
    }

    pub fn resident_columns(&self) -> &[ChunkColumn] {
        self.resident.columns()
    }

    pub fn support_columns(&self) -> impl Iterator<Item = ChunkColumn> + '_ {
        self.resident
            .columns()
            .iter()
            .copied()
            .filter(|&column| !self.visible.contains(column))
    }

    pub fn contains_visible_column(&self, column: ChunkColumn) -> bool {
        self.visible.contains(column)
    }

    pub fn contains_resident_column(&self, column: ChunkColumn) -> bool {
        self.resident.contains(column)
    }

    pub fn contains_visible_chunk(&self, position: ChunkPos) -> bool {
        self.contains_chunk_in(position, &self.visible)
    }

    pub fn contains_resident_chunk(&self, position: ChunkPos) -> bool {
        self.contains_chunk_in(position, &self.resident)
    }

    pub fn visible_chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.chunks_in(&self.visible)
    }

    pub fn resident_chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.chunks_in(&self.resident)
    }

    pub fn visible_column_count(&self) -> usize {
        self.visible.len()
    }

    pub fn resident_column_count(&self) -> usize {
        self.resident.len()
    }

    pub fn visible_chunk_count(&self) -> usize {
        self.chunk_count_in(&self.visible)
    }

    pub fn resident_chunk_count(&self) -> usize {
        self.chunk_count_in(&self.resident)
    }

    fn contains_chunk_in(&self, position: ChunkPos, columns: &OrderedColumnSet) -> bool {
        let Some(key) = self.key else {
            return false;
        };
        (0..key.height.chunks_i32()).contains(&position.y()) && columns.contains(position.column())
    }

    fn chunks_in<'a>(
        &'a self,
        columns: &'a OrderedColumnSet,
    ) -> impl Iterator<Item = ChunkPos> + 'a {
        let height = self.key.map_or(0, |key| key.height.chunks_i32());
        columns
            .columns()
            .iter()
            .flat_map(move |column| (0..height).map(|y| column.chunk(y)))
    }

    fn chunk_count_in(&self, columns: &OrderedColumnSet) -> usize {
        self.key
            .map_or(0, |key| columns.len() * key.height.chunks())
    }
}

fn resident_closure(visible: &OrderedColumnSet, center: ChunkColumn) -> Vec<ChunkColumn> {
    let mut closure = visible
        .columns()
        .iter()
        .flat_map(|column| column.chebyshev_neighborhood(1))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    sort_nearest_first(&mut closure, center);
    closure
}

fn columns_in_radius(center: ChunkColumn, radius: i32) -> Vec<ChunkColumn> {
    let radius = radius.max(0);
    let radius_squared = i64::from(radius) * i64::from(radius);
    let offsets = (-radius..=radius)
        .flat_map(|z| (-radius..=radius).map(move |x| (x, z)))
        .filter(|&(x, z)| {
            i64::from(x) * i64::from(x) + i64::from(z) * i64::from(z) <= radius_squared
        })
        .collect::<Vec<_>>();
    let mut columns = offsets
        .into_iter()
        .map(|(x, z)| ChunkColumn::new(center.x() + x, center.z() + z))
        .collect::<Vec<_>>();
    sort_nearest_first(&mut columns, center);
    columns
}

fn sort_nearest_first(columns: &mut [ChunkColumn], center: ChunkColumn) {
    columns.sort_unstable_by_key(|column| {
        let x = i64::from(column.x()) - i64::from(center.x());
        let z = i64::from(column.z()) - i64::from(center.z());
        (x * x + z * z, z, x)
    });
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

        assert_eq!(view.visible_columns()[0], center);
        assert_eq!(view.resident_columns()[0], center);
        assert_eq!(
            view.visible_column_count(),
            expected_column_count(TEST_VIEW_DISTANCE)
        );
        assert_eq!(
            view.visible_chunk_count(),
            view.visible_column_count() * height.chunks()
        );
        assert_eq!(
            view.resident_chunk_count(),
            view.resident_column_count() * height.chunks()
        );

        let visible_edge = ChunkColumn::new(-3 + TEST_VIEW_DISTANCE, 7);
        let support_edge = ChunkColumn::new(-3 + TEST_VIEW_DISTANCE + 1, 7);
        assert!(view.contains_visible_column(visible_edge));
        assert!(view.contains_resident_column(visible_edge));
        assert!(!view.contains_visible_column(support_edge));
        assert!(view.contains_resident_column(support_edge));
        assert!(!view.contains_resident_column(ChunkColumn::new(-3 + TEST_VIEW_DISTANCE + 2, 7)));
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
        let chunks = view.resident_chunks().collect::<Vec<_>>();

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
    fn resident_view_is_the_exact_one_column_dependency_closure() {
        let center = ChunkColumn::new(0, 0);
        let height = WorldHeight::new(2).unwrap();
        let mut view = DesiredColumnView::default();
        view.refresh(center, ViewDistance::new(1), height);

        assert_eq!(view.visible_column_count(), 5);
        assert_eq!(view.resident_column_count(), 21);
        assert_eq!(view.support_columns().count(), 16);

        for &visible in view.visible_columns() {
            for dependency in visible.chebyshev_neighborhood(1) {
                assert!(view.contains_resident_column(dependency));
            }
        }
        for &resident in view.resident_columns() {
            assert!(
                view.visible_columns().iter().any(|visible| visible
                    .chebyshev_neighborhood(1)
                    .any(|item| item == resident)),
                "resident closure contains unrelated column {resident:?}"
            );
        }

        let row_widths = (-2..=2)
            .map(|z| {
                view.resident_columns()
                    .iter()
                    .filter(|column| column.z() == z)
                    .count()
            })
            .collect::<Vec<_>>();
        assert_eq!(row_widths, vec![3, 5, 5, 5, 3]);
    }

    #[test]
    fn visible_and_resident_chunk_queries_include_height_bounds() {
        let center = ChunkColumn::new(3, -8);
        let height = WorldHeight::new(2).unwrap();
        let mut view = DesiredColumnView::default();
        view.refresh(center, ViewDistance::new(1), height);

        let support = ChunkColumn::new(5, -8);
        assert!(view.contains_visible_chunk(center.chunk(0)));
        assert!(view.contains_resident_chunk(center.chunk(1)));
        assert!(!view.contains_visible_chunk(support.chunk(0)));
        assert!(view.contains_resident_chunk(support.chunk(0)));
        assert!(!view.contains_resident_chunk(center.chunk(-1)));
        assert!(!view.contains_resident_chunk(center.chunk(2)));
        assert_eq!(view.visible_chunks().count(), 5 * height.chunks());
        assert_eq!(view.resident_chunks().count(), 21 * height.chunks());
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
