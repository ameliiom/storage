//! What crosses the network. The server owns every item; the client only shows what it was told
//! and asks. Registered (version "1", optional) in `Hooks::register`.
use mod_api::{PayloadCodec, VarInt, VarLong, ToClient, ToServer};
use std::sync::OnceLock;

/// The most entries one view carries: a payload list holds at most 4096.
pub const MAX_ENTRIES: usize = 4000;

/// Server → client: the merged view of the network this player opened, in walk order.
/// `ids[i]` (an item id) and `totals[i]` describe entry `i`; requests name entries by index and
/// id, so a view that changed in between is refused instead of taking the wrong item.
#[derive(PayloadCodec, Clone, Debug)]
pub struct View {
    pub ids: Vec<String>,
    pub totals: Vec<VarLong>,
    /// How many containers the terminal reaches.
    pub containers: VarInt,
}

/// Client → server: take `amount` of entry `index` (whose id was `id`) into the inventory.
#[derive(PayloadCodec, Clone, Debug)]
pub struct Take {
    pub index: VarInt,
    pub id: String,
    pub amount: VarInt,
}

pub static VIEW: OnceLock<ToClient<View>> = OnceLock::new();
pub static TAKE: OnceLock<ToServer<Take>> = OnceLock::new();
