use std::collections::VecDeque;

use bevy::{
    prelude::*,
    ui::{RelativeCursorPosition, UiSystems},
};

use crate::{
    game_state::{GameState, InputRouting},
    item::{DropItemRequest, Inventory, Item, ItemStack},
    player::{
        Player,
        cam::{MouseCam, MouseState},
        control::KeyBindings,
    },
};

use super::item_stack::stack_count_text;

pub(super) struct InventoryUiPlugin;

impl Plugin for InventoryUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InventoryUi>()
            .init_resource::<DragPreview>()
            .add_observer(return_carried_stack)
            .add_systems(
                PreUpdate,
                (toggle_inventory_ui, handle_inventory_ui_input)
                    .chain()
                    .after(UiSystems::Focus)
                    .after(InputRouting::Route)
                    .before(InputRouting::Derive),
            )
            .add_systems(
                FixedPreUpdate,
                dispatch_inventory_drop_requests.run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                (
                    update_gameplay_hud_visibility,
                    (
                        update_drag_preview,
                        update_inventory_slots,
                        update_slot_highlights,
                        update_cursor_stack,
                    )
                        .chain()
                        .run_if(inventory_ui_open),
                ),
            );
    }
}

#[derive(Resource, Default)]
pub(crate) struct InventoryUi {
    open: bool,
    close_requested: bool,
    cursor_stack: Option<ItemStack>,
    drag: Option<StackDrag>,
    last_left_click: Option<(usize, Item, f64)>,
    pending_drops: VecDeque<PendingInventoryDrop>,
}

impl InventoryUi {
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Asks the next inventory update to close, keeping teardown on the
    /// inventory side while the back router stays ignorant of item state.
    pub(crate) fn request_close(&mut self) {
        self.close_requested = true;
    }

    fn take_close_request(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }
}

#[derive(Clone, Debug)]
struct StackDrag {
    button: MouseButton,
    origin: usize,
    slots: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DragProjection {
    slots: Vec<(usize, ItemStack)>,
    cursor: Option<ItemStack>,
}

#[derive(Resource, Default)]
struct DragPreview(Option<DragProjection>);

#[derive(Clone, Copy, Debug)]
struct PendingInventoryDrop {
    stack: ItemStack,
    inventory_slot: Option<usize>,
}

#[derive(Component)]
struct InventoryOverlay;

#[derive(Component)]
struct InventoryPanel;

#[derive(Component)]
struct InventorySlot(usize);

#[derive(Component)]
struct InventorySlotIcon(usize);

#[derive(Component)]
struct InventorySlotCount(usize);

#[derive(Component)]
struct InventorySlotPreview(usize);

#[derive(Component)]
struct InventoryPreviewIcon(usize);

#[derive(Component)]
struct InventoryPreviewCount(usize);

#[derive(Component)]
struct CursorStackRoot;

#[derive(Component)]
struct CursorStackIcon;

#[derive(Component)]
struct CursorStackCount;

const INVENTORY_PANEL_WIDTH: f32 = 602.0;
const INVENTORY_PANEL_HEIGHT: f32 = 348.0;
const INVENTORY_PANEL_PADDING: f32 = 28.0;
const INVENTORY_PANEL_BORDER: f32 = 4.0;
const INVENTORY_TITLE_HEIGHT: f32 = 30.0;
const INVENTORY_TITLE_MARGIN: f32 = 4.0;
const INVENTORY_SECTION_GAP: f32 = 18.0;
const SLOT_SIZE: f32 = 54.0;
const ITEM_ICON_SIZE: f32 = 44.0;
const SLOT_GAP: f32 = 4.0;
const SLOT_BORDER_WIDTH: f32 = 2.0;
const SLOT_COUNT_RIGHT: f32 = 2.0;
const SLOT_COUNT_BOTTOM: f32 = 1.0;
const CURSOR_OFFSET: f32 = 8.0;
const CURSOR_ICON_INSET: f32 = 5.0;
const CURSOR_COUNT_RIGHT: f32 = 1.0;
const CURSOR_COUNT_BOTTOM: f32 = 1.0;
const SLOT_IDLE: Color = Color::srgb(0.32, 0.32, 0.32);
const SLOT_HOVER: Color = Color::srgb(0.48, 0.48, 0.48);
const SLOT_PRESSED: Color = Color::srgb(0.62, 0.62, 0.62);
/// A pending drag is highlighted on the slot itself, behind its contents, so
/// it cannot be confused with the committed hover/pressed backgrounds.
const SLOT_PENDING: Color = Color::srgb(0.58, 0.58, 0.58);
const SLOT_BORDER: Color = Color::srgb(0.12, 0.12, 0.12);
const PENDING_BORDER: Color = Color::srgb(1.0, 0.82, 0.25);
const PENDING_COUNT: Color = Color::srgb(1.0, 0.85, 0.2);

/// Spawns the panel. The overlay's lifetime is the open state: it is spawned
/// by the toggle and despawned on close (or by [`DespawnOnExit`] when gameplay
/// ends), with [`return_carried_stack`] handling the teardown.
///
/// The actual spawn functions live in [`spawn`].
mod spawn;
#[cfg(test)]
mod tests;
mod visuals;

use spawn::spawn_inventory_ui;
use visuals::{
    update_cursor_stack, update_drag_preview, update_gameplay_hud_visibility,
    update_inventory_slots, update_slot_highlights,
};

fn toggle_inventory_ui(
    mut commands: Commands,
    keyboard: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    game_state: Res<State<GameState>>,
    mut next_mouse_state: ResMut<NextState<MouseState>>,
    mut ui: ResMut<InventoryUi>,
    overlay: Query<Entity, With<InventoryOverlay>>,
) {
    if ui.take_close_request() || (ui.is_open() && keyboard.just_pressed(bindings.open_inventory)) {
        close_inventory(&mut ui, &mut commands, &overlay);
        next_mouse_state.set(MouseState::Grabbed);
        return;
    }

    if !ui.is_open()
        && *game_state.get() == GameState::Playing
        && keyboard.just_pressed(bindings.open_inventory)
    {
        ui.open = true;
        spawn_inventory_ui(&mut commands);
        next_mouse_state.set(MouseState::Free);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_inventory_ui_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    mouse: Res<ButtonInput<MouseButton>>,
    real_time: Res<Time<Real>>,
    mut ui: ResMut<InventoryUi>,
    mut inventory: Single<&mut Inventory, With<Player>>,
    slots: Query<(&InventorySlot, &RelativeCursorPosition)>,
    panel: Single<&RelativeCursorPosition, With<InventoryPanel>>,
) {
    if !ui.open {
        return;
    }

    // `Pressed` deliberately remains on the press origin while the mouse is
    // held. Dragging must instead follow the physical cursor.
    let hovered = hovered_inventory_slot(&slots);
    let whole_stack_modifier = whole_stack_drop_modifier_pressed(&keyboard);

    if keyboard.just_pressed(bindings.drop_item) {
        // The payload is about to change underneath the drag; dropping out of
        // a drag is a cancel, not a click on release.
        ui.drag = None;
        if let Some(stack) = ui.cursor_stack {
            let count = if whole_stack_modifier {
                stack.count()
            } else {
                1
            };
            if let Some(dropped) = take_from_stack(&mut ui.cursor_stack, count) {
                queue_owned_drop(&mut ui, dropped);
            }
        } else if let Some(index) = hovered
            && let Some(stack) = inventory.slot(index)
        {
            let count = if whole_stack_modifier {
                stack.count()
            } else {
                1
            };
            ui.pending_drops.push_back(PendingInventoryDrop {
                stack: ItemStack::try_new(stack.item, count).unwrap(),
                inventory_slot: Some(index),
            });
        }
        return;
    }

    if ui.cursor_stack.is_none()
        && let Some(index) = hovered
        && let Some(hotbar_slot) = pressed_number_slot(&keyboard)
    {
        inventory.swap_slots(index, hotbar_slot);
        return;
    }

    if mouse.just_pressed(MouseButton::Left)
        && let Some(held) = ui.cursor_stack
        && let Some(index) = hovered
        && ui
            .last_left_click
            .is_some_and(|(last_slot, item, clicked_at)| {
                last_slot == index
                    && item == held.item
                    && real_time.elapsed_secs_f64() - clicked_at <= 0.25
            })
    {
        collect_matching_stacks(&mut inventory, &mut ui.cursor_stack);
        ui.last_left_click = None;
        return;
    }

    for button in [MouseButton::Left, MouseButton::Right] {
        if mouse.just_pressed(button) && ui.drag.is_none() {
            match hovered {
                Some(index)
                    if button == MouseButton::Left
                        && shift_pressed(&keyboard)
                        && ui.cursor_stack.is_none() =>
                {
                    inventory.quick_move(index);
                }
                Some(index) if ui.cursor_stack.is_some() => {
                    ui.drag = Some(StackDrag {
                        button,
                        origin: index,
                        slots: Vec::new(),
                    });
                }
                Some(index) => {
                    click_slot(&mut inventory, index, &mut ui.cursor_stack, button);
                    if button == MouseButton::Left {
                        ui.last_left_click = ui
                            .cursor_stack
                            .map(|stack| (index, stack.item, real_time.elapsed_secs_f64()));
                    }
                }
                None if !panel.cursor_over() => {
                    let count = if button == MouseButton::Left {
                        ui.cursor_stack.map_or(0, ItemStack::count)
                    } else {
                        1
                    };
                    if let Some(dropped) = take_from_stack(&mut ui.cursor_stack, count) {
                        queue_owned_drop(&mut ui, dropped);
                    }
                }
                None => {}
            }
        }
    }

    let held = ui.cursor_stack;
    if let Some(drag) = &mut ui.drag
        && mouse.pressed(drag.button)
        && let Some(index) = hovered
        && index != drag.origin
        && !drag.slots.contains(&index)
        && let Some(held) = held
        && held.count() > drag.slots.len() as u16
        && slot_accepts(inventory.slot(index), held.item)
    {
        drag.slots.push(index);
    }

    if let Some(drag) = ui.drag.take_if(|drag| mouse.just_released(drag.button)) {
        if ui.cursor_stack.is_none() {
            // The carried stack was dropped (or otherwise cleared) during the
            // drag, so there is no click or distribution left to perform.
            ui.last_left_click = None;
            return;
        }

        if drag.slots.len() > 1 {
            distribute_drag(
                &mut inventory,
                &mut ui.cursor_stack,
                &drag.slots,
                drag.button,
            );
        } else if let Some(index) = drag.slots.first().copied() {
            // A single visited target behaves like a click, matching
            // Minecraft's quick-craft release for one slot.
            click_slot(&mut inventory, index, &mut ui.cursor_stack, drag.button);
            if drag.button == MouseButton::Left {
                ui.last_left_click = ui
                    .cursor_stack
                    .map(|stack| (index, stack.item, real_time.elapsed_secs_f64()));
            }
        } else {
            match hovered {
                Some(index) => {
                    click_slot(&mut inventory, index, &mut ui.cursor_stack, drag.button);
                    if drag.button == MouseButton::Left {
                        ui.last_left_click = ui
                            .cursor_stack
                            .map(|stack| (index, stack.item, real_time.elapsed_secs_f64()));
                    }
                }
                None if !panel.cursor_over() => {
                    let count = if drag.button == MouseButton::Left {
                        ui.cursor_stack.map_or(0, ItemStack::count)
                    } else {
                        1
                    };
                    if let Some(dropped) = take_from_stack(&mut ui.cursor_stack, count) {
                        queue_owned_drop(&mut ui, dropped);
                    }
                }
                None => {}
            }
        }
    }
}

fn hovered_inventory_slot(
    slots: &Query<(&InventorySlot, &RelativeCursorPosition)>,
) -> Option<usize> {
    slots
        .iter()
        .find_map(|(slot, cursor)| cursor.cursor_over().then_some(slot.0))
}

fn shift_pressed(keyboard: &ButtonInput<KeyCode>) -> bool {
    keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])
}

fn whole_stack_drop_modifier_pressed(keyboard: &ButtonInput<KeyCode>) -> bool {
    keyboard.any_pressed([
        KeyCode::ShiftLeft,
        KeyCode::ShiftRight,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
    ])
}

fn pressed_number_slot(keyboard: &ButtonInput<KeyCode>) -> Option<usize> {
    [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ]
    .into_iter()
    .position(|key| keyboard.just_pressed(key))
}

fn click_slot(
    inventory: &mut Inventory,
    index: usize,
    cursor: &mut Option<ItemStack>,
    button: MouseButton,
) {
    match button {
        MouseButton::Left => left_click_slot(inventory, index, cursor),
        MouseButton::Right => right_click_slot(inventory, index, cursor),
        _ => {}
    }
}

fn left_click_slot(inventory: &mut Inventory, index: usize, cursor: &mut Option<ItemStack>) {
    let slot = inventory.slot(index);
    match (*cursor, slot) {
        (None, _) => {
            *cursor = slot;
            inventory.set_slot(index, None);
        }
        (Some(held), None) => {
            inventory.set_slot(index, Some(held));
            *cursor = None;
        }
        (Some(held), Some(target)) if held.item == target.item => {
            let moved = held.count().min(ItemStack::MAX_COUNT - target.count());
            inventory.set_slot(
                index,
                ItemStack::try_new(target.item, target.count() + moved),
            );
            *cursor = ItemStack::try_new(held.item, held.count() - moved);
        }
        (Some(held), Some(target)) => {
            inventory.set_slot(index, Some(held));
            *cursor = Some(target);
        }
    }
}

fn right_click_slot(inventory: &mut Inventory, index: usize, cursor: &mut Option<ItemStack>) {
    let slot = inventory.slot(index);
    match (*cursor, slot) {
        (None, Some(stack)) => {
            let picked_up = stack.count().div_ceil(2);
            *cursor = ItemStack::try_new(stack.item, picked_up);
            inventory.set_slot(
                index,
                ItemStack::try_new(stack.item, stack.count() - picked_up),
            );
        }
        (None, None) => {}
        (Some(held), None) => {
            inventory.set_slot(index, Some(ItemStack::one(held.item)));
            *cursor = ItemStack::try_new(held.item, held.count() - 1);
        }
        (Some(held), Some(target))
            if held.item == target.item && target.count() < ItemStack::MAX_COUNT =>
        {
            inventory.set_slot(index, ItemStack::try_new(target.item, target.count() + 1));
            *cursor = ItemStack::try_new(held.item, held.count() - 1);
        }
        (Some(held), Some(target))
            if held.item == target.item && target.count() == ItemStack::MAX_COUNT => {}
        (Some(held), Some(target)) => {
            inventory.set_slot(index, Some(held));
            *cursor = Some(target);
        }
    }
}

fn distribute_drag(
    inventory: &mut Inventory,
    cursor: &mut Option<ItemStack>,
    slots: &[usize],
    button: MouseButton,
) {
    let Some(held) = *cursor else {
        return;
    };
    let accepted: Vec<usize> = slots
        .iter()
        .copied()
        .filter(|&index| slot_accepts(inventory.slot(index), held.item))
        .collect();
    if accepted.is_empty() {
        return;
    }

    // Minecraft's left-drag divides the carried stack evenly over the
    // dragged-over slots, capped by each slot's remaining capacity. Anything
    // that does not fit (including the division remainder) stays on the
    // cursor. Right-drag places one item per slot.
    let per_slot = match button {
        MouseButton::Left => held.count() / accepted.len() as u16,
        MouseButton::Right => 1,
        _ => return,
    };
    if per_slot == 0 {
        return;
    }

    for index in accepted {
        let Some(held) = *cursor else {
            break;
        };
        let existing = inventory.slot(index).map_or(0, ItemStack::count);
        let target = (existing + per_slot).min(ItemStack::MAX_COUNT);
        let moved = (target - existing).min(held.count());
        if moved == 0 {
            continue;
        }

        inventory.set_slot(index, ItemStack::try_new(held.item, existing + moved));
        *cursor = ItemStack::try_new(held.item, held.count() - moved);
    }
}

/// The slots a drag would change and the stack each would end up holding.
///
/// Runs the release-time distribution against a copy of the inventory so the
/// preview cannot drift from what releasing the drag actually does.
fn drag_preview(
    inventory: &Inventory,
    cursor: ItemStack,
    slots: &[usize],
    button: MouseButton,
) -> DragProjection {
    let mut projected = inventory.clone();
    let mut projected_cursor = Some(cursor);
    distribute_drag(&mut projected, &mut projected_cursor, slots, button);

    let slots = slots
        .iter()
        .filter_map(|&index| {
            let after = projected.slot(index)?;
            (inventory.slot(index) != Some(after)).then_some((index, after))
        })
        .collect();

    DragProjection {
        slots,
        cursor: projected_cursor,
    }
}

fn slot_accepts(slot: Option<ItemStack>, item: crate::item::Item) -> bool {
    slot.is_none_or(|stack| stack.item == item && stack.count() < ItemStack::MAX_COUNT)
}

fn take_from_stack(stack: &mut Option<ItemStack>, count: u16) -> Option<ItemStack> {
    let current = (*stack)?;
    let removed = count.min(current.count());
    *stack = ItemStack::try_new(current.item, current.count() - removed);
    ItemStack::try_new(current.item, removed)
}

fn collect_matching_stacks(inventory: &mut Inventory, cursor: &mut Option<ItemStack>) {
    let Some(mut held) = *cursor else {
        return;
    };

    for index in 0..crate::item::INVENTORY_SLOTS {
        if held.count() == ItemStack::MAX_COUNT {
            break;
        }
        let Some(stack) = inventory.slot(index) else {
            continue;
        };
        if stack.item != held.item {
            continue;
        }

        let capacity = ItemStack::MAX_COUNT - held.count();
        let Some(taken) = inventory.take(index, capacity) else {
            continue;
        };
        held = ItemStack::try_new(held.item, held.count() + taken.count()).unwrap();
    }
    *cursor = Some(held);
}

fn queue_owned_drop(ui: &mut InventoryUi, stack: ItemStack) {
    ui.pending_drops.push_back(PendingInventoryDrop {
        stack,
        inventory_slot: None,
    });
}

fn dispatch_inventory_drop_requests(
    mut ui: ResMut<InventoryUi>,
    camera: Single<&Transform, With<MouseCam>>,
    mut drops: MessageWriter<DropItemRequest>,
) {
    for pending in ui.pending_drops.drain(..) {
        drops.write(DropItemRequest {
            stack: pending.stack,
            inventory_slot: pending.inventory_slot,
            look_direction: *camera.forward(),
        });
    }
}

/// Closes the panel by despawning its overlay.
///
/// `open` flips immediately so the rest of this frame's input routing sees the
/// close; [`return_carried_stack`] performs the item cleanup at command flush.
fn close_inventory(
    ui: &mut InventoryUi,
    commands: &mut Commands,
    overlay: &Query<Entity, With<InventoryOverlay>>,
) {
    ui.open = false;
    for entity in overlay {
        commands.entity(entity).despawn();
    }
}

/// Returns the carried stack when the overlay goes away.
///
/// The overlay only exists while the inventory is open, so any despawn —
/// toggling closed or leaving gameplay via `DespawnOnExit` — funnels through
/// this one teardown, mirroring the pause menu's lifecycle.
fn return_carried_stack(
    _: On<Despawn<InventoryOverlay>>,
    mut ui: ResMut<InventoryUi>,
    mut inventories: Query<&mut Inventory, With<Player>>,
) {
    if let Ok(mut inventory) = inventories.single_mut()
        && let Some(stack) = ui.cursor_stack.take()
        && let Some(remainder) = inventory.insert(stack)
    {
        queue_owned_drop(&mut ui, remainder);
    }
    ui.open = false;
    ui.drag = None;
    ui.last_left_click = None;
}

fn inventory_ui_open(ui: Res<InventoryUi>) -> bool {
    ui.is_open()
}
