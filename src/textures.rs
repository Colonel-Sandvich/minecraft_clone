use bevy::{
    asset::{AssetTrackingSystems, RenderAssetUsages},
    image::{
        ImageAddressMode, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor,
    },
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{
        Extent3d, TextureDataOrder, TextureDescriptor, TextureDimension, TextureFormat,
        TextureUsages, TextureViewDescriptor, TextureViewDimension,
    },
};
use image::{RgbaImage, imageops::FilterType};
use strum::IntoEnumIterator;

use crate::block::{
    BlockTextureAnimation, BlockTextureLayer, BlockTextureMap, BlockType, WATER_RENDER_ID,
    render_id_for_block, render_id_to_texture_path,
};
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
                PreUpdate,
                check_textures
                    .after(AssetTrackingSystems)
                    .run_if(in_state(TextureState::Loading)),
            )
            .add_systems(OnEnter(TextureState::Finished), setup);
    }
}

#[derive(Resource)]
struct BlockTextureSources(Vec<(&'static str, Handle<Image>)>);

fn load_textures(mut commands: Commands, asset_server: Res<AssetServer>) {
    let sources = used_terrain_texture_paths()
        .into_iter()
        .map(|path| {
            let handle: Handle<Image> = asset_server
                .load_builder()
                .with_settings(|settings: &mut ImageLoaderSettings| {
                    settings.asset_usage = RenderAssetUsages::MAIN_WORLD;
                })
                .load(path);
            (path, handle)
        })
        .collect();
    commands.insert_resource(BlockTextureSources(sources));
}

fn check_textures(
    mut next_state: ResMut<NextState<TextureState>>,
    sources: Res<BlockTextureSources>,
    asset_server: Res<AssetServer>,
) {
    if sources
        .0
        .iter()
        .all(|(_, handle)| asset_server.is_loaded_with_dependencies(handle.id()))
    {
        next_state.set(TextureState::Finished);
        info!(count = sources.0.len(), "Textures loaded.");
    }
}

fn setup(
    mut commands: Commands,
    sources: Res<BlockTextureSources>,
    mut textures: ResMut<Assets<Image>>,
) {
    let (block_texture_map, terrain_texture, tile_size, mip_level_count) =
        create_terrain_texture_array(&sources, &mut textures);

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
    commands.remove_resource::<BlockTextureSources>();
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
    sources: &BlockTextureSources,
    textures: &mut Assets<Image>,
) -> (
    HashMap<String, BlockTextureAnimation>,
    Handle<Image>,
    UVec2,
    u32,
) {
    let mut layers = HashMap::with_capacity(sources.0.len());
    let mut source_layers = Vec::with_capacity(sources.0.len());
    let mut source_size = None;
    let mut source_format = None;

    for (path, handle) in &sources.0 {
        let image = textures
            .get(handle)
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

        let tile_size = if let Some(expected_size) = source_size {
            expected_size
        } else {
            let tile_size = UVec2::new(size.width, size.width);
            source_size = Some(tile_size);
            tile_size
        };
        assert_eq!(
            size.height % size.width,
            0,
            "animated terrain texture height must be a whole number of square frames: {path}"
        );

        if let Some(expected_format) = source_format {
            assert_eq!(
                format, expected_format,
                "all terrain texture-array layers must have the same format: {path}"
            );
        } else {
            source_format = Some(format);
        }

        let base_layer = BlockTextureLayer::new(source_layers.len() as u32);
        let frame_count = append_source_texture_frames(&mut source_layers, data, size, tile_size);
        layers.insert(
            (*path).to_owned(),
            BlockTextureAnimation::new(base_layer, frame_count),
        );
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

fn append_source_texture_frames(
    source_layers: &mut Vec<SourceTextureLayer>,
    data: &[u8],
    size: Extent3d,
    tile_size: UVec2,
) -> u32 {
    let frame_size = UVec2::splat(size.width);
    let frame_count = size.height / frame_size.y;
    let row_bytes = (frame_size.x * 4) as usize;
    let source_row_bytes = (size.width * 4) as usize;
    let frame_bytes = (frame_size.y as usize) * row_bytes;

    for frame in 0..frame_count {
        let mut frame_data = Vec::with_capacity(frame_bytes);
        for y in 0..frame_size.y {
            let source_y = frame * frame_size.y + y;
            let start = source_y as usize * source_row_bytes;
            frame_data.extend_from_slice(&data[start..start + row_bytes]);
        }

        if frame_size != tile_size {
            let alpha_mode = alpha_mode(&frame_data);
            let image = RgbaImage::from_raw(frame_size.x, frame_size.y, frame_data)
                .expect("terrain texture frame data should match RGBA8 dimensions");
            frame_data =
                resize_alpha_aware(&image, tile_size.x, tile_size.y, alpha_mode).into_raw();
        }

        source_layers.push(SourceTextureLayer {
            alpha_mode: alpha_mode(&frame_data),
            data: frame_data,
        });
    }

    frame_count
}

fn used_terrain_texture_paths() -> Vec<&'static str> {
    let mut paths = Vec::new();
    for block in BlockType::iter() {
        for side in Direction::iter() {
            let path = render_id_to_texture_path(render_id_for_block(block), side);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    for side in Direction::iter() {
        let path = render_id_to_texture_path(WATER_RENDER_ID, side);
        if !paths.contains(&path) {
            paths.push(path);
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

    #[test]
    fn vertical_texture_strip_splits_into_animation_frames() {
        let frame0 = [1u8; 2 * 2 * 4];
        let frame1 = [2u8; 2 * 2 * 4];
        let mut data = Vec::new();
        data.extend_from_slice(&frame0);
        data.extend_from_slice(&frame1);
        let mut layers = Vec::new();

        let frame_count = append_source_texture_frames(
            &mut layers,
            &data,
            Extent3d {
                width: 2,
                height: 4,
                depth_or_array_layers: 1,
            },
            UVec2::new(2, 2),
        );

        assert_eq!(frame_count, 2);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].data, frame0);
        assert_eq!(layers[1].data, frame1);
    }
}
