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
    /// The comforts this player has earned: bit 0 Find, bit 1 quick deposit.
    pub comforts: VarInt,
    /// Each entry's `organize::Category` index, beside `ids`.
    pub categories: Vec<u8>,
    /// Slots used, then slots in all, over the joined containers.
    pub slots: Vec<VarInt>,
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
}

pub static VIEW: OnceLock<ToClient<View>> = OnceLock::new();
pub static CLICK: OnceLock<ToServer<Click>> = OnceLock::new();
pub static COMFORT: OnceLock<ToServer<Comfort>> = OnceLock::new();
