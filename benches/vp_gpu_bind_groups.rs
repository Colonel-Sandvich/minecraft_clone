//! Headless GPU-API benchmark for the vertex-pulling light binding strategies.
//!
//! This deliberately measures the operation that differs for a light-only
//! update: allocating a storage buffer and bind group versus writing the same
//! fixed-size data into an existing `COPY_DST` buffer.
//!
//! Run with `cargo bench --bench vp_gpu_bind_groups`.

use std::{hint::black_box, time::Duration};

use bevy::tasks::block_on;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, Buffer, BufferBindingType, BufferUsages,
    Device, DeviceDescriptor, ExperimentalFeatures, Features, Instance, InstanceDescriptor, Limits,
    MemoryHints, PollType, Queue, RequestAdapterOptions, ShaderStages, Trace, util::DeviceExt,
};

const PADDED_LIGHT_WORDS: usize = 18 * 18 * 18 / 4;

struct GpuContext {
    device: Device,
    queue: Queue,
    combined_layout: BindGroupLayout,
    mesh_layout: BindGroupLayout,
    light_layout: BindGroupLayout,
    light_data: Vec<u32>,
}

impl GpuContext {
    fn new() -> Self {
        let instance = Instance::new(InstanceDescriptor::new_without_display_handle());
        let adapter = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
            .expect("headless GPU adapter is required for this benchmark");
        let adapter_info = adapter.get_info();
        eprintln!(
            "vp_gpu_bind_groups adapter: {} ({:?})",
            adapter_info.name, adapter_info.backend
        );
        let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("vp_gpu_bind_groups_bench"),
            required_features: Features::empty(),
            required_limits: Limits::default(),
            experimental_features: ExperimentalFeatures::disabled(),
            memory_hints: MemoryHints::Performance,
            trace: Trace::Off,
        }))
        .expect("headless GPU device should initialize");
        let storage_entry = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform_entry = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let combined_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("vp_bench_combined"),
            entries: &[storage_entry(0), uniform_entry(1), storage_entry(2)],
        });
        let mesh_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("vp_bench_mesh"),
            entries: &[storage_entry(0), uniform_entry(1)],
        });
        let light_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("vp_bench_light"),
            entries: &[storage_entry(0)],
        });
        let light_data = (0..PADDED_LIGHT_WORDS).map(|index| index as u32).collect();

        Self {
            device,
            queue,
            combined_layout,
            mesh_layout,
            light_layout,
            light_data,
        }
    }

    fn create_light_resource(&self, copy_dst: bool) -> (Buffer, BindGroup) {
        let mut usage = BufferUsages::STORAGE;
        if copy_dst {
            usage |= BufferUsages::COPY_DST;
        }
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vp_bench_light"),
                contents: bytemuck::cast_slice(&self.light_data),
                usage,
            });
        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("vp_bench_light"),
            layout: &self.light_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(buffer.as_entire_buffer_binding()),
            }],
        });
        (buffer, bind_group)
    }
}

struct ChunkMeshResources {
    light: Buffer,
    layers: Vec<(Buffer, Buffer)>,
}

fn mesh_resources(gpu: &GpuContext, chunk_count: usize) -> Vec<ChunkMeshResources> {
    (0..chunk_count)
        .map(|_| {
            let (light, _) = gpu.create_light_resource(true);
            let layers = (0..3)
                .map(|_| {
                    let descriptors = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("vp_bench_descriptors"),
                        size: 16,
                        usage: BufferUsages::STORAGE,
                        mapped_at_creation: false,
                    });
                    let origin = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("vp_bench_origin"),
                        size: 16,
                        usage: BufferUsages::UNIFORM,
                        mapped_at_creation: false,
                    });
                    (descriptors, origin)
                })
                .collect();
            ChunkMeshResources { light, layers }
        })
        .collect()
}

fn create_combined_groups(gpu: &GpuContext, resources: &[ChunkMeshResources]) -> Vec<BindGroup> {
    resources
        .iter()
        .flat_map(|chunk| {
            chunk.layers.iter().map(|(descriptors, origin)| {
                gpu.device.create_bind_group(&BindGroupDescriptor {
                    label: Some("vp_bench_combined"),
                    layout: &gpu.combined_layout,
                    entries: &[
                        BindGroupEntry {
                            binding: 0,
                            resource: descriptors.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: origin.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: chunk.light.as_entire_binding(),
                        },
                    ],
                })
            })
        })
        .collect()
}

fn create_split_groups(gpu: &GpuContext, resources: &[ChunkMeshResources]) -> Vec<BindGroup> {
    resources
        .iter()
        .flat_map(|chunk| {
            let light = gpu.device.create_bind_group(&BindGroupDescriptor {
                label: Some("vp_bench_split_light"),
                layout: &gpu.light_layout,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: chunk.light.as_entire_binding(),
                }],
            });
            std::iter::once(light).chain(chunk.layers.iter().map(|(descriptors, origin)| {
                gpu.device.create_bind_group(&BindGroupDescriptor {
                    label: Some("vp_bench_split_mesh"),
                    layout: &gpu.mesh_layout,
                    entries: &[
                        BindGroupEntry {
                            binding: 0,
                            resource: descriptors.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: origin.as_entire_binding(),
                        },
                    ],
                })
            }))
        })
        .collect()
}

fn bench_mesh_bind_group_creation(c: &mut Criterion) {
    let gpu = GpuContext::new();
    let mut group = c.benchmark_group("vp_gpu_mesh_bind_group_creation");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(30);

    for chunk_count in [16usize, 64, 256] {
        let resources = mesh_resources(&gpu, chunk_count);
        group.throughput(Throughput::Elements((chunk_count * 3) as u64));
        group.bench_function(BenchmarkId::new("split", chunk_count), |b| {
            b.iter(|| black_box(create_split_groups(&gpu, &resources)));
        });
        group.bench_function(BenchmarkId::new("combined", chunk_count), |b| {
            b.iter(|| black_box(create_combined_groups(&gpu, &resources)));
        });
    }

    group.finish();
}

fn bench_light_only_updates(c: &mut Criterion) {
    let gpu = GpuContext::new();
    let mut group = c.benchmark_group("vp_gpu_light_only_update");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(30);

    for chunk_count in [16usize, 64, 256] {
        group.throughput(Throughput::Elements(chunk_count as u64));

        group.bench_function(
            BenchmarkId::new("split_recreate_buffer_and_group", chunk_count),
            |b| {
                b.iter(|| {
                    let resources = (0..chunk_count)
                        .map(|_| gpu.create_light_resource(false))
                        .collect::<Vec<_>>();
                    gpu.queue.submit([]);
                    gpu.device.poll(PollType::Poll).unwrap();
                    black_box(resources);
                });
            },
        );

        let resources = (0..chunk_count)
            .map(|_| gpu.create_light_resource(true))
            .collect::<Vec<_>>();
        group.bench_function(
            BenchmarkId::new("combined_write_buffer", chunk_count),
            |b| {
                b.iter(|| {
                    for (buffer, _) in &resources {
                        gpu.queue
                            .write_buffer(buffer, 0, bytemuck::cast_slice(&gpu.light_data));
                    }
                    gpu.queue.submit([]);
                    gpu.device.poll(PollType::Poll).unwrap();
                    black_box(&resources);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_light_only_updates,
    bench_mesh_bind_group_creation
);
criterion_main!(benches);
