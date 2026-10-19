use std::collections::VecDeque;

use bevy::{
    picking::hover::Hovered,
    prelude::*,
    ui::{FocusPolicy, Pressed, RelativeCursorPosition, UiSystems},
    ui_widgets::Button,
    window::PrimaryWindow,
};

use crate::{
    game_state::{GameState, InputRouting},
    item::{DropItemRequest, HOTBAR_SLOTS, Inventory, Item, ItemStack},
    player::{
        Player,
        cam::{MouseCam, MouseState},
        control::KeyBindings,
    },
};

use super::{
    crosshair::CrosshairRoot,
    hotbar::{HotbarHudRoot, ItemIcons},
    item_stack::{stack_count_label, stack_count_text, stack_icon},
};

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

const SLOT_SIZE: f32 = 54.0;
const ITEM_ICON_SIZE: f32 = 44.0;
const SLOT_GAP: f32 = 4.0;
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
fn spawn_inventory_ui(commands: &mut Commands) {
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
            GlobalZIndex(2_000),
            FocusPolicy::Block,
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    InventoryPanel,
                    RelativeCursorPosition::default(),
                    Node {
                        width: Val::Px(602.0),
                        height: Val::Px(348.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(28.0)),
                        row_gap: Val::Px(SLOT_GAP),
                        border: UiRect::all(Val::Px(4.0)),
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
                            font_size: FontSize::Px(24.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.15, 0.15, 0.15)),
                        Node {
                            height: Val::Px(30.0),
                            margin: UiRect::bottom(Val::Px(4.0)),
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
                        height: Val::Px(18.0),
                        ..default()
                    });
                    spawn_slot_row(panel, 0..HOTBAR_SLOTS);
                });

            overlay
                .spawn((
                    CursorStackRoot,
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(SLOT_SIZE),
                        height: Val::Px(SLOT_SIZE),
                        display: Display::None,
                        ..default()
                    },
                    GlobalZIndex(2_100),
                    Pickable::IGNORE,
                ))
                .with_children(|cursor| {
                    cursor.spawn((
                        CursorStackIcon,
                        ImageNode::default(),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(5.0),
                            top: Val::Px(5.0),
                            width: Val::Px(ITEM_ICON_SIZE),
                            height: Val::Px(ITEM_ICON_SIZE),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    cursor.spawn((
                        CursorStackCount,
                        stack_count_text(),
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(1.0),
                            bottom: Val::Px(1.0),
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
            height: Val::Px(SLOT_SIZE),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(SLOT_GAP),
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
                        width: Val::Px(SLOT_SIZE),
                        height: Val::Px(SLOT_SIZE),
                        position_type: PositionType::Relative,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(Val::Px(2.0)),
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
                            width: Val::Px(ITEM_ICON_SIZE),
                            height: Val::Px(ITEM_ICON_SIZE),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    slot.spawn((
                        InventorySlotCount(index),
                        stack_count_text(),
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(2.0),
                            bottom: Val::Px(1.0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    slot.spawn((
                        InventorySlotPreview(index),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
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
                                width: Val::Px(ITEM_ICON_SIZE),
                                height: Val::Px(ITEM_ICON_SIZE),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                        preview.spawn((
                            InventoryPreviewCount(index),
                            stack_count_text(),
                            Node {
                                position_type: PositionType::Absolute,
                                right: Val::Px(2.0),
                                bottom: Val::Px(1.0),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                    });
                });
            }
        });
}

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
                        slots: vec![index],
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

    if let Some(drag) = &mut ui.drag
        && mouse.pressed(drag.button)
        && let Some(index) = hovered
        && !drag.slots.contains(&index)
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

        if drag.slots.len() == 1 {
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
        } else {
            distribute_drag(
                &mut inventory,
                &mut ui.cursor_stack,
                &drag.slots,
                drag.button,
            );
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

type GameplayHudVisibility<'w, 's> =
    Query<'w, 's, &'static mut Visibility, Or<(With<HotbarHudRoot>, With<CrosshairRoot>)>>;

fn update_gameplay_hud_visibility(ui: Res<InventoryUi>, mut gameplay_hud: GameplayHudVisibility) {
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

fn update_inventory_slots(
    inventory: Single<&Inventory, With<Player>>,
    icons: Res<ItemIcons>,
    mut slot_icons: Query<(&InventorySlotIcon, &mut ImageNode)>,
    mut slot_counts: Query<(&InventorySlotCount, &mut Text)>,
) {
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

fn update_cursor_stack(
    ui: Res<InventoryUi>,
    preview: Res<DragPreview>,
    icons: Res<ItemIcons>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut root: Single<&mut Node, With<CursorStackRoot>>,
    mut image: Single<&mut ImageNode, With<CursorStackIcon>>,
    mut count: Single<&mut Text, With<CursorStackCount>>,
    mut count_color: Single<&mut TextColor, With<CursorStackCount>>,
) {
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
        let left = Val::Px(position.x + 8.0);
        let top = Val::Px(position.y + 8.0);
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
    if count.0 != label {
        count.0 = label;
    }
    if count_color.0 != color {
        count_color.0 = color;
    }
}

fn update_slot_highlights(
    preview: Res<DragPreview>,
    mut slots: Query<
        (
            &InventorySlot,
            &Hovered,
            Has<Pressed>,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<InventorySlot>,
    >,
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
        if border.top != target_border {
            border.set_all(target_border);
        }
    }
}

fn update_drag_preview(
    ui: Res<InventoryUi>,
    inventory: Single<&Inventory, With<Player>>,
    icons: Res<ItemIcons>,
    mut preview: ResMut<DragPreview>,
    mut previews: Query<(&InventorySlotPreview, &mut Visibility), Without<InventorySlotCount>>,
    mut preview_icons: Query<(&InventoryPreviewIcon, &mut ImageNode)>,
    mut preview_counts: Query<(&InventoryPreviewCount, &mut Text, &mut TextColor)>,
    mut real_counts: Query<(&InventorySlotCount, &mut Visibility), Without<InventoryPreviewCount>>,
) {
    // A one-slot drag releases as a plain click, which the ghost cannot
    // predict, so only multi-slot painting previews a distribution.
    let active =
        ui.drag.as_ref().is_some_and(|drag| drag.slots.len() > 1) && ui.cursor_stack.is_some();
    if !active && preview.0.is_none() {
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

    for (slot, mut visibility) in &mut previews {
        let target = if planned_stack(planned, slot.0).is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != target {
            *visibility = target;
        }
    }
    for (slot, mut image) in &mut preview_icons {
        let target = stack_icon(&icons, planned_stack(planned, slot.0));
        if image.image != target {
            image.image = target;
        }
        if image.color != Color::WHITE {
            image.color = Color::WHITE;
        }
    }
    for (slot, mut text, mut color) in &mut preview_counts {
        let target = planned_stack(planned, slot.0)
            .map_or(String::new(), |stack| stack_count_label(Some(stack)));
        if text.0 != target {
            text.0 = target;
        }
        if color.0 != PENDING_COUNT {
            color.0 = PENDING_COUNT;
        }
    }
    for (slot, mut visibility) in &mut real_counts {
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

fn planned_stack(planned: Option<&[(usize, ItemStack)]>, index: usize) -> Option<ItemStack> {
    planned?
        .iter()
        .find(|(slot, _)| *slot == index)
        .map(|(_, stack)| *stack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Item;
    use bevy::state::app::StatesPlugin;

    #[test]
    fn left_click_picks_up_merges_and_swaps_stacks() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, ItemStack::try_new(Item::Dirt, 40));
        inventory.set_slot(1, ItemStack::try_new(Item::Dirt, 30));
        inventory.set_slot(2, Some(ItemStack::one(Item::Stone)));
        let mut cursor = None;

        left_click_slot(&mut inventory, 0, &mut cursor);
        assert_eq!(cursor, ItemStack::try_new(Item::Dirt, 40));
        assert_eq!(inventory.slot(0), None);

        left_click_slot(&mut inventory, 1, &mut cursor);
        assert_eq!(inventory.slot(1), Some(ItemStack::full(Item::Dirt)));
        assert_eq!(cursor, ItemStack::try_new(Item::Dirt, 6));

        left_click_slot(&mut inventory, 2, &mut cursor);
        assert_eq!(inventory.slot(2), ItemStack::try_new(Item::Dirt, 6));
        assert_eq!(cursor, Some(ItemStack::one(Item::Stone)));
    }

    #[test]
    fn right_click_splits_and_places_one() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, ItemStack::try_new(Item::Dirt, 7));
        let mut cursor = None;

        right_click_slot(&mut inventory, 0, &mut cursor);
        assert_eq!(inventory.slot(0), ItemStack::try_new(Item::Dirt, 3));
        assert_eq!(cursor, ItemStack::try_new(Item::Dirt, 4));

        right_click_slot(&mut inventory, 1, &mut cursor);
        assert_eq!(inventory.slot(1), Some(ItemStack::one(Item::Dirt)));
        assert_eq!(cursor, ItemStack::try_new(Item::Dirt, 3));
    }

    #[test]
    fn left_drag_splits_evenly_and_leaves_the_remainder_on_the_cursor() {
        let mut inventory = Inventory::empty();
        let mut cursor = ItemStack::try_new(Item::Dirt, 8);
        distribute_drag(&mut inventory, &mut cursor, &[0, 1, 2], MouseButton::Left);
        assert_eq!(inventory.slot(0), ItemStack::try_new(Item::Dirt, 2));
        assert_eq!(inventory.slot(1), ItemStack::try_new(Item::Dirt, 2));
        assert_eq!(inventory.slot(2), ItemStack::try_new(Item::Dirt, 2));
        assert_eq!(cursor, ItemStack::try_new(Item::Dirt, 2));
    }

    #[test]
    fn left_drag_fills_capacity_then_keeps_the_unplaced_remainder() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, ItemStack::try_new(Item::Dirt, 62));
        let mut cursor = ItemStack::try_new(Item::Dirt, 64);

        distribute_drag(&mut inventory, &mut cursor, &[0, 1], MouseButton::Left);

        assert_eq!(inventory.slot(0), Some(ItemStack::full(Item::Dirt)));
        assert_eq!(inventory.slot(1), ItemStack::try_new(Item::Dirt, 32));
        assert_eq!(cursor, ItemStack::try_new(Item::Dirt, 30));
    }

    #[test]
    fn right_drag_places_one_item_per_accepted_slot() {
        let mut inventory = Inventory::empty();
        let mut cursor = ItemStack::try_new(Item::Stone, 5);
        distribute_drag(&mut inventory, &mut cursor, &[0, 1, 2], MouseButton::Right);
        assert_eq!(inventory.slot(0), Some(ItemStack::one(Item::Stone)));
        assert_eq!(inventory.slot(1), Some(ItemStack::one(Item::Stone)));
        assert_eq!(inventory.slot(2), Some(ItemStack::one(Item::Stone)));
        assert_eq!(cursor, ItemStack::try_new(Item::Stone, 2));
    }

    #[test]
    fn drag_preview_plans_the_same_result_as_release() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(1, Some(ItemStack::full(Item::Stone)));

        let preview = drag_preview(
            &inventory,
            ItemStack::try_new(Item::Dirt, 8).unwrap(),
            &[0, 1, 2],
            MouseButton::Left,
        );

        // Slot 1 rejects the wrong item, so the eight items split evenly
        // across the two remaining slots, exactly as release would distribute.
        assert_eq!(
            preview,
            DragProjection {
                slots: vec![
                    (0, ItemStack::try_new(Item::Dirt, 4).unwrap()),
                    (2, ItemStack::try_new(Item::Dirt, 4).unwrap()),
                ],
                cursor: None,
            }
        );
        assert_eq!(inventory.slot(0), None);
    }

    #[test]
    fn drag_preview_stops_at_a_full_slot_and_right_drag_places_one_each() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, ItemStack::try_new(Item::Dirt, 62));
        inventory.set_slot(1, Some(ItemStack::full(Item::Stone)));

        assert_eq!(
            drag_preview(
                &inventory,
                ItemStack::try_new(Item::Dirt, 5).unwrap(),
                &[0, 1],
                MouseButton::Left,
            ),
            DragProjection {
                slots: vec![(0, ItemStack::full(Item::Dirt))],
                cursor: ItemStack::try_new(Item::Dirt, 3),
            }
        );
        assert_eq!(
            drag_preview(
                &inventory,
                ItemStack::try_new(Item::Dirt, 5).unwrap(),
                &[0, 1, 2],
                MouseButton::Right,
            ),
            DragProjection {
                slots: vec![
                    (0, ItemStack::try_new(Item::Dirt, 63).unwrap()),
                    (2, ItemStack::one(Item::Dirt)),
                ],
                cursor: ItemStack::try_new(Item::Dirt, 3),
            }
        );
    }

    #[test]
    fn taking_from_the_cursor_never_creates_zero_stacks() {
        let mut cursor = Some(ItemStack::one(Item::Dirt));
        assert_eq!(
            take_from_stack(&mut cursor, u16::MAX),
            Some(ItemStack::one(Item::Dirt))
        );
        assert_eq!(cursor, None);
    }

    #[test]
    fn double_click_collection_gathers_matching_items_up_to_the_limit() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, ItemStack::try_new(Item::Dirt, 20));
        inventory.set_slot(1, ItemStack::try_new(Item::Dirt, 40));
        inventory.set_slot(2, ItemStack::try_new(Item::Dirt, 10));
        let mut cursor = ItemStack::try_new(Item::Dirt, 8);

        collect_matching_stacks(&mut inventory, &mut cursor);

        assert_eq!(cursor, Some(ItemStack::full(Item::Dirt)));
        assert_eq!(inventory.slot(0), None);
        assert_eq!(inventory.slot(1), ItemStack::try_new(Item::Dirt, 4));
        assert_eq!(inventory.slot(2), ItemStack::try_new(Item::Dirt, 10));
    }

    #[test]
    fn inventory_overlay_spawns_all_thirty_six_slots() {
        let mut app = App::new();
        app.add_systems(Startup, |mut commands: Commands| {
            spawn_inventory_ui(&mut commands);
        });

        app.update();

        let mut slots = app.world_mut().query::<&InventorySlot>();
        assert_eq!(
            slots.iter(app.world()).count(),
            crate::item::INVENTORY_SLOTS
        );
    }

    #[test]
    fn slot_highlights_use_hovered_and_pressed_state() {
        let mut app = App::new();
        app.init_resource::<DragPreview>()
            .add_systems(Update, update_slot_highlights);
        let normal = app
            .world_mut()
            .spawn((
                InventorySlot(0),
                Hovered::default(),
                BackgroundColor(Color::BLACK),
                BorderColor::all(SLOT_BORDER),
            ))
            .id();
        let hovered = app
            .world_mut()
            .spawn((
                InventorySlot(1),
                Hovered(true),
                BackgroundColor(Color::BLACK),
                BorderColor::all(SLOT_BORDER),
            ))
            .id();
        let pressed = app
            .world_mut()
            .spawn((
                InventorySlot(2),
                Hovered(true),
                Pressed,
                BackgroundColor(Color::BLACK),
                BorderColor::all(SLOT_BORDER),
            ))
            .id();

        app.update();

        let background = |entity| app.world().get::<BackgroundColor>(entity).map(|bg| bg.0);
        assert_eq!(background(normal), Some(SLOT_IDLE));
        assert_eq!(background(hovered), Some(SLOT_HOVER));
        assert_eq!(background(pressed), Some(SLOT_PRESSED));
    }

    #[test]
    fn pending_drag_highlights_slots_and_hides_real_counts() {
        let mut app = App::new();
        app.init_resource::<InventoryUi>()
            .init_resource::<DragPreview>()
            .init_resource::<ItemIcons>()
            .add_systems(
                Update,
                (update_drag_preview, update_slot_highlights).chain(),
            );
        app.world_mut()
            .spawn((Player::default(), Inventory::empty()));
        let slots: Vec<Entity> = (0..2)
            .map(|index| {
                app.world_mut()
                    .spawn((
                        InventorySlot(index),
                        Hovered::default(),
                        BackgroundColor(SLOT_IDLE),
                        BorderColor::all(SLOT_BORDER),
                    ))
                    .id()
            })
            .collect();
        let counts: Vec<Entity> = (0..2)
            .map(|index| {
                app.world_mut()
                    .spawn((
                        InventorySlotCount(index),
                        Text::new(""),
                        Visibility::Inherited,
                    ))
                    .id()
            })
            .collect();
        {
            let mut ui = app.world_mut().resource_mut::<InventoryUi>();
            ui.cursor_stack = ItemStack::try_new(Item::Dirt, 5);
            ui.drag = Some(StackDrag {
                button: MouseButton::Left,
                slots: vec![0, 1],
            });
        }

        app.update();

        for slot in &slots {
            assert_eq!(
                app.world().get::<BackgroundColor>(*slot).map(|bg| bg.0),
                Some(SLOT_PENDING)
            );
            assert_eq!(
                app.world()
                    .get::<BorderColor>(*slot)
                    .map(|border| border.top),
                Some(PENDING_BORDER)
            );
        }
        for count in &counts {
            assert_eq!(
                app.world().get::<Visibility>(*count),
                Some(&Visibility::Hidden)
            );
        }

        app.world_mut().resource_mut::<InventoryUi>().drag = None;
        app.update();

        for slot in &slots {
            assert_eq!(
                app.world().get::<BackgroundColor>(*slot).map(|bg| bg.0),
                Some(SLOT_IDLE)
            );
            assert_eq!(
                app.world()
                    .get::<BorderColor>(*slot)
                    .map(|border| border.top),
                Some(SLOT_BORDER)
            );
        }
        for count in &counts {
            assert_eq!(
                app.world().get::<Visibility>(*count),
                Some(&Visibility::Inherited)
            );
        }
    }

    #[test]
    fn cursor_shows_the_projected_remainder_during_a_drag() {
        let mut app = App::new();
        app.init_resource::<InventoryUi>()
            .init_resource::<DragPreview>()
            .init_resource::<ItemIcons>()
            .add_systems(Update, update_cursor_stack);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut().spawn((CursorStackRoot, Node::default()));
        app.world_mut()
            .spawn((CursorStackIcon, ImageNode::default()));
        let count = app
            .world_mut()
            .spawn((CursorStackCount, Text::new(""), TextColor(Color::WHITE)))
            .id();
        {
            let mut ui = app.world_mut().resource_mut::<InventoryUi>();
            ui.cursor_stack = ItemStack::try_new(Item::Dirt, 5);
        }
        app.world_mut().resource_mut::<DragPreview>().0 = Some(DragProjection {
            slots: vec![],
            cursor: ItemStack::try_new(Item::Dirt, 2),
        });

        app.update();

        assert_eq!(
            app.world().get::<Text>(count).map(|text| text.0.as_str()),
            Some("2")
        );
        assert_eq!(
            app.world().get::<TextColor>(count).map(|color| color.0),
            Some(Color::WHITE)
        );

        app.world_mut().resource_mut::<DragPreview>().0 = Some(DragProjection {
            slots: vec![],
            cursor: None,
        });
        app.update();

        assert_eq!(
            app.world().get::<Text>(count).map(|text| text.0.as_str()),
            Some("0")
        );
        assert_eq!(
            app.world().get::<TextColor>(count).map(|color| color.0),
            Some(PENDING_COUNT)
        );
    }

    #[test]
    fn inventory_key_opens_and_escape_closes_without_losing_a_carried_stack() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .insert_state(GameState::Playing)
            .insert_state(MouseState::Grabbed)
            .init_resource::<InventoryUi>()
            .init_resource::<KeyBindings>()
            .insert_resource(ButtonInput::<KeyCode>::default())
            .add_observer(return_carried_stack)
            .add_systems(PreUpdate, crate::game_state::route_back_input)
            .add_systems(
                PreUpdate,
                toggle_inventory_ui.after(crate::game_state::route_back_input),
            );
        let player = app.world_mut().spawn(Player::default()).id();

        let inventory_key = app.world().resource::<KeyBindings>().open_inventory;
        press_key(&mut app, inventory_key);
        app.world_mut().run_schedule(PreUpdate);
        assert!(app.world().resource::<InventoryUi>().is_open());
        assert_eq!(overlay_count(&mut app), 1, "opening spawns the overlay");

        {
            let mut ui = app.world_mut().resource_mut::<InventoryUi>();
            ui.cursor_stack = Some(ItemStack::one(Item::Dirt));
        }
        press_key(&mut app, KeyCode::Escape);
        app.world_mut().run_schedule(PreUpdate);

        let ui = app.world().resource::<InventoryUi>();
        assert!(!ui.is_open());
        assert!(!ui.close_requested, "the back request must be consumed");
        assert!(ui.cursor_stack.is_none());
        assert!(
            matches!(
                *app.world().resource::<NextState<GameState>>(),
                NextState::Unchanged
            ),
            "closing the inventory must not also pause the game"
        );
        assert_eq!(overlay_count(&mut app), 0, "closing despawns the overlay");
        assert_eq!(
            app.world().get::<Inventory>(player).unwrap().slot(9),
            Some(ItemStack::one(Item::Dirt))
        );
    }

    fn overlay_count(app: &mut App) -> usize {
        let mut overlays = app.world_mut().query::<&InventoryOverlay>();
        overlays.iter(app.world()).count()
    }

    fn press_key(app: &mut App, key: KeyCode) {
        let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keyboard.reset_all();
        keyboard.press(key);
    }

    #[test]
    fn despawn_queues_cursor_overflow_instead_of_losing_it() {
        let mut app = App::new();
        app.add_observer(return_carried_stack)
            .init_resource::<InventoryUi>();
        let player = app.world_mut().spawn(Player::default()).id();
        {
            let mut inventories = app.world_mut().query::<&mut Inventory>();
            let mut inventory = inventories.single_mut(app.world_mut()).unwrap();
            for index in 0..crate::item::INVENTORY_SLOTS {
                inventory.set_slot(index, Some(ItemStack::full(Item::Stone)));
            }
        }
        {
            let mut ui = app.world_mut().resource_mut::<InventoryUi>();
            ui.open = true;
            ui.cursor_stack = Some(ItemStack::one(Item::Dirt));
        }
        let overlay = app.world_mut().spawn(InventoryOverlay).id();

        app.world_mut().despawn(overlay);

        let ui = app.world().resource::<InventoryUi>();
        assert!(!ui.open);
        assert!(ui.cursor_stack.is_none());
        let mut drops = ui.pending_drops.iter();
        let Some(drop) = drops.next() else {
            panic!("expected the cursor overflow to be queued");
        };
        assert!(drops.next().is_none(), "only the overflow may be queued");
        assert_eq!(drop.stack, ItemStack::one(Item::Dirt));
        assert_eq!(drop.inventory_slot, None);
        assert_eq!(
            app.world().get::<Inventory>(player).unwrap().slot(9),
            Some(ItemStack::full(Item::Stone))
        );
    }

    #[test]
    fn drag_uses_current_cursor_slot_not_stale_pressed_origin() {
        let mut app = inventory_input_app();
        let origin = app
            .world_mut()
            .spawn((
                InventorySlot(0),
                Hovered::default(),
                Pressed,
                RelativeCursorPosition {
                    cursor_over: false,
                    normalized: None,
                },
            ))
            .id();
        let current = app
            .world_mut()
            .spawn((
                InventorySlot(1),
                Hovered(true),
                RelativeCursorPosition {
                    cursor_over: true,
                    normalized: Some(Vec2::splat(0.5)),
                },
            ))
            .id();
        app.world_mut()
            .spawn((InventoryPanel, RelativeCursorPosition::default()));
        {
            let mut ui = app.world_mut().resource_mut::<InventoryUi>();
            ui.cursor_stack = ItemStack::try_new(Item::Dirt, 4);
            ui.drag = Some(StackDrag {
                button: MouseButton::Left,
                slots: vec![0],
            });
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);

        app.world_mut().run_schedule(PreUpdate);

        assert_eq!(
            app.world()
                .resource::<InventoryUi>()
                .drag
                .as_ref()
                .unwrap()
                .slots,
            vec![0, 1]
        );
        // `Pressed` stays on the press origin during a drag; cursor geometry is
        // the authoritative current target.
        assert!(app.world().get::<Pressed>(origin).is_some());
        assert_eq!(app.world().get::<Hovered>(current), Some(&Hovered(true)));

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.world_mut().run_schedule(PreUpdate);
        let inventory = app
            .world_mut()
            .query::<&Inventory>()
            .single(app.world())
            .unwrap();
        assert_eq!(inventory.slot(0), ItemStack::try_new(Item::Dirt, 2));
        assert_eq!(inventory.slot(1), ItemStack::try_new(Item::Dirt, 2));
        assert_eq!(app.world().resource::<InventoryUi>().cursor_stack, None);
    }

    #[test]
    fn double_click_collection_requires_the_same_slot() {
        let mut app = inventory_input_app();
        app.world_mut().spawn((
            InventorySlot(0),
            Hovered::default(),
            RelativeCursorPosition {
                cursor_over: false,
                normalized: None,
            },
        ));
        app.world_mut().spawn((
            InventorySlot(1),
            Hovered(true),
            RelativeCursorPosition {
                cursor_over: true,
                normalized: Some(Vec2::splat(0.5)),
            },
        ));
        app.world_mut()
            .spawn((InventoryPanel, RelativeCursorPosition::default()));
        {
            let mut inventories = app.world_mut().query::<&mut Inventory>();
            let mut inventory = inventories.single_mut(app.world_mut()).unwrap();
            inventory.set_slot(0, ItemStack::try_new(Item::Dirt, 20));
            inventory.set_slot(1, ItemStack::try_new(Item::Dirt, 40));
        }
        {
            let mut ui = app.world_mut().resource_mut::<InventoryUi>();
            ui.cursor_stack = ItemStack::try_new(Item::Dirt, 8);
            ui.last_left_click = Some((0, Item::Dirt, 0.0));
        }
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            mouse.press(MouseButton::Left);
            mouse.release(MouseButton::Left);
        }

        app.world_mut().run_schedule(PreUpdate);

        let inventory = app
            .world_mut()
            .query::<&Inventory>()
            .single(app.world())
            .unwrap();
        assert_eq!(inventory.slot(0), ItemStack::try_new(Item::Dirt, 20));
        assert_eq!(inventory.slot(1), ItemStack::try_new(Item::Dirt, 48));
        assert_eq!(app.world().resource::<InventoryUi>().cursor_stack, None);
    }

    #[test]
    fn releasing_a_one_slot_drag_outside_the_panel_drops_instead_of_clicking() {
        let mut app = inventory_input_app();
        app.world_mut().spawn((
            InventorySlot(0),
            Hovered::default(),
            RelativeCursorPosition {
                cursor_over: false,
                normalized: None,
            },
        ));
        app.world_mut()
            .spawn((InventoryPanel, RelativeCursorPosition::default()));
        {
            let mut ui = app.world_mut().resource_mut::<InventoryUi>();
            ui.cursor_stack = ItemStack::try_new(Item::Dirt, 5);
            ui.drag = Some(StackDrag {
                button: MouseButton::Left,
                slots: vec![0],
            });
        }
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            mouse.press(MouseButton::Left);
            mouse.release(MouseButton::Left);
        }

        app.world_mut().run_schedule(PreUpdate);

        let ui = app.world().resource::<InventoryUi>();
        assert!(ui.drag.is_none());
        assert_eq!(ui.cursor_stack, None);
        assert_eq!(ui.pending_drops.len(), 1);
        assert_eq!(
            ui.pending_drops.front().map(|drop| drop.stack),
            ItemStack::try_new(Item::Dirt, 5)
        );
        let inventory = app
            .world_mut()
            .query::<&Inventory>()
            .single(app.world())
            .unwrap();
        assert_eq!(inventory.slot(0), None);
    }

    #[test]
    fn dropping_the_carried_stack_cancels_the_drag() {
        let mut app = inventory_input_app();
        app.world_mut().spawn((
            InventorySlot(0),
            Hovered(true),
            RelativeCursorPosition {
                cursor_over: true,
                normalized: Some(Vec2::splat(0.5)),
            },
        ));
        app.world_mut()
            .spawn((InventoryPanel, RelativeCursorPosition::default()));
        {
            let mut inventory = app
                .world_mut()
                .query::<&mut Inventory>()
                .single_mut(app.world_mut())
                .unwrap();
            inventory.set_slot(0, Some(ItemStack::one(Item::Stone)));
        }
        {
            let mut ui = app.world_mut().resource_mut::<InventoryUi>();
            ui.cursor_stack = Some(ItemStack::one(Item::Dirt));
            ui.drag = Some(StackDrag {
                button: MouseButton::Left,
                slots: vec![0],
            });
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        let drop_key = app.world().resource::<KeyBindings>().drop_item;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(drop_key);

        app.world_mut().run_schedule(PreUpdate);

        {
            let ui = app.world().resource::<InventoryUi>();
            assert!(ui.drag.is_none());
            assert_eq!(ui.cursor_stack, None);
            assert_eq!(ui.pending_drops.len(), 1);
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);

        app.world_mut().run_schedule(PreUpdate);

        let ui = app.world().resource::<InventoryUi>();
        assert!(ui.drag.is_none());
        assert_eq!(ui.cursor_stack, None);
        let inventory = app
            .world_mut()
            .query::<&Inventory>()
            .single(app.world())
            .unwrap();
        assert_eq!(inventory.slot(0), Some(ItemStack::one(Item::Stone)));
    }

    #[test]
    fn drag_preview_marks_planned_slots_and_clears_when_the_drag_ends() {
        let mut app = inventory_input_app();
        app.init_resource::<ItemIcons>()
            .init_resource::<DragPreview>();
        app.add_systems(Update, update_drag_preview);
        let overlays: Vec<Entity> = (0..2)
            .map(|index| {
                app.world_mut()
                    .spawn((InventorySlotPreview(index), Visibility::Hidden))
                    .id()
            })
            .collect();
        for index in 0..2 {
            app.world_mut()
                .spawn((InventoryPreviewIcon(index), ImageNode::default()));
            app.world_mut()
                .spawn((InventoryPreviewCount(index), Text::new("")));
        }
        {
            let mut ui = app.world_mut().resource_mut::<InventoryUi>();
            ui.cursor_stack = ItemStack::try_new(Item::Dirt, 5);
            ui.drag = Some(StackDrag {
                button: MouseButton::Left,
                slots: vec![0, 1],
            });
        }

        app.update();

        for overlay in &overlays {
            assert_eq!(
                app.world().get::<Visibility>(*overlay),
                Some(&Visibility::Visible)
            );
        }
        let mut counts = app.world_mut().query::<(&InventoryPreviewCount, &Text)>();
        let mut labels: Vec<_> = counts
            .iter(app.world())
            .map(|(slot, text)| (slot.0, text.0.clone()))
            .collect();
        labels.sort();
        assert_eq!(labels, vec![(0, "2".to_string()), (1, "2".to_string())]);

        app.world_mut().resource_mut::<InventoryUi>().drag = None;
        app.update();

        for overlay in &overlays {
            assert_eq!(
                app.world().get::<Visibility>(*overlay),
                Some(&Visibility::Hidden)
            );
        }
    }

    #[test]
    fn drop_key_queues_one_item_from_the_hovered_slot() {
        let mut app = inventory_input_app();
        app.world_mut().spawn((
            InventorySlot(0),
            Hovered::default(),
            RelativeCursorPosition {
                cursor_over: true,
                normalized: Some(Vec2::splat(0.5)),
            },
        ));
        app.world_mut()
            .spawn((InventoryPanel, RelativeCursorPosition::default()));
        let mut inventories = app.world_mut().query::<&mut Inventory>();
        inventories
            .single_mut(app.world_mut())
            .unwrap()
            .set_slot(0, ItemStack::try_new(Item::Dirt, 5));

        let drop_key = app.world().resource::<KeyBindings>().drop_item;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(drop_key);
        app.world_mut().run_schedule(PreUpdate);

        let ui = app.world().resource::<InventoryUi>();
        assert_eq!(ui.pending_drops.len(), 1);
        let drop = ui.pending_drops.front().unwrap();
        assert_eq!(drop.stack, ItemStack::one(Item::Dirt));
        assert_eq!(drop.inventory_slot, Some(0));
        // The slot is only consumed when the request dispatches on a fixed tick.
        let inventory = app
            .world_mut()
            .query::<&Inventory>()
            .single(app.world())
            .unwrap();
        assert_eq!(inventory.slot(0), ItemStack::try_new(Item::Dirt, 5));
    }

    #[test]
    fn inventory_drop_respects_configured_drop_binding() {
        let mut app = inventory_input_app();
        app.world_mut().spawn((
            InventorySlot(0),
            Hovered::default(),
            RelativeCursorPosition {
                cursor_over: true,
                normalized: Some(Vec2::splat(0.5)),
            },
        ));
        app.world_mut()
            .spawn((InventoryPanel, RelativeCursorPosition::default()));
        app.world_mut().resource_mut::<KeyBindings>().drop_item = KeyCode::KeyR;
        app.world_mut().resource_mut::<InventoryUi>().cursor_stack =
            Some(ItemStack::one(Item::Dirt));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        app.world_mut().run_schedule(PreUpdate);
        assert_eq!(
            app.world().resource::<InventoryUi>().cursor_stack,
            Some(ItemStack::one(Item::Dirt))
        );
        assert!(
            app.world()
                .resource::<InventoryUi>()
                .pending_drops
                .is_empty()
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyR);

        app.world_mut().run_schedule(PreUpdate);

        let ui = app.world().resource::<InventoryUi>();
        assert_eq!(ui.cursor_stack, None);
        assert_eq!(ui.pending_drops.len(), 1);
        assert_eq!(
            ui.pending_drops.front().unwrap().stack,
            ItemStack::one(Item::Dirt)
        );
    }

    fn inventory_input_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<InventoryUi>()
            .init_resource::<KeyBindings>()
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .add_systems(PreUpdate, handle_inventory_ui_input);
        app.world_mut()
            .spawn((Player::default(), Inventory::empty()));
        app.world_mut().resource_mut::<InventoryUi>().open = true;
        app
    }
}
