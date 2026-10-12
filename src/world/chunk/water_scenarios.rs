//! Readable ASCII scenes for water-flow fidelity testing.
//!
//! Water bugs are easy to hide in numeric grids, so scenarios are declared as a
//! stack of small text maps: both the setup and the expected final state can be
//! reviewed by eye in the test source. Every scene is a complete little world:
//! a floor, walls, one or more water sources, and nothing else. The driver ticks
//! the production fluid step until the cell state stops changing, then the final
//! state is compared cell by cell against the hand-written expectation.
//!
//! Fluids only step every fifth game tick in the running game, so one driver
//! step here is one game fluid update (five game ticks).
//!
//! Character legend:
//! - `.` air
//! - `#` stone
//! - `G` glass
//! - `L` oak log
//! - `S` water source (level 8)
//! - `1`..`7` flowing water level
//! - `8` falling water (flowing level 8)
//!
//! Layout: `layers[y]` is the horizontal slice at height `y`, `layers[y][z]` is
//! the row at depth `z`, and each row's characters run along `x`. Every layer
//! needs the same number of rows and every row the same width. Row 0 is `z = 0`
//! and layer 0 is `y = 0`; scenes start at world origin unless built with
//! [`FluidSceneWorld::from_layers_at`].

use std::collections::HashMap;

use bevy::prelude::*;

use crate::item::Item;

use super::{
    Chunk, ChunkCell, ChunkPos, FluidProfile, FluidSnapshot, WorldBlockPos, simulate_fluid_step,
};

pub(crate) type SceneLayers = [&'static [&'static str]];

fn cell_from_char(c: char) -> ChunkCell {
    match c {
        '.' | ' ' => ChunkCell::EMPTY,
        '#' => ChunkCell::block(Item::Stone),
        'G' => ChunkCell::block(Item::Glass),
        'L' => ChunkCell::block(Item::OakLog),
        'S' => ChunkCell::water_source(),
        '1'..='7' => ChunkCell::water_flow(c.to_digit(10).expect("digit") as u8),
        '8' => ChunkCell::fluid(FluidProfile::WATER.falling()),
        other => panic!("unknown scene character {other:?}"),
    }
}

fn char_from_cell(cell: ChunkCell) -> char {
    match cell {
        ChunkCell::Empty => '.',
        ChunkCell::Block(Item::Stone) => '#',
        ChunkCell::Block(Item::Glass) => 'G',
        ChunkCell::Block(Item::OakLog) => 'L',
        ChunkCell::Block(other) => panic!("scene renderer has no character for {other:?}"),
        ChunkCell::Fluid(fluid) if fluid.is_source() => 'S',
        ChunkCell::Fluid(fluid) => char::from(b'0' + fluid.level().get()),
    }
}

/// A tiny multi-chunk world that can only be driven by the fluid solver.
pub(crate) struct FluidSceneWorld {
    chunks: HashMap<IVec3, Chunk>,
    origin: IVec3,
    size: IVec3,
}

impl FluidSceneWorld {
    pub(crate) fn from_layers(layers: &SceneLayers) -> Self {
        Self::from_layers_at(IVec3::ZERO, layers)
    }

    pub(crate) fn from_layers_at(origin: IVec3, layers: &SceneLayers) -> Self {
        assert!(!layers.is_empty(), "a scene needs at least one layer");

        let depth = layers[0].len();
        let width = layers[0].first().map_or(0, |row| row.chars().count());
        assert!(width > 0, "scene rows must not be empty");
        assert!(depth > 0, "scene layers must not be empty");

        for layer in layers {
            assert_eq!(
                layer.len(),
                depth,
                "every layer needs the same number of z rows"
            );
            for row in *layer {
                assert_eq!(
                    row.chars().count(),
                    width,
                    "every row needs the same width (offending row: {row:?})"
                );
            }
        }

        let size = IVec3::new(width as i32, layers.len() as i32, depth as i32);
        let mut world = Self {
            chunks: HashMap::new(),
            origin,
            size,
        };

        for (y, layer) in layers.iter().enumerate() {
            for (z, row) in layer.iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    let cell = cell_from_char(c);
                    if !cell.is_empty() {
                        world.set_cell(world.world_pos(x, y, z), cell);
                    }
                }
            }
        }

        world.seal();

        world
    }

    /// Walls the scene in with a one-block stone shell.
    ///
    /// Scenes should describe only the cells under review; without a shell
    /// water would silently escape sideways along the open chunk and fall out
    /// of the world, which reads as a mysterious test failure. The shell keeps
    /// the reviewed box self-contained.
    fn seal(&mut self) {
        for y in -1..=self.size.y {
            for z in -1..=self.size.z {
                for x in -1..=self.size.x {
                    let on_shell = x == -1
                        || y == -1
                        || z == -1
                        || x == self.size.x
                        || y == self.size.y
                        || z == self.size.z;
                    if on_shell {
                        self.set_cell(self.world_pos_signed(x, y, z), Item::Stone.into());
                    }
                }
            }
        }
    }

    pub(crate) fn set_cell(&mut self, world: IVec3, cell: ChunkCell) {
        let address = WorldBlockPos::from_ivec3(world).split();
        let chunk = self.chunks.entry(address.chunk().as_ivec3()).or_default();
        chunk.set_cell(address.local().as_uvec3(), cell);
    }

    /// Reads a scene-local cell; positions outside the scene read as air.
    pub(crate) fn cell(&self, x: usize, y: usize, z: usize) -> ChunkCell {
        self.cell_at(self.world_pos(x, y, z))
    }

    pub(crate) fn cell_at(&self, world: IVec3) -> ChunkCell {
        let address = WorldBlockPos::from_ivec3(world).split();
        self.chunks
            .get(&address.chunk().as_ivec3())
            .map_or(ChunkCell::EMPTY, |chunk| {
                chunk.get_cell(address.local().as_uvec3())
            })
    }

    pub(crate) fn world_pos(&self, x: usize, y: usize, z: usize) -> IVec3 {
        self.world_pos_signed(x as i32, y as i32, z as i32)
    }

    fn world_pos_signed(&self, x: i32, y: i32, z: i32) -> IVec3 {
        self.origin + IVec3::new(x, y, z)
    }

    fn snapshot(&self) -> FluidSnapshot {
        FluidSnapshot::new(
            self.chunks
                .iter()
                .map(|(&pos, chunk)| (pos, Box::new(chunk.to_cell_buffer())))
                .collect(),
        )
    }

    /// Runs one fluid update step (what the game does every fifth game tick).
    ///
    /// Returns `true` when any cell changed.
    pub(crate) fn step(&mut self) -> bool {
        let snapshot = self.snapshot();
        let mut source_chunks = self
            .chunks
            .iter()
            .filter_map(|(&pos, chunk)| (chunk.compute_content_counts().fluids > 0).then_some(pos))
            .collect::<Vec<_>>();
        source_chunks.sort_by_key(|pos| (pos.x, pos.y, pos.z));

        let step = simulate_fluid_step(&snapshot, &source_chunks, FluidProfile::WATER);
        if step.is_empty() {
            return false;
        }

        for update in &step.updates {
            let address = WorldBlockPos::from_ivec3(update.pos).split();
            if let Some(chunk) = self.chunks.get_mut(&address.chunk().as_ivec3()) {
                chunk.set_cell(address.local().as_uvec3(), update.cell);
            }
        }

        true
    }

    /// Steps until the cell state stops changing and returns the number of
    /// mutating steps it took. Panics if the scene never settles.
    pub(crate) fn settle(&mut self, max_steps: usize) -> usize {
        for steps in 0..max_steps {
            if !self.step() {
                return steps;
            }
        }

        panic!(
            "water did not reach a fixpoint within {max_steps} fluid steps; current state:\n{}",
            self.render()
        );
    }

    pub(crate) fn render(&self) -> String {
        let mut out = String::new();
        for y in 0..self.size.y as usize {
            out.push_str(&format!("y={y}:\n"));
            for z in 0..self.size.z as usize {
                let row = (0..self.size.x as usize)
                    .map(|x| char_from_cell(self.cell(x, y, z)))
                    .collect::<String>();
                out.push_str(&format!("  z={z}: {row}\n"));
            }
        }
        out
    }

    /// Compares the settled world against an expected scene of the same shape.
    ///
    /// Reports every mismatching row plus any fluid that leaked outside the
    /// declared scene bounds, so tests fail loudly instead of silently ignoring
    /// water that floods past the reviewed area.
    pub(crate) fn diff(&self, expected: &SceneLayers) -> Result<(), String> {
        let expected_size = IVec3::new(
            expected
                .first()
                .and_then(|layer| layer.first())
                .map_or(0, |row| row.chars().count()) as i32,
            expected.len() as i32,
            expected.first().map_or(0, |layer| layer.len()) as i32,
        );
        if expected_size != self.size {
            return Err(format!(
                "expected scene size {expected_size:?} does not match scene size {:?}",
                self.size
            ));
        }

        let mut problems = Vec::new();

        for (y, layer) in expected.iter().enumerate() {
            for (z, row) in layer.iter().enumerate() {
                let actual = (0..self.size.x as usize)
                    .map(|x| char_from_cell(self.cell(x, y, z)))
                    .collect::<String>();
                if actual != *row {
                    problems.push(format!(
                        "y={y} z={z} (world {:?}..):\n  actual  : {actual}\n  expected: {row}",
                        self.world_pos(0, y, z)
                    ));
                }
            }
        }

        let mut leaks = Vec::new();
        for (&chunk_pos, chunk) in &self.chunks {
            for (cell, local) in chunk.iter() {
                if !cell.is_fluid() {
                    continue;
                }
                let world = ChunkPos::from_ivec3(chunk_pos)
                    .block(local)
                    .world()
                    .as_ivec3();
                let local_in_scene = world - self.origin;
                let inside = local_in_scene.cmpge(IVec3::ZERO).all()
                    && local_in_scene.cmplt(self.size).all();
                if !inside && leaks.len() < 8 {
                    leaks.push(format!("  {world} holds {}", cell.name()));
                }
            }
        }
        if !leaks.is_empty() {
            problems.push(format!(
                "fluid leaked outside the reviewed scene bounds ({} shown):\n{}",
                leaks.len(),
                leaks.join("\n")
            ));
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "{}\nactual scene:\n{}",
                problems.join("\n"),
                self.render()
            ))
        }
    }
}

/// Settles a scene and asserts its full final state.
pub(crate) fn settle_and_assert(world: &mut FluidSceneWorld, expected: &SceneLayers, max_steps: usize) {
    let steps = world.settle(max_steps);
    if let Err(problem) = world.diff(expected) {
        panic!(
            "scene did not match the expected final state after {steps} settling steps:\n{problem}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_scene(input: &SceneLayers, expected: &SceneLayers, max_steps: usize) {
        let mut world = FluidSceneWorld::from_layers(input);
        settle_and_assert(&mut world, expected, max_steps);
    }

    /// One source on a flat 9x9 floor spreads into a diamond whose level is
    /// `8 - manhattan_distance`, stopping at level 1.
    #[test]
    fn source_on_flat_floor_spreads_level_diamond() {
        assert_scene(
            &[
                &["#########"; 9],
                &[
                    ".........",
                    ".........",
                    ".........",
                    ".........",
                    "....S....",
                    ".........",
                    ".........",
                    ".........",
                    ".........",
                ],
            ],
            &[
                &["#########"; 9],
                &[
                    ".1234321.",
                    "123454321",
                    "234565432",
                    "345676543",
                    "4567S7654",
                    "345676543",
                    "234565432",
                    "123454321",
                    ".1234321.",
                ],
            ],
            64,
        );
    }

    /// A 1-wide channel with a source at one end: levels step down by one per
    /// block and the last level-1 block sits seven blocks from the source.
    #[test]
    fn single_wide_channel_stops_seven_blocks_from_source() {
        assert_scene(
            &[
                &["##########"; 3],
                &["##########", "#S........", "##########"],
            ],
            &[
                &["##########"; 3],
                &["##########", "#S7654321.", "##########"],
            ],
            64,
        );
    }

    /// Two sources with one air gap over solid ground: the gap becomes a
    /// source (infinite water) and the pool keeps spreading outward.
    #[test]
    fn gap_between_two_sources_over_stone_becomes_source() {
        assert_scene(
            &[
                &["########"; 3],
                &["########", "#S.S....", "########"],
            ],
            &[
                &["########"; 3],
                &["########", "#SSS7654", "########"],
            ],
            64,
        );
    }

    /// Water prefers routes that reach a drop. With flat floor to the left and
    /// a hole to the right, the water must ignore the left side entirely.
    #[test]
    fn water_prefers_the_direction_of_a_nearby_drop() {
        assert_scene(
            &[
                &["#############"; 3],
                &[
                    "#############",
                    "######.######",
                    "#############",
                ],
                &[
                    "#############",
                    "#..S.........",
                    "#############",
                ],
            ],
            &[
                &["#############"; 3],
                &[
                    "#############",
                    "######8######",
                    "#############",
                ],
                &[
                    "#############",
                    "#..S765......",
                    "#############",
                ],
            ],
            64,
        );
    }

    /// Water falling over a ledge lands at level 8 and spreads sideways from
    /// the landing cell with the usual decay.
    #[test]
    fn falling_water_spreads_from_the_landing_cell() {
        assert_scene(
            &[
                &["#########"; 3],
                &["#########", "####.....", "#########"],
                &["#########", "#S.......", "#########"],
            ],
            &[
                &["#########"; 3],
                &["#########", "####87654", "#########"],
                &["#########", "#S765....", "#########"],
            ],
            64,
        );
    }

    /// The same channel as above but with the source near a chunk border: the
    /// flow must continue into the neighbouring chunk.
    #[test]
    fn channel_flow_crosses_chunk_boundary() {
        assert_scene(
            &[
                &["########################"; 3],
                &[
                    "########################",
                    "##############S.........",
                    "########################",
                ],
            ],
            &[
                &["########################"; 3],
                &[
                    "########################",
                    "##############S7654321..",
                    "########################",
                ],
            ],
            64,
        );
    }
}
