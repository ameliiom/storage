# Storage

For [Horizon](https://playhorizon.dev) (Minecraft: Java Edition 26.3 in Rust). Open any chest that touches other chests, and beside the chest's own screen a panel shows everything in all of them as one list: same items merged with their total, search, sort. No new blocks, no cables: your chests, your inventory and the cursor stay the game's own. English and Arabic.

## How it plays
- **Open any chest, barrel or shulker box** joined to others (touching, or touching one that does). The panel sits beside the chest screen.
- **Click** an entry: a stack to the cursor (right-click: half). **Shift-click**: straight into your inventory. **Double-click**: as much as your inventory holds.
- **Shift + wheel** over an entry: one at a time (down takes one, up puts one back). **Q** drops one in front of you, **Ctrl+Q** a stack.
- **Put in**: click the list with something on the cursor (right-click: one).
- **Search** in any language, `@mod` for a mod's items, `#tag` for an item tag, `$text` inside tooltips (`$sharp` finds Sharpness swords, in any language). **Ctrl+F** jumps to the search box; the **x** clears it.
- Items show as they are: enchantment glint, durability bar, the game's own tooltip.
- Worn and enchanted copies of one item are **grouped** under one entry (marked +); click it to see each.
- **Pin** an entry with a middle-click: pinned items stay first, remembered between games.
- The panel uses the **whole screen height** beside the chest.
- **Tabs** on the panel's side: everything, tools and armour, food, redstone, materials, blocks, other mods.
- **Sort** by count, name, mod, id or what changed last (right-click the sort button to reverse). The list holds still while you shift-click.
- **Sort chests** (brush): every joined chest is compacted and like goes with like. **Label** a chest by hanging an item frame on it: that item (and things like it) goes there first.
- **Deposit matching** (hopper) puts in what storage already holds; **Restock** (dropper) fills your stacks from storage.
- A **capacity bar** shows how full the chests are.
- **Sort your bag**: a brush button at the top right of your inventory (and on a chest's inventory row) merges and orders your main inventory the same way; the hotbar stays.
- Every move updates every open panel at once, so friends see the same storage.
- **Crafting table, furnace…** touching your chests: the same panel shows beside it, to grab ingredients or fuel.

## Comforts from Skills
With [Skills](https://github.com/ameliiom/skills) 0.2 or later, a sixth tree, **Organization**, grows as you move items and open chests:
- **Where Is It?**: F over an entry makes its chests glow for everyone.
- **Quick Deposit**: a button puts your whole inventory (not the hotbar) in.
- **Recipes From Chests**: at a crafting table beside your chests, one click on a recipe in the recipe book takes the ingredients from the chests.
- **Bigger Network** I/II/III: 64, 256 and 1024 chests together (16 to start).
- **Running Low**: a pinned item keeps a minimum (Ctrl + wheel over it, a stack at a time); below it the entry turns red, and an item you ran out of still shows, empty and red.
- **Tidy On Close**: when you close a chest after something moved, the chests sort themselves (not while a friend has them open).
- **Storage In Your Bag**: open your own inventory within reach of your chests (5 blocks) and the same panel shows beside it; walk away and it goes.

Without Skills, the server setting **All comforts without Skills** opens them all.

## Install
1. Copy `storage.hmod` into `%APPDATA%\horizon\horizon-native-mods\`.
2. Add the line `mod.storage:true` to `%APPDATA%\horizon\horizon-mods.txt`.
3. Build: the launcher's Mods page **Build**, or `horizon-launcher native-build --instance horizon --trust-native-code`. A native mod runs with your rights: install it only from a source you trust.

The server and every player need the mod. Built for Horizon Nightly `f037744` (2026-11-21).

## Add your containers
Any block with slots can join: add it to the block tag `storage:linked` from a data pack.

## Licence
MIT, see [LICENSE](LICENSE). Not affiliated with the Horizon project.
