# Storage: design

Open any chest that touches other chests, and beside the chest's own screen a panel shows
everything in all of them as one list: the same items merged into one entry with their total,
a search box, a sort button. Click to take, click with something on the cursor to put it in.
No new block, no cables, no separate screen: the chest, its slots, your inventory and the cursor
stay the game's own.

## 1. Domain map

| Concept | Owner | Stored? |
|---|---|---|
| **Container**: a chest, trapped chest, barrel, shulker box, or any block in the tag `storage:linked` | the world | vanilla's own |
| **Network**: the opened container and every linked block touching it, or touching one that does | derived on each open, nearest first | never |
| **Entry**: one kind of item (same item and components) and its total across the network | derived from the containers | never |
| **View**: search text, sort, scroll | each player's client | client only |
| **Comforts**: Find, quick deposit, network size | tags on the player, given by the Skills tree Organization | vanilla's tags |

The containers are the only truth: Storage keeps no copy of any item, and removing the mod
leaves every item where it was.

## 2. How it plays

- **Open any chest** (barrel, shulker box…). When it touches others, the panel appears beside the
  chest's screen (left, or right when there's no room; 3 to 9 columns as fit; as tall as the
  chest's screen). A chest alone shows no panel.
- **Take**: click an entry: a stack to the cursor (right-click: half a stack). Shift-click: a stack
  straight into the inventory. Nearest containers give first.
- **Put**: with something on the cursor, click the list: all of it goes in (right-click: one).
  It fills stacks already in storage first, then empty slots, as hoppers do. Shift-clicks in the
  inventory move into the opened chest, as always.
- **Search** in any language; `@mod` for a mod's items, `#tag` for an item tag. The search is
  kept for the next chest and always shown.
- **Sort**: by count, name or mod (one button, its icon showing which).
- **Live**: every move refreshes every player's open panel at once; other changes (hoppers, a
  player at another chest) show within a second.

## 3. Comforts earned in Skills (the Organization tree)

| Always | Earned in Organization |
|---|---|
| The panel for up to 16 joined chests, take, put, search, sort | **Where Is It?** (F over an entry: its chests glow for 10 s, seen by everyone) → **Quick Deposit** (a button puts the whole inventory but the hotbar in) · **Bigger Network** I/II/III (64, 256, 1024 chests) |

- Experience: items moved through the panel (Storage's statistic `storage:items_moved`), opening
  chests, barrels and shulker boxes.
- **No code tie between the mods.** An unlocked node gives the player a vanilla tag
  (`storage.find`, `storage.quick_deposit`, `storage.network.64`…), which Storage reads; Skills
  gained a generic "tag" perk kind for this. The tree is a data file in Storage's pack that only
  Skills reads. Without Skills, the server setting `all_comforts` opens everything.
- First tried as Storage's own attributes: Horizon f037744 does not register attributes a native
  mod declares (`declare_attribute` returns Ok, the attribute catalog never has them), so tags
  carry the comforts instead.

## 4. Choices (compared, one picked)

| Question | Picked | Over | Why |
|---|---|---|---|
| Where the list lives | beside any chest's own screen | a terminal block with its own menu | the owner's rule: build on what the player already uses; no new block |
| How chests join | touching each other | cables or connectors | no chores |
| What a click moves | the game's own cursor (`ItemPlace::PlayerCursor`) | a cursor drawn by the mod | native feel: carry, place, drop as always |

## 5. Measured (owner's PC, Horizon f037744, 2026-11-24)

| What | Measured |
|---|---|
| Reading 50 chests (1350 slots, 170 kinds) and merging | 1.2 ms on open, 0.9 ms per take |
| The view sent to the client | about 4.9 KB (≈ 29 bytes a kind) |
| A take of 64 / a put / quick deposit | items arrive whole, totals conserved (checked with `/clear … 0` and the list) |

## 6. Safety

- Every move is one transaction: what leaves the containers reaches the player (or the cursor
  stays as it was); a put moves nothing unless all of it fits.
- A request names the entry by index and item id: a list that changed in between is refused,
  never the wrong item taken.
- A double chest is read once (its left half), the walk still passing through both halves.
- Only blocks in `storage:linked` join: furnaces, hoppers and machines are never read or emptied.

## 7. Crafting from the chests (0.2)

Open the vanilla crafting table (or a furnace, any station) touching your chests: the same panel
shows beside it, to the right, because the game's recipe book opens on the left. With the
comfort **Recipes From Chests**, one click on a recipe in the recipe book takes the missing
ingredients from those chests and the game's own placement fills the grid.

**Measured (2026-11-25, Horizon f037744):**

| What | Result |
|---|---|
| Horizon's `recipe_sources` (lend chests to a recipe book) at the vanilla table | not asked: 0 calls (mod crafting menus only). Storage still answers it, so a mod's own station beside chests gets them for free |
| `player_recipe_book_click` | asked on every click, **before** the game places the recipe: Storage moves the missing ingredients into the inventory and the same click fills the grid, no second click |
| One click on a stone pickaxe, 64 cobblestone and 32 sticks in two chests | grid holds 3 cobblestone and 2 sticks, result ready; chests 61 and 30: exactly one craft taken |
| One click on a bow with no string anywhere | nothing taken (an ingredient nothing fits makes the recipe impossible; only an empty pattern cell is skipped) |

- Crafting recipes are type `minecraft:crafting` (not the serializer names).
- Kinds the player already holds are used first, then the most plentiful in the network.
- Shift-click asks for as many crafts as there are ingredients for, up to a stack of the result
  (tested in the planner, not yet in the game).
- The recipe book keeps its red "missing" look for recipes whose ingredients are only in the
  chests (the client counts the inventory alone); the click works.

## 8. Not in this version

- Grouping damaged or enchanted copies of one tool under one entry.
- Opening storage from afar.
