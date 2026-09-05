use bevy::prelude::*;
use std::ops::Range;

use super::{Item, ItemStack};

pub const HOTBAR_SLOTS: usize = 9;
pub const INVENTORY_SLOTS: usize = 36;

/// The local player's carried items. The first nine slots are the hotbar.
///
/// An empty slot is `None`; an occupied slot always contains a non-empty,
/// size-limited [`ItemStack`].
#[derive(Component, Debug, PartialEq, Eq)]
pub struct Inventory {
    slots: [Option<ItemStack>; INVENTORY_SLOTS],
}

impl Default for Inventory {
    fn default() -> Self {
        let mut slots = [None; INVENTORY_SLOTS];
        for (slot, item) in slots.iter_mut().zip([
            Item::Dirt,
            Item::Stone,
            Item::Sand,
            Item::Glass,
            Item::OakLog,
            Item::OakLeaves,
            Item::Glowstone,
            Item::Grass,
            Item::Ice,
        ]) {
            *slot = Some(ItemStack::full(item));
        }
        Self { slots }
    }
}

impl Inventory {
    pub fn empty() -> Self {
        Self {
            slots: [None; INVENTORY_SLOTS],
        }
    }

    pub fn slot(&self, index: usize) -> Option<ItemStack> {
        self.slots.get(index).copied().flatten()
    }

    pub fn set_slot(&mut self, index: usize, stack: Option<ItemStack>) {
        if let Some(slot) = self.slots.get_mut(index) {
            *slot = stack;
        }
    }

    pub fn slot_with(&self, item: Item) -> Option<usize> {
        self.slots
            .iter()
            .position(|stack| stack.is_some_and(|stack| stack.item == item))
    }

    pub fn swap_slots(&mut self, first: usize, second: usize) {
        if first < INVENTORY_SLOTS && second < INVENTORY_SLOTS {
            self.slots.swap(first, second);
        }
    }

    /// Inserts as much of `stack` as possible and returns any remainder.
    /// Existing matching stacks are filled before an empty slot is used.
    pub fn insert(&mut self, stack: ItemStack) -> Option<ItemStack> {
        self.insert_into(stack, 0..INVENTORY_SLOTS)
    }

    /// Inserts into a specific slot range, filling matching stacks before
    /// using empty slots, and returns anything that did not fit.
    pub fn insert_into(&mut self, stack: ItemStack, range: Range<usize>) -> Option<ItemStack> {
        let mut remaining = stack.count();
        let start = range.start.min(INVENTORY_SLOTS);
        let end = range.end.min(INVENTORY_SLOTS);
        if start >= end {
            return Some(stack);
        }

        for slot in self.slots[start..end].iter_mut().flatten() {
            if slot.item != stack.item || slot.count() == ItemStack::MAX_COUNT {
                continue;
            }

            let moved = remaining.min(ItemStack::MAX_COUNT - slot.count());
            *slot = ItemStack::try_new(slot.item, slot.count() + moved).unwrap();
            remaining -= moved;
            if remaining == 0 {
                return None;
            }
        }

        if let Some(empty_slot) = self.slots[start..end]
            .iter_mut()
            .find(|slot| slot.is_none())
        {
            *empty_slot = ItemStack::try_new(stack.item, remaining);
            return None;
        }

        ItemStack::try_new(stack.item, remaining)
    }

    /// Quick-moves a stack between the hotbar and main inventory.
    pub fn quick_move(&mut self, index: usize) {
        let Some(stack) = self.slot(index) else {
            return;
        };
        let target = if index < HOTBAR_SLOTS {
            HOTBAR_SLOTS..INVENTORY_SLOTS
        } else {
            0..HOTBAR_SLOTS
        };

        self.set_slot(index, None);
        let remainder = self.insert_into(stack, target);
        self.set_slot(index, remainder);
    }

    /// Removes up to `count` items from a slot.
    ///
    /// The slot becomes empty when its last item is removed, rather than ever
    /// containing a zero-sized stack.
    pub fn take(&mut self, index: usize, count: u16) -> Option<ItemStack> {
        if count == 0 {
            return None;
        }

        let slot = self.slots.get_mut(index)?;
        let stack = (*slot)?;
        let removed = count.min(stack.count());
        let remaining = stack.count() - removed;
        *slot = ItemStack::try_new(stack.item, remaining);
        ItemStack::try_new(stack.item, removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taking_the_last_item_empties_the_slot() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, Some(ItemStack::one(Item::Dirt)));

        assert_eq!(inventory.take(0, 1), Some(ItemStack::one(Item::Dirt)));
        assert_eq!(inventory.slot(0), None);
        assert_eq!(inventory.take(0, 1), None);
    }

    #[test]
    fn taking_more_than_available_cannot_underflow() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, ItemStack::try_new(Item::Dirt, 2));

        assert_eq!(
            inventory.take(0, u16::MAX),
            ItemStack::try_new(Item::Dirt, 2)
        );
        assert_eq!(inventory.slot(0), None);
    }

    #[test]
    fn insertion_fills_matching_stacks_before_empty_slots() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, ItemStack::try_new(Item::Dirt, 63));
        inventory.set_slot(1, Some(ItemStack::one(Item::Stone)));

        assert_eq!(
            inventory.insert(ItemStack::try_new(Item::Dirt, 3).unwrap()),
            None
        );
        assert_eq!(inventory.slot(0), Some(ItemStack::full(Item::Dirt)));
        assert_eq!(inventory.slot(2), ItemStack::try_new(Item::Dirt, 2));
    }

    #[test]
    fn insertion_returns_items_that_do_not_fit() {
        let mut inventory = Inventory::empty();
        for index in 0..INVENTORY_SLOTS {
            inventory.set_slot(index, Some(ItemStack::full(Item::Stone)));
        }

        let dirt = ItemStack::try_new(Item::Dirt, 4).unwrap();
        assert_eq!(inventory.insert(dirt), Some(dirt));
    }

    #[test]
    fn slots_can_move_between_storage_and_the_hotbar() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, Some(ItemStack::one(Item::Stone)));
        inventory.set_slot(HOTBAR_SLOTS, Some(ItemStack::one(Item::Dirt)));

        assert_eq!(inventory.slot_with(Item::Dirt), Some(HOTBAR_SLOTS));
        inventory.swap_slots(0, HOTBAR_SLOTS);

        assert_eq!(inventory.slot(0), Some(ItemStack::one(Item::Dirt)));
        assert_eq!(
            inventory.slot(HOTBAR_SLOTS),
            Some(ItemStack::one(Item::Stone))
        );
    }

    #[test]
    fn quick_move_uses_only_the_opposite_inventory_section() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(0, ItemStack::try_new(Item::Dirt, 5));
        inventory.set_slot(HOTBAR_SLOTS, ItemStack::try_new(Item::Dirt, 62));

        inventory.quick_move(0);

        assert_eq!(inventory.slot(0), None);
        assert_eq!(
            inventory.slot(HOTBAR_SLOTS),
            Some(ItemStack::full(Item::Dirt))
        );
        assert_eq!(
            inventory.slot(HOTBAR_SLOTS + 1),
            ItemStack::try_new(Item::Dirt, 3)
        );
    }

    #[test]
    fn quick_move_restores_the_source_when_the_target_is_full() {
        let mut inventory = Inventory::empty();
        inventory.set_slot(HOTBAR_SLOTS, ItemStack::try_new(Item::Dirt, 5));
        for index in 0..HOTBAR_SLOTS {
            inventory.set_slot(index, Some(ItemStack::full(Item::Stone)));
        }

        inventory.quick_move(HOTBAR_SLOTS);

        assert_eq!(
            inventory.slot(HOTBAR_SLOTS),
            ItemStack::try_new(Item::Dirt, 5)
        );
    }
}
