use bevy::{
    asset::{AssetId, LoadedFolder, RenderAssetUsages},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{
        Extent3d, TextureDataOrder, TextureDescriptor, TextureDimension, TextureFormat,
        TextureUsages, TextureViewDescriptor, TextureViewDimension,
    },
};
use image::{RgbaImage, imageops::FilterType};
use strum::IntoEnumIterator;

use crate::block::{BlockTextureLayer, BlockTextureMap, BlockType, block_and_side_to_texture_path};
use crate::quad::Direction;

const TERRAIN_ANISOTROPY: u16 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, States)]
pub enum TextureState {
    #[default]
    Loading,
    Finished,
}

pub struct BlockTexturePlugin;

impl Plugin for BlockTexturePlugin {
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
    mut messages: MessageReader<AssetEvent<LoadedFolder>>,
) {
    for message in messages.read() {
        if message.is_loaded_with_dependencies(&block_texture_folder.0) {
            next_state.set(TextureState::Finished);
            info!("Textures loaded.");
        }
    }
}

fn setup(
    mut commands: Commands,
    block_texture_handles: Res<BlockTextureFolder>,
    asset_server: Res<AssetServer>,
    loaded_folders: Res<Assets<LoadedFolder>>,
    mut textures: ResMut<Assets<Image>>,
) {
    let loaded_folder = loaded_folders.get(&block_texture_handles.0).unwrap();
    let (block_texture_map, terrain_texture, tile_size, mip_level_count) =
        create_terrain_texture_array(loaded_folder, &asset_server, &mut textures);

    info!(
        "Terrain texture array: {} layers, {}x{}, {} mip levels, {}x anisotropy",
        block_texture_map.len(),
        tile_size.x,
        tile_size.y,
        mip_level_count,
        TERRAIN_ANISOTROPY
    );

    commands.insert_resource(BlockTextures {
        terrain: terrain_texture,
    });
    commands.insert_resource(BlockTextureMap(block_texture_map));
}

#[derive(Resource)]
pub struct BlockTextures {
    pub terrain: Handle<Image>,
}

impl BlockTextures {
    #[cfg(test)]
    pub(crate) fn test_handles() -> Self {
        Self {
            terrain: Handle::default(),
        }
    }
}

fn create_terrain_texture_array(
    loaded_folder: &LoadedFolder,
    asset_server: &AssetServer,
    textures: &mut Assets<Image>,
) -> (
    HashMap<String, BlockTextureLayer>,
    Handle<Image>,
    UVec2,
    u32,
) {
    let used_paths = used_terrain_texture_paths();
    let images_by_path = loaded_images_by_path(loaded_folder, asset_server);

    let mut layers = HashMap::with_capacity(used_paths.len());
    let mut source_layers = Vec::with_capacity(used_paths.len());
    let mut source_size = None;
    let mut source_format = None;

    for (layer, path) in used_paths.iter().enumerate() {
        let id = images_by_path
            .get(*path)
            .unwrap_or_else(|| panic!("terrain texture was not loaded: {path}"));
        let image = textures
            .get(*id)
            .unwrap_or_else(|| panic!("terrain texture asset is missing: {path}"));
        let data = image
            .data
            .as_ref()
            .unwrap_or_else(|| panic!("terrain texture has no CPU data: {path}"));
        let size = image.texture_descriptor.size;
        let format = image.texture_descriptor.format;

        assert_eq!(
            size.depth_or_array_layers, 1,
            "terrain texture must be a single 2D image: {path}"
        );
        assert_eq!(
            format,
            TextureFormat::Rgba8UnormSrgb,
            "terrain texture must be RGBA8 sRGB for mip generation: {path}"
        );
        assert_eq!(
            data.len(),
            (size.width * size.height * 4) as usize,
            "terrain texture data length does not match RGBA8 dimensions: {path}"
        );

        if let Some(expected_size) = source_size {
            assert_eq!(
                UVec2::new(size.width, size.height),
                expected_size,
                "all terrain texture-array layers must have the same dimensions: {path}"
            );
        } else {
            source_size = Some(UVec2::new(size.width, size.height));
        }

        if let Some(expected_format) = source_format {
            assert_eq!(
                format, expected_format,
                "all terrain texture-array layers must have the same format: {path}"
            );
        } else {
            source_format = Some(format);
        }

        source_layers.push(SourceTextureLayer {
            data: data.clone(),
            alpha_mode: alpha_mode(data),
        });
        layers.insert((*path).to_owned(), BlockTextureLayer::new(layer as u32));
    }

    let tile_size = source_size.expect("at least one terrain texture is required");
    let format = source_format.expect("at least one terrain texture is required");
    let mip_level_count = mip_level_count(tile_size.x, tile_size.y);
    let texture_data =
        build_mipmapped_array_data(&source_layers, tile_size.x, tile_size.y, mip_level_count);
    let layer_count = source_layers.len() as u32;

    let image = Image {
        data: Some(texture_data),
        data_order: TextureDataOrder::LayerMajor,
        texture_descriptor: TextureDescriptor {
            label: Some("terrain_texture_array"),
            size: Extent3d {
                width: tile_size.x,
                height: tile_size.y,
                depth_or_array_layers: layer_count,
            },
            mip_level_count,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        sampler: terrain_sampler(),
        texture_view_descriptor: Some(TextureViewDescriptor {
            label: Some("terrain_texture_array_view"),
            dimension: Some(TextureViewDimension::D2Array),
            mip_level_count: Some(mip_level_count),
            array_layer_count: Some(layer_count),
            ..Default::default()
        }),
        asset_usage: RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        copy_on_resize: false,
    };

    (layers, textures.add(image), tile_size, mip_level_count)
}

fn loaded_images_by_path(
    loaded_folder: &LoadedFolder,
    asset_server: &AssetServer,
) -> HashMap<String, AssetId<Image>> {
    let mut images_by_path = HashMap::with_capacity(loaded_folder.handles.len());
    for handle in loaded_folder.handles.iter() {
        let id = handle.id().typed_debug_checked::<Image>();
        let Some(path) = asset_server.get_path(id) else {
            continue;
        };
        images_by_path.insert(path.to_string(), id);
    }
    images_by_path
}

fn used_terrain_texture_paths() -> Vec<&'static str> {
    let mut paths = Vec::new();
    for block in BlockType::iter().filter(|block| block.is_rendered()) {
        for side in Direction::iter() {
            let path = block_and_side_to_texture_path(block, side);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

fn terrain_sampler() -> ImageSampler {
    let mut sampler = ImageSamplerDescriptor::default();
    sampler.set_address_mode(ImageAddressMode::ClampToEdge);
    sampler.mag_filter = ImageFilterMode::Nearest;
    sampler.min_filter = ImageFilterMode::Linear;
    sampler.mipmap_filter = ImageFilterMode::Linear;
    sampler.anisotropy_clamp = TERRAIN_ANISOTROPY;
    ImageSampler::Descriptor(sampler)
}

fn mip_level_count(width: u32, height: u32) -> u32 {
    let mut levels = 1;
    let mut w = width;
    let mut h = height;
    while w > 1 || h > 1 {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        levels += 1;
    }
    levels
}

fn build_mipmapped_array_data(
    source_layers: &[SourceTextureLayer],
    width: u32,
    height: u32,
    expected_mip_levels: u32,
) -> Vec<u8> {
    let mut data = Vec::new();
    for source in source_layers {
        let mut mip = RgbaImage::from_raw(width, height, source.data.clone())
            .expect("terrain texture data should match RGBA8 dimensions");
        data.extend_from_slice(mip.as_raw());

        let cutout_coverage = match source.alpha_mode {
            TextureAlphaMode::BinaryCutout => Some(alpha_coverage(mip.as_raw())),
            TextureAlphaMode::Opaque | TextureAlphaMode::Translucent => None,
        };

        let mut generated_levels = 1;
        while mip.width() > 1 || mip.height() > 1 {
            let next_width = (mip.width() / 2).max(1);
            let next_height = (mip.height() / 2).max(1);
            mip = resize_alpha_aware(&mip, next_width, next_height, source.alpha_mode);
            if let Some(coverage) = cutout_coverage {
                preserve_binary_alpha_coverage(&mut mip, coverage);
            }
            data.extend_from_slice(mip.as_raw());
            generated_levels += 1;
        }

        debug_assert_eq!(generated_levels, expected_mip_levels);
    }
    data
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextureAlphaMode {
    Opaque,
    BinaryCutout,
    Translucent,
}

struct SourceTextureLayer {
    data: Vec<u8>,
    alpha_mode: TextureAlphaMode,
}

fn alpha_mode(data: &[u8]) -> TextureAlphaMode {
    let mut has_transparent = false;
    let mut has_partial = false;
    for alpha in data.chunks_exact(4).map(|pixel| pixel[3]) {
        has_transparent |= alpha < 255;
        has_partial |= alpha != 0 && alpha != 255;
    }

    match (has_transparent, has_partial) {
        (false, _) => TextureAlphaMode::Opaque,
        (true, false) => TextureAlphaMode::BinaryCutout,
        (true, true) => TextureAlphaMode::Translucent,
    }
}

fn resize_alpha_aware(
    image: &RgbaImage,
    width: u32,
    height: u32,
    alpha_mode: TextureAlphaMode,
) -> RgbaImage {
    if alpha_mode == TextureAlphaMode::Opaque {
        return image::imageops::resize(image, width, height, FilterType::Triangle);
    }

    let premultiplied = premultiply_alpha(image);
    let mut resized = image::imageops::resize(&premultiplied, width, height, FilterType::Triangle);
    unpremultiply_alpha(&mut resized);
    resized
}

fn premultiply_alpha(image: &RgbaImage) -> RgbaImage {
    let mut premultiplied = image.clone();
    for pixel in premultiplied.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        pixel[0] = ((u16::from(pixel[0]) * alpha + 127) / 255) as u8;
        pixel[1] = ((u16::from(pixel[1]) * alpha + 127) / 255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * alpha + 127) / 255) as u8;
    }
    premultiplied
}

fn unpremultiply_alpha(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            continue;
        }
        pixel[0] = ((u16::from(pixel[0]) * 255 + alpha / 2) / alpha).min(255) as u8;
        pixel[1] = ((u16::from(pixel[1]) * 255 + alpha / 2) / alpha).min(255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * 255 + alpha / 2) / alpha).min(255) as u8;
    }
}

fn alpha_coverage(data: &[u8]) -> f32 {
    let pixel_count = data.len() / 4;
    if pixel_count == 0 {
        return 0.0;
    }
    let opaque_count = data.chunks_exact(4).filter(|pixel| pixel[3] >= 128).count();
    opaque_count as f32 / pixel_count as f32
}

fn preserve_binary_alpha_coverage(image: &mut RgbaImage, coverage: f32) {
    let pixel_count = image.as_raw().len() / 4;
    if pixel_count == 0 {
        return;
    }

    let mut opaque_count = (coverage * pixel_count as f32).round() as usize;
    if coverage > 0.0 && opaque_count == 0 {
        opaque_count = 1;
    }
    opaque_count = opaque_count.min(pixel_count);

    let mut pixels_by_alpha = image
        .as_raw()
        .chunks_exact(4)
        .enumerate()
        .map(|(index, pixel)| (index, pixel[3]))
        .collect::<Vec<_>>();
    pixels_by_alpha.sort_by(|a, b| b.1.cmp(&a.1));

    let mut keep = vec![false; pixel_count];
    for (index, alpha) in pixels_by_alpha.into_iter().take(opaque_count) {
        keep[index] = alpha > 0;
    }

    for (index, pixel) in image.pixels_mut().enumerate() {
        pixel[3] = if keep[index] { 255 } else { 0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_texture_paths_are_unique() {
        let paths = used_terrain_texture_paths();
        let mut seen = std::collections::HashSet::new();
        assert!(paths.iter().all(|path| seen.insert(*path)));
    }

    #[test]
    fn mip_level_count_reaches_one_by_one() {
        assert_eq!(mip_level_count(16, 16), 5);
        assert_eq!(mip_level_count(16, 8), 5);
        assert_eq!(mip_level_count(1, 1), 1);
    }

    #[test]
    fn alpha_mode_detects_binary_cutouts() {
        assert_eq!(
            alpha_mode(&[0, 0, 0, 0, 255, 255, 255, 255]),
            TextureAlphaMode::BinaryCutout
        );
        assert_eq!(alpha_mode(&[0, 0, 0, 255]), TextureAlphaMode::Opaque);
        assert_eq!(alpha_mode(&[0, 0, 0, 64]), TextureAlphaMode::Translucent);
    }
}
