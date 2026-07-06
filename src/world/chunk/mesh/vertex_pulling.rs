//! Vertex-pulling mesh generation and rendering — Phase 2.
//!
//! Non-greedy, texture-array terrain, per-vertex smooth lighting.
//! CPU emits an 8-byte `FaceDescriptor` per visible face. The vertex shader decodes
//! descriptors via `@builtin(vertex_index)` and samples the per-chunk light buffer.
//!
//! Bind group 0 (per frame): view_proj + terrain texture array + texture_layers
//! Bind group 1 (per chunk layer): face descriptor SSBO + chunk_origin + light_data

use std::{
    any::TypeId,
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::{
    asset::AssetId,
    camera::visibility::{self, VisibilityClass},
    core_pipeline::core_3d::{
        Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey, Transparent3d, TransparentSortingInfo3d,
    },
    ecs::{
        change_detection::Ref,
        component::Component,
        system::{
            SystemParamItem,
            lifetimeless::{Read, SRes},
        },
    },
    math::Mat4,
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        mesh::allocator::MeshSlabs,
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
            PhaseItemExtraIndex, RenderCommand, RenderCommandResult, SetItemPipeline,
            TrackedRenderPass, ViewBinnedRenderPhases, ViewSortedRenderPhases,
        },
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        sync_component::{SyncComponent, SyncComponentPlugin},
        sync_world::RenderEntity,
        texture::GpuImage,
        view::{ExtractedView, RenderVisibleEntities},
    },
};

use crate::{
    block::{BLOCK_FLAG_TRANSLUCENT, BlockMaterialLayer, RENDER_ID_COUNT, WATER_RENDER_ID},
    world::dimension::Dimension,
};

use super::super::{Chunk, ChunkCell, fluid_sim::world_to_chunk_local};
use super::{
    CHUNK_SIZE, ChunkMeshBlocks, DIRECTION_COUNT, DIRECTION_INDEX_OFFSETS, block_mesh_flags,
    face_ao_key_from_indices, material_layer_index_from_flags, padded_chunk_index,
    should_emit_face_from_flags, should_emit_translucent_face, water_below_pair,
    water_corner_heights, water_flow_code,
};

const WATER_GEOMETRY_BIT: u32 = 1 << 5;

// ---------------------------------------------------------------------------
// Face descriptor (8 bytes, GPU-visible via bytemuck)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FaceDescriptor {
    pub packed: u32,
    pub info: u32,
}

impl FaceDescriptor {
    #[inline]
    pub fn new(x: u32, y: u32, z: u32, face_dir: u32, block_type: u32, ao_key: u32) -> Self {
        Self {
            packed: (x << 27) | (z << 22) | (y << 17) | (face_dir << 14),
            info: block_type | (ao_key << 8),
        }
    }

    /// Pack 4 corner water heights (0-9 ninths) into the upper 16 bits of `info`.
    /// Layout: `h00:4 h10:4 h01:4 h11:4` where:
    ///   h00 = corner at (x+0, z+0)  h10 = corner at (x+1, z+0)
    ///   h01 = corner at (x+0, z+1)  h11 = corner at (x+1, z+1)
    /// Sets an explicit water-geometry bit because all four packed heights may
    /// legitimately be zero for very shallow water.
    #[inline]
    pub fn with_corner_heights(mut self, h00: u32, h10: u32, h01: u32, h11: u32) -> Self {
        self.packed |= WATER_GEOMETRY_BIT;
        self.info |= (h00 & 0xF) << 16 | (h10 & 0xF) << 20 | (h01 & 0xF) << 24 | (h11 & 0xF) << 28;
        self
    }

    /// Set water surface heights for the cell directly below (0-9 ninths).
    /// Two values because the bottom-left and bottom-right vertices of a side
    /// face need to extend to different lower-surface heights along the shared
    /// edge. Stored in `packed` bits 6-9 (lo) and 10-13 (hi).
    #[inline]
    pub fn with_water_below(mut self, lo: u32, hi: u32) -> Self {
        self.packed |= ((lo & 0xF) << 6) | ((hi & 0xF) << 10);
        self
    }

    /// Mark the UP face to use the flow texture and orient it by a quantized
    /// horizontal flow direction. Bit 0 means flowing; bits 1-4 store the code.
    #[inline]
    pub fn with_water_up_flow(mut self, flow_code: u32) -> Self {
        self.packed |= 1 | ((flow_code & 0xF) << 1);
        self
    }

    #[inline]
    pub fn x(self) -> u32 {
        (self.packed >> 27) & 0x1F
    }

    #[inline]
    pub fn y(self) -> u32 {
        (self.packed >> 17) & 0x1F
    }

    #[inline]
    pub fn z(self) -> u32 {
        (self.packed >> 22) & 0x1F
    }

    #[inline]
    pub fn face_dir(self) -> u32 {
        (self.packed >> 14) & 0x7
    }

    #[inline]
    pub fn block_type(self) -> u32 {
        self.info & 0xFF
    }

    #[inline]
    pub fn water_up_flowing(self) -> bool {
        self.packed & 1 != 0
    }

    #[inline]
    pub fn water_flow_code(self) -> u32 {
        (self.packed >> 1) & 0xF
    }

    #[inline]
    pub fn has_water_geometry(self) -> bool {
        self.packed & WATER_GEOMETRY_BIT != 0
    }
}

pub(super) struct WaterDescriptorData {
    packed_by_side: [u32; DIRECTION_COUNT],
    info: u32,
}

impl WaterDescriptorData {
    pub(super) fn from_cell(blocks: &ChunkMeshBlocks, padded_index: usize) -> Self {
        let level = blocks.get_fluid_level(padded_index);
        let (h00, h10, h01, h11) = water_corner_heights(level, blocks, padded_index);
        let corner_descriptor = FaceDescriptor::default().with_corner_heights(h00, h10, h01, h11);
        let mut packed_by_side = [corner_descriptor.packed; DIRECTION_COUNT];

        let below_idx = (padded_index as isize + DIRECTION_INDEX_OFFSETS[2]) as usize;
        let below = unsafe { *blocks.blocks.get_unchecked(below_idx) };
        if below == WATER_RENDER_ID {
            let below_level = blocks.get_fluid_level(below_idx);
            let (bh00, bh10, bh01, bh11) = water_corner_heights(below_level, blocks, below_idx);
            for (side_index, packed) in packed_by_side.iter_mut().enumerate() {
                let (lo, hi) = water_below_pair(side_index, bh00, bh10, bh01, bh11);
                *packed |= FaceDescriptor::default().with_water_below(lo, hi).packed;
            }
        }

        let flow_code = water_flow_code(level, blocks, padded_index);
        if flow_code != 0 || (h00 | h10 | h01 | h11) != 8 {
            packed_by_side[3] |= FaceDescriptor::default()
                .with_water_up_flow(flow_code)
                .packed;
        }

        Self {
            packed_by_side,
            info: corner_descriptor.info,
        }
    }

    #[inline(always)]
    pub(super) fn apply(&self, mut desc: FaceDescriptor, side_index: usize) -> FaceDescriptor {
        desc.packed |= self.packed_by_side[side_index];
        desc.info |= self.info;
        desc
    }
}

// ---------------------------------------------------------------------------
// Padded chunk blocks → non-greedy FaceDescriptors (per material layer)
// ---------------------------------------------------------------------------

pub fn build_descriptors(
    blocks: &ChunkMeshBlocks,
) -> Vec<(BlockMaterialLayer, Vec<FaceDescriptor>)> {
    if blocks.can_skip_mesh() {
        return Vec::new();
    }

    let capacity = blocks.center_rendered_blocks as usize;
    let mut descriptors: [Vec<FaceDescriptor>; BlockMaterialLayer::COUNT] =
        std::array::from_fn(|_| Vec::with_capacity(capacity));

    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let mut padded_index = padded_chunk_index(1, y + 1, z + 1);

            for x in 0..CHUNK_SIZE {
                let block = unsafe { *blocks.blocks.get_unchecked(padded_index) };
                let block_flags = block_mesh_flags(block);

                if block_flags != 0 {
                    let is_water = block == WATER_RENDER_ID;
                    let mut water_data = None;

                    for side_index in 0..DIRECTION_COUNT {
                        let neighbor_index =
                            (padded_index as isize + DIRECTION_INDEX_OFFSETS[side_index]) as usize;
                        let neighbor = unsafe { *blocks.blocks.get_unchecked(neighbor_index) };
                        let neighbor_flags = block_mesh_flags(neighbor);

                        let emit = if block_flags & BLOCK_FLAG_TRANSLUCENT != 0 {
                            should_emit_translucent_face(
                                block,
                                block_flags,
                                neighbor,
                                neighbor_flags,
                            )
                        } else {
                            should_emit_face_from_flags(
                                block,
                                block_flags,
                                neighbor,
                                neighbor_flags,
                            )
                        };

                        if emit {
                            let ao_key = face_ao_key_from_indices(blocks, padded_index, side_index);
                            let desc = FaceDescriptor::new(
                                x as u32,
                                y as u32,
                                z as u32,
                                side_index as u32,
                                block as u32,
                                ao_key,
                            );
                            descriptors[material_layer_index_from_flags(block_flags)].push(
                                if is_water {
                                    water_data
                                        .get_or_insert_with(|| {
                                            WaterDescriptorData::from_cell(blocks, padded_index)
                                        })
                                        .apply(desc, side_index)
                                } else {
                                    desc
                                },
                            );
                        }
                    }
                }

                padded_index += 1;
            }
        }
    }

    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            let mut layer_descriptors = std::mem::take(&mut descriptors[layer.index()]);
            layer_descriptors.shrink_to_fit();
            (!layer_descriptors.is_empty()).then_some((layer, layer_descriptors))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Main-world component
// ---------------------------------------------------------------------------

#[derive(Component, Clone)]
pub struct ChunkMeshDescriptors(pub Vec<FaceDescriptor>);

#[derive(Component, Clone)]
pub struct VertexPullingMesh {
    pub face_count: u32,
    pub material_layer: BlockMaterialLayer,
    pub chunk_origin: Vec3,
}

impl SyncComponent for VertexPullingMesh {
    type Target = VertexPullingMesh;
}

#[derive(Component, Clone)]
pub struct VertexPullingLight {
    pub light_data: Arc<[u32]>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SharedLightDataKey {
    ptr: usize,
    len: usize,
}

impl VertexPullingLight {
    pub fn data_key(&self) -> SharedLightDataKey {
        SharedLightDataKey {
            ptr: self.light_data.as_ptr() as usize,
            len: self.light_data.len(),
        }
    }
}

// ---------------------------------------------------------------------------
// Render-world resources
// ---------------------------------------------------------------------------

#[derive(Component)]
pub struct PreparedChunkVp {
    pub chunk_bind_group: BindGroup,
    pub face_count: u32,
    pub material_layer: BlockMaterialLayer,
    pub chunk_origin: Vec3,
    desc_buf: Buffer,
    light_buf: Buffer,
}

#[derive(Resource)]
pub struct VpGlobals {
    pub group0_bind_group: BindGroup,
}

#[derive(Resource, Default)]
pub struct VpStaticResources {
    pub texture_layers_buf: Option<Buffer>,
    pub tint_colors_buf: Option<Buffer>,
    pub emission_factors_buf: Option<Buffer>,
    pub ao_brightness_buf: Option<Buffer>,
    pub visual_settings_buf: Option<Buffer>,
    pub view_proj_buf: Option<Buffer>,
}

#[derive(Resource)]
pub struct VpPipeline {
    pub chunk_bind_group_layout: BindGroupLayout,
    pub group0_bind_group_layout: BindGroupLayout,
    pub opaque_id: CachedRenderPipelineId,
    pub cutout_id: CachedRenderPipelineId,
    pub translucent_id: CachedRenderPipelineId,
}

#[derive(Resource, Clone)]
pub struct VpTextureState {
    pub terrain_texture_handle: Handle<Image>,
    pub texture_layers: Vec<u32>,
    pub tint_colors: Vec<[f32; 4]>,
    pub emission_factors: Vec<f32>,
    pub ao_brightness: [f32; 4],
}

#[derive(Resource, Default, Clone, Copy)]
pub struct VpAnimationClock {
    pub seconds: f32,
}

impl ExtractResource for VpAnimationClock {
    type Source = VpAnimationClock;

    fn extract_resource(source: &Self::Source) -> Self {
        *source
    }
}

impl ExtractResource for VpTextureState {
    type Source = VpTextureState;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
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

fn vp_group0_entries() -> Vec<BindGroupLayoutEntry> {
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

fn vp_group1_entries() -> Vec<BindGroupLayoutEntry> {
    vec![
        read_only_storage_entry(0, ShaderStages::VERTEX),
        uniform_entry(1, ShaderStages::VERTEX, 16),
        read_only_storage_entry(2, ShaderStages::VERTEX),
    ]
}

fn vp_pipeline_descriptor(
    label: &'static str,
    shader: Handle<Shader>,
    group0_desc: BindGroupLayoutDescriptor,
    group1_desc: BindGroupLayoutDescriptor,
    cull_mode: Option<Face>,
    alpha_to_coverage_enabled: bool,
    blend: Option<BlendState>,
    depth_write_enabled: bool,
) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some(Cow::Borrowed(label)),
        layout: vec![group0_desc, group1_desc],
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

#[derive(Resource, Reflect, Debug, Clone, Copy)]
#[reflect(Resource)]
pub struct TerrainVisualSettings {
    pub sky_light_color: Vec3,
    pub block_light_color: Vec3,
    pub fog_color: Vec3,
    pub fog_start: f32,
    pub fog_end: f32,
    pub fog_strength: f32,
    pub screen_tint_strength: f32,
}

const MINECRAFT_WATER_FOG_START: f32 = -8.0;
const MINECRAFT_WATER_FOG_END: f32 = 96.0;
const MINECRAFT_UNDERWATER_OVERLAY_ALPHA: f32 = 0.1;

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

#[repr(C)]
#[derive(Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainVisualSettingsUniform {
    sky_light_color: [f32; 4],
    block_light_color: [f32; 4],
    fog_color: [f32; 4],
    camera_position: [f32; 4],
    fog_params: [f32; 4],
}

impl TerrainVisualSettingsUniform {
    fn new(settings: TerrainVisualSettings, camera_position: Vec3, animation_seconds: f32) -> Self {
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

// ---------------------------------------------------------------------------
// Chunk-origin uniform (16 bytes, for binding 1 in group 1)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ChunkOriginUniform {
    origin: [f32; 4], // Vec4 (xyz=origin, w=0)
}

// ---------------------------------------------------------------------------
// Draw command
// ---------------------------------------------------------------------------

pub struct DrawVertexPulled;

impl<P: PhaseItem> RenderCommand<P> for DrawVertexPulled {
    type Param = SRes<VpGlobals>;
    type ViewQuery = ();
    type ItemQuery = Read<PreparedChunkVp>;

    fn render<'w>(
        _item: &P,
        _view: (),
        data: Option<&'w PreparedChunkVp>,
        globals: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let globals = globals.into_inner();
        let Some(data) = data else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(0, &globals.group0_bind_group, &[]);
        pass.set_bind_group(1, &data.chunk_bind_group, &[]);
        pass.draw(0..data.face_count * 6, 0..1);
        RenderCommandResult::Success
    }
}

pub type DrawVpCmds = (SetItemPipeline, DrawVertexPulled);

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

const SHADER_PATH: &str = "shaders/vertex_pulling.wgsl";
pub const SHADER_SOURCE: &str = include_str!("../../../../assets/shaders/vertex_pulling.wgsl");

pub struct VertexPullingPlugin;

impl Plugin for VertexPullingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerrainVisualSettings>()
            .init_resource::<VpAnimationClock>()
            .register_type::<TerrainVisualSettings>();
        app.register_required_components::<VertexPullingMesh, Transform>()
            .register_required_components::<VertexPullingMesh, Visibility>()
            .register_required_components::<VertexPullingMesh, VisibilityClass>();
        app.world_mut()
            .register_component_hooks::<VertexPullingMesh>()
            .on_add(visibility::add_visibility_class::<VertexPullingMesh>);

        app.add_plugins((
            SyncComponentPlugin::<VertexPullingMesh>::default(),
            ExtractResourcePlugin::<VpTextureState>::default(),
            ExtractResourcePlugin::<TerrainVisualSettings>::default(),
            ExtractResourcePlugin::<VpAnimationClock>::default(),
        ));
        app.add_systems(
            Update,
            (update_vp_animation_clock, update_camera_fluid_visuals),
        );

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_systems(
                ExtractSchedule,
                (extract_changed_vp_data, extract_changed_vp_lights),
            )
            .init_resource::<VpStaticResources>()
            .add_systems(
                Render,
                (
                    prepare_gpu_data.in_set(RenderSystems::PrepareResources),
                    queue_vp_meshes.in_set(RenderSystems::Queue),
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app.add_render_command::<Opaque3d, DrawVpCmds>();
        render_app.add_render_command::<Transparent3d, DrawVpCmds>();

        let render_device = render_app.world().resource::<RenderDevice>().clone();

        let shader: Handle<Shader> = render_app
            .world_mut()
            .resource_mut::<AssetServer>()
            .load(SHADER_PATH);

        let group0_entries = vp_group0_entries();
        let group0_layout =
            render_device.create_bind_group_layout("vp_g0_globals", &group0_entries);

        let group1_entries = vp_group1_entries();
        let group1_layout = render_device.create_bind_group_layout("vp_g1_chunk", &group1_entries);

        let group0_desc = BindGroupLayoutDescriptor::new("vp_g0_globals", &group0_entries);
        let group1_desc = BindGroupLayoutDescriptor::new("vp_g1_chunk", &group1_entries);

        let pipeline_cache = render_app.world().resource::<PipelineCache>();

        let opaque_id = pipeline_cache.queue_render_pipeline(vp_pipeline_descriptor(
            "vp_opaque",
            shader.clone(),
            group0_desc.clone(),
            group1_desc.clone(),
            Some(Face::Back),
            false,
            None,
            true,
        ));

        let cutout_id = pipeline_cache.queue_render_pipeline(vp_pipeline_descriptor(
            "vp_cutout",
            shader.clone(),
            group0_desc.clone(),
            group1_desc.clone(),
            Some(Face::Back),
            true,
            None,
            true,
        ));

        let translucent_id = pipeline_cache.queue_render_pipeline(vp_pipeline_descriptor(
            "vp_translucent",
            shader,
            group0_desc,
            group1_desc,
            None,
            false,
            Some(BlendState::ALPHA_BLENDING),
            false,
        ));

        render_app.world_mut().insert_resource(VpPipeline {
            chunk_bind_group_layout: group1_layout,
            group0_bind_group_layout: group0_layout.clone(),
            opaque_id,
            cutout_id,
            translucent_id,
        });

        // Create placeholder group 0 bind group (replaced each frame)
        let dummy_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_view_proj"),
            contents: &[0u8; 64],
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let dummy_tex = render_device.create_texture(&TextureDescriptor {
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
        let dummy_tex_view = dummy_tex.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            array_layer_count: Some(1),
            ..Default::default()
        });
        let dummy_sampler = render_device.create_sampler(&SamplerDescriptor::default());
        let dummy_texture_layers = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_texture_layers"),
            contents: bytemuck::cast_slice(&vec![0u32; RENDER_ID_COUNT * DIRECTION_COUNT]),
            usage: BufferUsages::STORAGE,
        });
        let dummy_tints = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_tints"),
            contents: bytemuck::cast_slice(&vec![0.0f32; RENDER_ID_COUNT * DIRECTION_COUNT * 4]),
            usage: BufferUsages::STORAGE,
        });
        let dummy_ao = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_ao"),
            contents: bytemuck::cast_slice(&[1.0f32; 4]),
            usage: BufferUsages::UNIFORM,
        });
        let dummy_emissions = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_emissions"),
            contents: bytemuck::cast_slice(&vec![0.0f32; RENDER_ID_COUNT * DIRECTION_COUNT]),
            usage: BufferUsages::STORAGE,
        });
        let dummy_visual_settings =
            TerrainVisualSettingsUniform::new(TerrainVisualSettings::default(), Vec3::ZERO, 0.0);
        let dummy_visual_settings_buf =
            render_device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("vp_g0_dummy_visual_settings"),
                contents: bytemuck::bytes_of(&dummy_visual_settings),
                usage: BufferUsages::UNIFORM,
            });
        let dummy_group0 = render_device.create_bind_group(
            "vp_g0_dummy",
            &group0_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: dummy_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&dummy_tex_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&dummy_sampler),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: dummy_texture_layers.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: dummy_tints.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: dummy_ao.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: dummy_emissions.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 8,
                    resource: dummy_visual_settings_buf.as_entire_binding(),
                },
            ],
        );
        render_app.world_mut().insert_resource(VpGlobals {
            group0_bind_group: dummy_group0,
        });
    }
}

// ---------------------------------------------------------------------------
// Systems (module-level functions so `in_set` works)
// ---------------------------------------------------------------------------

fn update_vp_animation_clock(time: Res<Time>, mut clock: ResMut<VpAnimationClock>) {
    clock.seconds = time.elapsed_secs_wrapped();
}

fn update_camera_fluid_visuals(
    mut settings: ResMut<TerrainVisualSettings>,
    mut clear_color: ResMut<ClearColor>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    dimensions: Query<&Dimension>,
    chunks: Query<&Chunk>,
) {
    let underwater = cameras.iter().next().is_some_and(|camera| {
        let Some(dimension) = dimensions.iter().next() else {
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
    let (chunk_pos, local) = world_to_chunk_local(world_pos);
    chunks
        .get(dimension.chunk_entity(chunk_pos)?)
        .ok()
        .map(|chunk| chunk.get_cell(local))
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

fn extract_changed_vp_data(
    mut commands: Commands,
    meshes: Extract<Query<(RenderEntity, &VertexPullingMesh), Changed<VertexPullingMesh>>>,
    descriptors: Extract<
        Query<(RenderEntity, &ChunkMeshDescriptors), Changed<ChunkMeshDescriptors>>,
    >,
) {
    commands.try_insert_batch(
        meshes
            .iter()
            .map(|(entity, mesh)| (entity, mesh.clone()))
            .collect::<Vec<_>>(),
    );
    for (entity, desc) in &descriptors {
        commands.entity(entity).insert(desc.clone());
    }
}

fn extract_changed_vp_lights(
    mut commands: Commands,
    lights: Extract<Query<(RenderEntity, &VertexPullingLight), Changed<VertexPullingLight>>>,
) {
    commands.try_insert_batch(
        lights
            .iter()
            .map(|(entity, light)| (entity, light.clone()))
            .collect::<Vec<_>>(),
    );
}

fn prepare_gpu_data(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline: Option<Res<VpPipeline>>,
    texture_state: Option<Res<VpTextureState>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    chunks_q: Query<
        (Entity, &VertexPullingMesh, Option<Ref<VertexPullingLight>>),
        Changed<VertexPullingMesh>,
    >,
    descriptors_q: Query<&ChunkMeshDescriptors>,
    lights_q: Query<(Entity, Ref<VertexPullingLight>), Changed<VertexPullingLight>>,
    all_meshes_q: Query<&VertexPullingMesh>,
    prepared_q: Query<&PreparedChunkVp>,
    cameras_q: Query<&ExtractedView>,
    visual_settings: Option<Res<TerrainVisualSettings>>,
    animation_clock: Option<Res<VpAnimationClock>>,
    mut globals: ResMut<VpGlobals>,
    mut static_res: ResMut<VpStaticResources>,
) {
    let Some(pipeline) = pipeline else { return };

    let view = cameras_q
        .iter()
        .find(|v| v.clip_from_view.col(2).w.abs() > 0.5)
        .or_else(|| cameras_q.iter().next());
    let view_proj = view
        .map(|v| {
            v.clip_from_world.unwrap_or_else(|| {
                let view_from_world = v.world_from_view.affine().inverse();
                v.clip_from_view * Mat4::from(view_from_world)
            })
        })
        .unwrap_or(Mat4::IDENTITY);
    let camera_position = view
        .map(|v| v.world_from_view.translation())
        .unwrap_or(Vec3::ZERO);
    let visual_settings = visual_settings
        .map(|settings| *settings)
        .unwrap_or_default();
    let animation_seconds = animation_clock
        .map(|clock| clock.seconds)
        .unwrap_or_default();
    let visual_settings_uniform =
        TerrainVisualSettingsUniform::new(visual_settings, camera_position, animation_seconds);

    let Some(texture_state) = texture_state.as_deref() else {
        return;
    };
    let Some(gpu_image) = gpu_images.get(&texture_state.terrain_texture_handle) else {
        return;
    };

    // Create static buffers once and keep them forever
    if static_res.view_proj_buf.is_none() {
        static_res.texture_layers_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_texture_layers"),
                contents: bytemuck::cast_slice(&texture_state.texture_layers),
                usage: BufferUsages::STORAGE,
            },
        ));

        let tints_flat: Vec<f32> = texture_state
            .tint_colors
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        static_res.tint_colors_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_tint_colors"),
                contents: bytemuck::cast_slice(&tints_flat),
                usage: BufferUsages::STORAGE,
            },
        ));

        static_res.emission_factors_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_emission_factors"),
                contents: bytemuck::cast_slice(&texture_state.emission_factors),
                usage: BufferUsages::STORAGE,
            },
        ));

        static_res.view_proj_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_view_proj"),
                contents: bytemuck::cast_slice(view_proj.as_ref()),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            },
        ));

        static_res.ao_brightness_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_ao_brightness"),
                contents: bytemuck::cast_slice(&texture_state.ao_brightness),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            },
        ));

        static_res.visual_settings_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_visual_settings"),
                contents: bytemuck::bytes_of(&visual_settings_uniform),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            },
        ));

        let group0_entries: [BindGroupEntry; 8] = [
            BindGroupEntry {
                binding: 0,
                resource: static_res
                    .view_proj_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(&gpu_image.texture_view),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(&gpu_image.sampler),
            },
            BindGroupEntry {
                binding: 4,
                resource: static_res
                    .texture_layers_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
            BindGroupEntry {
                binding: 5,
                resource: static_res
                    .tint_colors_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
            BindGroupEntry {
                binding: 6,
                resource: static_res
                    .ao_brightness_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
            BindGroupEntry {
                binding: 7,
                resource: static_res
                    .emission_factors_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
            BindGroupEntry {
                binding: 8,
                resource: static_res
                    .visual_settings_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
        ];

        globals.group0_bind_group = render_device.create_bind_group(
            "vp_g0_globals",
            &pipeline.group0_bind_group_layout,
            &group0_entries,
        );
    } else {
        // Update view_proj buffer in-place (same buffer, same bind group)
        render_queue.0.write_buffer(
            static_res.view_proj_buf.as_ref().unwrap(),
            0,
            bytemuck::cast_slice(view_proj.as_ref()),
        );
    }

    render_queue.0.write_buffer(
        static_res.ao_brightness_buf.as_ref().unwrap(),
        0,
        bytemuck::cast_slice(&texture_state.ao_brightness),
    );
    render_queue.0.write_buffer(
        static_res.visual_settings_buf.as_ref().unwrap(),
        0,
        bytemuck::bytes_of(&visual_settings_uniform),
    );

    let mut mesh_prepared = HashSet::new();
    let mut light_buffers_by_data = HashMap::new();
    let mut updated_light_buffers = HashSet::new();

    // Mesh changes require a new combined bind group, but can retain and update the fixed-size
    // light buffer in place.
    for (entity, mesh, light) in &chunks_q {
        mesh_prepared.insert(entity);

        if mesh.face_count == 0 {
            commands.entity(entity).remove::<PreparedChunkVp>();
            continue;
        }

        let desc_buf = if let Ok(desc) = descriptors_q.get(entity) {
            create_descriptor_buffer_from_slice(&render_device, &desc.0)
        } else if let Ok(prepared) = prepared_q.get(entity) {
            prepared.desc_buf.clone()
        } else {
            continue;
        };
        let origin_buf = create_origin_buffer(&render_device, mesh);
        let prepared = prepared_q.get(entity).ok();
        let light_buf = if let Some(light) = light.as_ref() {
            let (buffer, created) = light_buffer_for(
                &render_device,
                &mut light_buffers_by_data,
                light,
                prepared.map(|prepared| &prepared.light_buf),
            );
            if light.is_changed() && !created {
                write_light_buffer_once(&render_queue, &mut updated_light_buffers, &buffer, light);
            }
            buffer
        } else if let Some(prepared) = prepared {
            prepared.light_buf.clone()
        } else {
            // A mesh cannot be rendered until its synchronized light component arrives.
            continue;
        };
        let chunk_bind_group = create_chunk_bind_group(
            &render_device,
            &pipeline,
            &desc_buf,
            &origin_buf,
            &light_buf,
        );

        commands.entity(entity).remove::<ChunkMeshDescriptors>();
        commands.entity(entity).insert(PreparedChunkVp {
            chunk_bind_group,
            face_count: mesh.face_count,
            material_layer: mesh.material_layer,
            chunk_origin: mesh.chunk_origin,
            desc_buf,
            light_buf,
        });
    }

    // Light-only changes write into the existing fixed-size buffer. The combined bind group keeps
    // referencing the same buffer, so neither it nor descriptor/origin buffers need replacement.
    for (entity, light) in &lights_q {
        if mesh_prepared.contains(&entity) {
            continue;
        }

        if let Ok(prepared) = prepared_q.get(entity) {
            write_light_buffer_once(
                &render_queue,
                &mut updated_light_buffers,
                &prepared.light_buf,
                &light,
            );
            continue;
        }

        let Ok(desc) = descriptors_q.get(entity) else {
            continue;
        };
        let Ok(mesh) = all_meshes_q.get(entity) else {
            continue;
        };
        if mesh.face_count == 0 {
            commands.entity(entity).remove::<PreparedChunkVp>();
            continue;
        }

        let desc_buf = create_descriptor_buffer_from_slice(&render_device, &desc.0);
        let origin_buf = create_origin_buffer(&render_device, mesh);
        let (light_buf, _) =
            light_buffer_for(&render_device, &mut light_buffers_by_data, &light, None);
        let chunk_bind_group = create_chunk_bind_group(
            &render_device,
            &pipeline,
            &desc_buf,
            &origin_buf,
            &light_buf,
        );

        commands.entity(entity).remove::<ChunkMeshDescriptors>();
        commands.entity(entity).insert(PreparedChunkVp {
            chunk_bind_group,
            face_count: mesh.face_count,
            material_layer: mesh.material_layer,
            chunk_origin: mesh.chunk_origin,
            desc_buf,
            light_buf,
        });
    }
}

fn create_descriptor_buffer_from_slice(
    render_device: &RenderDevice,
    descriptors: &[FaceDescriptor],
) -> Buffer {
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_desc"),
        contents: bytemuck::cast_slice(descriptors),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    })
}

fn create_origin_buffer(render_device: &RenderDevice, mesh: &VertexPullingMesh) -> Buffer {
    let origin_uniform = ChunkOriginUniform {
        origin: [
            mesh.chunk_origin.x,
            mesh.chunk_origin.y,
            mesh.chunk_origin.z,
            0.0,
        ],
    };
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_origin"),
        contents: bytemuck::bytes_of(&origin_uniform),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    })
}

fn create_light_buffer(render_device: &RenderDevice, light: &VertexPullingLight) -> Buffer {
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_light"),
        contents: bytemuck::cast_slice(light.light_data.as_ref()),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    })
}

fn light_buffer_for(
    render_device: &RenderDevice,
    light_buffers_by_data: &mut HashMap<SharedLightDataKey, Buffer>,
    light: &VertexPullingLight,
    reusable_existing: Option<&Buffer>,
) -> (Buffer, bool) {
    let key = light.data_key();
    if let Some(buffer) = light_buffers_by_data.get(&key) {
        return (buffer.clone(), false);
    }

    let (buffer, created) = reusable_existing
        .map(|buffer| (buffer.clone(), false))
        .unwrap_or_else(|| (create_light_buffer(render_device, light), true));
    light_buffers_by_data.insert(key, buffer.clone());
    (buffer, created)
}

fn write_light_buffer_once(
    render_queue: &RenderQueue,
    updated_light_buffers: &mut HashSet<BufferId>,
    buffer: &Buffer,
    light: &VertexPullingLight,
) {
    if updated_light_buffers.insert(buffer.id()) {
        let bytes = bytemuck::cast_slice(light.light_data.as_ref());
        debug_assert_eq!(buffer.size(), bytes.len() as u64);
        render_queue.0.write_buffer(buffer, 0, bytes);
    }
}

fn create_chunk_bind_group(
    render_device: &RenderDevice,
    pipeline: &VpPipeline,
    desc_buf: &Buffer,
    origin_buf: &Buffer,
    light_buf: &Buffer,
) -> BindGroup {
    render_device.create_bind_group(
        "vp_chunk",
        &pipeline.chunk_bind_group_layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: desc_buf.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: origin_buf.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: light_buf.as_entire_binding(),
            },
        ],
    )
}

fn queue_vp_meshes(
    pipeline: Option<Res<VpPipeline>>,
    mut opaque_phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    mut transparent_phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<(&ExtractedView, &RenderVisibleEntities)>,
    prepared_chunks: Query<&PreparedChunkVp>,
    opaque_draw_fns: Res<DrawFunctions<Opaque3d>>,
    transparent_draw_fns: Res<DrawFunctions<Transparent3d>>,
) {
    let Some(pipeline) = pipeline.as_deref() else {
        return;
    };
    if prepared_chunks.is_empty() {
        return;
    }

    let opaque_draw_fn = opaque_draw_fns.read().id::<DrawVpCmds>();
    let transparent_draw_fn = transparent_draw_fns.read().id::<DrawVpCmds>();
    let opaque_batch_set_key = Opaque3dBatchSetKey {
        pipeline: pipeline.opaque_id,
        draw_function: opaque_draw_fn,
        material_bind_group_index: None,
        slabs: MeshSlabs::default(),
        lightmap_slab: None,
    };
    let cutout_batch_set_key = Opaque3dBatchSetKey {
        pipeline: pipeline.cutout_id,
        ..opaque_batch_set_key.clone()
    };
    let bin_key = Opaque3dBinKey {
        asset_id: AssetId::<Mesh>::invalid().untyped(),
    };

    for (view, visible_entities) in &views {
        let Some(opaque_phase) = opaque_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(transparent_phase) = transparent_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let rangefinder = view.rangefinder3d();
        let Some(visible_class) = visible_entities
            .classes
            .get(&TypeId::of::<VertexPullingMesh>())
        else {
            continue;
        };
        for &(entity, main_entity) in &visible_class.entities_cpu_culling {
            let Ok(data) = prepared_chunks.get(entity) else {
                continue;
            };
            match data.material_layer {
                BlockMaterialLayer::Opaque => opaque_phase.add(
                    opaque_batch_set_key.clone(),
                    bin_key.clone(),
                    (entity, main_entity),
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                ),
                BlockMaterialLayer::Cutout => opaque_phase.add(
                    cutout_batch_set_key.clone(),
                    bin_key.clone(),
                    (entity, main_entity),
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                ),
                BlockMaterialLayer::Translucent => {
                    let chunk_center = data.chunk_origin + Vec3::splat(CHUNK_SIZE as f32 * 0.5);
                    transparent_phase.add_retained(Transparent3d {
                        sorting_info: TransparentSortingInfo3d::Sorted {
                            mesh_center: chunk_center,
                            depth_bias: 0.0,
                        },
                        entity: (entity, main_entity),
                        pipeline: pipeline.translucent_id,
                        draw_function: transparent_draw_fn,
                        distance: rangefinder.distance(&chunk_center),
                        batch_range: 0..1,
                        extra_index: PhaseItemExtraIndex::None,
                        indexed: false,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
