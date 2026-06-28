use bevy::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkCell, FluidProfile, FluidState,
    chunk_linear_index,
};

const HORIZONTAL_DIRS: [IVec3; 4] = [IVec3::NEG_X, IVec3::X, IVec3::NEG_Z, IVec3::Z];
const MAX_DROP_SEARCH_DISTANCE: u8 = 4;

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
        let (chunk_pos, local) = world_to_chunk_local(world_pos);
        self.chunks.get(&chunk_pos).map(|cells| {
            cells[chunk_linear_index(local.x as usize, local.y as usize, local.z as usize)]
        })
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
                        chunk_local_to_world(chunk_pos, uvec3(x as u32, y as u32, z as u32)),
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct FluidStep {
    pub(crate) updates: Vec<FluidUpdate>,
}

impl FluidStep {
    pub(crate) fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }
}

pub(crate) fn simulate_fluid_step(
    snapshot: &FluidSnapshot,
    source_chunks: &[IVec3],
    profile: FluidProfile,
) -> FluidStep {
    let mut cleared = HashSet::new();
    let mut next_fluids = HashMap::new();

    for &chunk_pos in source_chunks {
        for (pos, fluid) in snapshot.fluid_positions_in_chunk(chunk_pos, profile) {
            cleared.insert(pos);

            if fluid.is_source() {
                write_next_fluid(snapshot, &mut next_fluids, profile, pos, profile.source());
            }

            if can_flow_down_from(snapshot, pos, profile) {
                write_next_fluid(
                    snapshot,
                    &mut next_fluids,
                    profile,
                    pos + IVec3::NEG_Y,
                    profile.falling(),
                );
                continue;
            }

            let Some(next_fluid) = profile.decayed_flow(fluid) else {
                continue;
            };
            for dir in spread_dirs(snapshot, pos, profile) {
                write_next_fluid(snapshot, &mut next_fluids, profile, pos + dir, next_fluid);
            }
        }
    }

    promote_new_sources(snapshot, &cleared, &mut next_fluids, profile);

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
    FluidStep { updates }
}

pub(crate) fn world_to_chunk_local(world: IVec3) -> (IVec3, UVec3) {
    let chunk = ivec3(
        world.x.div_euclid(CHUNK_ISIZE),
        world.y.div_euclid(CHUNK_ISIZE),
        world.z.div_euclid(CHUNK_ISIZE),
    );
    let local = uvec3(
        world.x.rem_euclid(CHUNK_ISIZE) as u32,
        world.y.rem_euclid(CHUNK_ISIZE) as u32,
        world.z.rem_euclid(CHUNK_ISIZE) as u32,
    );
    (chunk, local)
}

fn chunk_local_to_world(chunk: IVec3, local: UVec3) -> IVec3 {
    chunk * CHUNK_ISIZE + local.as_ivec3()
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

fn can_flow_down_from(snapshot: &FluidSnapshot, pos: IVec3, profile: FluidProfile) -> bool {
    can_flow_into(snapshot, pos + IVec3::NEG_Y, profile)
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
    if can_flow_down_from(snapshot, start, profile) {
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
            if can_flow_down_from(snapshot, next, profile) {
                return Some(distance + 1);
            }
            queue.push_back((next, distance + 1));
        }
    }

    None
}

fn promote_new_sources(
    snapshot: &FluidSnapshot,
    cleared: &HashSet<IVec3>,
    next_fluids: &mut HashMap<IVec3, FluidState>,
    profile: FluidProfile,
) {
    if !profile.creates_sources {
        return;
    }

    let mut candidates = next_fluids
        .iter()
        .filter_map(|(&pos, &fluid)| (!fluid.is_source()).then_some(pos))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|pos| (pos.x, pos.y, pos.z));

    let mut promotions = Vec::new();
    for pos in candidates {
        if !is_source_supported(snapshot, cleared, next_fluids, pos, profile) {
            continue;
        }

        let source_neighbors = HORIZONTAL_DIRS
            .iter()
            .filter(|&&dir| is_source_after(snapshot, cleared, next_fluids, pos + dir, profile))
            .count();
        if source_neighbors >= 2 {
            promotions.push(pos);
            continue;
        }

        if source_neighbors >= 1
            && is_source_after(snapshot, cleared, next_fluids, pos + IVec3::Y, profile)
        {
            promotions.push(pos);
        }
    }

    for pos in promotions {
        next_fluids.insert(pos, profile.source());
    }
}

fn is_source_supported(
    snapshot: &FluidSnapshot,
    cleared: &HashSet<IVec3>,
    next_fluids: &HashMap<IVec3, FluidState>,
    pos: IVec3,
    profile: FluidProfile,
) -> bool {
    let below = pos + IVec3::NEG_Y;
    let Some(cell) = cell_after(snapshot, cleared, next_fluids, below) else {
        return false;
    };

    cell.is_block()
        || cell
            .as_fluid()
            .is_some_and(|fluid| fluid.ty() == profile.ty && fluid.is_source())
}

fn is_source_after(
    snapshot: &FluidSnapshot,
    cleared: &HashSet<IVec3>,
    next_fluids: &HashMap<IVec3, FluidState>,
    pos: IVec3,
    profile: FluidProfile,
) -> bool {
    cell_after(snapshot, cleared, next_fluids, pos)
        .and_then(ChunkCell::as_fluid)
        .is_some_and(|fluid| fluid.ty() == profile.ty && fluid.is_source())
}

fn cell_after(
    snapshot: &FluidSnapshot,
    cleared: &HashSet<IVec3>,
    next_fluids: &HashMap<IVec3, FluidState>,
    pos: IVec3,
) -> Option<ChunkCell> {
    if let Some(&fluid) = next_fluids.get(&pos) {
        return Some(ChunkCell::fluid(fluid));
    }
    if cleared.contains(&pos) {
        return Some(ChunkCell::EMPTY);
    }
    snapshot.cell(pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;

    #[derive(Default)]
    struct TestFluidWorld {
        chunks: HashMap<IVec3, Chunk>,
    }

    impl TestFluidWorld {
        fn fill_floor(&mut self, chunk_pos: IVec3) {
            let chunk = self.chunks.entry(chunk_pos).or_default();
            for x in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    chunk.set_cell_xyz(x, 0, z, BlockType::Stone.into());
                }
            }
        }

        fn set_cell(&mut self, pos: IVec3, cell: ChunkCell) {
            let (chunk_pos, local) = world_to_chunk_local(pos);
            let chunk = self.chunks.entry(chunk_pos).or_default();
            chunk.set_cell(local, cell);
        }

        fn cell(&self, pos: IVec3) -> Option<ChunkCell> {
            let (chunk_pos, local) = world_to_chunk_local(pos);
            self.chunks
                .get(&chunk_pos)
                .map(|chunk| chunk.get_cell(local))
        }

        fn has_fluids(&self) -> bool {
            self.chunks.values().any(Chunk::has_fluids)
        }

        fn step(&mut self) -> FluidStep {
            let snapshot = self.snapshot();
            let mut source_chunks = self
                .chunks
                .iter()
                .filter_map(|(&pos, chunk)| chunk.has_fluids().then_some(pos))
                .collect::<Vec<_>>();
            source_chunks.sort_by_key(|pos| (pos.x, pos.y, pos.z));

            let step = simulate_fluid_step(&snapshot, &source_chunks, FluidProfile::WATER);
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
                let (chunk_pos, local) = world_to_chunk_local(update.pos);
                let Some(chunk) = self.chunks.get_mut(&chunk_pos) else {
                    continue;
                };
                chunk.set_cell(local, update.cell);
            }
        }
    }

    #[test]
    fn source_creation_promotes_flow_between_two_sources() {
        let mut world = TestFluidWorld::default();
        world.fill_floor(IVec3::ZERO);
        world.set_cell(ivec3(7, 1, 8), ChunkCell::water_source());
        world.set_cell(ivec3(9, 1, 8), ChunkCell::water_source());

        world.step();

        assert_eq!(world.cell(ivec3(8, 1, 8)), Some(ChunkCell::water_source()));
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
        world.set_cell(ivec3(7, 1, 8), BlockType::Stone.into());
        world.set_cell(ivec3(8, 1, 7), BlockType::Stone.into());
        world.set_cell(ivec3(8, 1, 9), BlockType::Stone.into());

        world.step();
        assert_eq!(world.cell(ivec3(9, 1, 8)), Some(ChunkCell::water_flow(7)));

        world.set_cell(ivec3(9, 1, 8), BlockType::Stone.into());
        world.set_cell(ivec3(8, 1, 7), ChunkCell::EMPTY);
        world.step();

        assert_eq!(world.cell(ivec3(9, 1, 8)), Some(BlockType::Stone.into()));
        assert_eq!(world.cell(ivec3(8, 1, 7)), Some(ChunkCell::water_flow(7)));
    }
}
