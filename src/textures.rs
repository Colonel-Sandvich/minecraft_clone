use bevy::{
    asset::LoadedFolder, prelude::*, render::texture::ImageSampler, utils::hashbrown::HashMap,
};

use crate::block::BlockTextureMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, States)]
pub enum TextureState {
    #[default]
    Loading,
    Finished,
}

pub struct BlockTextureAtlasPlugin;

impl Plugin for BlockTextureAtlasPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<TextureState>()
            .add_systems(OnEnter(TextureState::Loading), load_textures)
            .add_systems(
                Update,
                check_textures.run_if(in_state(TextureState::Loading)),
            )
            .add_systems(OnEnter(TextureState::Finished), setup);
    }
}

#[derive(Resource, Default)]
struct BlockTextureFolder(Handle<LoadedFolder>);

fn load_textures(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(BlockTextureFolder(
        asset_server.load_folder("textures/block"),
    ));
}

fn check_textures(
    mut next_state: ResMut<NextState<TextureState>>,
    block_texture_folder: Res<BlockTextureFolder>,
    mut events: EventReader<AssetEvent<LoadedFolder>>,
) {
    // Advance the `LoadState` once all block texture handles have been loaded by the `AssetServer`
    for event in events.read() {
        if event.is_loaded_with_dependencies(&block_texture_folder.0) {
            next_state.set(TextureState::Finished);
            info!("Textures loaded.")
        }
    }
}

fn setup(
    mut commands: Commands,
    block_texture_handles: Res<BlockTextureFolder>,
    asset_server: Res<AssetServer>,
    loaded_folders: Res<Assets<LoadedFolder>>,
    mut textures: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let loaded_folder = loaded_folders.get(&block_texture_handles.0).unwrap();

    // TODO: Look into mipmaps
    // Look into padding for MipMaps
    let (texture_atlas_nearest, nearest_texture) =
        create_texture_atlas(loaded_folder, None, ImageSampler::nearest(), &mut textures);

    commands.insert_resource(BlockStandardMaterial(materials.add(StandardMaterial {
        base_color_texture: Some(nearest_texture),
        metallic: 0.,
        reflectance: 0.,
        alpha_mode: AlphaMode::Mask(0.1),
        ..default()
    })));

    let block_texture_map = create_texture_map(loaded_folder, asset_server, texture_atlas_nearest);

    commands.insert_resource(BlockTextureMap(block_texture_map));
}

#[derive(Resource, Deref)]
pub struct BlockStandardMaterial(Handle<StandardMaterial>);

/// Create a texture atlas with the given padding and sampling settings
/// from the individual sprites in the given folder.
fn create_texture_atlas(
    folder: &LoadedFolder,
    padding: Option<UVec2>,
    sampling: ImageSampler,
    textures: &mut ResMut<Assets<Image>>,
) -> (TextureAtlasLayout, Handle<Image>) {
    // Build a texture atlas using the individual texture pngs
    let mut texture_atlas_builder =
        TextureAtlasBuilder::default().padding(padding.unwrap_or_default());
    for handle in folder.handles.iter() {
        let id = handle.id().typed_debug_checked::<Image>();
        let Some(texture) = textures.get(id) else {
            warn!(
                "{:?} did not resolve to an `Image` asset.",
                handle.path().unwrap()
            );
            continue;
        };

        texture_atlas_builder.add_texture(Some(id), texture);
    }

    let (texture_atlas_layout, mut texture) = texture_atlas_builder.finish().unwrap();
    texture.sampler = sampling;

    let texture_handle = textures.add(texture);

    (texture_atlas_layout, texture_handle)
}

fn create_texture_map(
    loaded_folder: &LoadedFolder,
    asset_server: Res<AssetServer>,
    texture_atlas_nearest: TextureAtlasLayout,
) -> HashMap<String, Rect> {
    let size = texture_atlas_nearest.size;

    let mut block_texture_map = HashMap::with_capacity(loaded_folder.handles.len());

    for handle in loaded_folder.handles.iter() {
        let id = handle.id().typed_debug_checked::<Image>();

        let path = asset_server.get_path(id).unwrap();

        let tex_index = texture_atlas_nearest.get_texture_index(id).unwrap();

        let tex_rect = texture_atlas_nearest.textures.get(tex_index).unwrap();

        block_texture_map.insert(
            path.to_string(),
            tex_rect.normalize(Rect {
                min: Vec2::ZERO,
                max: size,
            }),
        );
    }
    block_texture_map
}
