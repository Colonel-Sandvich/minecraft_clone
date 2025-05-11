use std::f32::consts::PI;

use super::{
    PLAYER_HEIGHT, PLAYER_LENGTH, PLAYER_WIDTH, Player,
    cam::{MouseCam, MouseSettings},
};
use avian3d::prelude::Collider;
use bevy::{
    prelude::*,
    render::view::{NoCpuCulling, NoIndirectDrawing},
};

use crate::{
    chunk::CHUNK_SIZE,
    mob::controller::{CharacterController, FlyController},
};

pub struct SpawnPlayerPlugin;

impl Plugin for SpawnPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
    }
}

pub const SPAWN_POINT: Vec3 = Vec3::new(0.5, CHUNK_SIZE as f32 + 2.0, 0.5);

pub const EYELINE: f32 = 0.1;

fn setup(mut commands: Commands) {
    commands.spawn((
        Player::default(),
        Transform::from_translation(SPAWN_POINT),
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
