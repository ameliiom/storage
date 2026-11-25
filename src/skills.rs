//! The comforts earned in the Skills tree Organization, with no code tie to Skills: an unlocked
//! node gives the player a vanilla tag (`/tag`) that Storage reads (`storage.find`,
//! `storage.quick_deposit`, `storage.network.<n>`), Skills earns tree experience from Storage's own
//! statistic, and the tree ships as a data file only Skills reads. Without Skills, the server's
//! `all_comforts` setting opens them.
use crate::payloads::{Comfort, comfort};
use crate::server;
#[allow(unused_imports)]
use mod_api::prelude::*;
use mod_api::{IPayloadContext, PlayerRef, Registrar, ServerHost, StatType};
use std::sync::Mutex;

/// The tag that lights the chests holding an item.
pub const FIND: &str = "storage.find";
/// The tag that shows the button emptying the inventory into storage.
pub const EMPTY: &str = "storage.quick_deposit";
/// The tag that lets a recipe-book click at a crafting table take its ingredients from the chests.
pub const CRAFT: &str = "storage.craft";
/// Tags `storage.network.<n>`: how many containers a chest shows together; 16 without any.
const NETWORK: &str = "storage.network.";
const NETWORK_START: usize = 16;
const NETWORK_MAX: usize = 4096;
/// `storage:items_moved`: every item a player moves through the storage panel.
const MOVED: &str = "items_moved";
/// How long found chests glow, in server ticks, and how often they puff.
const GLOW_TICKS: u32 = 200;
const GLOW_EVERY: u32 = 10;

pub fn register(r: &mut dyn Registrar) {
    if let Err(e) = r.register_custom_stat(MOVED) {
        eprintln!("[storage] declaring statistic {MOVED}: {e}");
    }
}

fn all(host: &dyn ServerHost) -> bool {
    host.setting(crate::MOD_ID, crate::setting::ALL_COMFORTS).is_some_and(|v| v.trim() == "true")
}

fn tags(host: &dyn ServerHost, p: PlayerRef) -> Vec<String> {
    p.entity().tags(host).unwrap_or_default()
}

/// Whether a player has the comfort `tag`.
pub fn has(host: &dyn ServerHost, p: PlayerRef, tag: &str) -> bool {
    all(host) || tags(host, p).iter().any(|t| t == tag)
}

/// How many containers a player's chests show together.
pub fn network(host: &dyn ServerHost, p: PlayerRef) -> usize {
    if all(host) {
        return NETWORK_MAX;
    }
    network_of(&tags(host, p))
}

/// The largest `storage.network.<n>` among `tags`, at least the start.
fn network_of(tags: &[String]) -> usize {
    tags.iter()
        .filter_map(|t| t.strip_prefix(NETWORK)?.parse::<usize>().ok())
        .fold(NETWORK_START, usize::max)
        .min(NETWORK_MAX)
}

/// Counts moved items toward Organization's experience.
pub fn moved(host: &mut dyn ServerHost, p: PlayerRef, n: u64) {
    let amount = n.min(i32::MAX as u64) as i32;
    if !p.award_stat(host, StatType::Custom, &format!("{}:{MOVED}", crate::MOD_ID), amount) {
        eprintln!("[storage] could not award {MOVED}");
    }
}

struct Glow {
    dimension: String,
    at: Vec<[i32; 3]>,
    left: u32,
}

static GLOWS: Mutex<Vec<Glow>> = Mutex::new(Vec::new());

fn glows<R>(f: impl FnOnce(&mut Vec<Glow>) -> R) -> R {
    f(&mut GLOWS.lock().unwrap_or_else(|e| e.into_inner()))
}

pub fn comfort(request: Comfort, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let p: PlayerRef = cx.player().map_err(|e| format!("{e:?}"))?;
    let host = cx.server_host().map_err(|e| format!("{e:?}"))?;
    match request.kind.0 {
        comfort::FIND if has(&*host, p, FIND) => {
            if let Some((dimension, at)) = server::holders(&*host, p.uuid(), request.index.0, &request.id) {
                glows(|g| g.push(Glow { dimension, at, left: GLOW_TICKS }));
            }
        }
        comfort::EMPTY if has(&*host, p, EMPTY) => server::empty(host, p),
        comfort::SORT => server::sort_chests(host, p),
        comfort::MATCH => server::deposit_matching(host, p),
        comfort::RESTOCK => server::restock(host, p),
        _ => {}
    }
    Ok(())
}

/// Each server tick: found chests puff glowing particles for everyone to see.
pub fn tick(host: &mut dyn ServerHost, ticks: u32) {
    if !ticks.is_multiple_of(GLOW_EVERY) {
        return;
    }
    let lit: Vec<(String, Vec<[i32; 3]>)> = glows(|g| {
        g.retain_mut(|x| {
            x.left = x.left.saturating_sub(GLOW_EVERY);
            x.left > 0
        });
        g.iter().map(|x| (x.dimension.clone(), x.at.clone())).collect()
    });
    for (dimension, at) in lit {
        for [x, y, z] in at {
            let line = format!(
                "execute in {dimension} run particle minecraft:glow {:.1} {:.1} {:.1} 0.35 0.35 0.35 0 6 force",
                x as f64 + 0.5,
                y as f64 + 0.6,
                z as f64 + 0.5
            );
            host.commands().run_command(&line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_biggest_network_tag_wins_within_bounds() {
        let t = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(network_of(&t(&[])), 16);
        assert_eq!(network_of(&t(&["storage.find", "storage.network.64"])), 64);
        assert_eq!(network_of(&t(&["storage.network.64", "storage.network.1024"])), 1024);
        assert_eq!(network_of(&t(&["storage.network.8"])), 16);
        assert_eq!(network_of(&t(&["storage.network.99999", "storage.network.x"])), 4096);
    }

    /// The tree file and this code name the same tags: every tag perk in the shipped
    /// Organization tree is one Storage reads, and every comfort Storage reads is in the tree.
    #[test]
    fn the_shipped_tree_gives_exactly_the_tags_storage_reads() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/storage/skills/organization.json");
        let tree: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let given: Vec<String> = tree["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|n| n["perks"].as_array().unwrap().iter())
            .filter_map(|p| p["tag"].as_str().map(str::to_owned))
            .collect();
        for tag in &given {
            let read = tag == FIND || tag == EMPTY || tag == CRAFT || tag.strip_prefix(NETWORK).is_some_and(|n| n.parse::<usize>().is_ok());
            assert!(read, "the tree gives {tag}, which Storage never reads");
        }
        for wanted in [FIND, EMPTY, CRAFT] {
            assert!(given.iter().any(|t| t == wanted), "no node gives {wanted}");
        }
        assert!(given.iter().any(|t| network_of(std::slice::from_ref(t)) > NETWORK_START));
    }
}
