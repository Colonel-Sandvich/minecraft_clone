use bevy::{picking::hover::Hovered, ui::FocusPolicy, ui_widgets::Button};

use crate::item::HOTBAR_SLOTS;

use super::super::{
    settings::{rem, rem_font},
    z,
};
use super::*;

pub(super) fn spawn_inventory_ui(commands: &mut Commands) {
    commands
        .spawn((
            InventoryOverlay,
            DespawnOnExit(GameState::Playing),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.58)),
            GlobalZIndex(z::INVENTORY),
            FocusPolicy::Block,
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    InventoryPanel,
                    RelativeCursorPosition::default(),
                    Node {
                        width: rem(INVENTORY_PANEL_WIDTH),
                        height: rem(INVENTORY_PANEL_HEIGHT),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(rem(INVENTORY_PANEL_PADDING)),
                        row_gap: rem(SLOT_GAP),
                        border: UiRect::all(rem(INVENTORY_PANEL_BORDER)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.72, 0.72, 0.72)),
                    BorderColor::all(Color::srgb(0.2, 0.2, 0.2)),
                    FocusPolicy::Block,
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("Inventory"),
                        TextFont {
                            font_size: rem_font(24.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.15, 0.15, 0.15)),
                        Node {
                            height: rem(INVENTORY_TITLE_HEIGHT),
                            margin: UiRect::bottom(rem(INVENTORY_TITLE_MARGIN)),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));

                    for row in 0..3 {
                        spawn_slot_row(
                            panel,
                            (0..HOTBAR_SLOTS)
                                .map(|column| HOTBAR_SLOTS + row * HOTBAR_SLOTS + column),
                        );
                    }

                    panel.spawn(Node {
                        height: rem(INVENTORY_SECTION_GAP),
                        ..default()
                    });
                    spawn_slot_row(panel, 0..HOTBAR_SLOTS);
                });

            overlay
                .spawn((
                    CursorStackRoot,
                    Node {
                        position_type: PositionType::Absolute,
                        width: rem(SLOT_SIZE),
                        height: rem(SLOT_SIZE),
                        display: Display::None,
                        ..default()
                    },
                    GlobalZIndex(z::INVENTORY_CURSOR),
                    Pickable::IGNORE,
                ))
                .with_children(|cursor| {
                    cursor.spawn((
                        CursorStackIcon,
                        ImageNode::default(),
                        Node {
                            position_type: PositionType::Absolute,
                            left: rem(CURSOR_ICON_INSET),
                            top: rem(CURSOR_ICON_INSET),
                            width: rem(ITEM_ICON_SIZE),
                            height: rem(ITEM_ICON_SIZE),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    cursor.spawn((
                        CursorStackCount,
                        stack_count_text(),
                        Node {
                            position_type: PositionType::Absolute,
                            right: rem(CURSOR_COUNT_RIGHT),
                            bottom: rem(CURSOR_COUNT_BOTTOM),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                });
        });
}

fn spawn_slot_row(parent: &mut ChildSpawnerCommands, slots: impl IntoIterator<Item = usize>) {
    parent
        .spawn(Node {
            height: rem(SLOT_SIZE),
            flex_direction: FlexDirection::Row,
            column_gap: rem(SLOT_GAP),
            ..default()
        })
        .with_children(|row| {
            for index in slots {
                row.spawn((
                    InventorySlot(index),
                    RelativeCursorPosition::default(),
                    Button,
                    Hovered::default(),
                    FocusPolicy::Block,
                    Node {
                        width: rem(SLOT_SIZE),
                        height: rem(SLOT_SIZE),
                        position_type: PositionType::Relative,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(rem(SLOT_BORDER_WIDTH)),
                        ..default()
                    },
                    BackgroundColor(SLOT_IDLE),
                    BorderColor::all(SLOT_BORDER),
                ))
                .with_children(|slot| {
                    slot.spawn((
                        InventorySlotIcon(index),
                        ImageNode::default(),
                        Node {
                            width: rem(ITEM_ICON_SIZE),
                            height: rem(ITEM_ICON_SIZE),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    slot.spawn((
                        InventorySlotCount(index),
                        stack_count_text(),
                        Node {
                            position_type: PositionType::Absolute,
                            right: rem(SLOT_COUNT_RIGHT),
                            bottom: rem(SLOT_COUNT_BOTTOM),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    slot.spawn((
                        InventorySlotPreview(index),
                        Node {
                            position_type: PositionType::Absolute,
                            left: rem(0.0),
                            top: rem(0.0),
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        Visibility::Hidden,
                        Pickable::IGNORE,
                    ))
                    .with_children(|preview| {
                        preview.spawn((
                            InventoryPreviewIcon(index),
                            ImageNode::default(),
                            Node {
                                width: rem(ITEM_ICON_SIZE),
                                height: rem(ITEM_ICON_SIZE),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                        preview.spawn((
                            InventoryPreviewCount(index),
                            stack_count_text(),
                            Node {
                                position_type: PositionType::Absolute,
                                right: rem(SLOT_COUNT_RIGHT),
                                bottom: rem(SLOT_COUNT_BOTTOM),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                    });
                });
            }
        });
}
