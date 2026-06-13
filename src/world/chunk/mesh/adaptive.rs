use super::{ChunkLayerMeshes, ChunkMeshInput, ChunkMesher, DirectChunkMesher, GreedyChunkMesher};

#[derive(Debug, Default, Clone, Copy)]
pub struct AdaptiveChunkMesher;

impl ChunkMesher for AdaptiveChunkMesher {
    fn name(&self) -> &'static str {
        "adaptive"
    }

    fn mesh(&self, input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
        if input.blocks.can_skip_mesh() {
            return Vec::new();
        }

        let rendered = input.blocks.center_rendered_blocks as usize;

        if rendered < 2048 {
            return DirectChunkMesher.mesh(input);
        }

        GreedyChunkMesher.mesh(input)
    }
}
