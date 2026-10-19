use bevy::{
    image::{ImageLoaderSettings, ImageSampler},
    input_focus::{AutoFocus, FocusCause, InputFocus, IsFocused, IsFocusedHelper},
    picking::hover::Hovered,
    prelude::*,
    ui::{Pressed, widget::NodeImageMode},
    ui_widgets::{Activate, ActivateOnPress, Button},
};
use bevy_settings::SaveSettingsDeferred;

use crate::game_state::GameState;

use super::{
    settings::{UiSettings, rem, rem_font},
    z,
};

pub struct PauseMenuPlugin;

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiSettings>()
            .add_systems(OnEnter(GameState::Paused), spawn_pause_menu)
            .add_observer(handle_pause_menu_action)
            .add_systems(
                Update,
                (update_button_visuals, update_ui_scale_text).run_if(in_state(GameState::Paused)),
            );
    }
}

#[derive(Component)]
struct PauseMenuRoot;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum PauseMenuAction {
    Resume,
    OpenOptions,
    CloseOptions,
    CycleUiScale,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum PauseMenuPage {
    Main,
    Options,
}

#[derive(Component)]
struct UiScaleValueText;

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

fn spawn_pause_menu(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Res<UiSettings>,
) {
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
            GlobalZIndex(z::PAUSE_MENU),
        ))
        .with_children(|overlay| {
            overlay
                .spawn(Node {
                    width: Val::Vw(76.0),
                    max_width: rem(600.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|menu| {
                    menu.spawn((
                        PauseMenuPage::Main,
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            row_gap: rem(12.0),
                            ..default()
                        },
                    ))
                    .with_children(|page| {
                        spawn_menu_title(page, "Game Menu", 38.0);

                        spawn_menu_button(
                            page,
                            "Back to Game",
                            Some(PauseMenuAction::Resume),
                            &textures,
                        );
                        spawn_menu_button(
                            page,
                            "Options...",
                            Some(PauseMenuAction::OpenOptions),
                            &textures,
                        );
                        spawn_menu_button(page, "Save and Quit  (coming soon)", None, &textures);

                        spawn_menu_hint(page, "Press Esc to return to the game");
                    });

                    menu.spawn((
                        PauseMenuPage::Options,
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            row_gap: rem(12.0),
                            display: Display::None,
                            ..default()
                        },
                    ))
                    .with_children(|page| {
                        spawn_menu_title(page, "Options", 38.0);

                        page.spawn((
                            Text::new(ui_scale_label(&settings)),
                            TextFont {
                                font_size: rem_font(24.0),
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            TextShadow {
                                offset: Vec2::splat(2.0),
                                color: Color::BLACK,
                            },
                            UiScaleValueText,
                            Pickable::IGNORE,
                        ));

                        spawn_menu_button(
                            page,
                            "Change UI Scale",
                            Some(PauseMenuAction::CycleUiScale),
                            &textures,
                        );
                        spawn_menu_button(
                            page,
                            "Back",
                            Some(PauseMenuAction::CloseOptions),
                            &textures,
                        );

                        spawn_menu_hint(page, "UI Scale cycles 75% / 100% / 125% / 150%");
                    });
                });
        });
}

fn spawn_menu_title(parent: &mut ChildSpawnerCommands, title: &'static str, font_px: f32) {
    parent.spawn((
        Text::new(title),
        TextFont {
            font_size: rem_font(font_px),
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow {
            offset: Vec2::splat(3.0),
            color: Color::BLACK,
        },
        Node {
            margin: UiRect {
                bottom: rem(28.0),
                ..default()
            },
            ..default()
        },
    ));
}

fn spawn_menu_hint(parent: &mut ChildSpawnerCommands, hint: &'static str) {
    parent.spawn((
        Text::new(hint),
        TextFont {
            font_size: rem_font(18.0),
            ..default()
        },
        TextColor(Color::srgb(0.8, 0.8, 0.8)),
        TextShadow {
            offset: Vec2::splat(2.0),
            color: Color::BLACK,
        },
        Node {
            margin: UiRect {
                top: rem(18.0),
                ..default()
            },
            ..default()
        },
    ));
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
            height: rem(60.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    ));

    if let Some(action) = action {
        button.insert((
            Button,
            Hovered::default(),
            ActivateOnPress,
            action,
            PauseButtonVisual {
                normal: textures.button.clone(),
                highlighted: textures.button_highlighted.clone(),
            },
        ));
        if matches!(action, PauseMenuAction::Resume) {
            button.insert(AutoFocus);
        }
    }

    button.with_child((
        Text::new(label),
        TextFont {
            font_size: rem_font(28.0),
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

fn handle_pause_menu_action(
    activate: On<Activate>,
    actions: Query<&PauseMenuAction>,
    game_state: Res<State<GameState>>,
    mut next_game_state: ResMut<NextState<GameState>>,
    mut settings: ResMut<UiSettings>,
    mut commands: Commands,
    mut pages: Query<(&PauseMenuPage, &mut Node)>,
    focus_targets: Query<(Entity, &PauseMenuAction)>,
    mut input_focus: Option<ResMut<InputFocus>>,
) {
    if *game_state.get() != GameState::Paused {
        return;
    }

    let Ok(action) = actions.get(activate.entity) else {
        return;
    };

    match *action {
        PauseMenuAction::Resume => next_game_state.set(GameState::Playing),
        PauseMenuAction::OpenOptions => {
            show_page(&mut pages, PauseMenuPage::Options);
            focus_action(
                PauseMenuAction::CycleUiScale,
                &focus_targets,
                &mut input_focus,
            );
        }
        PauseMenuAction::CloseOptions => {
            show_page(&mut pages, PauseMenuPage::Main);
            focus_action(PauseMenuAction::Resume, &focus_targets, &mut input_focus);
        }
        PauseMenuAction::CycleUiScale => {
            let next = settings.next_scale();
            *settings = next;
            commands.queue(SaveSettingsDeferred::default());
        }
    }
}

fn show_page(pages: &mut Query<(&PauseMenuPage, &mut Node)>, page: PauseMenuPage) {
    for (candidate, mut node) in pages.iter_mut() {
        let display = if *candidate == page {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}

fn focus_action(
    target: PauseMenuAction,
    focus_targets: &Query<(Entity, &PauseMenuAction)>,
    input_focus: &mut Option<ResMut<InputFocus>>,
) {
    let Some((entity, _)) = focus_targets.iter().find(|(_, action)| **action == target) else {
        return;
    };
    if let Some(focus) = input_focus.as_mut() {
        focus.set(entity, FocusCause::Navigated);
    }
}

fn ui_scale_label(settings: &UiSettings) -> String {
    format!("UI Scale: {}%", settings.scale_percent())
}

fn update_ui_scale_text(
    settings: Res<UiSettings>,
    mut labels: Query<&mut Text, With<UiScaleValueText>>,
) {
    let target = ui_scale_label(&settings);
    for mut text in &mut labels {
        if text.0 != target {
            text.0 = target.clone();
        }
    }
}

fn update_button_visuals(
    focus: IsFocusedHelper,
    mut buttons: Query<
        (
            Entity,
            &Hovered,
            Has<Pressed>,
            &PauseButtonVisual,
            &mut ImageNode,
        ),
        With<Button>,
    >,
) {
    for (entity, hovered, pressed, visual, mut image) in &mut buttons {
        let highlighted = hovered.get() || pressed || focus.is_focus_visible(entity);
        let target = if highlighted {
            &visual.highlighted
        } else {
            &visual.normal
        };
        if &image.image != target {
            image.image = target.clone();
        }
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
