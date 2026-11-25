//! What crosses the network. The server owns every item; the client only shows what it was told
//! and asks. Registered (version "1", optional) in `Hooks::register`.
use mod_api::{PayloadCodec, ToClient, ToServer, VarInt, VarLong};
use std::sync::OnceLock;

/// The most entries one view carries: a payload list holds at most 4096.
pub const MAX_ENTRIES: usize = 4000;

/// Server → client: the merged view of everything joined to the open chest, in the server's
/// order. `ids[i]` (an item id) and `totals[i]` describe entry `i`; requests name entries by
/// index and id, so a view that changed in between is refused instead of taking the wrong item.
#[derive(PayloadCodec, Clone, Debug)]
pub struct View {
    pub ids: Vec<String>,
    pub totals: Vec<VarLong>,
    /// How many containers are joined (1: the chest alone, and no panel is shown).
    pub containers: VarInt,
    /// The comforts this player has earned, a bit each in `skills::COMFORTS` order: Find,
    /// quick deposit, auto-sort, the inventory panel, low-stock marks.
    pub comforts: VarInt,
    /// Each entry's `organize::Category` index, beside `ids`.
    pub categories: Vec<u8>,
    /// Slots used, then slots in all, over the joined containers.
    pub slots: Vec<VarInt>,
    /// Beside `ids`: an entry's whole stack as typed JSON (`nbtjson`) when it has components (an
    /// enchanted or worn tool, a potion), so the client draws and describes the real thing; else
    /// empty, and the plain item is meant.
    pub stacks: Vec<String>,
}

/// Client → server: a click on the storage panel, as a chest's slot takes one. On entry `index`
/// (whose id was `id`; -1 for the empty space after the list): with shift, a stack of it into the
/// inventory; with something on the cursor, that goes into storage (all of it with the left
/// button, one with the right); with an empty cursor, a stack of it (half with the right button).
#[derive(PayloadCodec, Clone, Debug)]
pub struct Click {
    pub index: VarInt,
    pub id: String,
    pub button: VarInt,
    pub shift: bool,
    /// What the gesture asks (`action`): a plain click, or one of the quick moves.
    pub action: VarInt,
}

pub mod action {
    /// A click as described on `Click`.
    pub const CLICK: i32 = 0;
    /// Shift + wheel down: one of the entry into the inventory.
    pub const ONE_OUT: i32 = 1;
    /// Shift + wheel up: one of the entry from the inventory back in.
    pub const ONE_IN: i32 = 2;
    /// Double-click: as much of the entry as the inventory holds.
    pub const ALL: i32 = 3;
    /// Q: one of the entry dropped in front of the player; Ctrl+Q a stack.
    pub const DROP: i32 = 4;
    pub const DROP_STACK: i32 = 5;
}

/// Client → server: an Organization comfort. `FIND` lights the chests holding entry `index`;
/// `EMPTY` puts the whole inventory but the hotbar into storage.
#[derive(PayloadCodec, Clone, Debug)]
pub struct Comfort {
    pub kind: VarInt,
    pub index: VarInt,
    pub id: String,
}

pub mod comfort {
    pub const FIND: i32 = 0;
    pub const EMPTY: i32 = 1;
    /// Sort chests, deposit matching and restock: not comforts, everyone has them.
    pub const SORT: i32 = 2;
    pub const MATCH: i32 = 3;
    pub const RESTOCK: i32 = 4;
    /// Sort the bag: the player's main inventory merged and ordered (everyone has it).
    pub const BAG: i32 = 5;
}

/// Client → server: the player's own inventory screen opened (`open`) or closed; with the
/// comfort, the chests within reach show beside it.
#[derive(PayloadCodec, Clone, Debug)]
pub struct Near {
    pub open: bool,
}

/// Server → client: the client-side comforts this player has earned (`perk` bits), sent when
/// they change.
#[derive(PayloadCodec, Clone, Debug)]
pub struct Perks {
    pub bits: VarInt,
}

pub mod perk {
    /// Shulker peek: a shulker box's slots in its tooltip.
    pub const PEEK: i32 = 1;
}

pub static VIEW: OnceLock<ToClient<View>> = OnceLock::new();
pub static PERKS: OnceLock<ToClient<Perks>> = OnceLock::new();
pub static NEAR: OnceLock<ToServer<Near>> = OnceLock::new();
pub static CLICK: OnceLock<ToServer<Click>> = OnceLock::new();
pub static COMFORT: OnceLock<ToServer<Comfort>> = OnceLock::new();
