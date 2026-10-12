//! Readable ASCII scenes for water-flow fidelity testing.
//!
//! Water bugs are easy to hide in numeric grids, so scenarios are declared as a
//! stack of small text maps: both the setup and the expected final state can be
//! reviewed by eye in the test source. Every scene is a complete little world:
//! a floor, walls, one or more water sources, and nothing else. The headless
//! driver ticks the production fluid step until the cell state stops changing,
//! then the final state is compared cell by cell against the hand-written
//! expectation. The in-game runner (`crate::game_test`) builds the same scenes
//! into the live world and compares the settled chunks the same way.
//!
//! Fluids only step every fifth game tick in the running game, so one driver
//! step is one game fluid update (five game ticks).
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
//! [`layout_scene`].

use bevy::prelude::*;

use crate::item::Item;

use super::{ChunkCell, FluidProfile};

pub(crate) type SceneLayers = [&'static [&'static str]];

/// A named water scenario shared by the headless tests and the in-game runner.
pub(crate) struct WaterScenario {
    pub(crate) name: &'static str,
    /// First cell of the scene; usually `IVec3::ZERO`.
    pub(crate) origin: IVec3,
    /// Scene before any fluid step.
    pub(crate) input: &'static SceneLayers,
    /// Expected final state once the scene stops changing.
    pub(crate) expected: &'static SceneLayers,
    /// Maximum fluid steps the scene may take before it must have settled.
    pub(crate) max_steps: u32,
    /// Why this scenario exists and what vanilla behaviour it pins down.
    pub(crate) description: &'static str,
}

/// The registered fidelity scenarios, in run order.
pub(crate) const WATER_SCENARIOS: &[WaterScenario] = &[
    WaterScenario {
        name: "flat_floor_diamond",
        origin: IVec3::ZERO,
        max_steps: 64,
        input: &[
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
        expected: &[
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
        description: "One source on a flat 9x9 floor spreads into a diamond whose \
                      level is 8 - manhattan distance, stopping at level 1.",
    },
    WaterScenario {
        name: "channel_seven_block_range",
        origin: IVec3::ZERO,
        max_steps: 64,
        input: &[
            &["##########"; 3],
            &["##########", "#S........", "##########"],
        ],
        expected: &[
            &["##########"; 3],
            &["##########", "#S7654321.", "##########"],
        ],
        description: "A 1-wide channel with a source at one end: levels step down by \
                      one per block and the last level-1 block sits seven blocks from \
                      the source.",
    },
    WaterScenario {
        name: "infinite_source_gap_over_stone",
        origin: IVec3::ZERO,
        max_steps: 64,
        input: &[
            &["########"; 3],
            &["########", "#S.S....", "########"],
        ],
        expected: &[
            &["########"; 3],
            &["########", "#SSS7654", "########"],
        ],
        description: "Two sources with one air gap over solid ground: the gap becomes \
                      a source (infinite water) and the pool keeps spreading outward.",
    },
    WaterScenario {
        name: "nearest_drop_preference",
        origin: IVec3::ZERO,
        max_steps: 64,
        input: &[
            &["#############"; 3],
            &["#############", "######.######", "#############"],
            &["#############", "#..S.........", "#############"],
        ],
        expected: &[
            &["#############"; 3],
            &["#############", "######8######", "#############"],
            &["#############", "#..S765......", "#############"],
        ],
        description: "Water prefers routes that reach a drop. With flat floor to the \
                      left and a hole to the right, the water must ignore the left side \
                      entirely.",
    },
    WaterScenario {
        name: "falling_water_landing_spread",
        origin: IVec3::ZERO,
        max_steps: 64,
        input: &[
            &["#########"; 3],
            &["#########", "####.....", "#########"],
            &["#########", "#S.......", "#########"],
        ],
        expected: &[
            &["#########"; 3],
            &["#########", "####87654", "#########"],
            &["#########", "#S765....", "#########"],
        ],
        description: "Water falling over a ledge lands at level 8 and spreads sideways \
                      from the landing cell with the usual decay.",
    },
    WaterScenario {
        name: "cross_chunk_channel_flow",
        origin: IVec3::ZERO,
        max_steps: 64,
        input: &[
            &["########################"; 3],
            &[
                "########################",
                "##############S.........",
                "########################",
            ],
        ],
        expected: &[
            &["########################"; 3],
            &[
                "########################",
                "##############S7654321..",
                "########################",
            ],
        ],
        description: "The same channel as above but with the source near a chunk \
                      border: the flow must continue into the neighbouring chunk.",
    },
];

/// A scene expanded into world-space cells, ready for either driver.
///
/// The layout includes the one-block stone shell that seals the reviewed box
/// (see [`layout_scene`]).
pub(crate) struct SceneLayout {
    pub(crate) origin: IVec3,
    pub(crate) size: IVec3,
    pub(crate) cells: Vec<(IVec3, ChunkCell)>,
}

/// Expands scene text into world-space cells at `origin`.
///
/// The scene is walled in with a one-block stone shell on every side except the
/// top. Scenes describe only the cells under review; without a shell water
/// would silently escape sideways along the open chunk and read as a mysterious
/// test failure. The top stays open so a developer can look into the scene from
/// above; water never flows upward.
pub(crate) fn layout_scene(origin: IVec3, layers: &SceneLayers) -> SceneLayout {
    let size = scene_size(layers);
    let mut cells = Vec::new();

    for (y, layer) in layers.iter().enumerate() {
        for (z, row) in layer.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                let cell = cell_from_char(c);
                if !cell.is_empty() {
                    cells.push((origin + IVec3::new(x as i32, y as i32, z as i32), cell));
                }
            }
        }
    }

    for y in -1..size.y {
        for z in -1..=size.z {
            for x in -1..=size.x {
                let on_shell = x == -1 || y == -1 || z == -1 || x == size.x || z == size.z;
                if on_shell {
                    cells.push((origin + IVec3::new(x, y, z), Item::Stone.into()));
                }
            }
        }
    }

    SceneLayout {
        origin,
        size,
        cells,
    }
}

/// Validates the layer shape and returns `(width, height, depth)`.
pub(crate) fn scene_size(layers: &SceneLayers) -> IVec3 {
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

    IVec3::new(width as i32, layers.len() as i32, depth as i32)
}

pub(crate) fn cell_from_char(c: char) -> ChunkCell {
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

pub(crate) fn char_from_cell(cell: ChunkCell) -> char {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::chunk::{
        Chunk, ChunkPos, FluidSnapshot, WorldBlockPos, simulate_fluid_step,
    };
    use std::collections::HashMap;

    /// A tiny multi-chunk world that can only be driven by the fluid solver.
    struct FluidSceneWorld {
        chunks: HashMap<IVec3, Chunk>,
        origin: IVec3,
        size: IVec3,
    }

    impl FluidSceneWorld {
        fn from_layout(layout: &SceneLayout) -> Self {
            let mut world = Self {
                chunks: HashMap::new(),
                origin: layout.origin,
                size: layout.size,
            };
            for &(pos, cell) in &layout.cells {
                world.set_cell(pos, cell);
            }
            world
        }

        fn set_cell(&mut self, world: IVec3, cell: ChunkCell) {
            let address = WorldBlockPos::from_ivec3(world).split();
            let chunk = self.chunks.entry(address.chunk().as_ivec3()).or_default();
            chunk.set_cell(address.local().as_uvec3(), cell);
        }

        fn cell_at(&self, world: IVec3) -> ChunkCell {
            let address = WorldBlockPos::from_ivec3(world).split();
            self.chunks
                .get(&address.chunk().as_ivec3())
                .map_or(ChunkCell::EMPTY, |chunk| {
                    chunk.get_cell(address.local().as_uvec3())
                })
        }

        fn snapshot(&self) -> FluidSnapshot {
            FluidSnapshot::new(
                self.chunks
                    .iter()
                    .map(|(&pos, chunk)| (pos, Box::new(chunk.to_cell_buffer())))
                    .collect(),
            )
        }

        /// Runs one fluid update step (what the game does every fifth tick).
        fn step(&mut self) -> bool {
            let snapshot = self.snapshot();
            let mut source_chunks = self
                .chunks
                .iter()
                .filter_map(|(&pos, chunk)| {
                    (chunk.compute_content_counts().fluids > 0).then_some(pos)
                })
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

        fn settle(&mut self, max_steps: usize) -> usize {
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

        fn render(&self) -> String {
            let mut out = String::new();
            for y in 0..self.size.y as usize {
                out.push_str(&format!("y={y}:\n"));
                for z in 0..self.size.z as usize {
                    let row = (0..self.size.x as usize)
                        .map(|x| {
                            let local = IVec3::new(x as i32, y as i32, z as i32);
                            char_from_cell(self.cell_at(self.origin + local))
                        })
                        .collect::<String>();
                    out.push_str(&format!("  z={z}: {row}\n"));
                }
            }
            out
        }

        /// Compares the settled world against an expected scene of the same
        /// shape, reporting every mismatching row plus any fluid that leaked
        /// outside the declared scene bounds.
        fn diff(&self, expected: &SceneLayers) -> Result<(), String> {
            let expected_size = scene_size(expected);
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
                        .map(|x| {
                            let local = IVec3::new(x as i32, y as i32, z as i32);
                            char_from_cell(self.cell_at(self.origin + local))
                        })
                        .collect::<String>();
                    if actual != *row {
                        problems.push(format!(
                            "y={y} z={z} (world {:?}..):\n  actual  : {actual}\n  expected: {row}",
                            self.origin + IVec3::new(0, y as i32, z as i32)
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
                    let world = ChunkPos::from_ivec3(chunk_pos).block(local).world().as_ivec3();
                    let local = world - self.origin;
                    let inside = local.cmpge(IVec3::ZERO).all() && local.cmplt(self.size).all();
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

    fn check_scenario(scenario: &WaterScenario) {
        let layout = layout_scene(scenario.origin, scenario.input);
        let mut world = FluidSceneWorld::from_layout(&layout);
        let steps = world.settle(scenario.max_steps as usize);
        if let Err(problem) = world.diff(scenario.expected) {
            panic!(
                "scenario `{}` did not match the expected final state after {steps} settling \
                 steps.\nScenario: {}\n{problem}",
                scenario.name, scenario.description
            );
        }
    }

    #[test]
    fn registered_water_scenarios_settle_to_expected_state() {
        for scenario in WATER_SCENARIOS {
            check_scenario(scenario);
        }
    }

    #[test]
    fn water_scenario_names_are_unique() {
        let mut names = WATER_SCENARIOS.iter().map(|s| s.name).collect::<Vec<_>>();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "water scenario names must be unique");
    }

    #[test]
    fn scene_layout_shape_validation_rejects_ragged_rows() {
        let layers: &SceneLayers = &[&["##", "#"]];
        let result = std::panic::catch_unwind(|| scene_size(layers));
        assert!(result.is_err(), "ragged rows must be rejected");
    }
}
