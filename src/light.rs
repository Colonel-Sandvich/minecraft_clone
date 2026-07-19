use std::f32::consts::TAU;

use bevy::{
    image::{ImageLoaderSettings, ImageSampler},
    prelude::*,
};

pub struct LightPlugin;

/// Minecraft's full day is 24,000 game ticks and lasts 20 real minutes.
pub const TICKS_PER_DAY: f32 = 24_000.0;
const DEFAULT_DAY_LENGTH_SECONDS: f32 = 20.0 * 60.0;
const SKY_RADIUS: f32 = 700.0;
// The downloaded 32x32 textures contain a small centered disc with generous
// black padding, so the billboard itself needs to be larger than the apparent
// celestial body.
const CELESTIAL_SIZE: f32 = 812.0;
const DAY_AMBIENT_BRIGHTNESS: f32 = 500.0;
const NIGHT_AMBIENT_BRIGHTNESS: f32 = 20.0;
const NIGHT_DIRECTIONAL_ILLUMINANCE: f32 = 400.0;

impl Plugin for LightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DayNightCycle>()
            .register_type::<DayNightCycle>()
            .insert_resource(GlobalAmbientLight {
                color: Color::srgb(0.78, 0.86, 1.0),
                brightness: DAY_AMBIENT_BRIGHTNESS,
                affects_lightmapped_meshes: true,
            })
            .add_systems(Startup, spawn_sky_lighting)
            .add_systems(Update, update_day_night_cycle);
    }
}

/// Mutable world time. The defaults start at noon and reproduce Minecraft's
/// 20-minute day length. This is registered for the F5 inspector.
#[derive(Resource, Reflect, Debug, Clone, Copy)]
#[reflect(Resource)]
pub struct DayNightCycle {
    /// Minecraft-style time in the range `0..24000`: sunrise at 0, noon at
    /// 6000, sunset at 12000, and midnight at 18000.
    pub time_of_day_ticks: f32,
    pub day_length_seconds: f32,
    pub enabled: bool,
}

impl Default for DayNightCycle {
    fn default() -> Self {
        Self {
            time_of_day_ticks: 6_000.0,
            day_length_seconds: DEFAULT_DAY_LENGTH_SECONDS,
            enabled: true,
        }
    }
}

impl DayNightCycle {
    fn advance(&mut self, delta_seconds: f32) {
        if !self.enabled {
            return;
        }
        let day_length = self.day_length_seconds.max(1.0);
        self.time_of_day_ticks = (self.time_of_day_ticks
            + delta_seconds * TICKS_PER_DAY / day_length)
            .rem_euclid(TICKS_PER_DAY);
    }

    /// Unit vector from the camera/world toward the sun. The moon is exactly
    /// opposite, matching Minecraft's single rotating celestial plane.
    pub fn sun_direction(&self) -> Vec3 {
        let angle = self.time_of_day_ticks.rem_euclid(TICKS_PER_DAY) / TICKS_PER_DAY * TAU;
        vec3(angle.cos(), angle.sin(), 0.0)
    }

    pub(crate) fn lighting(&self) -> DayNightLighting {
        DayNightLighting::from_sun_elevation(self.sun_direction().y)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DayNightLighting {
    pub(crate) daylight: f32,
    pub(crate) sky_light_color: Vec3,
    pub(crate) sky_color: Vec3,
}

impl DayNightLighting {
    fn from_sun_elevation(sun_elevation: f32) -> Self {
        // The broad transition keeps dawn/dusk readable instead of snapping
        // when the sun crosses the horizon.
        let daylight = smoothstep(-0.18, 0.12, sun_elevation);
        let twilight = 1.0 - smoothstep(0.0, 0.32, sun_elevation.abs());

        let night_sky_light = vec3(0.08, 0.10, 0.16);
        let day_sky_light = vec3(0.94, 0.97, 1.0);
        let dusk_sky_light = vec3(0.72, 0.45, 0.34);
        let mut sky_light_color = night_sky_light.lerp(day_sky_light, daylight);
        sky_light_color = sky_light_color.lerp(dusk_sky_light, twilight * 0.22);

        let night_sky = vec3(0.008, 0.014, 0.045);
        let day_sky = vec3(0.455, 0.702, 1.0);
        let dusk_sky = vec3(0.58, 0.27, 0.16);
        let mut sky_color = night_sky.lerp(day_sky, daylight);
        sky_color = sky_color.lerp(dusk_sky, twilight * 0.55);

        Self {
            daylight,
            sky_light_color,
            sky_color,
        }
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum CelestialBody {
    Sun,
    Moon,
}

#[derive(Component)]
struct CelestialDirectionalLight;

fn load_pixel_texture(asset_server: &AssetServer, path: &'static str) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.sampler = ImageSampler::nearest();
        })
        .load(path)
}

fn celestial_material(texture: Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(texture),
        unlit: true,
        fog_enabled: false,
        // These palette textures encode transparency as solid black. Additive
        // blending makes those pixels invisible without modifying the source
        // asset, while preserving the luminous sun/moon colors.
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        ..default()
    }
}

fn spawn_sky_lighting(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Name::new("Celestial directional light"),
        CelestialDirectionalLight,
        DirectionalLight {
            illuminance: light_consts::lux::AMBIENT_DAYLIGHT,
            color: Color::srgb(1.0, 0.96, 0.9),
            // Shadow maps alias badly at the game's current draw distances.
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::default(),
    ));

    let quad = meshes.add(Rectangle::new(CELESTIAL_SIZE, CELESTIAL_SIZE));
    for (name, body, path) in [
        (
            "Sun",
            CelestialBody::Sun,
            "textures/environment/celestial/sun.png",
        ),
        (
            "Moon",
            CelestialBody::Moon,
            "textures/environment/celestial/moon/full_moon.png",
        ),
    ] {
        let texture = load_pixel_texture(&asset_server, path);
        let material = materials.add(celestial_material(texture));
        commands.spawn((
            Name::new(name),
            body,
            Mesh3d(quad.clone()),
            MeshMaterial3d(material),
            Transform::default(),
        ));
    }
}

fn update_day_night_cycle(
    time: Res<Time>,
    mut cycle: ResMut<DayNightCycle>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut celestial_bodies: Query<(&CelestialBody, &mut Transform)>,
    mut directional_lights: Query<
        (&mut DirectionalLight, &mut Transform),
        (With<CelestialDirectionalLight>, Without<CelestialBody>),
    >,
    mut ambient: ResMut<GlobalAmbientLight>,
) {
    cycle.advance(time.delta_secs());
    let sun_direction = cycle.sun_direction();
    let lighting = cycle.lighting();

    if let Some(camera_position) = cameras.iter().next().map(GlobalTransform::translation) {
        for (body, mut transform) in &mut celestial_bodies {
            let direction = match body {
                CelestialBody::Sun => sun_direction,
                CelestialBody::Moon => -sun_direction,
            };
            let position = camera_position + direction * SKY_RADIUS;
            *transform = Transform::from_translation(position).looking_at(camera_position, Vec3::Y);
        }
    }

    ambient.color = Color::srgb(
        lighting.sky_light_color.x,
        lighting.sky_light_color.y,
        lighting.sky_light_color.z,
    );
    ambient.brightness = NIGHT_AMBIENT_BRIGHTNESS
        + (DAY_AMBIENT_BRIGHTNESS - NIGHT_AMBIENT_BRIGHTNESS) * lighting.daylight;

    for (mut light, mut transform) in &mut directional_lights {
        let night = 1.0 - lighting.daylight;
        light.illuminance = light_consts::lux::AMBIENT_DAYLIGHT * lighting.daylight
            + NIGHT_DIRECTIONAL_ILLUMINANCE * night;
        light.color = Color::srgb(
            0.54 + 0.46 * lighting.daylight,
            0.62 + 0.34 * lighting.daylight,
            0.82 + 0.08 * lighting.daylight,
        );
        let light_source_direction = if lighting.daylight >= 0.5 {
            sun_direction
        } else {
            -sun_direction
        };
        *transform = Transform::default().looking_to(-light_source_direction, Vec3::Y);
    }
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minecraft_clock_places_celestials_at_cardinal_points() {
        for (ticks, expected) in [
            (0.0, Vec3::X),
            (6_000.0, Vec3::Y),
            (12_000.0, Vec3::NEG_X),
            (18_000.0, Vec3::NEG_Y),
        ] {
            let cycle = DayNightCycle {
                time_of_day_ticks: ticks,
                ..default()
            };
            assert!(cycle.sun_direction().abs_diff_eq(expected, 0.0001));
        }
    }

    #[test]
    fn full_day_uses_minecraft_twenty_minute_duration() {
        let mut cycle = DayNightCycle::default();
        cycle.advance(DEFAULT_DAY_LENGTH_SECONDS);
        assert!((cycle.time_of_day_ticks - 6_000.0).abs() < 0.01);
    }

    #[test]
    fn night_dims_skylight_without_changing_propagated_levels() {
        let noon = DayNightLighting::from_sun_elevation(1.0);
        let midnight = DayNightLighting::from_sun_elevation(-1.0);
        assert!(noon.daylight > 0.99);
        assert!(midnight.daylight < 0.01);
        assert!(noon.sky_light_color.length() > midnight.sky_light_color.length() * 5.0);
    }
}
