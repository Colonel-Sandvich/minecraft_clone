use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages, input::mouse::MouseWheel, prelude::*, render::render_resource::*,
};
use image::{Rgba, RgbaImage, imageops::FilterType};
use strum::IntoEnumIterator;

use crate::block::{
    BlockType, WATER_RENDER_ID, render_id_for_block, render_id_to_colour, render_id_to_texture_path,
};
use crate::quad::Direction;
use crate::world::chunk::ChunkCell;

pub struct HotbarPlugin;

impl Plugin for HotbarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hotbar>()
            .init_resource::<BlockIcons>()
            .add_systems(
                Update,
                (
                    generate_block_icons.run_if(in_state(crate::textures::TextureState::Finished)),
                    setup_gui_textures.run_if(in_state(crate::textures::TextureState::Finished)),
                    update_hotbar_ui,
                ),
            )
            .add_systems(
                PreUpdate,
                handle_hotbar_input.after(bevy::input::InputSystems),
            );
    }
}

pub const HOTBAR_SLOTS: usize = 9;

#[derive(Resource)]
pub struct Hotbar {
    pub slots: [Option<ChunkCell>; HOTBAR_SLOTS],
    pub selected: usize,
}

impl Default for Hotbar {
    fn default() -> Self {
        Self {
            slots: [
                Some(BlockType::Dirt.into()),
                Some(BlockType::Stone.into()),
                Some(BlockType::Sand.into()),
                Some(BlockType::Glass.into()),
                Some(BlockType::OakLog.into()),
                Some(BlockType::OakLeaves.into()),
                Some(BlockType::Glowstone.into()),
                Some(ChunkCell::water_source()),
                Some(BlockType::Ice.into()),
            ],
            selected: 0,
        }
    }
}

impl Hotbar {
    pub fn selected_cell(&self) -> Option<ChunkCell> {
        self.slots[self.selected]
    }

    pub fn set_selected_cell(&mut self, cell: ChunkCell) {
        self.slots[self.selected] = cell.is_rendered().then_some(cell);
    }
}

#[derive(Resource, Default)]
pub struct BlockIcons {
    pub icons: HashMap<u16, Handle<Image>>,
}

#[derive(Component)]
struct HotbarSlot(usize);

#[derive(Component)]
struct HotbarSelection;

fn setup_gui_textures(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut spawned: Local<bool>,
) {
    if *spawned {
        return;
    }
    *spawned = true;

    let hotbar_bg = load_scale("textures/gui/sprites/hud/hotbar.png", 546, 66, &mut images);
    let selection = load_scale(
        "textures/gui/sprites/hud/hotbar_selection.png",
        72,
        69,
        &mut images,
    );

    spawn_hotbar_ui(&mut commands, &hotbar_bg, &selection);
}

fn load_scale(path: &str, w: u32, h: u32, images: &mut ResMut<Assets<Image>>) -> Handle<Image> {
    let img = image::open(format!("assets/{path}"))
        .expect("missing GUI texture asset")
        .to_rgba8();
    let scaled = image::imageops::resize(&img, w, h, FilterType::Nearest);
    images.add(Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        scaled.into_raw(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    ))
}

fn spawn_hotbar_ui(commands: &mut Commands, bg_handle: &Handle<Image>, sel_handle: &Handle<Image>) {
    commands
        .spawn(Node {
            width: Val::Vw(100.0),
            height: Val::Vh(100.0),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::End,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|parent| {
            parent
                .spawn(Node {
                    width: Val::Px(546.0),
                    height: Val::Px(66.0),
                    position_type: PositionType::Relative,
                    ..default()
                })
                .with_children(|hotbar| {
                    // background
                    hotbar.spawn((
                        ImageNode::new(bg_handle.clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Px(546.0),
                            height: Val::Px(66.0),
                            ..default()
                        },
                    ));

                    // slot row
                    hotbar
                        .spawn(Node {
                            width: Val::Px(546.0),
                            height: Val::Px(66.0),
                            display: Display::Flex,
                            flex_direction: FlexDirection::Row,
                            padding: UiRect::new(
                                Val::Px(3.0),
                                Val::Px(3.0),
                                Val::Px(0.0),
                                Val::Px(0.0),
                            ),
                            ..default()
                        })
                        .with_children(|row| {
                            for i in 0..HOTBAR_SLOTS {
                                row.spawn((
                                    Node {
                                        width: Val::Px(60.0),
                                        height: Val::Px(66.0),
                                        display: Display::Flex,
                                        align_items: AlignItems::Center,
                                        justify_content: JustifyContent::Center,
                                        ..default()
                                    },
                                    HotbarSlot(i),
                                ))
                                .with_child((
                                    ImageNode::default(),
                                    Node {
                                        width: Val::Px(48.0),
                                        height: Val::Px(48.0),
                                        ..default()
                                    },
                                ));
                            }
                        });

                    // selection highlight
                    hotbar.spawn((
                        ImageNode::new(sel_handle.clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(-3.0),
                            top: Val::Px(-3.0),
                            width: Val::Px(72.0),
                            height: Val::Px(69.0),
                            ..default()
                        },
                        HotbarSelection,
                    ));
                });
        });
}

fn generate_block_icons(
    mut commands: Commands,
    block_icons: Res<BlockIcons>,
    mut images: ResMut<Assets<Image>>,
) {
    if !block_icons.icons.is_empty() {
        return;
    }

    let mut icons = HashMap::new();
    for block in BlockType::iter() {
        let rid = render_id_for_block(block);
        let top_path = render_id_to_texture_path(rid, Direction::Up);
        let side_path = render_id_to_texture_path(rid, Direction::Right);
        let Some(top_tex) = load_block_texture(top_path) else {
            continue;
        };
        let Some(side_tex) = load_block_texture(side_path) else {
            continue;
        };

        let top_tint = render_id_to_colour(rid, Direction::Up);
        let side_tint = render_id_to_colour(rid, Direction::Right);
        let icon = render_isometric_block(&top_tex, &side_tex, top_tint, side_tint);
        let handle = images.add(icon);
        icons.insert(rid, handle);
    }

    let rid = WATER_RENDER_ID;
    let Some(top_tex) = load_block_texture(render_id_to_texture_path(rid, Direction::Up)) else {
        commands.insert_resource(BlockIcons { icons });
        return;
    };
    let Some(side_tex) = load_block_texture(render_id_to_texture_path(rid, Direction::Right))
    else {
        commands.insert_resource(BlockIcons { icons });
        return;
    };
    let icon = render_isometric_block(
        &top_tex,
        &side_tex,
        render_id_to_colour(rid, Direction::Up),
        render_id_to_colour(rid, Direction::Right),
    );
    icons.insert(rid, images.add(icon));

    info!("Generated {} block icons", icons.len());
    commands.insert_resource(BlockIcons { icons });
}

const ICON_W: u32 = 48;
const ICON_H: u32 = 48;

static TOP_FACE: [(f32, f32); 4] = [(24.0, 4.5), (42.0, 13.5), (24.0, 22.5), (6.0, 13.5)];
static LEFT_FACE: [(f32, f32); 4] = [(6.0, 13.5), (24.0, 22.5), (24.0, 43.5), (6.0, 34.5)];
static RIGHT_FACE: [(f32, f32); 4] = [(24.0, 22.5), (42.0, 13.5), (42.0, 34.5), (24.0, 43.5)];

static TOP_UV: [(f32, f32); 4] = [(1.0, 1.0), (1.0, 0.0), (0.0, 0.0), (0.0, 1.0)];
static SIDE_UV: [(f32, f32); 4] = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];

fn load_block_texture(path: &str) -> Option<RgbaImage> {
    match image::open(format!("assets/{path}")) {
        Ok(image) => Some(first_animation_frame(image.to_rgba8())),
        Err(error) => {
            warn!("failed to load block icon texture {path}: {error}");
            None
        }
    }
}

fn first_animation_frame(image: RgbaImage) -> RgbaImage {
    if image.height() > image.width() && image.height() % image.width() == 0 {
        image::imageops::crop_imm(&image, 0, 0, image.width(), image.width()).to_image()
    } else {
        image
    }
}

fn render_isometric_block(
    top_tex: &RgbaImage,
    side_tex: &RgbaImage,
    top_tint: Vec4,
    side_tint: Vec4,
) -> Image {
    let mut canvas = RgbaImage::new(ICON_W, ICON_H);

    warp_face(&mut canvas, side_tex, &LEFT_FACE, &SIDE_UV, side_tint, 0.8);
    warp_face(&mut canvas, side_tex, &RIGHT_FACE, &SIDE_UV, side_tint, 0.6);
    warp_face(&mut canvas, top_tex, &TOP_FACE, &TOP_UV, top_tint, 1.0);

    Image::new(
        Extent3d {
            width: ICON_W,
            height: ICON_H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        canvas.into_raw(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

fn warp_face(
    canvas: &mut RgbaImage,
    src: &RgbaImage,
    quad: &[(f32, f32); 4],
    uv: &[(f32, f32); 4],
    tint: Vec4,
    brightness: f32,
) {
    let (v0x, v0y) = quad[0];

    let a00 = quad[1].0 - v0x;
    let a10 = quad[1].1 - v0y;
    let a01 = quad[3].0 - v0x;
    let a11 = quad[3].1 - v0y;

    let det = a00 * a11 - a01 * a10;
    if det.abs() < 1e-6 {
        return;
    }

    let inv00 = a11 / det;
    let inv01 = -a01 / det;
    let inv10 = -a10 / det;
    let inv11 = a00 / det;

    let min_x = (quad.iter().map(|v| v.0).fold(f32::MAX, f32::min) - 1.0).floor() as i32;
    let max_x = (quad.iter().map(|v| v.0).fold(f32::MIN, f32::max) + 1.0).ceil() as i32;
    let min_y = (quad.iter().map(|v| v.1).fold(f32::MAX, f32::min) - 1.0).floor() as i32;
    let max_y = (quad.iter().map(|v| v.1).fold(f32::MIN, f32::max) + 1.0).ceil() as i32;

    let cw = canvas.width() as i32;
    let ch = canvas.height() as i32;
    let sw = src.width().saturating_sub(1);
    let sh = src.height().saturating_sub(1);

    for y in min_y.max(0)..=max_y.min(ch - 1) {
        for x in min_x.max(0)..=max_x.min(cw - 1) {
            let dx = x as f32 - v0x;
            let dy = y as f32 - v0y;

            let u = inv00 * dx + inv01 * dy;
            let v = inv10 * dx + inv11 * dy;

            if u < 0.0 || u > 1.0 || v < 0.0 || v > 1.0 {
                continue;
            }

            let tex_u = (1.0 - u) * (1.0 - v) * uv[0].0
                + u * (1.0 - v) * uv[1].0
                + u * v * uv[2].0
                + (1.0 - u) * v * uv[3].0;
            let tex_v = (1.0 - u) * (1.0 - v) * uv[0].1
                + u * (1.0 - v) * uv[1].1
                + u * v * uv[2].1
                + (1.0 - u) * v * uv[3].1;

            let sx = (tex_u * sw as f32).round() as u32;
            let sy = (tex_v * sh as f32).round() as u32;
            let p = src.get_pixel(sx.min(sw), sy.min(sh));
            if p[3] == 0 {
                continue;
            }

            let r = (p[0] as f32 * tint.x * brightness).min(255.0) as u8;
            let g = (p[1] as f32 * tint.y * brightness).min(255.0) as u8;
            let b = (p[2] as f32 * tint.z * brightness).min(255.0) as u8;

            canvas.put_pixel(x as u32, y as u32, Rgba([r, g, b, p[3]]));
        }
    }
}

fn handle_hotbar_input(
    mut hotbar: ResMut<Hotbar>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut mouse_wheel: MessageReader<MouseWheel>,
) {
    for i in 0..HOTBAR_SLOTS {
        let key = match i {
            0 => KeyCode::Digit1,
            1 => KeyCode::Digit2,
            2 => KeyCode::Digit3,
            3 => KeyCode::Digit4,
            4 => KeyCode::Digit5,
            5 => KeyCode::Digit6,
            6 => KeyCode::Digit7,
            7 => KeyCode::Digit8,
            8 => KeyCode::Digit9,
            _ => unreachable!(),
        };
        if keyboard.just_pressed(key) {
            hotbar.selected = i;
        }
    }

    for event in mouse_wheel.read() {
        if event.y > 0.0 {
            hotbar.selected = (hotbar.selected + HOTBAR_SLOTS - 1) % HOTBAR_SLOTS;
        } else if event.y < 0.0 {
            hotbar.selected = (hotbar.selected + 1) % HOTBAR_SLOTS;
        }
    }
}

fn update_hotbar_ui(
    hotbar: Res<Hotbar>,
    block_icons: Res<BlockIcons>,
    mut selection: Query<&mut Node, With<HotbarSelection>>,
    slot_children: Query<(&HotbarSlot, &Children)>,
    mut images: Query<&mut ImageNode>,
) {
    if let Ok(mut node) = selection.single_mut() {
        node.left = Val::Px(hotbar.selected as f32 * 60.0 - 3.0);
    }

    for (slot, children) in &slot_children {
        for child in children.iter() {
            if let Ok(mut img) = images.get_mut(child)
                && let Some(cell) = hotbar.slots[slot.0]
                && let Some(handle) = block_icons.icons.get(&cell.kind())
            {
                img.image = handle.clone();
            }
        }
    }
}
