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
//! - `#` glass (the solid fixture block, kept see-through so in-game scenes
//!   can be read from the side)
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

/// One settle-and-compare step after the opening scene.
///
/// Phases cover the steady state -> block edit -> recompute -> steady state
/// path that a single build-and-settle scene cannot reach: the runner settles
/// the previous phase, applies [`Self::edit`] like a player edit, then settles
/// and compares again.
pub(crate) struct ScenePhase {
    /// Cells to overwrite, map-shaped like the scene. `~` leaves a cell alone,
    /// `.` clears it (breaking a block), and anything else is
    /// [`cell_from_char`].
    pub(crate) edit: &'static SceneLayers,
    /// Expected state once the edit has settled.
    pub(crate) expected: &'static SceneLayers,
    /// Maximum fluid steps this phase may take to settle.
    pub(crate) max_steps: u32,
    /// When set, the headless driver asserts the phase settles in exactly
    /// this many fluid steps, so the *duration* is part of the behaviour
    /// under test and not just the end state.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) expect_steps: Option<u32>,
}

const impl Default for ScenePhase {
    fn default() -> Self {
        Self {
            edit: &[],
            expected: &[],
            max_steps: DEFAULT_MAX_STEPS,
            expect_steps: None,
        }
    }
}

/// Default fluid-step budget for a phase: enough for the current scenes to
/// settle with slack.
const DEFAULT_MAX_STEPS: u32 = 64;

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
    /// When set, the headless driver asserts the opening scene settles in
    /// exactly this many fluid steps.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) expect_steps: Option<u32>,
    /// Follow-up phases applied to the settled scene, in order. Empty for
    /// scenarios that only test the initial flow.
    pub(crate) phases: &'static [ScenePhase],
    /// Why this scenario exists and what vanilla behaviour it pins down.
    pub(crate) description: &'static str,
}

const impl Default for WaterScenario {
    /// Defaults for the fields most scenarios share: origin at zero, the
    /// standard step budget, no phases. Written by hand rather than derived
    /// because a derived default would set `max_steps` to 0, making every
    /// scenario time out instantly.
    fn default() -> Self {
        Self {
            name: "",
            origin: IVec3::ZERO,
            input: &[],
            expected: &[],
            max_steps: DEFAULT_MAX_STEPS,
            expect_steps: None,
            phases: &[],
            description: "",
        }
    }
}

/// The registered fidelity scenarios, in run order.
pub(crate) const WATER_SCENARIOS: &[WaterScenario] = &[
    WaterScenario {
        name: "flat_floor_diamond",
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
        ..Default::default()
    },
    WaterScenario {
        name: "channel_seven_block_range",
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
        ..Default::default()
    },
    WaterScenario {
        name: "infinite_source_gap_over_stone",
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
        ..Default::default()
    },
    WaterScenario {
        name: "nearest_drop_preference",
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
        ..Default::default()
    },
    WaterScenario {
        name: "falling_water_landing_spread",
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
        ..Default::default()
    },
    WaterScenario {
        name: "source_above_is_falling_not_source",
        input: &[
            &["#####"; 3],
            &["#####", "#S..#", "#####"],
            &["#####", "##S##", "#####"],
        ],
        expected: &[
            &["#####"; 3],
            &["#####", "#S87#", "#####"],
            &["#####", "##S##", "#####"],
        ],
        description: "Java Edition has no one-source-plus-source-above conversion rule; \
                      the cell under a source becomes falling water and feeds the open \
                      neighbour at level 7.",
        ..Default::default()
    },
    WaterScenario {
        name: "three_sources_spread_while_falling",
        input: &[
            &["######", "##.###", "######"],
            &["##S###", "#S....", "##S###"],
        ],
        expected: &[
            &["######", "##8###", "######"],
            &["##S###", "#S76..", "##S###"],
        ],
        description: "A water cell with three horizontal source neighbours spreads \
                      sideways even while it pours into a hole; the sideways flow is \
                      one level weaker.",
        ..Default::default()
    },
    WaterScenario {
        name: "source_over_hole_spreads_sideways",
        input: &[
            &["#######", "###.###", "#######"],
            &["#######", "#..S...", "#######"],
        ],
        expected: &[
            &["#######", "###8###", "#######"],
            &["#######", "#67S765", "#######"],
        ],
        description: "A source pours into a hole on its first tick, then pours fail \
                      and, being a source, it spreads sideways on the next tick.",
        ..Default::default()
    },
    WaterScenario {
        name: "gap_over_source_becomes_source",
        input: &[
            &["#####", "##S##", "#####"],
            &["#####", "#S.S#", "#####"],
        ],
        expected: &[
            &["#####", "##S##", "#####"],
            &["#####", "#SSS#", "#####"],
        ],
        description: "Vanilla's conversion support also accepts a water source below, \
                      not just a solid block, so a gap above a source converts too.",
        ..Default::default()
    },
    WaterScenario {
        name: "flow_into_hole_stays_narrow",
        input: &[
            &["#######", "###.###", "#######"],
            &["#######", "#S.....", "#######"],
        ],
        expected: &[
            &["#######", "###8###", "#######"],
            &["#######", "#S76...", "#######"],
        ],
        description: "A flowing cell above a hole pours into it and never spreads \
                      sideways there, so the stream stays one block wide.",
        ..Default::default()
    },
    WaterScenario {
        name: "cross_chunk_channel_flow",
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
        ..Default::default()
    },
    WaterScenario {
        name: "channel_dam_after_settle",
        input: &[
            &["##########"; 3],
            &["##########", "#S........", "##########"],
        ],
        expected: &[
            &["##########"; 3],
            &["##########", "#S7654321.", "##########"],
        ],
        phases: &[ScenePhase {
            edit: &[
                &["~~~~~~~~~~"; 3],
                &["~~~~~~~~~~", "~~~~#~~~~~", "~~~~~~~~~~"],
            ],
            expected: &[
                &["##########"; 3],
                &["##########", "#S76#.....", "##########"],
            ],
            ..Default::default()
        }],
        description: "Block placed into a settled channel: the flow upstream of the \
                      dam stays put and everything downstream drains away.",
        ..Default::default()
    },
    WaterScenario {
        name: "channel_wall_break_after_settle",
        input: &[
            &["##########"; 3],
            &["##########", "#S........", "##########"],
        ],
        expected: &[
            &["##########"; 3],
            &["##########", "#S7654321.", "##########"],
        ],
        phases: &[ScenePhase {
            edit: &[
                &["~~~~~~~~~~"; 3],
                &["~.~~~~~~~~", "~~~~~~~~~~", "~~~~~~~~~~"],
            ],
            expected: &[
                &["##########"; 3],
                &["#7########", "#S7654321.", "##########"],
            ],
            ..Default::default()
        }],
        description: "Wall block broken beside the source: water fills the one-cell \
                      pocket at level 7, and the channel does not change.",
        ..Default::default()
    },
    WaterScenario {
        name: "channel_source_removed_after_settle",
        input: &[
            &["##########"; 3],
            &["##########", "#S........", "##########"],
        ],
        expected: &[
            &["##########"; 3],
            &["##########", "#S7654321.", "##########"],
        ],
        phases: &[ScenePhase {
            edit: &[
                &["~~~~~~~~~~"; 3],
                &["~~~~~~~~~~", "~.~~~~~~~~", "~~~~~~~~~~"],
            ],
            expected: &[
                &["##########"; 3],
                &["##########", "#.........", "##########"],
            ],
            ..Default::default()
        }],
        description: "Source removed from a settled channel: no flowing water is \
                      left, so the whole channel drains away.",
        ..Default::default()
    },
    WaterScenario {
        name: "hanging_waterfall_dissipates_after_source_removed",
        input: &[
            &["#########"; 3],
            &["#########", ".........", "#########"],
            &["#########", ".........", "#########"],
            &["#########", "####.....", "#########"],
            &["#########", "#S.......", "#########"],
        ],
        expected: &[
            &["#########"; 3],
            &["#########", "456787654", "#########"],
            &["#########", "....8....", "#########"],
            &["#########", "####8....", "#########"],
            &["#########", "#S765....", "#########"],
        ],
        phases: &[ScenePhase {
            edit: &[
                &["~~~~~~~~~"; 3],
                &["~~~~~~~~~"; 3],
                &["~~~~~~~~~"; 3],
                &["~~~~~~~~~"; 3],
                &["~~~~~~~~~", "~.~~~~~~~", "~~~~~~~~~"],
            ],
            expected: &[
                &["#########"; 3],
                &["#########", ".........", "#########"],
                &["#########", ".........", "#########"],
                &["#########", "####.....", "#########"],
                &["#########", "#........", "#########"],
            ],
            expect_steps: Some(17),
            ..Default::default()
        }],
        description: "Source removed from a settled hanging waterfall: the falling \
                      column and the landing pool drain away completely. The total \
                      dissipation time is pinned so it cannot become instant.",
        ..Default::default()
    },
    WaterScenario {
        name: "reported_block_in_basin",
        input: &[
            &["#############"; 13],
            &[
                "#############",
                "#############",
                "#############",
                "#############",
                "#############",
                "#############",
                "#####65######",
                "#####76######",
                "#####87######",
                "#############",
                "#############",
                "#############",
                "#############",
            ],
            &[
                "###########..",
                "##########...",
                "######.......",
                "#234321......",
                "#345432......",
                "#4565#1......",
                "#567#........",
                "#67S#........",
                "#56765.......",
                "#............",
                "#............",
                "##...........",
                "###..........",
            ],
        ],
        expected: &[
            &["#############"; 13],
            &[
                "#############",
                "#############",
                "#############",
                "#############",
                "#############",
                "#############",
                "#####65######",
                "#####76######",
                "#####87######",
                "#############",
                "#############",
                "#############",
                "#############",
            ],
            &[
                "###########..",
                "##########...",
                "######.......",
                "#234321......",
                "#345432......",
                "#4565#1......",
                "#567#........",
                "#67S#........",
                "#56765.......",
                "#............",
                "#............",
                "##...........",
                "###..........",
            ],
        ],
        description: "Hand-reported case in the developer's save, chunk (258,1,258): \
                      crop world x4128..=4140, z4128..=4140, layers y=27..29 (layer 0 \
                      is world y=27). `input` is the exact saved field; the reported \
                      block is the `#` at world (4133,29,4133), placed into the pool. \
                      `expected` repeats the input only because the saved field is \
                      already a fixpoint of the current solver: replace it with the \
                      field Minecraft is expected to show and every wrong cell becomes \
                      a failing diff. The wide pool itself is leftover history: from a \
                      fresh build with the pocket open, the nearest-drop rule sends the \
                      source straight down the stream to (4133,29,4136) and this pool \
                      never forms (vanilla `FlowingFluid.getSpread` keeps only the \
                      nearest-drop directions, so it pools the same way).",
        ..Default::default()
    },
];

/// The expected map for `phase`, where phase 0 is [`WaterScenario::expected`]
/// and later phases are the entries of [`WaterScenario::phases`].
pub(crate) fn phase_expected(scenario: &WaterScenario, phase: usize) -> &'static SceneLayers {
    match phase {
        0 => scenario.expected,
        later => scenario.phases[later - 1].expected,
    }
}

/// The fluid-step budget for `phase`.
pub(crate) fn phase_max_steps(scenario: &WaterScenario, phase: usize) -> u32 {
    match phase {
        0 => scenario.max_steps,
        later => scenario.phases[later - 1].max_steps,
    }
}

/// Calls `set` for every cell an edit map changes: `~` skips a cell, `.`
/// clears it, and any other character is parsed with [`cell_from_char`].
pub(crate) fn for_each_edit_cell(
    origin: IVec3,
    size: IVec3,
    edit: &SceneLayers,
    mut set: impl FnMut(IVec3, ChunkCell),
) {
    assert_eq!(scene_size(edit), size, "edit map must match the scene size");
    for (y, layer) in edit.iter().enumerate() {
        for (z, row) in layer.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                if c == '~' {
                    continue;
                }
                set(
                    origin + IVec3::new(x as i32, y as i32, z as i32),
                    cell_from_char(c),
                );
            }
        }
    }
}

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
/// The scene is walled in with a one-block glass shell on every side except
/// the top; glass keeps the fixture see-through so the water can be watched
/// from a standing camera angle, not only from directly overhead. Scenes
/// describe only the cells under review; without a shell water would silently
/// escape sideways along the open chunk and read as a mysterious test failure.
/// The top stays open so a developer can look into the scene from above; water
/// never flows upward.
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
                    cells.push((origin + IVec3::new(x, y, z), Item::Glass.into()));
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
        '#' => ChunkCell::block(Item::Glass),
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
        ChunkCell::Block(Item::Glass) => '#',
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

        /// Applies an edit map like a player changing blocks in the scene.
        fn apply_edit(&mut self, edit: &SceneLayers) {
            for_each_edit_cell(self.origin, self.size, edit, |position, cell| {
                self.set_cell(position, cell);
            });
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
        check_phase(scenario, 0, &world, steps);

        for (index, phase) in scenario.phases.iter().enumerate() {
            world.apply_edit(phase.edit);
            let steps = world.settle(phase.max_steps as usize);
            check_phase(scenario, index + 1, &world, steps);
        }
    }

    /// The pinned settle duration for `phase`, when the scenario declares one.
    fn phase_expect_steps(scenario: &WaterScenario, phase: usize) -> Option<u32> {
        match phase {
            0 => scenario.expect_steps,
            later => scenario.phases[later - 1].expect_steps,
        }
    }

    fn check_phase(scenario: &WaterScenario, phase: usize, world: &FluidSceneWorld, steps: usize) {
        if let Some(expected) = phase_expect_steps(scenario, phase) {
            assert_eq!(
                steps as u32, expected,
                "scenario `{}` phase {phase} settled in {steps} fluid steps, expected \
                 {expected}; the duration is part of the behaviour under test",
                scenario.name
            );
        }
        if let Err(problem) = world.diff(phase_expected(scenario, phase)) {
            panic!(
                "scenario `{}` phase {phase} did not match the expected state after {steps} \
                 settling steps.\nScenario: {}\n{problem}",
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
    fn fixtures_are_glass_not_stone() {
        for scenario in WATER_SCENARIOS {
            let layout = layout_scene(scenario.origin, scenario.input);
            assert!(
                layout
                    .cells
                    .iter()
                    .all(|(_, cell)| *cell != ChunkCell::block(Item::Stone)),
                "scenario `{}` still contains stone; fixtures are glass so the \
                 water can be watched through the shell",
                scenario.name
            );
        }
    }

    #[test]
    fn scenario_phases_match_their_scene_shape() {
        for scenario in WATER_SCENARIOS {
            let size = scene_size(scenario.input);
            assert_eq!(
                scene_size(scenario.expected),
                size,
                "`{}` expected map shape",
                scenario.name
            );
            for (index, phase) in scenario.phases.iter().enumerate() {
                assert_eq!(
                    scene_size(phase.edit),
                    size,
                    "`{}` phase {index} edit shape",
                    scenario.name
                );
                assert_eq!(
                    scene_size(phase.expected),
                    size,
                    "`{}` phase {index} expected shape",
                    scenario.name
                );
            }
        }
    }

    #[test]
    fn pinned_settle_times_fit_their_budgets() {
        for scenario in WATER_SCENARIOS {
            for phase in 0..=scenario.phases.len() {
                let Some(expected) = phase_expect_steps(scenario, phase) else {
                    continue;
                };
                let budget = phase_max_steps(scenario, phase);
                assert!(
                    expected <= budget,
                    "`{}` phase {phase} pins {expected} steps but only budgets {budget}",
                    scenario.name
                );
            }
        }
    }

    #[test]
    fn scene_layout_shape_validation_rejects_ragged_rows() {
        let layers: &SceneLayers = &[&["##", "#"]];
        let result = std::panic::catch_unwind(|| scene_size(layers));
        assert!(result.is_err(), "ragged rows must be rejected");
    }
}
