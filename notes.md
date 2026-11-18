# Integrated Crafting → Horizon (slice 1)

Port of **Integrated Crafting 1.8.1** (`CyclopsMC/IntegratedCrafting`, **MIT** per the jar's
`META-INF/neoforge.mods.toml`), Minecraft 1.21.1 NeoForge, from the exact ATM10 pack jar:
`integratedcrafting-1.21.1-neoforge-1.8.1.jar`, **521,588 B**, SHA-256
`44b2ac7b36260933f1e131da098cb2a0e472a99d1d04185b7f3aa94ba555e3ff`, kept at
`ports/_shared/licence-cache/integratedcrafting-1.21.1-neoforge-1.8.1.jar` (evidence — never
deleted). The jar inventory and the behaviour read out of it are in [`FINDINGS.md`](FINDINGS.md).

**Package version 0.2.0 — first publication** (Horizon mod API 0.2, the flat SDK layout of
2026-11-18).

**Class + tier:** `content` / **`deep`** (ledger row; provenance
`ports/_shared/source-map/atm10-sources.csv`). `precheck-mod.py integratedcrafting` prints
`class=content tier=deep kind=mech role=addon prio=P0`, licence `MIT` rank 0 (read from the jar).
Approach per Horizon's `docs/modpacks.md`: a `deep` mod is entities/renderers/dimensions/new hooks
*unless the behaviour is the weight* — this mod has **four part models and no blocks**; what it is,
is a **crafting-job engine**, so the slice takes the engine plus the one interface that drives it.
A `deep` attempt at a renderer would have been wasted here; leaving it at registrations would have
been the wrong shape too.

**Licence:** MIT → **rank 0** (permissive). Local-only until the maintainer approves publishing
(ARCHITECTURE.md §8); `LICENSE` (Cyclops, 2015, MIT) and `LICENSE-NOTICE.md` travel inside the
package.

**Why now:** licence-first queue (rank 0), mechanical add-on of the already-ported parent
(`integrateddynamics` is `ported-partial`, so the add-on is workable), and the family's shape is
the parent's own SDK pattern — the fourth child built from it.

## Slice 1: the crafting-job engine and the Crafting Interface

A **Crafting Interface** block that accepts a request for *N* of an item, resolves it against the
game's own `minecraft:crafting` recipes, keeps that recipe for the job's life, holds the
ingredients in its own slots, crafts one recipe at a time, blocks with the reason when it cannot
continue, and reports its queue.

### What the slice does

- **`integratedcrafting:part_interface_crafting`** — a part-shaped block (the carried part model:
  a 10 px plate with a 3×3 grid), a block entity holding the job queue in its saved compound
  (`jobs`, `next_job`), and a **declared item container of 13 slots**: nine ingredient slots (a
  crafting table's grid) and four result slots.
- **The engine** (`src/runtime.rs`, 14 tests, no game types): a request resolves to a recipe and
  `ceil(requested / output_count)` crafts; the whole job's ingredients are checked at submission
  (upstream reserves them when the job starts) and refused with the deficit if the slots cannot
  pay; one craft takes `TICKS_PER_CRAFT` = 20 ticks; ingredients are consumed and the result
  inserted only on the tick a craft completes; a craft that cannot finish consumes nothing; a job
  blocks on `missing` (with the exact deficit) or `output full` and retries every tick; a finished
  job leaves the queue.
- **The tick** (`src/support.rs`): rebuilds the engine from the compound, advances it once, writes
  the queue and the slots back, and answers **`BlockEntityTickReply::Unchanged`** when it changed
  nothing — so an idle interface does not dirty its chunk or notify comparators.
- **`/integratedcrafting status <pos>`** — the readout: the queue, the running job's craft and
  progress, the blocked reason. The text is produced by the engine (`Engine::readout`) and pinned
  byte for byte by two tests.
- **`/integratedcrafting request|cancel|feed`** — submit a job, drop every unfinished job, and put
  items into the ingredient slots (the path the part's out-of-slice screen and the missing network
  would otherwise provide).
- **Content**: the interface's texture (byte-identical), model geometry (the jar's ten cubes), a
  single blockstate, an item model, the block declaration, a self-drop loot table, a creative tab,
  the recipe and the `c:ingots/iron` tag it names. `carry-diff.txt` in the evidence folder prints
  every carried file's size and SHA-256 against the jar's.

### What the parent provides, and what it does not

This is the whole reason the network half is a gap, so it is recorded with symbols. Read from
`ports/integrateddynamics/src/lib.rs` and `src/support.rs` (read-only):

**Provides (its own slice, not an API):**

- `struct Cable` with `#[mod_api::horizon_part] impl BlockPart` (`block_signal`, `is_signal_source`,
  `use_item_on`) and `impl BlockEntityPart` (`block_entity`, `block_entity_ticks`,
  `block_entity_tick`), attached in `Hooks::register` to `integrateddynamics:cable`
  (`block_key` → `attach_block_part`; `declare_block_entity_type` → `attach_block_entity_part`).
- A redstone **channel** in the cable's saved compound under the key `value` (0..15) and a `status`
  state property, recomputed at `level_tick_post` over the connected cable set
  (`support::on_level_tick_post`, `runtime::collect_network`).
- `/integrateddynamics info|part|probe`, and the `integrateddynamics:part_redstone_reader|writer`
  items, placed on a cable face by the cable's `use_item_on`.

**Does not provide:**

- **Any exported API.** `lib.rs` exposes `pub mod runtime` and nothing else; `support` is `mod
  support` (private). Horizon gives each mod its own injected crate with **no shared symbol table**
  (ARCHITECTURE.md §3, "No cross-mod API between ports": *"a port can only call the pin's
  hooks/helpers, never another port's Rust"*), so `integratedcrafting` cannot call
  `integrateddynamics::support::…` even though both are ours.
- **A part container that admits a new part.** The cable's faces are a closed enum of six
  `part_<face>` state properties whose values are `none|reader|writer`
  (`data/integrateddynamics/horizon/block/cable.json`, `state_properties`); a state's values are
  fixed when the block is declared, so `set_block_property(pos, "part_north", "crafting_interface")`
  cannot take, and no Part hook answers for "a part I do not know".
- **Anything the crafting half needs from a network**: no item storage, no recipe list, no job
  queue, no crafting CPU (`integrateddynamics`' own notes list the storage reader/writer parts as
  unported). The only interop that exists in the world is the cable's redstone `value` compound key
  — a redstone level, which carries no ingredients, no recipes and no jobs.

Therefore: **`blocked(no-parent-network-attachment-api)`** — with the symbols, not a shrug. The
interface is not a decorative block: it is a working machine (jobs, ingredients, crafts, results,
refusals, readout), and the *network* is what it cannot reach. When a cross-mod API or the parent's
part container lands, the change is local: `support::recipes_for` reads the parent's exposed recipe
list instead of `minecraft:crafting`, and the interface's inputs come from the network's storage
instead of its own slots.

### The crafting writer: recorded, not faked

`brief` item 3 allows the recipe-recording path **if expressible**. It is not:
upstream `part_crafting_writer` is a part on a cable holding a **variable card** whose aspect
starts jobs (its own info book, `crafting_writer.text2`; the jar's `part/aspect/CraftingAspects` and
`CraftingAspectWriteBuilders` implement the aspects). This pin has no variable-card layer, no
aspect API, and no way to place a part on the parent's cable (above). A writer that "records a
held recipe" would be a new item with an invented semantic.

**`blocked(no-variable-card-layer)`** — the writer's *role* (starting jobs) is covered by
`/integratedcrafting request`, which is the same "start a job for this recipe/output" decision
without the card.

### Deliberate deviations

| Upstream | Here | Why |
|---|---|---|
| A part on an Integrated Dynamics cable, pointing at a machine | a block of its own, holding its own slots | no cross-mod API, and the parent's cable faces are a closed enum (above) |
| Ingredients pulled from the **network's storage** and inserted into the bound machine | ingredients come from the interface's own nine slots; the results stay in its four result slots | the network gap; upstream's documented crafting-table exception is the closest true shape: "only a Crafting Interface is required to craft with this [crafting table]" |
| A job completes when its outputs reach the network's storage | a job completes when its outputs are in the interface's result slots | no network to insert into; a hopper below takes them |
| The recipe comes from a variable card, or (attuned) from the bound machine's recipe list | the recipe comes from the game's `minecraft:crafting` table, resolved once per request | no cards, no bound machine; the recipe the game itself has is the honest table |
| Blocking mode configurable; distribution over several interfaces; priorities | one job at a time, fixed (the upstream default) | settings screen out of slice |
| Recipe duration measured per machine (`RecipeDurationStatistics`) | `TICKS_PER_CRAFT` = 20 ticks | no machine to measure; a crafting-table craft's second |
| A craft whose recipe makes more than asked | makes the recipe's whole output; the job completes when the requested amount is reached | physical crafting produces the whole result; the readout shows both numbers (`x8 … produced 8/5` never happens — the job ends at `produced >= requested`) |
| Attuned / enumerated interface variants, their screens, and re-usable ingredients | not carried | out of slice, recorded below |
| The part is drawn by the cable's multipart renderer, with a `facing` per mount face | one block state, the carried model in its jar-default orientation (south) | a block, not a cable part; `facing` would be a state nothing sets |
| The recipe needs `integrateddynamics:variable_transformer_output`, `…_input` and `crystalized_menril_block` | the same pattern with `integrateddynamics:cable` (V, W) and `integrateddynamics:crystalized_menril_chunk` (M) | those three items are not in this campaign's parent port, so the unedited recipe could **never** match — a dead recipe is a defect, not fidelity |

## Upstream defects fixed

Checked, and **none found** that this slice would reproduce. What was checked, from the artefact:

- **Recipe vs. code ids.** The interface's recipe (`data/integratedcrafting/recipe/crafting/
  part_interface_crafting.json`) names `integrateddynamics:variable_transformer_output`,
  `..._input` and `crystalized_menril_block` — items that exist in **upstream** Integrated Dynamics
  (1.37.1+, its declared dependency) but **not** in this campaign's parent port (which carries only
  `cable` and `crystalized_menril_chunk`). Upstream is consistent; *the port* would ship a recipe
  that can never match, which the campaign's ruling counts as a defect ("a recipe … that can never
  match at the target version"). **Fixed** by substituting the two cable ends and the menril chunk
  for the three unported items, keeping the pattern and the iron/crafting-table identity. A player
  notices the recipe is craftable with what this pack's ports provide. Not an upstream report.
- **The `c:ingots/iron` tag.** 26.3 ships **no** `c:` tags (measured: 0 entries under
  `data/c/tags/item` in `26.3.zip`), so the carried recipe's tag would match nothing. Declared here
  as the parent port declares `c:dusts/redstone` for its own cable recipe — one file,
  `data/c/tags/item/ingots/iron.json`, values `["minecraft:iron_ingot"]`.
- **The 3×3 grid accounting.** Upstream's `CraftingJob` sees the recipe's *cell* list, so two cells
  of one item are two items, not one. The engine groups cells by their candidate set and asks for
  the sum (`missing_ingredients`), which the test
  `two_cells_that_accept_the_same_item_are_accounted_separately` pins: a naive per-cell check
  against the same single plank would accept the job and then starve it mid-craft.
- Nothing was "fixed" that upstream does on purpose: the one-job-at-a-time default, the
  "ingredients are held by the job" rule, and "a running job keeps its recipe" are upstream's own
  documented behaviour and are implemented, not changed.

## Gaps (next slices)

1. **`blocked(no-parent-network-attachment-api)`** — the network half: a part on
   `integrateddynamics:cable` (its `part_<face>` values are a closed enum), the network's recipe
   list and item storage. Symbols: `integrateddynamics/src/lib.rs` (`struct Cable`,
   `Hooks::register`), `integrateddynamics/src/support.rs:on_level_tick_post`,
   `data/integrateddynamics/horizon/block/cable.json`.
2. **`blocked(no-variable-card-layer)`** — the Crafting Writer part and every aspect the mod ships
   (`part/aspect/CraftingAspects`, `CraftingAspectWriteBuilders`), the attuned interface's recipe
   grid and the enumerated variant's list value.
3. **The part screens** — upstream has five containers and their screens
   (`ContainerPartInterfaceCrafting`, `…Settings`, `…AttunedRecipes`, `…AttunedOffsets`; the jar's
   GUI textures). Slice 1 has the container and the commands, no screen.
4. **The job dependency graph** — a job whose ingredient is itself craftable
   (`CraftingJobDependencyGraph`, `PartialCraftingJobCalculation`) is not implemented: a request
   that needs a crafted intermediate is refused as missing ingredients. This is the largest
   remaining piece of the engine.
5. **Priorities, distributed jobs over several interfaces, several crafting CPUs, blocking-mode
   settings, re-usable ingredients, fluids and energy** (`ICraftingNetwork`, `CraftingNetwork`).
6. **The bound machine** — inserting the inputs into the block the interface faces through the item
   capability, and waiting for the machine's output (`level.capability(&ITEMS, pos, side)`); the
   crafting-table case this slice stands in for is upstream's own exception, but a furnace or
   modded machine needs this.
7. **Recipe types other than `minecraft:crafting`** (`smelting`, `stonecutting`, `smithing` are in
   `CraftingProcessOverrides`); `RecipeRule`-typed recipes (stonecutting, smithing, special) expose
   no `ingredients`/`results` lists at this pin, so they are not resolvable by this engine as
   written — a recorded API shape, not a bug.
8. **Advancements, the info book, the gametests** (7 advancements, 2 XMLs, 47 KB of gametests) are
   not carried.

## Where the glue sits against the SDK

| Need | SDK surface used |
|---|---|
| the block's Part | `#[mod_api::horizon_part] impl BlockPart` (empty: no block-level interaction) + `impl BlockEntityPart` (`block_entity`, `block_entity_ticks`, `block_entity_tick`) |
| the type and the container | `Registrar::block_key`, `declare_block_entity_type`, `attach_block_part`, `attach_block_entity_part`, `block_container(&block, ModContainer)` |
| the queue | the block entity's `nbt` compound in the tick (`ModBlockEntityView`, `block_entity_tick`'s `event.entity`); `BlockEntityRef::data`/`replace_data` from a command |
| the slots | `event.entity.items` in the tick; `BlockEntityRef::item_slot`/`set_item_slot` (inside `host.transaction`) from a command |
| the recipe | `Host::recipes_of_type("minecraft:crafting")`, `Ingredient::item_keys`, `Host::item_key`, `Host::max_stack_size` |
| the idle answer | `*event.reply = BlockEntityTickReply::Unchanged` |
| the commands | `Command::new("… <pos:block_pos> <item:item> <count:integer>")`, typed `CommandInvocation`, `Commands::feedback` |

Traps paid for in this port (new to it): `JavaString` has **no** `From<String>` and its inherent
`to_string()` returns `Result` — write `.as_str().into()` and read `.to_string_lossy()`; a `Tag`'s
`get(&str)` borrows a thread-local key, so a `&Tag` cannot outlive the call that made it (clone the
compound out of the event first); an empty `impl BlockPart for X {}` is legal and registers no
block hooks; `BlockEntityRef` reads take `&dyn Host` while `set_item_slot` takes
`&mut dyn ServerHost` **and** a `Transaction`, so a command writes through
`host.transaction(|host, tx| …)`.

## Skills

Skills: superpowers/using-superpowers,brainstorming,writing-plans,test-driven-development,systematic-debugging,verification-before-completion; BeautifulRust/rust-best-practices,rust-port

## Verification

Every build, test and package step ran through `ports/tools/build-slot.sh` with `BUILD_LOCAL=1`,
`CARGO_INCREMENTAL=0`, `rm -f Cargo.lock` before each package build, no bare `cargo` outside a
lease, no `--target-dir`. The content-load row compiled nothing — it used the **published shared
binaries** and a content-only package — so it took no slot ("`ctl` and custom ladders are local",
`build-slots-usage.md`). Logs: `ports/_shared/evidence/integratedcrafting/`.

| Step | Command | Result |
|---|---|---|
| engine only, **red** (`src/lib.rs` = `pub mod runtime;`) | `cargo test` | **14 failed**, 0 passed — every test `not yet implemented` (`sdk-test-runtime-only-red.log`) |
| engine only, **green, before the glue existed** | `cargo test` | **14 passed**, 0 failed (`sdk-test-runtime-only.log`) |
| whole crate | `cargo test` | **19 passed**, 0 failed (14 engine + 5 glue) (`sdk-test.log`) |
| lints | `cargo clippy --all-targets -- -D warnings` | exit 0, **0 warnings** (`sdk-clippy.log`) |
| formatting | `cargo fmt --check` | exit 0 after one `cargo fmt`; the first check failed on two layout nits (`sdk-fmt-first-run.log`) |
| package | `cargo horizon-mod build` | exit 0, `target/horizon-mod/integratedcrafting.hmod` (`sdk-build.log`) |
| reproducibility | build twice, `sha256sum` + `cmp` | **both `aab6b6fd…5da70`**, `BYTE-IDENTICAL` (`sdk-build-repro.log`) |
| package check | `cargo horizon-mod check integratedcrafting.hmod` | exit 0, verbatim below (`sdk-check.log`) |
| fits | `cargo horizon-mod fits integratedcrafting.hmod --build nightly` | exit 0, `0b14f8a  made for` (`sdk-fits.log`) |
| carried content, **loaded** | the shared unmodded game at the pin, with a **content-only** build of the same `assets/`+`data/`: `setblock … part_interface_crafting` → `{"block":"integratedcrafting:part_interface_crafting",…}`, `give` → `Gave 1 [Crafting Interface]`; **the same run without the package** → `Unknown block type 'integratedcrafting:part_interface_crafting'` and the position stays air | `content-load-transcript.txt`, `content-load-launch.log`, `content-load-control-launch.log` — the check is not vacuous |
| carried content | `carry-diff.txt` | texture byte-identical; every other carried file's change recorded with size + SHA-256 |

`check`, verbatim:

```
integratedcrafting 0.2.0 (native-mod, format native-1/0b14f8acfd97fa2c750e42795249dcc981178ca4/e59da9094f0c2fdda6c627fe9eff22dec6a906f0, for 26.3)
  34521 bytes, SHA-256 aab6b6fd5e83525343006d6cd8e698040d490bf314a7b0f0fba542c9c745da70, unsigned
  native source: unsandboxed; not executed by the registry check. Compile and test against the exact pinned Horizon and Mixins revisions before review.
```

`fits`, verbatim:

```
Integrated Crafting:
0b14f8a  made for
```

### The deliverable (one file)

`ports/integratedcrafting/integratedcrafting.hmod` is **the** deliverable:

- **34,521 B**, SHA-256
  `aab6b6fd5e83525343006d6cd8e698040d490bf314a7b0f0fba542c9c745da70`, **20 entries**:
  `horizon-mod.toml`, `Cargo.toml`, `LICENSE`, `LICENSE-NOTICE.md`, `README.md`, `src/lib.rs`,
  `src/runtime.rs`, `src/support.rs`, `uses.json` (`made_for 0b14f8a`), and the 11 carried content
  files under `assets/integratedcrafting/` and `data/` (including `data/c/tags/item/ingots/iron.json`).
- **no `Cargo.lock`** inside (a lock would pin newer crates than the SDK's own and change the sha).
- `cargo horizon-mod check` and `fits` accept it at `0b14f8a`.

## What still needs an in-game run

**The content half is verified; the native half is not.** The content-only variant of this package
(the same `assets/` and `data/`, no `[native]`) was launched on the **shared unmodded game** at the
pin under a scratch home, and its block, state name, item and display name all came from the
package — with the counterfactual that the same binary without the package answers `Unknown block
type`. Transcript and both launch logs: `ports/_shared/evidence/integratedcrafting/
content-load-*`.

What cannot run here is the **native** half: a native SDK mod is compiled into the game by the
**launcher**, and `horizon-launcher` is not installed in this environment (`cargo horizon-mod run`
answers: *"code mods run through the launcher's native build: horizon-launcher native-build .
--trust-native-code"*). `tools/e2e.sh` is written, `bash -n` clean, and **not run**:

```sh
horizon-launcher native-build ports/integratedcrafting --instance <instance> --trust-native-code
GAME=<that instance's horizon-client> CTL=ports/_shared/bin/horizon-1d63e0a5/horizon-ctl tools/e2e.sh
```

Three things only prove out there, and are named as risks rather than claims:

1. **The block entity comes from the declared type** on a content template of
   `minecraft:stone` — the same pattern the parent port uses (and the 0.1.0 `.hnmod` pins had as a
   hook); the SDK declares the type instead. If a launcher run shows no block entity and the
   interface never ticks, the one-line fix is a block-entity-capable template in
   `data/integratedcrafting/horizon/block/part_interface_crafting.json`.
2. **The declared container's 13 slots line up with `event.entity.items`** and with the hopper
   faces (`down` = results, `up` = ingredients). If hoppers do not reach it, `/integratedcrafting
   feed` still fills the ingredient slots, so the engine is testable without them.
3. **The model and texture draw.** The content load proved the block's *state* resolves; a headless
   run has no renderer, so the carried model (parent rewritten to `block/block`, the jar's ten
   cubes) and its texture are unproven until someone looks at the block in a windowed run.

## Known gaps

The slice boundary and gaps above stand. Recorded for this slice:

- `TICKS_PER_CRAFT` is fixed at 20; upstream measures each machine.
- one job at a time, no priorities, no distribution, no dependency graph;
- a job's ingredients can be taken by an earlier queued job between submission and the craft (the
  engine does not reserve slots); the job then blocks and names the deficit — which is the honest
  report, not a silent failure;
- `minecraft:crafting` only; a `RecipeRule` type (stonecutting, smithing, special) resolves to no
  recipe and is refused as `no crafting recipe makes …`;
- the interface has no screen, no comparator output and no block state for `running`/`blocked`; its
  state is in the readout and in the saved compound;
- the ingredient item ids in a job's saved cells are sorted, so a job and its readout do not depend
  on a set's iteration order.

## Changelog for the first publication

> First publication: no earlier PR. A flat SDK package (`api = "0.2"`, `[native]` pinned to
> `0b14f8a`/`e59da90`), version **0.2.0** — the campaign's package version, not the Java mod's
> 1.8.1. It carries the crafting-job engine as a pure, unit-tested `src/runtime.rs` (14 tests) plus
> the Crafting Interface block that drives it (5 glue tests), the readout and three working
> commands, and the interface's carried content. The network half is `blocked(...)` with the exact
> symbols it would need, and the crafting writer/attuned/enumerated variants are recorded as
> unported, not stubbed.
