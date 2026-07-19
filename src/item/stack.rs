use bevy::prelude::*;

use super::Item;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    pub item: Item,
    pub count: u32,
}

impl ItemStack {
    pub const fn new(item: Item, count: u32) -> Self {
        Self { item, count }
    }

    pub const fn one(item: Item) -> Self {
        Self::new(item, 1)
    }
}
