//! On-screen hint for the simulation freeze.
//!
//! The controls live in [`crate::tick_control`]; this overlay only explains
//! why the world stopped moving. It is hidden while the simulation runs, so it
//! never adds noise during normal play.

use bevy::prelude::*;

use crate::{
    tick_control::{TickControl, ticks_until_fluid_update},
    world::dimension::fluid::FluidTickCounter,
};

pub(super) struct TickControlOverlayPlugin;

impl Plugin for TickControlOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_tick_control_overlay)
            .add_systems(Update, update_tick_control_overlay);
    }
}

#[derive(Component)]
struct TickControlOverlayRoot;

#[derive(Component)]
struct TickControlOverlayText;

fn spawn_tick_control_overlay(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(6.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            TickControlOverlayRoot,
            Pickable::IGNORE,
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        padding: UiRect::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
                    Pickable::IGNORE,
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new(""),
                        TextFont {
                            font_size: FontSize::Px(13.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.95, 0.95, 1.0)),
                        TickControlOverlayText,
                        Pickable::IGNORE,
                    ));
                });
        });
}

fn update_tick_control_overlay(
    control: Option<Res<TickControl>>,
    counter: Option<Res<FluidTickCounter>>,
    mut root_q: Query<&mut Node, With<TickControlOverlayRoot>>,
    mut text_q: Query<&mut Text, With<TickControlOverlayText>>,
) {
    let frozen = control.as_ref().is_some_and(|control| control.frozen());
    for mut node in &mut root_q {
        node.display = if frozen { Display::Flex } else { Display::None };
    }
    if !frozen {
        return;
    }

    let Some(control) = control else {
        return;
    };
    let counter = counter.map_or(0, |counter| counter.0);
    if let Ok(mut text) = text_q.single_mut() {
        text.0 = frozen_status_line(&control, counter);
    }
}

/// The hint shown while frozen: the keys on the first line, what the next
/// fluid update is waiting for on the second.
fn frozen_status_line(control: &TickControl, fluid_counter: u32) -> String {
    let player = if control.player_included() {
        "player: frozen"
    } else {
        "player: free"
    };
    format!(
        "SIMULATION FROZEN   P resume | O step one tick | I step to fluid | U {player}\n\
         ticks to next fluid update: {}",
        ticks_until_fluid_update(fluid_counter),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_status_line_names_the_keys_and_the_fluid_phase() {
        let control = TickControl::default();
        let line = frozen_status_line(&control, 3);

        assert!(line.contains("P resume"));
        assert!(line.contains("player: free"));
        assert!(line.contains("ticks to next fluid update: 2"));
    }
}
