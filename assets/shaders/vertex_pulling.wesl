// Vertex-pulling terrain backend root.
//
// The shader decodes one triangle-list vertex per `vertex_index` from the
// compact face descriptors. All shading/geometry logic lives in
// `terrain_shading.wesl` so the mesh-shader backend stays equivalent.

import super::terrain_shading as terrain;
import super::terrain_shading::VertexOutput;

@vertex
fn vertex(@builtin(vertex_index) vid: u32) -> VertexOutput {
    return terrain::terrain_vertex(vid);
}

@fragment
fn fragment(@location(0) world_pos: vec3<f32>,
            @location(1) world_normal: vec3<f32>,
            @location(2) @interpolate(flat) block_type: u32,
            @location(3) @interpolate(flat) face_dir: u32,
            @location(4) @interpolate(flat) ao_key: u32,
            @location(5) light: vec2<f32>,
            @location(6) @interpolate(flat) water_up_flow: u32,
            @location(7) @interpolate(flat) water_flow_code: u32) -> @location(0) vec4<f32> {
    return terrain::terrain_fragment(
        world_pos,
        world_normal,
        block_type,
        face_dir,
        ao_key,
        light,
        water_up_flow,
        water_flow_code,
    );
}
