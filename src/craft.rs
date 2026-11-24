//! One click on a recipe takes its ingredients from the chests: what to take out of the network
//! so the inventory holds enough for the recipe. Pure and generic over the item kind, so the
//! choice is tested here; the server matches kinds to ingredients and moves the items.

use std::collections::HashMap;
use std::hash::Hash;

/// For each ingredient slot of a recipe, the kinds that fit it, best first (the server lists the
/// kinds the player already holds before the network's, then by how many there are); None for an
/// empty cell of a shaped pattern. A slot no kind fits (`Some(vec![])`) makes the recipe
/// impossible.
pub type Slots<K> = Vec<Option<Vec<K>>>;

/// How many of each kind to take from the network so `crafts` crafts of the recipe can be
/// placed from the inventory. The inventory (`have`) is used first; the network (`stored`) fills
/// the rest. None when even both together are not enough for one craft.
pub fn plan<K: Clone + Eq + Hash>(slots: &Slots<K>, crafts: u64, have: &HashMap<K, u64>, stored: &HashMap<K, u64>) -> Option<HashMap<K, u64>> {
    let mut have = have.clone();
    let mut stored = stored.clone();
    let mut take: HashMap<K, u64> = HashMap::new();
    for fits in slots.iter().flatten() {
        let mut need = crafts;
        for kind in fits {
            let from_have = have.get(kind).copied().unwrap_or(0).min(need);
            if from_have > 0 {
                *have.entry(kind.clone()).or_default() -= from_have;
                need -= from_have;
            }
            if need == 0 {
                break;
            }
        }
        for kind in fits {
            if need == 0 {
                break;
            }
            let from_stored = stored.get(kind).copied().unwrap_or(0).min(need);
            if from_stored > 0 {
                *stored.entry(kind.clone()).or_default() -= from_stored;
                *take.entry(kind.clone()).or_default() += from_stored;
                need -= from_stored;
            }
        }
        if need > 0 {
            return None;
        }
    }
    Some(take)
}

/// How many crafts a shift-click asks for: as many as inventory and network allow, up to `most`
/// (a stack of the result). Found by trying the plan, largest first.
pub fn most<K: Clone + Eq + Hash>(slots: &Slots<K>, most: u64, have: &HashMap<K, u64>, stored: &HashMap<K, u64>) -> u64 {
    let (mut low, mut high) = (0, most);
    while low < high {
        let mid = (low + high).div_ceil(2);
        if plan(slots, mid, have, stored).is_some() { low = mid } else { high = mid - 1 }
    }
    low
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&'static str, u64)]) -> HashMap<&'static str, u64> {
        pairs.iter().copied().collect()
    }

    /// A wooden pickaxe: three planks (any wood) and two sticks, with empty grid slots.
    fn pickaxe() -> Slots<&'static str> {
        let planks = Some(vec!["oak_planks", "birch_planks"]);
        let stick = Some(vec!["stick"]);
        vec![planks.clone(), planks.clone(), planks, None, stick.clone(), None, None, stick, None]
    }

    #[test]
    fn the_inventory_is_used_first_and_the_network_fills_the_rest() {
        let take = plan(&pickaxe(), 1, &map(&[("oak_planks", 1), ("stick", 2)]), &map(&[("oak_planks", 10), ("stick", 5)])).unwrap();
        assert_eq!(take, map(&[("oak_planks", 2)]));
    }

    #[test]
    fn any_fitting_kind_is_used_and_nothing_is_taken_when_it_cannot_be_made() {
        let take = plan(&pickaxe(), 1, &map(&[]), &map(&[("oak_planks", 1), ("birch_planks", 2), ("stick", 2)])).unwrap();
        assert_eq!(take, map(&[("oak_planks", 1), ("birch_planks", 2), ("stick", 2)]));
        assert_eq!(plan(&pickaxe(), 1, &map(&[]), &map(&[("oak_planks", 3), ("stick", 1)])), None);
    }

    #[test]
    fn nothing_is_taken_when_the_inventory_already_has_it() {
        assert_eq!(plan(&pickaxe(), 1, &map(&[("birch_planks", 3), ("stick", 2)]), &map(&[("oak_planks", 64)])), Some(map(&[])));
    }

    #[test]
    fn an_ingredient_nothing_fits_takes_nothing() {
        // A bow with no string anywhere: the sticks must stay in the chests.
        let bow: Slots<&str> = vec![None, Some(vec!["stick"]), Some(vec![]), Some(vec!["stick"]), None, Some(vec![]), None, Some(vec!["stick"]), Some(vec![])];
        assert_eq!(plan(&bow, 1, &map(&[]), &map(&[("stick", 32)])), None);
    }

    #[test]
    fn a_shift_click_makes_as_many_as_there_is_for() {
        assert_eq!(most(&pickaxe(), 64, &map(&[]), &map(&[("oak_planks", 9), ("stick", 100)])), 3);
        assert_eq!(most(&pickaxe(), 2, &map(&[]), &map(&[("oak_planks", 90), ("stick", 100)])), 2);
        assert_eq!(most(&pickaxe(), 64, &map(&[]), &map(&[("stick", 100)])), 0);
    }

    #[test]
    fn a_plan_never_takes_more_than_stored() {
        let stored = map(&[("oak_planks", 7), ("birch_planks", 4), ("stick", 9)]);
        for crafts in 0..6 {
            if let Some(take) = plan(&pickaxe(), crafts, &map(&[]), &stored) {
                for (k, n) in &take {
                    assert!(*n <= stored[k]);
                }
                assert_eq!(take.values().sum::<u64>(), crafts * 5);
            }
        }
    }
}
