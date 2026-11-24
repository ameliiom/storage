//! The server side. Opening any linked chest (barrel, shulker box…) starts a session: the chest
//! and everything joined to it are read and sent as one merged view, shown beside the chest's own
//! screen. Clicks on that view take to the cursor or the inventory, or put the cursor's stack in,
//! each in one transaction, and every viewer gets the new view. Nothing is cached but each
//! viewer's last view, so the containers stay the only truth. Containers are read and written
//! through the game's `STORAGE` capability (by kind, by the rules hoppers follow).
use crate::chain::{self, Block, Pos};
use crate::entries::{self, Entry};
use crate::payloads::{self, Click, View};
use crate::skills;
use mod_api::{
    BlockMetadata, Hand, Host, HostBlockStates, HostTransaction, IPayloadContext, ItemPlace, ItemResource, LevelRef,
    PlayerRef, STORAGE, ServerHost, StoragePage, Transaction, VarInt, VarLong,
};
use std::collections::BTreeMap;
use std::sync::Mutex;

const LINKED: &str = "storage:linked";
/// Server ticks between refreshes of open chests, for changes made elsewhere (a hopper, a player
/// at another chest): the game tells no one when a chest's contents change.
const REFRESH: u32 = 20;

/// A player's open chest.
struct Session {
    dimension: String,
    chest: Pos,
    /// How many containers this player's network reaches (their `storage:network`).
    cap: usize,
    /// The entries as last sent, in the server's order: requests name them by index.
    entries: Vec<Entry<ItemResource>>,
}

struct State {
    sessions: BTreeMap<u128, Session>,
    ticks: u32,
}

static STATE: Mutex<State> = Mutex::new(State { sessions: BTreeMap::new(), ticks: 0 });

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(&mut STATE.lock().unwrap_or_else(|e| e.into_inner()))
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

fn walk(host: &dyn Host, level: LevelRef, chest: Pos, cap: usize) -> Vec<Pos> {
    chain::walk(chest, cap, |p| block_at(host, level, p))
}

/// Reads the network: the containers in walk order, and the merged entries.
fn read(host: &mut dyn ServerHost, level: LevelRef, chest: Pos, cap: usize) -> (Vec<Pos>, Vec<Entry<ItemResource>>) {
    let found = walk(&*host, level, chest, cap);
    let held: Vec<_> = found.iter().map(|&pos| held(host, level, pos)).collect();
    (found, entries::merge(held))
}

fn player(host: &dyn ServerHost, uuid: u128) -> Option<PlayerRef> {
    host.players().server_players().into_iter().find(|p| p.uuid() == uuid)
}

/// The session's level, chest and network size.
fn session(host: &dyn ServerHost, uuid: u128) -> Option<(LevelRef, Pos, usize)> {
    let (dimension, chest, cap) = with(|s| s.sessions.get(&uuid).map(|x| (x.dimension.clone(), x.chest, x.cap)))?;
    Some((host.level(&dimension)?, chest, cap))
}

/// Puts up to `amount` of `kind` into the network in `tx`: the containers holding it first, then
/// the rest, nearest first, as hoppers fill. How much went in.
fn insert(host: &mut dyn ServerHost, tx: &Transaction, uuid: u128, kind: &ItemResource, amount: u64) -> u64 {
    let Some((level, chest, cap)) = session(&*host, uuid) else { return 0 };
    let (found, entries) = read(host, level, chest, cap);
    let holding: Vec<usize> =
        entries.iter().find(|e| &e.kind == kind).map(|e| e.holders.iter().map(|h| h.0).collect()).unwrap_or_default();
    let order: Vec<usize> = holding.iter().copied().chain((0..found.len()).filter(|c| !holding.contains(c))).collect();
    let mut left = amount;
    for c in order {
        if left == 0 {
            break;
        }
        if let Some(storage) = level.capability(host, &STORAGE, found[c], None) {
            left -= storage.insert(host, tx, kind, left).min(left);
        }
    }
    amount - left
}

/// Reads `uuid`'s open network again and sends the view.
fn refresh(host: &mut dyn ServerHost, uuid: u128) {
    let (Some((level, chest, cap)), Some(p)) = (session(&*host, uuid), player(&*host, uuid)) else { return };
    let (found, entries) = read(host, level, chest, cap);
    let shown = &entries[..entries.len().min(payloads::MAX_ENTRIES)];
    // The comforts this player has earned, so the panel shows only what works for them.
    let comforts = [skills::FIND, skills::EMPTY]
        .iter()
        .enumerate()
        .filter(|(_, tag)| skills::has(&*host, p, tag))
        .fold(0, |bits, (i, _)| bits | (1 << i));
    let view = View {
        ids: shown.iter().map(|e| e.kind.item().id(&*host)).collect(),
        totals: shown.iter().map(|e| VarLong(e.total.min(i64::MAX as u64) as i64)).collect(),
        containers: VarInt(found.len() as i32),
        comforts: VarInt(comforts),
    };
    with(|s| {
        if let Some(x) = s.sessions.get_mut(&uuid) {
            x.entries = entries;
        }
    });
    let Some(token) = payloads::VIEW.get() else { return };
    if let Err(e) = token.send(host, p, &view) {
        eprintln!("[storage] sending the view: {e:?}");
    }
}

/// Every open chest: after a change one player made, the others see it at once.
fn refresh_all(host: &mut dyn ServerHost) {
    for uuid in with(|s| s.sessions.keys().copied().collect::<Vec<_>>()) {
        refresh(host, uuid);
    }
}

/// A player opened a container's menu at `pos`: when it's a linked block, show its network.
pub fn opened(host: &mut dyn ServerHost, uuid: u128, dimension: &str, pos: Pos) {
    let Some(level) = host.level(dimension) else { return };
    if block_at(&*host, level, pos) == Block::Other {
        return;
    }
    let Some(p) = player(&*host, uuid) else { return };
    let cap = skills::network(&*host, p);
    with(|s| s.sessions.insert(uuid, Session { dimension: dimension.to_owned(), chest: pos, cap, entries: Vec::new() }));
    refresh(host, uuid);
}

/// A player closed it, or left.
pub fn closed(uuid: u128) {
    with(|s| s.sessions.remove(&uuid));
}

pub fn tick(host: &mut dyn ServerHost) {
    let (ticks, due) = with(|s| {
        s.ticks = s.ticks.wrapping_add(1);
        (s.ticks, s.ticks.is_multiple_of(REFRESH) && !s.sessions.is_empty())
    });
    skills::tick(host, ticks);
    if due {
        refresh_all(host);
    }
}

/// The kind entry `index` was, if the client named it right (a view that changed in between is
/// refused rather than taking the wrong item).
fn seen(host: &dyn ServerHost, uuid: u128, index: i32, id: &str) -> Option<ItemResource> {
    let kind = with(|s| Some(s.sessions.get(&uuid)?.entries.get(usize::try_from(index).ok()?)?.kind.clone()))?;
    (kind.item().id(host) == id).then_some(kind)
}

/// Takes up to `amount` of `kind` to `place` (the cursor, or the inventory), nearest containers
/// first, in one transaction: all of it reaches the player, or nothing moves.
fn take(host: &mut dyn ServerHost, p: PlayerRef, place: ItemPlace, kind: &ItemResource, amount: u64) {
    let Some((level, chest, cap)) = session(&*host, p.uuid()) else { return };
    let (found, entries) = read(host, level, chest, cap);
    let Some(entry) = entries.iter().find(|e| &e.kind == kind) else { return };
    let plan = entries::split_take(&entry.holders, amount);
    let moved = host.transaction(|host, tx| {
        let mut moved = 0u64;
        for &(c, n) in &plan {
            if let Some(storage) = level.capability(host, &STORAGE, found[c], None) {
                moved += storage.extract(host, tx, kind, n);
            }
        }
        let given = u64::from(place.insert(host, tx, kind, moved as u32));
        if given == moved { Ok(moved) } else { Err(moved - given) }
    });
    match moved {
        Ok(n) if n > 0 => skills::moved(host, p, n),
        Err(short) => eprintln!("[storage] a take was rolled back ({short} would not fit)"),
        _ => {}
    }
    refresh_all(host);
}

/// Moves up to `want` from `place` into the network in one transaction; all or nothing.
fn store(host: &mut dyn ServerHost, p: PlayerRef, place: ItemPlace, want: Option<u32>) {
    let Some(kind) = place.resource(host) else { return };
    let want = want.unwrap_or(u32::MAX).min(place.amount(host));
    let uuid = p.uuid();
    let moved = host.transaction(|host, tx| {
        let taken = u64::from(place.extract(host, tx, &kind, want));
        let stored = insert(host, tx, uuid, &kind, taken);
        // What the network can't hold stays where it was: nothing moves unless all of it fits.
        if stored == taken { Ok(stored) } else { Err(()) }
    });
    if let Ok(n) = moved
        && n > 0
    {
        skills::moved(host, p, n);
    }
}

/// A click on the storage panel (see `payloads::Click`).
pub fn click(request: Click, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let p: PlayerRef = cx.player().map_err(|e| format!("{e:?}"))?;
    let uuid = p.uuid();
    let host = cx.server_host().map_err(|e| format!("{e:?}"))?;
    let Some(menu) = p.container_menu(&*host) else { return Ok(()) };
    let cursor = ItemPlace::PlayerCursor { menu };
    let right = request.button.0 == 1;
    let held = cursor.resource(host);
    let entry = seen(&*host, uuid, request.index.0, &request.id);
    match (request.shift, held, entry) {
        // Shift: a stack of the entry into the inventory.
        (true, _, Some(kind)) => {
            let stack = u64::from(host.max_stack_size(kind.item()).unwrap_or(64));
            take(host, p, ItemPlace::PlayerHand { player: uuid, hand: Hand::Main }, &kind, stack);
        }
        // Something on the cursor: it goes in (one with the right button).
        (false, Some(_), _) => {
            store(host, p, cursor, right.then_some(1));
            refresh_all(host);
        }
        // An empty cursor on an entry: a stack of it, or half of one.
        (false, None, Some(kind)) => {
            let stack = u64::from(host.max_stack_size(kind.item()).unwrap_or(64));
            let total = with(|s| s.sessions.get(&uuid)?.entries.iter().find(|e| e.kind == kind).map(|e| e.total)).unwrap_or(0);
            let amount = if right { total.min(stack).div_ceil(2) } else { stack };
            take(host, p, cursor, &kind, amount);
        }
        _ => {}
    }
    Ok(())
}

/// The chests holding entry `index` (whose id was `id`) of `uuid`'s open network, for Find.
pub fn holders(host: &dyn ServerHost, uuid: u128, index: i32, id: &str) -> Option<(String, Vec<Pos>)> {
    let kind = seen(host, uuid, index, id)?;
    let (level, chest, cap) = session(host, uuid)?;
    let dimension = with(|s| s.sessions.get(&uuid).map(|x| x.dimension.clone()))?;
    let found = walk(host, level, chest, cap);
    let entry = with(|s| s.sessions.get(&uuid)?.entries.iter().find(|e| e.kind == kind).cloned())?;
    Some((dimension, entry.holders.iter().filter_map(|&(c, _)| found.get(c).copied()).collect()))
}

/// Quick deposit: every stack of the main inventory (not the hotbar) into storage.
pub fn empty(host: &mut dyn ServerHost, p: PlayerRef) {
    for slot in 9..36 {
        store(host, p, ItemPlace::PlayerSlot { player: p.uuid(), slot }, None);
    }
    refresh_all(host);
}

/// The containers a crafting station at `pos` lends its recipe book: every network touching it,
/// up to the player's network size.
pub fn sources(host: &dyn ServerHost, uuid: u128, pos: Pos) -> Vec<Pos> {
    let Some(p) = player(host, uuid) else { return Vec::new() };
    let Some(level) = p.dimension(host).and_then(|d| host.level(&d.to_string())) else { return Vec::new() };
    let cap = skills::network(host, p);
    let mut found: Vec<Pos> = Vec::new();
    for side in chain::SIDES {
        let next = chain::add(pos, side);
        if found.contains(&next) || block_at(host, level, next) == Block::Other {
            continue;
        }
        for c in walk(host, level, next, cap) {
            if !found.contains(&c) && found.len() < cap {
                found.push(c);
            }
        }
    }
    found
}
