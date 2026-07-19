use bevy::{
    prelude::*,
    reflect::TypePath,
    render::{render_resource::AsBindGroup, storage::ShaderBuffer},
    shader::ShaderRef,
};

use crate::{
    block::{BlockMaterialLayer, BlockTextureMap, BlockVisualTable},
    quad::Direction,
    textures::BlockTextures,
};

use super::ItemStack;

const DROPPED_BLOCK_SHADER_PATH: &str = "shaders/dropped_block.wgsl";
const CUBOID_VERTICES_PER_FACE: usize = 4;

/// Face order emitted by Bevy's `CuboidMeshBuilder`.
///
/// This is intentionally kept beside the cuboid adapter. It is not a general
/// direction ordering used by gameplay or terrain meshing.
const BEVY_CUBOID_FACE_ORDER: [Direction; Direction::COUNT] = [
    Direction::Backward,
    Direction::Forward,
    Direction::Right,
    Direction::Left,
    Direction::Up,
    Direction::Down,
];

pub(super) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<DroppedBlockMaterial>::default());
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct DroppedBlockMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    terrain_texture: Handle<Image>,
    #[storage(2, read_only)]
    texture_layers: Handle<ShaderBuffer>,
    #[storage(3, read_only)]
    tint_colors: Handle<ShaderBuffer>,
    #[uniform(4)]
    settings: Vec4,
    #[storage(5, read_only)]
    emission_factors: Handle<ShaderBuffer>,
    alpha_mode: AlphaMode,
}

impl Material for DroppedBlockMaterial {
    fn fragment_shader() -> ShaderRef {
        DROPPED_BLOCK_SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
}

#[derive(Resource)]
pub(super) struct DroppedItemRenderAssets {
    pub(super) cube_mesh: Handle<Mesh>,
    materials: [Handle<DroppedBlockMaterial>; BlockMaterialLayer::COUNT],
}

impl DroppedItemRenderAssets {
    pub(super) fn material_for(&self, stack: ItemStack) -> Handle<DroppedBlockMaterial> {
        let layer = stack
            .item
            .material_layer()
            .expect("only block-items have a cube dropped-item model");
        self.materials[layer.index()].clone()
    }

    #[cfg(test)]
    pub(super) fn test_handles() -> Self {
        Self {
            cube_mesh: Handle::default(),
            materials: std::array::from_fn(|_| Handle::default()),
        }
    }
}

pub(super) fn prepare_dropped_item_render_assets(
    block_textures: Res<BlockTextures>,
    block_texture_map: Res<BlockTextureMap>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut materials: ResMut<Assets<DroppedBlockMaterial>>,
) {
    let visuals = BlockVisualTable::build(&block_texture_map);
    let texture_layers = buffers.add(ShaderBuffer::from(visuals.texture_layers));
    let tint_colors = buffers.add(ShaderBuffer::from(visuals.tint_colors));
    let emission_factors = buffers.add(ShaderBuffer::from(visuals.emission_factors));
    let make_material = |alpha_mode, alpha_cutoff, materials: &mut Assets<DroppedBlockMaterial>| {
        materials.add(DroppedBlockMaterial {
            terrain_texture: block_textures.terrain.clone(),
            texture_layers: texture_layers.clone(),
            tint_colors: tint_colors.clone(),
            settings: vec4(alpha_cutoff, 0.0, 0.0, 0.0),
            emission_factors: emission_factors.clone(),
            alpha_mode,
        })
    };

    commands.insert_resource(DroppedItemRenderAssets {
        cube_mesh: meshes.add(dropped_block_mesh()),
        materials: [
            make_material(AlphaMode::Opaque, 0.0, &mut materials),
            make_material(AlphaMode::AlphaToCoverage, 0.5, &mut materials),
            make_material(AlphaMode::Blend, 0.0, &mut materials),
        ],
    });
}

fn dropped_block_mesh() -> Mesh {
    let mut mesh = Mesh::from(Cuboid::from_length(0.25));
    assert_eq!(
        mesh.count_vertices(),
        BEVY_CUBOID_FACE_ORDER.len() * CUBOID_VERTICES_PER_FACE,
        "Bevy's Cuboid vertex layout changed"
    );
    let face_ids = BEVY_CUBOID_FACE_ORDER
        .into_iter()
        .flat_map(|direction| [[direction.index() as f32, 0.0, 0.0, 1.0]; CUBOID_VERTICES_PER_FACE])
        .collect::<Vec<_>>();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, face_ids);
    mesh
}

#[cfg(test)]
mod tests {
    use bevy::mesh::VertexAttributeValues;

    use super::*;

    #[test]
    fn dropped_block_mesh_encodes_bevy_cuboid_face_order() {
        let mesh = dropped_block_mesh();
        let face_ids = match mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap() {
            VertexAttributeValues::Float32x4(values) => values,
            _ => panic!("unexpected face ID format"),
        };

        for (colors, direction) in face_ids
            .chunks_exact(CUBOID_VERTICES_PER_FACE)
            .zip(BEVY_CUBOID_FACE_ORDER)
        {
            assert!(
                colors
                    .iter()
                    .all(|color| color[0] as usize == direction.index())
            );
        }
    }
}
