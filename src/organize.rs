//! Keeping chests tidy: which category an item belongs to, where each kind should live (the chest
//! labelled for it with an item frame, else any unlabelled chest), and the order Sort chests lays
//! kinds out in. Pure, so the rules are tested here; the server reads frames and tags.

/// What kind of thing an item is, in the order chests are sorted in and the tabs are shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// Tools, weapons and armour: anything with durability, a tool or weapon part, or worn.
    Gear,
    Food,
    /// Items in the item tag `storage:category/redstone` (a data file packs can extend).
    Redstone,
    /// Everything else that isn't a block: ingots, dyes, drops…
    Materials,
    /// Items that place a block of the same id.
    Blocks,
}

impl Category {
    /// Every category, in order (a closed set: the tabs and Sort chests follow it).
    pub const ALL: [Category; 5] = [Category::Gear, Category::Food, Category::Redstone, Category::Materials, Category::Blocks];

    pub fn index(self) -> u8 {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0) as u8
    }

    pub fn from_index(i: u8) -> Option<Self> {
        Self::ALL.get(i as usize).copied()
    }
}

/// What the server knows of a kind to place it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Traits {
    pub gear: bool,
    pub food: bool,
    pub redstone: bool,
    pub block: bool,
}

pub fn category(t: Traits) -> Category {
    if t.gear {
        Category::Gear
    } else if t.food {
        Category::Food
    } else if t.redstone {
        Category::Redstone
    } else if t.block {
        Category::Blocks
    } else {
        Category::Materials
    }
}

/// A chest's label, read from an item frame on it: the framed item's id and its category.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub id: String,
    pub category: Category,
}

/// Where a kind should go, best first: chests labelled with exactly this item, then chests
/// labelled with its category, then unlabelled chests, then the other labelled chests (only when
/// nothing else has room). Indices into `labels`, in walk order within each group.
pub fn targets(id: &str, category: Category, labels: &[Option<Label>]) -> Vec<usize> {
    let mut exact = Vec::new();
    let mut same = Vec::new();
    let mut free = Vec::new();
    let mut other = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        match label {
            Some(l) if l.id == id => exact.push(i),
            Some(l) if l.category == category => same.push(i),
            None => free.push(i),
            Some(_) => other.push(i),
        }
    }
    exact.into_iter().chain(same).chain(free).chain(other).collect()
}

/// The order Sort chests puts kinds in: kinds with a labelled home first (so they claim their
/// chests), then by category, then by id, so the same items always end up side by side.
pub fn order<T>(kinds: &mut [T], key: impl Fn(&T) -> (&str, Category, bool)) {
    kinds.sort_by(|a, b| {
        let (a, b) = (key(a), key(b));
        b.2.cmp(&a.2).then(a.1.cmp(&b.1)).then(a.0.cmp(b.0))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(id: &str, category: Category) -> Option<Label> {
        Some(Label { id: id.into(), category })
    }

    #[test]
    fn categories_follow_what_an_item_is() {
        assert_eq!(category(Traits { gear: true, block: false, ..Default::default() }), Category::Gear);
        assert_eq!(category(Traits { food: true, ..Default::default() }), Category::Food);
        assert_eq!(category(Traits { redstone: true, block: true, ..Default::default() }), Category::Redstone);
        assert_eq!(category(Traits { block: true, ..Default::default() }), Category::Blocks);
        assert_eq!(category(Traits::default()), Category::Materials);
        for c in Category::ALL {
            assert_eq!(Category::from_index(c.index()), Some(c));
        }
    }

    #[test]
    fn a_kind_goes_to_its_own_chest_then_its_categorys_then_free_ones_then_the_rest() {
        let labels = vec![
            label("minecraft:iron_pickaxe", Category::Gear),
            None,
            label("minecraft:cobblestone", Category::Blocks),
            label("minecraft:diamond_sword", Category::Gear),
            None,
        ];
        assert_eq!(targets("minecraft:diamond_sword", Category::Gear, &labels), vec![3, 0, 1, 4, 2]);
        assert_eq!(targets("minecraft:stone", Category::Blocks, &labels), vec![2, 1, 4, 0, 3]);
        assert_eq!(targets("minecraft:apple", Category::Food, &labels), vec![1, 4, 0, 2, 3]);
        // Every chest is offered exactly once.
        let mut all = targets("x", Category::Materials, &labels);
        all.sort();
        assert_eq!(all, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn sorting_puts_labelled_kinds_first_then_category_then_id() {
        let mut kinds = vec![
            ("minecraft:stone", Category::Blocks, false),
            ("minecraft:apple", Category::Food, false),
            ("minecraft:dirt", Category::Blocks, true),
            ("minecraft:bread", Category::Food, false),
        ];
        order(&mut kinds, |k| *k);
        let ids: Vec<&str> = kinds.iter().map(|k| k.0).collect();
        assert_eq!(ids, vec!["minecraft:dirt", "minecraft:apple", "minecraft:bread", "minecraft:stone"]);
    }
}
