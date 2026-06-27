use std::time::Duration;

use avian3d::PhysicsPlugins;
#[cfg(debug_assertions)]
use bevy::render::settings::InstanceFlags;
use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    input::common_conditions::input_toggle_active,
    prelude::*,
    render::{
        RenderPlugin,
        settings::{RenderCreation, WgpuSettings},
    },
};
use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};
use bevy_settings::SettingsPlugin;

use crate::{
    block::BlockPlugin,
    game_state::GameStatePlugin,
    light::LightPlugin,
    memory::{MemoryTrackingPlugin, memory_profiler_enabled},
    mob::MobControllerPlugin,
    player::{PlayerPlugin, cam::MouseSettings},
    textures::BlockTexturePlugin,
    ui::UIPlugin,
    world::chunk::{
        Chunk, ChunkNeedsLightRebuild, ChunkNeedsLightUpload, ChunkNeedsMeshRebuild,
        ChunkPerfCounters, ChunkPosition,
        mesh::vertex_pulling::{VertexPullingMesh, VertexPullingPlugin},
    },
    world::{WorldConfig, WorldMetadata, WorldPlugin, dimension::ViewDistance},
};

pub const FIXED_TICK_RATE_HZ: f64 = 20.0;

pub struct AppPlugin;

impl Plugin for AppPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Minecraft".to_string(),
                        ..default()
                    }),
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(Box::new(wgpu_settings())),
                    ..default()
                }),
        )
        .register_type::<ViewDistance>()
        .register_type::<MouseSettings>()
        .add_plugins(SettingsPlugin::new("io.github.matt.minecraft_clone"))
        .add_plugins(EguiPlugin::default())
        .insert_resource(ClearColor(Srgba::hex("74b3ff").unwrap().into()))
        .add_plugins(GameStatePlugin)
        .add_plugins(LightPlugin)
        .add_plugins(MobControllerPlugin)
        .add_plugins(PlayerPlugin)
        .add_plugins(BlockPlugin)
        .add_plugins(BlockTexturePlugin)
        .insert_resource({
            let metadata = WorldMetadata::default();
            #[cfg(feature = "turso-store")]
            {
                WorldConfig::development_turso(metadata)
            }
            #[cfg(not(feature = "turso-store"))]
            {
                WorldConfig::development_sqlite(metadata)
            }
        })
        .add_plugins(WorldPlugin)
        .add_plugins(VertexPullingPlugin)
        .add_plugins(UIPlugin)
        .insert_resource(Time::<Fixed>::from_hz(FIXED_TICK_RATE_HZ))
        .insert_resource(Time::<Virtual>::from_max_delta(Duration::from_secs_f64(
            1.0 / FIXED_TICK_RATE_HZ,
        )))
        .add_plugins(PhysicsPlugins::default())
        // .add_plugins(PhysicsDebugPlugin::default())
        .add_plugins((FrameTimeDiagnosticsPlugin::default(),))
        .add_systems(Update, log_frame_perf)
        .add_plugins(WorldInspectorPlugin::new().run_if(input_toggle_active(false, KeyCode::F5)));
        if memory_profiler_enabled() {
            app.add_plugins(MemoryTrackingPlugin);
        }
        // .insert_resource(FramepaceSettings {
        //     limiter: Limiter::from_framerate(60.0),
        // })
    }
}

pub fn run() {
    App::new().add_plugins(AppPlugin).run();
}

fn wgpu_settings() -> WgpuSettings {
    #[allow(unused_mut)]
    let mut settings = WgpuSettings::default();
    #[cfg(debug_assertions)]
    {
        settings.instance_flags.insert(InstanceFlags::debugging());
    }
    settings
}

fn log_frame_perf(
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    chunks: Query<(), With<Chunk>>,
    dirty_mesh_chunks: Query<&ChunkPosition, (With<Chunk>, With<ChunkNeedsMeshRebuild>)>,
    dirty_light_upload_chunks: Query<&ChunkPosition, (With<Chunk>, With<ChunkNeedsLightUpload>)>,
    dirty_light_rebuild_chunks: Query<&ChunkPosition, (With<Chunk>, With<ChunkNeedsLightRebuild>)>,
    vp_meshes: Query<&VertexPullingMesh>,
    mut chunk_perf: Option<ResMut<ChunkPerfCounters>>,
    mut timer: Local<Option<Timer>>,
) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(5.0, TimerMode::Repeating));
    if !timer.tick(time.delta()).just_finished() {
        return;
    }

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|diagnostic| diagnostic.smoothed())
        .unwrap_or_default();
    let frame_ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|diagnostic| diagnostic.smoothed())
        .unwrap_or_else(|| time.delta_secs_f64() * 1000.0);

    let mut vp_layers = 0usize;
    let mut vp_faces = 0u64;
    for mesh in &vp_meshes {
        vp_layers += 1;
        vp_faces += u64::from(mesh.face_count);
    }
    let dirty_mesh_positions = dirty_mesh_chunks
        .iter()
        .map(|pos| pos.0)
        .collect::<Vec<_>>();
    let dirty_light_upload_positions = dirty_light_upload_chunks
        .iter()
        .map(|pos| pos.0)
        .collect::<Vec<_>>();
    let dirty_light_rebuild_positions = dirty_light_rebuild_chunks
        .iter()
        .map(|pos| pos.0)
        .collect::<Vec<_>>();
    let dirty_mesh_sample = format_position_sample(&dirty_mesh_positions);
    let dirty_light_upload_sample = format_position_sample(&dirty_light_upload_positions);
    let dirty_light_rebuild_sample = format_position_sample(&dirty_light_rebuild_positions);
    let chunk_perf = chunk_perf
        .as_deref_mut()
        .map(ChunkPerfCounters::take)
        .unwrap_or_default();

    info!(
        target: "perf",
        fps = format_args!("{fps:.1}"),
        frame_ms = format_args!("{frame_ms:.3}"),
        chunks = chunks.iter().count(),
        dirty_mesh = dirty_mesh_positions.len(),
        dirty_mesh_sample = %dirty_mesh_sample,
        dirty_light_upload = dirty_light_upload_positions.len(),
        dirty_light_upload_sample = %dirty_light_upload_sample,
        dirty_light_rebuild = dirty_light_rebuild_positions.len(),
        dirty_light_rebuild_sample = %dirty_light_rebuild_sample,
        mesh_rebuilds_5s = chunk_perf.mesh_rebuilds,
        light_rebuild_targets_5s = chunk_perf.light_rebuild_targets,
        light_uploads_5s = chunk_perf.light_uploads,
        vp_layers,
        vp_faces,
        "frame perf"
    );
}

fn format_position_sample(positions: &[IVec3]) -> String {
    if positions.is_empty() {
        return "[]".to_string();
    }

    let mut sample = positions
        .iter()
        .take(8)
        .map(|pos| format!("{},{},{}", pos.x, pos.y, pos.z))
        .collect::<Vec<_>>()
        .join(" ");
    if positions.len() > 8 {
        sample.push_str(" ...");
    }
    format!("[{sample}]")
}
