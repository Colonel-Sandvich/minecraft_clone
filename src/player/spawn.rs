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
    world::{chunk::CHUNK_SIZE, dimension::Dimension},
};

pub struct SpawnPlayerPlugin;

impl Plugin for SpawnPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnExit(GameState::GenWorld), spawn_player);
    }
}

pub const SPAWN_POINT: Vec3 = Vec3::new(
    CHUNK_SIZE as f32 / 2.0,
    CHUNK_SIZE as f32 + 2.0,
    CHUNK_SIZE as f32 / 2.0,
);

pub const EYELINE: f32 = 0.1;

fn spawn_player(mut commands: Commands, dimension_q: Query<Entity, With<Dimension>>) {
    commands.spawn((
        ChildOf(dimension_q.single().unwrap()),
        Player::default(),
        RigidBody::Kinematic,
        Position::new(SPAWN_POINT),
        Transform::from_translation(SPAWN_POINT),
        TransformInterpolation,
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

pub fn make_player_collider() -> Collider {
    Collider::cuboid(PLAYER_LENGTH, PLAYER_HEIGHT, PLAYER_WIDTH)
}
