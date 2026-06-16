struct BlockMaterial {
    tile_size: vec2<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var material_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var material_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> material: BlockMaterial;

struct FragmentInput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) uv_b: vec2<f32>,
    @location(5) color: vec4<f32>,
}

@fragment
fn fragment(in: FragmentInput) -> @location(0) vec4<f32> {
    // Compute per-block UV from world position + face normal.
    // U comes from the horizontal axis perpendicular to the normal.
    // V always uses world Y, flipped so V=0 = top of block = top of texture.
    let n = abs(in.world_normal);
    let wp = in.world_position.xyz;
    var face_uv: vec2<f32>;
    if n.x > 0.5 {
        // X-face (Left/Right): horizontal = Z, vertical = Y (flipped)
        face_uv = vec2(wp.z, 1.0 - wp.y);
    } else if n.y > 0.5 {
        // Y-face (Up/Down): horizontal = X, vertical = Z
        face_uv = vec2(wp.x, wp.z);
    } else {
        // Z-face (Forward/Backward): horizontal = X, vertical = Y (flipped)
        face_uv = vec2(wp.x, 1.0 - wp.y);
    }
    let block_uv = fract(face_uv);
    let atlas_uv = in.uv_b + block_uv * material.tile_size;
    let color = textureSample(material_texture, material_sampler, atlas_uv) * in.color;
    if color.a < 0.5 {
        discard;
    }
    return color;
}
