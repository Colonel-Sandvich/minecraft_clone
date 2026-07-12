use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::Resource,
};

use crate::world::chunk::ChunkColumn;

const INITIAL_LIGHT_TILE_WIDTH: i32 = 6;

/// Soft target for subchunks admitted to one asynchronous lighting patch.
///
/// Zero pauses admission of new work. Initial compact tiles and the minimum
/// dependency closure remain indivisible and may exceed a smaller nonzero
/// target.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ColumnLightBudget(pub usize);

impl Default for ColumnLightBudget {
    fn default() -> Self {
        // One 4x4 runtime calculation union at the default five-chunk height.
        Self(80)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LightPatchPlan {
    commit_columns: Vec<ChunkColumn>,
    calculation_columns: Vec<ChunkColumn>,
}

/// How a visible column participates in dependency-complete initial lighting.
///
/// Runtime relighting must report [`Self::Excluded`], even when the published
/// column's authoritative light is pending. It remains the responsibility of
/// the connected runtime planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitialLightColumnState {
    /// A staged, pending column whose complete H1 dependency set is resident.
    Ready,
    /// Initial lighting is still waiting for this visible column or its H1 data.
    Waiting,
    /// The column is not initial-light work, for example because it is lit or published.
    Excluded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct InitialLightTile {
    x: i32,
    z: i32,
}

#[derive(Default)]
struct InitialLightTileCandidates {
    ready: Vec<ChunkColumn>,
    waiting: bool,
}

impl InitialLightTile {
    fn containing(column: ChunkColumn) -> Self {
        Self {
            x: column.x().div_euclid(INITIAL_LIGHT_TILE_WIDTH),
            z: column.z().div_euclid(INITIAL_LIGHT_TILE_WIDTH),
        }
    }
}

impl LightPatchPlan {
    /// Builds one compact dependency-complete initial-light patch.
    ///
    /// Tiles are anchored to the world chunk grid rather than the current
    /// view. A tile is admitted only after every waiting column in its visible
    /// intersection becomes ready. Columns outside the circular visible view
    /// do not hold up a partial edge tile.
    pub(crate) fn build_initial_tile(
        visible_nearest_first: &[ChunkColumn],
        desired_view_center: ChunkColumn,
        mut state: impl FnMut(ChunkColumn) -> InitialLightColumnState,
    ) -> Self {
        let center_is_visible = visible_nearest_first.contains(&desired_view_center);
        let center_state = center_is_visible.then(|| state(desired_view_center));
        if center_state == Some(InitialLightColumnState::Ready) {
            return Self::from_initial_commits(vec![desired_view_center]);
        }

        let mut tiles_nearest_first = Vec::<(InitialLightTile, InitialLightTileCandidates)>::new();
        let mut tile_indexes = HashMap::new();
        for &column in visible_nearest_first {
            let tile = InitialLightTile::containing(column);
            let tile_index = *tile_indexes.entry(tile).or_insert_with(|| {
                let index = tiles_nearest_first.len();
                tiles_nearest_first.push((tile, InitialLightTileCandidates::default()));
                index
            });
            let column_state = if column == desired_view_center {
                center_state.expect("visible center state must be classified")
            } else {
                state(column)
            };
            let candidates = &mut tiles_nearest_first[tile_index].1;
            match column_state {
                InitialLightColumnState::Ready => candidates.ready.push(column),
                InitialLightColumnState::Waiting => candidates.waiting = true,
                InitialLightColumnState::Excluded => {}
            }
        }

        for (_, candidates) in tiles_nearest_first {
            if !candidates.waiting && !candidates.ready.is_empty() {
                return Self::from_initial_commits(candidates.ready);
            }
        }

        Self::default()
    }

    pub(crate) fn build(
        visible_nearest_first: &[ChunkColumn],
        height_chunks: usize,
        target_budget: usize,
        mut is_ready: impl FnMut(ChunkColumn) -> bool,
    ) -> Self {
        if target_budget == 0 {
            return Self::default();
        }
        let candidates = visible_nearest_first
            .iter()
            .copied()
            .filter(|&column| is_ready(column))
            .collect::<Vec<_>>();
        let Some(&first) = candidates.first() else {
            return Self::default();
        };

        let mut commit_columns = vec![first];
        let mut commit_set = HashSet::from([first]);
        let mut calculation_set = first.chebyshev_neighborhood(1).collect::<HashSet<_>>();

        loop {
            let mut changed = false;
            for &candidate in candidates.iter().skip(1) {
                if commit_set.contains(&candidate)
                    || !commit_columns
                        .iter()
                        .any(|&commit| columns_touch(commit, candidate))
                {
                    continue;
                }

                let dependencies = candidate.chebyshev_neighborhood(1).collect::<Vec<_>>();
                let added = dependencies
                    .iter()
                    .filter(|&&dependency| !calculation_set.contains(&dependency))
                    .count();
                let target_chunks = calculation_set
                    .len()
                    .saturating_add(added)
                    .saturating_mul(height_chunks);
                if target_chunks > target_budget {
                    continue;
                }

                commit_columns.push(candidate);
                commit_set.insert(candidate);
                calculation_set.extend(dependencies);
                changed = true;
            }
            if !changed {
                break;
            }
        }

        let mut calculation_columns = calculation_set.into_iter().collect::<Vec<_>>();
        calculation_columns.sort_unstable_by_key(|column| (column.z(), column.x()));
        Self {
            commit_columns,
            calculation_columns,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.commit_columns.is_empty()
    }

    pub(crate) fn commit_columns(&self) -> &[ChunkColumn] {
        &self.commit_columns
    }

    pub(crate) fn calculation_columns(&self) -> &[ChunkColumn] {
        &self.calculation_columns
    }

    pub(crate) fn commits(&self, column: ChunkColumn) -> bool {
        self.commit_columns.contains(&column)
    }

    pub(crate) fn calculation_chunk_count(&self, height_chunks: usize) -> usize {
        self.calculation_columns.len() * height_chunks
    }

    pub(crate) fn scratch_chunk_count(&self, height_chunks: usize) -> usize {
        (self.calculation_columns.len() - self.commit_columns.len()) * height_chunks
    }

    fn from_initial_commits(commit_columns: Vec<ChunkColumn>) -> Self {
        let mut calculation_columns = commit_columns
            .iter()
            .flat_map(|column| column.chebyshev_neighborhood(1))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        calculation_columns.sort_unstable_by_key(|column| (column.z(), column.x()));
        Self {
            commit_columns,
            calculation_columns,
        }
    }
}

fn columns_touch(left: ChunkColumn, right: ChunkColumn) -> bool {
    (i64::from(left.x()) - i64::from(right.x())).abs() <= 1
        && (i64::from(left.z()) - i64::from(right.z())).abs() <= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> Vec<ChunkColumn> {
        (min_z..=max_z)
            .flat_map(|z| (min_x..=max_x).map(move |x| ChunkColumn::new(x, z)))
            .collect()
    }

    fn circle_nearest_first(center: ChunkColumn, radius: i32) -> Vec<ChunkColumn> {
        let mut columns = rectangle(
            center.x() - radius,
            center.x() + radius,
            center.z() - radius,
            center.z() + radius,
        )
        .into_iter()
        .filter(|column| {
            let x = i64::from(column.x()) - i64::from(center.x());
            let z = i64::from(column.z()) - i64::from(center.z());
            x * x + z * z <= i64::from(radius) * i64::from(radius)
        })
        .collect::<Vec<_>>();
        columns.sort_unstable_by_key(|column| {
            let x = i64::from(column.x()) - i64::from(center.x());
            let z = i64::from(column.z()) - i64::from(center.z());
            (x * x + z * z, z, x)
        });
        columns
    }

    #[test]
    fn initial_center_fast_path_commits_one_core_with_its_full_h1_union() {
        let center = ChunkColumn::new(2, -3);
        let visible = circle_nearest_first(center, 4);
        let mut classified = Vec::new();
        let plan = LightPatchPlan::build_initial_tile(&visible, center, |column| {
            classified.push(column);
            InitialLightColumnState::Ready
        });

        assert_eq!(classified, vec![center]);
        assert_eq!(plan.commit_columns(), &[center]);
        assert_eq!(
            plan.calculation_columns(),
            center.chebyshev_neighborhood(1).collect::<Vec<_>>()
        );
    }

    #[test]
    fn initial_tiles_are_world_stable_across_zero_and_negative_coordinates() {
        let left = ChunkColumn::new(-1, 0);
        let right = ChunkColumn::new(0, 0);
        let center = ChunkColumn::new(20, 20);
        let plan = LightPatchPlan::build_initial_tile(&[left, right], center, |_| {
            InitialLightColumnState::Ready
        });

        assert_eq!(plan.commit_columns(), &[left]);
        assert!(!plan.commits(right));
        assert_eq!(plan.calculation_columns().len(), 9);

        let negative_tile = rectangle(-6, -1, -6, -1);
        let plan = LightPatchPlan::build_initial_tile(&negative_tile, center, |_| {
            InitialLightColumnState::Ready
        });
        assert_eq!(plan.commit_columns(), negative_tile.as_slice());
        assert_eq!(plan.calculation_columns().len(), 64);
        assert_eq!(plan.calculation_chunk_count(2), 128);
        assert_eq!(plan.scratch_chunk_count(2), 56);
    }

    #[test]
    fn incomplete_tile_waits_and_published_runtime_work_is_never_committed() {
        let center = ChunkColumn::new(20, 20);
        let ready = ChunkColumn::new(0, 0);
        let waiting = ChunkColumn::new(1, 0);
        let published_runtime_pending = ChunkColumn::new(0, 1);
        let visible = [ready, waiting, published_runtime_pending];

        let blocked = LightPatchPlan::build_initial_tile(&visible, center, |column| {
            if column == waiting {
                InitialLightColumnState::Waiting
            } else if column == published_runtime_pending {
                InitialLightColumnState::Excluded
            } else {
                InitialLightColumnState::Ready
            }
        });
        assert!(blocked.is_empty());

        let plan = LightPatchPlan::build_initial_tile(&visible, center, |column| {
            if column == published_runtime_pending {
                InitialLightColumnState::Excluded
            } else {
                InitialLightColumnState::Ready
            }
        });
        assert_eq!(plan.commit_columns(), &[ready, waiting]);
        assert!(!plan.commits(published_runtime_pending));
        assert!(
            plan.calculation_columns()
                .contains(&published_runtime_pending),
            "an excluded column may still be required as scratch H1 input"
        );
    }

    #[test]
    fn circular_edge_tile_commits_only_its_visible_partial_intersection() {
        let center = ChunkColumn::new(0, 0);
        let visible = circle_nearest_first(center, 7);
        let edge_tile = InitialLightTile::containing(ChunkColumn::new(7, 0));
        let expected = visible
            .iter()
            .copied()
            .filter(|&column| InitialLightTile::containing(column) == edge_tile)
            .collect::<Vec<_>>();
        assert!(!expected.is_empty());
        assert!(expected.len() < (INITIAL_LIGHT_TILE_WIDTH * INITIAL_LIGHT_TILE_WIDTH) as usize);

        let plan = LightPatchPlan::build_initial_tile(&visible, center, |column| {
            if InitialLightTile::containing(column) == edge_tile {
                InitialLightColumnState::Ready
            } else {
                InitialLightColumnState::Excluded
            }
        });

        assert_eq!(plan.commit_columns(), expected.as_slice());
        assert!(
            plan.commit_columns()
                .iter()
                .all(|column| visible.contains(column))
        );
        let expected_calculation = expected
            .iter()
            .flat_map(|column| column.chebyshev_neighborhood(1))
            .collect::<HashSet<_>>();
        assert_eq!(
            plan.calculation_columns()
                .iter()
                .copied()
                .collect::<HashSet<_>>(),
            expected_calculation
        );
    }

    #[test]
    fn radius_24_initial_trace_uses_compact_tiles() {
        let center = ChunkColumn::new(0, 0);
        let visible = circle_nearest_first(center, 24);
        let mut pending = visible.iter().copied().collect::<HashSet<_>>();
        let mut patch_runs = 0;
        let mut calculation_columns = 0;
        let mut scratch_columns = 0;
        let mut max_calculation_columns = 0;

        while !pending.is_empty() {
            let plan = LightPatchPlan::build_initial_tile(&visible, center, |column| {
                if pending.contains(&column) {
                    InitialLightColumnState::Ready
                } else {
                    InitialLightColumnState::Excluded
                }
            });
            assert!(!plan.is_empty(), "ready initial columns must make progress");
            patch_runs += 1;
            calculation_columns += plan.calculation_columns().len();
            scratch_columns += plan.calculation_columns().len() - plan.commit_columns().len();
            max_calculation_columns = max_calculation_columns.max(plan.calculation_columns().len());
            for &column in plan.commit_columns() {
                assert!(pending.remove(&column), "a core must commit exactly once");
            }
        }

        assert_eq!(visible.len(), 1_793);
        assert_eq!(patch_runs, 63);
        assert_eq!(calculation_columns, 3_373);
        assert_eq!(scratch_columns, 1_580);
        assert_eq!(max_calculation_columns, 64);
        assert_eq!(calculation_columns * 5, 16_865);
    }

    #[test]
    fn one_core_always_admits_its_complete_dependency_closure() {
        let center = ChunkColumn::new(-3, 8);
        let plan = LightPatchPlan::build(&[center], 5, 1, |_| true);

        assert_eq!(plan.commit_columns(), &[center]);
        assert_eq!(plan.calculation_columns().len(), 9);
        assert_eq!(plan.calculation_chunk_count(5), 45);
        assert_eq!(plan.scratch_chunk_count(5), 40);
    }

    #[test]
    fn zero_budget_explicitly_pauses_lighting() {
        let center = ChunkColumn::new(0, 0);
        assert!(LightPatchPlan::build(&[center], 5, 0, |_| true).is_empty());
    }

    #[test]
    fn adjacent_cores_share_their_calculation_halo() {
        let left = ChunkColumn::new(0, 0);
        let right = ChunkColumn::new(1, 0);
        let plan = LightPatchPlan::build(&[left, right], 2, 24, |_| true);

        assert_eq!(plan.commit_columns(), &[left, right]);
        assert_eq!(plan.calculation_columns().len(), 12);
        assert_eq!(plan.calculation_chunk_count(2), 24);

        let constrained = LightPatchPlan::build(&[left, right], 2, 23, |_| true);
        assert_eq!(constrained.commit_columns(), &[left]);
        assert_eq!(constrained.calculation_columns().len(), 9);
    }

    #[test]
    fn two_by_two_core_batch_calculates_one_four_by_four_union() {
        let cores = [
            ChunkColumn::new(0, 0),
            ChunkColumn::new(1, 0),
            ChunkColumn::new(0, 1),
            ChunkColumn::new(1, 1),
        ];
        let plan = LightPatchPlan::build(&cores, 5, 80, |_| true);

        assert_eq!(plan.commit_columns(), cores.as_slice());
        assert_eq!(plan.calculation_columns().len(), 16);
        assert_eq!(plan.calculation_chunk_count(5), 80);
    }

    #[test]
    fn readiness_and_connectivity_bound_the_batch() {
        let first = ChunkColumn::new(0, 0);
        let blocked = ChunkColumn::new(1, 0);
        let disconnected = ChunkColumn::new(3, 0);
        let plan = LightPatchPlan::build(&[first, blocked, disconnected], 1, 100, |column| {
            column != blocked
        });

        assert_eq!(plan.commit_columns(), &[first]);
        assert!(!plan.commits(disconnected));
    }
}
