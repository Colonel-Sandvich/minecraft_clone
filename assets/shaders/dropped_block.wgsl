#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_functions,
    mesh_view_bindings::view,
    pbr_types::{
        STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT,
        PbrInput,
        pbr_input_new,
    },
    pbr_functions as pbr,
    pbr_bindings,
}
#import bevy_core_pipeline::tonemapping::tone_mapping

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var terrain_texture: texture_2d_array<f32>;

@group(#{MATERIAL_BIND_GROUP}) @binding(1)
var terrain_sampler: sampler;

// Indexed by terrain render ID * 6 + canonical local face direction.
@group(#{MATERIAL_BIND_GROUP}) @binding(2)
var<storage, read> texture_layers: array<u32>;

@group(#{MATERIAL_BIND_GROUP}) @binding(3)
var<storage, read> tint_colors: array<vec4<f32>>;

// x = alpha cutoff (zero disables cutout)
@group(#{MATERIAL_BIND_GROUP}) @binding(4)
var<uniform> settings: vec4<f32>;

@group(#{MATERIAL_BIND_GROUP}) @binding(5)
var<storage, read> emission_factors: array<f32>;

@fragment
fn fragment(
    @builtin(front_facing) is_front: bool,
    mesh: VertexOutput,
) -> @location(0) vec4<f32> {
    let render_id = mesh_functions::get_tag(mesh.instance_index);
    var face = 3u;
#ifdef VERTEX_COLORS
    face = u32(round(mesh.color.r));
#endif
    let lookup = render_id * 6u + face;
    let texture_layer = i32(texture_layers[lookup] & 0x00ffffffu);

    var pbr_input: PbrInput = pbr_input_new();
    pbr_input.material.base_color = textureSample(
        terrain_texture,
        terrain_sampler,
        mesh.uv,
        texture_layer,
    ) * tint_colors[lookup];
    if settings.x > 0.0 && pbr_input.material.base_color.a < settings.x {
        discard;
    }
    let emission = emission_factors[lookup];
    pbr_input.material.emissive = vec4<f32>(
        pbr_input.material.base_color.rgb * emission,
        1.0,
    );

    let double_sided =
        (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
    pbr_input.frag_coord = mesh.position;
    pbr_input.world_position = mesh.world_position;
    pbr_input.world_normal = pbr::prepare_world_normal(
        mesh.world_normal,
        double_sided,
        is_front,
    );
    pbr_input.is_orthographic = view.clip_from_view[3].w == 1.0;
    pbr_input.N = normalize(pbr_input.world_normal);

#ifdef VERTEX_TANGENTS
    let normal_map = textureSampleBias(
        pbr_bindings::normal_map_texture,
        pbr_bindings::normal_map_sampler,
        mesh.uv,
        view.mip_bias,
    ).rgb;
    let tangent_basis = pbr::calculate_tbn_mikktspace(
        mesh.world_normal,
        mesh.world_tangent,
    );
    pbr_input.N = pbr::apply_normal_mapping(
        pbr_input.material.flags,
        tangent_basis,
        double_sided,
        is_front,
        normal_map,
    );
#endif

    pbr_input.V = pbr::calculate_view(mesh.world_position, pbr_input.is_orthographic);

    return tone_mapping(pbr::apply_pbr_lighting(pbr_input), view.color_grading);
}
