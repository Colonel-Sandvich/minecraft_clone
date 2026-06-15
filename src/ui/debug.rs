use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    platform::collections::HashMap,
    prelude::*,
};

use crate::{
    player::Player,
    player::cam::MouseCam,
    world::chunk::ambient_occlusion::AmbientOcclusionSettings,
    world::chunk::{
        CHUNK_ISIZE, ChunkLight, ChunkPosition, light::world_to_chunk_local,
    },
    world::dimension::ViewDistance,
};

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugVisible>()
            .init_resource::<ChunkBordersVisible>()
            .init_resource::<LightOverlayVisible>()
            .add_systems(Startup, spawn_debug_overlay)
            .add_systems(
                Update,
                (
                    toggle_debug,
                    update_debug_text,
                    toggle_chunk_borders,
                    toggle_light_overlay,
                    draw_chunk_borders,
                    draw_light_overlay,
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

#[derive(Resource, Default)]
struct LightOverlayVisible(bool);

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
    chunk_lights: Query<(&ChunkPosition, &ChunkLight)>,
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

    let foot_world = IVec3::new(bx, by - 1, bz);
    let (foot_chunk, foot_local) = world_to_chunk_local(foot_world);
    let light_map: HashMap<IVec3, &ChunkLight> = chunk_lights
        .iter()
        .map(|(p, l)| (p.0, l))
        .collect();
    let foot_light = light_map
        .get(&foot_chunk)
        .map(|l| l.packed_light(foot_local))
        .unwrap_or(0xF0);
    let foot_sky = foot_light >> 4;
    let foot_block = foot_light & 0x0F;

    let facing = facing_dir(&cam_q);
    let loaded = chunk_count.iter().count();

    text.0 = format!(
        "Minecraft Clone\n\
         FPS: {fps:.1}\n\
         \n\
         XYZ: {x:.3} / {y:.3} / {z:.3}\n\
         Block: {bx} {by} {bz}\n\
         Light: {ft_sky} sky  {ft_block} block\n\
         Chunk: {cx} {cy} {cz}\n\
         \n\
         Facing: {facing}\n\
         Chunks loaded: {loaded}\n\
         View dist: {vd}\n\
         AO: {ao:?}",
        x = pos.x,
        y = pos.y,
        z = pos.z,
        ft_sky = foot_sky,
        ft_block = foot_block,
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

fn toggle_light_overlay(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<LightOverlayVisible>,
) {
    if keys.just_pressed(KeyCode::KeyL) && keys.pressed(KeyCode::F3) {
        overlay.0 = !overlay.0;
    }
}

fn light_color(level: u8) -> Color {
    let t = level as f32 / 15.0;
    if level < 7 {
        Color::srgba(1.0, t * 0.5, 0.0, 0.7)
    } else {
        Color::srgba(t * 0.6, 0.9, t * 0.3, 0.6)
    }
}

fn draw_light_overlay(
    overlay: Res<LightOverlayVisible>,
    mut gizmos: Gizmos,
    player_q: Single<&Transform, With<Player>>,
    chunks: Query<(&ChunkPosition, &ChunkLight)>,
) {
    if !overlay.0 {
        return;
    }

    let chunk_map: HashMap<IVec3, &ChunkLight> = chunks
        .iter()
        .map(|(pos, light)| (pos.0, light))
        .collect();

    let radius: i32 = 5;
    let player_pos = player_q.translation;
    let origin = IVec3::new(
        player_pos.x.floor() as i32,
        player_pos.y.floor() as i32,
        player_pos.z.floor() as i32,
    );

    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                let world = origin + IVec3::new(dx, dy, dz);
                let (chunk_pos, local) = world_to_chunk_local(world);

                let light = chunk_map
                    .get(&chunk_pos)
                    .map(|l| l.packed_light(local))
                    .unwrap_or(0xF0);

                let block_light = light & 0x0F;

                let base = Vec3::new(world.x as f32, world.y as f32, world.z as f32);
                let s = 1.01;
                let color = light_color(block_light);

                gizmos.line(base, base + Vec3::X * s, color);
                gizmos.line(base, base + Vec3::Z * s, color);
                gizmos.line(base + Vec3::X * s, base + Vec3::X * s + Vec3::Z * s, color);
                gizmos.line(base + Vec3::Z * s, base + Vec3::X * s + Vec3::Z * s, color);

                gizmos.line(base + Vec3::Y * s, base + Vec3::X * s + Vec3::Y * s, color);
                gizmos.line(base + Vec3::Y * s, base + Vec3::Z * s + Vec3::Y * s, color);
                gizmos.line(base + Vec3::X * s + Vec3::Y * s, base + Vec3::X * s + Vec3::Z * s + Vec3::Y * s, color);
                gizmos.line(base + Vec3::Z * s + Vec3::Y * s, base + Vec3::X * s + Vec3::Z * s + Vec3::Y * s, color);

                gizmos.line(base, base + Vec3::Y * s, color);
                gizmos.line(base + Vec3::X * s, base + Vec3::X * s + Vec3::Y * s, color);
                gizmos.line(base + Vec3::Z * s, base + Vec3::Z * s + Vec3::Y * s, color);
                gizmos.line(base + Vec3::X * s + Vec3::Z * s, base + Vec3::X * s + Vec3::Z * s + Vec3::Y * s, color);
            }
        }
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
