use bevy::{prelude::*, render::extract_resource::ExtractResource};
use strum::IntoEnumIterator;

use crate::{
    block::{
        BlockTextureLayer, BlockTextureMap, RENDER_ID_COUNT, WATER_RENDER_ID, from_render_id,
        render_id_to_colour,
    },
    quad::Direction,
    textures::{BlockTextures, TextureState},
    world::chunk::ambient_occlusion::AO_BRIGHTNESS,
};

use super::super::DIRECTION_COUNT;

#[derive(Resource, Clone)]
pub(super) struct TerrainMaterialState {
    pub(super) terrain_texture_handle: Handle<Image>,
    pub(super) texture_layers: Vec<u32>,
    pub(super) tint_colors: Vec<[f32; 4]>,
    pub(super) emission_factors: Vec<f32>,
    pub(super) ao_brightness: [f32; 4],
}

impl ExtractResource for TerrainMaterialState {
    type Source = TerrainMaterialState;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

pub(super) fn install(app: &mut App) {
    app.add_systems(
        Update,
        sync_terrain_material_state.run_if(in_state(TextureState::Finished)),
    );
}

pub(super) fn sync_terrain_material_state(
    mut commands: Commands,
    block_textures: Res<BlockTextures>,
    block_texture_map: Res<BlockTextureMap>,
    current: Option<Res<TerrainMaterialState>>,
) {
    if current.is_some() && !block_textures.is_changed() && !block_texture_map.is_changed() {
        return;
    }

    commands.insert_resource(build_terrain_material_state(
        &block_textures,
        &block_texture_map,
    ));
}

fn build_terrain_material_state(
    block_textures: &BlockTextures,
    block_texture_map: &BlockTextureMap,
) -> TerrainMaterialState {
    let entry_count = RENDER_ID_COUNT * DIRECTION_COUNT;
    let mut texture_layers = Vec::with_capacity(entry_count);
    let mut tint_colors = Vec::with_capacity(entry_count);
    let mut emission_factors = Vec::with_capacity(entry_count);

    for render_id in 0..RENDER_ID_COUNT as u16 {
        let emission = match render_id {
            0 | WATER_RENDER_ID => 0.0,
            _ => f32::from(from_render_id(render_id).unwrap().light_emission()) / 15.0,
        };

        for side in Direction::iter() {
            if render_id == 0 {
                texture_layers.push(pack_texture_layer(BlockTextureLayer::default(), 1));
                tint_colors.push([0.0; 4]);
            } else {
                let animation = block_texture_map.render_id_to_texture_animation(render_id, side);
                let color = render_id_to_colour(render_id, side);
                texture_layers.push(pack_texture_layer(
                    animation.base_layer(),
                    animation.frame_count(),
                ));
                tint_colors.push([color.x, color.y, color.z, color.w]);
            }
            emission_factors.push(emission);
        }
    }

    TerrainMaterialState {
        terrain_texture_handle: block_textures.terrain.clone(),
        texture_layers,
        tint_colors,
        emission_factors,
        ao_brightness: AO_BRIGHTNESS,
    }
}

fn pack_texture_layer(layer: BlockTextureLayer, frame_count: u32) -> u32 {
    layer.index() | (frame_count.min(255) << 24)
}

#[cfg(test)]
mod tests {
    use bevy::platform::collections::HashMap;

    use super::*;
    use crate::block::{
        BlockTextureAnimation, BlockType, render_id_for_block, render_id_to_texture_path,
    };

    #[derive(Resource, Default)]
    struct TerrainMaterialStateChangeCount(usize);

    fn count_terrain_material_state_changes(
        material_state: Option<Res<TerrainMaterialState>>,
        mut changes: ResMut<TerrainMaterialStateChangeCount>,
    ) {
        if material_state.is_some_and(|state| state.is_changed()) {
            changes.0 += 1;
        }
    }

    #[test]
    fn terrain_material_state_only_changes_with_texture_resources() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(test_texture_map())
            .insert_resource(BlockTextures::test_handles())
            .init_resource::<TerrainMaterialStateChangeCount>()
            .add_systems(
                Update,
                (
                    sync_terrain_material_state,
                    count_terrain_material_state_changes,
                )
                    .chain(),
            );

        app.update();

        let entry_count = RENDER_ID_COUNT * DIRECTION_COUNT;
        let material_state = app.world().resource::<TerrainMaterialState>();
        assert_eq!(material_state.texture_layers.len(), entry_count);
        assert_eq!(material_state.tint_colors.len(), entry_count);
        assert_eq!(material_state.emission_factors.len(), entry_count);
        assert_eq!(
            app.world().resource::<TerrainMaterialStateChangeCount>().0,
            1
        );

        app.update();
        assert_eq!(
            app.world().resource::<TerrainMaterialStateChangeCount>().0,
            1,
            "unrelated frames must not recreate the texture state"
        );

        let stone_render_id = render_id_for_block(BlockType::Stone);
        let stone_path = render_id_to_texture_path(stone_render_id, Direction::Up);
        app.world_mut().resource_mut::<BlockTextureMap>().0.insert(
            stone_path.to_owned(),
            BlockTextureAnimation::new(BlockTextureLayer::new(123), 4),
        );

        app.update();

        let up_index = Direction::iter()
            .position(|side| side == Direction::Up)
            .unwrap();
        let stone_up_index = stone_render_id as usize * DIRECTION_COUNT + up_index;
        assert_eq!(
            app.world()
                .resource::<TerrainMaterialState>()
                .texture_layers[stone_up_index],
            pack_texture_layer(BlockTextureLayer::new(123), 4)
        );
        assert_eq!(
            app.world().resource::<TerrainMaterialStateChangeCount>().0,
            2
        );
    }

    fn test_texture_map() -> BlockTextureMap {
        let mut paths = HashMap::default();
        for render_id in 1..RENDER_ID_COUNT as u16 {
            for side in Direction::iter() {
                let path = render_id_to_texture_path(render_id, side).to_owned();
                let next_layer = paths.len() as u32;
                paths.entry(path).or_insert(BlockTextureAnimation::new(
                    BlockTextureLayer::new(next_layer),
                    1,
                ));
            }
        }
        BlockTextureMap(paths)
    }

    #[test]
    fn texture_layer_packs_animation_count_in_high_byte() {
        let packed = pack_texture_layer(BlockTextureLayer::default(), 300);
        assert_eq!(packed >> 24, 255);
    }

    #[test]
    fn material_table_has_one_entry_per_render_id_face_pair() {
        assert_eq!(RENDER_ID_COUNT * DIRECTION_COUNT, RENDER_ID_COUNT * 6);
    }
}
