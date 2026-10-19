// Mesh-shader terrain backend root (native Vulkan/DX12/Metal only).
//
// Each mesh workgroup handles up to 32 packed faces. With six vertices and two
// triangles per face that caps a workgroup at 192 vertices / 64 primitives,
// safely below wgpu's recommended minimum mesh-shader limits (256/256) and the
// 128-invocation workgroup limit. The CPU dispatches
// `ceil(face_count / 32)` workgroups for a chunk layer; the final workgroup
// writes only the faces that exist.
//
// Geometry and shading are imported from `terrain_shading.wesl`, so the output
// matches the vertex-pulling backend exactly.

import super::terrain_shading as terrain;
import super::terrain_shading::VertexOutput;

enable wgpu_mesh_shader;

const FACES_PER_WORKGROUP: u32 = 32u;
const VERTICES_PER_FACE: u32 = 6u;
const PRIMITIVES_PER_FACE: u32 = 2u;
const MAX_VERTICES: u32 = FACES_PER_WORKGROUP * VERTICES_PER_FACE;
const MAX_PRIMITIVES: u32 = FACES_PER_WORKGROUP * PRIMITIVES_PER_FACE;

struct MeshPrimitiveOutput {
    @builtin(triangle_indices) indices: vec3<u32>,
}

struct MeshOutput {
    @builtin(vertex_count) vertex_count: u32,
    @builtin(primitive_count) primitive_count: u32,
    @builtin(vertices) vertices: array<VertexOutput, MAX_VERTICES>,
    @builtin(primitives) primitives: array<MeshPrimitiveOutput, MAX_PRIMITIVES>,
}

var<workgroup> mesh_output: MeshOutput;

@mesh(mesh_output)
@workgroup_size(32, 1, 1)
fn mesh(@builtin(local_invocation_index) invocation: u32,
        @builtin(workgroup_id) workgroup_id: vec3<u32>) {
    // `chunk_origin.w` carries the layer's face count; every chunk has fewer
    // than 2^24 faces, so f32 represents it exactly.
    let face_count = terrain::terrain_face_count();
    let first_face = workgroup_id.x * FACES_PER_WORKGROUP;

    if first_face >= face_count {
        mesh_output.vertex_count = 0u;
        mesh_output.primitive_count = 0u;
        return;
    }

    let valid_faces = min(face_count - first_face, FACES_PER_WORKGROUP);
    mesh_output.vertex_count = valid_faces * VERTICES_PER_FACE;
    mesh_output.primitive_count = valid_faces * PRIMITIVES_PER_FACE;

    let face_idx = first_face + invocation;
    if face_idx < face_count {
        let vertex_base = invocation * VERTICES_PER_FACE;
        let primitive_base = invocation * PRIMITIVES_PER_FACE;

        for (var corner = 0u; corner < VERTICES_PER_FACE; corner += 1u) {
            mesh_output.vertices[vertex_base + corner] =
                terrain::terrain_vertex(face_idx * VERTICES_PER_FACE + corner);
        }

        mesh_output.primitives[primitive_base] = MeshPrimitiveOutput(
            vec3<u32>(vertex_base, vertex_base + 1u, vertex_base + 2u),
        );
        mesh_output.primitives[primitive_base + 1u] = MeshPrimitiveOutput(
            vec3<u32>(vertex_base + 3u, vertex_base + 4u, vertex_base + 5u),
        );
    }

    // Naga copies the workgroup output to the stage outputs on return; make
    // sure every invocation's writes are visible before that copy starts.
    workgroupBarrier();
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
