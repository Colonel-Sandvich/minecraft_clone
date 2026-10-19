use super::spawn::spawn_inventory_ui;
use super::visuals::{update_cursor_stack, update_drag_preview, update_slot_highlights};
use super::*;
use crate::{item::Item, ui::hotbar::ItemIcons};
use bevy::{
    picking::hover::Hovered,
    state::app::StatesPlugin,
    ui::{ComputedNode, Pressed, UiGlobalTransform},
    window::PrimaryWindow,
};

#[derive(Resource, Default)]
struct FoundSlots(Vec<usize>);

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
            origin: 0,
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
            origin: 0,
            slots: Vec::new(),
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
        vec![1]
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
    assert_eq!(inventory.slot(0), None);
    assert_eq!(inventory.slot(1), ItemStack::try_new(Item::Dirt, 4));
    assert_eq!(app.world().resource::<InventoryUi>().cursor_stack, None);
}

#[test]
fn drag_path_sampling_includes_slots_between_fast_pointer_positions() {
    let mut app = App::new();
    for (index, x) in [100.0, 160.0, 220.0].into_iter().enumerate() {
        app.world_mut().spawn((
            InventorySlot(index),
            RelativeCursorPosition::default(),
            ComputedNode {
                size: Vec2::splat(54.0),
                ..default()
            },
            UiGlobalTransform::from_translation(Vec2::new(x, 100.0)),
        ));
    }
    app.init_resource::<FoundSlots>().add_systems(
        Update,
        |slots: InventorySlotQuery, mut found: ResMut<FoundSlots>| {
            found.0 = slots_between(&slots, Vec2::new(100.0, 100.0), Vec2::new(220.0, 100.0));
        },
    );
    app.update();

    assert_eq!(app.world().resource::<FoundSlots>().0, vec![0, 1, 2]);
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
            origin: 0,
            slots: Vec::new(),
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
            origin: 0,
            slots: Vec::new(),
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
            origin: 0,
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
    app.world_mut().resource_mut::<InventoryUi>().cursor_stack = Some(ItemStack::one(Item::Dirt));
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
