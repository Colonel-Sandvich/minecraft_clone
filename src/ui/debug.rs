use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    platform::collections::HashMap,
    prelude::*,
};

use crate::{
    input::ModifierCombo,
    memory::GameMemorySnapshot,
    player::Player,
    player::cam::MouseCam,
    world::chunk::{CHUNK_ISIZE, ChunkLight, ChunkPos, ChunkPosition, WorldBlockPos},
    world::dimension::ViewDistance,
};

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugVisible>()
            .init_resource::<ChunkBordersVisible>()
            .init_resource::<LightOverlayVisible>()
            .init_resource::<ModifierCombo>()
            .add_systems(Startup, (spawn_debug_overlay, spawn_label_parent))
            .add_systems(PreUpdate, manage_light_labels)
            .add_systems(
                Update,
                (
                    toggle_debug,
                    update_debug_text,
                    toggle_chunk_borders,
                    toggle_light_overlay,
                    draw_chunk_borders,
                    update_light_label_positions,
                ),
            );
    }
}

#[derive(Component)]
struct DebugOverlay;

#[derive(Component)]
struct LightOverlayLabel;

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
            font_size: FontSize::Px(14.0),
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

fn spawn_label_parent(mut commands: Commands) {
    commands.spawn((
        Node {
            width: Val::Vw(100.0),
            height: Val::Vh(100.0),
            position_type: PositionType::Absolute,
            ..default()
        },
        LightLabelParent,
    ));
}

fn toggle_debug(
    keys: Res<ButtonInput<KeyCode>>,
    mut visible: ResMut<DebugVisible>,
    mut combo: ResMut<ModifierCombo>,
) {
    if keys.just_released(KeyCode::F3) && combo.check_solo() {
        visible.0 = !visible.0;
    }
}

fn update_debug_text(
    visible: Res<DebugVisible>,
    diagnostics: Res<DiagnosticsStore>,
    player_q: Single<&Transform, With<Player>>,
    cam_q: Single<(&Transform, &GlobalTransform), (With<MouseCam>, Without<Player>)>,
    view_distance: Res<ViewDistance>,
    memory: Option<Res<GameMemorySnapshot>>,
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
    let block = pos.floor().as_ivec3();
    let chunk = ChunkPos::containing_translation(pos).as_ivec3();

    let (cam_transform, cam_global) = *cam_q;
    let light_world = cam_global.translation().floor().as_ivec3() + IVec3::NEG_Y;
    let light_address = WorldBlockPos::from_ivec3(light_world).split();
    let light_map: HashMap<IVec3, &ChunkLight> = chunk_lights
        .iter()
        .map(|(p, l)| (p.as_ivec3(), l))
        .collect();
    let current_light = light_map
        .get(&light_address.chunk().as_ivec3())
        .map(|l| l.packed_light(light_address.local().as_uvec3()))
        .unwrap_or(0xF0);
    let current_sky = current_light >> 4;
    let current_block = current_light & 0x0F;

    let facing = facing_dir(cam_transform);
    let loaded = chunk_count.iter().count();
    let memory = memory
        .as_deref()
        .map(|memory| format!("\n\nMemory:\n{}", memory.format_for_debug()))
        .unwrap_or_default();

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
         View dist: {vd}{memory}",
        x = pos.x,
        y = pos.y,
        z = pos.z,
        bx = block.x,
        by = block.y,
        bz = block.z,
        cx = chunk.x,
        cy = chunk.y,
        cz = chunk.z,
        ft_sky = current_sky,
        ft_block = current_block,
        facing = facing,
        loaded = loaded,
        vd = view_distance.chunks(),
        memory = memory,
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

    format!("{dir} ({norm:.1})")
}

fn toggle_chunk_borders(
    keys: Res<ButtonInput<KeyCode>>,
    mut borders: ResMut<ChunkBordersVisible>,
    mut combo: ResMut<ModifierCombo>,
) {
    if keys.just_pressed(KeyCode::KeyG) && keys.pressed(KeyCode::F3) {
        borders.0 = !borders.0;
        combo.mark_combo();
    }
}

fn toggle_light_overlay(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<LightOverlayVisible>,
    mut combo: ResMut<ModifierCombo>,
) {
    if keys.just_pressed(KeyCode::KeyL) && keys.pressed(KeyCode::F3) {
        overlay.0 = !overlay.0;
        combo.mark_combo();
    }
}

const LIGHT_LABEL_RADIUS: i32 = 4;

#[derive(Component)]
struct LightLabelPos(IVec3);

#[derive(Component)]
struct LightLabelParent;

fn manage_light_labels(
    mut commands: Commands,
    overlay: Res<LightOverlayVisible>,
    parent: Single<Entity, With<LightLabelParent>>,
    labels: Query<(Entity, &LightLabelPos)>,
    cam_q: Single<&GlobalTransform, With<MouseCam>>,
    chunk_lights: Query<(&ChunkPosition, &ChunkLight)>,
) {
    if !overlay.0 {
        for (entity, _) in &labels {
            commands.entity(entity).despawn();
        }
        return;
    }

    let center = cam_q.translation().floor().as_ivec3() + IVec3::NEG_Y;

    let light_map: HashMap<IVec3, &ChunkLight> = chunk_lights
        .iter()
        .map(|(p, l)| (p.as_ivec3(), l))
        .collect();

    let existing: Vec<(Entity, IVec3)> = labels.iter().map(|(e, p)| (e, p.0)).collect();

    let mut needed: HashMap<IVec3, String> = HashMap::new();
    for dx in -LIGHT_LABEL_RADIUS..=LIGHT_LABEL_RADIUS {
        for dz in -LIGHT_LABEL_RADIUS..=LIGHT_LABEL_RADIUS {
            let world = center + IVec3::new(dx, 0, dz);
            let address = WorldBlockPos::from_ivec3(world).split();
            let light = light_map
                .get(&address.chunk().as_ivec3())
                .map(|l| l.packed_light(address.local().as_uvec3()))
                .unwrap_or(0xF0);
            let sky = light >> 4;
            let block = light & 0x0F;
            needed.insert(world, format!("B{block}\nS{sky}"));
        }
    }

    let mut to_despawn: Vec<Entity> = Vec::new();
    for (entity, pos) in &existing {
        if let Some(label) = needed.remove(pos) {
            commands.entity(*entity).insert(Text::new(label));
        } else {
            to_despawn.push(*entity);
        }
    }

    for entity in to_despawn {
        commands.entity(entity).despawn();
    }

    let parent_entity = *parent;
    for (world, label) in needed {
        commands.entity(parent_entity).with_children(|p| {
            p.spawn((
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(10.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.95, 0.75)),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    ..default()
                },
                LightLabelPos(world),
                LightOverlayLabel,
            ));
        });
    }
}

fn update_light_label_positions(
    camera_q: Single<(&Camera, &GlobalTransform), With<MouseCam>>,
    mut labels: Query<(&LightLabelPos, &mut Node)>,
) {
    let (camera, cam_transform) = *camera_q;

    for (pos, mut node) in &mut labels {
        let world = pos.0.as_vec3() + Vec3::splat(0.5);
        let screen = match camera.world_to_viewport(cam_transform, world) {
            Ok(vp) => vp,
            Err(_) => Vec2::new(-9999.0, -9999.0),
        };

        node.left = Val::Px(screen.x - 10.0);
        node.top = Val::Px(screen.y - 10.0);
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

    let player_chunk = ChunkPos::containing_translation(player_q.translation).as_ivec3();

    let vd = view_distance.chunks();
    let s = CHUNK_ISIZE as f32;
    let color = Color::srgba(0.0, 1.0, 0.0, 0.8);

    for chunk_pos in &chunks {
        let pos = chunk_pos.as_ivec3();
        let dx = (pos.x - player_chunk.x).abs();
        let dz = (pos.z - player_chunk.z).abs();
        if dx > vd || dz > vd {
            continue;
        }

        let o = pos.as_vec3() * s;
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
