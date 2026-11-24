//! Which containers a terminal reaches: every linked block (`storage:linked`) touching the
//! terminal or touching one that does, nearest first, up to a cap. Pure: the server says what
//! each position holds; the walk is tested here.
//!
//! A double chest is two blocks that both answer for all 54 slots, so only one half of it is
//! counted (the walk still passes through both, so chests behind it are reached).
use std::collections::{BTreeSet, VecDeque};

pub type Pos = [i32; 3];

/// What stands at a position, as far as the walk cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// Not linked: the walk stops here.
    Other,
    /// A linked container that answers for its own slots.
    Container,
    /// One half of a double chest: `counted` for the half that is read, not for the other.
    Half { counted: bool },
}

const SIDES: [Pos; 6] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];

fn add(a: Pos, b: Pos) -> Pos {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The containers to read, nearest first (breadth first from the terminal), at most `cap` of them.
pub fn walk(terminal: Pos, cap: usize, at: impl Fn(Pos) -> Block) -> Vec<Pos> {
    let mut seen = BTreeSet::from([terminal]);
    let mut queue = VecDeque::from([terminal]);
    let mut found = Vec::new();
    while let Some(pos) = queue.pop_front() {
        for side in SIDES {
            let next = add(pos, side);
            if !seen.insert(next) {
                continue;
            }
            let counted = match at(next) {
                Block::Other => continue,
                Block::Container => true,
                Block::Half { counted } => counted,
            };
            if counted {
                if found.len() == cap {
                    return found;
                }
                found.push(next);
            }
            queue.push_back(next);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn world(blocks: &[(Pos, Block)]) -> impl Fn(Pos) -> Block + '_ {
        let map: BTreeMap<Pos, Block> = blocks.iter().copied().collect();
        move |p| map.get(&p).copied().unwrap_or(Block::Other)
    }

    #[test]
    fn a_wall_of_chests_is_reached_through_each_other_nearest_first() {
        let blocks: Vec<(Pos, Block)> = (1..=4).map(|x| ([x, 0, 0], Block::Container)).collect();
        let found = walk([0, 0, 0], 256, world(&blocks));
        assert_eq!(found, vec![[1, 0, 0], [2, 0, 0], [3, 0, 0], [4, 0, 0]]);
    }

    #[test]
    fn a_gap_stops_the_chain() {
        let blocks = [([1, 0, 0], Block::Container), ([3, 0, 0], Block::Container)];
        assert_eq!(walk([0, 0, 0], 256, world(&blocks)), vec![[1, 0, 0]]);
    }

    #[test]
    fn a_double_chest_counts_once_and_still_leads_on() {
        let blocks = [
            ([1, 0, 0], Block::Half { counted: false }),
            ([2, 0, 0], Block::Half { counted: true }),
            ([3, 0, 0], Block::Container),
        ];
        assert_eq!(walk([0, 0, 0], 256, world(&blocks)), vec![[2, 0, 0], [3, 0, 0]]);
    }

    #[test]
    fn the_cap_keeps_the_nearest() {
        let blocks: Vec<(Pos, Block)> = (1..=10).map(|x| ([x, 0, 0], Block::Container)).collect();
        assert_eq!(walk([0, 0, 0], 3, world(&blocks)), vec![[1, 0, 0], [2, 0, 0], [3, 0, 0]]);
    }

    #[test]
    fn a_big_room_is_walked_once_per_block() {
        // A 16 × 8 × 8 room of chests, the terminal in a corner: every chest exactly once.
        let mut blocks = Vec::new();
        for x in 0..16 {
            for y in 0..8 {
                for z in 0..8 {
                    if [x, y, z] != [0, 0, 0] {
                        blocks.push(([x, y, z], Block::Container));
                    }
                }
            }
        }
        let found = walk([0, 0, 0], 10_000, world(&blocks));
        assert_eq!(found.len(), blocks.len());
        assert_eq!(found.iter().collect::<BTreeSet<_>>().len(), found.len());
    }
}
