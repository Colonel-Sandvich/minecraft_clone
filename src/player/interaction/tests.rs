use crate::{
    block::BlockType,
    world::chunk::{ChunkBlockPos, ChunkCell, ChunkPos, LocalBlockPos},
};

use super::*;
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

/// Test helper: counts interaction requests sent via bevy messages
/// so that tests can assert the count of pick/break/place actions.
#[derive(Resource, Default)]
struct InteractionCounts {
    pick: usize,
    break_block: usize,
    place: usize,
}

fn count_interaction_requests(
    mut requests: MessageReader<BlockInteractionRequest>,
    mut counts: ResMut<InteractionCounts>,
) {
    for request in requests.read() {
        match request.kind {
            BlockInteractionKind::Pick => counts.pick += 1,
            BlockInteractionKind::Break => counts.break_block += 1,
            BlockInteractionKind::Place => counts.place += 1,
        }
    }
}

fn block_pos(block: UVec3) -> ChunkBlockPos {
    ChunkBlockPos::new(ChunkPos::ZERO, LocalBlockPos::try_from(block).unwrap())
}

fn target() -> BlockTarget {
    BlockTarget {
        hit_block: block_pos(uvec3(1, 2, 3)),
        adjacent_block: block_pos(uvec3(1, 3, 3)),
    }
}

#[test]
fn block_face_normal_uses_dominant_axis() {
    assert_eq!(block_face_normal(vec3(0.9999, 0.0001, 0.0)), ivec3(1, 0, 0));
    assert_eq!(
        block_face_normal(vec3(-0.9999, 0.0001, 0.0)),
        ivec3(-1, 0, 0)
    );
    assert_eq!(
        block_face_normal(vec3(0.001, -0.998, 0.002)),
        ivec3(0, -1, 0)
    );
    assert_eq!(block_face_normal(vec3(0.001, 0.002, 0.998)), ivec3(0, 0, 1));
}

#[test]
fn placing_air_is_ignored() {
    let mut chunk = Chunk::default();
    let pos = uvec3(1, 2, 3);

    assert!(chunk.place_cell(pos, ChunkCell::EMPTY).is_none());
    assert_eq!(chunk.get_cell(pos), ChunkCell::EMPTY);
}

#[test]
fn water_placement_does_not_require_actor_clearance() {
    assert!(!placement_requires_actor_clearance(
        ChunkCell::water_source()
    ));
    assert!(placement_requires_actor_clearance(BlockType::Stone.into()));
    assert!(placement_requires_actor_clearance(BlockType::Ice.into()));
}

fn app_with_request_emitter() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(Time::<Fixed>::from_hz(20.0))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 60.0,
        )))
        .insert_resource(ButtonInput::<MouseButton>::default())
        .insert_resource(CurrentBlockTarget(Some(target())))
        .init_resource::<InteractionCounts>()
        .add_message::<BlockInteractionRequest>()
        .add_systems(
            FixedUpdate,
            emit_block_interaction_requests.in_set(BlockInteractionSystems::EmitRequests),
        )
        .add_systems(
            FixedUpdate,
            count_interaction_requests.after(BlockInteractionSystems::EmitRequests),
        );

    app
}

#[test]
fn held_buttons_emit_one_request_per_action_per_fixed_tick() {
    let mut app = app_with_request_emitter();
    {
        let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        buttons.press(MouseButton::Middle);
        buttons.press(MouseButton::Left);
        buttons.press(MouseButton::Right);
    }

    // The first app update initializes time, then 60 simulated render
    // frames at 1/60s should produce exactly 20 fixed ticks at 20 Hz.
    for _ in 0..61 {
        app.update();
    }

    let counts = app.world().resource::<InteractionCounts>();
    assert_eq!(counts.pick, 20);
    assert_eq!(counts.break_block, 20);
    assert_eq!(counts.place, 20);
}

#[test]
fn interaction_request_uses_action_specific_block_position() {
    let target = target();

    assert_eq!(
        BlockInteractionRequest {
            kind: BlockInteractionKind::Pick,
            target,
        }
        .block_pos(),
        target.hit_block
    );
    assert_eq!(
        BlockInteractionRequest {
            kind: BlockInteractionKind::Break,
            target,
        }
        .block_pos(),
        target.hit_block
    );
    assert_eq!(
        BlockInteractionRequest {
            kind: BlockInteractionKind::Place,
            target,
        }
        .block_pos(),
        target.adjacent_block
    );
}

#[test]
fn clearing_interaction_state_discards_target_and_pending_requests() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(CurrentBlockTarget(Some(target())))
        .add_message::<BlockInteractionRequest>()
        .add_systems(Update, clear_block_interaction_state);
    app.world_mut()
        .resource_mut::<Messages<BlockInteractionRequest>>()
        .write(BlockInteractionRequest {
            kind: BlockInteractionKind::Break,
            target: target(),
        });

    app.update();

    assert_eq!(app.world().resource::<CurrentBlockTarget>().0, None);
    assert!(
        app.world()
            .resource::<Messages<BlockInteractionRequest>>()
            .is_empty()
    );
}
