pub mod ambient_occlusion;
pub mod collider;
mod fluid;
mod fluid_sim;
pub mod light;
pub mod mesh;

use bevy::prelude::*;
use collider::ChunkColliderPlugin;
use fluid::ChunkFluidPlugin;
use mesh::ChunkMeshPlugin;
use std::{fmt, num::NonZeroU8, str::FromStr};
use strum::{Display, EnumCount, EnumString};

pub use light::{ChunkHeightmap, ChunkLight};

use crate::block::{
    BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED, BlockStateId, BlockType, HotBlockStateMeta,
};

pub struct ChunkPlugin;

impl Plugin for ChunkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChunkPerfCounters>()
            .add_plugins((ChunkFluidPlugin, ChunkMeshPlugin));
        if std::env::var_os("MINECRAFT_CLONE_DISABLE_CHUNK_COLLIDERS").is_none() {
            app.add_plugins(ChunkColliderPlugin);
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct ChunkPerfCounters {
    pub mesh_rebuilds: usize,
    pub light_rebuild_targets: usize,
    pub light_uploads: usize,
}

impl ChunkPerfCounters {
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }
}

pub const CHUNK_SIZE: usize = 16;
pub const CHUNK_ISIZE: i32 = 16;

pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

pub const AIR_BLOCK_STATE_ID: BlockStateId = BlockStateId(0);
const FIRST_BLOCK_STATE_ID: u32 = 1;
const FIRST_FLUID_STATE_ID: u32 = FIRST_BLOCK_STATE_ID + BlockType::COUNT as u32;

#[derive(Debug, Clone, Copy, Default)]
pub struct BlockRegistry;

pub const BLOCK_REGISTRY: BlockRegistry = BlockRegistry;

impl BlockRegistry {
    pub fn hot_meta(&self, state: BlockStateId) -> Option<HotBlockStateMeta> {
        cell_from_state_id(state).map(ChunkCell::hot_meta)
    }

    pub fn cell(&self, state: BlockStateId) -> Option<ChunkCell> {
        cell_from_state_id(state)
    }
}

/// Logical scan order is y-fastest, then z, then x.
#[inline(always)]
pub const fn chunk_linear_index(x: usize, y: usize, z: usize) -> usize {
    y + CHUNK_SIZE * (z + CHUNK_SIZE * x)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, EnumString, Display)]
#[strum(serialize_all = "snake_case")]
pub enum FluidType {
    Water,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, EnumCount, EnumString, Display)]
pub enum FluidForm {
    #[strum(serialize = "source")]
    Source,
    #[strum(serialize = "flow")]
    Flowing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Reflect)]
pub struct FluidLevel(NonZeroU8);

impl FluidLevel {
    pub const fn new(level: u8) -> Option<Self> {
        match NonZeroU8::new(level) {
            Some(level) => Some(Self(level)),
            None => None,
        }
    }

    pub const fn new_const(level: u8) -> Self {
        match NonZeroU8::new(level) {
            Some(nz) => FluidLevel(nz),
            None => panic!("fluid level must be non-zero"),
        }
    }

    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

impl fmt::Display for FluidLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl FromStr for FluidLevel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let level = s.parse::<u8>().map_err(|_| ())?;
        Self::new(level).ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect)]
pub struct FluidState {
    ty: FluidType,
    form: FluidForm,
    level: FluidLevel,
}

impl FluidState {
    const fn new(ty: FluidType, form: FluidForm, level: FluidLevel) -> Self {
        Self { ty, form, level }
    }

    const fn source(ty: FluidType, level: FluidLevel) -> Self {
        Self::new(ty, FluidForm::Source, level)
    }

    const fn flowing(ty: FluidType, level: FluidLevel) -> Self {
        Self::new(ty, FluidForm::Flowing, level)
    }

    pub fn water_source() -> Self {
        FluidProfile::WATER.source()
    }

    pub fn water_flow(level: u8) -> Self {
        FluidProfile::WATER
            .flowing_level(level)
            .expect("water flow level must be within the water profile range")
    }

    pub const fn ty(self) -> FluidType {
        self.ty
    }

    pub const fn form(self) -> FluidForm {
        self.form
    }

    pub const fn level(self) -> FluidLevel {
        self.level
    }

    pub const fn is_source(self) -> bool {
        matches!(self.form, FluidForm::Source)
    }

    pub fn name(self) -> String {
        self.to_string()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        name.parse().ok()
    }
}

impl fmt::Display for FluidState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}_{}", self.ty, self.form, self.level)
    }
}

impl FromStr for FluidState {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name_and_form, level) = s.rsplit_once('_').ok_or(())?;
        let (ty, form) = name_and_form.rsplit_once('_').ok_or(())?;
        let ty = ty.parse::<FluidType>().map_err(|_| ())?;
        let form = form.parse::<FluidForm>().map_err(|_| ())?;
        let level = level.parse::<FluidLevel>()?;
        let state = Self::new(ty, form, level);

        if FluidProfile::default_for_type(ty).contains(state) {
            Ok(state)
        } else {
            Err(())
        }
    }
}

/// Simulation rules for a single fluid type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect)]
pub struct FluidProfile {
    pub ty: FluidType,
    pub full_level: FluidLevel,
    pub horizontal_decay: NonZeroU8,
    pub min_flow_level: FluidLevel,
    pub creates_sources: bool,
}

impl FluidProfile {
    pub const WATER: Self = Self {
        ty: FluidType::Water,
        full_level: FluidLevel::new_const(8),
        horizontal_decay: nonzero_u8(1),
        min_flow_level: FluidLevel::new_const(1),
        creates_sources: true,
    };

    pub const fn default_for_type(ty: FluidType) -> Self {
        match ty {
            FluidType::Water => Self::WATER,
        }
    }

    pub fn source(self) -> FluidState {
        FluidState::source(self.ty, self.full_level)
    }

    pub fn falling(self) -> FluidState {
        FluidState::flowing(self.ty, self.full_level)
    }

    pub fn flowing(self, level: FluidLevel) -> Option<FluidState> {
        if level < self.min_flow_level || level > self.full_level {
            return None;
        }

        Some(FluidState::flowing(self.ty, level))
    }

    pub fn flowing_level(self, level: u8) -> Option<FluidState> {
        self.flowing(FluidLevel::new(level)?)
    }

    pub fn decayed_flow(self, fluid: FluidState) -> Option<FluidState> {
        if fluid.ty() != self.ty {
            return None;
        }

        let next_level = fluid
            .level()
            .get()
            .saturating_sub(self.horizontal_decay.get());
        self.flowing_level(next_level)
    }

    pub fn contains(self, fluid: FluidState) -> bool {
        if fluid.ty() != self.ty {
            return false;
        }

        match fluid.form() {
            FluidForm::Source => fluid.level() == self.full_level,
            FluidForm::Flowing => {
                fluid.level() >= self.min_flow_level && fluid.level() <= self.full_level
            }
        }
    }
}

const fn nonzero_u8(value: u8) -> NonZeroU8 {
    match NonZeroU8::new(value) {
        Some(value) => value,
        None => panic!("value must be non-zero"),
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Hash, Reflect, Debug)]
pub enum ChunkCell {
    #[default]
    Empty,
    Block(BlockType),
    Fluid(FluidState),
}

impl ChunkCell {
    pub const EMPTY: Self = Self::Empty;

    pub const fn block(block: BlockType) -> Self {
        Self::Block(block)
    }

    pub const fn fluid(fluid: FluidState) -> Self {
        Self::Fluid(fluid)
    }

    pub fn water_source() -> Self {
        Self::Fluid(FluidState::water_source())
    }

    pub fn water_flow(level: u8) -> Self {
        Self::Fluid(FluidState::water_flow(level))
    }

    #[inline(always)]
    pub fn state_id(self) -> BlockStateId {
        match self {
            Self::Empty => AIR_BLOCK_STATE_ID,
            Self::Block(block) => BlockStateId(FIRST_BLOCK_STATE_ID + block as u32),
            Self::Fluid(fluid) => {
                let profile = FluidProfile::default_for_type(fluid.ty());
                debug_assert!(profile.contains(fluid));
                let level_offset = fluid.level().get() as u32 - 1;
                let form_offset = match fluid.form() {
                    FluidForm::Flowing => 0,
                    FluidForm::Source => profile.full_level.get() as u32,
                };
                BlockStateId(FIRST_FLUID_STATE_ID + level_offset + form_offset)
            }
        }
    }

    pub fn from_state_id(state: BlockStateId) -> Option<Self> {
        cell_from_state_id(state)
    }

    #[inline(always)]
    pub const fn hot_meta(self) -> HotBlockStateMeta {
        match self {
            Self::Empty => HotBlockStateMeta::AIR,
            Self::Block(block) => HotBlockStateMeta::for_block(block),
            Self::Fluid(fluid) => HotBlockStateMeta::water(fluid.level().get()),
        }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        matches!(self, Self::Empty)
    }

    #[inline(always)]
    pub const fn is_block(self) -> bool {
        matches!(self, Self::Block(_))
    }

    #[inline(always)]
    pub const fn is_fluid(self) -> bool {
        matches!(self, Self::Fluid(_))
    }

    pub const fn as_block(self) -> Option<BlockType> {
        match self {
            Self::Block(b) => Some(b),
            _ => None,
        }
    }

    pub const fn as_fluid(self) -> Option<FluidState> {
        match self {
            Self::Fluid(f) => Some(f),
            _ => None,
        }
    }

    pub const fn kind(self) -> u16 {
        self.hot_meta().render_id
    }

    #[inline(always)]
    pub const fn is_rendered(self) -> bool {
        self.hot_meta().mesh_flags & BLOCK_FLAG_RENDERED != 0
    }

    #[inline(always)]
    pub const fn is_full_cube(self) -> bool {
        self.hot_meta().mesh_flags & BLOCK_FLAG_FULL_CUBE != 0
    }

    #[inline(always)]
    pub const fn light_emission(self) -> u8 {
        self.hot_meta().light_emission
    }

    #[inline(always)]
    pub const fn light_opacity(self) -> u8 {
        self.hot_meta().light_opacity
    }

    #[inline(always)]
    pub const fn is_transparent_to_sky_light(self) -> bool {
        self.light_opacity() < 15
    }

    #[inline(always)]
    pub const fn is_solid(self) -> bool {
        self.is_block()
    }

    pub const fn can_be_replaced_by_placement(self) -> bool {
        !self.is_solid()
    }

    pub fn name(self) -> String {
        match self {
            Self::Empty => "air".to_owned(),
            Self::Block(b) => b.name(),
            Self::Fluid(f) => f.name(),
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        if name == "air" {
            return Some(Self::Empty);
        }
        if let Some(fluid) = FluidState::from_name(name) {
            return Some(Self::Fluid(fluid));
        }
        BlockType::from_name(name).map(Self::Block)
    }
}

fn cell_from_state_id(state: BlockStateId) -> Option<ChunkCell> {
    let raw = state.0;
    if raw == AIR_BLOCK_STATE_ID.0 {
        return Some(ChunkCell::Empty);
    }

    if raw < FIRST_FLUID_STATE_ID {
        let block_id = (raw - FIRST_BLOCK_STATE_ID) as u16;
        return BlockType::from_storage_id(block_id).map(ChunkCell::Block);
    }

    let profile = FluidProfile::WATER;
    let level_count = profile.full_level.get() as u32;
    let fluid_offset = raw - FIRST_FLUID_STATE_ID;
    if fluid_offset >= level_count * FluidForm::COUNT as u32 {
        return None;
    }
    let level = FluidLevel::new((fluid_offset % level_count) as u8 + 1)?;
    let form = if fluid_offset >= level_count {
        FluidForm::Source
    } else {
        FluidForm::Flowing
    };

    Some(ChunkCell::Fluid(FluidState::new(profile.ty, form, level)))
}

impl From<BlockType> for ChunkCell {
    #[inline(always)]
    fn from(block: BlockType) -> Self {
        Self::Block(block)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellDelta {
    pub old: ChunkCell,
    pub new: ChunkCell,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FluidStepResult {
    pub changed: bool,
    pub boundary_changed: bool,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChunkBlockCounts {
    pub rendered: u16,
    pub full_cubes: u16,
    pub translucent: u16,
}

impl ChunkBlockCounts {
    pub fn apply_delta(&mut self, delta: CellDelta) {
        let (old_rendered, old_full, old_trans) = cell_counts(delta.old);
        let (new_rendered, new_full, new_trans) = cell_counts(delta.new);
        self.rendered = self
            .rendered
            .wrapping_add(new_rendered)
            .wrapping_sub(old_rendered);
        self.full_cubes = self
            .full_cubes
            .wrapping_add(new_full)
            .wrapping_sub(old_full);
        self.translucent = self
            .translucent
            .wrapping_add(new_trans)
            .wrapping_sub(old_trans);
    }
}

fn cell_counts(cell: ChunkCell) -> (u16, u16, u16) {
    meta_counts(cell.hot_meta())
}

fn meta_counts(meta: HotBlockStateMeta) -> (u16, u16, u16) {
    let rendered = (meta.mesh_flags & BLOCK_FLAG_RENDERED != 0) as u16;
    let full_cubes = (meta.mesh_flags & BLOCK_FLAG_FULL_CUBE != 0) as u16;
    (rendered, full_cubes, rendered.saturating_sub(full_cubes))
}

pub(crate) fn chunk_neighbor_offsets() -> impl Iterator<Item = IVec3> {
    (-1..=1).flat_map(|x| {
        (-1..=1).flat_map(move |y| {
            (-1..=1).filter_map(move |z| {
                let offset = ivec3(x, y, z);
                (offset != IVec3::ZERO).then_some(offset)
            })
        })
    })
}

pub(crate) fn chunk_neighbor_offsets_for_block(block: UVec3) -> impl Iterator<Item = IVec3> {
    chunk_neighbor_offsets().filter(move |offset| {
        neighbor_axis_can_sample_block(offset.x, block.x)
            && neighbor_axis_can_sample_block(offset.y, block.y)
            && neighbor_axis_can_sample_block(offset.z, block.z)
    })
}

fn neighbor_axis_can_sample_block(offset: i32, coord: u32) -> bool {
    match offset {
        -1 => coord == 0,
        0 => true,
        1 => coord == CHUNK_SIZE as u32 - 1,
        _ => false,
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkPosition(pub IVec3);

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsSave;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsMeshRebuild;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsLightUpload;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsColliderRebuild;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsLightRebuild;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkHasActiveFluids;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    pub state: BlockStateId,
    pub hot: HotBlockStateMeta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPalette {
    entries: Vec<PaletteEntry>,
}

impl Default for ChunkPalette {
    fn default() -> Self {
        Self {
            entries: vec![PaletteEntry {
                state: AIR_BLOCK_STATE_ID,
                hot: HotBlockStateMeta::AIR,
            }],
        }
    }
}

impl ChunkPalette {
    pub fn entries(&self) -> &[PaletteEntry] {
        &self.entries
    }

    #[inline(always)]
    fn entry(&self, index: u32) -> PaletteEntry {
        self.entries[index as usize]
    }

    #[inline(always)]
    fn cell(&self, index: u32) -> ChunkCell {
        ChunkCell::from_state_id(self.entry(index).state).expect("invalid state in chunk palette")
    }

    fn index_for_state(&self, state: BlockStateId) -> Option<u32> {
        self.entries
            .iter()
            .position(|entry| entry.state == state)
            .map(|index| index as u32)
    }

    fn get_or_insert_cell(&mut self, cell: ChunkCell) -> u32 {
        let state = cell.state_id();
        if let Some(index) = self.index_for_state(state) {
            return index;
        }

        let index = self.entries.len() as u32;
        self.entries.push(PaletteEntry {
            state,
            hot: BLOCK_REGISTRY
                .hot_meta(state)
                .expect("state id from chunk cell must be valid"),
        });
        index
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellStorage {
    U8(Box<[u8; CHUNK_VOLUME]>),
    U16(Box<[u16; CHUNK_VOLUME]>),
    U32(Box<[u32; CHUNK_VOLUME]>),
}

impl Default for CellStorage {
    fn default() -> Self {
        Self::U8(Box::new([0; CHUNK_VOLUME]))
    }
}

impl CellStorage {
    #[inline(always)]
    pub fn get_linear(&self, index: usize) -> u32 {
        match self {
            Self::U8(cells) => cells[index] as u32,
            Self::U16(cells) => cells[index] as u32,
            Self::U32(cells) => cells[index],
        }
    }

    #[inline(always)]
    fn set_linear(&mut self, index: usize, palette_index: u32) {
        match self {
            Self::U8(cells) => cells[index] = palette_index as u8,
            Self::U16(cells) => cells[index] = palette_index as u16,
            Self::U32(cells) => cells[index] = palette_index,
        }
    }

    #[inline(always)]
    fn max_index(&self) -> u32 {
        match self {
            Self::U8(_) => u8::MAX as u32,
            Self::U16(_) => u16::MAX as u32,
            Self::U32(_) => u32::MAX,
        }
    }

    fn promote_for_index(&mut self, palette_index: u32) {
        if palette_index <= self.max_index() {
            return;
        }

        if palette_index <= u16::MAX as u32 {
            let mut promoted = Box::new([0u16; CHUNK_VOLUME]);
            for (index, cell) in promoted.iter_mut().enumerate() {
                *cell = self.get_linear(index) as u16;
            }
            *self = Self::U16(promoted);
        } else {
            let mut promoted = Box::new([0u32; CHUNK_VOLUME]);
            for (index, cell) in promoted.iter_mut().enumerate() {
                *cell = self.get_linear(index);
            }
            *self = Self::U32(promoted);
        }
    }

    fn fill(&mut self, palette_index: u32) {
        self.promote_for_index(palette_index);
        match self {
            Self::U8(cells) => cells.fill(palette_index as u8),
            Self::U16(cells) => cells.fill(palette_index as u16),
            Self::U32(cells) => cells.fill(palette_index),
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct Chunk {
    palette: ChunkPalette,
    cells: CellStorage,
}

impl PartialEq for Chunk {
    fn eq(&self, other: &Self) -> bool {
        (0..CHUNK_VOLUME).all(|index| self.state_id_linear(index) == other.state_id_linear(index))
    }
}

impl Eq for Chunk {}

impl Default for Chunk {
    fn default() -> Self {
        Self {
            palette: ChunkPalette::default(),
            cells: CellStorage::default(),
        }
    }
}

impl Chunk {
    pub fn filled(cell: ChunkCell) -> Self {
        let mut chunk = Self::default();
        chunk.fill(cell);
        chunk
    }

    pub fn from_cell_fn(mut cell_at: impl FnMut(usize, usize, usize) -> ChunkCell) -> Self {
        let mut chunk = Self::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let index = chunk_linear_index(x, y, z);
                    chunk.write_cell_linear(index, cell_at(x, y, z));
                }
            }
        }
        chunk
    }

    pub fn palette(&self) -> &ChunkPalette {
        &self.palette
    }

    pub fn cell_storage(&self) -> &CellStorage {
        &self.cells
    }

    #[inline(always)]
    pub fn get_cell(&self, pos: UVec3) -> ChunkCell {
        self.cell_xyz(pos.x as usize, pos.y as usize, pos.z as usize)
    }

    #[inline(always)]
    pub fn cell_xyz(&self, x: usize, y: usize, z: usize) -> ChunkCell {
        self.cell_linear(chunk_linear_index(x, y, z))
    }

    #[inline(always)]
    pub fn cell_linear(&self, index: usize) -> ChunkCell {
        self.palette.cell(self.cells.get_linear(index))
    }

    #[inline(always)]
    pub fn palette_index(&self, pos: UVec3) -> u32 {
        self.palette_index_xyz(pos.x as usize, pos.y as usize, pos.z as usize)
    }

    #[inline(always)]
    pub fn palette_index_xyz(&self, x: usize, y: usize, z: usize) -> u32 {
        self.cells.get_linear(chunk_linear_index(x, y, z))
    }

    #[inline(always)]
    pub fn palette_index_linear(&self, index: usize) -> u32 {
        self.cells.get_linear(index)
    }

    #[inline(always)]
    pub fn state_id(&self, pos: UVec3) -> BlockStateId {
        self.state_id_linear(chunk_linear_index(
            pos.x as usize,
            pos.y as usize,
            pos.z as usize,
        ))
    }

    #[inline(always)]
    pub fn state_id_linear(&self, index: usize) -> BlockStateId {
        self.palette.entry(self.cells.get_linear(index)).state
    }

    #[inline(always)]
    pub fn hot_meta(&self, pos: UVec3) -> HotBlockStateMeta {
        self.hot_meta_xyz(pos.x as usize, pos.y as usize, pos.z as usize)
    }

    #[inline(always)]
    pub fn hot_meta_xyz(&self, x: usize, y: usize, z: usize) -> HotBlockStateMeta {
        self.hot_meta_linear(chunk_linear_index(x, y, z))
    }

    #[inline(always)]
    pub fn hot_meta_linear(&self, index: usize) -> HotBlockStateMeta {
        self.palette.entry(self.cells.get_linear(index)).hot
    }

    #[inline(always)]
    pub fn get_block(&self, pos: UVec3) -> Option<BlockType> {
        self.get_cell(pos).as_block()
    }

    #[inline(always)]
    pub fn set_cell(&mut self, pos: UVec3, cell: ChunkCell) -> CellDelta {
        self.set_cell_xyz(pos.x as usize, pos.y as usize, pos.z as usize, cell)
    }

    #[inline(always)]
    pub fn set_cell_xyz(&mut self, x: usize, y: usize, z: usize, cell: ChunkCell) -> CellDelta {
        self.set_cell_linear(chunk_linear_index(x, y, z), cell)
    }

    pub fn set_cell_linear(&mut self, index: usize, cell: ChunkCell) -> CellDelta {
        let old = self.cell_linear(index);
        let new = self.write_cell_linear(index, cell);
        CellDelta { old, new }
    }

    pub fn set_state(
        &mut self,
        pos: UVec3,
        state: BlockStateId,
        registry: &BlockRegistry,
    ) -> Option<CellDelta> {
        registry.cell(state).map(|cell| self.set_cell(pos, cell))
    }

    fn write_cell_linear(&mut self, index: usize, cell: ChunkCell) -> ChunkCell {
        let palette_index = self.palette.get_or_insert_cell(cell);
        self.cells.promote_for_index(palette_index);
        self.cells.set_linear(index, palette_index);
        cell
    }

    pub fn set_block(&mut self, pos: UVec3, block: BlockType) -> CellDelta {
        self.set_cell(pos, block.into())
    }

    pub fn set_empty(&mut self, pos: UVec3) -> CellDelta {
        self.set_cell(pos, ChunkCell::EMPTY)
    }

    pub fn set_fluid(&mut self, pos: UVec3, fluid: FluidState) -> CellDelta {
        self.set_cell(pos, ChunkCell::fluid(fluid))
    }

    pub fn fill(&mut self, cell: ChunkCell) {
        let palette_index = self.palette.get_or_insert_cell(cell);
        self.cells.fill(palette_index);
    }

    pub fn get_i(&self, x: i32, y: i32, z: i32) -> Option<ChunkCell> {
        let outside = |a: i32| !(0..CHUNK_ISIZE).contains(&a);
        if outside(x) || outside(y) || outside(z) {
            return None;
        }

        Some(self.cell_xyz(x as usize, y as usize, z as usize))
    }

    pub fn place_cell(&mut self, pos: UVec3, cell: ChunkCell) -> Option<CellDelta> {
        if !cell.is_rendered() {
            return None;
        }

        let old = self.get_cell(pos);
        if !old.can_be_replaced_by_placement() {
            return None;
        };

        Some(self.set_cell(pos, cell))
    }

    pub fn place_block(&mut self, pos: UVec3, block: BlockType) -> Option<CellDelta> {
        if !block.is_placeable() {
            return None;
        }

        self.place_cell(pos, block.into())
    }

    pub fn break_block(&mut self, pos: UVec3) -> Option<CellDelta> {
        if !self.get_cell(pos).is_solid() {
            return None;
        };

        Some(self.set_empty(pos))
    }

    pub fn step_fluids(&mut self, profile: &FluidProfile) -> FluidStepResult {
        let old_cells = self.to_cell_buffer();

        let snapshot = fluid_sim::FluidSnapshot::from_chunk(IVec3::ZERO, self);
        let step = fluid_sim::simulate_fluid_step(&snapshot, &[IVec3::ZERO], *profile);
        for update in step.updates {
            let (chunk_pos, local) = fluid_sim::world_to_chunk_local(update.pos);
            if chunk_pos == IVec3::ZERO {
                self.set_cell(local, update.cell);
            }
        }

        self.fluid_step_result_from(&old_cells)
    }

    pub(super) fn fluid_step_result_from(
        &self,
        old_cells: &[ChunkCell; CHUNK_VOLUME],
    ) -> FluidStepResult {
        let mut result = FluidStepResult::default();
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let index = chunk_linear_index(x, y, z);
                    if old_cells[index] != self.cell_linear(index) {
                        result.changed = true;
                        result.boundary_changed |= is_chunk_boundary_cell(x, y, z);
                    }
                }
            }
        }
        result
    }

    pub fn compute_block_counts(&self) -> ChunkBlockCounts {
        let mut counts = ChunkBlockCounts::default();
        for index in 0..CHUNK_VOLUME {
            let (r, fc, t) = meta_counts(self.hot_meta_linear(index));
            counts.rendered += r;
            counts.full_cubes += fc;
            counts.translucent += t;
        }
        counts
    }

    pub fn iter(&self) -> BlockIterator<'_> {
        BlockIterator {
            chunk: self,
            index: 0,
        }
    }

    fn build_palette(&self) -> Vec<ChunkCell> {
        let mut palette = Vec::new();
        for (cell, _) in self.iter() {
            if !palette.contains(&cell) {
                palette.push(cell);
            }
        }
        palette
    }

    /// Bit-packed semantic palette encoding.
    ///
    /// ```text
    /// [u16 LE: palette_size]
    /// for each entry: [u8: name_len] [name bytes]
    /// [u8: bits_per_index]
    /// [bit-packed body, MSB-first, padded to byte boundary]
    /// ```
    /// bits_per_index = ceil(log2(palette_size)), min 1.
    pub fn to_storage_bytes(&self) -> Vec<u8> {
        let palette = self.build_palette();
        let bits = bits_for(palette.len());
        let cell_to_idx: std::collections::HashMap<ChunkCell, u32> = palette
            .iter()
            .enumerate()
            .map(|(i, &cell)| (cell, i as u32))
            .collect();
        let indices: Vec<u32> = self.iter().map(|(cell, _)| cell_to_idx[&cell]).collect();

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(palette.len() as u16).to_le_bytes());
        for &cell in &palette {
            let name = cell.name();
            bytes.push(name.len() as u8);
            bytes.extend_from_slice(name.as_bytes());
        }
        bytes.push(bits);

        let body_start = bytes.len();
        let body_bytes = (indices.len() * bits as usize).div_ceil(8);
        bytes.resize(body_start + body_bytes, 0);
        pack(&mut bytes[body_start..], &indices, bits);

        bytes
    }

    pub fn try_from_storage_bytes(bytes: &[u8]) -> Result<Self, ChunkDecodeError> {
        if bytes.len() < 3 {
            return Err(ChunkDecodeError::Truncated);
        }

        let palette_size = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        if palette_size == 0 {
            return Err(ChunkDecodeError::Truncated);
        }

        let mut pos = 2usize;
        let mut palette = Vec::with_capacity(palette_size);

        for _ in 0..palette_size {
            if pos >= bytes.len() {
                return Err(ChunkDecodeError::Truncated);
            }
            let len = bytes[pos] as usize;
            pos += 1;
            if pos + len > bytes.len() {
                return Err(ChunkDecodeError::Truncated);
            }
            let name = std::str::from_utf8(&bytes[pos..pos + len])
                .map_err(|_| ChunkDecodeError::InvalidHeader)?;
            pos += len;
            let cell = ChunkCell::from_name(name)
                .ok_or_else(|| ChunkDecodeError::UnknownBlock(name.to_owned()))?;
            palette.push(cell);
        }

        let bits = *bytes.get(pos).ok_or(ChunkDecodeError::Truncated)?;
        if bits == 0 || bits > 32 {
            return Err(ChunkDecodeError::InvalidHeader);
        }
        pos += 1;

        let body = bytes.get(pos..).ok_or(ChunkDecodeError::Truncated)?;
        let mask = if bits == 32 {
            u32::MAX
        } else {
            (1u32 << bits) - 1
        };
        let body_bytes = (CHUNK_VOLUME * bits as usize).div_ceil(8);

        let mut chunk = Chunk::default();
        let mut bit_pos = 0usize;

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let idx =
                        read_bits(body, &mut bit_pos, bits).ok_or(ChunkDecodeError::Truncated)?;
                    let idx = (idx & mask) as usize;
                    if idx >= palette.len() {
                        return Err(ChunkDecodeError::InvalidHeader);
                    }
                    chunk.write_cell_linear(chunk_linear_index(x, y, z), palette[idx]);
                }
            }
        }

        pos += body_bytes;
        if pos != bytes.len() {
            return Err(ChunkDecodeError::InvalidHeader);
        }

        Ok(chunk)
    }

    pub fn has_fluids(&self) -> bool {
        (0..CHUNK_VOLUME).any(|index| self.hot_meta_linear(index).fluid_level > 0)
    }

    pub(super) fn to_cell_buffer(&self) -> [ChunkCell; CHUNK_VOLUME] {
        std::array::from_fn(|index| self.cell_linear(index))
    }
}

fn is_chunk_boundary_cell(x: usize, y: usize, z: usize) -> bool {
    x == 0 || x == CHUNK_SIZE - 1 || y == 0 || y == CHUNK_SIZE - 1 || z == 0 || z == CHUNK_SIZE - 1
}

fn bits_for(palette_size: usize) -> u8 {
    match palette_size {
        0 | 1 => 1,
        n => (usize::BITS - (n - 1).leading_zeros()) as u8,
    }
}

/// Pack `indices` (each `bits` wide, MSB-first) into `buf`.
#[inline]
fn pack(buf: &mut [u8], indices: &[u32], bits: u8) {
    let mut bp = 0usize;
    for &idx in indices {
        let mut val = idx;
        for _ in 0..bits {
            buf[bp >> 3] |= (((val >> (bits - 1)) & 1) as u8) << (7 - (bp & 7));
            val <<= 1;
            bp += 1;
        }
    }
}

/// Read one `bits`-wide value from `buf` at the current `bit_pos`.
#[inline]
fn read_bits(buf: &[u8], bit_pos: &mut usize, bits: u8) -> Option<u32> {
    let mut val = 0u32;
    for _ in 0..bits {
        let byte = buf.get(*bit_pos >> 3)?;
        let bit = (byte >> (7 - (*bit_pos & 7))) & 1;
        val = (val << 1) | bit as u32;
        *bit_pos += 1;
    }
    Some(val)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkDecodeError {
    Truncated,
    InvalidHeader,
    UnknownBlock(String),
}

impl std::fmt::Display for ChunkDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "chunk data truncated"),
            Self::InvalidHeader => write!(f, "invalid chunk header"),
            Self::UnknownBlock(name) => write!(f, "unknown block: {name}"),
        }
    }
}

impl std::error::Error for ChunkDecodeError {}

pub struct BlockIterator<'a> {
    chunk: &'a Chunk,
    index: usize,
}

impl<'a> Iterator for BlockIterator<'a> {
    type Item = (ChunkCell, (usize, usize, usize));

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= CHUNK_VOLUME {
            return None;
        }

        let index = self.index;
        self.index += 1;

        let x = index / (CHUNK_SIZE * CHUNK_SIZE);
        let in_slice = index % (CHUNK_SIZE * CHUNK_SIZE);
        let z = in_slice / CHUNK_SIZE;
        let y = in_slice % CHUNK_SIZE;

        Some((self.chunk.cell_linear(index), (x, y, z)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_neighbor_offsets_cover_all_adjacent_chunks() {
        let offsets = chunk_neighbor_offsets().collect::<Vec<_>>();

        assert_eq!(offsets.len(), 26);
        assert!(!offsets.contains(&IVec3::ZERO));
        assert!(offsets.contains(&IVec3::NEG_X));
        assert!(offsets.contains(&ivec3(1, 1, 1)));
    }

    #[test]
    fn block_boundary_neighbor_offsets_cover_faces_edges_and_corners() {
        assert_eq!(chunk_neighbor_offsets_for_block(uvec3(1, 2, 3)).count(), 0);
        assert_eq!(
            chunk_neighbor_offsets_for_block(uvec3(0, 2, 3)).collect::<Vec<_>>(),
            vec![IVec3::NEG_X]
        );

        let edge_offsets = chunk_neighbor_offsets_for_block(uvec3(0, 0, 3)).collect::<Vec<_>>();
        assert_eq!(edge_offsets.len(), 3);
        assert!(edge_offsets.contains(&IVec3::NEG_X));
        assert!(edge_offsets.contains(&IVec3::NEG_Y));
        assert!(edge_offsets.contains(&ivec3(-1, -1, 0)));

        assert_eq!(chunk_neighbor_offsets_for_block(UVec3::ZERO).count(), 7);
    }

    #[test]
    fn chunk_storage_bytes_roundtrip_in_iteration_order() {
        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, BlockType::Grass.into());
        chunk.set_cell_xyz(1, 0, 0, BlockType::Dirt.into());
        chunk.set_cell_xyz(0, 0, 1, BlockType::Stone.into());
        chunk.set_cell_xyz(0, 1, 0, BlockType::OakLog.into());
        chunk.set_cell_xyz(15, 15, 15, BlockType::OakLeaves.into());

        let bytes = chunk.to_storage_bytes();
        let decoded = Chunk::try_from_storage_bytes(&bytes).unwrap();
        assert_eq!(decoded, chunk);
    }

    #[test]
    fn chunk_storage_bytes_roundtrip_all_air() {
        let chunk = Chunk::default();
        let bytes = chunk.to_storage_bytes();
        let decoded = Chunk::try_from_storage_bytes(&bytes).unwrap();
        assert_eq!(decoded, chunk);
    }

    #[test]
    fn chunk_storage_bytes_roundtrip_full_stone() {
        let chunk = Chunk::filled(BlockType::Stone.into());
        let bytes = chunk.to_storage_bytes();
        let decoded = Chunk::try_from_storage_bytes(&bytes).unwrap();
        assert_eq!(decoded, chunk);
    }

    #[test]
    fn chunk_uses_u8_palette_storage_for_common_chunks() {
        let mut chunk = Chunk::default();
        chunk.set_cell(uvec3(1, 2, 3), BlockType::Stone.into());
        chunk.set_cell(uvec3(2, 2, 3), ChunkCell::water_source());

        assert!(matches!(chunk.cell_storage(), CellStorage::U8(_)));
        assert_eq!(chunk.palette().entries().len(), 3);
        assert_eq!(chunk.hot_meta(uvec3(2, 2, 3)).fluid_level, 8);
    }

    #[test]
    fn chunk_can_set_cells_by_block_state_id() {
        let mut chunk = Chunk::default();
        let pos = uvec3(4, 5, 6);
        let state = ChunkCell::block(BlockType::Glowstone).state_id();

        assert_eq!(
            chunk.set_state(pos, state, &BLOCK_REGISTRY),
            Some(CellDelta {
                old: ChunkCell::EMPTY,
                new: BlockType::Glowstone.into(),
            })
        );
        assert_eq!(chunk.state_id(pos), state);
        assert_eq!(chunk.hot_meta(pos).light_emission, 15);
        assert_eq!(
            chunk.set_state(pos, BlockStateId(u32::MAX), &BLOCK_REGISTRY),
            None
        );
    }

    #[test]
    fn chunk_storage_bytes_roundtrip_water_fluid() {
        let mut chunk = Chunk::default();
        let pos = uvec3(2, 3, 4);
        chunk.set_cell(pos, ChunkCell::water_source());

        let bytes = chunk.to_storage_bytes();
        let decoded = Chunk::try_from_storage_bytes(&bytes).unwrap();

        assert_eq!(decoded, chunk);
        assert_eq!(decoded.get_cell(pos), ChunkCell::water_source());
    }

    #[test]
    fn fluid_storage_names_include_form_and_level() {
        let source = FluidProfile::WATER.source();
        let falling = FluidProfile::WATER.falling();

        assert_eq!(source.name(), "water_source_8");
        assert_eq!(falling.name(), "water_flow_8");
        assert_ne!(source, falling);
        assert_eq!(FluidState::from_name("water_source_8"), Some(source));
        assert_eq!(
            FluidState::from_name("water_flow_7"),
            Some(FluidState::water_flow(7))
        );
        assert_eq!(FluidState::from_name("water"), None);
        assert_eq!(FluidState::from_name("water_7"), None);
        assert_eq!(FluidState::from_name("water_flow_0"), None);
        assert_eq!(FluidState::from_name("water_flow_9"), None);
        assert_eq!(FluidState::from_name("water_source_7"), None);
    }

    #[test]
    fn water_placement_stores_fluid_cell_and_is_not_breakable() {
        let mut chunk = Chunk::default();
        let pos = uvec3(1, 2, 3);

        assert_eq!(
            chunk.place_cell(pos, ChunkCell::water_source()),
            Some(CellDelta {
                old: ChunkCell::EMPTY,
                new: ChunkCell::water_source(),
            })
        );
        assert_eq!(chunk.get_cell(pos), ChunkCell::water_source());

        assert_eq!(chunk.break_block(pos), None);
        assert_eq!(chunk.get_cell(pos), ChunkCell::water_source());
    }

    #[test]
    fn solid_block_placement_replaces_water_fluid_cell() {
        let mut chunk = Chunk::default();
        let pos = uvec3(1, 2, 3);
        chunk.place_cell(pos, ChunkCell::water_source()).unwrap();

        assert_eq!(
            chunk.place_block(pos, BlockType::Stone),
            Some(CellDelta {
                old: ChunkCell::water_source(),
                new: BlockType::Stone.into(),
            })
        );
        assert_eq!(chunk.get_cell(pos), BlockType::Stone.into());
    }

    #[test]
    fn water_flows_down_before_spreading_sideways() {
        let mut chunk = Chunk::default();
        let source = uvec3(8, 8, 8);
        chunk.set_cell(source, ChunkCell::water_source());

        assert!(chunk.step_fluids(&FluidProfile::WATER).changed);

        assert_eq!(chunk.get_cell(source), ChunkCell::water_source());
        assert_eq!(chunk.get_cell(uvec3(8, 7, 8)), ChunkCell::water_flow(8));
        assert_eq!(chunk.get_cell(uvec3(7, 8, 8)), ChunkCell::EMPTY);
        assert_eq!(chunk.get_cell(uvec3(9, 8, 8)), ChunkCell::EMPTY);
        assert_eq!(chunk.get_cell(uvec3(8, 8, 7)), ChunkCell::EMPTY);
        assert_eq!(chunk.get_cell(uvec3(8, 8, 9)), ChunkCell::EMPTY);
    }

    #[test]
    fn blocked_water_spreads_sideways_with_decay() {
        let mut chunk = Chunk::default();
        let source = uvec3(8, 1, 8);
        chunk.set_cell(source, ChunkCell::water_source());
        chunk.set_block(uvec3(8, 0, 8), BlockType::Stone);

        assert!(chunk.step_fluids(&FluidProfile::WATER).changed);

        assert_eq!(chunk.get_cell(source), ChunkCell::water_source());
        for pos in [
            uvec3(7, 1, 8),
            uvec3(9, 1, 8),
            uvec3(8, 1, 7),
            uvec3(8, 1, 9),
        ] {
            assert_eq!(chunk.get_cell(pos), ChunkCell::water_flow(7));
        }
    }

    #[test]
    fn water_flow_does_not_enter_solid_blocks() {
        let mut chunk = Chunk::default();
        let source = uvec3(8, 1, 8);
        chunk.set_cell(source, ChunkCell::water_source());
        chunk.set_block(uvec3(8, 0, 8), BlockType::Stone);
        chunk.set_block(uvec3(7, 1, 8), BlockType::Stone);

        assert!(chunk.step_fluids(&FluidProfile::WATER).changed);

        assert_eq!(chunk.get_cell(uvec3(7, 1, 8)), BlockType::Stone.into());
    }

    #[test]
    fn unreplenished_low_level_water_disappears() {
        let mut chunk = Chunk::default();
        let pos = uvec3(8, 1, 8);
        chunk.set_cell(pos, ChunkCell::water_flow(1));
        chunk.set_block(uvec3(8, 0, 8), BlockType::Stone);

        assert!(chunk.step_fluids(&FluidProfile::WATER).changed);

        assert_eq!(chunk.get_cell(pos), ChunkCell::EMPTY);
    }

    #[test]
    fn water_step_reports_changed_only_when_final_state_changes() {
        let mut chunk = Chunk::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                chunk.set_cell_xyz(x, 0, z, BlockType::Stone.into());
            }
        }
        chunk.set_cell(uvec3(7, 1, 8), ChunkCell::water_source());
        chunk.set_cell(uvec3(9, 1, 8), ChunkCell::water_source());

        let mut settled = false;
        for _ in 0..128 {
            let before = chunk.clone();
            let result = chunk.step_fluids(&FluidProfile::WATER);
            assert_eq!(result.changed, before != chunk);
            if !result.changed {
                settled = true;
                break;
            }
        }

        assert!(settled, "water should settle in a finite chunk");
    }

    #[test]
    fn chunk_storage_bytes_reject_garbled_data() {
        assert!(Chunk::try_from_storage_bytes(&[]).is_err());
    }

    #[test]
    fn chunk_storage_bytes_reject_unknown_block_name() {
        let name = b"nonexistent";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u16.to_le_bytes()); // palette_size = 1
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name);
        bytes.push(1); // bits_per_index
        bytes.resize(bytes.len() + 512, 0);

        match Chunk::try_from_storage_bytes(&bytes) {
            Err(ChunkDecodeError::UnknownBlock(n)) => assert_eq!(n, "nonexistent"),
            other => panic!("expected UnknownBlock, got {other:?}"),
        }
    }
}
