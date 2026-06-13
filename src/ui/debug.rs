use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
};

use crate::{
    player::Player,
    player::cam::MouseCam,
    world::chunk::ambient_occlusion::AmbientOcclusionSettings,
    world::chunk::{CHUNK_ISIZE, ChunkPosition},
    world::dimension::ViewDistance,
};

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugVisible>()
            .init_resource::<ChunkBordersVisible>()
            .add_systems(Startup, spawn_debug_overlay)
            .add_systems(
                Update,
                (
                    toggle_debug,
                    update_debug_text,
                    toggle_chunk_borders,
                    draw_chunk_borders,
                ),
            );
    }
}

#[derive(Component)]
struct DebugOverlay;

#[derive(Resource, Default)]
struct DebugVisible(bool);

#[derive(Resource, Default)]
struct ChunkBordersVisible(bool);

fn spawn_debug_overlay(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(3.0),
            left: Val::Px(3.0),
            ..default()
        },
        DebugOverlay,
    ));
}

// TODO: need to swap some keys from just_pressed to just_released for these multi key keybindings
// e.g. F3, F3 + G
fn toggle_debug(keys: Res<ButtonInput<KeyCode>>, mut visible: ResMut<DebugVisible>) {
    if keys.just_pressed(KeyCode::F3) {
        visible.0 = !visible.0;
    }
}

fn update_debug_text(
    visible: Res<DebugVisible>,
    diagnostics: Res<DiagnosticsStore>,
    player_q: Single<&Transform, With<Player>>,
    cam_q: Single<&Transform, (With<MouseCam>, Without<Player>)>,
    view_distance: Res<ViewDistance>,
    ao: Res<AmbientOcclusionSettings>,
    chunk_count: Query<(), With<crate::world::chunk::Chunk>>,
    mut text: Single<&mut Text, With<DebugOverlay>>,
) {
    if !visible.0 {
        text.0 = String::new();
        return;
    }

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    let pos = player_q.translation;
    let bx = pos.x.floor() as i32;
    let by = pos.y.floor() as i32;
    let bz = pos.z.floor() as i32;
    let cx = (pos.x / CHUNK_ISIZE as f32).floor() as i32;
    let cy = (pos.y / CHUNK_ISIZE as f32).floor() as i32;
    let cz = (pos.z / CHUNK_ISIZE as f32).floor() as i32;

    let facing = facing_dir(&cam_q);
    let loaded = chunk_count.iter().count();

    text.0 = format!(
        "Minecraft Clone\n\
         FPS: {fps:.1}\n\
         \n\
         XYZ: {x:.3} / {y:.3} / {z:.3}\n\
         Block: {bx} {by} {bz}\n\
         Chunk: {cx} {cy} {cz}\n\
         \n\
         Facing: {facing}\n\
         Chunks loaded: {loaded}\n\
         View dist: {vd}\n\
         AO: {ao:?}",
        x = pos.x,
        y = pos.y,
        z = pos.z,
        facing = facing,
        loaded = loaded,
        vd = view_distance.chunks(),
        ao = ao.mode,
    );
}

fn facing_dir(cam: &Transform) -> String {
    let fwd = cam.forward();
    let angle = f32::atan2(fwd.x, fwd.z).to_degrees();
    let norm = ((angle % 360.0) + 360.0) % 360.0;

    let dir = match norm {
        d if d < 22.5 || d >= 337.5 => "South",
        d if d < 67.5 => "Southwest",
        d if d < 112.5 => "West",
        d if d < 157.5 => "Northwest",
        d if d < 202.5 => "North",
        d if d < 247.5 => "Northeast",
        d if d < 292.5 => "East",
        _ => "Southeast",
    };

    format!("{dir} ({norm:.1}°)")
}

fn toggle_chunk_borders(keys: Res<ButtonInput<KeyCode>>, mut borders: ResMut<ChunkBordersVisible>) {
    if keys.just_pressed(KeyCode::KeyG) && keys.pressed(KeyCode::F3) {
        borders.0 = !borders.0;
    }
}

fn draw_chunk_borders(
    borders: Res<ChunkBordersVisible>,
    mut gizmos: Gizmos,
    player_q: Single<&Transform, With<Player>>,
    chunks: Query<&ChunkPosition>,
    view_distance: Res<ViewDistance>,
) {
    if !borders.0 {
        return;
    }

    let pcx = (player_q.translation.x / CHUNK_ISIZE as f32).floor() as i32;
    let pcz = (player_q.translation.z / CHUNK_ISIZE as f32).floor() as i32;

    let vd = view_distance.chunks();
    let s = CHUNK_ISIZE as f32;
    let color = Color::srgba(0.0, 1.0, 0.0, 0.8);

    for chunk_pos in &chunks {
        let pos = chunk_pos.0;
        let dx = (pos.x - pcx).abs();
        let dz = (pos.z - pcz).abs();
        if dx > vd || dz > vd {
            continue;
        }

        let o = Vec3::new(pos.x as f32 * s, pos.y as f32 * s, pos.z as f32 * s);
        let (x, y, z) = (Vec3::X * s, Vec3::Y * s, Vec3::Z * s);

        gizmos.line(o, o + x, color);
        gizmos.line(o, o + z, color);
        gizmos.line(o + x, o + x + z, color);
        gizmos.line(o + z, o + x + z, color);

        gizmos.line(o + y, o + x + y, color);
        gizmos.line(o + y, o + z + y, color);
        gizmos.line(o + x + y, o + x + z + y, color);
        gizmos.line(o + z + y, o + x + z + y, color);

        gizmos.line(o, o + y, color);
        gizmos.line(o + x, o + x + y, color);
        gizmos.line(o + z, o + z + y, color);
        gizmos.line(o + x + z, o + x + z + y, color);
    }
}
