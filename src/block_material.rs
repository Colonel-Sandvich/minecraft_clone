use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy_shader::ShaderRef;

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct BlockMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Option<Handle<Image>>,
    #[uniform(2)]
    pub tile_size: Vec2,
    pub alpha_mode: AlphaMode,
}

impl Material for BlockMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/block_material.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
}
