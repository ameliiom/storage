# Storage

For [Horizon](https://playhorizon.dev) (Minecraft: Java Edition 26.3 in Rust). Open any chest that touches other chests, and beside the chest's own screen a panel shows everything in all of them as one list: same items merged with their total, search, sort. No new blocks, no cables: your chests, your inventory and the cursor stay the game's own. English and Arabic.

## How it plays
- **Open any chest, barrel or shulker box** joined to others (touching, or touching one that does). The panel sits beside the chest screen.
- **Click** an entry: a stack to the cursor (right-click: half). **Shift-click**: straight into your inventory.
- **Put in**: click the list with something on the cursor (right-click: one).
- **Search** in any language, `@mod` for a mod's items, `#tag` for an item tag. **Sort** by count, name or mod.
- Every move updates every open panel at once, so friends see the same storage.

## Comforts from Skills
With [Skills](https://github.com/ameliiom/skills) 0.2, a sixth tree, **Organization**, grows as you move items and open chests:
- **Where Is It?**: F over an entry makes its chests glow for everyone.
- **Quick Deposit**: a button puts your whole inventory (not the hotbar) in.
- **Bigger Network** I/II/III: 64, 256 and 1024 chests together (16 to start).

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
