//! In-game water test status overlay.
//!
//! The runner lives in [`crate::game_test`]; this plugin only renders its
//! status so a developer can watch the scenes settle and see which ones pass.
//! Toggled with `F7` by the runner.

use bevy::prelude::*;

use crate::{
    game_test::{TestStatus, WaterTestEntry, WaterTestSuite},
    tick_control::TickControl,
    world::chunk::WATER_SCENARIOS,
};

pub(super) struct GameTestOverlayPlugin;

impl Plugin for GameTestOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WaterTestSuite>()
            .add_systems(Startup, spawn_game_test_overlay)
            .add_systems(Update, update_game_test_overlay);
    }
}

#[derive(Component)]
struct GameTestOverlayRoot;

#[derive(Component)]
struct GameTestOverlayHeader;

#[derive(Component)]
struct GameTestOverlayRow(usize);

#[derive(Component)]
struct GameTestOverlayDetail;

/// Cap on detail lines rendered in the overlay so a noisy failure cannot
/// swallow the whole screen; the full text remains available on the entry.
const MAX_OVERLAY_DETAIL_LINES: usize = 10;

fn spawn_game_test_overlay(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(6.0),
                right: Val::Px(6.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                padding: UiRect::all(Val::Px(8.0)),
                display: Display::None,
                max_width: Val::Px(360.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            GameTestOverlayRoot,
        ))
        .with_children(|overlay| {
            overlay.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                GameTestOverlayHeader,
            ));
            for (index, scenario) in WATER_SCENARIOS.iter().enumerate() {
                overlay.spawn((
                    Text::new(format!(
                        "{:>7}  {}",
                        TestStatus::Pending.label(),
                        scenario.name
                    )),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(status_color(TestStatus::Pending)),
                    GameTestOverlayRow(index),
                ));
            }
            overlay.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.75, 0.75)),
                GameTestOverlayDetail,
            ));
        });
}

fn update_game_test_overlay(
    suite: Res<WaterTestSuite>,
    tick_control: Option<Res<TickControl>>,
    mut root_q: Query<&mut Node, With<GameTestOverlayRoot>>,
    mut header_q: Query<&mut Text, (With<GameTestOverlayHeader>, Without<GameTestOverlayRow>)>,
    mut rows_q: Query<
        (&GameTestOverlayRow, &mut Text, &mut TextColor),
        (Without<GameTestOverlayHeader>, Without<GameTestOverlayDetail>),
    >,
    mut detail_q: Query<
        &mut Text,
        (
            With<GameTestOverlayDetail>,
            Without<GameTestOverlayHeader>,
            Without<GameTestOverlayRow>,
        ),
    >,
) {
    for mut node in &mut root_q {
        node.display = if suite.overlay_visible {
            Display::Flex
        } else {
            Display::None
        };
    }

    if let Ok(mut header) = header_q.single_mut() {
        header.0 = format!(
            "water tests: {} pass / {} fail\nF8 run + fly to field   F9 next test   F11 dump plots   F7 hide",
            suite.passed(),
            suite.failed(),
        );
        if tick_control
            .as_ref()
            .is_some_and(|control| control.frozen())
        {
            header
                .0
                .push_str("\nsimulation frozen: P resume | O step | I step fluid");
        }
    }

    for (row, mut text, mut color) in &mut rows_q {
        let Some(entry) = suite.entries.get(row.0) else {
            continue;
        };
        let marker = if suite.viewing() == Some(row.0) {
            '>'
        } else {
            ' '
        };
        text.0 = format!(
            "{marker} {:>7}  {}{}",
            entry.status.label(),
            entry.scenario.name,
            phase_progress(entry)
        );
        color.0 = status_color(entry.status);
    }

    if let Ok(mut detail) = detail_q.single_mut() {
        detail.0 = first_failure_report(&suite);
    }
}

/// Shows the first failure's diff under the list; the dev can still press F9
/// to stand inside the scene for the full picture.
fn first_failure_report(suite: &WaterTestSuite) -> String {
    let Some(entry) = suite
        .entries
        .iter()
        .find(|entry| matches!(entry.status, TestStatus::Failed | TestStatus::TimedOut))
    else {
        return String::new();
    };

    let mut lines = Vec::with_capacity(MAX_OVERLAY_DETAIL_LINES + 1);
    lines.push(format!("{}:", entry.scenario.name));
    for line in entry.detail.lines().take(MAX_OVERLAY_DETAIL_LINES) {
        lines.push(format!("  {line}"));
    }
    let shown = lines.len() - 1;
    if entry.detail.lines().count() > shown {
        lines.push("  ...".to_owned());
    }
    lines.join("\n")
}

fn status_color(status: TestStatus) -> Color {
    match status {
        TestStatus::Pending => Color::srgb(0.72, 0.72, 0.72),
        TestStatus::Running => Color::srgb(1.0, 0.92, 0.4),
        TestStatus::Passed => Color::srgb(0.4, 0.92, 0.4),
        TestStatus::Failed => Color::srgb(1.0, 0.4, 0.4),
        TestStatus::TimedOut => Color::srgb(1.0, 0.62, 0.2),
    }
}

/// Phase progress for scenarios with follow-up edits, e.g. `[2/3]`; empty for
/// single-phase scenarios and for entries that have not started yet.
fn phase_progress(entry: &WaterTestEntry) -> String {
    let phases = entry.scenario.phases;
    if phases.is_empty() || entry.status == TestStatus::Pending {
        return String::new();
    }
    format!("  [{}/{}]", entry.phase + 1, phases.len() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_failure_report_is_empty_without_failures() {
        let suite = WaterTestSuite::new();
        assert!(first_failure_report(&suite).is_empty());
    }

    #[test]
    fn first_failure_report_names_the_scenario_and_caps_lines() {
        let mut suite = WaterTestSuite::new();
        suite.entries[0].status = TestStatus::Failed;
        suite.entries[0].detail = (0..50).map(|i| format!("row {i}")).collect::<Vec<_>>().join("\n");

        let report = first_failure_report(&suite);
        let lines = report.lines().count();
        assert!(report.starts_with(suite.entries[0].scenario.name));
        assert_eq!(lines, MAX_OVERLAY_DETAIL_LINES + 2, "name + cap + ellipsis");
        assert!(report.ends_with("  ..."));
    }

    #[test]
    fn phase_progress_marks_multi_phase_scenarios() {
        let mut suite = WaterTestSuite::new();
        let index = suite
            .entries
            .iter()
            .position(|entry| !entry.scenario.phases.is_empty())
            .expect("at least one scenario must have follow-up phases");
        let total = suite.entries[index].scenario.phases.len() + 1;
        let entry = &mut suite.entries[index];

        entry.status = TestStatus::Pending;
        assert_eq!(phase_progress(entry), "", "not started yet");
        entry.status = TestStatus::Running;
        assert_eq!(phase_progress(entry), format!("  [1/{total}]"));
        entry.phase = total - 1;
        entry.status = TestStatus::Passed;
        assert_eq!(phase_progress(entry), format!("  [{total}/{total}]"));
    }
}
