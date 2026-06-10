use avian3d::collider_tree::ColliderTreeDiagnostics;
use avian3d::collision::CollisionDiagnostics;
use avian3d::dynamics::solver::SolverDiagnostics;
use avian3d::prelude::*;
use avian3d::spatial_query::SpatialQueryDiagnostics;
use bevy::prelude::*;
use bevy::scene::ScenePlugin;
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

use super::MobPhysicsPlugin;
use super::controller::{
    CharacterController, Grounded, Velocity, apply_jump_impulse, horizontal_velocity_delta,
    world_move_direction,
};
use crate::player::control::{KeyBindings, PlayerMovementIntent};

const PLAYER_HH: f32 = 0.9; // half-height
const PLAYER_HW: f32 = 0.3; // half-width
const SKIN: f32 = 0.015;
const EPSILON: f32 = 0.02;
const STAT_EPSILON: f32 = 0.01;
const DT: f32 = 1.0 / 20.0;
const GROUND_DRAG: f32 = 0.546;
const AIR_DRAG: f32 = 0.91;
const WALK_ACCEL_PER_TICK: f32 = 0.098;
const SPRINT_ACCEL_PER_TICK: f32 = 0.1274;
const AIR_WALK_ACCEL_PER_TICK: f32 = 0.02;
const AIR_SPRINT_ACCEL_PER_TICK: f32 = 0.026;
const SNAP_THRESHOLD: f32 = 0.005;

struct MovementTest {
    app: App,
    player: Entity,
}

impl MovementTest {
    fn new(player_pos: Vec3) -> Self {
        Self::new_with_interpolation(player_pos, false, Duration::from_secs_f32(1.0 / 20.0))
    }

    fn new_with_interpolation(
        player_pos: Vec3,
        interpolate: bool,
        frame_duration: Duration,
    ) -> Self {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ScenePlugin::default(),
        ))
        .init_asset::<Mesh>();
        app.add_plugins(PhysicsPlugins::default());
        app.insert_resource(Time::<Fixed>::from_hz(20.0))
            .insert_resource(TimeUpdateStrategy::ManualDuration(frame_duration))
            .add_plugins(MobPhysicsPlugin);

        // App::update() does not drive the normal runner lifecycle in tests.
        // Finish explicitly so plugins that add sub-plugins in `finish`, such
        // as bevy_transform_interpolation's easing plugin, are active.
        app.finish();
        app.cleanup();

        // App::update() does not call Plugin::finish() in test environments,
        // but several physics diagnostics resources are initialized in finish().
        // Since tests use app.update() (not app.run()), we need to explicitly
        // initialize these resources here.
        app.init_resource::<CollisionDiagnostics>();
        app.init_resource::<SolverDiagnostics>();
        app.init_resource::<SpatialQueryDiagnostics>();
        app.init_resource::<ColliderTreeDiagnostics>();

        let player = app
            .world_mut()
            .spawn((
                CharacterController,
                Collider::cuboid(0.6, 1.8, 0.6),
                Position::new(player_pos),
                Transform::from_translation(player_pos),
            ))
            .id();

        if interpolate {
            app.world_mut()
                .entity_mut(player)
                .insert(TransformInterpolation);
        }

        MovementTest { app, player }
    }

    fn set_velocity(&mut self, vel: Vec3) {
        *self
            .app
            .world_mut()
            .entity_mut(self.player)
            .get_mut::<Velocity>()
            .unwrap() = Velocity(vel);
    }

    fn tick(&mut self) {
        self.app.update();
    }

    fn tick_n(&mut self, n: usize) {
        for _ in 0..n {
            self.tick();
        }
    }

    fn frame_n_with_horizontal_velocity(&mut self, n: usize, vel: Vec3) {
        for _ in 0..n {
            let mut current = self.vel();
            current.x = vel.x;
            current.z = vel.z;
            self.set_velocity(current);
            self.tick();
        }
    }

    fn tick_n_with_horizontal_velocity(&mut self, n: usize, vel: Vec3) {
        for _ in 0..n {
            let mut current = self.vel();
            current.x = vel.x;
            current.z = vel.z;
            self.set_velocity(current);
            self.tick();
        }
    }

    fn warmup_query_pipeline(&mut self) {
        self.set_velocity(Vec3::ZERO);
        self.tick();
        // Extra tick to ensure colliders are fully registered in spatial query
        self.tick();
    }

    fn set_pos(&mut self, pos: Vec3) {
        let mut entity = self.app.world_mut().entity_mut(self.player);
        entity.get_mut::<Position>().unwrap().0 = pos;
        entity.get_mut::<Transform>().unwrap().translation = pos;
    }

    fn pos(&self) -> Vec3 {
        self.sim_pos()
    }

    fn sim_pos(&self) -> Vec3 {
        self.app.world().get::<Position>(self.player).unwrap().0
    }

    fn vel(&self) -> Vec3 {
        self.app.world().get::<Velocity>(self.player).unwrap().0
    }

    fn grounded(&self) -> bool {
        self.app.world().entity(self.player).contains::<Grounded>()
    }

    fn spawn_static(&mut self, pos: Vec3, size: Vec3) {
        self.app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(size.x, size.y, size.z),
            Transform::from_translation(pos),
        ));
    }

    fn remove_player_collider(&mut self) {
        self.app
            .world_mut()
            .entity_mut(self.player)
            .remove::<Collider>();
    }

    fn insert_grounded(&mut self) {
        self.app
            .world_mut()
            .entity_mut(self.player)
            .insert(Grounded);
    }
}

fn floor_top_y() -> f32 {
    0.5
}

fn resting_y() -> f32 {
    floor_top_y() + PLAYER_HH + SKIN
}

fn simulate_straight_horizontal(ticks: usize, drag: f32, acceleration_per_tick: f32) -> (f32, f32) {
    let mut speed_blocks_per_sec = 0.0;
    let mut distance = 0.0;
    for _ in 0..ticks {
        speed_blocks_per_sec = speed_blocks_per_sec * drag + acceleration_per_tick * 20.0;
        if speed_blocks_per_sec.abs() < SNAP_THRESHOLD {
            speed_blocks_per_sec = 0.0;
        }
        distance += speed_blocks_per_sec / 20.0;
    }
    (distance, speed_blocks_per_sec)
}

fn simulate_release(mut speed_blocks_per_sec: f32, drag: f32) -> (usize, f32) {
    let mut distance = 0.0;
    for tick in 1..=120 {
        speed_blocks_per_sec *= drag;
        if speed_blocks_per_sec.abs() < SNAP_THRESHOLD {
            speed_blocks_per_sec = 0.0;
        }
        distance += speed_blocks_per_sec * DT;
        if speed_blocks_per_sec == 0.0 {
            return (tick, distance);
        }
    }

    panic!("release simulation did not snap to zero");
}

fn simulate_jump_vertical() -> (f32, usize, usize) {
    let mut velocity_y = 8.4;
    let mut height = 0.0;
    let mut peak = 0.0;
    let mut peak_tick = 0;

    for tick in 1..=80 {
        height += velocity_y * DT;
        if height > peak {
            peak = height;
            peak_tick = tick;
        }

        velocity_y = (velocity_y - 1.6) * 0.98;
        if velocity_y.abs() < SNAP_THRESHOLD {
            velocity_y = 0.0;
        }

        if tick > 1 && height <= 0.0 {
            return (peak, peak_tick, tick);
        }
    }

    panic!("jump simulation did not land");
}

#[test]
fn horizontal_velocity_delta_uses_vanilla_air_control() {
    let direction = Vec3::Z;
    let ground = horizontal_velocity_delta(direction, 39.2, 8.0, 0.05, true, false, false);
    let air = horizontal_velocity_delta(direction, 39.2, 8.0, 0.05, false, false, false);
    let sprint_air = horizontal_velocity_delta(direction, 39.2, 8.0, 0.05, false, false, true);

    assert!((ground.z - 1.96).abs() < STAT_EPSILON);
    assert!((air.z - 0.4).abs() < STAT_EPSILON);
    assert!((sprint_air.z - 0.52).abs() < STAT_EPSILON);
}

#[test]
fn jump_impulse_sets_vertical_velocity_and_adds_sprint_boost() {
    let mut velocity = Vec3::new(0.0, -1.6, 2.548);
    apply_jump_impulse(&mut velocity, Vec3::Z, 8.4, true);

    assert!((velocity.y - 8.4).abs() < STAT_EPSILON);
    assert!((velocity.z - 6.548).abs() < STAT_EPSILON);
}

#[test]
fn diagonal_input_is_normalized() {
    let forward = horizontal_velocity_delta(Vec3::Z, 39.2, 8.0, 0.05, true, false, false);
    let diagonal = horizontal_velocity_delta(
        Vec3::new(1.0, 0.0, 1.0).normalize(),
        39.2,
        8.0,
        0.05,
        true,
        false,
        false,
    );

    assert!((diagonal.length() - forward.length()).abs() < STAT_EPSILON);
}

#[test]
fn pitched_camera_does_not_skew_diagonal_input() {
    let near_down_forward = Vec3::new(0.01, -1.0, 0.0).normalize();
    let direction = world_move_direction(near_down_forward, Vec3::Z, Vec3::new(-1.0, 0.0, -1.0));

    let expected = (Vec3::NEG_X + Vec3::NEG_Z).normalize();
    assert!(
        direction.dot(expected) > 0.999,
        "direction {direction:?} should preserve backward-left input while looking down"
    );
}

#[test]
fn keybindings_build_player_movement_intent() {
    let key_bindings = KeyBindings::default();
    let mut keys = ButtonInput::default();
    keys.press(key_bindings.move_forward);
    keys.press(key_bindings.move_left);
    keys.press(key_bindings.move_ascend);
    keys.press(key_bindings.sprint);

    let intent = key_bindings.movement_intent(&keys);

    assert_eq!(intent.local_move_axis, Vec3::new(-1.0, 1.0, 1.0));
    assert!(intent.jump);
    assert!(intent.sprint);
    assert!(intent.wants_forward_sprint());
}

#[test]
fn sprint_intent_requires_forward_input() {
    assert!(
        PlayerMovementIntent {
            local_move_axis: Vec3::new(0.0, 0.0, 1.0),
            sprint: true,
            ..default()
        }
        .wants_forward_sprint()
    );
    assert!(
        PlayerMovementIntent {
            local_move_axis: Vec3::new(-1.0, 0.0, 1.0),
            sprint: true,
            ..default()
        }
        .wants_forward_sprint()
    );
    assert!(
        !PlayerMovementIntent {
            local_move_axis: Vec3::new(0.0, 0.0, -1.0),
            sprint: true,
            ..default()
        }
        .wants_forward_sprint()
    );
    assert!(
        !PlayerMovementIntent {
            local_move_axis: Vec3::new(-1.0, 0.0, 0.0),
            sprint: true,
            ..default()
        }
        .wants_forward_sprint()
    );
    assert!(
        !PlayerMovementIntent {
            local_move_axis: Vec3::new(0.0, 0.0, 1.0),
            sprint: false,
            ..default()
        }
        .wants_forward_sprint()
    );
}

#[test]
fn vanilla_ground_acceleration_curve_reference_stats() {
    // Our velocity is blocks/sec. The per-tick acceleration deltas below are
    // Vanilla's 0.098/0.1274/0.02/0.026 blocks/tick converted to blocks/sec.
    let (walk_5t, walk_speed_5t) =
        simulate_straight_horizontal(5, GROUND_DRAG, WALK_ACCEL_PER_TICK);
    let (walk_7t, walk_speed_7t) =
        simulate_straight_horizontal(7, GROUND_DRAG, WALK_ACCEL_PER_TICK);
    let (walk_1s, walk_speed_1s) =
        simulate_straight_horizontal(20, GROUND_DRAG, WALK_ACCEL_PER_TICK);
    let (walk_2s, walk_speed_2s) =
        simulate_straight_horizontal(40, GROUND_DRAG, WALK_ACCEL_PER_TICK);
    let (walk_3s, walk_speed_3s) =
        simulate_straight_horizontal(60, GROUND_DRAG, WALK_ACCEL_PER_TICK);
    let (sprint_1s, sprint_speed_1s) =
        simulate_straight_horizontal(20, GROUND_DRAG, SPRINT_ACCEL_PER_TICK);
    let (sprint_2s, sprint_speed_2s) =
        simulate_straight_horizontal(40, GROUND_DRAG, SPRINT_ACCEL_PER_TICK);
    let (sprint_3s, sprint_speed_3s) =
        simulate_straight_horizontal(60, GROUND_DRAG, SPRINT_ACCEL_PER_TICK);

    assert!((walk_5t - 0.83).abs() < STAT_EPSILON);
    assert!((walk_speed_5t - 4.11).abs() < STAT_EPSILON);
    assert!((walk_7t - 1.26).abs() < STAT_EPSILON);
    assert!((walk_speed_7t - 4.25).abs() < STAT_EPSILON);

    assert!((walk_1s - 4.06).abs() < STAT_EPSILON);
    assert!((walk_speed_1s - 4.32).abs() < STAT_EPSILON);
    assert!((walk_2s - 8.37).abs() < STAT_EPSILON);
    assert!((walk_speed_2s - 4.32).abs() < STAT_EPSILON);
    assert!((walk_3s - 12.69).abs() < STAT_EPSILON);
    assert!((walk_speed_3s - 4.32).abs() < STAT_EPSILON);

    assert!((sprint_1s - 5.27).abs() < STAT_EPSILON);
    assert!((sprint_speed_1s - 5.61).abs() < STAT_EPSILON);
    assert!((sprint_2s - 10.89).abs() < STAT_EPSILON);
    assert!((sprint_speed_2s - 5.61).abs() < STAT_EPSILON);
    assert!((sprint_3s - 16.50).abs() < STAT_EPSILON);
    assert!((sprint_speed_3s - 5.61).abs() < STAT_EPSILON);
}

#[test]
fn vanilla_air_acceleration_and_terminal_reference_stats() {
    let (air_walk_1s, air_walk_speed_1s) =
        simulate_straight_horizontal(20, AIR_DRAG, AIR_WALK_ACCEL_PER_TICK);
    let (_, air_walk_speed_30s) =
        simulate_straight_horizontal(600, AIR_DRAG, AIR_WALK_ACCEL_PER_TICK);
    let (air_sprint_1s, air_sprint_speed_1s) =
        simulate_straight_horizontal(20, AIR_DRAG, AIR_SPRINT_ACCEL_PER_TICK);
    let (_, air_sprint_speed_30s) =
        simulate_straight_horizontal(600, AIR_DRAG, AIR_SPRINT_ACCEL_PER_TICK);

    assert!((air_walk_1s - 2.54).abs() < STAT_EPSILON);
    assert!((air_walk_speed_1s - 3.77).abs() < STAT_EPSILON);
    assert!((air_walk_speed_30s - 4.44).abs() < STAT_EPSILON);
    assert!((air_sprint_1s - 3.30).abs() < STAT_EPSILON);
    assert!((air_sprint_speed_1s - 4.90).abs() < STAT_EPSILON);
    assert!((air_sprint_speed_30s - 5.78).abs() < STAT_EPSILON);
}

#[test]
fn vanilla_release_deceleration_reference_stats() {
    let walk_terminal = WALK_ACCEL_PER_TICK * 20.0 / (1.0 - GROUND_DRAG);
    let sprint_terminal = SPRINT_ACCEL_PER_TICK * 20.0 / (1.0 - GROUND_DRAG);
    let (walk_ticks, walk_slide) = simulate_release(walk_terminal, GROUND_DRAG);
    let (sprint_ticks, sprint_slide) = simulate_release(sprint_terminal, GROUND_DRAG);

    assert_eq!(walk_ticks, 12);
    assert!((walk_slide - 0.26).abs() < STAT_EPSILON);
    assert_eq!(sprint_ticks, 12);
    assert!((sprint_slide - 0.34).abs() < STAT_EPSILON);
}

#[test]
fn vanilla_jump_vertical_reference_stats() {
    let (peak, peak_tick, landing_tick) = simulate_jump_vertical();

    assert!((peak - 1.25).abs() < STAT_EPSILON);
    assert_eq!(peak_tick, 6);
    assert_eq!(landing_tick, 12);
}

#[test]
fn ecs_release_from_walk_terminal_decays_and_snaps() {
    let mut t = MovementTest::new(Vec3::new(0.0, resting_y(), 0.0));
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    t.warmup_query_pipeline();
    t.set_pos(Vec3::new(0.0, resting_y(), 0.0));
    t.set_velocity(Vec3::new(
        0.0,
        0.0,
        WALK_ACCEL_PER_TICK * 20.0 / (1.0 - GROUND_DRAG),
    ));

    let start = t.sim_pos();
    t.tick_n(20);

    let distance = t.sim_pos().z - start.z;
    let horizontal_speed = vec2(t.vel().x, t.vel().z).length();
    assert!(
        horizontal_speed <= SNAP_THRESHOLD,
        "horizontal_speed={horizontal_speed} distance={distance} grounded={}",
        t.grounded()
    );
    assert!(
        distance < 0.75,
        "release slide distance should remain bounded; distance={distance}"
    );
}

#[test]
fn fixed_movement_at_60_fps_without_interpolation_matches_expected_speed() {
    let mut t = MovementTest::new_with_interpolation(
        Vec3::new(0.0, resting_y(), 0.0),
        false,
        Duration::from_secs_f32(1.0 / 60.0),
    );
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    t.warmup_query_pipeline();
    t.set_pos(Vec3::new(0.0, resting_y(), 0.0));

    let start = t.sim_pos();
    t.frame_n_with_horizontal_velocity(60, Vec3::new(4.0, 0.0, 0.0));

    let distance = t.sim_pos().x - start.x;
    assert!(
        (2.2..2.45).contains(&distance),
        "control: fixed movement should remain in the established 20 TPS range; distance={distance} pos={:?} vel={:?}",
        t.sim_pos(),
        t.vel(),
    );
}

#[test]
fn interpolation_does_not_feed_eased_render_transform_back_into_fixed_movement() {
    let mut control = MovementTest::new_with_interpolation(
        Vec3::new(0.0, resting_y(), 0.0),
        false,
        Duration::from_secs_f32(1.0 / 60.0),
    );
    control.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    control.warmup_query_pipeline();
    control.set_pos(Vec3::new(0.0, resting_y(), 0.0));
    let control_start = control.sim_pos();
    control.frame_n_with_horizontal_velocity(60, Vec3::new(4.0, 0.0, 0.0));
    let control_distance = control.sim_pos().x - control_start.x;

    let mut interpolated = MovementTest::new_with_interpolation(
        Vec3::new(0.0, resting_y(), 0.0),
        true,
        Duration::from_secs_f32(1.0 / 60.0),
    );
    interpolated.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    interpolated.warmup_query_pipeline();
    interpolated.set_pos(Vec3::new(0.0, resting_y(), 0.0));

    let interpolated_start = interpolated.sim_pos();
    interpolated.frame_n_with_horizontal_velocity(60, Vec3::new(4.0, 0.0, 0.0));

    let interpolated_distance = interpolated.sim_pos().x - interpolated_start.x;
    assert!(
        (interpolated_distance - control_distance).abs() < 0.01,
        "interpolation should not change the observed fixed-step displacement; control_distance={control_distance} interpolated_distance={interpolated_distance} control_pos={:?} interpolated_pos={:?}",
        control.sim_pos(),
        interpolated.sim_pos(),
    );
}

#[test]
fn colliderless_controller_moves_without_collision() {
    let mut t = MovementTest::new(Vec3::new(0.0, resting_y(), 0.0));
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    t.warmup_query_pipeline();
    t.set_pos(Vec3::new(0.0, resting_y(), 0.0));

    t.insert_grounded();
    t.remove_player_collider();
    t.set_velocity(Vec3::new(4.0, 0.0, 0.0));
    let start = t.sim_pos();
    t.tick();

    assert!(
        t.sim_pos().x > start.x,
        "colliderless controller should still move"
    );
    assert!(
        !t.grounded(),
        "colliderless controller should not remain grounded"
    );
}

#[test]
fn ecs_air_acceleration_from_zero_stays_bounded() {
    let mut speed = 0.0;
    for _ in 0..600 {
        speed = speed * AIR_DRAG + AIR_WALK_ACCEL_PER_TICK * 20.0;
    }

    assert!(
        speed < 4.45,
        "air walk speed should converge below 4.45, got {speed}"
    );
}

#[test]
fn ecs_jump_vertical_arc_matches_reference_peak() {
    let mut t = MovementTest::new(Vec3::new(0.0, resting_y(), 0.0));
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    t.warmup_query_pipeline();
    t.set_pos(Vec3::new(0.0, resting_y(), 0.0));
    t.set_velocity(Vec3::new(0.0, 8.4, 0.0));

    let launch_y = t.sim_pos().y;
    let mut peak = launch_y;
    let mut landed = false;
    for _ in 0..30 {
        t.tick();
        peak = peak.max(t.sim_pos().y);
        if t.grounded() && t.sim_pos().y <= launch_y + EPSILON {
            landed = true;
            break;
        }
    }

    let jump_height = peak - launch_y;
    assert!(landed, "jump should land within 30 ticks");
    assert!(
        (jump_height - 1.25).abs() < EPSILON,
        "jump_height={jump_height} peak={peak} launch_y={launch_y}"
    );
}

#[test]
fn grounded_on_flat_floor() {
    let mut t = MovementTest::new(Vec3::new(0.0, 2.0, 0.0));
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    t.warmup_query_pipeline();

    t.set_velocity(Vec3::ZERO);
    t.tick_n(30);

    assert!(t.grounded(), "should be grounded");
    let expected = resting_y();
    assert!(
        (t.pos().y - expected).abs() < EPSILON,
        "player y {:.4} should be near {:.4}",
        t.pos().y,
        expected
    );
}
#[test]
fn walk_into_wall_slides_along() {
    let mut t = MovementTest::new(Vec3::new(0.0, 2.0, 0.0));
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 100.0));
    // wall at x=5, blocking +X (very long in Z so player doesn't slide past it)
    t.spawn_static(Vec3::new(5.0, 1.5, 0.0), Vec3::new(0.5, 3.0, 50.0));

    t.warmup_query_pipeline();

    // Continuous diagonal input: +X into wall, +Z to test sliding.
    t.tick_n_with_horizontal_velocity(120, Vec3::new(4.0, 0.0, 2.0));

    let pos = t.pos();
    // X blocked at wall (wall left face at 4.75) minus player half-width and skin
    assert!(
        pos.x <= 4.75 - PLAYER_HW - SKIN + EPSILON,
        "player x {:.4} should be blocked near wall left face",
        pos.x
    );
    // Z should have moved forward
    assert!(
        pos.z > 2.0,
        "player z {:.4} should have moved along wall",
        pos.z
    );
    assert!(t.grounded(), "should remain grounded");
}

#[test]
fn walk_off_edge_goes_airborne() {
    let mut t = MovementTest::new(Vec3::new(0.0, 2.0, 0.0));
    // platform from x=-10 to x=3 (center at x=-3.5, size x=13)
    t.spawn_static(Vec3::new(-3.5, 0.0, 0.0), Vec3::new(13.0, 1.0, 20.0));
    t.warmup_query_pipeline();

    // Walk rightwards off the platform edge at x=3
    t.tick_n_with_horizontal_velocity(90, Vec3::new(8.0, 0.0, 0.0));

    assert!(!t.grounded(), "should have fallen off edge");
    assert!(
        t.pos().y < floor_top_y(),
        "player y {:.4} should be below floor top",
        t.pos().y
    );
}

#[test]
fn corner_slide() {
    let mut t = MovementTest::new(Vec3::new(-3.0, 2.0, -3.0));
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    // wall at x=3, extending in +Z direction, blocking +X
    t.spawn_static(Vec3::new(3.0, 1.5, 0.0), Vec3::new(0.5, 3.0, 10.0));
    // perpendicular wall at z=3, blocking +Z
    t.spawn_static(Vec3::new(0.0, 1.5, 3.0), Vec3::new(10.0, 3.0, 0.5));
    t.warmup_query_pipeline();

    // Continuous diagonal input into the convex corner.
    t.tick_n_with_horizontal_velocity(90, Vec3::new(8.0, 0.0, 8.0));

    let pos = t.pos();
    // Walls at x=3 (left face 2.75) and z=3 (front face 2.75). The player
    // should be contained by both faces, not tunnel into either wall.
    let max_coord = 2.75 - PLAYER_HW - SKIN + EPSILON;
    assert!(
        pos.x <= max_coord && pos.z <= max_coord,
        "player ({:.3}, {:.3}) should not pass corner walls",
        pos.x,
        pos.z
    );
    let slid_x = (pos.x - (2.75 - PLAYER_HW - SKIN)).abs() < EPSILON * 2.0;
    let slid_z = (pos.z - (2.75 - PLAYER_HW - SKIN)).abs() < EPSILON * 2.0;
    assert!(
        slid_x && slid_z,
        "player ({:.3}, {:.3}) should finish tucked against the convex corner",
        pos.x,
        pos.z
    );
}

#[test]
fn autostep_onto_block() {
    let mut t = MovementTest::new(Vec3::new(0.0, 2.0, 0.0));
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    // step block 1.0 wide, 0.6 tall, at x=4
    t.spawn_static(Vec3::new(4.5, 0.3, 0.0), Vec3::new(1.0, 0.6, 1.0));
    t.warmup_query_pipeline();

    // Start at floor level so the player's Y-range already overlaps the
    // step-block's Y-range when the player reaches it horizontally.
    // Starting above means the player flies over the 0.6-tall block.
    let floor_rest_y = resting_y();
    t.set_pos(Vec3::new(0.0, floor_rest_y, 0.0));

    // Continuous movement into the 0.6m step should choose the stepped path
    // over the blocked flat path. Stop as soon as the step-up is observed so
    // the test cannot pass/fail based on later walking off the far edge.
    let expected_y = 0.6 + PLAYER_HH + SKIN;
    let mut stepped = None;
    for _ in 0..60 {
        t.tick_n_with_horizontal_velocity(1, Vec3::new(10.0, 0.0, 0.0));
        let pos = t.pos();
        if pos.x > 4.35 && (pos.y - expected_y).abs() < EPSILON * 2.0 {
            stepped = Some((pos, t.vel()));
            break;
        }
    }
    let (pos, vel) = stepped.unwrap_or_else(|| (t.pos(), t.vel()));

    assert!(
        pos.x > 4.35,
        "player x {:.4} should be clearly on top of the step, not just past the blocked edge",
        pos.x
    );
    assert!(
        (pos.y - expected_y).abs() < EPSILON * 2.0,
        "player y {:.4} should be ~{:.4} on top of the step, not floor-height {:.4}",
        pos.y,
        expected_y,
        resting_y()
    );
    assert!(
        vel.x > 1.0,
        "successful autostep should preserve horizontal velocity; got {:?}",
        vel
    );
}

#[test]
fn ceiling_bump_zeroes_velocity() {
    let mut t = MovementTest::new(Vec3::new(0.0, 2.0, 0.0));
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    // ceiling slab: bottom at y=3.6, top at y=4.1
    t.spawn_static(Vec3::new(0.0, 3.85, 0.0), Vec3::new(10.0, 0.5, 10.0));
    t.warmup_query_pipeline();
    t.set_pos(Vec3::new(0.0, resting_y(), 0.0));

    // A plausible strong jump/updraft, integrated over several ticks, should
    // bump the ceiling without tunneling through it.
    t.set_velocity(Vec3::new(0.0, 8.0, 0.0));
    let mut hit_ceiling = false;
    for _ in 0..60 {
        t.tick();
        let top = t.pos().y + PLAYER_HH;
        let ceiling_bottom = 3.85 - 0.25;
        if top > 3.4 && top <= ceiling_bottom + SKIN + EPSILON && t.vel().y <= EPSILON {
            hit_ceiling = true;
            break;
        }
    }

    assert!(
        hit_ceiling,
        "expected to hit ceiling and zero vertical velocity"
    );

    // player top should not exceed ceiling bottom + skin
    let pos = t.pos();
    let player_top = pos.y + PLAYER_HH;
    let ceiling_bottom = 3.85 - 0.25;
    assert!(
        player_top <= ceiling_bottom + SKIN + EPSILON,
        "player top {:.4} should not exceed ceiling bottom {:.4}",
        player_top,
        ceiling_bottom
    );
    // velocity.y should not be upward (post-move gravity/drag may make it negative)
    assert!(
        t.vel().y <= EPSILON,
        "velocity.y {:.4} should be <= 0 after ceiling hit (post-move physics may pull down)",
        t.vel().y
    );
    assert!(!t.grounded(), "should not be grounded (was moving up)");
}

#[test]
fn two_bounce_hallway() {
    let mut t = MovementTest::new(Vec3::new(0.0, 2.0, 0.0));
    t.spawn_static(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 20.0));
    // hallway walls: left at x=-2, right at x=2
    t.spawn_static(Vec3::new(-2.0, 1.5, 0.0), Vec3::new(0.5, 3.0, 10.0));
    t.spawn_static(Vec3::new(2.0, 1.5, 0.0), Vec3::new(0.5, 3.0, 10.0));
    t.warmup_query_pipeline();

    // Continuous strong movement toward the right wall.
    t.tick_n_with_horizontal_velocity(90, Vec3::new(8.0, 0.0, 0.0));

    let pos = t.pos();
    // player should be contained within hallway
    let right_wall_face = 2.0 - 0.25;
    let left_wall_face = -2.0 + 0.25;
    assert!(
        pos.x <= right_wall_face - PLAYER_HW - SKIN + EPSILON,
        "player x {:.4} should not pass right wall",
        pos.x
    );
    assert!(
        pos.x >= left_wall_face + PLAYER_HW + SKIN - EPSILON,
        "player x {:.4} should not pass left wall",
        pos.x
    );
    assert!(
        (pos.x - (right_wall_face - PLAYER_HW - SKIN)).abs() < EPSILON * 2.0,
        "player x {:.4} should be stopped against the right hallway wall",
        pos.x
    );
}
