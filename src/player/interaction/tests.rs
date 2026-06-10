use super::*;
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

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

fn block_pos(block: UVec3) -> BlockPos {
    BlockPos {
        chunk: IVec3::ZERO,
        block,
    }
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

    assert!(!chunk.place_block(pos, BlockType::Air));
    assert_eq!(chunk.get(pos), BlockType::Air);
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
