//! The server side. Opening any linked chest (barrel, shulker box…), or a station touching one (a
//! crafting table, a furnace), starts a session: everything joined to it is read and sent as one
//! merged view, shown beside the screen. Clicks on that view take to the cursor or the inventory, or put the cursor's stack in,
//! each in one transaction, and every viewer gets the new view. Nothing is cached but each
//! viewer's last view, so the containers stay the only truth. Containers are read and written
//! through the game's `STORAGE` capability (by kind, by the rules hoppers follow).
use crate::chain::{self, Block, Pos};
use crate::entries::{self, Entry};
use crate::craft;
use crate::organize::{self, Category, Label, Traits};
use crate::nbtjson;
use crate::payloads::{self, Click, View, action};
use crate::skills;
use mod_api::{
    BlockMetadata, ComponentId, Compound, Host, HostBlockStates, HostTransaction, IPayloadContext, ITEMS, ItemPlace, ItemResource,
    ItemStack, LevelRef, PlayerRef, STORAGE, ServerHost, StoragePage, Tag, Transaction, VarInt, VarLong,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

const LINKED: &str = "storage:linked";
/// Server ticks between refreshes of open chests, for changes made elsewhere (a hopper, a player
/// at another chest): the game tells no one when a chest's contents change.
const REFRESH: u32 = 20;
/// How far the inventory reaches for chests (a player's block reach), and how far the player may
/// then walk before the panel goes.
const NEAR: i32 = 5;
const NEAR_KEEP: i32 = 8;

/// A player's open chest.
struct Session {
    dimension: String,
    chest: Pos,
    /// How many containers this player's network reaches (their `storage:network`).
    cap: usize,
    /// The entries as last sent, in the server's order: requests name them by index.
    entries: Vec<Entry<ItemResource>>,
    /// Opened from the player's own inventory beside the chests, not by opening one.
    near: bool,
    /// What the network held when it opened (ids and totals, sorted), to tell on close whether
    /// anything moved.
    before: Option<Vec<(String, u64)>>,
}

/// What a network holds, comparable: each kind's id and total, sorted.
fn fingerprint(host: &dyn Host, entries: &[Entry<ItemResource>]) -> Vec<(String, u64)> {
    let mut f: Vec<(String, u64)> = entries.iter().map(|e| (e.kind.item().id(host), e.total)).collect();
    f.sort();
    f
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

/// The network seen from `pos`: from it when it is a linked container (a chest), else from every
/// linked container touching it (a crafting table or a furnace beside chests), at most `cap`.
pub(crate) fn walk(host: &dyn Host, level: LevelRef, pos: Pos, cap: usize) -> Vec<Pos> {
    let at = |p| block_at(host, level, p);
    if at(pos) != Block::Other {
        return chain::walk(pos, cap, at);
    }
    let mut found: Vec<Pos> = Vec::new();
    for side in chain::SIDES {
        let next = chain::add(pos, side);
        if found.contains(&next) || at(next) == Block::Other {
            continue;
        }
        for c in chain::walk(next, cap, at) {
            if found.len() == cap {
                return found;
            }
            if !found.contains(&c) {
                found.push(c);
            }
        }
    }
    found
}

/// Reads the network: the containers in walk order, and the merged entries.
fn read(host: &mut dyn ServerHost, level: LevelRef, chest: Pos, cap: usize) -> (Vec<Pos>, Vec<Entry<ItemResource>>) {
    let found = walk(&*host, level, chest, cap);
    let entries = entries_of(host, level, &found);
    (found, entries)
}

use crate::kit::players::by_uuid as player;

/// The session's level, chest and network size.
fn session(host: &dyn ServerHost, uuid: u128) -> Option<(LevelRef, Pos, usize)> {
    let (dimension, chest, cap) = with(|s| s.sessions.get(&uuid).map(|x| (x.dimension.clone(), x.chest, x.cap)))?;
    Some((host.level(&dimension)?, chest, cap))
}

/// Puts up to `amount` of `kind` into the network in `tx`: the containers holding it first, then
/// the rest, nearest first, as hoppers fill. How much went in.
fn insert(host: &mut dyn ServerHost, tx: &Transaction, uuid: u128, kind: &ItemResource, amount: u64) -> u64 {
    let Some((level, chest, cap)) = session(&*host, uuid) else { return 0 };
    let found = walk(&*host, level, chest, cap);
    put_into(host, tx, level, &found, kind, amount)
}

/// The merged entries of the containers `found`, in their order.
pub(crate) fn entries_of(host: &mut dyn ServerHost, level: LevelRef, found: &[Pos]) -> Vec<Entry<ItemResource>> {
    let held: Vec<_> = found.iter().map(|&pos| held(host, level, pos)).collect();
    entries::merge(held)
}

/// Takes up to `amount` of entry `entry` (of `entries_of(found)`) out of the containers `found` in
/// `tx`, nearest first. How much came out.
pub(crate) fn take_from(
    host: &mut dyn ServerHost,
    tx: &Transaction,
    level: LevelRef,
    found: &[Pos],
    entry: &Entry<ItemResource>,
    amount: u64,
) -> u64 {
    let mut moved = 0;
    for (c, n) in entries::split_take(&entry.holders, amount) {
        if let Some(storage) = level.capability(host, &STORAGE, found[c], None) {
            moved += storage.extract(host, tx, &entry.kind, n);
        }
    }
    moved
}

/// Puts up to `amount` of `kind` into the containers `found` in `tx`, as `insert` orders them.
pub(crate) fn put_into(host: &mut dyn ServerHost, tx: &Transaction, level: LevelRef, found: &[Pos], kind: &ItemResource, amount: u64) -> u64 {
    let entries = entries_of(host, level, found);
    let labels = labels(host, level, found);
    let holding: Vec<usize> =
        entries.iter().find(|e| &e.kind == kind).map(|e| e.holders.iter().map(|h| h.0).collect()).unwrap_or_default();
    // Its labelled chests first, then where it already is, then the rest in label order.
    let ranked = organize::targets(&kind.item().id(&*host), category(&*host, kind), &labels);
    let homes = ranked.iter().copied().take_while(|&c| labels[c].is_some() && !holding.contains(&c));
    let order: Vec<usize> = homes
        .chain(holding.iter().copied().filter(|c| labels[*c].as_ref().is_none_or(|l| l.category == category(&*host, kind))))
        .chain(ranked.iter().copied())
        .fold(Vec::new(), |mut v, c| {
            if !v.contains(&c) {
                v.push(c);
            }
            v
        });
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
    let comforts = skills::COMFORTS
        .iter()
        .enumerate()
        .filter(|(_, tag)| skills::has(&*host, p, tag))
        .fold(0, |bits, (i, _)| bits | (1 << i));
    let view = View {
        ids: shown.iter().map(|e| e.kind.item().id(&*host)).collect(),
        totals: shown.iter().map(|e| VarLong(e.total.min(i64::MAX as u64) as i64)).collect(),
        containers: VarInt(found.len() as i32),
        comforts: VarInt(comforts),
        categories: shown.iter().map(|e| category(&*host, &e.kind).index()).collect(),
        slots: {
            let (used, total) = usage(host, level, &found);
            vec![VarInt(used as i32), VarInt(total as i32)]
        },
        stacks: shown.iter().map(|e| stack_json(&*host, &e.kind)).collect(),
    };
    let held = fingerprint(&*host, &entries);
    with(|s| {
        if let Some(x) = s.sessions.get_mut(&uuid) {
            x.entries = entries;
            x.before.get_or_insert(held);
        }
    });
    let Some(token) = payloads::VIEW.get() else { return };
    if let Err(e) = token.send(host, p, &view) {
        eprintln!("[storage] sending the view: {e:?}");
    }
}

/// The kind's stack as JSON when it has components (a payload string holds at most 4096
/// bytes: a bigger one, a long written book, shows as the plain item).
fn stack_json(host: &dyn Host, kind: &ItemResource) -> String {
    let Some(nbt) = kind.as_stack().to_nbt(host) else { return String::new() };
    if !nbt.contains_key(&"components".into()) {
        return String::new();
    }
    let text = nbtjson::write(&nbt);
    if text.len() <= 4000 { text } else { String::new() }
}

/// Tells the client to put the panel away (a view of no containers).
fn hide(host: &mut dyn ServerHost, uuid: u128) {
    let (Some(p), Some(token)) = (player(&*host, uuid), payloads::VIEW.get()) else { return };
    let view = View {
        ids: Vec::new(),
        totals: Vec::new(),
        containers: VarInt(0),
        comforts: VarInt(0),
        categories: Vec::new(),
        slots: Vec::new(),
        stacks: Vec::new(),
    };
    if let Err(e) = token.send(host, p, &view) {
        eprintln!("[storage] hiding the panel: {e:?}");
    }
}

/// Every open chest: after a change one player made, the others see it at once.
pub(crate) fn refresh_all(host: &mut dyn ServerHost) {
    for uuid in with(|s| s.sessions.keys().copied().collect::<Vec<_>>()) {
        refresh(host, uuid);
    }
}

/// A player opened a container's menu at `pos`: when it's a linked block, show its network.
pub fn opened(host: &mut dyn ServerHost, uuid: u128, dimension: &str, pos: Pos) {
    let Some(level) = host.level(dimension) else { return };
    if walk(&*host, level, pos, 1).is_empty() {
        return;
    }
    start(host, uuid, dimension, pos, false);
    // Furnaces from chests: the furnaces of this network are remembered.
    if let Some(p) = player(&*host, uuid)
        && skills::has(&*host, p, skills::FURNACE)
    {
        let found = walk(&*host, level, pos, skills::network(&*host, p));
        crate::furnace::look_around(&*host, dimension, level, pos, &found);
    }
}

fn start(host: &mut dyn ServerHost, uuid: u128, dimension: &str, pos: Pos, near: bool) {
    let Some(p) = player(&*host, uuid) else { return };
    let cap = skills::network(&*host, p);
    let session = Session { dimension: dimension.to_owned(), chest: pos, cap, entries: Vec::new(), near, before: None };
    with(|s| s.sessions.insert(uuid, session));
    refresh(host, uuid);
}

/// The player's own inventory opened (`open`) or closed: with the comfort, the chests within
/// reach show beside it, the nearest one's network.
pub fn near(host: &mut dyn ServerHost, p: PlayerRef, open: bool) {
    let uuid = p.uuid();
    if !open {
        if with(|s| s.sessions.get(&uuid).is_some_and(|x| x.near)) {
            closed(host, uuid);
        }
        return;
    }
    if !skills::has(&*host, p, skills::INVENTORY) {
        return;
    }
    let (Some(dimension), Some(at)) = (p.dimension(&*host), p.entity().block_position(&*host)) else { return };
    let dimension = dimension.to_string();
    let Some(level) = host.level(&dimension) else { return };
    let mut nearest: Option<(i32, Pos)> = None;
    for dx in -NEAR..=NEAR {
        for dy in -NEAR..=NEAR {
            for dz in -NEAR..=NEAR {
                let d = dx * dx + dy * dy + dz * dz;
                if d > NEAR * NEAR || nearest.is_some_and(|(best, _)| best <= d) {
                    continue;
                }
                let pos = [at[0] + dx, at[1] + 1 + dy, at[2] + dz];
                if block_at(&*host, level, pos) != Block::Other {
                    nearest = Some((d, pos));
                }
            }
        }
    }
    if let Some((_, pos)) = nearest {
        start(host, uuid, &dimension, pos, true);
    }
}

/// An inventory panel whose player walked away from the chests goes.
fn too_far(host: &dyn ServerHost, uuid: u128) -> bool {
    let Some((dimension, chest, near)) = with(|s| s.sessions.get(&uuid).map(|x| (x.dimension.clone(), x.chest, x.near))) else { return false };
    if !near {
        return false;
    }
    let Some(p) = player(host, uuid) else { return true };
    let here = p.dimension(host).map(|d| d.to_string());
    let Some(at) = p.entity().block_position(host) else { return true };
    let d = (0..3).map(|i| (at[i] - chest[i]).abs()).max().unwrap_or(0);
    here.as_deref() != Some(dimension.as_str()) || d > NEAR_KEEP
}

/// A player closed the chest (or the inventory beside it). With Auto-Sort, a network whose
/// contents changed while it was open sorts itself, unless someone else has it open.
pub fn closed(host: &mut dyn ServerHost, uuid: u128) {
    // The session goes first, so the sort's refresh sends nothing to a panel being closed.
    let place = session(&*host, uuid);
    let Some(gone) = with(|s| s.sessions.remove(&uuid)) else { return };
    if let (Some(p), Some((level, chest, cap))) = (player(&*host, uuid), place)
        && skills::has(&*host, p, skills::AUTO_SORT)
    {
        let (found, entries) = read(host, level, chest, cap);
        let changed = gone.before.is_some_and(|b| b != fingerprint(&*host, &entries));
        let shared = with(|s| s.sessions.values().any(|x| x.dimension == gone.dimension && found.contains(&x.chest)));
        if changed && !shared {
            sort_network(host, level, chest, cap);
        }
    }
}

/// A player left: their session goes, nothing sorts.
pub fn forget(uuid: u128) {
    with(|s| s.sessions.remove(&uuid));
}

pub fn tick(host: &mut dyn ServerHost) {
    let (ticks, due) = with(|s| {
        s.ticks = s.ticks.wrapping_add(1);
        (s.ticks, s.ticks.is_multiple_of(REFRESH) && !s.sessions.is_empty())
    });
    skills::tick(host, ticks);
    crate::furnace::tick(host);
    if ticks.is_multiple_of(REFRESH) {
        skills::send_perks(host);
    }
    if due {
        for uuid in with(|s| s.sessions.keys().copied().collect::<Vec<_>>()) {
            if too_far(&*host, uuid) {
                forget(uuid);
                hide(host, uuid);
            }
        }
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
    pull(host, p, kind, amount, |host, tx, moved| u64::from(place.insert(host, tx, kind, moved as u32)) == moved);
}

/// Takes up to `amount` of `kind` into the main inventory (`PlayerHand` holds only the hand's
/// slot: 168 cobblestone to it rolled back).
fn take_to_inventory(host: &mut dyn ServerHost, p: PlayerRef, kind: &ItemResource, amount: u64) {
    pull(host, p, kind, amount, |host, tx, moved| crate::kit::inventory::fill(host, tx, p, kind, moved) == moved);
}

/// Takes up to `amount` of `kind` out of the network, nearest first, and hands what came out to
/// `give`, all in one transaction: when `give` can't place all of it, nothing moves.
fn pull(
    host: &mut dyn ServerHost,
    p: PlayerRef,
    kind: &ItemResource,
    amount: u64,
    give: impl FnOnce(&mut dyn ServerHost, &Transaction, u64) -> bool,
) {
    let Some((level, chest, cap)) = session(&*host, p.uuid()) else { return };
    let (found, entries) = read(host, level, chest, cap);
    let Some(entry) = entries.iter().find(|e| &e.kind == kind) else { return };
    let mut give = Some(give);
    let moved = host.transaction(|host, tx| {
        let moved = take_from(host, tx, level, &found, entry, amount);
        if moved == 0 {
            return Ok(0);
        }
        if give.take().is_some_and(|give| give(host, tx, moved)) { Ok(moved) } else { Err(moved) }
    });
    match moved {
        Ok(n) if n > 0 => skills::moved(host, p, n),
        Err(n) => eprintln!("[storage] a take of {n} was rolled back (it had nowhere to go)"),
        _ => {}
    }
    refresh_all(host);
}

/// How many of `kind` the main inventory (hotbar included) still takes.
fn room(host: &dyn ServerHost, p: PlayerRef, kind: &ItemResource) -> u64 {
    let max = u64::from(host.max_stack_size(kind.item()).unwrap_or(64));
    let slots = p.inventory_slots(host).unwrap_or_default();
    let held: Vec<&ItemStack> = slots.iter().filter(|(s, st)| *s < 36 && st.count() > 0).map(|(_, st)| st).collect();
    let free = 36u64.saturating_sub(held.len() as u64) * max;
    free + held.iter().filter(|st| kind.matches(st)).map(|st| max.saturating_sub(u64::from(st.count()))).sum::<u64>()
}

/// Drops `amount` of `kind` from the network in front of the player, as Q drops from a slot.
fn drop_out(host: &mut dyn ServerHost, p: PlayerRef, kind: &ItemResource, amount: u64) {
    let Some(dimension) = with(|s| s.sessions.get(&p.uuid()).map(|x| x.dimension.clone())) else { return };
    let body = p.entity();
    let (Some(eye), Some(look)) = (body.eye_position(&*host), body.view_vector(&*host)) else { return };
    pull(host, p, kind, amount, |host, _, moved| {
        let Some(item) = kind.to_stack(moved as u32).and_then(|s| s.to_nbt(&*host)) else { return false };
        let motion = Tag::List(look.iter().map(|v| Tag::Double((v * 0.3).to_bits())).collect());
        let nbt: Compound =
            [("Item".into(), Tag::Compound(item)), ("Motion".into(), motion), ("PickupDelay".into(), Tag::Short(40))].into_iter().collect();
        host.summon(&dimension, "minecraft:item", [eye[0], eye[1] - 0.3, eye[2]], nbt).is_some()
    });
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
    // The cursor of the open menu (the chest's, or the inventory's); some menus have none to
    // offer, and then only the moves into the inventory work.
    let cursor = p.container_menu(&*host).map(|menu| ItemPlace::PlayerCursor { menu });
    let right = request.button.0 == 1;
    let held = cursor.as_ref().and_then(|c| c.resource(host));
    let entry = seen(&*host, uuid, request.index.0, &request.id);
    let stack_of = |host: &dyn ServerHost, kind: &ItemResource| u64::from(host.max_stack_size(kind.item()).unwrap_or(64));
    match (request.action.0, &entry) {
        (action::CLICK, _) => {}
        (action::ONE_OUT, Some(kind)) => take_to_inventory(host, p, kind, 1),
        (action::ONE_IN, Some(kind)) => {
            // From the main inventory before the hotbar, as a shift-click fills.
            let slots = p.inventory_slots(&*host).unwrap_or_default();
            let slot = slots.iter().filter(|(s, st)| *s < 36 && kind.matches(st)).map(|(s, _)| *s).max_by_key(|s| (*s >= 9, *s));
            if let Some(slot) = slot {
                store(host, p, ItemPlace::PlayerSlot { player: uuid, slot: slot as i32 }, Some(1));
                refresh_all(host);
            }
        }
        (action::ALL, Some(kind)) => {
            let amount = room(&*host, p, kind);
            take_to_inventory(host, p, kind, amount);
        }
        (action::DROP, Some(kind)) => drop_out(host, p, kind, 1),
        (action::DROP_STACK, Some(kind)) => {
            let amount = stack_of(&*host, kind);
            drop_out(host, p, kind, amount);
        }
        _ => {}
    }
    if request.action.0 != action::CLICK {
        return Ok(());
    }
    let Some(cursor) = cursor else {
        if let (true, Some(kind)) = (request.shift, entry) {
            let stack = stack_of(&*host, &kind);
            take_to_inventory(host, p, &kind, stack);
        }
        return Ok(());
    };
    match (request.shift, held, entry) {
        // Shift: a stack of the entry into the inventory.
        (true, _, Some(kind)) => {
            let stack = stack_of(&*host, &kind);
            take_to_inventory(host, p, &kind, stack);
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
pub fn holders(host: &dyn ServerHost, uuid: u128, index: i32, id: &str) -> Option<(String, Vec<(Pos, u64)>)> {
    let kind = seen(host, uuid, index, id)?;
    let (level, chest, cap) = session(host, uuid)?;
    let dimension = with(|s| s.sessions.get(&uuid).map(|x| x.dimension.clone()))?;
    let found = walk(host, level, chest, cap);
    let entry = with(|s| s.sessions.get(&uuid)?.entries.iter().find(|e| e.kind == kind).cloned())?;
    Some((dimension, entry.holders.iter().filter_map(|&(c, n)| Some((*found.get(c)?, n))).collect()))
}

/// Quick deposit: every stack of the main inventory (not the hotbar) into storage.
pub fn empty(host: &mut dyn ServerHost, p: PlayerRef) {
    for slot in 9..36 {
        store(host, p, ItemPlace::PlayerSlot { player: p.uuid(), slot }, None);
    }
    refresh_all(host);
}

/// The containers a mod crafting station at `pos` lends its recipe book: its network.
pub fn sources(host: &dyn ServerHost, uuid: u128, pos: Pos) -> Vec<Pos> {
    let Some(p) = player(host, uuid) else { return Vec::new() };
    let Some(level) = p.dimension(host).and_then(|d| host.level(&d.to_string())) else { return Vec::new() };
    walk(host, level, pos, skills::network(host, p))
}

/// The crafting recipe types a crafting table's book places.
const CRAFTING: [&str; 3] = ["minecraft:crafting", "minecraft:crafting_shaped", "minecraft:crafting_shapeless"];

/// A recipe-book click at a station beside the network (with the comfort): take what the
/// inventory lacks for it from the network. The game places the recipe after this hook (measured),
/// so its own placement then fills the grid from the inventory with one click.
pub fn recipe_clicked(host: &mut dyn ServerHost, uuid: u128, recipe: &str, all: bool) {
    let (Some((level, chest, cap)), Some(p)) = (session(&*host, uuid), player(&*host, uuid)) else { return };
    if !skills::has(&*host, p, skills::CRAFT) {
        return;
    }
    let Some(found) = CRAFTING.iter().find_map(|kind| host.recipes_of_type(kind).iter().find(|r| r.id == recipe).cloned()) else {
        return;
    };
    let mut have: HashMap<ItemResource, u64> = HashMap::new();
    for stack in p.inventory(&*host).unwrap_or_default() {
        *have.entry(ItemResource::of(&stack)).or_default() += u64::from(stack.count());
    }
    let (_, entries) = read(host, level, chest, cap);
    let stored: HashMap<ItemResource, u64> = entries.iter().map(|e| (e.kind.clone(), e.total)).collect();
    let mut kinds: Vec<&ItemResource> = have.keys().chain(stored.keys().filter(|k| !have.contains_key(*k))).collect();
    // Kinds the player holds first, then the most plentiful in the network.
    kinds.sort_by_key(|k| (!have.contains_key(*k), std::cmp::Reverse(stored.get(*k).copied().unwrap_or(0))));
    let slots: craft::Slots<ItemResource> = found
        .ingredients
        .iter()
        .map(|ing| {
            let empty = ing.items.is_empty() && ing.custom.is_none();
            (!empty).then(|| kinds.iter().filter(|k| ing.test(&*host, k.as_stack())).map(|k| (*k).clone()).collect())
        })
        .collect();
    let crafts = if all {
        let per = found.results.first().map_or(1, |r| u64::from(r.count().max(1)));
        let stack = found.results.first().and_then(|r| host.max_stack_size(r.item())).unwrap_or(64);
        craft::most(&slots, (u64::from(stack) / per).max(1), &have, &stored)
    } else {
        1
    };
    let Some(take_list) = craft::plan(&slots, crafts.max(1), &have, &stored) else { return };
    if take_list.is_empty() {
        return;
    }
    for (kind, n) in &take_list {
        take_to_inventory(host, p, kind, *n);
    }
}

/// What an item is, for its category: worn, a tool or weapon, or with durability is gear; with a
/// food component, food; in the item tag `storage:category/redstone`, redstone; else a block when
/// a block shares its id, else a material.
fn category(host: &dyn ServerHost, kind: &ItemResource) -> Category {
    let has = |id: &str| ComponentId::new(id).is_some_and(|c| kind.value(host, &c).is_some());
    let id = kind.item().id(host);
    let tagged = |tag: &str| item_tagged(host, kind, tag);
    organize::category(Traits {
        gear: has("minecraft:max_damage") || has("minecraft:tool") || has("minecraft:weapon") || has("minecraft:equippable"),
        food: has("minecraft:food"),
        redstone: tagged("storage:category/redstone"),
        block: host.registry_entry("minecraft:block", &id).is_some(),
    })
}

/// Whether the kind's item is in item tag `tag`.
pub(crate) fn item_tagged(host: &dyn ServerHost, kind: &ItemResource, tag: &str) -> bool {
    let id = kind.item().id(host);
    host.registry_entry("minecraft:item", &id).and_then(|e| host.registry_entry_in_tag(&e, tag)).unwrap_or(false)
}

/// Each container's label: the item in an item frame hanging on it.
fn labels(host: &mut dyn ServerHost, level: LevelRef, containers: &[Pos]) -> Vec<Option<Label>> {
    containers.iter().map(|&pos| label(host, level, pos)).collect()
}

/// The item frames in the loaded world, by dimension and entity id: `entities_in` leaves frames
/// out, so they are tracked as they join and leave their level.
static FRAMES: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());

fn frames<R>(f: impl FnOnce(&mut Vec<(String, u32)>) -> R) -> R {
    f(&mut FRAMES.lock().unwrap_or_else(|e| e.into_inner()))
}

fn is_frame(kind: &str) -> bool {
    kind == "minecraft:item_frame" || kind == "minecraft:glow_item_frame"
}

/// An entity joined a level: remember it when it is an item frame.
pub fn entity_joined(dimension: &str, entity: u32, kind: &str) {
    if is_frame(kind) {
        frames(|f| f.push((dimension.to_owned(), entity)));
    }
}

pub fn entity_left(entity: u32, kind: &str) {
    if is_frame(kind) {
        frames(|f| f.retain(|(_, id)| *id != entity));
    }
}

fn label(host: &mut dyn ServerHost, level: LevelRef, pos: Pos) -> Option<Label> {
    let dimension = level.dimension().id(&*host)?;
    let centre = pos.map(|c| f64::from(c) + 0.5);
    let near: Vec<u32> = frames(|f| f.iter().filter(|(d, _)| *d == dimension).map(|(_, id)| *id).collect());
    for id in near {
        let Some(frame) = host.entity(id) else { continue };
        let Some(at) = frame.position(&*host) else { continue };
        if (0..3).any(|i| (at[i] - centre[i]).abs() > 1.5) {
            continue;
        }
        // An item frame's data: `Facing` (0 down, 1 up, 2 north, 3 south, 4 west, 5 east) and `Item`.
        let Some(data) = frame.nbt(host) else { continue };
        let data = Tag::Compound(data);
        let (Some(Tag::Byte(facing)), Some(item)) = (data.get("Facing"), data.get("Item")) else { continue };
        let back: Pos = match facing {
            0 => [0, 1, 0],
            1 => [0, -1, 0],
            2 => [0, 0, 1],
            3 => [0, 0, -1],
            4 => [1, 0, 0],
            _ => [-1, 0, 0],
        };
        let block = [at[0].floor() as i32 + back[0], at[1].floor() as i32 + back[1], at[2].floor() as i32 + back[2]];
        if block != pos {
            continue;
        }
        let Some(Tag::String(id)) = item.get("id") else { continue };
        let id = id.to_string_lossy();
        let stack = mod_api::Identifier::parse(&id).ok().and_then(|i| host.item_key(&i)).and_then(|k| mod_api::ItemStack::new(k, 1))?;
        return Some(Label { category: category(&*host, &ItemResource::of(&stack)), id });
    }
    None
}

/// Slots used and slots in all, over the containers.
fn usage(host: &mut dyn ServerHost, level: LevelRef, containers: &[Pos]) -> (usize, usize) {
    let (mut used, mut total) = (0, 0);
    for &pos in containers {
        let Some(items) = level.capability(host, &ITEMS, pos, None) else { continue };
        let size = items.size(host);
        total += size;
        used += (0..size).filter(|&i| items.amount(host, i) > 0).count();
    }
    (used, total)
}

/// Sort chests: every kind taken out and put back in order (labelled homes first, then category,
/// then id), so partial stacks merge and the same items sit together. One transaction: if
/// anything would not fit back, nothing moves.
pub fn sort_chests(host: &mut dyn ServerHost, p: PlayerRef) {
    let Some((level, chest, cap)) = session(&*host, p.uuid()) else { return };
    sort_network(host, level, chest, cap);
}

/// Compacts and orders the network seen from `chest`, then refreshes every open panel.
fn sort_network(host: &mut dyn ServerHost, level: LevelRef, chest: Pos, cap: usize) {
    let (found, entries) = read(host, level, chest, cap);
    let labels = labels(host, level, &found);
    let mut kinds: Vec<(String, Category, bool, usize)> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let id = e.kind.item().id(&*host);
            let labelled = labels.iter().any(|l| l.as_ref().is_some_and(|l| l.id == id));
            (id, category(&*host, &e.kind), labelled, i)
        })
        .collect();
    organize::order(&mut kinds, |k| (k.0.as_str(), k.1, k.2));
    let result = host.transaction(|host, tx| {
        for e in &entries {
            for &(c, n) in &e.holders {
                if let Some(storage) = level.capability(host, &STORAGE, found[c], None) {
                    storage.extract(host, tx, &e.kind, n);
                }
            }
        }
        for (id, cat, _, i) in &kinds {
            let e = &entries[*i];
            let mut left = e.total;
            for c in organize::targets(id, *cat, &labels) {
                if left == 0 {
                    break;
                }
                if let Some(storage) = level.capability(host, &STORAGE, found[c], None) {
                    left -= storage.insert(host, tx, &e.kind, left).min(left);
                }
            }
            if left > 0 {
                return Err(id.clone());
            }
        }
        Ok(())
    });
    if let Err(id) = result {
        eprintln!("[storage] sort chests rolled back: {id} would not fit back");
    }
    refresh_all(host);
}

/// Deposit matching: every stack of the main inventory (not the hotbar) whose kind storage
/// already holds goes in.
pub fn deposit_matching(host: &mut dyn ServerHost, p: PlayerRef) {
    let Some((level, chest, cap)) = session(&*host, p.uuid()) else { return };
    let (_, entries) = read(host, level, chest, cap);
    for slot in 9..36 {
        let place = ItemPlace::PlayerSlot { player: p.uuid(), slot };
        if place.resource(host).is_some_and(|r| entries.iter().any(|e| e.kind == r)) {
            store(host, p, place, None);
        }
    }
    refresh_all(host);
}

/// Restock: every partial stack in the inventory (hotbar included) is filled from storage.
/// Sort the bag: the main inventory (slots 9 to 35; the hotbar and armour stay) with partial
/// stacks merged, ordered as Sort chests orders chests, in one transaction: if the stacks would
/// not go back, nothing moves.
pub fn sort_bag(host: &mut dyn ServerHost, p: PlayerRef) {
    let uuid = p.uuid();
    let bag = || (9..36).map(move |slot| ItemPlace::PlayerSlot { player: uuid, slot });
    let mut kinds: Vec<(ItemResource, u64)> = Vec::new();
    for place in bag() {
        let Some(kind) = place.resource(host) else { continue };
        let n = u64::from(place.amount(host));
        match kinds.iter_mut().find(|(k, _)| *k == kind) {
            Some(entry) => entry.1 += n,
            None => kinds.push((kind, n)),
        }
    }
    if kinds.is_empty() {
        return;
    }
    let mut order: Vec<(String, Category, bool, usize)> =
        kinds.iter().enumerate().map(|(i, (k, _))| (k.item().id(&*host), category(&*host, k), false, i)).collect();
    organize::order(&mut order, |k| (k.0.as_str(), k.1, k.2));
    let result = host.transaction(|host, tx| {
        for place in bag() {
            if let Some(kind) = place.resource(host) {
                let n = place.amount(host);
                place.extract(host, tx, &kind, n);
            }
        }
        let mut slots = bag();
        for (_, _, _, i) in &order {
            let (kind, total) = &kinds[*i];
            let max = u64::from(host.max_stack_size(kind.item()).unwrap_or(64));
            let mut left = *total;
            while left > 0 {
                let Some(place) = slots.next() else { return Err(()) };
                let put = u64::from(place.insert(host, tx, kind, left.min(max) as u32));
                if put == 0 {
                    return Err(());
                }
                left -= put;
            }
        }
        Ok(())
    });
    if result.is_err() {
        eprintln!("[storage] sorting the bag rolled back");
    }
}

pub fn restock(host: &mut dyn ServerHost, p: PlayerRef) {
    for slot in 0..36 {
        let place = ItemPlace::PlayerSlot { player: p.uuid(), slot };
        let Some(kind) = place.resource(host) else { continue };
        let max = u64::from(host.max_stack_size(kind.item()).unwrap_or(64));
        let room = max.saturating_sub(u64::from(place.amount(host)));
        if room > 0 {
            take(host, p, place, &kind, room);
        }
    }
}
