use std::{collections::HashSet, fmt::Write, mem::size_of};

use avian3d::prelude::Collider;
use bevy::prelude::*;

use crate::{
    world::chunk::mesh::{ChunkMeshFaces, ChunkMeshLayer, ChunkMeshLight, PackedFace},
    world::{
        chunk::{Chunk, ChunkContentCounts, ChunkHeightmap, ChunkLight, ChunkPos, ChunkPosition},
        dimension::{ChunkSaveTasks, ColumnLoadTaskStats, DesiredColumnView, Dimension},
    },
};

const SNAPSHOT_INTERVAL_SECONDS: f32 = 1.0;
const CHUNK_ORIGIN_UNIFORM_BYTES: usize = 16;
const MEMORY_PROFILER_ENV: &str = "MINECRAFT_CLONE_MEMORY_PROFILER";

pub fn memory_profiler_enabled() -> bool {
    std::env::var_os(MEMORY_PROFILER_ENV).is_some()
}

pub struct MemoryTrackingPlugin;

impl Plugin for MemoryTrackingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameMemorySnapshot>()
            .init_resource::<MemoryTrackingTimer>()
            .add_systems(Update, update_memory_snapshot);
    }
}

#[derive(Resource)]
struct MemoryTrackingTimer {
    timer: Timer,
    initialized: bool,
}

impl Default for MemoryTrackingTimer {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(SNAPSHOT_INTERVAL_SECONDS, TimerMode::Repeating),
            initialized: false,
        }
    }
}

#[derive(Resource, Debug, Clone, Default)]
pub struct GameMemorySnapshot {
    pub rss_bytes: Option<usize>,
    pub virtual_bytes: Option<usize>,
    pub target_chunks: usize,
    pub chunk_count: usize,
    pub rendered_blocks: usize,
    pub solid_blocks: usize,
    pub translucent_blocks: usize,
    pub mesh_entities: usize,
    pub face_descriptors: usize,
    pub face_descriptor_capacity: usize,
    pub padded_light_components: usize,
    pub collider_entities: usize,
    pub collider_solid_blocks: usize,
    pub load_tasks: usize,
    pub load_failures: usize,
    pub save_tasks: usize,
    pub save_failures: usize,
    pub bytes: GameMemoryBytes,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GameMemoryBytes {
    pub chunk_blocks: usize,
    pub chunk_lights: usize,
    pub chunk_heightmaps: usize,
    pub chunk_metadata: usize,
    pub dimension_maps: usize,
    pub main_mesh_descriptor_used: usize,
    pub main_mesh_descriptor_capacity: usize,
    pub main_mesh_light_data: usize,
    pub main_mesh_components: usize,
    pub render_world_mesh_mirror: usize,
    pub gpu_mesh_buffers_estimate: usize,
    pub collider_shapes: usize,
    pub task_payloads: usize,
    pub tracked_cpu_total: usize,
}

impl GameMemorySnapshot {
    pub fn format_for_debug(&self) -> String {
        let mut out = String::new();
        let rss = self
            .rss_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "n/a".to_owned());
        let virt = self
            .virtual_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "n/a".to_owned());

        let _ = writeln!(
            out,
            "RSS: {rss}  VM: {virt}  tracked CPU: {}",
            format_bytes(self.bytes.tracked_cpu_total)
        );
        let _ = writeln!(
            out,
            "Chunks: {}/{}  blocks {}  light {}  height {}  meta {}",
            self.chunk_count,
            self.target_chunks,
            format_bytes(self.bytes.chunk_blocks),
            format_bytes(self.bytes.chunk_lights),
            format_bytes(self.bytes.chunk_heightmaps),
            format_bytes(self.bytes.chunk_metadata + self.bytes.dimension_maps),
        );
        let _ = writeln!(
            out,
            "Blocks: rendered {}  solid {}  translucent {}",
            self.rendered_blocks, self.solid_blocks, self.translucent_blocks
        );
        let _ = writeln!(
            out,
            "Mesh CPU: {} ents  {} faces  desc {}/{}  unique lights {}",
            self.mesh_entities,
            self.face_descriptors,
            format_bytes(self.bytes.main_mesh_descriptor_used),
            format_bytes(self.bytes.main_mesh_descriptor_capacity),
            format_bytes(self.bytes.main_mesh_light_data),
        );
        let _ = writeln!(
            out,
            "Render mirror est: CPU {}  GPU {}  padded light components {}",
            format_bytes(self.bytes.render_world_mesh_mirror),
            format_bytes(self.bytes.gpu_mesh_buffers_estimate),
            self.padded_light_components,
        );
        let _ = writeln!(
            out,
            "Colliders: {} entities  {} solid blocks  shapes {}",
            self.collider_entities,
            self.collider_solid_blocks,
            format_bytes(self.bytes.collider_shapes),
        );
        let _ = write!(
            out,
            "Tasks: load {}/{}  save {}/{}  est {}",
            self.load_tasks,
            self.load_failures,
            self.save_tasks,
            self.save_failures,
            format_bytes(self.bytes.task_payloads),
        );

        out
    }

    fn format_for_log(&self) -> String {
        format!(
            "MEMORY rss_bytes={} vm_bytes={} tracked_cpu_bytes={} untracked_rss_bytes={} chunks={} target_chunks={} rendered_blocks={} solid_blocks={} translucent_blocks={} chunk_blocks_bytes={} chunk_lights_bytes={} chunk_heightmaps_bytes={} chunk_meta_bytes={} dimension_maps_bytes={} mesh_entities={} face_descriptors={} face_descriptor_capacity={} mesh_desc_used_bytes={} mesh_desc_capacity_bytes={} main_mesh_light_data_bytes={} render_mirror_bytes={} gpu_mesh_buffers_estimate_bytes={} padded_light_components={} collider_entities={} collider_solid_blocks={} collider_shapes_bytes={} load_tasks={} load_failures={} save_tasks={} save_failures={} task_payloads_bytes={}",
            self.rss_bytes.unwrap_or(0),
            self.virtual_bytes.unwrap_or(0),
            self.bytes.tracked_cpu_total,
            self.rss_bytes
                .unwrap_or(0)
                .saturating_sub(self.bytes.tracked_cpu_total),
            self.chunk_count,
            self.target_chunks,
            self.rendered_blocks,
            self.solid_blocks,
            self.translucent_blocks,
            self.bytes.chunk_blocks,
            self.bytes.chunk_lights,
            self.bytes.chunk_heightmaps,
            self.bytes.chunk_metadata,
            self.bytes.dimension_maps,
            self.mesh_entities,
            self.face_descriptors,
            self.face_descriptor_capacity,
            self.bytes.main_mesh_descriptor_used,
            self.bytes.main_mesh_descriptor_capacity,
            self.bytes.main_mesh_light_data,
            self.bytes.render_world_mesh_mirror,
            self.bytes.gpu_mesh_buffers_estimate,
            self.padded_light_components,
            self.collider_entities,
            self.collider_solid_blocks,
            self.bytes.collider_shapes,
            self.load_tasks,
            self.load_failures,
            self.save_tasks,
            self.save_failures,
            self.bytes.task_payloads,
        )
    }
}

fn update_memory_snapshot(
    time: Res<Time>,
    mut timer: ResMut<MemoryTrackingTimer>,
    mut snapshot: ResMut<GameMemorySnapshot>,
    chunk_q: Query<(&ChunkContentCounts, Option<&Children>), With<Chunk>>,
    collider_q: Query<&Collider>,
    mesh_q: Query<&ChunkMeshLayer>,
    mesh_faces_q: Query<&ChunkMeshFaces>,
    mesh_light_q: Query<&ChunkMeshLight>,
    dimensions_q: Query<&Dimension>,
    desired_view: Res<DesiredColumnView>,
    save_tasks: Option<Res<ChunkSaveTasks>>,
) {
    if timer.initialized && !timer.timer.tick(time.delta()).just_finished() {
        return;
    }
    timer.initialized = true;

    let mut chunk_count = 0usize;
    let mut rendered_blocks = 0usize;
    let mut solid_blocks = 0usize;
    let mut translucent_blocks = 0usize;
    let mut collider_entities = 0usize;
    let mut collider_solid_blocks = 0usize;
    let mut collider_shape_bytes = 0usize;

    for (counts, children) in &chunk_q {
        let chunk_solid_blocks = counts.solid as usize;
        chunk_count += 1;
        rendered_blocks += counts.rendered as usize;
        translucent_blocks += counts.translucent as usize;
        solid_blocks += chunk_solid_blocks;

        if let Some(children) = children {
            let mut chunk_collider_entities = 0usize;
            for child in children {
                let Ok(collider) = collider_q.get(*child) else {
                    continue;
                };

                chunk_collider_entities += 1;
                collider_shape_bytes =
                    collider_shape_bytes.saturating_add(collider.shape().as_voxels().map_or_else(
                        || chunk_solid_blocks.saturating_mul(size_of::<(Vec3, Quat, Collider)>()),
                        |voxels| voxels.total_memory_size(),
                    ));
            }
            collider_entities += chunk_collider_entities;
            if chunk_collider_entities > 0 {
                collider_solid_blocks += chunk_solid_blocks;
            }
        }
    }

    let mesh_entities = mesh_q
        .contiguous_iter()
        .expect("ChunkMeshLayer memory scan should stay dense")
        .map(<[ChunkMeshLayer]>::len)
        .sum();

    let mut face_descriptors = 0usize;
    let mut face_descriptor_capacity = 0usize;
    for face_components in mesh_faces_q
        .contiguous_iter()
        .expect("ChunkMeshFaces memory scan should stay dense")
    {
        for faces in face_components {
            face_descriptors += faces.len();
            face_descriptor_capacity += faces.capacity();
        }
    }

    let mut padded_light_components = 0usize;
    let mut unique_padded_light_words = 0usize;
    let mut gpu_padded_light_words_estimate = 0usize;
    let mut unique_padded_lights = HashSet::new();
    for lights in mesh_light_q
        .contiguous_iter()
        .expect("ChunkMeshLight memory scan should stay dense")
    {
        padded_light_components += lights.len();
        for light in lights {
            gpu_padded_light_words_estimate += light.data().len();
            if unique_padded_lights.insert(light.data_key()) {
                unique_padded_light_words += light.data().len();
            }
        }
    }

    let target_chunks = desired_view.chunk_count();
    let dimension_maps = dimensions_q
        .iter()
        .map(|dimension| dimension.chunk_map_capacity() * size_of::<(ChunkPos, Entity)>())
        .sum::<usize>();

    let load_stats = dimensions_q.iter().map(Dimension::load_task_stats).fold(
        ColumnLoadTaskStats::default(),
        |mut total, stats| {
            total.tasks = total.tasks.saturating_add(stats.tasks);
            total.failures = total.failures.saturating_add(stats.failures);
            total.estimated_payload_bytes = total
                .estimated_payload_bytes
                .saturating_add(stats.estimated_payload_bytes);
            total
        },
    );
    let save_stats = save_tasks
        .as_deref()
        .map(ChunkSaveTasks::stats)
        .unwrap_or_default();

    let process_memory = ProcessMemory::read();
    let bytes = compute_memory_bytes(MemoryByteInputs {
        chunk_count,
        mesh_entities,
        face_descriptors,
        face_descriptor_capacity,
        padded_light_components,
        unique_padded_light_words,
        gpu_padded_light_words_estimate,
        collider_entities,
        dimension_maps,
        collider_shape_bytes,
        task_payloads: load_stats
            .estimated_payload_bytes
            .saturating_add(save_stats.estimated_payload_bytes),
    });

    *snapshot = GameMemorySnapshot {
        rss_bytes: process_memory.rss_bytes,
        virtual_bytes: process_memory.virtual_bytes,
        target_chunks,
        chunk_count,
        rendered_blocks,
        solid_blocks,
        translucent_blocks,
        mesh_entities,
        face_descriptors,
        face_descriptor_capacity,
        padded_light_components,
        collider_entities,
        collider_solid_blocks,
        load_tasks: load_stats.tasks,
        load_failures: load_stats.failures,
        save_tasks: save_stats.tasks,
        save_failures: save_stats.failures,
        bytes,
    };
    println!("{}", snapshot.format_for_log());
}

struct MemoryByteInputs {
    chunk_count: usize,
    mesh_entities: usize,
    face_descriptors: usize,
    face_descriptor_capacity: usize,
    padded_light_components: usize,
    unique_padded_light_words: usize,
    gpu_padded_light_words_estimate: usize,
    collider_entities: usize,
    dimension_maps: usize,
    collider_shape_bytes: usize,
    task_payloads: usize,
}

fn compute_memory_bytes(input: MemoryByteInputs) -> GameMemoryBytes {
    let chunk_blocks = bytes_for::<Chunk>(input.chunk_count);
    let chunk_lights = bytes_for::<ChunkLight>(input.chunk_count);
    let chunk_heightmaps = bytes_for::<ChunkHeightmap>(input.chunk_count);
    let chunk_metadata = bytes_for::<ChunkContentCounts>(input.chunk_count)
        .saturating_add(bytes_for::<ChunkPosition>(input.chunk_count))
        .saturating_add(bytes_for::<Transform>(input.chunk_count))
        .saturating_add(bytes_for::<Visibility>(input.chunk_count));
    let main_mesh_descriptor_used = bytes_for::<PackedFace>(input.face_descriptors);
    let main_mesh_descriptor_capacity = bytes_for::<PackedFace>(input.face_descriptor_capacity);
    let main_mesh_light_data = input
        .unique_padded_light_words
        .saturating_mul(size_of::<u32>());
    let light_component_bytes = bytes_for::<ChunkMeshLight>(input.padded_light_components);
    let main_mesh_components =
        bytes_for::<ChunkMeshLayer>(input.mesh_entities).saturating_add(light_component_bytes);
    let render_world_mesh_mirror = bytes_for::<ChunkMeshLayer>(input.mesh_entities)
        .saturating_add(main_mesh_descriptor_used)
        .saturating_add(light_component_bytes);
    let gpu_mesh_buffers_estimate = main_mesh_descriptor_used
        .saturating_add(
            input
                .mesh_entities
                .saturating_mul(CHUNK_ORIGIN_UNIFORM_BYTES),
        )
        .saturating_add(
            input
                .gpu_padded_light_words_estimate
                .saturating_mul(size_of::<u32>()),
        );
    let collider_shapes = input
        .collider_shape_bytes
        .saturating_add(bytes_for::<Collider>(input.collider_entities));

    let tracked_cpu_total = chunk_blocks
        .saturating_add(chunk_lights)
        .saturating_add(chunk_heightmaps)
        .saturating_add(chunk_metadata)
        .saturating_add(input.dimension_maps)
        .saturating_add(main_mesh_descriptor_capacity)
        .saturating_add(main_mesh_light_data)
        .saturating_add(main_mesh_components)
        .saturating_add(render_world_mesh_mirror)
        .saturating_add(collider_shapes)
        .saturating_add(input.task_payloads);

    GameMemoryBytes {
        chunk_blocks,
        chunk_lights,
        chunk_heightmaps,
        chunk_metadata,
        dimension_maps: input.dimension_maps,
        main_mesh_descriptor_used,
        main_mesh_descriptor_capacity,
        main_mesh_light_data,
        main_mesh_components,
        render_world_mesh_mirror,
        gpu_mesh_buffers_estimate,
        collider_shapes,
        task_payloads: input.task_payloads,
        tracked_cpu_total,
    }
}

fn bytes_for<T>(count: usize) -> usize {
    count.saturating_mul(size_of::<T>())
}

#[derive(Debug, Clone, Copy, Default)]
struct ProcessMemory {
    rss_bytes: Option<usize>,
    virtual_bytes: Option<usize>,
}

impl ProcessMemory {
    fn read() -> Self {
        let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
            return Self::default();
        };

        Self {
            rss_bytes: proc_status_bytes(&status, "VmRSS:"),
            virtual_bytes: proc_status_bytes(&status, "VmSize:"),
        }
    }
}

fn proc_status_bytes(status: &str, label: &str) -> Option<usize> {
    status.lines().find_map(|line| {
        line.strip_prefix(label).and_then(|value| {
            value
                .split_whitespace()
                .next()
                .and_then(|kb| kb.parse::<usize>().ok())
                .map(|kb| kb.saturating_mul(1024))
        })
    })
}

pub fn format_bytes(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{bytes:.0} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_binary_byte_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MiB");
    }
}
