use std::borrow::Cow;

use bevy::{
    prelude::*,
    render::{render_resource::*, renderer::RenderDevice},
};

use crate::block::RENDER_ID_COUNT;

use super::super::{
    super::DIRECTION_COUNT,
    visuals::{TerrainVisualSettings, TerrainVisualSettingsUniform},
};

const SHADER_PATH: &str = "shaders/vertex_pulling.wgsl";

#[derive(Resource)]
pub(super) struct Pipeline {
    pub(super) chunk_bind_group_layout: BindGroupLayout,
    pub(super) global_bind_group_layout: BindGroupLayout,
    pub(super) opaque_id: CachedRenderPipelineId,
    pub(super) cutout_id: CachedRenderPipelineId,
    pub(super) translucent_id: CachedRenderPipelineId,
}

#[derive(Resource)]
pub(super) struct Globals {
    pub(super) bind_group: BindGroup,
    pub(super) frame_buffers: Option<FrameBuffers>,
    pub(super) material_state_dirty: bool,
}

pub(super) struct FrameBuffers {
    pub(super) view_projection: Buffer,
    pub(super) visual_settings: Buffer,
}

pub(super) fn initialize(render_app: &mut bevy::app::SubApp) {
    let render_device = render_app.world().resource::<RenderDevice>().clone();
    let shader: Handle<Shader> = render_app
        .world_mut()
        .resource_mut::<AssetServer>()
        .load(SHADER_PATH);

    let global_entries = global_bind_group_entries();
    let global_layout = render_device.create_bind_group_layout("vp_g0_globals", &global_entries);
    let chunk_entries = chunk_bind_group_entries();
    let chunk_layout = render_device.create_bind_group_layout("vp_g1_chunk", &chunk_entries);

    let global_descriptor = BindGroupLayoutDescriptor::new("vp_g0_globals", &global_entries);
    let chunk_descriptor = BindGroupLayoutDescriptor::new("vp_g1_chunk", &chunk_entries);
    let pipeline_cache = render_app.world().resource::<PipelineCache>();

    let opaque_id = pipeline_cache.queue_render_pipeline(pipeline_descriptor(
        "vp_opaque",
        shader.clone(),
        global_descriptor.clone(),
        chunk_descriptor.clone(),
        Some(Face::Back),
        false,
        None,
        true,
    ));
    let cutout_id = pipeline_cache.queue_render_pipeline(pipeline_descriptor(
        "vp_cutout",
        shader.clone(),
        global_descriptor.clone(),
        chunk_descriptor.clone(),
        Some(Face::Back),
        true,
        None,
        true,
    ));
    let translucent_id = pipeline_cache.queue_render_pipeline(pipeline_descriptor(
        "vp_translucent",
        shader,
        global_descriptor,
        chunk_descriptor,
        None,
        false,
        Some(BlendState::ALPHA_BLENDING),
        false,
    ));

    let placeholder_bind_group =
        create_placeholder_global_bind_group(&render_device, &global_layout);
    render_app.world_mut().insert_resource(Pipeline {
        chunk_bind_group_layout: chunk_layout,
        global_bind_group_layout: global_layout,
        opaque_id,
        cutout_id,
        translucent_id,
    });
    render_app.world_mut().insert_resource(Globals {
        bind_group: placeholder_bind_group,
        frame_buffers: None,
        material_state_dirty: true,
    });
}

fn global_bind_group_entries() -> Vec<BindGroupLayoutEntry> {
    vec![
        uniform_entry(0, ShaderStages::VERTEX, 64),
        texture_2d_array_entry(1, ShaderStages::FRAGMENT),
        filtering_sampler_entry(2, ShaderStages::FRAGMENT),
        read_only_storage_entry(4, ShaderStages::FRAGMENT),
        read_only_storage_entry(5, ShaderStages::FRAGMENT),
        uniform_entry(6, ShaderStages::FRAGMENT, 16),
        read_only_storage_entry(7, ShaderStages::FRAGMENT),
        uniform_entry(
            8,
            ShaderStages::FRAGMENT,
            std::mem::size_of::<TerrainVisualSettingsUniform>() as u64,
        ),
    ]
}

fn chunk_bind_group_entries() -> Vec<BindGroupLayoutEntry> {
    vec![
        read_only_storage_entry(0, ShaderStages::VERTEX),
        uniform_entry(1, ShaderStages::VERTEX, 16),
        read_only_storage_entry(2, ShaderStages::VERTEX),
    ]
}

fn uniform_entry(
    binding: u32,
    visibility: ShaderStages,
    min_binding_size: u64,
) -> BindGroupLayoutEntry {
    buffer_entry(
        binding,
        visibility,
        BufferBindingType::Uniform,
        BufferSize::new(min_binding_size),
    )
}

fn read_only_storage_entry(binding: u32, visibility: ShaderStages) -> BindGroupLayoutEntry {
    buffer_entry(
        binding,
        visibility,
        BufferBindingType::Storage { read_only: true },
        None,
    )
}

fn buffer_entry(
    binding: u32,
    visibility: ShaderStages,
    ty: BufferBindingType,
    min_binding_size: Option<BufferSize>,
) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility,
        ty: BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size,
        },
        count: None,
    }
}

fn texture_2d_array_entry(binding: u32, visibility: ShaderStages) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility,
        ty: BindingType::Texture {
            sample_type: TextureSampleType::Float { filterable: true },
            view_dimension: TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn filtering_sampler_entry(binding: u32, visibility: ShaderStages) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility,
        ty: BindingType::Sampler(SamplerBindingType::Filtering),
        count: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn pipeline_descriptor(
    label: &'static str,
    shader: Handle<Shader>,
    global_layout: BindGroupLayoutDescriptor,
    chunk_layout: BindGroupLayoutDescriptor,
    cull_mode: Option<Face>,
    alpha_to_coverage_enabled: bool,
    blend: Option<BlendState>,
    depth_write_enabled: bool,
) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some(Cow::Borrowed(label)),
        layout: vec![global_layout, chunk_layout],
        immediate_size: 0,
        vertex: VertexState {
            shader: shader.clone(),
            shader_defs: vec![],
            entry_point: Some(Cow::Borrowed("vertex")),
            buffers: vec![],
        },
        primitive: PrimitiveState {
            topology: PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: FrontFace::Ccw,
            cull_mode,
            unclipped_depth: false,
            polygon_mode: PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(DepthStencilState {
            format: TextureFormat::Depth32Float,
            depth_write_enabled: Some(depth_write_enabled),
            depth_compare: Some(CompareFunction::GreaterEqual),
            stencil: StencilState::default(),
            bias: DepthBiasState::default(),
        }),
        multisample: MultisampleState {
            count: 4,
            mask: !0,
            alpha_to_coverage_enabled,
        },
        fragment: Some(FragmentState {
            shader,
            shader_defs: vec![],
            entry_point: Some(Cow::Borrowed("fragment")),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8UnormSrgb,
                blend,
                write_mask: ColorWrites::ALL,
            })],
        }),
        zero_initialize_workgroup_memory: false,
    }
}

fn create_placeholder_global_bind_group(
    render_device: &RenderDevice,
    layout: &BindGroupLayout,
) -> BindGroup {
    let view_projection = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_g0_dummy_view_proj"),
        contents: &[0u8; 64],
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    let texture = render_device.create_texture(&TextureDescriptor {
        label: Some("vp_g0_dummy_tex"),
        size: Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let texture_view = texture.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        array_layer_count: Some(1),
        ..Default::default()
    });
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let texture_layers = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_g0_dummy_texture_layers"),
        contents: bytemuck::cast_slice(&vec![0u32; RENDER_ID_COUNT * DIRECTION_COUNT]),
        usage: BufferUsages::STORAGE,
    });
    let tints = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_g0_dummy_tints"),
        contents: bytemuck::cast_slice(&vec![0.0f32; RENDER_ID_COUNT * DIRECTION_COUNT * 4]),
        usage: BufferUsages::STORAGE,
    });
    let ambient_occlusion = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_g0_dummy_ao"),
        contents: bytemuck::cast_slice(&[1.0f32; 4]),
        usage: BufferUsages::UNIFORM,
    });
    let emissions = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_g0_dummy_emissions"),
        contents: bytemuck::cast_slice(&vec![0.0f32; RENDER_ID_COUNT * DIRECTION_COUNT]),
        usage: BufferUsages::STORAGE,
    });
    let visual_settings =
        TerrainVisualSettingsUniform::new(TerrainVisualSettings::default(), Vec3::ZERO, 0.0);
    let visual_settings = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_g0_dummy_visual_settings"),
        contents: bytemuck::bytes_of(&visual_settings),
        usage: BufferUsages::UNIFORM,
    });

    render_device.create_bind_group(
        "vp_g0_dummy",
        layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: view_projection.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(&texture_view),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(&sampler),
            },
            BindGroupEntry {
                binding: 4,
                resource: texture_layers.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 5,
                resource: tints.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 6,
                resource: ambient_occlusion.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 7,
                resource: emissions.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 8,
                resource: visual_settings.as_entire_binding(),
            },
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_origin_binding_has_vec4_size() {
        let entry = &chunk_bind_group_entries()[1];
        let BindingType::Buffer {
            min_binding_size, ..
        } = entry.ty
        else {
            panic!("chunk origin should be a buffer binding");
        };
        assert_eq!(min_binding_size.unwrap().get(), 16);
    }

    #[test]
    fn visual_settings_binding_matches_uniform_size() {
        let entry = &global_bind_group_entries()[7];
        let BindingType::Buffer {
            min_binding_size, ..
        } = entry.ty
        else {
            panic!("visual settings should be a buffer binding");
        };
        assert_eq!(
            min_binding_size.unwrap().get(),
            std::mem::size_of::<TerrainVisualSettingsUniform>() as u64
        );
    }
}
