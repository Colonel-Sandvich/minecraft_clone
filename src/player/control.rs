use crate::{
    mob::controller::{FlyController, Flying, Velocity},
    world::{dimension::ViewDistance, generation::WorldMetadata},
};

use super::{
    GameMode, Player,
    cam::{MouseCam, MouseState},
    spawn::{make_player_collider, spawn_point},
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
        app.add_systems(PreUpdate, adjust_view_distance);
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
    pub view_distance_decrease: KeyCode,
    pub view_distance_increase: KeyCode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayerMovementIntent {
    /// Local input axis: +X right, +Y ascend, +Z forward.
    pub local_move_axis: Vec3,
    pub jump: bool,
    pub sprint: bool,
}

impl PlayerMovementIntent {
    pub fn wants_forward_sprint(self) -> bool {
        self.sprint && self.local_move_axis.z > 0.0
    }
}

impl KeyBindings {
    pub fn movement_intent(&self, keys: &ButtonInput<KeyCode>) -> PlayerMovementIntent {
        let mut local_move_axis = Vec3::ZERO;

        if keys.pressed(self.move_forward) {
            local_move_axis.z += 1.0;
        }
        if keys.pressed(self.move_backward) {
            local_move_axis.z -= 1.0;
        }
        if keys.pressed(self.move_right) {
            local_move_axis.x += 1.0;
        }
        if keys.pressed(self.move_left) {
            local_move_axis.x -= 1.0;
        }
        if keys.pressed(self.move_ascend) {
            local_move_axis.y += 1.0;
        }
        if keys.pressed(self.move_descend) {
            local_move_axis.y -= 1.0;
        }

        PlayerMovementIntent {
            local_move_axis,
            jump: keys.pressed(self.jump),
            sprint: keys.pressed(self.sprint),
        }
    }
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
            view_distance_decrease: KeyCode::BracketLeft,
            view_distance_increase: KeyCode::BracketRight,
        }
    }
}

fn change_gamemode(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    player: Single<(Entity, &mut Player, &mut Velocity)>,
) {
    let (player_entity, mut player, mut velocity) = player.into_inner();
    let player_entity = &mut commands.get_entity(player_entity).unwrap();

    if keys.just_pressed(key_bindings.change_gamemode) {
        match player.gamemode {
            GameMode::Survival => todo!(),
            GameMode::Creative => {
                player_entity.remove::<Collider>();
                player_entity.insert(Flying);
                **velocity = Vec3::ZERO;
                player.gamemode = GameMode::Spectator;
            }
            GameMode::Adventure => todo!(),
            GameMode::Spectator => {
                player_entity.insert(make_player_collider());
                player_entity.remove::<Flying>();
                **velocity = Vec3::ZERO;
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
    metadata: Res<WorldMetadata>,
    player_q: Single<(&mut Transform, &mut Position, &mut Velocity), With<Player>>,
    mut camera: Single<&mut Transform, (With<MouseCam>, Without<Player>)>,
) {
    if keys.just_pressed(key_bindings.debug_reset_character) {
        let (mut transform, mut position, mut velocity) = player_q.into_inner();
        let spawn_point = spawn_point(&metadata);
        **velocity = Vec3::ZERO;
        position.0 = spawn_point;
        *transform = Transform::from_translation(spawn_point);
        camera.rotation = Transform::default().looking_to(Vec3::X, Vec3::Y).rotation;
    }
}

fn adjust_view_distance(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mut view_distance: ResMut<ViewDistance>,
) {
    let old_distance = view_distance.chunks();

    if keys.just_pressed(key_bindings.view_distance_decrease) {
        view_distance.decrease();
    }
    if keys.just_pressed(key_bindings.view_distance_increase) {
        view_distance.increase();
    }

    let new_distance = view_distance.chunks();
    if new_distance != old_distance {
        info!(view_distance = new_distance, "View distance changed");
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
