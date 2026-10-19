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
    player::interaction::{BlockTarget, CurrentBlockTarget},
    world::chunk::{
        CHUNK_ISIZE, Chunk, ChunkBlockPos, ChunkCell, ChunkLight, ChunkPos, ChunkPosition,
        FluidProfile, FluidState, WorldBlockPos,
    },
    world::dimension::{Active, Dimension, ViewDistance},
};

use super::{
    settings::{rem, rem_font},
    z,
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

#[derive(Resource, Default)]
pub(super) struct DebugVisible(pub(super) bool);

#[derive(Resource, Default)]
struct ChunkBordersVisible(bool);

#[derive(Resource, Default)]
struct LightOverlayVisible(bool);

fn spawn_debug_overlay(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: rem_font(14.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: rem(3.0),
            left: rem(3.0),
            ..default()
        },
        DebugOverlay,
        Pickable::IGNORE,
        GlobalZIndex(z::HUD),
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
        Pickable::IGNORE,
        GlobalZIndex(z::HUD),
    ));
}

pub(super) fn toggle_debug(
    keys: Res<ButtonInput<KeyCode>>,
    mut visible: ResMut<DebugVisible>,
    mut combo: ResMut<ModifierCombo>,
) {
    if keys.just_released(KeyCode::F3) && combo.check_solo() {
        visible.0 = !visible.0;
    }
}

#[allow(clippy::too_many_arguments)]
fn update_debug_text(
    visible: Res<DebugVisible>,
    diagnostics: Res<DiagnosticsStore>,
    player_q: Single<&Transform, With<Player>>,
    cam_q: Single<(&Transform, &GlobalTransform), (With<MouseCam>, Without<Player>)>,
    view_distance: Res<ViewDistance>,
    memory: Option<Res<GameMemorySnapshot>>,
    active_dimension: Option<Single<&Dimension, With<Active>>>,
    block_target: Res<CurrentBlockTarget>,
    chunk_lights: Query<&ChunkLight>,
    chunks: Query<&Chunk>,
    mut text: Single<&mut Text, With<DebugOverlay>>,
) {
    if !visible.0 {
        if !text.0.is_empty() {
            text.0.clear();
        }
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
    let current_light = active_dimension
        .as_ref()
        .and_then(|dimension| {
            packed_light_at(dimension, light_address, |entity| {
                chunk_lights.get(entity).ok()
            })
        })
        .unwrap_or(0xF0);
    let current_sky = current_light >> 4;
    let current_block = current_light & 0x0F;

    let target = block_target.0;
    let target_cell = target.and_then(|target| {
        active_dimension.as_ref().and_then(|dimension| {
            cell_at(dimension, target.hit_block, |entity| {
                chunks.get(entity).ok()
            })
        })
    });
    let target_lines = target_report(target, target_cell);

    let facing = facing_dir(cam_transform);
    let (loaded, published) = active_dimension.as_ref().map_or((0, 0), |dimension| {
        (
            dimension.loaded_chunk_count(),
            dimension.published_chunk_count(),
        )
    });
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
         {target_lines}\n\
         \n\
         Facing: {facing}\n\
         Chunks loaded: {loaded}  published: {published}\n\
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
        target_lines = target_lines,
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

/// The crosshair "looking at" section: the block the interaction raycast
/// points at, plus the fluid state when that cell holds fluid.
fn target_report(target: Option<BlockTarget>, cell: Option<ChunkCell>) -> String {
    let Some(target) = target else {
        return "Block: none".to_owned();
    };

    let block_line = block_target_line(target, cell);
    match cell.and_then(ChunkCell::as_fluid) {
        Some(fluid) => format!(
            "{block_line}\n{}",
            fluid_target_line(target.hit_block, fluid)
        ),
        None => block_line,
    }
}

fn block_target_line(target: BlockTarget, cell: Option<ChunkCell>) -> String {
    let world = target.hit_block.world().as_ivec3();
    let chunk = target.hit_block.chunk().as_ivec3();
    let local = target.hit_block.local().as_uvec3();
    let name = cell.map_or_else(|| "unloaded".to_owned(), ChunkCell::name);

    format!(
        "Block: {name} @ ({wx}, {wy}, {wz}) chunk ({cx}, {cy}, {cz}) local ({lx}, {ly}, {lz})",
        wx = world.x,
        wy = world.y,
        wz = world.z,
        cx = chunk.x,
        cy = chunk.y,
        cz = chunk.z,
        lx = local.x,
        ly = local.y,
        lz = local.z,
    )
}

fn fluid_target_line(position: ChunkBlockPos, fluid: FluidState) -> String {
    let world = position.world().as_ivec3();

    format!(
        "Fluid: {ty} level={level} {form} falling={falling} @ ({x}, {y}, {z})",
        ty = fluid.ty(),
        level = fluid.level(),
        form = fluid.form(),
        falling = fluid_is_falling(fluid),
        x = world.x,
        y = world.y,
        z = world.z,
    )
}

/// Falling fluid is stored as a full-level flow; the simulation marks any
/// cell under the same fluid that way.
fn fluid_is_falling(fluid: FluidState) -> bool {
    fluid == FluidProfile::default_for_type(fluid.ty()).falling()
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
struct LightLabelPos(WorldBlockPos);

#[derive(Component)]
struct LightLabelParent;

fn manage_light_labels(
    mut commands: Commands,
    overlay: Res<LightOverlayVisible>,
    parent: Single<Entity, With<LightLabelParent>>,
    mut labels: Query<(Entity, &LightLabelPos, &mut Text)>,
    cam_q: Single<&GlobalTransform, With<MouseCam>>,
    active_dimension: Option<Single<&Dimension, With<Active>>>,
    chunk_lights: Query<&ChunkLight>,
) {
    if !overlay.0 {
        commands.despawn_all::<With<LightLabelPos>>();
        return;
    }

    let center = WorldBlockPos::from_translation(cam_q.translation()).offset(IVec3::NEG_Y);

    let existing: Vec<(Entity, WorldBlockPos)> = labels
        .iter()
        .map(|(entity, position, _)| (entity, position.0))
        .collect();

    let mut needed: HashMap<WorldBlockPos, String> = HashMap::new();
    for dx in -LIGHT_LABEL_RADIUS..=LIGHT_LABEL_RADIUS {
        for dz in -LIGHT_LABEL_RADIUS..=LIGHT_LABEL_RADIUS {
            let world = center.offset(IVec3::new(dx, 0, dz));
            let address = world.split();
            let light = active_dimension
                .as_ref()
                .and_then(|dimension| {
                    packed_light_at(dimension, address, |entity| chunk_lights.get(entity).ok())
                })
                .unwrap_or(0xF0);
            let sky = light >> 4;
            let block = light & 0x0F;
            needed.insert(world, format!("B{block}\nS{sky}"));
        }
    }

    let mut to_despawn: Vec<Entity> = Vec::new();
    for (entity, pos) in &existing {
        if let Some(label) = needed.remove(pos) {
            if let Ok((_, _, mut text)) = labels.get_mut(*entity)
                && text.0 != label
            {
                text.0 = label;
            }
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
                    font_size: rem_font(10.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.95, 0.75)),
                Node {
                    position_type: PositionType::Absolute,
                    // These are overwritten every frame with a world-space
                    // projection in physical/logical screen pixels below.
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    ..default()
                },
                LightLabelPos(world),
                Pickable::IGNORE,
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
        let world = pos.0.as_ivec3().as_vec3() + Vec3::splat(0.5);
        let screen = match camera.world_to_viewport(cam_transform, world) {
            Ok(vp) => vp,
            Err(_) => Vec2::new(-9999.0, -9999.0),
        };

        // Projected screen positions must not scale with the UI; they are
        // physical window coordinates centered under the world block.
        let left = Val::Px(screen.x - 10.0);
        let top = Val::Px(screen.y - 10.0);
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
    }
}

fn draw_chunk_borders(
    borders: Res<ChunkBordersVisible>,
    mut gizmos: Gizmos,
    player_q: Single<&Transform, With<Player>>,
    active_dimension: Option<Single<&Dimension, With<Active>>>,
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

    let Some(dimension) = active_dimension else {
        return;
    };
    for (position, entity) in dimension.iter_published_chunks() {
        let Ok(actual_position) = chunks.get(entity) else {
            continue;
        };
        if actual_position.chunk_pos() != position {
            continue;
        }
        let pos = position.as_ivec3();
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

fn packed_light_at<'a>(
    dimension: &Dimension,
    address: ChunkBlockPos,
    light_for_entity: impl FnOnce(Entity) -> Option<&'a ChunkLight>,
) -> Option<u8> {
    let entity = dimension.published_chunk_entity(address.chunk())?;
    light_for_entity(entity).map(|light| light.packed_light(address.local()))
}

fn cell_at<'a>(
    dimension: &Dimension,
    position: ChunkBlockPos,
    chunk_for_entity: impl FnOnce(Entity) -> Option<&'a Chunk>,
) -> Option<ChunkCell> {
    let entity = dimension.published_chunk_entity(position.chunk())?;
    chunk_for_entity(entity).map(|chunk| chunk.cell(position.local()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{item::Item, world::chunk::LocalBlockPos};

    fn sample_target() -> BlockTarget {
        BlockTarget {
            hit_block: ChunkBlockPos::new(ChunkPos::new(258, 1, 258), LocalBlockPos::new(5, 13, 5)),
            adjacent_block: ChunkBlockPos::new(
                ChunkPos::new(258, 1, 258),
                LocalBlockPos::new(5, 14, 5),
            ),
        }
    }

    #[test]
    fn target_report_is_compact_without_a_target() {
        assert_eq!(target_report(None, None), "Block: none");
    }

    #[test]
    fn target_report_names_the_block_and_every_coordinate_space() {
        let report = target_report(Some(sample_target()), Some(ChunkCell::block(Item::Stone)));

        assert_eq!(
            report,
            "Block: stone @ (4133, 29, 4133) chunk (258, 1, 258) local (5, 13, 5)"
        );
    }

    #[test]
    fn target_report_adds_a_fluid_line_for_fluid_cells() {
        let target = sample_target();

        assert_eq!(
            target_report(Some(target), Some(ChunkCell::water_source())),
            "Block: water_source_8 @ (4133, 29, 4133) chunk (258, 1, 258) local (5, 13, 5)\n\
             Fluid: water level=8 source falling=false @ (4133, 29, 4133)"
        );
    }

    #[test]
    fn fluid_target_line_reports_flow_levels_and_falling() {
        let position = sample_target().hit_block;

        assert_eq!(
            fluid_target_line(position, FluidState::water_flow(5)),
            "Fluid: water level=5 flow falling=false @ (4133, 29, 4133)"
        );
        assert_eq!(
            fluid_target_line(position, FluidProfile::WATER.falling()),
            "Fluid: water level=8 flow falling=true @ (4133, 29, 4133)"
        );
    }

    #[test]
    fn debug_light_lookup_uses_the_selected_dimension_registry() {
        let mut world = World::new();
        let active_entity = world.spawn_empty().id();
        let foreign_entity = world.spawn_empty().id();
        let position = ChunkPos::new(-4, 2, 9);
        let local = LocalBlockPos::new(3, 5, 7);
        let address = position.block(local);

        let mut active_light = ChunkLight::default();
        active_light.set_block_light(local, 3);
        let mut foreign_light = ChunkLight::default();
        foreign_light.set_block_light(local, 12);

        let mut dimension = Dimension::default();
        dimension.register_published_chunk(position, active_entity);
        let packed = packed_light_at(&dimension, address, |entity| match entity {
            entity if entity == active_entity => Some(&active_light),
            entity if entity == foreign_entity => Some(&foreign_light),
            _ => None,
        });

        assert_eq!(packed, Some(3));
    }
}
