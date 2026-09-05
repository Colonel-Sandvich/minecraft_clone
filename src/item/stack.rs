use std::num::NonZeroU16;

use bevy::prelude::*;

use super::Item;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    pub item: Item,
    count: NonZeroU16,
}

impl ItemStack {
    pub const MAX_COUNT: u16 = 64;

    pub const fn try_new(item: Item, count: u16) -> Option<Self> {
        if count > Self::MAX_COUNT {
            return None;
        }

        match NonZeroU16::new(count) {
            Some(count) => Some(Self { item, count }),
            None => None,
        }
    }

    pub const fn one(item: Item) -> Self {
        Self {
            item,
            count: NonZeroU16::new(1).unwrap(),
        }
    }

    pub const fn full(item: Item) -> Self {
        Self {
            item,
            count: NonZeroU16::new(Self::MAX_COUNT).unwrap(),
        }
    }

    pub const fn count(self) -> u16 {
        self.count.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacks_reject_zero_and_counts_above_the_limit() {
        assert_eq!(ItemStack::try_new(Item::Dirt, 0), None);
        assert_eq!(
            ItemStack::try_new(Item::Dirt, ItemStack::MAX_COUNT + 1),
            None
        );
        assert_eq!(
            ItemStack::try_new(Item::Dirt, ItemStack::MAX_COUNT)
                .unwrap()
                .count(),
            ItemStack::MAX_COUNT
        );
    }
}
