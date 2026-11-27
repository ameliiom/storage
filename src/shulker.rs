//! Open a shulker box from your bag (comfort `storage.open_box`, the node after Shulker Peek):
//! right-click a shulker box in your inventory, on any screen that shows it, and it opens without
//! being placed, its 27 slots in the game's own 3-row chest screen, every change written into the
//! item.
//!
//! How, and why it can't duplicate:
//! - The click is seen in `item_stacked_on_other` (a right-click with an empty cursor on a slot
//!   holding one shulker box), on both sides, and cancelled so vanilla's half-pickup doesn't run;
//!   that hook has no host, so the server opens the box on its next tick.
//! - The game can back a block-less menu's slots only with a stack in a hand
//!   (`MenuSlotPlace::HeldItem`), so the box goes to the main hand first: a hotbar box is
//!   selected; a box in the main inventory changes places with the selected hotbar slot (one
//!   transaction, a move, never a copy) and goes back when the box closes, if both are still
//!   where they were left.
//! - Then `open_menu` (3 rows, the box's name) and `back_mod_menu_slots`: each click and
//!   shift-click on the 27 slots runs through the box's own item handler in a transaction. The
//!   game locks the hand's hotbar slot while it is open (the box can't go into itself) and closes
//!   the menu once the hand stops holding that box (Q, death, another menu...).
#[allow(unused_imports)]
use mod_api::prelude::*;
use mod_api::{Hand, HostTransaction, ItemKey, ItemPlace, ItemStackedOnOtherEvent, LoopContext, MenuReplace, MenuSlotPlace, PlayerRef, ServerHost, Text};
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const SLOTS: usize = 27;
pub const HOTBAR: usize = 9;
const INVENTORY: usize = 36;
const TAG: &str = "minecraft:shulker_boxes";

struct State {
    /// The shulker box item keys (the item tag), read once the tags are loaded, for the server
    /// and the client apart: in single player both run in one process with their own keys.
    boxes: Vec<ItemKey>,
    client_boxes: Vec<ItemKey>,
    /// Players with the comfort, as the server last read their tags (the click hook has no host).
    allowed: Vec<u128>,
    /// Clicks seen, to open on the next server tick: player and inventory slot.
    asked: Vec<(u128, usize)>,
    /// Open boxes: the player's menu window, and the swap to undo on close.
    open: BTreeMap<u128, Open>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Open {
    pub window: i32,
    /// The hotbar slot holding the box while it is open.
    pub hand: usize,
    /// The main-inventory slot the box came from, when it changed places with `hand`.
    pub from: Option<usize>,
}

static STATE: Mutex<State> = Mutex::new(State { boxes: Vec::new(), client_boxes: Vec::new(), allowed: Vec::new(), asked: Vec::new(), open: BTreeMap::new() });

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(&mut STATE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Where the box must go so the game can back the menu with it: `(hand slot, slot it came from
/// when it moves)`. A hotbar box is selected where it is; a main-inventory box changes places
/// with the selected hotbar slot. None for a slot outside the 36 (armour, off hand, crafting).
pub fn plan(slot: usize, selected: usize) -> Option<(usize, Option<usize>)> {
    match slot {
        s if s < HOTBAR => Some((s, None)),
        s if s < INVENTORY && selected < HOTBAR => Some((selected, Some(s))),
        _ => None,
    }
}

/// Whether a click is the one that opens a box: the secondary button, nothing on the cursor, one
/// box in the slot.
pub fn opens(primary: bool, carried: bool, count: u32, is_box: bool) -> bool {
    !primary && !carried && count == 1 && is_box
}

/// Both sides: a right-click on one shulker box with an empty cursor, by a player with the
/// comfort, in their own inventory (the menu's player-inventory slots), is cancelled and asked for.
pub fn clicked(context: &dyn LoopContext, event: &mut ItemStackedOnOtherEvent<'_, '_>) -> bool {
    let e = &mut *event.event;
    let Some(stack) = e.clicked.item() else { return false };
    let server = matches!(context.side(), mod_api::LoopSide::Server);
    let is_box = with(|s| if server { &s.boxes } else { &s.client_boxes }.contains(&stack.item()));
    if !opens(e.primary, e.carried.get().is_some(), stack.count(), is_box) || e.container_slot >= INVENTORY {
        return false;
    }
    let player = e.player;
    // The player's own inventory: its slots are the last 36 of any menu, the first 36 counted
    // by inventory index in the inventory screen; the container index tells them apart.
    eprintln!("[storage] box click: server={server} slot={} container={} container_slot={}", e.slot, e.container, e.container_slot);
    if !inventory_container(e.container) {
        return false;
    }
    if server {
        let Some(uuid) = player else { return false };
        if !with(|s| s.allowed.contains(&uuid)) {
            eprintln!("[storage] box click: not allowed");
            return false;
        }
        with(|s| {
            if !s.asked.iter().any(|(p, _)| *p == uuid) {
                s.asked.push((uuid, e.container_slot));
            }
        });
        true
    } else {
        eprintln!("[storage] box click: client earned={}", crate::peek::open_earned());
        crate::peek::open_earned()
    }
}

/// The container index of the player's own inventory in `StackedOnOther` (0: the menu's first
/// container is the player's for the inventory screen; in other menus the player's comes second).
/// Kept separate so the in-game check decides it.
fn inventory_container(container: usize) -> bool {
    container == INVENTORY_CONTAINER.load(std::sync::atomic::Ordering::Relaxed)
}

static INVENTORY_CONTAINER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Server tick: the shulker box keys once tags are loaded, who has the comfort, and the boxes
/// asked for.
pub fn tick(host: &mut dyn ServerHost, ticks: u32) {
    if with(|s| s.boxes.is_empty())
        && let Some(keys) = host.tags().key(&mod_api::Identifier::parse(TAG).expect("a tag id")).and_then(|k| host.tags().items(k))
    {
        with(|s| s.boxes = keys);
    }
    if ticks.is_multiple_of(20) {
        let on = crate::setting::SHULKER_OPEN.read(&*host, crate::MOD_ID);
        let allowed: Vec<u128> = if on {
            host.players().server_players().into_iter().filter(|p| crate::skills::has(&*host, *p, crate::skills::OPEN_BOX)).map(|p| p.uuid()).collect()
        } else {
            Vec::new()
        };
        with(|s| s.allowed = allowed);
    }
    for (uuid, slot) in with(|s| std::mem::take(&mut s.asked)) {
        if let Some(p) = crate::kit::players::by_uuid(&*host, uuid) {
            open(host, p, slot);
        }
    }
}

/// Client tick: the shulker box keys, for the click on the client's side.
pub fn client_tick(host: &dyn mod_api::ClientHost) {
    if with(|s| s.client_boxes.is_empty())
        && let Some(keys) = host.tags().key(&mod_api::Identifier::parse(TAG).expect("a tag id")).and_then(|k| host.tags().items(k))
    {
        with(|s| s.client_boxes = keys);
    }
}

fn is_box(stack: Option<&mod_api::ItemStack>) -> bool {
    stack.is_some_and(|st| st.count() == 1 && with(|s| s.boxes.contains(&st.item())))
}

fn slot_stack(host: &dyn ServerHost, p: PlayerRef, slot: usize) -> Option<mod_api::ItemStack> {
    p.inventory_slots(host).unwrap_or_default().into_iter().find(|(s, st)| *s as usize == slot && st.count() > 0).map(|(_, st)| st)
}

/// Opens the box in inventory slot `slot` for `p`.
fn open(host: &mut dyn ServerHost, p: PlayerRef, slot: usize) {
    if with(|s| s.open.contains_key(&p.uuid())) || !is_box(slot_stack(&*host, p, slot).as_ref()) {
        eprintln!("[storage] box open: not a box at {slot}");
        return;
    }
    let selected = p.selected_slot(&*host).unwrap_or(0);
    let Some((hand, from)) = plan(slot, selected) else { return };
    if let Some(from) = from
        && !swap(host, p, from, hand)
    {
        return;
    }
    if from.is_none() && selected != hand && !p.set_selected_slot(host, hand) {
        return;
    }
    let stack = slot_stack(&*host, p, hand);
    // A renamed box shows its name (an anvil's plain text), as a placed one does; else the
    // game's own title.
    let title = stack
        .as_ref()
        .and_then(|st| st.components())
        .and_then(|c| match c.as_nbt().get(&"minecraft:custom_name".into()) {
            Some(mod_api::Tag::String(s)) => Some(s.to_string_lossy()),
            Some(mod_api::Tag::Compound(t)) => match t.get(&"text".into()) {
                Some(mod_api::Tag::String(s)) => Some(s.to_string_lossy()),
                _ => None,
            },
            _ => None,
        })
        .filter(|s| !s.is_empty())
        .map_or_else(|| Text::translate("container.shulkerBox", Vec::new()), Text::literal);
    let window = match host.open_menu(p.uuid(), 3, title, Vec::new(), MenuReplace::CloseFirst) {
        Ok(Some(w)) => w,
        other => {
            eprintln!("[storage] box open: open_menu {other:?}");
            undo(host, p, Open { window: -1, hand, from });
            return;
        }
    };
    let places = (0..SLOTS).map(|slot| MenuSlotPlace::HeldItem { hand: Hand::Main, slot }).collect();
    if let Err(e) = p.back_mod_menu_slots(host, window, places) {
        eprintln!("[storage] the box's slots were not backed: {e:?}");
        host.close_menu(p.uuid());
        undo(host, p, Open { window, hand, from });
        return;
    }
    eprintln!("[storage] box open: window {window} hand {hand} from {from:?}");
    with(|s| s.open.insert(p.uuid(), Open { window, hand, from }));
}

/// Moves the stacks of inventory slots `a` and `b` past each other in one transaction (all of
/// both, or nothing).
fn swap(host: &mut dyn ServerHost, p: PlayerRef, a: usize, b: usize) -> bool {
    let uuid = p.uuid();
    let place = |s: usize| ItemPlace::PlayerSlot { player: uuid, slot: s as i32 };
    let (pa, pb) = (place(a), place(b));
    let ka = pa.resource(host).map(|k| (k, pa.amount(host)));
    let kb = pb.resource(host).map(|k| (k, pb.amount(host)));
    host.transaction(|host, tx| {
        let take = |host: &mut dyn ServerHost, at: &ItemPlace, k: &Option<(mod_api::ItemResource, u32)>| {
            k.as_ref().is_none_or(|(kind, n)| at.extract(host, tx, kind, *n) == *n)
        };
        let give = |host: &mut dyn ServerHost, at: &ItemPlace, k: &Option<(mod_api::ItemResource, u32)>| {
            k.as_ref().is_none_or(|(kind, n)| at.insert(host, tx, kind, *n) == *n)
        };
        if take(host, &pa, &ka) && take(host, &pb, &kb) && give(host, &pa, &kb) && give(host, &pb, &ka) { Ok(()) } else { Err(()) }
    })
    .is_ok()
}

/// Puts the box back where it came from, when it changed places and both are still as left.
fn undo(host: &mut dyn ServerHost, p: PlayerRef, open: Open) {
    let Some(from) = open.from else { return };
    if is_box(slot_stack(&*host, p, open.hand).as_ref()) {
        swap(host, p, open.hand, from);
    }
}

/// A menu closed (any way: Esc, another menu, death, the box leaving the hand): the box goes back.
pub fn closed(host: &mut dyn ServerHost, uuid: u128) {
    let Some(open) = with(|s| s.open.remove(&uuid)) else { return };
    if let Some(p) = crate::kit::players::by_uuid(&*host, uuid) {
        undo(host, p, open);
    }
}

/// A player left: nothing to put back (the box stayed in their inventory, its slots in the item).
pub fn forget(uuid: u128) {
    with(|s| {
        s.open.remove(&uuid);
        s.asked.retain(|(p, _)| *p != uuid);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_goes_to_the_hand_the_game_can_back() {
        assert_eq!(plan(3, 0), Some((3, None)));
        assert_eq!(plan(20, 4), Some((4, Some(20))));
        assert_eq!(plan(35, 8), Some((8, Some(35))));
        assert_eq!(plan(36, 0), None);
        assert_eq!(plan(40, 0), None);
        assert_eq!(plan(20, 9), None);
    }

    #[test]
    fn only_a_right_click_with_an_empty_cursor_on_one_box_opens() {
        assert!(opens(false, false, 1, true));
        assert!(!opens(true, false, 1, true));
        assert!(!opens(false, true, 1, true));
        assert!(!opens(false, false, 2, true));
        assert!(!opens(false, false, 1, false));
    }
}
