use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages, input::mouse::MouseWheel, prelude::*, render::render_resource::*,
};
use image::{Rgba, RgbaImage, imageops::FilterType};

use crate::quad::Direction;
use crate::{
    game_state::{InputMode, InputRouting, gameplay_input_active},
    item::{HOTBAR_SLOTS, Inventory, Item},
    player::Player,
};

use super::{
    item_stack::{stack_count_label, stack_count_text, stack_icon},
    z,
};

const HOTBAR_WIDTH: f32 = 546.0;
const HOTBAR_HEIGHT: f32 = 66.0;
const HOTBAR_SLOT_WIDTH: f32 = 60.0;
const HOTBAR_ICON_SIZE: f32 = 48.0;
const HOTBAR_SELECTION_WIDTH: f32 = 72.0;
const HOTBAR_SELECTION_HEIGHT: f32 = 69.0;
const HOTBAR_SELECTION_INSET: f32 = 3.0;
const HOTBAR_PADDING: f32 = 3.0;
const HOTBAR_COUNT_RIGHT: f32 = 4.0;
const HOTBAR_COUNT_BOTTOM: f32 = 5.0;

pub struct HotbarPlugin;

impl Plugin for HotbarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hotbar>()
            .init_resource::<ItemIcons>()
            .add_systems(
                Update,
                (
                    generate_block_icons.run_if(in_state(crate::textures::TextureState::Finished)),
                    setup_gui_textures.run_if(in_state(crate::textures::TextureState::Finished)),
                    update_hotbar_ui,
                ),
            )
            .add_systems(PreUpdate, handle_hotbar_input.after(InputRouting::Derive));
    }
}

#[derive(Resource, Default)]
pub struct Hotbar {
    pub selected: usize,
}

impl Hotbar {
    pub fn selected_stack(&self, inventory: &Inventory) -> Option<crate::item::ItemStack> {
        inventory.slot(self.selected)
    }
}

#[derive(Resource, Default)]
pub(super) struct ItemIcons {
    pub icons: HashMap<Item, Handle<Image>>,
}

#[derive(Component)]
pub(super) struct HotbarHudRoot;

#[derive(Component)]
struct HotbarIcon(usize);

#[derive(Component)]
struct HotbarCount(usize);

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

    let hotbar_bg = load_scale(
        "textures/gui/sprites/hud/hotbar.png",
        HOTBAR_WIDTH as u32,
        HOTBAR_HEIGHT as u32,
        &mut images,
    );
    let selection = load_scale(
        "textures/gui/sprites/hud/hotbar_selection.png",
        HOTBAR_SELECTION_WIDTH as u32,
        HOTBAR_SELECTION_HEIGHT as u32,
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
        .spawn((
            HotbarHudRoot,
            Pickable::IGNORE,
            GlobalZIndex(z::HUD),
            Node {
                width: Val::Vw(100.0),
                height: Val::Vh(100.0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::End,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: Val::Px(HOTBAR_WIDTH),
                        height: Val::Px(HOTBAR_HEIGHT),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|hotbar| {
                    // background
                    hotbar.spawn((
                        ImageNode::new(bg_handle.clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Px(HOTBAR_WIDTH),
                            height: Val::Px(HOTBAR_HEIGHT),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));

                    // slot row
                    hotbar
                        .spawn((
                            Node {
                                width: Val::Px(HOTBAR_WIDTH),
                                height: Val::Px(HOTBAR_HEIGHT),
                                display: Display::Flex,
                                flex_direction: FlexDirection::Row,
                                padding: UiRect::new(
                                    Val::Px(HOTBAR_PADDING),
                                    Val::Px(HOTBAR_PADDING),
                                    Val::Px(0.0),
                                    Val::Px(0.0),
                                ),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ))
                        .with_children(|row| {
                            for i in 0..HOTBAR_SLOTS {
                                row.spawn((
                                    Node {
                                        width: Val::Px(HOTBAR_SLOT_WIDTH),
                                        height: Val::Px(HOTBAR_HEIGHT),
                                        display: Display::Flex,
                                        position_type: PositionType::Relative,
                                        align_items: AlignItems::Center,
                                        justify_content: JustifyContent::Center,
                                        ..default()
                                    },
                                    Pickable::IGNORE,
                                ))
                                .with_children(|slot| {
                                    slot.spawn((
                                        ImageNode::default(),
                                        Node {
                                            width: Val::Px(HOTBAR_ICON_SIZE),
                                            height: Val::Px(HOTBAR_ICON_SIZE),
                                            ..default()
                                        },
                                        HotbarIcon(i),
                                        Pickable::IGNORE,
                                    ));
                                    slot.spawn((
                                        stack_count_text(),
                                        Node {
                                            position_type: PositionType::Absolute,
                                            right: Val::Px(HOTBAR_COUNT_RIGHT),
                                            bottom: Val::Px(HOTBAR_COUNT_BOTTOM),
                                            ..default()
                                        },
                                        HotbarCount(i),
                                        Pickable::IGNORE,
                                    ));
                                });
                            }
                        });

                    // selection highlight
                    hotbar.spawn((
                        ImageNode::new(sel_handle.clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(-HOTBAR_SELECTION_INSET),
                            top: Val::Px(-HOTBAR_SELECTION_INSET),
                            width: Val::Px(HOTBAR_SELECTION_WIDTH),
                            height: Val::Px(HOTBAR_SELECTION_HEIGHT),
                            ..default()
                        },
                        HotbarSelection,
                        Pickable::IGNORE,
                    ));
                });
        });
}

fn generate_block_icons(
    mut commands: Commands,
    item_icons: Res<ItemIcons>,
    mut images: ResMut<Assets<Image>>,
    mut generated: Local<bool>,
) {
    if *generated || !item_icons.icons.is_empty() {
        return;
    }
    *generated = true;

    let mut icons = HashMap::new();
    for item in Item::BLOCKS {
        let top_path = item.texture_path(Direction::Up);
        let side_path = item.texture_path(Direction::Right);
        let Some(top_tex) = load_block_texture(top_path) else {
            continue;
        };
        let Some(side_tex) = load_block_texture(side_path) else {
            continue;
        };

        let top_tint = item.tint(Direction::Up);
        let side_tint = item.tint(Direction::Right);
        let icon = render_isometric_block(&top_tex, &side_tex, top_tint, side_tint);
        let handle = images.add(icon);
        icons.insert(item, handle);
    }

    // Water is placeable but is not a block item, so it gets the same
    // isometric icon from its still texture with the terrain tint.
    if let Some(water_tex) = load_block_texture("textures/block/water_still.png") {
        let tint = Srgba::hex("55B8FF").unwrap().to_vec4();
        let icon = render_isometric_block(&water_tex, &water_tex, tint, tint);
        icons.insert(Item::Water, images.add(icon));
    }

    info!("Generated {} item icons", icons.len());
    commands.insert_resource(ItemIcons { icons });
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
    input_mode: Option<Res<InputMode>>,
) {
    if !gameplay_input_active(input_mode) {
        // Advance the reader so scroll input generated over the pause menu
        // cannot select a different slot immediately after resuming.
        mouse_wheel.read().for_each(drop);
        return;
    }

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
    inventory: Single<Ref<Inventory>, With<Player>>,
    item_icons: Res<ItemIcons>,
    mut selection: Query<&mut Node, With<HotbarSelection>>,
    mut icons: Query<(&HotbarIcon, &mut ImageNode)>,
    mut counts: Query<(&HotbarCount, &mut Text)>,
    mut initialized: Local<bool>,
) {
    if selection.is_empty() || icons.is_empty() || counts.is_empty() {
        return;
    }
    if *initialized && !inventory.is_changed() && !hotbar.is_changed() && !item_icons.is_changed() {
        return;
    }
    *initialized = true;

    if let Ok(mut node) = selection.single_mut() {
        let left = Val::Px(hotbar.selected as f32 * HOTBAR_SLOT_WIDTH - HOTBAR_SELECTION_INSET);
        if node.left != left {
            node.left = left;
        }
    }

    for (slot, mut image) in &mut icons {
        let target = stack_icon(&item_icons, inventory.slot(slot.0));
        if image.image != target {
            image.image = target;
        }
    }

    for (slot, mut text) in &mut counts {
        let target = stack_count_label(inventory.slot(slot.0));
        if text.0 != target {
            text.0 = target;
        }
    }
}
