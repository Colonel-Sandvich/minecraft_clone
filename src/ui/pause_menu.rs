use bevy::{
    image::{ImageLoaderSettings, ImageSampler},
    input_focus::{
        AutoFocus, FocusCause, InputFocus, IsFocused, IsFocusedHelper,
        tab_navigation::{TabGroup, TabIndex, TabNavigationPlugin},
    },
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
        if !app.is_plugin_added::<TabNavigationPlugin>() {
            app.add_plugins(TabNavigationPlugin);
        }

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

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum PauseMenuPage {
    Main,
    Options,
}

#[derive(Component, Clone, Copy)]
struct PauseMenuButtonPage(PauseMenuPage);

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
            TabGroup::modal(),
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
                            PauseMenuPage::Main,
                            &textures,
                        );
                        spawn_menu_button(
                            page,
                            "Options...",
                            Some(PauseMenuAction::OpenOptions),
                            PauseMenuPage::Main,
                            &textures,
                        );
                        spawn_menu_button(
                            page,
                            "Save and Quit  (coming soon)",
                            None,
                            PauseMenuPage::Main,
                            &textures,
                        );

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
                            PauseMenuPage::Options,
                            &textures,
                        );
                        spawn_menu_button(
                            page,
                            "Back",
                            Some(PauseMenuAction::CloseOptions),
                            PauseMenuPage::Options,
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
    page: PauseMenuPage,
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
        let tab_index = if page == PauseMenuPage::Main { 0 } else { -1 };
        button.insert((
            Button,
            Hovered::default(),
            ActivateOnPress,
            PauseMenuButtonPage(page),
            TabIndex(tab_index),
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
    mut tab_indices: Query<(&PauseMenuButtonPage, &mut TabIndex)>,
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
            show_page(&mut pages, &mut tab_indices, PauseMenuPage::Options);
            focus_action(
                PauseMenuAction::CycleUiScale,
                &focus_targets,
                &mut input_focus,
            );
        }
        PauseMenuAction::CloseOptions => {
            show_page(&mut pages, &mut tab_indices, PauseMenuPage::Main);
            focus_action(PauseMenuAction::Resume, &focus_targets, &mut input_focus);
        }
        PauseMenuAction::CycleUiScale => {
            let next = settings.next_scale();
            *settings = next;
            commands.queue(SaveSettingsDeferred::default());
        }
    }
}

fn show_page(
    pages: &mut Query<(&PauseMenuPage, &mut Node)>,
    tab_indices: &mut Query<(&PauseMenuButtonPage, &mut TabIndex)>,
    page: PauseMenuPage,
) {
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

    // Bevy's visibility propagation does not always mark `Display::None` subtrees hidden
    // immediately, so also take hidden-page buttons out of sequential tab navigation.
    for (button_page, mut tab_index) in tab_indices.iter_mut() {
        let target = if button_page.0 == page { 0 } else { -1 };
        if tab_index.0 != target {
            tab_index.0 = target;
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

    use bevy::{
        asset::AssetPlugin,
        camera::visibility::VisibilityPlugin,
        image::Image,
        input::{
            ButtonState, InputPlugin,
            keyboard::{Key, KeyboardInput},
        },
        input_focus::{
            AutoFocus, InputDispatchPlugin, InputFocus, InputFocusPlugin, InputFocusVisible,
            tab_navigation::{TabIndex, TabNavigationPlugin},
        },
        mesh::{Mesh, skinning::SkinnedMeshInverseBindposes},
        state::app::StatesPlugin,
        ui_widgets::ButtonPlugin,
        window::PrimaryWindow,
    };

    use super::*;

    fn pause_menu_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            StatesPlugin,
            AssetPlugin::default(),
            VisibilityPlugin,
            InputPlugin,
            InputFocusPlugin,
            InputDispatchPlugin,
            ButtonPlugin,
            PauseMenuPlugin,
        ))
        .insert_state(GameState::Playing);
        app.init_asset::<Image>();
        app.init_asset::<Mesh>();
        app.init_asset::<SkinnedMeshInverseBindposes>();
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.update();

        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::Paused);
        app.update();
        app
    }

    fn find_named(app: &mut App, name: &str) -> Entity {
        let mut query = app.world_mut().query::<(Entity, &Name)>();
        query
            .iter(app.world())
            .find_map(|(entity, candidate)| (candidate.as_str() == name).then_some(entity))
            .unwrap_or_else(|| panic!("missing entity named {name}"))
    }

    fn page_entity(app: &mut App, page: PauseMenuPage) -> Entity {
        let mut query = app.world_mut().query::<(Entity, &PauseMenuPage)>();
        query
            .iter(app.world())
            .find_map(|(entity, candidate)| (*candidate == page).then_some(entity))
            .unwrap_or_else(|| panic!("missing page {page:?}"))
    }

    fn page_display(app: &mut App, page: PauseMenuPage) -> Display {
        let entity = page_entity(app, page);
        app.world().get::<Node>(entity).expect("page node").display
    }

    fn tab_index(app: &App, entity: Entity) -> Option<i32> {
        app.world().get::<TabIndex>(entity).map(|index| index.0)
    }

    fn press_key(app: &mut App, key_code: KeyCode, logical_key: Key) {
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .expect("primary window");
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
        app.update();
    }

    fn press_tab(app: &mut App, shift: bool) {
        if shift {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::ShiftLeft);
        }
        press_key(app, KeyCode::Tab, Key::Tab);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
    }

    #[test]
    fn pause_menu_main_page_tab_order_skips_disabled_and_hidden_buttons() {
        let mut app = pause_menu_app();
        let root = find_named(&mut app, "Pause Menu");
        let resume = find_named(&mut app, "Back to Game");
        let options = find_named(&mut app, "Options...");
        let save_and_quit = find_named(&mut app, "Save and Quit  (coming soon)");
        let change_scale = find_named(&mut app, "Change UI Scale");
        let back = find_named(&mut app, "Back");

        let tab_group = app.world().get::<TabGroup>(root).expect("modal tab group");
        assert!(tab_group.modal, "the pause menu must trap tab navigation");
        assert_eq!(page_display(&mut app, PauseMenuPage::Main), Display::Flex);
        assert_eq!(
            page_display(&mut app, PauseMenuPage::Options),
            Display::None
        );
        assert_eq!(tab_index(&app, resume), Some(0));
        assert_eq!(tab_index(&app, options), Some(0));
        assert_eq!(tab_index(&app, change_scale), Some(-1));
        assert_eq!(tab_index(&app, back), Some(-1));
        assert!(app.world().get::<AutoFocus>(resume).is_some());
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(resume),
            "Resume keeps autofocus when the pause menu opens"
        );

        assert!(app.world().get::<TabIndex>(save_and_quit).is_none());
        assert!(app.world().get::<Button>(save_and_quit).is_none());

        // Enabled main-page buttons tab in visual order: Resume -> Options -> Resume.
        press_tab(&mut app, false);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(options));
        assert!(app.world().resource::<InputFocusVisible>().0);
        let visual = app.world().get::<PauseButtonVisual>(options).unwrap();
        let image = app.world().get::<ImageNode>(options).unwrap();
        assert_eq!(
            image.image, visual.highlighted,
            "keyboard focus must use the existing highlighted-button visual"
        );

        press_tab(&mut app, false);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(resume));

        // Reverse wraps from the first button to the last enabled one, still skipping disabled
        // and hidden-page buttons.
        press_tab(&mut app, true);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(options));
    }

    #[test]
    fn options_page_switches_tab_indices_and_focus() {
        let mut app = pause_menu_app();
        let options_button = find_named(&mut app, "Options...");
        let resume = find_named(&mut app, "Back to Game");
        let change_scale = find_named(&mut app, "Change UI Scale");
        let back = find_named(&mut app, "Back");

        press_tab(&mut app, false);
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(options_button)
        );
        press_key(&mut app, KeyCode::Enter, Key::Enter);

        assert_eq!(page_display(&mut app, PauseMenuPage::Main), Display::None);
        assert_eq!(
            page_display(&mut app, PauseMenuPage::Options),
            Display::Flex
        );
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(change_scale),
            "opening Options keeps the existing focus transfer"
        );
        assert_eq!(tab_index(&app, resume), Some(-1));
        assert_eq!(tab_index(&app, options_button), Some(-1));
        assert_eq!(tab_index(&app, change_scale), Some(0));
        assert_eq!(tab_index(&app, back), Some(0));

        // Hidden main-page buttons are not part of the wrap on the Options page.
        press_tab(&mut app, false);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(back));
        press_tab(&mut app, false);
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(change_scale)
        );
        press_tab(&mut app, true);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(back));

        press_key(&mut app, KeyCode::Enter, Key::Enter);
        assert_eq!(page_display(&mut app, PauseMenuPage::Main), Display::Flex);
        assert_eq!(
            page_display(&mut app, PauseMenuPage::Options),
            Display::None
        );
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(resume),
            "closing Options keeps the existing focus transfer"
        );
        assert_eq!(tab_index(&app, resume), Some(0));
        assert_eq!(tab_index(&app, options_button), Some(0));
        assert_eq!(tab_index(&app, change_scale), Some(-1));
        assert_eq!(tab_index(&app, back), Some(-1));
    }

    #[test]
    fn keyboard_activation_cycles_ui_scale_on_the_options_page() {
        let mut app = pause_menu_app();
        let options_button = find_named(&mut app, "Options...");
        let change_scale = find_named(&mut app, "Change UI Scale");

        press_tab(&mut app, false);
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(options_button)
        );
        press_key(&mut app, KeyCode::Enter, Key::Enter);
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(change_scale)
        );

        assert_eq!(app.world().resource::<UiSettings>().scale, 1.0);
        press_key(&mut app, KeyCode::Space, Key::Space);
        assert_eq!(
            app.world().resource::<UiSettings>().scale,
            1.25,
            "Space must use the existing Activate handler to cycle UI scale"
        );
    }

    #[test]
    fn enter_activates_the_focused_resume_button() {
        let mut app = pause_menu_app();
        let resume = find_named(&mut app, "Back to Game");
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(resume));

        press_key(&mut app, KeyCode::Enter, Key::Enter);

        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Playing,
            "Enter must use the existing Activate handler to resume"
        );
    }

    #[test]
    fn space_activates_the_focused_resume_button() {
        let mut app = pause_menu_app();
        let resume = find_named(&mut app, "Back to Game");
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(resume));

        press_key(&mut app, KeyCode::Space, Key::Space);

        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Playing,
            "Space must use the existing Activate handler to resume"
        );
    }

    #[test]
    fn pause_menu_registers_tab_navigation() {
        let mut app = App::new();
        app.add_plugins(PauseMenuPlugin);
        assert!(
            app.is_plugin_added::<TabNavigationPlugin>(),
            "the pause menu must install tab navigation exactly when it is missing"
        );
    }

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
