use std::f32::consts::PI;

use super::{
    PLAYER_HEIGHT, PLAYER_LENGTH, PLAYER_WIDTH, Player, PlayerDimension,
    cam::{MouseCam, MouseSettings},
};
use avian3d::prelude::{Collider, Position, RigidBody, TransformInterpolation};
use bevy::prelude::*;

use crate::{
    game_state::GameState,
    mob::controller::{CharacterController, FlyController},
    world::{
        ACTOR_COLLISION_LAYERS,
        dimension::{Active, Dimension},
    },
};

pub struct SpawnPlayerPlugin;

impl Plugin for SpawnPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnExit(GameState::GenWorld), spawn_player);
    }
}

pub const EYELINE: f32 = 0.1;

fn spawn_player(mut commands: Commands, dimension: Single<&Dimension, With<Active>>) {
    let spawn_point = dimension.arrival();

    commands.spawn((
        Player::default(),
        PlayerDimension::new(dimension.id()),
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
        )],
    ));
}

pub fn make_player_collider() -> Collider {
    Collider::cuboid(PLAYER_LENGTH, PLAYER_HEIGHT, PLAYER_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{DimensionCatalog, DimensionId, WorldMetadata};

    #[test]
    fn player_uses_active_arrival_without_becoming_a_dimension_child() {
        let definition = *DimensionCatalog::for_world(&WorldMetadata::with_seed(123))
            .get(DimensionId::GRASS_FLOOR)
            .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, spawn_player);
        let owner = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(owner)
            .insert((Dimension::new(owner, definition), Active));

        app.update();

        let mut query = app
            .world_mut()
            .query_filtered::<(&PlayerDimension, &Position, Option<&ChildOf>), With<Player>>();
        let (membership, position, parent) = query.single(app.world()).unwrap();
        assert_eq!(membership.id(), definition.id());
        assert_eq!(position.0, definition.arrival());
        assert!(parent.is_none());
    }
}
