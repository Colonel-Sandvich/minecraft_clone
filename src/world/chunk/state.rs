use std::{fmt, num::NonZeroU8, str::FromStr};

use bevy::prelude::Reflect;
use strum::{Display, EnumCount, EnumString};

use crate::block::{
    BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED, BLOCK_FLAG_TRANSLUCENT, BlockType, WATER_RENDER_ID,
    render_id_for_block,
};

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellStateId(pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HotCellMeta {
    pub render_id: u16,
    pub mesh_flags: u8,
    pub light_opacity: u8,
    pub light_emission: u8,
    pub fluid_level: u8,
}

impl HotCellMeta {
    pub const AIR: Self = Self {
        render_id: 0,
        mesh_flags: 0,
        light_opacity: 0,
        light_emission: 0,
        fluid_level: 0,
    };

    pub const fn for_block(block: BlockType) -> Self {
        Self {
            render_id: render_id_for_block(block),
            mesh_flags: block.mesh_flags(),
            light_opacity: block.light_opacity(),
            light_emission: block.light_emission(),
            fluid_level: 0,
        }
    }

    pub const fn water(level: u8) -> Self {
        Self {
            render_id: WATER_RENDER_ID,
            mesh_flags: BLOCK_FLAG_RENDERED | BLOCK_FLAG_TRANSLUCENT,
            light_opacity: 0,
            light_emission: 0,
            fluid_level: level,
        }
    }
}

pub const AIR_CELL_STATE_ID: CellStateId = CellStateId(0);
const FIRST_BLOCK_STATE_ID: u32 = 1;
const FIRST_FLUID_STATE_ID: u32 = FIRST_BLOCK_STATE_ID + BlockType::COUNT as u32;

#[derive(Debug, Clone, Copy, Default)]
pub struct CellRegistry;

pub const CELL_REGISTRY: CellRegistry = CellRegistry;

impl CellRegistry {
    pub fn hot_meta(&self, state: CellStateId) -> Option<HotCellMeta> {
        cell_from_state_id(state).map(ChunkCell::hot_meta)
    }

    pub fn cell(&self, state: CellStateId) -> Option<ChunkCell> {
        cell_from_state_id(state)
    }
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
    pub fn state_id(self) -> CellStateId {
        match self {
            Self::Empty => AIR_CELL_STATE_ID,
            Self::Block(block) => CellStateId(FIRST_BLOCK_STATE_ID + block as u32),
            Self::Fluid(fluid) => {
                let profile = FluidProfile::default_for_type(fluid.ty());
                debug_assert!(profile.contains(fluid));
                let level_offset = fluid.level().get() as u32 - 1;
                let form_offset = match fluid.form() {
                    FluidForm::Flowing => 0,
                    FluidForm::Source => profile.full_level.get() as u32,
                };
                CellStateId(FIRST_FLUID_STATE_ID + level_offset + form_offset)
            }
        }
    }

    pub fn from_state_id(state: CellStateId) -> Option<Self> {
        cell_from_state_id(state)
    }

    #[inline(always)]
    pub const fn hot_meta(self) -> HotCellMeta {
        match self {
            Self::Empty => HotCellMeta::AIR,
            Self::Block(block) => HotCellMeta::for_block(block),
            Self::Fluid(fluid) => HotCellMeta::water(fluid.level().get()),
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
            Self::Block(block) => block.name(),
            Self::Fluid(fluid) => fluid.name(),
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

fn cell_from_state_id(state: CellStateId) -> Option<ChunkCell> {
    let raw = state.0;
    if raw == AIR_CELL_STATE_ID.0 {
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
