use std::f32::consts::PI;

use super::{
    PLAYER_HEIGHT, PLAYER_LENGTH, PLAYER_WIDTH, Player,
    cam::{MouseCam, MouseSettings},
};
use avian3d::prelude::{Collider, Position, RigidBody, TransformInterpolation};
use bevy::{camera::visibility::NoCpuCulling, prelude::*, render::view::NoIndirectDrawing};

use crate::{
    game_state::GameState,
    mob::controller::{CharacterController, FlyController},
    world::{
        ACTOR_COLLISION_LAYERS,
        chunk::CHUNK_SIZE,
        dimension::Dimension,
        generation::{WorldMetadata, terrain_height},
    },
};

pub struct SpawnPlayerPlugin;

impl Plugin for SpawnPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnExit(GameState::GenWorld), spawn_player);
    }
}

pub const EYELINE: f32 = 0.1;

fn spawn_player(
    mut commands: Commands,
    dimension_q: Query<Entity, With<Dimension>>,
    metadata: Res<WorldMetadata>,
) {
    let spawn_point = spawn_point(&metadata);

    commands.spawn((
        ChildOf(dimension_q.single().unwrap()),
        Player::default(),
        RigidBody::Kinematic,
        Position::new(spawn_point),
        Transform::from_translation(spawn_point),
        TransformInterpolation,
        ACTOR_COLLISION_LAYERS,
        make_player_collider(),
        CharacterController,
        FlyController,
        Visibility::default(),
        children![(
            MouseCam,
            Camera3d::default(),
            Transform::default()
                .looking_to(Vec3::X, Vec3::Y)
                .with_translation(Vec3::Y * (PLAYER_HEIGHT / 2.0 - EYELINE)),
            Projection::Perspective(PerspectiveProjection {
                fov: MouseSettings::default().fov / 180.0 * PI,
                ..default()
            }),
            IsDefaultUiCamera,
            NoCpuCulling,
            NoIndirectDrawing,
        )],
    ));
}

pub fn spawn_point(metadata: &WorldMetadata) -> Vec3 {
    let x = CHUNK_SIZE as f32 / 2.0;
    let z = CHUNK_SIZE as f32 / 2.0;
    let y = terrain_height(metadata, x as i32, z as i32) as f32 + PLAYER_HEIGHT + 2.0;

    Vec3::new(x, y, z)
}

pub fn make_player_collider() -> Collider {
    Collider::cuboid(PLAYER_LENGTH, PLAYER_HEIGHT, PLAYER_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_point_is_above_seeded_terrain() {
        let metadata = WorldMetadata::with_seed(123);
        let spawn = spawn_point(&metadata);
        let surface_y = terrain_height(&metadata, spawn.x as i32, spawn.z as i32) as f32;

        assert!(spawn.y > surface_y + PLAYER_HEIGHT);
    }
}
