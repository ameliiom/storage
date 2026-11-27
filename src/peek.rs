//! Shulker peek (comfort `storage.peek`): hovering a shulker box item anywhere shows its 27 slots
//! as a 9x3 grid of the game's slot sprites, each item drawn with its count, inside the tooltip.
//! An empty box shows nothing extra. Client-side: an item part on the item tag
//! `minecraft:shulker_boxes` answers `tooltip_component` and draws in `tooltip_component_draw`;
//! the server tells the client whether the comfort is earned (`payloads::Perks`).
use mod_api::{ClientHost, ItemStack, Tag, TooltipComponentDrawEvent, TooltipComponentEvent};
#[allow(unused_imports)]
use mod_api::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub const COLUMNS: usize = 9;
pub const ROWS: usize = 3;
pub const SLOTS: usize = COLUMNS * ROWS;
/// A slot sprite's size, as the game's container screens draw them.
const CELL: f32 = 18.0;
const SPRITE: &str = "container/slot";

/// Whether the server says this player has the comfort.
static EARNED: AtomicBool = AtomicBool::new(false);
/// Whether the server says this player may open a shulker box from the bag (`shulker.rs`).
static OPEN_EARNED: AtomicBool = AtomicBool::new(false);

pub fn earned(peek: bool, open: bool) {
    EARNED.store(peek, Ordering::Relaxed);
    OPEN_EARNED.store(open, Ordering::Relaxed);
}

pub fn open_earned() -> bool {
    OPEN_EARNED.load(Ordering::Relaxed)
}

/// Each of the 27 cells' entry in `items` (by its `slot`), or None; out-of-range slots and
/// empty stacks are left out. None when no cell holds anything.
pub fn cells(items: &[(i32, u32)]) -> Option<[Option<usize>; SLOTS]> {
    let mut grid = [None; SLOTS];
    for (i, &(slot, count)) in items.iter().enumerate() {
        if let Ok(s) = usize::try_from(slot)
            && s < SLOTS
            && count > 0
        {
            grid[s] = Some(i);
        }
    }
    grid.iter().any(Option::is_some).then_some(grid)
}

/// A cell's top left inside the grid.
pub fn at(cell: usize) -> [f32; 2] {
    [(cell % COLUMNS) as f32 * CELL, (cell / COLUMNS) as f32 * CELL]
}

/// The box's stacks with their slots, from its `minecraft:container` component
/// (`[{slot, item: {id, count, components}}]`).
fn contents(host: &dyn ClientHost, stack: &ItemStack) -> Vec<(i32, ItemStack)> {
    let Some(patch) = stack.components() else { return Vec::new() };
    let Some(Tag::List(list)) = patch.as_nbt().get(&"minecraft:container".into()) else { return Vec::new() };
    list.iter()
        .filter_map(|entry| {
            let Tag::Compound(entry) = entry else { return None };
            let slot = match entry.get(&"slot".into())? {
                Tag::Int(s) => *s,
                Tag::Byte(s) => i32::from(*s),
                Tag::Short(s) => i32::from(*s),
                _ => return None,
            };
            let Tag::Compound(item) = entry.get(&"item".into())? else { return None };
            Some((slot, ItemStack::from_nbt(host, item)?))
        })
        .collect()
}

fn on(host: &dyn ClientHost) -> bool {
    EARNED.load(Ordering::Relaxed) && crate::setting::SHULKER_PEEK.read_client(host)
}

#[derive(Clone)]
pub struct Peek;

#[mod_api::horizon_part]
impl mod_api::ItemPart for Peek {
    fn tooltip_component(&self, host: &dyn ClientHost, event: &mut TooltipComponentEvent<'_>) -> Option<[f32; 2]> {
        if !on(host) {
            return None;
        }
        let items: Vec<(i32, u32)> = contents(host, event.stack).iter().map(|(s, st)| (*s, st.count())).collect();
        cells(&items)?;
        Some([COLUMNS as f32 * CELL, ROWS as f32 * CELL + 2.0])
    }

    fn tooltip_component_draw(&self, host: &dyn ClientHost, event: &mut TooltipComponentDrawEvent<'_>) {
        if !on(host) {
            return;
        }
        let items = contents(host, event.stack);
        let counts: Vec<(i32, u32)> = items.iter().map(|(s, st)| (*s, st.count())).collect();
        let Some(grid) = cells(&counts) else { return };
        let [x, y] = event.origin;
        for (cell, item) in grid.iter().enumerate() {
            let [dx, dy] = at(cell);
            let (cx, cy) = (x + dx, y + dy);
            crate::client::sprite(&mut *event.draw, SPRITE, [cx, cy, CELL, CELL]);
            if let Some(i) = item {
                let stack = &items[*i].1;
                let n = stack.count();
                let label = (n > 1).then(|| n.to_string());
                event.draw.gui_draw_item_with(stack, [cx + 1.0, cy + 1.0], label.as_deref());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_box_shows_nothing() {
        assert!(cells(&[]).is_none());
        assert!(cells(&[(0, 0), (30, 5), (-1, 3)]).is_none());
    }

    #[test]
    fn each_slot_lands_in_its_cell() {
        let grid = cells(&[(0, 1), (8, 64), (9, 2), (26, 3)]).unwrap();
        assert_eq!(grid[0], Some(0));
        assert_eq!(grid[8], Some(1));
        assert_eq!(grid[9], Some(2));
        assert_eq!(grid[26], Some(3));
        assert_eq!(grid.iter().filter(|c| c.is_some()).count(), 4);
        assert_eq!(at(0), [0.0, 0.0]);
        assert_eq!(at(8), [144.0, 0.0]);
        assert_eq!(at(9), [0.0, 18.0]);
        assert_eq!(at(26), [144.0, 36.0]);
    }
}
