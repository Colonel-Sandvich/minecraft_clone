use crate::mob::controller::{FlyController, Flying, Velocity};

use super::{
    GameMode, Player,
    cam::{MouseCam, MouseState},
    spawn::{SPAWN_POINT, make_player_collider},
};
use avian3d::prelude::*;
use bevy::prelude::*;

pub struct ControlPlayerPlugin;

impl Plugin for ControlPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyBindings>();

        app.add_systems(PreUpdate, change_gamemode);
        app.add_systems(PreUpdate, toggle_fly);
        app.add_systems(PreUpdate, debug_reset_character);
        app.add_systems(PreUpdate, toggle_grab_cursor);
    }
}

#[derive(Resource)]
pub struct KeyBindings {
    pub move_forward: KeyCode,
    pub move_backward: KeyCode,
    pub move_left: KeyCode,
    pub move_right: KeyCode,
    pub move_ascend: KeyCode,
    pub move_descend: KeyCode,
    pub jump: KeyCode,
    pub sprint: KeyCode,
    pub toggle_grab_cursor: KeyCode,
    pub toggle_fly: KeyCode,
    pub change_gamemode: KeyCode,
    pub debug_reset_character: KeyCode,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            move_forward: KeyCode::KeyW,
            move_backward: KeyCode::KeyS,
            move_left: KeyCode::KeyA,
            move_right: KeyCode::KeyD,
            move_ascend: KeyCode::Space,
            move_descend: KeyCode::ControlLeft,
            jump: KeyCode::Space,
            sprint: KeyCode::ShiftLeft,
            toggle_grab_cursor: KeyCode::Escape,
            toggle_fly: KeyCode::KeyF,
            change_gamemode: KeyCode::F4,
            debug_reset_character: KeyCode::KeyR,
        }
    }
}

fn change_gamemode(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    player: Single<(Entity, &mut Player)>,
) {
    let (player_entity, mut player) = player.into_inner();
    let player_entity = &mut commands.get_entity(player_entity).unwrap();

    if keys.just_pressed(key_bindings.change_gamemode) {
        match player.gamemode {
            GameMode::Survival => todo!(),
            GameMode::Creative => {
                // controller.filter_flags = QueryFilterFlags::all();
                player_entity.remove::<Collider>();
                player.gamemode = GameMode::Spectator;
            }
            GameMode::Adventure => todo!(),
            GameMode::Spectator => {
                // controller.filter_flags = KinematicCharacterController::default().filter_flags;
                player_entity.insert(make_player_collider());
                player.gamemode = GameMode::Creative;
            }
        };
    }
}

fn toggle_fly(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    flyer: Single<(Entity, Has<Flying>), With<FlyController>>,
) {
    let (entity, flying) = *flyer;
    let entity = &mut commands.get_entity(entity).unwrap();

    if keys.just_pressed(key_bindings.toggle_fly) {
        match flying {
            true => entity.remove::<Flying>(),
            false => entity.insert(Flying),
        };
    }
}

fn debug_reset_character(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    player_q: Single<(&mut Transform, &mut Velocity, &Children), With<Player>>,
    mut camera_q: Query<&mut Transform, (With<MouseCam>, Without<Player>)>,
) {
    if keys.just_pressed(key_bindings.debug_reset_character) {
        let (mut transform, mut velocity, children) = player_q.into_inner();
        **velocity = Vec3::ZERO;
        *transform = Transform::from_translation(SPAWN_POINT);
        let mut cameras = camera_q.iter_many_mut(children);
        while let Some(mut camera) = cameras.fetch_next() {
            camera.rotation = Transform::default().looking_to(Vec3::X, Vec3::Y).rotation;
        }
    }
}

fn toggle_grab_cursor(
    pressed_keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mouse_state: ResMut<State<MouseState>>,
    mut next_mouse_state: ResMut<NextState<MouseState>>,
) {
    if pressed_keys.just_pressed(key_bindings.toggle_grab_cursor) {
        match mouse_state.get() {
            MouseState::Free => {
                next_mouse_state.set(MouseState::Grabbed);
            }
            MouseState::Grabbed => {
                next_mouse_state.set(MouseState::Free);
            }
        };
    }
}
