use bevy::{platform::collections::HashSet, prelude::Resource};

use crate::world::chunk::ChunkColumn;

/// Soft target for subchunks admitted to one synchronous lighting patch.
///
/// Zero pauses lighting. A nonzero value always admits one complete 3x3
/// dependency patch, even when that minimum exceeds the target.
#[derive(Resource, Debug, Clone, Copy)]
pub struct ColumnLightBudget(pub usize);

impl Default for ColumnLightBudget {
    fn default() -> Self {
        // One 4x4 calculation union at the default five-chunk world height.
        Self(80)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LightPatchPlan {
    commit_columns: Vec<ChunkColumn>,
    calculation_columns: Vec<ChunkColumn>,
}

impl LightPatchPlan {
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
}

fn columns_touch(left: ChunkColumn, right: ChunkColumn) -> bool {
    (i64::from(left.x()) - i64::from(right.x())).abs() <= 1
        && (i64::from(left.z()) - i64::from(right.z())).abs() <= 1
}

#[cfg(test)]
mod tests {
    use super::*;

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
