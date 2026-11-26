//! The merged view: every kind of item across the network as one entry with its total, and which
//! containers hold it. Generic over the kind (`K`: the server's item resource, or a plain key in
//! tests), so merging, searching, sorting and splitting a take are all tested here without a game.
use std::collections::{HashMap, HashSet};

/// One kind of item across the network.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry<K> {
    pub kind: K,
    pub total: u64,
    /// Container index (in walk order) and how many of this kind it holds.
    pub holders: Vec<(usize, u64)>,
}

/// Merges what each container holds into one entry per kind, in first-seen order.
pub fn merge<K: Eq + std::hash::Hash + Clone>(containers: impl IntoIterator<Item = Vec<(K, u64)>>) -> Vec<Entry<K>> {
    let mut index: HashMap<K, usize> = HashMap::new();
    let mut entries: Vec<Entry<K>> = Vec::new();
    for (c, held) in containers.into_iter().enumerate() {
        for (kind, n) in held {
            if n == 0 {
                continue;
            }
            let i = *index.entry(kind.clone()).or_insert_with(|| {
                entries.push(Entry { kind, total: 0, holders: Vec::new() });
                entries.len() - 1
            });
            let e = &mut entries[i];
            e.total += n;
            match e.holders.last_mut() {
                Some((last, m)) if *last == c => *m += n,
                _ => e.holders.push((c, n)),
            }
        }
    }
    entries
}

/// How the list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// Most first.
    #[default]
    Count,
    /// By the name the player reads.
    Name,
    /// By mod (namespace), then name.
    Mod,
    /// By item id.
    Id,
    /// What changed last first.
    Recent,
}

impl Sort {
    /// Every order, in the order the sort button steps through them.
    pub const ALL: [Sort; 5] = [Sort::Count, Sort::Name, Sort::Mod, Sort::Id, Sort::Recent];

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// What the client knows of an entry to search and sort it.
pub struct Shown<'a> {
    /// The translated name, in the player's language.
    pub name: &'a str,
    /// `minecraft:stone`.
    pub id: &'a str,
    pub total: u64,
    /// When its total last changed (a counter the client keeps; larger is later).
    pub changed: u64,
    /// Its tooltip in lower case (enchantments, effects, lore), read only by a `$` word; empty
    /// until the client fills it.
    pub tip: &'a str,
}

/// The search box's meaning: words that must all be in the name (or id), `@mod` for a
/// namespace, `#tag` for a tag (`has_tag` answers it), `$text` for words in the tooltip
/// (`$sharp` finds Sharpness). Case and spaces around are ignored.
pub fn matches(query: &str, shown: &Shown, has_tag: impl Fn(&str) -> bool) -> bool {
    let name = shown.name.to_lowercase();
    let id = shown.id.to_lowercase();
    let namespace = id.split(':').next().unwrap_or("");
    query.split_whitespace().all(|word| {
        let word = word.to_lowercase();
        if let Some(m) = word.strip_prefix('@') {
            namespace.starts_with(m)
        } else if let Some(t) = word.strip_prefix('#') {
            !t.is_empty() && has_tag(t)
        } else if let Some(t) = word.strip_prefix('$') {
            shown.tip.contains(t)
        } else {
            name.contains(&word) || id.contains(&word)
        }
    })
}

/// Orders shown entries (indices into `shown`), reversed when `reverse`; ties fall back to the
/// id so the order is stable.
pub fn order(shown: &[Shown], sort: Sort, reverse: bool) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..shown.len()).collect();
    idx.sort_by(|&a, &b| {
        let (a, b) = (&shown[a], &shown[b]);
        let by = match sort {
            Sort::Count => b.total.cmp(&a.total),
            Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Sort::Mod => {
                let ns = |s: &str| s.split(':').next().unwrap_or("").to_owned();
                ns(a.id).cmp(&ns(b.id)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }
            Sort::Id => a.id.cmp(b.id),
            Sort::Recent => b.changed.cmp(&a.changed),
        };
        let by = by.then_with(|| a.id.cmp(b.id));
        if reverse { by.reverse() } else { by }
    });
    idx
}

/// Groups variants of one item (a worn sword and an enchanted one share the id): of `shown`
/// (indices into `ids`, in order), each id with two or more kinds keeps only its first, at its
/// place, unless the id is `open`. Also gives each group head's total and how many kinds it holds.
pub fn group(ids: &[String], totals: &[u64], shown: Vec<usize>, open: &HashSet<String>) -> (Vec<usize>, HashMap<usize, (u64, usize)>) {
    let mut sums: HashMap<&str, (u64, usize)> = HashMap::new();
    for &i in &shown {
        let s = sums.entry(ids[i].as_str()).or_default();
        s.0 += totals[i];
        s.1 += 1;
    }
    let mut heads = HashMap::new();
    let mut seen = HashSet::new();
    let kept = shown
        .into_iter()
        .filter(|&i| {
            let id = ids[i].as_str();
            let (sum, kinds) = sums[id];
            if kinds < 2 || open.contains(id) {
                return true;
            }
            let first = seen.insert(id);
            if first {
                heads.insert(i, (sum, kinds));
            }
            first
        })
        .collect();
    (kept, heads)
}

/// A pinned item: its id and the least of it the player wants kept (0: no minimum).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub id: String,
    pub min: u64,
}

/// The pins file: one pin a line, `id` or `id minimum`. Blank and broken lines are skipped; a
/// repeated id keeps its first line.
pub fn read_pins(text: &str) -> Vec<Pin> {
    let mut pins: Vec<Pin> = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let Some(id) = words.next() else { continue };
        let min = match words.next() {
            None => 0,
            Some(n) => match n.parse() {
                Ok(n) => n,
                Err(_) => continue,
            },
        };
        if !pins.iter().any(|p| p.id == id) {
            pins.push(Pin { id: id.to_owned(), min });
        }
    }
    pins
}

pub fn write_pins(pins: &[Pin]) -> String {
    pins.iter().map(|p| if p.min > 0 { format!("{} {}", p.id, p.min) } else { p.id.clone() }).collect::<Vec<_>>().join("\n")
}

/// Whether a total is below a pin's minimum (no minimum: never).
pub fn low(min: u64, total: u64) -> bool {
    min > 0 && total < min
}

/// A total as an item count shows it: up to 999 as is, then `1.2k`, `15k`, `1.2M`, Latin digits.
pub fn count_text(n: u64) -> String {
    fn short(n: u64, unit: u64, suffix: &str) -> String {
        let tenths = n * 10 / unit;
        if tenths < 100 && !tenths.is_multiple_of(10) {
            format!("{}.{}{suffix}", tenths / 10, tenths % 10)
        } else {
            format!("{}{suffix}", n / unit)
        }
    }
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => short(n, 1_000, "k"),
        1_000_000..=999_999_999 => short(n, 1_000_000, "M"),
        _ => short(n, 1_000_000_000, "G"),
    }
}

/// Splits taking `want` of an entry across its holders, nearest container first: how many from
/// which container. Never more than a holder has, never more than `want`.
pub fn split_take(holders: &[(usize, u64)], want: u64) -> Vec<(usize, u64)> {
    let mut left = want;
    let mut plan = Vec::new();
    for &(c, has) in holders {
        if left == 0 {
            break;
        }
        let n = has.min(left);
        if n > 0 {
            plan.push((c, n));
            left -= n;
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_kinds_merge_across_containers_with_their_holders() {
        let entries = merge(vec![
            vec![("stone", 64), ("dirt", 10), ("stone", 32)],
            vec![("dirt", 5)],
            vec![("stone", 1), ("air", 0)],
        ]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], Entry { kind: "stone", total: 97, holders: vec![(0, 96), (2, 1)] });
        assert_eq!(entries[1], Entry { kind: "dirt", total: 15, holders: vec![(0, 10), (1, 5)] });
    }

    #[test]
    fn search_reads_names_ids_mods_and_tags_in_any_language() {
        let shown = Shown { name: "حجر مصقول", id: "minecraft:polished_andesite", total: 3, changed: 0, tip: "polished andesite\nsharpness iii" };
        let no_tag = |_: &str| false;
        assert!(matches("", &shown, no_tag));
        assert!(matches("حجر", &shown, no_tag));
        assert!(matches("ANDES", &shown, no_tag));
        assert!(matches("@mine", &shown, no_tag));
        assert!(!matches("@create", &shown, no_tag));
        assert!(matches("#stone", &shown, |t| t == "stone"));
        assert!(!matches("#", &shown, |_| true));
        assert!(!matches("حجر خشب", &shown, no_tag));
        assert!(matches("$sharp", &shown, no_tag));
        assert!(matches("$SHARPNESS حجر", &shown, no_tag));
        assert!(!matches("$fire", &shown, no_tag));
    }

    #[test]
    fn sorting_is_stable_and_follows_the_choice() {
        let shown = [
            Shown { name: "Dirt", id: "minecraft:dirt", total: 5, changed: 2, tip: "" },
            Shown { name: "apple", id: "minecraft:apple", total: 5, changed: 7, tip: "" },
            Shown { name: "Gear", id: "create:gear", total: 9, changed: 1, tip: "" },
        ];
        assert_eq!(order(&shown, Sort::Count, false), vec![2, 1, 0]);
        assert_eq!(order(&shown, Sort::Count, true), vec![0, 1, 2]);
        assert_eq!(order(&shown, Sort::Name, false), vec![1, 0, 2]);
        assert_eq!(order(&shown, Sort::Mod, false), vec![2, 1, 0]);
        assert_eq!(order(&shown, Sort::Id, false), vec![2, 1, 0]);
        assert_eq!(order(&shown, Sort::Recent, false), vec![1, 0, 2]);
        // The button steps through every order and comes back.
        let mut s = Sort::Count;
        for _ in 0..Sort::ALL.len() {
            s = s.next();
        }
        assert_eq!(s, Sort::Count);
    }

    #[test]
    fn counts_read_short_with_latin_digits() {
        let cases = [(0, "0"), (999, "999"), (1_000, "1k"), (1_234, "1.2k"), (15_400, "15k"), (999_999, "999k"), (1_250_000, "1.2M"), (3_000_000_000, "3G")];
        for (n, text) in cases {
            assert_eq!(count_text(n), text, "{n}");
            assert!(count_text(n).is_ascii());
        }
    }

    #[test]
    /// (That both languages hold the same keys is the workspace's `tools/lang_gate.py`.)
    fn every_key_the_code_shows_has_its_line() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let en: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("assets/storage/lang/en_us.json")).unwrap()).unwrap();
        // The panel (client.rs) is the only code that shows text: every `storage.` key it names.
        let text = std::fs::read_to_string(root.join("src/client.rs")).unwrap();
        for part in text.split('"').skip(1).step_by(2).filter(|p| p.starts_with("storage.") && !p.contains(' ')) {
            assert!(en.get(part).is_some(), "the panel shows {part}, missing from en_us");
        }
    }

    /// A tiny deterministic generator, so the property tests need no crate.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % below.max(1)
        }
    }

    #[test]
    fn a_take_never_overdraws_and_takes_what_it_can() {
        let mut rng = Rng(7);
        for _ in 0..2_000 {
            let holders: Vec<(usize, u64)> = (0..rng.next(6)).map(|c| (c as usize, rng.next(200))).collect();
            let total: u64 = holders.iter().map(|h| h.1).sum();
            let want = rng.next(400);
            let plan = split_take(&holders, want);
            assert_eq!(plan.iter().map(|p| p.1).sum::<u64>(), want.min(total));
            for (c, n) in plan {
                assert!(n > 0 && n <= holders.iter().find(|h| h.0 == c).unwrap().1);
            }
        }
    }

    #[test]
    fn two_players_taking_at_once_never_change_the_grand_total() {
        // The server applies one request at a time against the current containers, as the
        // real transaction does: whatever order two players' takes arrive in, what left the
        // containers is exactly what the players received.
        let mut rng = Rng(42);
        for _ in 0..500 {
            let mut chests: Vec<u64> = (0..8).map(|_| rng.next(128)).collect();
            let start: u64 = chests.iter().sum();
            let mut received = [0u64; 2];
            for step in 0..20 {
                let player = step % 2;
                let holders: Vec<(usize, u64)> = chests.iter().copied().enumerate().collect();
                for (c, n) in split_take(&holders, rng.next(100)) {
                    chests[c] -= n;
                    received[player] += n;
                }
            }
            assert_eq!(chests.iter().sum::<u64>() + received[0] + received[1], start);
        }
    }

    #[test]
    fn variants_of_one_item_show_as_one_group_until_opened() {
        let ids: Vec<String> = ["sword", "dirt", "sword", "sword", "apple"].map(String::from).to_vec();
        let totals = [1, 40, 1, 2, 3];
        let (kept, heads) = group(&ids, &totals, vec![1, 2, 0, 4, 3], &HashSet::new());
        assert_eq!(kept, vec![1, 2, 4]);
        assert_eq!(heads, HashMap::from([(2, (4, 3))]));
        let open = HashSet::from(["sword".to_owned()]);
        let (kept, heads) = group(&ids, &totals, vec![1, 2, 0, 4, 3], &open);
        assert_eq!(kept, vec![1, 2, 0, 4, 3]);
        assert!(heads.is_empty());
        // A search that leaves one variant shows it alone, not as a group.
        let (kept, heads) = group(&ids, &totals, vec![3, 1], &HashSet::new());
        assert_eq!((kept, heads.len()), (vec![3, 1], 0));
    }

    #[test]
    fn pins_keep_their_minimums_through_the_file() {
        let pins = read_pins("minecraft:torch 128\n\nminecraft:apple\nbroken line here\nminecraft:torch 5\nminecraft:coal x\n");
        assert_eq!(
            pins,
            vec![Pin { id: "minecraft:torch".into(), min: 128 }, Pin { id: "minecraft:apple".into(), min: 0 }]
        );
        assert_eq!(read_pins(&write_pins(&pins)), pins);
        assert_eq!(write_pins(&pins), "minecraft:torch 128\nminecraft:apple");
        assert!(low(128, 127) && !low(128, 128) && !low(0, 0));
    }
}
