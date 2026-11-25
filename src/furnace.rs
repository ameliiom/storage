//! Furnaces from chests (comfort `storage.furnace`): a furnace, smoker or blast furnace touching a
//! chest of a network takes what it can cook and fuel from the network when its slots are empty,
//! and puts finished results back. Only kinds in the item tag `storage:furnace_input` are cooked
//! (ores, raw metals and raw food: never the cobblestone or logs a player keeps) and only kinds in
//! `storage:furnace_fuel` are burned, never one this furnace could also cook; each slot holds one
//! stack at most. Furnaces are learned as players open them or their chests, and as they burn or
//! finish; each is looked at the tick after it burns or finishes, and once a second for new work.
use crate::chain::{self, Pos};
use crate::server;
use crate::skills;
#[allow(unused_imports)]
use mod_api::prelude::*;
use mod_api::{BlockMetadata, Host, HostBlockStates, HostTransaction, ITEMS, ItemResource, LevelRef, Recipe, ServerHost};
use std::sync::Mutex;

/// The item tags of what a furnace takes from the network.
const INPUT: &str = "storage:furnace_input";
const FUEL: &str = "storage:furnace_fuel";
/// A furnace's slots, as vanilla's `AbstractFurnaceBlockEntity` numbers them.
const SLOT_INPUT: usize = 0;
const SLOT_FUEL: usize = 1;
const SLOT_RESULT: usize = 2;
/// Ticks between looks at every known furnace for new work.
const EVERY: u32 = 20;
/// Vanilla cooking time when a recipe gives none.
const COOK: u32 = 200;

/// The recipe type each vanilla furnace cooks.
fn recipe_type(block: &str) -> Option<&'static str> {
    match block {
        "minecraft:furnace" => Some("minecraft:smelting"),
        "minecraft:smoker" => Some("minecraft:smoking"),
        "minecraft:blast_furnace" => Some("minecraft:blasting"),
        _ => None,
    }
}

/// How many fuel items cook `items` at `cook` ticks each, one burning `burn` ticks: at least one,
/// at most a stack.
pub fn fuel_needed(items: u32, cook: u32, burn: u32, stack: u32) -> u32 {
    if burn == 0 {
        return 0;
    }
    let ticks = u64::from(items) * u64::from(cook.max(1));
    (ticks.div_ceil(u64::from(burn)) as u32).clamp(1, stack.max(1))
}

/// The candidate with the most in the network (ties: the first), by index.
pub fn most(candidates: &[(usize, u64)]) -> Option<usize> {
    candidates.iter().fold(None, |best: Option<(usize, u64)>, &(i, n)| match best {
        Some((_, b)) if b >= n => best,
        _ if n > 0 => Some((i, n)),
        _ => best,
    }).map(|(i, _)| i)
}

/// Whether a kind may burn: tagged fuel that really burns and that this furnace can't cook.
pub fn may_burn(tagged: bool, burn: Option<i32>, cookable: bool) -> bool {
    tagged && burn.is_some_and(|b| b > 0) && !cookable
}

struct Known {
    dimension: String,
    pos: Pos,
}

struct State {
    known: Vec<Known>,
    /// Furnaces whose hook fired, to look at on the next tick (hooks give no dimension).
    woken: Vec<Pos>,
    ticks: u32,
}

static STATE: Mutex<State> = Mutex::new(State { known: Vec::new(), woken: Vec::new(), ticks: 0 });

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(&mut STATE.lock().unwrap_or_else(|e| e.into_inner()))
}

fn block(host: &dyn Host, level: LevelRef, pos: Pos) -> Option<&'static str> {
    let state = level.block_state(host, pos)?;
    let meta = host.block_states();
    let name = meta.name(state.block()).ok()?;
    recipe_type(name)
}

fn learn(dimension: &str, pos: Pos) {
    with(|s| {
        if !s.known.iter().any(|k| k.pos == pos && k.dimension == dimension) {
            s.known.push(Known { dimension: dimension.to_owned(), pos });
        }
    });
}

/// A player opened a container at `pos` or a network reaching `containers`: the furnaces among
/// them and touching them are remembered.
pub fn look_around(host: &dyn Host, dimension: &str, level: LevelRef, pos: Pos, containers: &[Pos]) {
    let around = containers.iter().flat_map(|&c| chain::SIDES.into_iter().map(move |s| chain::add(c, s)));
    for at in std::iter::once(pos).chain(around) {
        if block(host, level, at).is_some() {
            learn(dimension, at);
        }
    }
}

/// A furnace at `pos` burned fuel, finished an item or was emptied: it is looked at on the next
/// tick. The hook runs inside the furnace's own tick, where reading players or blocks hung the
/// server (measured), so it only notes the position.
pub fn woke(pos: Pos) {
    with(|s| {
        if !s.woken.contains(&pos) {
            s.woken.push(pos);
        }
    });
}

/// Where the woken furnace at `pos` stands: the dimensions it is known in, else those of the
/// players with the comfort where a furnace stands there (it is then remembered).
fn places(host: &dyn ServerHost, pos: Pos) -> Vec<String> {
    let mut places: Vec<String> = with(|s| s.known.iter().filter(|k| k.pos == pos).map(|k| k.dimension.clone()).collect());
    if places.is_empty() {
        for p in host.players().server_players() {
            if let Some(d) = p.dimension(host).map(|d| d.to_string())
                && !places.contains(&d)
                && skills::has(host, p, skills::FURNACE)
                && host.level(&d).is_some_and(|l| block(host, l, pos).is_some())
            {
                learn(&d, pos);
                places.push(d);
            }
        }
    }
    places
}

fn enabled(host: &dyn ServerHost) -> bool {
    host.setting(crate::MOD_ID, crate::setting::FURNACE_FEED).is_none_or(|v| v.trim() != "false")
}

/// How many containers a furnace in `dimension` reaches: the biggest network among the players
/// there with the comfort; None when no one there has it.
fn reach(host: &dyn ServerHost, dimension: &str) -> Option<usize> {
    host.players()
        .server_players()
        .into_iter()
        .filter(|p| p.dimension(host).is_some_and(|d| d.to_string() == dimension) && skills::has(host, *p, skills::FURNACE))
        .map(|p| skills::network(host, p))
        .max()
}

pub fn tick(host: &mut dyn ServerHost) {
    let (all, woken) = with(|s| {
        s.ticks = s.ticks.wrapping_add(1);
        (s.ticks.is_multiple_of(EVERY), std::mem::take(&mut s.woken))
    });
    if (!all && woken.is_empty()) || !enabled(&*host) {
        return;
    }
    let mut work: Vec<(String, Pos)> = woken.iter().flat_map(|&pos| places(&*host, pos).into_iter().map(move |d| (d, pos))).collect();
    if all {
        for k in with(|s| s.known.iter().map(|k| (k.dimension.clone(), k.pos)).collect::<Vec<_>>()) {
            if !work.contains(&k) {
                work.push(k);
            }
        }
    }
    let mut gone: Vec<(String, Pos)> = Vec::new();
    let mut moved = false;
    for (dimension, pos) in work {
        let Some(cap) = reach(&*host, &dimension) else { continue };
        let Some(level) = host.level(&dimension) else { continue };
        if !level.is_loaded(&*host, pos) {
            continue;
        }
        let Some(kind) = block(&*host, level, pos) else {
            gone.push((dimension, pos));
            continue;
        };
        moved |= serve(host, level, pos, cap, kind);
    }
    if !gone.is_empty() {
        with(|s| s.known.retain(|k| !gone.iter().any(|(d, p)| *d == k.dimension && *p == k.pos)));
    }
    if moved {
        server::refresh_all(host);
    }
}

/// Whether `kind` goes into a furnace cooking `recipes`, and the recipe's cooking time.
fn cooks(host: &dyn Host, recipes: &[Recipe], kind: &ItemResource) -> Option<u32> {
    recipes
        .iter()
        .find(|r| r.ingredients.first().is_some_and(|i| i.test(host, kind.as_stack())))
        .map(|r| r.cooking_time.unwrap_or(COOK))
}

/// One look at the furnace at `pos`: its result goes to the network, an empty input slot is filled
/// with what it can cook, an empty fuel slot with fuel for it. Whether anything moved.
fn serve(host: &mut dyn ServerHost, level: LevelRef, pos: Pos, cap: usize, kind: &str) -> bool {
    let Some(slots) = level.capability(host, &ITEMS, pos, None) else { return false };
    if slots.size(host) <= SLOT_RESULT {
        return false;
    }
    let result = slots.resource(host, SLOT_RESULT).filter(|_| slots.amount(host, SLOT_RESULT) > 0);
    let input_empty = slots.amount(host, SLOT_INPUT) == 0;
    let fuel_empty = slots.amount(host, SLOT_FUEL) == 0;
    if result.is_none() && !input_empty && !fuel_empty {
        return false;
    }
    let found = server::walk(&*host, level, pos, cap);
    if found.is_empty() {
        return false;
    }
    let mut moved = false;
    // The finished items go back into the network, all of them or none.
    if let Some(out) = result {
        let n = slots.amount(host, SLOT_RESULT);
        let done = host.transaction(|host, tx| {
            let taken = slots.extract(host, tx, SLOT_RESULT, &out, n);
            let stored = server::put_into(host, tx, level, &found, &out, u64::from(taken));
            if taken > 0 && stored == u64::from(taken) { Ok(()) } else { Err(()) }
        });
        moved |= done.is_ok();
    }
    let recipes = host.recipes_of_type(kind);
    if input_empty {
        let entries = server::entries_of(host, level, &found);
        let candidates: Vec<(usize, u64)> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| server::item_tagged(&*host, &e.kind, INPUT) && cooks(&*host, &recipes, &e.kind).is_some())
            .map(|(i, e)| (i, e.total))
            .collect();
        if let Some(i) = most(&candidates) {
            let e = &entries[i];
            let stack = u64::from(host.max_stack_size(e.kind.item()).unwrap_or(64));
            let want = e.total.min(stack);
            let done = host.transaction(|host, tx| {
                let taken = server::take_from(host, tx, level, &found, e, want);
                let went = slots.insert(host, tx, SLOT_INPUT, &e.kind, taken as u32);
                if taken > 0 && u64::from(went) == taken { Ok(()) } else { Err(()) }
            });
            moved |= done.is_ok();
        }
    }
    // Fuel only for something to cook, never a kind this furnace could cook itself.
    if fuel_empty && let Some(cooking) = slots.resource(host, SLOT_INPUT) {
        let count = slots.amount(host, SLOT_INPUT);
        let cook = cooks(&*host, &recipes, &cooking).unwrap_or(COOK);
        let entries = server::entries_of(host, level, &found);
        let candidates: Vec<(usize, u64)> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                may_burn(server::item_tagged(&*host, &e.kind, FUEL), host.item_burn_time(e.kind.item()), cooks(&*host, &recipes, &e.kind).is_some())
            })
            .map(|(i, e)| (i, e.total))
            .collect();
        if let Some(i) = most(&candidates) {
            let e = &entries[i];
            let burn = host.item_burn_time(e.kind.item()).unwrap_or(0).max(0) as u32;
            let stack = host.max_stack_size(e.kind.item()).unwrap_or(64);
            let want = u64::from(fuel_needed(count, cook, burn, stack)).min(e.total);
            let done = host.transaction(|host, tx| {
                let taken = server::take_from(host, tx, level, &found, e, want);
                let went = slots.insert(host, tx, SLOT_FUEL, &e.kind, taken as u32);
                if taken > 0 && u64::from(went) == taken { Ok(()) } else { Err(()) }
            });
            moved |= done.is_ok();
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuel_covers_the_input_within_one_stack() {
        // 64 raw iron at 200 ticks, coal burns 1600: 8 coal.
        assert_eq!(fuel_needed(64, 200, 1600, 64), 8);
        // 3 items need part of a coal: one.
        assert_eq!(fuel_needed(3, 200, 1600, 64), 1);
        // A blast furnace cooks in 100: 64 ores, 4 coal.
        assert_eq!(fuel_needed(64, 100, 1600, 64), 4);
        // Weak fuel never goes past a stack.
        assert_eq!(fuel_needed(64, 200, 100, 64), 64);
        // A coal block (16000) is one for a whole stack; a blaze rod stacks to 64 too.
        assert_eq!(fuel_needed(64, 200, 16000, 64), 1);
        assert_eq!(fuel_needed(5, 200, 0, 64), 0);
    }

    #[test]
    fn the_most_plentiful_kind_is_taken() {
        assert_eq!(most(&[]), None);
        assert_eq!(most(&[(0, 0)]), None);
        assert_eq!(most(&[(0, 5), (1, 64), (2, 64)]), Some(1));
        assert_eq!(most(&[(3, 1)]), Some(3));
    }

    #[test]
    fn what_can_be_cooked_is_never_burned() {
        // Coal: tagged, burns, not cookable.
        assert!(may_burn(true, Some(1600), false));
        // Logs a pack added to the fuel tag: they make charcoal here, so they stay.
        assert!(!may_burn(true, Some(300), true));
        // Planks are not tagged fuel: a player's building blocks never burn.
        assert!(!may_burn(false, Some(300), false));
        // Tagged but not a fuel in this game.
        assert!(!may_burn(true, None, false));
        assert!(!may_burn(true, Some(0), false));
    }

    #[test]
    fn only_the_three_vanilla_furnaces_cook() {
        assert_eq!(recipe_type("minecraft:furnace"), Some("minecraft:smelting"));
        assert_eq!(recipe_type("minecraft:smoker"), Some("minecraft:smoking"));
        assert_eq!(recipe_type("minecraft:blast_furnace"), Some("minecraft:blasting"));
        assert_eq!(recipe_type("minecraft:chest"), None);
    }
}
