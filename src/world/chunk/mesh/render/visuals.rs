use bevy::{
    prelude::*,
    render::{extract_resource::ExtractResource, render_resource::ShaderType},
};

use crate::world::{
    chunk::{Chunk, ChunkCell, WorldBlockPos},
    dimension::{Active, Dimension},
};

const MINECRAFT_WATER_FOG_START: f32 = -8.0;
const MINECRAFT_WATER_FOG_END: f32 = 96.0;
const MINECRAFT_UNDERWATER_OVERLAY_ALPHA: f32 = 0.1;

#[derive(Resource, Reflect, Debug, Clone, Copy)]
#[reflect(Resource)]
pub(crate) struct TerrainVisualSettings {
    pub sky_light_color: Vec3,
    pub block_light_color: Vec3,
    pub fog_color: Vec3,
    pub fog_start: f32,
    pub fog_end: f32,
    pub fog_strength: f32,
    pub screen_tint_strength: f32,
}

impl Default for TerrainVisualSettings {
    fn default() -> Self {
        Self {
            sky_light_color: vec3(0.94, 0.97, 1.0),
            block_light_color: vec3(1.0, 0.78, 0.50),
            fog_color: vec3(0.455, 0.702, 1.0),
            fog_start: 220.0,
            fog_end: 560.0,
            fog_strength: 1.0,
            screen_tint_strength: 0.0,
        }
    }
}

impl ExtractResource for TerrainVisualSettings {
    type Source = TerrainVisualSettings;

    fn extract_resource(source: &Self::Source) -> Self {
        *source
    }
}

#[derive(Resource, Default, Clone, Copy)]
pub(super) struct TerrainAnimationClock {
    pub(super) seconds: f32,
}

impl ExtractResource for TerrainAnimationClock {
    type Source = TerrainAnimationClock;

    fn extract_resource(source: &Self::Source) -> Self {
        *source
    }
}

#[repr(C)]
#[derive(Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct TerrainVisualSettingsUniform {
    sky_light_color: [f32; 4],
    block_light_color: [f32; 4],
    fog_color: [f32; 4],
    camera_position: [f32; 4],
    fog_params: [f32; 4],
}

impl TerrainVisualSettingsUniform {
    pub(super) fn new(
        settings: TerrainVisualSettings,
        camera_position: Vec3,
        animation_seconds: f32,
    ) -> Self {
        Self {
            sky_light_color: [
                settings.sky_light_color.x,
                settings.sky_light_color.y,
                settings.sky_light_color.z,
                0.0,
            ],
            block_light_color: [
                settings.block_light_color.x,
                settings.block_light_color.y,
                settings.block_light_color.z,
                0.0,
            ],
            fog_color: [
                settings.fog_color.x,
                settings.fog_color.y,
                settings.fog_color.z,
                settings.screen_tint_strength,
            ],
            camera_position: [camera_position.x, camera_position.y, camera_position.z, 0.0],
            fog_params: [
                settings.fog_start,
                settings.fog_end,
                settings.fog_strength,
                animation_seconds,
            ],
        }
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<TerrainVisualSettings>()
        .init_resource::<TerrainAnimationClock>()
        .register_type::<TerrainVisualSettings>()
        .add_systems(
            Update,
            (update_terrain_animation_clock, update_camera_fluid_visuals),
        );
}

fn update_terrain_animation_clock(time: Res<Time>, mut clock: ResMut<TerrainAnimationClock>) {
    clock.seconds = time.elapsed_secs_wrapped();
}

fn update_camera_fluid_visuals(
    mut settings: ResMut<TerrainVisualSettings>,
    mut clear_color: ResMut<ClearColor>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    dimension: Option<Single<&Dimension, With<Active>>>,
    chunks: Query<&Chunk>,
) {
    let underwater = cameras.iter().next().is_some_and(|camera| {
        let Some(dimension) = dimension.as_deref() else {
            return false;
        };
        camera_is_underwater(camera.translation(), dimension, &chunks)
    });

    if underwater {
        settings.fog_color = minecraft_water_fog_color();
        settings.fog_start = MINECRAFT_WATER_FOG_START;
        settings.fog_end = MINECRAFT_WATER_FOG_END;
        settings.fog_strength = 1.0;
        settings.screen_tint_strength = MINECRAFT_UNDERWATER_OVERLAY_ALPHA;
        clear_color.0 = Color::srgb(
            settings.fog_color.x,
            settings.fog_color.y,
            settings.fog_color.z,
        );
    } else {
        let defaults = TerrainVisualSettings::default();
        settings.fog_color = defaults.fog_color;
        settings.fog_start = defaults.fog_start;
        settings.fog_end = defaults.fog_end;
        settings.fog_strength = defaults.fog_strength;
        settings.screen_tint_strength = 0.0;
        clear_color.0 = default_clear_color();
    }
}

fn camera_is_underwater(
    camera_position: Vec3,
    dimension: &Dimension,
    chunks: &Query<&Chunk>,
) -> bool {
    let world_pos = camera_position.floor().as_ivec3();
    let Some(fluid) =
        chunk_cell_at_world(dimension, chunks, world_pos).and_then(ChunkCell::as_fluid)
    else {
        return false;
    };
    let water_above = chunk_cell_at_world(dimension, chunks, world_pos + IVec3::Y)
        .and_then(ChunkCell::as_fluid)
        .is_some_and(|above| above.ty() == fluid.ty());
    camera_y_is_below_fluid_surface(
        camera_position.y,
        world_pos.y,
        fluid.level().get(),
        water_above,
    )
}

fn chunk_cell_at_world(
    dimension: &Dimension,
    chunks: &Query<&Chunk>,
    world_pos: IVec3,
) -> Option<ChunkCell> {
    let address = WorldBlockPos::from_ivec3(world_pos).split();
    chunks
        .get(dimension.published_chunk_entity(address.chunk())?)
        .ok()
        .map(|chunk| chunk.get_cell(address.local().as_uvec3()))
}

fn camera_y_is_below_fluid_surface(
    camera_y: f32,
    cell_y: i32,
    fluid_level: u8,
    water_above: bool,
) -> bool {
    let local_y = camera_y - cell_y as f32;
    local_y <= water_surface_height_fraction(fluid_level, water_above)
}

fn water_surface_height_fraction(fluid_level: u8, water_above: bool) -> f32 {
    if water_above {
        return 1.0;
    }
    f32::from(fluid_level.min(8)) / 9.0
}

fn minecraft_water_fog_color() -> Vec3 {
    // Vanilla default water fog color: 0x050533.
    vec3(5.0 / 255.0, 5.0 / 255.0, 0x33 as f32 / 255.0)
}

fn default_clear_color() -> Color {
    Color::srgb(0x74 as f32 / 255.0, 0xB3 as f32 / 255.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_uniform_matches_shader_layout() {
        assert_eq!(std::mem::size_of::<TerrainVisualSettingsUniform>(), 80);
    }

    #[test]
    fn water_surface_height_uses_vanilla_ninths() {
        assert!((water_surface_height_fraction(8, false) - 8.0 / 9.0).abs() < f32::EPSILON);
        assert!((water_surface_height_fraction(4, false) - 4.0 / 9.0).abs() < f32::EPSILON);
        assert_eq!(water_surface_height_fraction(8, true), 1.0);
    }

    #[test]
    fn camera_submersion_respects_water_surface_height() {
        assert!(camera_y_is_below_fluid_surface(10.85, 10, 8, false));
        assert!(!camera_y_is_below_fluid_surface(10.90, 10, 8, false));
        assert!(camera_y_is_below_fluid_surface(10.99, 10, 8, true));
    }
}
