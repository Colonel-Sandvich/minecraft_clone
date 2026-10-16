use bevy::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};

use super::{
    CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkCell, ChunkPos, FluidProfile, FluidState, LocalBlockPos,
    WorldBlockPos, chunk_linear_index,
};

const HORIZONTAL_DIRS: [IVec3; 4] = [IVec3::NEG_X, IVec3::X, IVec3::NEG_Z, IVec3::Z];
const MAX_DROP_SEARCH_DISTANCE: u8 = 4;

/// The changed cell plus its six face neighbours: the positions a block update
/// reaches, which is what makes vanilla schedule their fluid ticks.
const CELL_NOTIFY_OFFSETS: [IVec3; 7] = [
    IVec3::ZERO,
    IVec3::NEG_X,
    IVec3::X,
    IVec3::NEG_Y,
    IVec3::Y,
    IVec3::NEG_Z,
    IVec3::Z,
];

#[derive(Debug, Clone)]
pub(crate) struct FluidSnapshot {
    chunks: HashMap<IVec3, Box<[ChunkCell; CHUNK_VOLUME]>>,
}

impl FluidSnapshot {
    pub(crate) fn new(chunks: HashMap<IVec3, Box<[ChunkCell; CHUNK_VOLUME]>>) -> Self {
        Self { chunks }
    }

    pub(crate) fn from_chunk(pos: IVec3, chunk: &Chunk) -> Self {
        Self::new(HashMap::from([(pos, Box::new(chunk.to_cell_buffer()))]))
    }

    pub(crate) fn cell(&self, world_pos: IVec3) -> Option<ChunkCell> {
        let address = WorldBlockPos::from_ivec3(world_pos).split();
        self.chunks
            .get(&address.chunk().as_ivec3())
            .map(|cells| cells[address.local().index().as_usize()])
    }

    fn fluid_positions_in_chunk(
        &self,
        chunk_pos: IVec3,
        profile: FluidProfile,
    ) -> Vec<(IVec3, FluidState)> {
        let Some(cells) = self.chunks.get(&chunk_pos) else {
            return Vec::new();
        };

        let mut positions = Vec::new();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let Some(fluid) = cells[chunk_linear_index(x, y, z)].as_fluid() else {
                        continue;
                    };
                    if fluid.ty() != profile.ty {
                        continue;
                    }

                    positions.push((
                        ChunkPos::from_ivec3(chunk_pos)
                            .block(LocalBlockPos::new(x as u32, y as u32, z as u32))
                            .world()
                            .as_ivec3(),
                        fluid,
                    ));
                }
            }
        }

        positions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FluidUpdate {
    pub(crate) pos: IVec3,
    pub(crate) cell: ChunkCell,
}

/// The fluid cells with a pending update, mirroring vanilla's scheduled fluid
/// ticks.
///
/// Vanilla only ticks fluids the world has scheduled: placing or breaking a
/// block wakes the edited cell and its face neighbours, and every fluid write
/// wakes the cell it landed in. A settled pool therefore has no pending ticks
/// and stays put until something disturbs it. Ticking every existing cell each
/// step instead let untouched water flood whenever the drop landscape changed
/// somewhere within a four-block search radius of a pool edge.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ScheduledFluids {
    cells: HashSet<IVec3>,
}

impl ScheduledFluids {
    pub(crate) fn contains(&self, pos: IVec3) -> bool {
        self.cells.contains(&pos)
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "asserted by fluid tests"))]
    pub(crate) fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = IVec3> + '_ {
        self.cells.iter().copied()
    }

    /// Schedules one exact cell, for wakes that already name the position.
    pub(crate) fn wake_cell(&mut self, pos: IVec3) {
        self.cells.insert(pos);
    }

    /// Schedules a cell and its six face neighbours, the set a block update
    /// reaches through `neighborChanged`.
    pub(crate) fn wake_with_neighbours(&mut self, pos: IVec3) {
        for offset in CELL_NOTIFY_OFFSETS {
            self.cells.insert(pos + offset);
        }
    }

    /// Schedules every fluid cell of one chunk, as if the chunk had just been
    /// written cell by cell.
    fn all_fluids_in(snapshot: &FluidSnapshot, chunk_pos: IVec3, profile: FluidProfile) -> Self {
        let mut scheduled = Self::default();
        for (pos, _) in snapshot.fluid_positions_in_chunk(chunk_pos, profile) {
            scheduled.wake_cell(pos);
        }
        scheduled
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct FluidStep {
    pub(crate) updates: Vec<FluidUpdate>,
    pub(crate) scheduled: ScheduledFluids,
}

impl FluidStep {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by the headless scene driver")
    )]
    pub(crate) fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FluidStepResult {
    pub changed: bool,
    pub boundary_changed: bool,
}

impl Chunk {
    pub fn step_fluids(&mut self, profile: &FluidProfile) -> FluidStepResult {
        let snapshot = FluidSnapshot::from_chunk(IVec3::ZERO, self);
        // Isolated single-chunk stepping wakes every fluid cell, as if the
        // chunk had just been written. The running game tracks scheduled cells
        // explicitly so settled water is not re-ticked.
        let scheduled = ScheduledFluids::all_fluids_in(&snapshot, IVec3::ZERO, *profile);
        let step = simulate_fluid_step(&snapshot, &[IVec3::ZERO], &scheduled, *profile);
        let mut result = FluidStepResult::default();
        for update in step.updates {
            let address = WorldBlockPos::from_ivec3(update.pos).split();
            if address.chunk() == ChunkPos::ZERO {
                self.set_cell(address.local().as_uvec3(), update.cell);
                result.changed = true;
                result.boundary_changed |= address.local().is_boundary();
            }
        }

        result
    }
}

pub(crate) fn simulate_fluid_step(
    snapshot: &FluidSnapshot,
    source_chunks: &[IVec3],
    scheduled: &ScheduledFluids,
    profile: FluidProfile,
) -> FluidStep {
    let mut cleared = HashSet::new();
    let mut next_fluids = HashMap::new();
    let mut own_states = Vec::new();
    let mut next_scheduled = ScheduledFluids::default();

    // Phase 1 mirrors FlowingFluid.getNewLiquid: every scheduled fluid cell
    // recomputes its own state from its surroundings. Unscheduled water is
    // settled water; vanilla leaves it alone until a neighbouring change
    // schedules it, so it must not self-heal from this batch either.
    for &chunk_pos in source_chunks {
        for (pos, fluid) in snapshot.fluid_positions_in_chunk(chunk_pos, profile) {
            if !scheduled.contains(pos) {
                continue;
            }
            cleared.insert(pos);
            let Some(state) = recompute_fluid(snapshot, pos, fluid, profile) else {
                // The cell dried up: its neighbours' surroundings changed.
                next_scheduled.wake_with_neighbours(pos);
                continue;
            };
            write_next_fluid(snapshot, &mut next_fluids, profile, pos, state);
            if state != fluid {
                // A changed cell reschedules itself and notifies its neighbours.
                next_scheduled.wake_with_neighbours(pos);
            }
            own_states.push((pos, state));
        }
    }

    // Scheduled cells outside this batch keep their place, so wakes that cross
    // a chunk the budget deferred are not lost. Wakes for cells that no longer
    // hold this fluid are spent and dropped.
    let processed_chunks = source_chunks.iter().copied().collect::<HashSet<_>>();
    for pos in scheduled.iter() {
        let chunk_pos = WorldBlockPos::from_ivec3(pos).split().chunk().as_ivec3();
        if processed_chunks.contains(&chunk_pos) {
            continue;
        }
        let holds_fluid = snapshot.cell(pos).is_none_or(|cell| {
            cell.as_fluid()
                .is_some_and(|fluid| fluid.ty() == profile.ty)
        });
        if holds_fluid {
            next_scheduled.wake_cell(pos);
        }
    }

    // Phase 2 mirrors FlowingFluid.spread: pour downward first; otherwise (or
    // additionally, when three or more sources surround the cell) spread
    // sideways.
    for (pos, state) in own_states {
        spread_fluid(
            snapshot,
            &mut next_fluids,
            &mut next_scheduled,
            profile,
            pos,
            state,
        );
    }

    let mut changed_positions = cleared;
    changed_positions.extend(next_fluids.keys().copied());

    let mut updates = Vec::new();
    for pos in changed_positions {
        let Some(old_cell) = snapshot.cell(pos) else {
            continue;
        };
        let new_cell = next_fluids
            .get(&pos)
            .copied()
            .map(ChunkCell::fluid)
            .unwrap_or(ChunkCell::EMPTY);
        if old_cell != new_cell {
            updates.push(FluidUpdate {
                pos,
                cell: new_cell,
            });
        }
    }

    updates.sort_by_key(|update| (update.pos.x, update.pos.y, update.pos.z));
    FluidStep {
        updates,
        scheduled: next_scheduled,
    }
}

/// Recomputes one existing fluid cell, following `getNewLiquid` order.
///
/// Returns `None` when the cell dries up.
fn recompute_fluid(
    snapshot: &FluidSnapshot,
    pos: IVec3,
    fluid: FluidState,
    profile: FluidProfile,
) -> Option<FluidState> {
    if fluid.is_source() {
        return Some(profile.source());
    }

    // A cell under the same fluid is falling, no matter how it was fed.
    if has_same_fluid_above(snapshot, pos, profile) {
        return Some(profile.falling());
    }

    // Two horizontal sources plus replaceable support convert a flowing cell
    // back into a source. Java Edition only has this two-source rule; the
    // one-source-plus-source-above variant is Bedrock behaviour.
    if profile.creates_sources
        && horizontal_source_neighbors(snapshot, pos, profile) >= 2
        && source_conversion_support(snapshot, pos, profile)
    {
        return Some(profile.source());
    }

    // The level is one below the fullest reachable horizontal neighbour; with
    // no neighbours the cell drains away.
    let highest = highest_horizontal_fluid_level(snapshot, pos, profile)?;
    let next_level = highest.checked_sub(profile.horizontal_decay.get())?;
    profile.flowing_level(next_level)
}

/// Spreads one recomputed fluid cell into the surrounding cells.
fn spread_fluid(
    snapshot: &FluidSnapshot,
    next_fluids: &mut HashMap<IVec3, FluidState>,
    next_scheduled: &mut ScheduledFluids,
    profile: FluidProfile,
    pos: IVec3,
    state: FluidState,
) {
    let sideways = profile.decayed_flow(state);

    if can_pour_down(snapshot, pos) {
        if write_next_fluid(
            snapshot,
            next_fluids,
            profile,
            pos + IVec3::NEG_Y,
            profile.falling(),
        ) {
            next_scheduled.wake_with_neighbours(pos + IVec3::NEG_Y);
        }
        // Java also spreads sideways from a cell that is pouring when it has
        // three or more horizontal source neighbours (wide pools spill over
        // their lip).
        if horizontal_source_neighbors(snapshot, pos, profile) >= 3 {
            write_sideways(
                snapshot,
                next_fluids,
                next_scheduled,
                profile,
                pos,
                sideways,
            );
        }
        return;
    }

    // With the pour blocked: a source always spreads sideways, while flowing
    // water over a water hole stays put.
    if state.is_source() || !is_water_hole(snapshot, pos) {
        write_sideways(
            snapshot,
            next_fluids,
            next_scheduled,
            profile,
            pos,
            sideways,
        );
    }
}

fn write_next_fluid(
    snapshot: &FluidSnapshot,
    next_fluids: &mut HashMap<IVec3, FluidState>,
    profile: FluidProfile,
    pos: IVec3,
    fluid: FluidState,
) -> bool {
    if !can_write_fluid(snapshot, pos, profile, fluid) {
        return false;
    }

    match next_fluids.get_mut(&pos) {
        Some(current) if current.is_source() => false,
        Some(current) if fluid.is_source() || fluid.level() > current.level() => {
            *current = fluid;
            true
        }
        Some(_) => false,
        None => {
            next_fluids.insert(pos, fluid);
            true
        }
    }
}

fn can_write_fluid(
    snapshot: &FluidSnapshot,
    pos: IVec3,
    profile: FluidProfile,
    fluid: FluidState,
) -> bool {
    let Some(cell) = snapshot.cell(pos) else {
        return false;
    };
    if cell.is_block() {
        return false;
    }
    match cell.as_fluid() {
        Some(existing) if existing.ty() != profile.ty => false,
        Some(existing) if existing.is_source() && !fluid.is_source() => false,
        _ => true,
    }
}

fn can_flow_into(snapshot: &FluidSnapshot, pos: IVec3, profile: FluidProfile) -> bool {
    let Some(cell) = snapshot.cell(pos) else {
        return false;
    };
    if cell.is_block() {
        return false;
    }
    cell.as_fluid()
        .is_none_or(|fluid| fluid.ty() == profile.ty && !fluid.is_source())
}

/// Vanilla pours downward only into a replaceable cell. Once the cell below
/// already holds water the pour fails until that water is gone.
fn can_pour_down(snapshot: &FluidSnapshot, pos: IVec3) -> bool {
    snapshot
        .cell(pos + IVec3::NEG_Y)
        .is_some_and(ChunkCell::is_empty)
}

/// A "water hole" is a cell below that fluid can occupy or is already in:
/// vanilla checks that the block below does not block motion.
fn is_water_hole(snapshot: &FluidSnapshot, pos: IVec3) -> bool {
    snapshot
        .cell(pos + IVec3::NEG_Y)
        .is_some_and(|cell| !cell.is_block())
}

fn has_same_fluid_above(snapshot: &FluidSnapshot, pos: IVec3, profile: FluidProfile) -> bool {
    snapshot
        .cell(pos + IVec3::Y)
        .and_then(ChunkCell::as_fluid)
        .is_some_and(|above| above.ty() == profile.ty)
}

fn horizontal_source_neighbors(
    snapshot: &FluidSnapshot,
    pos: IVec3,
    profile: FluidProfile,
) -> usize {
    HORIZONTAL_DIRS
        .iter()
        .filter(|&&dir| {
            snapshot
                .cell(pos + dir)
                .and_then(ChunkCell::as_fluid)
                .is_some_and(|fluid| fluid.ty() == profile.ty && fluid.is_source())
        })
        .count()
}

fn write_sideways(
    snapshot: &FluidSnapshot,
    next_fluids: &mut HashMap<IVec3, FluidState>,
    next_scheduled: &mut ScheduledFluids,
    profile: FluidProfile,
    pos: IVec3,
    fluid: Option<FluidState>,
) {
    let Some(fluid) = fluid else {
        return;
    };
    for dir in spread_dirs(snapshot, pos, profile) {
        let target = pos + dir;
        // Vanilla never overwrites water with water: a horizontal spread only
        // fills replaceable cells, and existing water raises or lowers itself
        // in the recompute phase instead.
        if snapshot.cell(target).is_some_and(ChunkCell::is_empty)
            && write_next_fluid(snapshot, next_fluids, profile, target, fluid)
        {
            // The write lands as a block update: the new cell and its
            // neighbours get scheduled.
            next_scheduled.wake_with_neighbours(target);
        }
    }
}

fn spread_dirs(snapshot: &FluidSnapshot, pos: IVec3, profile: FluidProfile) -> Vec<IVec3> {
    let candidates = HORIZONTAL_DIRS
        .iter()
        .copied()
        .filter(|dir| can_flow_into(snapshot, pos + *dir, profile))
        .collect::<Vec<_>>();
    if candidates.len() <= 1 {
        return candidates;
    }

    let distances = candidates
        .iter()
        .copied()
        .map(|dir| (dir, nearest_drop_distance(snapshot, pos + dir, profile)))
        .collect::<Vec<_>>();
    let best = distances.iter().filter_map(|(_, distance)| *distance).min();

    match best {
        Some(best) => distances
            .into_iter()
            .filter_map(|(dir, distance)| (distance == Some(best)).then_some(dir))
            .collect(),
        None => candidates,
    }
}

fn nearest_drop_distance(
    snapshot: &FluidSnapshot,
    start: IVec3,
    profile: FluidProfile,
) -> Option<u8> {
    if is_water_hole(snapshot, start) {
        return Some(0);
    }

    let mut visited = HashSet::from([start]);
    let mut queue = VecDeque::from([(start, 0)]);

    while let Some((pos, distance)) = queue.pop_front() {
        if distance >= MAX_DROP_SEARCH_DISTANCE {
            continue;
        }

        for dir in HORIZONTAL_DIRS {
            let next = pos + dir;
            if !visited.insert(next) || !can_flow_into(snapshot, next, profile) {
                continue;
            }
            if is_water_hole(snapshot, next) {
                return Some(distance + 1);
            }
            queue.push_back((next, distance + 1));
        }
    }

    None
}

/// Returns the level of the fullest horizontal same-fluid neighbour.
///
/// Sources count as level 8, matching vanilla `getNewLiquid`.
fn highest_horizontal_fluid_level(
    snapshot: &FluidSnapshot,
    pos: IVec3,
    profile: FluidProfile,
) -> Option<u8> {
    HORIZONTAL_DIRS
        .iter()
        .filter_map(|&dir| {
            snapshot
                .cell(pos + dir)
                .and_then(ChunkCell::as_fluid)
                .filter(|fluid| fluid.ty() == profile.ty)
                .map(|fluid| fluid.level().get())
        })
        .max()
}

/// Vanilla converts a flowing cell into a source when the block below is solid
/// or is itself a source of the same fluid.
fn source_conversion_support(
    snapshot: &FluidSnapshot,
    pos: IVec3,
    profile: FluidProfile,
) -> bool {
    let Some(cell) = snapshot.cell(pos + IVec3::NEG_Y) else {
        return false;
    };

    cell.is_block()
        || cell
            .as_fluid()
            .is_some_and(|fluid| fluid.ty() == profile.ty && fluid.is_source())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Item;

    #[derive(Default)]
    struct TestFluidWorld {
        chunks: HashMap<IVec3, Chunk>,
        scheduled: ScheduledFluids,
    }

    impl TestFluidWorld {
        fn fill_floor(&mut self, chunk_pos: IVec3) {
            let chunk = self.chunks.entry(chunk_pos).or_default();
            for x in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    chunk.set_cell_xyz(x, 0, z, Item::Stone.into());
                }
            }
        }

        fn set_cell(&mut self, pos: IVec3, cell: ChunkCell) {
            let address = WorldBlockPos::from_ivec3(pos).split();
            let chunk = self.chunks.entry(address.chunk().as_ivec3()).or_default();
            chunk.set_cell(address.local().as_uvec3(), cell);
            // Test edits stand in for block changes: they notify the cell and
            // its neighbours, exactly like the real invalidation path.
            self.scheduled.wake_with_neighbours(pos);
        }

        fn cell(&self, pos: IVec3) -> Option<ChunkCell> {
            let address = WorldBlockPos::from_ivec3(pos).split();
            self.chunks
                .get(&address.chunk().as_ivec3())
                .map(|chunk| chunk.get_cell(address.local().as_uvec3()))
        }

        fn has_fluids(&self) -> bool {
            self.chunks
                .values()
                .any(|chunk| chunk.compute_content_counts().fluids > 0)
        }

        fn step(&mut self) -> FluidStep {
            let snapshot = self.snapshot();
            let mut source_chunks = self
                .chunks
                .iter()
                .filter_map(|(&pos, chunk)| {
                    (chunk.compute_content_counts().fluids > 0).then_some(pos)
                })
                .collect::<Vec<_>>();
            source_chunks.sort_by_key(|pos| (pos.x, pos.y, pos.z));

            let step = simulate_fluid_step(
                &snapshot,
                &source_chunks,
                &self.scheduled,
                FluidProfile::WATER,
            );
            self.scheduled = step.scheduled.clone();
            self.apply(&step);
            step
        }

        fn snapshot(&self) -> FluidSnapshot {
            FluidSnapshot::new(
                self.chunks
                    .iter()
                    .map(|(&pos, chunk)| (pos, Box::new(chunk.to_cell_buffer())))
                    .collect(),
            )
        }

        fn apply(&mut self, step: &FluidStep) {
            for update in &step.updates {
                let address = WorldBlockPos::from_ivec3(update.pos).split();
                let Some(chunk) = self.chunks.get_mut(&address.chunk().as_ivec3()) else {
                    continue;
                };
                chunk.set_cell(address.local().as_uvec3(), update.cell);
            }
        }
    }

    #[test]
    fn source_creation_promotes_flow_between_two_sources() {
        let mut world = TestFluidWorld::default();
        world.fill_floor(IVec3::ZERO);
        world.set_cell(ivec3(7, 1, 8), ChunkCell::water_source());
        world.set_cell(ivec3(9, 1, 8), ChunkCell::water_source());

        // The gap is filled on the first step, then converts on its own next
        // update, matching vanilla's placed-fluid scheduling.
        world.step();
        assert_eq!(world.cell(ivec3(8, 1, 8)), Some(ChunkCell::water_flow(7)));
        world.step();

        assert_eq!(world.cell(ivec3(8, 1, 8)), Some(ChunkCell::water_source()));
    }

    #[test]
    fn unscheduled_water_neither_self_heals_nor_spreads() {
        let mut world = TestFluidWorld::default();
        world.fill_floor(IVec3::ZERO);
        world.set_cell(ivec3(8, 1, 8), ChunkCell::water_source());
        // A stale level with no pending tick must stay exactly as stored.
        world.set_cell(ivec3(7, 1, 8), ChunkCell::water_flow(3));

        let snapshot = world.snapshot();
        let idle = simulate_fluid_step(
            &snapshot,
            &[IVec3::ZERO],
            &ScheduledFluids::default(),
            FluidProfile::WATER,
        );

        assert!(
            idle.is_empty(),
            "water without a pending tick must neither self-heal nor spread"
        );
        assert!(idle.scheduled.is_empty());
    }

    #[test]
    fn fluid_writes_reschedule_the_written_cell() {
        let mut world = TestFluidWorld::default();
        world.fill_floor(IVec3::ZERO);
        world.set_cell(ivec3(8, 1, 8), ChunkCell::water_source());

        let snapshot = world.snapshot();
        let mut scheduled = ScheduledFluids::default();
        scheduled.wake_cell(ivec3(8, 1, 8));
        let step = simulate_fluid_step(&snapshot, &[IVec3::ZERO], &scheduled, FluidProfile::WATER);

        assert_eq!(step.updates.len(), 4, "the source spreads into four sides");
        for side in [
            ivec3(7, 1, 8),
            ivec3(9, 1, 8),
            ivec3(8, 1, 7),
            ivec3(8, 1, 9),
        ] {
            assert!(
                step.scheduled.contains(side),
                "written cell {side} must stay scheduled"
            );
        }
        assert!(
            !step.scheduled.contains(ivec3(8, 2, 8)),
            "untouched cells beyond the write must not be scheduled"
        );
    }

    #[test]
    fn water_flows_across_loaded_chunk_boundary() {
        let mut world = TestFluidWorld::default();
        world.fill_floor(IVec3::ZERO);
        world.fill_floor(IVec3::X);
        world.set_cell(ivec3(15, 1, 8), ChunkCell::water_source());

        world.step();

        assert_eq!(world.cell(ivec3(16, 1, 8)), Some(ChunkCell::water_flow(7)));
    }

    #[test]
    fn removed_source_drains_unreplenished_flow() {
        let mut world = TestFluidWorld::default();
        world.fill_floor(IVec3::ZERO);
        world.set_cell(ivec3(8, 1, 8), ChunkCell::water_source());
        for _ in 0..4 {
            world.step();
        }
        assert!(world.has_fluids());

        world.set_cell(ivec3(8, 1, 8), ChunkCell::EMPTY);
        for _ in 0..10 {
            world.step();
        }

        assert!(!world.has_fluids());
    }

    #[test]
    fn blocked_flow_reflows_to_newly_available_route() {
        let mut world = TestFluidWorld::default();
        world.fill_floor(IVec3::ZERO);
        world.set_cell(ivec3(8, 1, 8), ChunkCell::water_source());
        world.set_cell(ivec3(7, 1, 8), Item::Stone.into());
        world.set_cell(ivec3(8, 1, 7), Item::Stone.into());
        world.set_cell(ivec3(8, 1, 9), Item::Stone.into());

        world.step();
        assert_eq!(world.cell(ivec3(9, 1, 8)), Some(ChunkCell::water_flow(7)));

        world.set_cell(ivec3(9, 1, 8), Item::Stone.into());
        world.set_cell(ivec3(8, 1, 7), ChunkCell::EMPTY);
        world.step();

        assert_eq!(world.cell(ivec3(9, 1, 8)), Some(Item::Stone.into()));
        assert_eq!(world.cell(ivec3(8, 1, 7)), Some(ChunkCell::water_flow(7)));
    }

    #[test]
    fn falling_column_descends_one_cell_per_step() {
        let mut world = TestFluidWorld::default();
        let shaft = |y: i32| ivec3(8, y, 8);

        // A one-cell shaft above a stone floor with the source on top; every
        // side is sealed so the only thing the test can observe is the descent.
        for y in 1..=5 {
            world.set_cell(ivec3(7, y, 8), Item::Stone.into());
            world.set_cell(ivec3(9, y, 8), Item::Stone.into());
            world.set_cell(ivec3(8, y, 7), Item::Stone.into());
            world.set_cell(ivec3(8, y, 9), Item::Stone.into());
        }
        world.set_cell(ivec3(8, 0, 8), Item::Stone.into());
        world.set_cell(shaft(5), ChunkCell::water_source());

        // One step is five game ticks, so the leading edge should advance
        // exactly one block per step, matching vanilla's measured cadence.
        for step in 1..=4 {
            world.step();
            let tip_y = 5 - step;
            assert_eq!(
                world.cell(shaft(tip_y)),
                Some(ChunkCell::water_flow(8)),
                "falling column should reach y={tip_y} after {step} steps"
            );
            if tip_y > 1 {
                assert_eq!(
                    world.cell(shaft(tip_y - 1)),
                    Some(ChunkCell::EMPTY),
                    "column should not outrun one block per step"
                );
            }
        }

        // The bottom cell rests on the floor as falling water; nothing may
        // spread sideways out of the sealed shaft.
        world.step();
        assert_eq!(world.cell(shaft(1)), Some(ChunkCell::water_flow(8)));
        for y in 1..=5 {
            assert_eq!(world.cell(ivec3(7, y, 8)), Some(Item::Stone.into()));
            assert_eq!(world.cell(ivec3(9, y, 8)), Some(Item::Stone.into()));
            assert_eq!(world.cell(ivec3(8, y, 7)), Some(Item::Stone.into()));
            assert_eq!(world.cell(ivec3(8, y, 9)), Some(Item::Stone.into()));
        }
    }
}
