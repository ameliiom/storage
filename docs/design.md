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
- Only blocks in `storage:linked` join: furnaces, hoppers and machines are never read or emptied
  as storage. With Furnaces From Chests, a furnace touching the network takes only tagged ore,
  food and fuel into its own empty slots and gives back only its result slot.

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

## 8. Reassessment (2026-11-25): what is missing, ranked

The owner's verdict: filtering, sorting and much more are missing. Measured against the mods
players use for this (Refined Storage's grid: sort by name/count/id/last changed with direction,
sorting frozen while shift-clicking, tag/tooltip search, filter tabs; Tom's Simple Storage with
its category-filter addon; AE2 Favorites' pinned items; Quark and Inventory Profiles' sort and
"deposit matching / restock" buttons) and against the owner's own rule that every list ships its
full operator toolkit, 0.2 is a bare list. What is missing, by how much it changes daily play:

### Phase 1: organize (the core of a storage mod) — built in 0.3.0

| # | Gap | What it becomes | Built on |
|---|---|---|---|
| 1 | No physical sorting | **Sort chests** button: every joined chest compacted (partial stacks merged) and ordered by category, so the chests themselves are tidy | slot reads and writes through `ITEMS`, one transaction |
| 2 | No routing | **Labels with item frames**: an item in a frame on a chest makes that chest the home of that item (or its tag/category); deposits and Sort chests go there first | vanilla item frames: found with `level.entities_in`, their item read from the entity's data (`entity_nbt`, `Item`); both calls exist in the API |
| 3 | Only "put the cursor in" | **Deposit matching** (your inventory's items that storage already holds go in) and **Restock** (fill your partial stacks from storage) buttons | the existing transaction moves |
| 4 | No filter | **Category tabs** under the panel: all, blocks, tools and weapons, food, redstone, materials, mods | the API has no creative-tab membership, so the categories are a data file of item tags (`#minecraft:pickaxes`, `#c:foods`…) that packs can extend, each tab's icon a vanilla item; a client-side filter of the view |
| 5 | Sort is thin | sort by count, name, mod, **id, recently changed**, with a **direction** toggle; the order freezes while you shift-click so entries don't jump | client sort |
| 6 | No sense of space | **Capacity bar**: slots used / total and the number of chests, coloured as it fills | the server already reads every slot |

### Phase 2: speed — built in 0.4.0

| # | Gap | What it becomes |
|---|---|---|
| 7 | Few gestures | wheel over an entry takes/puts one, Q drops one, double-click takes all of a kind, Ctrl+F focuses search |
| 8 | Noisy tools list | damaged and enchanted copies of one item grouped under one entry, opened with a click |
| 9 | No favourites | **pin** an entry (stays first); pins remembered per player |
| 10 | Search is basic | `$` searches tooltips and enchantments (the API's `item_resource_tooltip`), a clear button |
| 11 | Panel height fixed | the panel uses the whole screen height beside the chest |

### Phase 3: comforts for the Organization tree — built in 0.5.0

- **Auto-sort**: chests sort themselves when you close them.
- **Storage from the inventory**: the panel beside your own inventory screen when you stand in
  your storage room (no new block, no remote item).
- **Low-stock marks**: an entry you pin can carry a minimum; below it the entry glows red.

Phase 1 as built and measured (2026-11-25, 50 chests, Horizon f037744):

- **Sort chests** (brush button): every kind taken out and put back in one transaction; the
  chest with a diamond in an item frame received all the diamonds first, its free room then took
  other materials (its category).
- **Labels**: `entities_in` leaves item frames out (its doc says so), so frames are tracked as
  they join and leave levels (`entity_join_level`), and a frame belongs to the chest behind it
  (`Facing`). No scan, no polling.
- **Deposit matching** (hopper): 20 dirt in the inventory went in, sticks storage does not hold
  stayed. **Restock** (dropper): cobblestone 10 → 64, oak logs 3 → 64.
- **Category tabs** on the panel's side (everything, gear, food, redstone, materials, blocks,
  mods), from item components (durability, tool, weapon, worn, food), the item tag
  `storage:category/redstone` (packs extend it) and whether a block shares the id.
- **Sort orders**: count, name, mod, id, recently changed; right-click reverses; the order holds
  still while you shift-click and settles when the mouse leaves the panel.
- **Capacity bar**: green, amber past 75%, red past 90%; its tooltip gives slots and chests.
- Buttons wrap to two rows on a narrow panel; a test checks every control stays inside the panel
  at every width.

Phase 2 as built and measured (2026-11-25, 50 chests, Horizon f037744):

- **Real stacks**: the view carries each kind with components (enchanted, worn, potions) as typed
  NBT in JSON (`nbtjson.rs`, a string per entry, plain items send nothing); the client rebuilds the
  stack, draws it with glint and durability bar and shows the game's own tooltip.
- **Groups**: 4 kinds of diamond sword (plain, worn, Sharpness III, Unbreaking) showed as one
  entry with a + mark; a click opened them. A search that leaves one variant shows it alone.
- **Tooltip search**: `$حدة` (the Arabic tooltip) and `$unbreak` (the enchantment id in the stack's
  data) both found the enchanted swords with the game in Arabic.
- **Gestures**: shift + wheel down took one mossy cobblestone, up put one back; Q dropped the
  Sharpness sword in front of the player, Ctrl+Q a stack of 64 apples; double-click took all 295
  bones into the inventory (4 stacks and 39); Ctrl+F focused the search box; the x cleared it.
- **Into the inventory** goes slot by slot (stacks of that kind first, then empty slots, hotbar
  first), each slot asked for no more than its room: `PlayerHand` holds only the hand's slot, and a
  slot asked for more than a stack takes nothing (168 at once rolled back).
- **Pins** live in `config/storage-pins.txt` (item ids, one a line); a pinned apple came first with
  a gold corner after a restart. The middle-click itself was not driven by the test tool (it has
  no middle-click in screens).
- **Panel height**: the whole screen height minus 4 pixels, beside the chest; a test checks every
  control stays inside at 6 widths.
- A double-click needs both presses within 300 ms; two scripted clicks 270 ms apart were caught.

Phase 3 as built and measured (2026-11-25, 50 chests, Horizon f037744):

- Three nodes in a third row of Organization, each needing its neighbour: **Running Low**
  (`storage.low_stock`), **Tidy On Close** (`storage.auto_sort`), **Storage In Your Bag**
  (`storage.inventory`).
- **Storage in your bag**: the own inventory is `ScreenView.id == "inventory"` with no menu; the
  client asks once per opening (`storage:near`), the server finds the nearest linked block within
  5 blocks of the player and opens a session on it. Its cursor is the inventory menu's
  (`container_menu` answers for it): a take to the cursor, a place into a slot and shift-clicks
  all worked. 14 blocks away the open panel went within a second (a view of no containers), and
  reopening there showed none.
- **Tidy on close**: a session remembers what the network held when it opened (ids and totals);
  on close, if that changed and no one else has the network open, it sorts. Taking 64 apples and
  closing the inventory sorted the room (the first chest went from a mixed stone stack to echo
  shards in category order; used slots fell from almost all to about a third). Closing the
  inventory also fires the server's container close, so either path ends the session once.
- The session leaves before the sort, so the sort's refresh never reaches the panel being closed;
  and the own inventory drops any view it held when it asks, so a view in flight from a chest just
  closed can't show (found when a post-sort view arrived after the close).
- **Running low**: pins are `id minimum` lines in `config/storage-pins.txt`; Ctrl + wheel over a
  pinned entry changed 512 to 576 apples; 223 apples showed red; with every torch removed, the
  pinned torch (minimum 64) still showed, empty and red.
- A frozen list (while shift-clicking) now keeps entries by id, not by index: the server's entry
  order can change between views, and the apples had jumped from second place to the eighth row.

### Phase 4: sort your bag — built in 0.6.0

- A slot-look button with the brush (as the panel's Sort chests) at the top right of your own
  inventory, and of the inventory rows under any chest: one click sorts your main inventory
  (not the hotbar, not armour) as Sort chests orders chests: partial stacks merged, then by
  category (gear, food, redstone, materials, blocks) and name. Free for everyone.
- On the server, in one pass: read slots 9 to 35, merge, order with `organize::order` (the same
  rules as the chests), write them back with `set_inventory_slot`.
- Measured: `set_inventory_slot` moves stacks safely and the client sees them the next tick (Hands,
  2026-11-25); the order is the chests' (tested in `organize.rs`).
- Measured in the game: own inventory, 10 + 20 dirt, a sword, bread, redstone and iron spread
  over the bag → sword, bread, redstone, iron, 30 dirt from the first slot, the hotbar untouched;
  from a chest's screen, 7 + 9 cobblestone and a golden apple → apple, 16 cobblestone. Done in one
  transaction (taken out, put back in order; a stack that would not go back rolls it all back).
  Not measured: the button's place with the recipe book open on a narrow screen.

### Phase 5: shulker peek and furnaces from chests — built in 0.7.0

Two nodes in a fourth row of Organization: **Shulker Peek** (`storage.peek`, needs Running Low)
and **Furnaces From Chests** (`storage.furnace`, needs Shulker Peek and Tidy On Close). Each can be
turned off in the mod's settings (`shulker_peek` for the player, `furnace_feed` for the server).

- **Shulker peek**: an item part on the item tag `minecraft:shulker_boxes` answers
  `tooltip_component` (162 × 56 GUI pixels) and draws in `tooltip_component_draw`: 27 of the
  game's `container/slot` sprites in 9 × 3, each item with its count, read from the box's
  `minecraft:container` component. An empty box answers nothing, so its tooltip stays vanilla.
  Tags live on the server only, so the server tells the client the comfort (`storage:perks`, a
  bit set, sent when it changes and again after a failed send).
- **Furnaces from chests**: a furnace, smoker or blast furnace touching a chest of a network
  takes from the network when a slot is empty and puts results back, all through the game's
  `ITEMS` slots (0 input, 1 fuel, 2 result), each move one transaction.
  - What it cooks: only kinds in the item tag `storage:furnace_input` (raw metals, ores, raw
    food, kelp) that this furnace's recipe type cooks, the most plentiful first, a stack at most.
    A player's cobblestone, sand or logs are never cooked; packs extend the tag.
  - What it burns: only kinds in `storage:furnace_fuel` (coal, charcoal, coal blocks, blaze rods,
    dried kelp blocks), never one this furnace could cook (a pack adding logs to the fuel tag
    still keeps them for charcoal), only when there is something to cook, as many as cook what
    is in the input (64 raw iron: 8 coal), a stack at most.
  - Results go back whole or stay (a full network leaves them in the furnace). Furnace experience
    stays in the furnace until a player takes a result from it or breaks it, as vanilla keeps it.
  - When: the tick after a furnace burns fuel, finishes an item or is emptied (`furnace_burn`,
    `furnace_smelt`, `furnace_extract`), and once a second for new work. Furnaces are learned when
    a player with the comfort opens them or a chest of their network, and when they burn or
    finish; the list lives in memory (after a restart, opening the chest or the furnace, or the
    furnace finishing an item, brings it back). It works while a player with the comfort is in
    that dimension, reaching as far as the biggest network among them.
- Measured in the game (2026-11-26): 3 raw iron and 16 coal in the far chest of two, logs and cobblestone in the near one: the furnace took 3 raw iron and 1 coal (one spare coal waits in its fuel slot), 3 iron ingots came back, logs and cobblestone untouched; without the tag nothing moved in 8 s, with it new raw gold went in within 2 s. A box with 4 kinds showed the 9 × 3 grid (`docs/shulker-peek.png`); an empty one kept the plain tooltip.
- Reading players or blocks inside `furnace_smelt` hung the server (the world dropped about 75 s after joining): the hooks only note the position, the work runs on the next server tick.

Not planned: grouping by Horizon-level "storage disks" or power (chores, against the owner's
rules).

### 0.7.1: Where Is It? through Team

Find's glow repeated Team's ping ("look here, everyone"), so with Team installed Find sends Team's
own marker instead: its loot ping, the chest's icon, in the player's colour, over the chest holding
the most of the item (`skills::most_held`), sent through Team's marker sender
(`team::payloads::SHOW`, an optional dependency, as Map reads Team). Without Team the chests still
glow. Measured in the game (2026-11-26): F over coal (only in the far chest of two) put a marker
with the chest icon and "4" over that chest and on the minimap (`docs/find-team.png`).
