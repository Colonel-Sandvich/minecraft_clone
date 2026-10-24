use bevy::{ecs::system::SystemParam, picking::hover::Hovered, ui::Pressed, window::PrimaryWindow};

use crate::{
    item::{Inventory, ItemStack},
    player::Player,
};

use super::super::{
    crosshair::CrosshairRoot,
    hotbar::{HotbarHudRoot, ItemIcons},
    item_stack::{stack_count_label, stack_icon},
};
use super::*;

type GameplayHudVisibility<'w, 's> =
    Query<'w, 's, &'static mut Visibility, Or<(With<HotbarHudRoot>, With<CrosshairRoot>)>>;

type InventorySlotHighlights<'w, 's> = Query<
    'w,
    's,
    (
        &'static InventorySlot,
        &'static Hovered,
        Has<Pressed>,
        &'static mut BackgroundColor,
        &'static mut BorderColor,
    ),
    With<InventorySlot>,
>;

pub(super) fn update_gameplay_hud_visibility(
    ui: Res<InventoryUi>,
    mut gameplay_hud: GameplayHudVisibility,
) {
    let target = if ui.open {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut visibility in &mut gameplay_hud {
        if *visibility != target {
            *visibility = target;
        }
    }
}

pub(super) fn update_inventory_slots(
    inventory: Single<Ref<Inventory>, With<Player>>,
    icons: Res<ItemIcons>,
    overlay: Query<Entity, With<InventoryOverlay>>,
    mut last_overlay: Local<Option<Entity>>,
    mut slot_icons: Query<(&InventorySlotIcon, &mut ImageNode)>,
    mut slot_counts: Query<(&InventorySlotCount, &mut Text)>,
) {
    if slot_icons.is_empty() || slot_counts.is_empty() {
        return;
    }
    let Some(overlay) = overlay.iter().next() else {
        return;
    };
    let opened = last_overlay.as_ref() != Some(&overlay);
    if !opened && !inventory.is_changed() && !icons.is_changed() {
        return;
    }
    *last_overlay = Some(overlay);

    for (slot, mut image) in &mut slot_icons {
        let target = stack_icon(&icons, inventory.slot(slot.0));
        if image.image != target {
            image.image = target;
        }
    }
    for (slot, mut text) in &mut slot_counts {
        let target = stack_count_label(inventory.slot(slot.0));
        if text.0 != target {
            text.0 = target;
        }
    }
}

pub(super) fn update_cursor_stack(
    ui: Res<InventoryUi>,
    preview: Res<DragPreview>,
    icons: Res<ItemIcons>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut root: Single<&mut Node, With<CursorStackRoot>>,
    mut image: Single<&mut ImageNode, With<CursorStackIcon>>,
    count: Single<(&mut Text, &mut TextColor), With<CursorStackCount>>,
) {
    let (mut text, mut text_color) = count.into_inner();
    let (item, label, color) = if let Some(projection) = &preview.0 {
        match projection.cursor {
            Some(stack) => (
                Some(stack.item),
                stack_count_label(Some(stack)),
                Color::WHITE,
            ),
            // Vanilla keeps a yellow "0" on the cursor when a quick-craft
            // consumes the whole carried stack.
            None => (
                ui.cursor_stack.map(|stack| stack.item),
                "0".to_string(),
                PENDING_COUNT,
            ),
        }
    } else {
        let Some(stack) = ui.cursor_stack else {
            if root.display != Display::None {
                root.display = Display::None;
            }
            return;
        };
        (
            Some(stack.item),
            stack_count_label(Some(stack)),
            Color::WHITE,
        )
    };

    let Some(item) = item else {
        if root.display != Display::None {
            root.display = Display::None;
        }
        return;
    };

    if root.display != Display::Flex {
        root.display = Display::Flex;
    }
    if let Some(position) = window.cursor_position() {
        // Cursor-following previews stay in screen pixels so they track the
        // pointer exactly; only the preview's own geometry scales in rem.
        let left = Val::Px(position.x + CURSOR_OFFSET);
        let top = Val::Px(position.y + CURSOR_OFFSET);
        if root.left != left {
            root.left = left;
        }
        if root.top != top {
            root.top = top;
        }
    }
    let target_image = icons.icons.get(&item).cloned().unwrap_or_default();
    if image.image != target_image {
        image.image = target_image;
    }
    if text.0 != label {
        text.0 = label;
    }
    if text_color.0 != color {
        text_color.0 = color;
    }
}

pub(super) fn update_slot_highlights(
    preview: Res<DragPreview>,
    mut slots: InventorySlotHighlights,
) {
    for (slot, hovered, pressed, mut background, mut border) in &mut slots {
        let pending = preview
            .0
            .as_ref()
            .is_some_and(|preview| preview.slots.iter().any(|(index, _)| *index == slot.0));
        let (target_background, target_border) = if pending {
            (SLOT_PENDING, PENDING_BORDER)
        } else if pressed {
            (SLOT_PRESSED, SLOT_BORDER)
        } else if hovered.get() {
            (SLOT_HOVER, SLOT_BORDER)
        } else {
            (SLOT_IDLE, SLOT_BORDER)
        };

        if background.0 != target_background {
            background.0 = target_background;
        }
        let target_border = BorderColor::all(target_border);
        if *border != target_border {
            *border = target_border;
        }
    }
}

#[derive(SystemParam)]
pub(super) struct DragPreviewQueries<'w, 's> {
    previews: Query<
        'w,
        's,
        (&'static InventorySlotPreview, &'static mut Visibility),
        Without<InventorySlotCount>,
    >,
    preview_icons: Query<'w, 's, (&'static InventoryPreviewIcon, &'static mut ImageNode)>,
    preview_counts: Query<
        'w,
        's,
        (
            &'static InventoryPreviewCount,
            &'static mut Text,
            &'static mut TextColor,
        ),
    >,
    real_counts: Query<
        'w,
        's,
        (&'static InventorySlotCount, &'static mut Visibility),
        Without<InventoryPreviewCount>,
    >,
}

pub(super) fn update_drag_preview(
    ui: Res<InventoryUi>,
    inventory: Single<Ref<Inventory>, With<Player>>,
    icons: Res<ItemIcons>,
    mut preview: ResMut<DragPreview>,
    mut queries: DragPreviewQueries,
) {
    // A one-slot drag releases as a plain click, which the ghost cannot
    // predict, so only multi-slot painting previews a distribution.
    let active =
        ui.drag.as_ref().is_some_and(|drag| drag.slots.len() > 1) && ui.cursor_stack.is_some();
    if !active && preview.0.is_none() {
        return;
    }
    if active
        && preview.0.is_some()
        && !ui.is_changed()
        && !inventory.is_changed()
        && !icons.is_changed()
    {
        return;
    }

    let projection = active.then(|| {
        let drag = ui.drag.as_ref().unwrap();
        let cursor = ui.cursor_stack.unwrap();
        drag_preview(&inventory, cursor, &drag.slots, drag.button)
    });
    let planned = projection
        .as_ref()
        .map(|projection| projection.slots.as_slice());

    for (slot, mut visibility) in &mut queries.previews {
        let target = if planned_stack(planned, slot.0).is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != target {
            *visibility = target;
        }
    }
    for (slot, mut image) in &mut queries.preview_icons {
        let target = stack_icon(&icons, planned_stack(planned, slot.0));
        if image.image != target {
            image.image = target;
        }
        if image.color != Color::WHITE {
            image.color = Color::WHITE;
        }
    }
    for (slot, mut text, mut color) in &mut queries.preview_counts {
        let target = planned_stack(planned, slot.0)
            .map_or(String::new(), |stack| stack_count_label(Some(stack)));
        if text.0 != target {
            text.0 = target;
        }
        if color.0 != PENDING_COUNT {
            color.0 = PENDING_COUNT;
        }
    }
    for (slot, mut visibility) in &mut queries.real_counts {
        let target = if planned_stack(planned, slot.0).is_some() {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != target {
            *visibility = target;
        }
    }

    if preview.0 != projection {
        preview.0 = projection;
    }
}

pub(super) fn planned_stack(
    planned: Option<&[(usize, ItemStack)]>,
    index: usize,
) -> Option<ItemStack> {
    planned?
        .iter()
        .find(|(slot, _)| *slot == index)
        .map(|(_, stack)| *stack)
}
