use bevy::{
    image::{ImageLoaderSettings, ImageSampler},
    prelude::*,
    ui::{FocusPolicy, widget::NodeImageMode},
};

use crate::game_state::GameState;

pub struct PauseMenuPlugin;

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Paused), spawn_pause_menu)
            .add_systems(
                Update,
                (handle_pause_menu_action, update_button_visuals)
                    .run_if(in_state(GameState::Paused)),
            );
    }
}

#[derive(Component)]
struct PauseMenuRoot;

#[derive(Component, Clone, Copy)]
enum PauseMenuAction {
    Resume,
}

#[derive(Component)]
struct PauseButtonVisual {
    normal: Handle<Image>,
    highlighted: Handle<Image>,
}

#[derive(Clone)]
struct PauseMenuTextures {
    button: Handle<Image>,
    button_highlighted: Handle<Image>,
    button_disabled: Handle<Image>,
}

fn load_ui_texture(asset_server: &AssetServer, path: &'static str) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.sampler = ImageSampler::nearest();
        })
        .load(path)
}

fn spawn_pause_menu(mut commands: Commands, asset_server: Res<AssetServer>) {
    let textures = PauseMenuTextures {
        button: load_ui_texture(&asset_server, "textures/gui/sprites/widget/button.png"),
        button_highlighted: load_ui_texture(
            &asset_server,
            "textures/gui/sprites/widget/button_highlighted.png",
        ),
        button_disabled: load_ui_texture(
            &asset_server,
            "textures/gui/sprites/widget/button_disabled.png",
        ),
    };

    commands
        .spawn((
            Name::new("Pause Menu"),
            PauseMenuRoot,
            DespawnOnExit(GameState::Paused),
            Node {
                position_type: PositionType::Absolute,
                left: Val::ZERO,
                top: Val::ZERO,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62)),
            FocusPolicy::Block,
            GlobalZIndex(1_000),
        ))
        .with_children(|overlay| {
            overlay
                .spawn(Node {
                    width: Val::Vw(76.0),
                    max_width: Val::Px(600.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(12.0),
                    ..default()
                })
                .with_children(|menu| {
                    menu.spawn((
                        Text::new("Game Menu"),
                        TextFont {
                            font_size: FontSize::Px(38.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        TextShadow {
                            offset: Vec2::splat(3.0),
                            color: Color::BLACK,
                        },
                        Node {
                            margin: UiRect {
                                bottom: Val::Px(28.0),
                                ..default()
                            },
                            ..default()
                        },
                    ));

                    spawn_menu_button(
                        menu,
                        "Back to Game",
                        Some(PauseMenuAction::Resume),
                        &textures,
                    );
                    spawn_menu_button(menu, "Options...  (coming soon)", None, &textures);
                    spawn_menu_button(menu, "Save and Quit  (coming soon)", None, &textures);

                    menu.spawn((
                        Text::new("Press Esc to return to the game"),
                        TextFont {
                            font_size: FontSize::Px(18.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextShadow {
                            offset: Vec2::splat(2.0),
                            color: Color::BLACK,
                        },
                        Node {
                            margin: UiRect {
                                top: Val::Px(18.0),
                                ..default()
                            },
                            ..default()
                        },
                    ));
                });
        });
}

fn spawn_menu_button(
    parent: &mut ChildSpawnerCommands,
    label: &'static str,
    action: Option<PauseMenuAction>,
    textures: &PauseMenuTextures,
) {
    let enabled = action.is_some();
    let image = if enabled {
        textures.button.clone()
    } else {
        textures.button_disabled.clone()
    };
    let mut button = parent.spawn((
        Name::new(label),
        ImageNode::new(image).with_mode(NodeImageMode::Stretch),
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(60.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    ));

    if let Some(action) = action {
        button.insert((
            Button,
            action,
            PauseButtonVisual {
                normal: textures.button.clone(),
                highlighted: textures.button_highlighted.clone(),
            },
        ));
    }

    button.with_child((
        Text::new(label),
        TextFont {
            font_size: FontSize::Px(28.0),
            ..default()
        },
        TextColor(if enabled {
            Color::WHITE
        } else {
            Color::srgb(0.62, 0.62, 0.62)
        }),
        TextShadow {
            offset: Vec2::splat(2.0),
            color: Color::BLACK,
        },
        Pickable::IGNORE,
    ));
}

type PauseMenuActionButtons<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static PauseMenuAction),
    (Changed<Interaction>, With<Button>),
>;

fn handle_pause_menu_action(
    buttons: PauseMenuActionButtons,
    mut next_game_state: ResMut<NextState<GameState>>,
) {
    for (interaction, action) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }

        match action {
            PauseMenuAction::Resume => next_game_state.set(GameState::Playing),
        }
    }
}

type PauseMenuVisualButtons<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        &'static PauseButtonVisual,
        &'static mut ImageNode,
    ),
    (Changed<Interaction>, With<Button>),
>;

fn update_button_visuals(mut buttons: PauseMenuVisualButtons) {
    for (interaction, visual, mut image) in &mut buttons {
        image.image = match interaction {
            Interaction::None => visual.normal.clone(),
            Interaction::Hovered | Interaction::Pressed => visual.highlighted.clone(),
        };
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn pause_menu_texture_assets_are_present() {
        for path in [
            "assets/textures/gui/sprites/widget/button.png",
            "assets/textures/gui/sprites/widget/button_highlighted.png",
            "assets/textures/gui/sprites/widget/button_disabled.png",
        ] {
            assert!(
                Path::new(path).is_file(),
                "missing pause-menu asset: {path}"
            );
        }
    }
}
