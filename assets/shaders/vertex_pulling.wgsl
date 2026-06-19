// Vertex-pulling shader — texture-array terrain, AO, smooth lighting.
//
// Bind group 0 (per frame, global):
//   binding 0: view_proj uniform (mat4x4<f32>)
//   binding 1: terrain_texture (texture_2d_array<f32>)
//   binding 2: terrain_sampler (sampler)
//   binding 4: texture_layers storage (array<u32>)       // (block_type * 6 + face_dir)
//   binding 5: tint_colors storage (array<vec4<f32>>)    // (block_type * 6 + face_dir)
//   binding 6: ao_brightness uniform (vec4<f32>)
//   binding 7: emission_factors storage (array<f32>)      // (block_type * 6 + face_dir)
//   binding 8: terrain_visual_settings uniform
// Bind group 1 (per chunk):
//   binding 0: faces storage (array<FaceDescriptor>)
//   binding 1: chunk_origin uniform (vec4<f32>)
//   binding 2: light_data storage (array<u32>) // padded 18³, 4 packed light cells per u32

struct FaceDescriptor {
    packed: u32,
    info: u32,
}

struct TerrainVisualSettings {
    sky_light_color: vec4<f32>,
    block_light_color: vec4<f32>,
    fog_color: vec4<f32>,
    camera_position: vec4<f32>,
    fog_params: vec4<f32>, // x=start, y=end, z=strength, w=unused
}

@group(0) @binding(0) var<uniform> view_proj: mat4x4<f32>;
@group(0) @binding(1) var terrain_texture: texture_2d_array<f32>;
@group(0) @binding(2) var terrain_sampler: sampler;
@group(0) @binding(4) var<storage, read> texture_layers: array<u32>;
@group(0) @binding(5) var<storage, read> tint_colors: array<vec4<f32>>;
@group(0) @binding(6) var<uniform> ao_brightness: vec4<f32>;
@group(0) @binding(7) var<storage, read> emission_factors: array<f32>;
@group(0) @binding(8) var<uniform> terrain_visuals: TerrainVisualSettings;

@group(1) @binding(0) var<storage, read> faces: array<FaceDescriptor>;
@group(1) @binding(1) var<uniform> chunk_origin: vec4<f32>;
@group(1) @binding(2) var<storage, read> light_data: array<u32>;

const CORNER_OFFSETS: array<array<vec3<f32>, 4>, 6> = array(
    /* Left    */ array(vec3(0,0,1), vec3(0,0,0), vec3(0,1,1), vec3(0,1,0)),
    /* Right   */ array(vec3(1,0,0), vec3(1,0,1), vec3(1,1,0), vec3(1,1,1)),
    /* Down    */ array(vec3(0,0,1), vec3(1,0,1), vec3(0,0,0), vec3(1,0,0)),
    /* Up      */ array(vec3(0,1,1), vec3(0,1,0), vec3(1,1,1), vec3(1,1,0)),
    /* Forward */ array(vec3(0,0,0), vec3(1,0,0), vec3(0,1,0), vec3(1,1,0)),
    /* Back    */ array(vec3(1,0,1), vec3(0,0,1), vec3(1,1,1), vec3(0,1,1)),
);

const FACE_NORMALS: array<vec3<i32>, 6> = array(
    vec3(-1,0,0), vec3(1,0,0), vec3(0,-1,0), vec3(0,1,0), vec3(0,0,-1), vec3(0,0,1),
);

const FACE_TANGENT_A: array<vec3<i32>, 6> = array(
    vec3(0,1,0), vec3(0,1,0), vec3(1,0,0), vec3(1,0,0), vec3(1,0,0), vec3(1,0,0),
);

const FACE_TANGENT_B: array<vec3<i32>, 6> = array(
    vec3(0,0,1), vec3(0,0,1), vec3(0,0,1), vec3(0,0,1), vec3(0,1,0), vec3(0,1,0),
);

const NORMALS: array<vec3<f32>, 6> = array(
    vec3(-1,0,0), vec3(1,0,0), vec3(0,-1,0), vec3(0,1,0), vec3(0,0,-1), vec3(0,0,1),
);

const FACE_BRIGHTNESS: array<f32, 6> = array(0.80, 0.80, 0.62, 1.0, 0.90, 0.90);
const TRI_TO_QUAD_A: array<u32, 6> = array(0u, 2u, 1u, 1u, 2u, 3u);
const TRI_TO_QUAD_B: array<u32, 6> = array(0u, 3u, 1u, 0u, 2u, 3u);

const LIGHT_FLOOR: f32 = 0.05;
const LIGHT_FALLOFF: f32 = 0.8;

const PADDED_DIM: u32 = 18u;
const PADDED_AREA: u32 = PADDED_DIM * PADDED_DIM;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) @interpolate(flat) block_type: u32,
    @location(3) @interpolate(flat) face_dir: u32,
    @location(4) @interpolate(flat) ao_key: u32,
    @location(5) light: vec2<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) vid: u32) -> VertexOutput {
    let face_idx = vid / 6u;
    let corner_raw = vid % 6u;

    let desc = faces[face_idx];

    let x = (desc.packed >> 27) & 0x1Fu;
    let z = (desc.packed >> 22) & 0x1Fu;
    let y = (desc.packed >> 17) & 0x1Fu;
    let face_dir = (desc.packed >> 14) & 0x7u;

    let block_type = desc.info & 0xFFu;
    let ao_key = (desc.info >> 8) & 0xFFu;

    let ao0 = ao_key & 0x3u;
    let ao1 = (ao_key >> 2u) & 0x3u;
    let ao2 = (ao_key >> 4u) & 0x3u;
    let ao3 = (ao_key >> 6u) & 0x3u;
    var qi = TRI_TO_QUAD_B[corner_raw];
    if ao1 + ao2 > ao0 + ao3 {
        qi = TRI_TO_QUAD_A[corner_raw];
    }

    let offset = CORNER_OFFSETS[face_dir][qi];
    let local_pos = vec3<f32>(f32(x) + offset.x, f32(y) + offset.y, f32(z) + offset.z);

    let light = corner_light(vec3<i32>(i32(x), i32(y), i32(z)), face_dir, offset);

    let world_pos = local_pos + chunk_origin.xyz;
    let clip_pos = view_proj * vec4(world_pos, 1.0);

    return VertexOutput(clip_pos, world_pos, NORMALS[face_dir], block_type, face_dir, ao_key, light);
}

fn padded_coord(value: i32) -> u32 {
    return u32(clamp(value + 1, 0, i32(PADDED_DIM) - 1));
}

fn sample_light(cell: vec3<i32>) -> vec2<f32> {
    let ilp = vec3<u32>(padded_coord(cell.x), padded_coord(cell.y), padded_coord(cell.z));
    let light_cell_idx = ilp.x + ilp.z * PADDED_DIM + ilp.y * PADDED_AREA;
    let light_word = light_data[light_cell_idx >> 2u];
    let packed_light = (light_word >> ((light_cell_idx & 0x3u) * 8u)) & 0xFFu;
    let block_light = f32(packed_light & 0x0Fu) / 15.0;
    let sky_light = f32((packed_light >> 4) & 0x0Fu) / 15.0;
    return vec2(block_light, sky_light);
}

fn corner_axis_sign(offset: vec3<f32>, axis: vec3<i32>) -> i32 {
    let component = offset.x * f32(abs(axis.x)) + offset.y * f32(abs(axis.y)) + offset.z * f32(abs(axis.z));
    if component > 0.5 {
        return 1;
    }
    return -1;
}

fn corner_tangent_offset(offset: vec3<f32>, axis: vec3<i32>) -> vec3<i32> {
    return axis * corner_axis_sign(offset, axis);
}

fn corner_light(block_cell: vec3<i32>, face_dir: u32, offset: vec3<f32>) -> vec2<f32> {
    let base = block_cell + FACE_NORMALS[face_dir];
    let tangent_a = corner_tangent_offset(offset, FACE_TANGENT_A[face_dir]);
    let tangent_b = corner_tangent_offset(offset, FACE_TANGENT_B[face_dir]);

    return (sample_light(base)
        + sample_light(base + tangent_a)
        + sample_light(base + tangent_b)
        + sample_light(base + tangent_a + tangent_b)) * 0.25;
}

fn corner_ao_brightness(ao_key: u32, corner: u32) -> f32 {
    let ao_val = (ao_key >> (corner * 2u)) & 0x3u;
    return ao_brightness[ao_val];
}

fn bilinear(v00: f32, v10: f32, v01: f32, v11: f32, uv: vec2<f32>) -> f32 {
    return mix(mix(v00, v10, uv.x), mix(v01, v11, uv.x), uv.y);
}

fn light_level_curve(level: f32) -> f32 {
    let clamped = clamp(level, 0.0, 1.0);
    return pow(LIGHT_FALLOFF, (1.0 - clamped) * 15.0);
}

fn combined_light_color(light: vec2<f32>) -> vec3<f32> {
    let sky = light_level_curve(light.y) * terrain_visuals.sky_light_color.rgb;
    let block = light_level_curve(light.x) * terrain_visuals.block_light_color.rgb;
    return max(max(sky, block), vec3(LIGHT_FLOOR));
}

fn apply_distance_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let fog_start = terrain_visuals.fog_params.x;
    let fog_end = max(terrain_visuals.fog_params.y, fog_start + 0.001);
    let fog_strength = clamp(terrain_visuals.fog_params.z, 0.0, 1.0);
    let view_distance = distance(world_pos, terrain_visuals.camera_position.xyz);
    let fog = smoothstep(fog_start, fog_end, view_distance) * fog_strength;
    return mix(color, terrain_visuals.fog_color.rgb, fog);
}

fn face_ao_brightness(ao_key: u32, face_dir: u32, block_uv: vec2<f32>) -> f32 {
    let a0 = corner_ao_brightness(ao_key, 0u);
    let a1 = corner_ao_brightness(ao_key, 1u);
    let a2 = corner_ao_brightness(ao_key, 2u);
    let a3 = corner_ao_brightness(ao_key, 3u);

    switch face_dir {
        case 0u: { return bilinear(a3, a2, a1, a0, block_uv); } // Left
        case 1u: { return bilinear(a2, a3, a0, a1, block_uv); } // Right
        case 2u: { return bilinear(a2, a3, a0, a1, block_uv); } // Down
        case 3u: { return bilinear(a1, a3, a0, a2, block_uv); } // Up
        case 4u: { return bilinear(a2, a3, a0, a1, block_uv); } // Forward
        default: { return bilinear(a3, a2, a1, a0, block_uv); } // Back
    }
}

@fragment
fn fragment(@location(0) world_pos: vec3<f32>,
            @location(1) world_normal: vec3<f32>,
            @location(2) @interpolate(flat) block_type: u32,
            @location(3) @interpolate(flat) face_dir: u32,
            @location(4) @interpolate(flat) ao_key: u32,
            @location(5) light: vec2<f32>) -> @location(0) vec4<f32> {
    let n = abs(world_normal);
    let wp = world_pos;
    var face_uv: vec2<f32>;
    if n.x > 0.5 {
        face_uv = vec2(wp.z, 1.0 - wp.y);
    } else if n.y > 0.5 {
        face_uv = vec2(wp.x, wp.z);
    } else {
        face_uv = vec2(wp.x, 1.0 - wp.y);
    }
    let block_uv = fract(face_uv);

    let lookup = block_type * 6u + face_dir;
    let layer = i32(texture_layers[lookup]);
    let tex_color = textureSampleGrad(
        terrain_texture,
        terrain_sampler,
        block_uv,
        layer,
        dpdx(face_uv),
        dpdy(face_uv),
    );
    let tint = tint_colors[lookup];

    let emissive = clamp(emission_factors[lookup], 0.0, 1.0);
    let face_brightness = mix(FACE_BRIGHTNESS[face_dir], 1.0, emissive);
    let ao = mix(face_ao_brightness(ao_key, face_dir, block_uv), 1.0, emissive);
    let light_color = max(combined_light_color(light) * ao * face_brightness, vec3(LIGHT_FLOOR * face_brightness));

    let shaded_color = tex_color.rgb * tint.rgb * light_color;
    let fogged_color = apply_distance_fog(shaded_color, world_pos);
    let color = vec4(fogged_color, tex_color.a * tint.a);
    if color.a < 0.5 {
        discard;
    }
    return color;
}
