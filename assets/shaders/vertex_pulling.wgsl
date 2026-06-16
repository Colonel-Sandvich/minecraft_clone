// Vertex-pulling shader — atlas texturing, AO, smooth lighting.
//
// Bind group 0 (per frame, global):
//   binding 0: view_proj uniform (mat4x4<f32>)
//   binding 1: atlas_texture (texture_2d<f32>)
//   binding 2: atlas_sampler (sampler)
//   binding 3: tile_size uniform (vec2<f32>)
//   binding 4: tile_offsets storage (array<vec2<f32>>)  // (block_type * 6 + face_dir)
//   binding 5: tint_colors storage (array<vec4<f32>>)    // (block_type * 6 + face_dir)
//   binding 6: ao_brightness uniform (vec4<f32>)
// Bind group 1 (per chunk):
//   binding 0: faces storage (array<FaceDescriptor>)
//   binding 1: chunk_origin uniform (vec4<f32>)
//   binding 2: light_data storage (array<u32>) // padded 18³, 4 packed light cells per u32

struct FaceDescriptor {
    packed: u32,
    info: u32,
}

@group(0) @binding(0) var<uniform> view_proj: mat4x4<f32>;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
@group(0) @binding(3) var<uniform> tile_size: vec2<f32>;
@group(0) @binding(4) var<storage, read> tile_offsets: array<vec2<f32>>;
@group(0) @binding(5) var<storage, read> tint_colors: array<vec4<f32>>;
@group(0) @binding(6) var<uniform> ao_brightness: vec4<f32>;

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

const NORMALS: array<vec3<f32>, 6> = array(
    vec3(-1,0,0), vec3(1,0,0), vec3(0,-1,0), vec3(0,1,0), vec3(0,0,-1), vec3(0,0,1),
);

const FACE_BRIGHTNESS: array<f32, 6> = array(0.86, 0.86, 0.68, 1.0, 0.86, 0.86);
const TRI_TO_QUAD_A: array<u32, 6> = array(0u, 2u, 1u, 1u, 2u, 3u);
const TRI_TO_QUAD_B: array<u32, 6> = array(0u, 3u, 1u, 0u, 2u, 3u);

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

    // Sample light from the cell directly in front of the face (one cell in
    // the face normal direction from the block center). This cell is always
    // air/transparent when the face is emitted, avoiding zero-light reads
    // that happen when per-corner offsets land inside solid diagonal blocks.
    let light_cell = vec3<i32>(i32(x), i32(y), i32(z)) + FACE_NORMALS[face_dir];
    let ilp = vec3<u32>(u32(light_cell.x + 1), u32(light_cell.y + 1), u32(light_cell.z + 1));
    let light_cell_idx = ilp.x + ilp.z * PADDED_DIM + ilp.y * PADDED_AREA;
    let light_word = light_data[light_cell_idx >> 2u];
    let packed_light = (light_word >> ((light_cell_idx & 0x3u) * 8u)) & 0xFFu;
    let block_light = f32(packed_light & 0x0Fu) / 15.0;
    let sky_light = f32((packed_light >> 4) & 0x0Fu) / 15.0;

    let world_pos = local_pos + chunk_origin.xyz;
    let clip_pos = view_proj * vec4(world_pos, 1.0);

    return VertexOutput(clip_pos, world_pos, NORMALS[face_dir], block_type, face_dir, ao_key, vec2(block_light, sky_light));
}

fn corner_ao_brightness(ao_key: u32, corner: u32) -> f32 {
    let ao_val = (ao_key >> (corner * 2u)) & 0x3u;
    return ao_brightness[ao_val];
}

fn bilinear(v00: f32, v10: f32, v01: f32, v11: f32, uv: vec2<f32>) -> f32 {
    return mix(mix(v00, v10, uv.x), mix(v01, v11, uv.x), uv.y);
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
    let tile_offset = tile_offsets[lookup];
    let atlas_uv = tile_offset + block_uv * tile_size;

    let tex_color = textureSample(atlas_texture, atlas_sampler, atlas_uv);
    let tint = tint_colors[lookup];

    let face_brightness = FACE_BRIGHTNESS[face_dir];
    let ao = face_ao_brightness(ao_key, face_dir, block_uv);
    let combined_light = min(light.x + light.y, 1.0);
    let final_brightness = combined_light * ao * face_brightness;

    let color = tex_color * tint * vec4(final_brightness, final_brightness, final_brightness, 1.0);
    if color.a < 0.5 {
        discard;
    }
    return color;
}
