# Storage: requirements

One block, the **Storage Terminal**, placed against your wall of chests. Open it and every chest,
barrel and shulker box joined to it shows as one inventory: the same items merged into one entry
with their total, a search box, one click to take, shift-click to put back. A button lights up
the chest that holds an item. Later, a crafting grid that pulls ingredients from storage.

## 1. Outcome and riskiest assumption

- **Outcome:** finding or storing any item takes one screen and a few seconds instead of opening
  chest after chest. Measured in the build stage: clicks and seconds to fetch 3 different items
  from a 30-chest room, before (vanilla) and after.
- **Riskiest assumption:** a mod screen can show a merged list that isn't made of real slots
  (hundreds of kinds) and move stacks between storage and the player's cursor and inventory
  correctly with two players at once. **Validated first**, in slice 1, before anything else is
  built: 50 chests, 1000 kinds, two players taking the same item at once; measure frame time, sync
  bytes and that no item is ever duplicated or lost.

### Measured in the skeleton (owner's PC, Horizon f037744, 2026-11-24)

A terminal against 50 single chests (10 × 5), all 1350 slots filled, 170 kinds:

| What | Measured |
|---|---|
| Reading the whole network (walk + every chest by kind + merge) | **1.18 ms** on open, 0.92 ms on a take |
| The view sent to the client | about 4.9 KB (≈ 29 bytes a kind; 1000 kinds ≈ 29 KB, under the 4000-entry list cap) |
| A click on stone (287 in storage) | 64 stone in the player's hand, the chest slots emptied nearest first, one transaction |

Seen and fixed in the build stage: three- and four-digit counts are wider than a slot (draw them
at half size, as storage mods do); the list keeps first-seen order, so an entry moves after a take
(sort on the client, by count by default). Not yet measured: two real players clicking at once
(the planner's property test covers the arithmetic; the server handles takes one at a time).

## 2. Prior art

| Mod | What it does | Used how |
|---|---|---|
| [Tom's Simple Storage](https://modrinth.com/mod/toms-storage) (Java, MIT) | Storage terminal + inventory connector + cables, crafting terminal, wireless terminal, filters | The feature set and behaviour (inspiration, not code); its art is not used |
| [Obscura's Simple Storage](https://www.modpackindex.com/mod/99727/obscuras-simple-storage-toms-simple-storage-fork) (MIT fork) | Same, optimised | Confirms the idea scales |
| Refined Storage, Applied Energistics 2 (Java) | Full digital storage with disks, power, channels | Too heavy: power and disks are chores |
| Mod browser (`mods search storage`) | Only resource packs for Tom's | Nothing runs as is in Horizon |

Java jars don't run in Horizon, so Storage is built natively on Horizon's container API
(`container_listing`, `container_insert_kind`, `container_extract_kind`, all in one transaction),
its block, block-entity and menu registration, and its screen widgets (text field, key and char
input).

## 3. Domain map

| Concept | Owner | Stored? |
|---|---|---|
| **Terminal**: a block that opens the storage screen | the world (a block) | yes, as a block |
| **Network**: the containers a terminal reaches | derived each time from the blocks touching it | never stored |
| **Container**: a chest, barrel, shulker box, or any block in the tag `storage:linked` | the world | vanilla's own |
| **Entry**: one kind of item (same item and components) and its total across the network | derived from the containers | never stored |
| **View**: search text, sort, scroll | each player's client | client only |

The containers stay the single source of truth: Storage keeps no copy of any item, so nothing can
drift, and removing the mod leaves every item where it was.

## 4. Choices (each compared, one picked)

**Linking chests to the terminal**

| | A. Tom's way: connector + cables | **B. Touching chain** |
|---|---|---|
| Setup | craft and place a connector and cables | none: chests touching each other or the terminal join |
| Fits | storage anywhere | a storage wall or room (how most bases store) |
| Chores | yes | none |

**B**, because the owner wants no chores. Linked blocks are a data tag (`storage:linked`:
chests, trapped chests, barrels, shulker boxes, and any mod's storage block added to the tag), so
furnaces and hoppers are never pulled from. Capped at 256 containers per terminal.

**The screen**

| | A. Real slots (a giant chest menu) | **B. A merged list drawn by the mod** |
|---|---|---|
| Same items | stay split across slots | one entry with its total |
| Search / sort | no | yes |

**B**, drawn in the game's own look: the creative inventory's panel, scrollbar and search field,
item icons with counts (`1.2k` above 999), the player's inventory under it.

## 5. How it plays

- **Open:** right-click the terminal. Every player can use it at once; each sees changes live.
- **Take:** click an entry: a stack to the cursor (right-click: half, shift-click: straight to
  the inventory).
- **Put:** shift-click a stack in your inventory, or drop the cursor's stack on the list. It fills
  stacks already in storage first, then empty slots, as hoppers do.
- **Search:** type a name in your language; `@mod` for a mod's items, `#tag` for a tag.
- **Sort:** by count, by name, by mod (one button cycling, its icon showing which).
- **Find:** hover an entry and press the find key: the chests holding it give off glowing
  particles for 10 seconds, seen by everyone, so a friend can be told "it's in that one".
- **Crafting terminal** (later slice): a 3×3 grid in the same screen; the recipe book fills it
  from storage; shift-click crafts as many as storage allows.

## 6. Comfort earned through Skills (the Organization tree)

Skills' idea is that what is tiring at first becomes comfortable as you play. Storage follows it:
the basics work from the first terminal, and the comforts are perks of a sixth skill tree,
**Organization**.

| Always | Earned in Organization |
|---|---|
| the terminal, chain linking, the merged list, take, put, search, sort | **Find** (chests glow) → **Quick deposit** (one button empties your inventory but the hotbar into storage) → **Crafting grid** in the terminal → **Remote reach**: open the network you last used from 16, then 48 blocks, then anywhere in the same dimension, with a key |

- **Earned by:** moving items through a terminal (Storage's own statistic `storage:items_moved`),
  opening chests and barrels, crafting.
- **No code tie between the two mods.** Storage declares its perks as its own attributes
  (`storage:find`, `storage:quick_deposit`, `storage:crafting`, `storage:remote_range`) and its
  statistic; it ships the Organization tree as a file in its data pack
  (`data/storage/skills/organization.json`), which only Skills reads. Skills already accepts any
  attribute and any statistic, so it needs no change, except one: an on/off perk shows as its name
  alone in the tooltip ("Find items") instead of "+1 Find items".
- **Without Skills** the comforts stay locked; a server setting `all_comforts` (off) opens them for
  a world that doesn't use Skills.

## 7. Slices, in order (each one works end to end in the game)

1. **See and take** (and validate the riskiest assumption, §1): terminal block and recipe, chain
   linking, the merged list, take to cursor and inventory.
2. **Put back, search, sort**, and the `storage:items_moved` statistic.
3. **The Organization tree and Find** (particles on the chests).
4. **Quick deposit, crafting grid, remote reach.**
5. Arabic and English throughout, settings.

## 8. Pre-mortem: "it failed six months later" — why, and the guard

| Cause | Guard |
|---|---|
| Items duplicated or lost when two players click at once | The server does every move in one container transaction and answers with the new totals; the client never assumes. A property test moves random stacks with two players and checks the grand total never changes |
| Lag with a big storage room | Containers are read only while a screen is open, at most once a second and on every move; the cap of 256; frame time and sync bytes measured in slice 1 with 50 chests |
| A mod's container behaves oddly (a machine, a locked slot) | Only blocks in `storage:linked` join; vanilla's own insert and extract rules (`canPlaceItem`, `canTakeItemThroughFace`) are used, never bypassed |

## 9. Settings

- Server: `max_containers` (16–1024, 256), `find_particles` (on), `all_comforts` (off).

## 10. Not in this version

- Access from another dimension.
- Auto-sorting items into the right chests.
- Filters and import/export hoppers.
