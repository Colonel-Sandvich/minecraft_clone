use bevy::{
    prelude::*,
    window::{PrimaryWindow, WindowResized},
};

pub struct CrosshairPlugin;

impl Plugin for CrosshairPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_crosshair)
            .add_systems(Update, window_resized_event);
    }
}

const CROSSHAIR_SIZE: f32 = 40.0;

#[derive(Component)]
struct Crosshair;

fn spawn_crosshair(mut commands: Commands, primary_window: Query<&Window, With<PrimaryWindow>>) {
    let window = match primary_window.get_single() {
        Ok(window) => window,
        Err(_) => {
            warn!("Could not get Primary window");
            return;
        }
    };

    let (top, left) =
        calc_top_and_left_from_dimensions(window.resolution.width(), window.resolution.height());

    commands.spawn((
        TextBundle::from_section(
            "+".to_string(),
            TextStyle {
                font_size: CROSSHAIR_SIZE,
                color: Color::WHITE,
                ..default()
            },
        )
        .with_style(Style {
            position_type: PositionType::Absolute,
            align_items: AlignItems::Center,
            top: Val::Px(top),
            left: Val::Px(left),
            ..default()
        }),
        Crosshair,
    ));
}

fn window_resized_event(
    mut events: EventReader<WindowResized>,
    mut query: Query<&mut Style, With<Crosshair>>,
) {
    if let Some(last_event) = events.read().last() {
        let mut style = query.single_mut();
        let (top, left) = calc_top_and_left_from_dimensions(last_event.width, last_event.height);
        style.top = Val::Px(top);
        style.left = Val::Px(left);
    };
}

fn calc_top_and_left_from_dimensions(width: f32, height: f32) -> (f32, f32) {
    (
        height / 2.0 - CROSSHAIR_SIZE / 2.0,
        width / 2.0 - CROSSHAIR_SIZE / 4.0,
    )
}
