//! The server side: a click on the terminal walks the chain of containers, merges what they hold
//! and sends the view; a take moves items from the containers to the player in one transaction
//! and sends the view again. Nothing is cached between requests but the player's last view, so
//! the containers stay the only truth. Containers are read through the game's `STORAGE`
//! capability (by kind, by the rules hoppers follow).
use crate::chain::{self, Block, Pos};
use crate::entries::{self, Entry};
use crate::payloads::{self, Take, View};
use mod_api::{
    BlockMetadata, Hand, Host, HostBlockStates, HostTransaction, IPayloadContext, ItemPlace, ItemResource, LevelRef, VarInt,
    VarLong, PlayerRef, STORAGE, ServerHost, StoragePage,
};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

const LINKED: &str = "storage:linked";
const CAP: usize = 256;

/// What a player last opened.
struct Session {
    dimension: String,
    terminal: Pos,
    entries: Vec<Entry<ItemResource>>,
}

static SESSIONS: Mutex<BTreeMap<u128, Session>> = Mutex::new(BTreeMap::new());

fn sessions() -> std::sync::MutexGuard<'static, BTreeMap<u128, Session>> {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// What stands at `pos` for the walk: a block in `storage:linked`, a double chest's half.
fn block_at(host: &dyn Host, level: LevelRef, pos: Pos) -> Block {
    let Some(state) = level.block_state(host, pos) else { return Block::Other };
    let meta = host.block_states();
    let Ok(name) = meta.name(state.block()) else { return Block::Other };
    let linked = host
        .registry_entry("minecraft:block", name)
        .and_then(|entry| host.registry_entry_in_tag(&entry, LINKED))
        .unwrap_or(false);
    if !linked {
        return Block::Other;
    }
    // A double chest answers for all 54 slots from either half: read the left half only.
    match state.property(&meta, "type").ok().flatten() {
        Some("left") => Block::Half { counted: true },
        Some("right") => Block::Half { counted: false },
        _ => Block::Container,
    }
}

/// Everything one container holds, by kind, a page at a time.
fn held(host: &mut dyn ServerHost, level: LevelRef, pos: Pos) -> Vec<(ItemResource, u64)> {
    let Some(storage) = level.capability(host, &STORAGE, pos, None) else { return Vec::new() };
    let Ok(mut page) = StoragePage::new(256) else { return Vec::new() };
    let mut kinds = Vec::new();
    let mut cursor = 0;
    loop {
        match storage.page(host, cursor, &mut page) {
            Ok(next) => {
                kinds.extend(page.entries().iter().cloned());
                match next {
                    Some(n) => cursor = n,
                    None => break,
                }
            }
            Err(e) => {
                eprintln!("[storage] reading {pos:?}: {e:?}");
                break;
            }
        }
    }
    kinds
}

/// Reads the network: the containers in walk order, and the merged entries.
fn read(host: &mut dyn ServerHost, level: LevelRef, terminal: Pos) -> (Vec<Pos>, Vec<Entry<ItemResource>>) {
    let containers = {
        let read: &dyn Host = &*host;
        chain::walk(terminal, CAP, |p| block_at(read, level, p))
    };
    let held: Vec<_> = containers.iter().map(|&pos| held(host, level, pos)).collect();
    (containers, entries::merge(held))
}

fn view(host: &dyn Host, containers: &[Pos], entries: &[Entry<ItemResource>]) -> View {
    let shown = &entries[..entries.len().min(payloads::MAX_ENTRIES)];
    View {
        ids: shown.iter().map(|e| e.kind.item().id(host)).collect(),
        totals: shown.iter().map(|e| VarLong(e.total.min(i64::MAX as u64) as i64)).collect(),
        containers: VarInt(containers.len() as i32),
    }
}

/// Reads the network for `player` at `terminal`, remembers it and sends the view.
pub fn open(host: &mut dyn ServerHost, player: u128, dimension: &str, terminal: Pos) {
    let Some(level) = host.level(dimension) else { return };
    let started = Instant::now();
    let (containers, entries) = read(host, level, terminal);
    let read_ms = started.elapsed().as_secs_f64() * 1000.0;
    let sent = view(&*host, &containers, &entries);
    eprintln!(
        "[storage] read {} containers, {} kinds in {read_ms:.2} ms; view about {} bytes",
        containers.len(),
        entries.len(),
        sent.ids.iter().map(|s| s.len() + 10).sum::<usize>()
    );
    sessions().insert(player, Session { dimension: dimension.to_owned(), terminal, entries });
    let Some(token) = payloads::VIEW.get() else { return };
    let Some(p) = host.players().server_players().into_iter().find(|p| p.uuid() == player) else { return };
    if let Err(e) = token.send(host, p, &sent) {
        eprintln!("[storage] sending the view: {e:?}");
    }
}

/// A take: re-read the network, find the entry the player saw, move what's there in one
/// transaction (nearest containers first, into the player's inventory; what doesn't fit drops at
/// their feet, as a full inventory does), and send the new view.
pub fn take(request: Take, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let player: PlayerRef = cx.player().map_err(|e| format!("{e:?}"))?;
    let uuid = player.uuid();
    let host = cx.server_host().map_err(|e| format!("{e:?}"))?;
    let index = request.index.0.max(0) as usize;
    let Some((dimension, terminal, seen)) =
        sessions().get(&uuid).and_then(|s| s.entries.get(index).map(|e| (s.dimension.clone(), s.terminal, e.kind.clone())))
    else {
        return Ok(());
    };
    if seen.item().id(&*host) == request.id {
        let Some(level) = host.level(&dimension) else { return Ok(()) };
        let (containers, entries) = read(host, level, terminal);
        if let Some(entry) = entries.iter().find(|e| e.kind == seen) {
            let plan = entries::split_take(&entry.holders, request.amount.0.max(0) as u64);
            let moved = host.transaction(|host, tx| {
                let mut moved = 0u64;
                for &(c, n) in &plan {
                    if let Some(storage) = level.capability(host, &STORAGE, containers[c], None) {
                        moved += storage.extract(host, tx, &seen, n);
                    }
                }
                let given = ItemPlace::PlayerHand { player: uuid, hand: Hand::Main }.insert(host, tx, &seen, moved as u32);
                // Everything taken out reached the player (or their feet), or nothing moves.
                if u64::from(given) == moved { Ok(moved) } else { Err(moved - u64::from(given)) }
            });
            if let Err(lost) = moved {
                eprintln!("[storage] a take of {} was rolled back ({lost} would not fit)", request.id);
            }
        }
    }
    open(host, uuid, &dimension, terminal);
    Ok(())
}

/// A player left: forget what they had open.
pub fn left(player: u128) {
    sessions().remove(&player);
}
