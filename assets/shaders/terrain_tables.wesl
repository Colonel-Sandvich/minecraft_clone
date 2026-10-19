// Shared terrain table decoding.
//
// Every terrain shader indexes the per-frame terrain tables by
// `render_id * 6 + canonical face direction`. Texture layer entries pack the
// base texture-array layer in the low 24 bits and the animation frame count in
// the high 8 bits.

const TERRAIN_FACE_COUNT: u32 = 6u;
const TEXTURE_LAYER_MASK: u32 = 0x00ffffffu;
const TEXTURE_FRAME_SHIFT: u32 = 24u;

public fn terrain_table_index(render_id: u32, face: u32) -> u32 {
    return render_id * TERRAIN_FACE_COUNT + face;
}

public fn texture_base_layer(texture_info: u32) -> u32 {
    return texture_info & TEXTURE_LAYER_MASK;
}

public fn texture_frame_count(texture_info: u32) -> u32 {
    return max(texture_info >> TEXTURE_FRAME_SHIFT, 1u);
}
