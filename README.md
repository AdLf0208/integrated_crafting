# Integrated Crafting → Horizon (slices 1–2)

A Horizon port of **Integrated Crafting 1.8.1** (`CyclopsMC/IntegratedCrafting`,
**MIT**), Minecraft 1.21.1 NeoForge, from the exact ATM10 pack jar.

**The port is the crafting-job engine and its interface block.** A **Crafting
Interface** accepts a request for *N* of an item, resolves it against the game's
own crafting recipes, keeps that recipe for the job's life, holds the ingredients
in its own slots, crafts one recipe at a time, blocks with the reason when it
cannot continue, and reports its queue through `/integratedcrafting status`.

**Slice 2 adds the job dependency graph**: a request whose ingredient is itself
craftable plans that dependency first — one log becomes planks, the planks become
four sticks — and the acceptance reply names the whole chain. A chain deeper than
eight steps, or one that needs itself, is refused with its deficit rather than
followed.

The network half is the recorded gap: upstream the interface is a part on an
Integrated Dynamics cable that pulls its ingredients from the network's storage
and inserts them into a bound machine. This pin has **no cross-mod API**, so a
part cannot join the parent's network — see `notes.md`,
`blocked(no-parent-network-attachment-api)`. What remains is the mod's job
engine, run standalone over the interface's own container.

What is not here: the variable-card layer, the part screens, the attuned and
enumerated variants, the crafting writer's aspects, priorities, distributed
jobs, reusable ingredients, fluids and energy, and the terminals integration.

## The package

One flat `.hmod` (Horizon mod API 0.2), content and code together:

```text
integratedcrafting.hmod
  horizon-mod.toml   the manifest and the only identity (0.2.2)
  Cargo.toml         horizon-mod-api from .horizon/sdk/ (fetched, never shipped)
  src/lib.rs         the one #[mod_api::horizon_mod] impl Hooks + the interface's Part
  src/runtime.rs     the pure job engine, its chain planner and their 28 tests
  src/support.rs     the glue: the saved queue, the slots, the recipe table, the commands
  assets/integratedcrafting/   the interface's blockstate, models, texture, lang
  data/integratedcrafting/     the block, creative tab, loot table and recipe
  LICENSE, LICENSE-NOTICE.md
```

## Commands

| Command | What it does |
|---|---|
| `/integratedcrafting status <x> <y> <z>` | the readout: queue, the running job's step, craft and progress, the blocked reason |
| `/integratedcrafting request <x> <y> <z> <item> <count>` | resolve the item's recipe (and its dependencies) and queue a job (refused, with the deficit, when the slots cannot pay) |
| `/integratedcrafting cancel <x> <y> <z>` | drop every unfinished job; the ingredients stay in the slots |
| `/integratedcrafting feed <x> <y> <z> <item> <count>` | move items from your own inventory into the ingredient slots, as a hopper above the interface does (you must be within 6 blocks; nothing is created) |

Hoppers are the ordinary route: a hopper above fills the nine ingredient slots,
hoppers below and beside take the four result slots.

## Building, checking and running

```sh
BUILD_LOCAL=1 ports/tools/build-slot.sh run --job sdk-ic --port ports/integratedcrafting \
    --step test -- bash -lc 'cd ports/integratedcrafting && cargo test --lib --offline'
BUILD_LOCAL=1 ports/tools/build-slot.sh run --job sdk-ic --port ports/integratedcrafting \
    --step check -- bash -lc 'cd ports/integratedcrafting && cargo clippy --all-targets -- -D warnings'
rm -f ports/integratedcrafting/Cargo.lock
BUILD_LOCAL=1 ports/tools/build-slot.sh run --job sdk-ic --port ports/integratedcrafting \
    --step build -- bash -lc 'cd ports/integratedcrafting && cargo horizon-mod build'
cp ports/integratedcrafting/target/horizon-mod/integratedcrafting.hmod ports/integratedcrafting/
cargo horizon-mod check ports/integratedcrafting/integratedcrafting.hmod
cargo horizon-mod fits ports/integratedcrafting/integratedcrafting.hmod --build 0a875da
```

The in-game scenario (`tools/e2e.sh`) runs on a **launcher-built** game:

```sh
ports/tools/launcher-game.sh run integratedcrafting
```

It proves, in a real world, the interface's placement and state, jobs A–F, the
dependency chain, the loot table (with an unknown-table differential), and — in a
game with the parent port installed (`PARENT_ITEMS=1`) — the package's own
carried recipe, crafted through the interface. The evidence and the ladder are in
`notes.md` and `VALIDATION.md`.

The jar is evidence at
`ports/_shared/licence-cache/integratedcrafting-1.21.1-neoforge-1.8.1.jar`
(SHA-256 in `LICENSE-NOTICE.md`).

**Local-only** until the maintainer approves publishing (ARCHITECTURE.md §8).
