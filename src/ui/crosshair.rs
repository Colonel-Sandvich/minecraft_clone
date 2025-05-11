use bevy::prelude::*;

pub struct CrosshairPlugin;

impl Plugin for CrosshairPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_crosshair);
    }
}

const CROSSHAIR_SIZE: f32 = 40.0;

#[derive(Component)]
#[require(Name::new("Crosshair"))]
struct Crosshair;

fn spawn_crosshair(mut commands: Commands) {
    commands.spawn((
        Node {
            display: Display::Flex,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            height: Val::Vh(100.0),
            width: Val::Vw(100.0),
            ..default()
        },
        children![(
            Crosshair,
            Text::new("+"),
            TextFont {
                font_size: CROSSHAIR_SIZE,
                ..default()
            },
            TextColor(Color::WHITE),
        )],
    ));
}
