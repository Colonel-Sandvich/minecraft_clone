use super::{fly_controller::Flying, make_collider, GameMode, Player};
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

pub struct ControlPlayerPlugin;

impl Plugin for ControlPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyBindings>();

        app.add_systems(PreUpdate, change_gamemode);
        app.add_systems(PreUpdate, toggle_fly);
        app.add_systems(PreUpdate, debug_toggle_colliders);
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
    pub sprint: KeyCode,
    pub toggle_grab_cursor: KeyCode,
    pub toggle_fly: KeyCode,
    pub change_gamemode: KeyCode,
    pub debug_toggle_rapier_render: KeyCode,
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
            sprint: KeyCode::ShiftLeft,
            toggle_grab_cursor: KeyCode::Escape,
            toggle_fly: KeyCode::KeyF,
            change_gamemode: KeyCode::F4,
            debug_toggle_rapier_render: KeyCode::KeyB,
        }
    }
}

pub fn change_gamemode(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mut player_q: Query<(Entity, &mut Player, &mut KinematicCharacterController)>,
) {
    let (player_entity, mut player, mut controller) = player_q.single_mut();
    let player_entity = &mut commands.get_entity(player_entity).unwrap();

    if keys.just_pressed(key_bindings.change_gamemode) {
        match player.gamemode {
            GameMode::Survival => todo!(),
            GameMode::Creative => {
                controller.filter_flags = QueryFilterFlags::all();
                player.gamemode = GameMode::Spectator;
            }
            GameMode::Adventure => todo!(),
            GameMode::Spectator => {
                controller.filter_flags = KinematicCharacterController::default().filter_flags;
                player.gamemode = GameMode::Creative;
            }
        };
    }
}

pub fn toggle_fly(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    player_q: Query<(Entity, Option<&Flying>), With<Player>>,
) {
    let (player, flying) = player_q.single();
    let player = &mut commands.get_entity(player).unwrap();

    if keys.just_pressed(key_bindings.toggle_fly) {
        match flying {
            Some(_) => player.remove::<Flying>(),
            None => player.insert(Flying),
        };
    }
}

pub fn debug_toggle_colliders(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mut rapier_render_context: ResMut<DebugRenderContext>,
) {
    if keys.just_pressed(key_bindings.debug_toggle_rapier_render) {
        rapier_render_context.enabled = !rapier_render_context.enabled;
    }
}
