use bevy::prelude::*;

use crate::block::{BlockStateId, BlockType};

use super::*;

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
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(name.len() as u8);
    bytes.extend_from_slice(name);
    bytes.push(1);
    bytes.resize(bytes.len() + 512, 0);

    match Chunk::try_from_storage_bytes(&bytes) {
        Err(ChunkDecodeError::UnknownBlock(name)) => assert_eq!(name, "nonexistent"),
        other => panic!("expected UnknownBlock, got {other:?}"),
    }
}
