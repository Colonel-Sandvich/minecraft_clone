use crate::{
    block::{BlockTextureMap, BlockVisualTable},
    textures::{BlockTextures, TextureState},
    world::chunk::ambient_occlusion::AO_BRIGHTNESS,
};
use bevy::{prelude::*, render::extract_resource::ExtractResource};

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
    let visuals = BlockVisualTable::build(block_texture_map);

    TerrainMaterialState {
        terrain_texture_handle: block_textures.terrain.clone(),
        texture_layers: visuals.texture_layers,
        tint_colors: visuals.tint_colors,
        emission_factors: visuals.emission_factors,
        ao_brightness: AO_BRIGHTNESS,
    }
}

#[cfg(test)]
mod tests {
    use bevy::platform::collections::HashMap;

    use super::*;
    use crate::block::{
        BlockTextureAnimation, BlockTextureLayer, RENDER_ID_COUNT, pack_texture_layer,
        render_id_for_block, render_id_to_texture_path,
    };
    use crate::item::Item;
    use crate::quad::Direction;

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

        let entry_count = RENDER_ID_COUNT * Direction::COUNT;
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

        let stone_render_id = render_id_for_block(Item::Stone);
        let stone_path = render_id_to_texture_path(stone_render_id, Direction::Up);
        app.world_mut().resource_mut::<BlockTextureMap>().0.insert(
            stone_path.to_owned(),
            BlockTextureAnimation::new(BlockTextureLayer::new(123), 4),
        );

        app.update();

        let up_index = Direction::Up.index();
        let stone_up_index = stone_render_id as usize * Direction::COUNT + up_index;
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
            for side in Direction::ALL {
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
        assert_eq!(RENDER_ID_COUNT * Direction::COUNT, RENDER_ID_COUNT * 6);
    }
}
