use bevy::prelude::*;

use crate::item::ItemStack;

use super::hotbar::ItemIcons;

pub(super) fn stack_icon(icons: &ItemIcons, stack: Option<ItemStack>) -> Handle<Image> {
    stack
        .and_then(|stack| icons.icons.get(&stack.item))
        .cloned()
        .unwrap_or_default()
}

pub(super) fn stack_count_label(stack: Option<ItemStack>) -> String {
    stack.map_or_else(String::new, |stack| {
        let count = stack.count();
        if count > 1 {
            count.to_string()
        } else {
            String::new()
        }
    })
}

pub(super) fn stack_count_text() -> (Text, TextFont, TextColor, TextShadow) {
    (
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(20.0),
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow {
            offset: Vec2::splat(2.0),
            color: Color::BLACK,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Item;

    #[test]
    fn count_label_is_hidden_for_empty_and_single_item_slots() {
        assert_eq!(stack_count_label(None), "");
        assert_eq!(stack_count_label(Some(ItemStack::one(Item::Dirt))), "");
        assert_eq!(stack_count_label(ItemStack::try_new(Item::Dirt, 64)), "64");
    }
}
