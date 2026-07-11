//! Real-renderer stress scenario for chunk mesh light uploads.
//!
//! Run with:
//! `MINECRAFT_CLONE_DISABLE_CHUNK_COLLIDERS=1 RUST_LOG=perf=info \
//! cargo run --release --example perf_light_upload -- 256 16`
//!
//! The optional argument is the maximum number of loaded chunks whose light
//! data is re-uploaded each fixed tick. The second optional argument is the
//! run duration in seconds. The normal `perf` log reports frame time and
//! actual uploads every five seconds.

use bevy::{prelude::*, window::PresentMode};
use minecraft_clone::{
    AppPlugin,
    world::chunk::{Chunk, ChunkNeedsRenderLightUpload},
};

#[derive(Resource)]
struct LightUploadStress {
    chunks_per_tick: usize,
    run_for_seconds: f32,
}

fn configure_window(mut windows: Query<&mut Window>) {
    for mut window in &mut windows {
        window.present_mode = PresentMode::AutoNoVsync;
    }
}

fn force_light_uploads(
    mut commands: Commands,
    stress: Res<LightUploadStress>,
    chunks: Query<Entity, With<Chunk>>,
) {
    for entity in chunks.iter().take(stress.chunks_per_tick) {
        commands.entity(entity).insert(ChunkNeedsRenderLightUpload);
    }
}

fn exit_after_duration(
    time: Res<Time<Real>>,
    stress: Res<LightUploadStress>,
    mut exit: MessageWriter<AppExit>,
) {
    if time.elapsed_secs() >= stress.run_for_seconds {
        exit.write(AppExit::Success);
    }
}

fn main() {
    let chunks_per_tick = std::env::args()
        .nth(1)
        .map(|value| {
            value
                .parse::<usize>()
                .expect("light-upload chunk count must be an unsigned integer")
        })
        .unwrap_or(256);
    let run_for_seconds = std::env::args()
        .nth(2)
        .map(|value| {
            value
                .parse::<f32>()
                .expect("run duration must be a number of seconds")
        })
        .unwrap_or(16.0);

    App::new()
        .insert_resource(LightUploadStress {
            chunks_per_tick,
            run_for_seconds,
        })
        .add_plugins(AppPlugin)
        .add_systems(Startup, configure_window)
        .add_systems(Update, (force_light_uploads, exit_after_duration))
        .run();
}
