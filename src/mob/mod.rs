pub mod collide_and_slide;
pub mod controller;

use crate::player::cam::gameplay_input_active;
use avian3d::physics_transform::PhysicsTransformSystems;
use avian3d::prelude::PhysicsSystems;
use bevy::prelude::*;
use collide_and_slide::move_character_controllers;
use controller::{
    Flying, Grounded, Velocity, apply_flight_vertical_input, apply_player_movement_input,
};

/// Core physics plugin with Minecraft-like movement physics.
/// Does **not** depend on input or MouseState, so tests can use it directly.
pub struct MobPhysicsPlugin;

impl Plugin for MobPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            apply_horizontal_drag
                .in_set(MobPhysicsSystems::HorizontalDrag)
                .before(PhysicsSystems::StepSimulation),
        );

        // Run movement and post-move physics in FixedPostUpdate after the
        // physics schedule has rebuilt the spatial query pipeline, so the
        // pipeline always has up-to-date collider data.
        app.add_systems(
            FixedPostUpdate,
            (move_character_controllers, apply_vertical_physics)
                .chain()
                .after(PhysicsSystems::StepSimulation)
                .before(PhysicsTransformSystems::PositionToTransform),
        );
    }
}

/// Full controller plugin that combines input with the core physics.
pub struct MobControllerPlugin;

impl Plugin for MobControllerPlugin {
    fn build(&self, app: &mut App) {
        // Input systems run first each fixed tick (before core physics)
        app.add_systems(
            FixedUpdate,
            (apply_flight_vertical_input, apply_player_movement_input)
                .distributive_run_if(gameplay_input_active)
                .after(MobPhysicsSystems::HorizontalDrag)
                .before(PhysicsSystems::StepSimulation),
        );

        // Core physics runs after input
        app.add_plugins(MobPhysicsPlugin);
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MobPhysicsSystems {
    HorizontalDrag,
}

// ---------------------------------------------------------------------------
// Minecraft-like movement physics (assumes 20 TPS)
//   Horizontal drag: applied before input each tick, then input acceleration is added.
//   Vertical post-move: v_y = (v_y - 1.6) * 0.98
// ---------------------------------------------------------------------------

const VERTICAL_GRAVITY: f32 = 1.6; // 0.08 blocks/tick × 20 TPS
const VERTICAL_DAMPING: f32 = 0.98;
const AIR_DRAG: f32 = 0.91;
const GROUND_DRAG: f32 = 0.546; // 0.91 × 0.6 (normal block friction)

fn apply_horizontal_drag(mut query: Query<(&mut Velocity, Has<Grounded>, Has<Flying>)>) {
    for (mut velocity, grounded, flying) in &mut query {
        let drag = if grounded { GROUND_DRAG } else { AIR_DRAG };
        velocity.x *= drag;
        velocity.z *= drag;

        if flying {
            velocity.y *= drag;
        }

        // Snap near-zero velocities to exactly zero to avoid drift
        if velocity.x.abs() < 0.005 {
            velocity.x = 0.0;
        }
        if velocity.z.abs() < 0.005 {
            velocity.z = 0.0;
        }
    }
}

fn apply_vertical_physics(mut query: Query<(&mut Velocity, Has<Flying>)>) {
    for (mut velocity, flying) in &mut query {
        if !flying {
            velocity.y = (velocity.y - VERTICAL_GRAVITY) * VERTICAL_DAMPING;
        }

        if velocity.y.abs() < 0.005 {
            velocity.y = 0.0;
        }
    }
}

#[cfg(test)]
mod tests;
