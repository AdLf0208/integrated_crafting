# Integrated Crafting → Horizon (slice 1: the engine and the interface; slice 2: the dependency graph)

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

## Review fixes (2026-11-18) — the registry's source review, package 0.2.1

The reviewer's file is `var/review/1270-integratedcrafting.md` (**held: this blocked listing**).
Every item is a change in the tree plus the test or run that pins it. New package:
**`integratedcrafting.hmod` — 40,878 B, sha256 `0798935ecaaaff52301b2160ae580deae33f34b1daf927d25b2170b9d479d4fc`**,
`check` = `integratedcrafting 0.2.1 (native-mod, …)`, `fits --build nightly` = `0b14f8a made for`.

### 1. The level-0 `feed` command created items in any loaded interface

**Reproduced** (`src/support.rs:334` in 0.2.0, into `plan_insert`/`write_slots`): `feed <pos> <item>
<count>` wrote the requested stack into the input slots with no inventory behind it — a survival
player could summon any item, and any interface at any coordinate.

**Fixed.** `feed` now **moves the acting player's own items**: it reads the player's inventory
through the `horizon:entity_items` capability, caps the amount at what the player holds, takes the
items in a transaction and writes the slots — nothing is created, and a slot the game refuses never
costs the player an item (the write is reverted and the items go back). The interface must also be
within [`REACH_BLOCKS`] (6.0 = vanilla's 4.5-block interaction range plus the server's own one-block
buffer) of a player source; the console has no inventory and is refused for that reason.

**Pinned by** the in-game phase F: after the interface is filled and the player's logs are spent,
`/integratedcrafting feed … minecraft:oak_log 2` answers `the player holds no minecraft:oak_log to
feed` and the interface stays `idle`; phase A asserts the player's oak logs are **0** after the feed
(the slots took the items, not copies).

### 2. `request`/`cancel` acted at any coordinates, and the queue had no cap

**Reproduced:** `src/support.rs:328` registered `request` and `cancel` at level 0 and the body
acted on any loaded interface; `src/runtime.rs:343` appended every accepted job with no limit, and
submission checked the same inputs again for each job without reserving them — so a player could
grow a blocked interface's saved NBT without bound.

**Fixed, three parts:**

- **Reach.** `request`, `cancel` and `feed` require a player source within [`REACH_BLOCKS`] of the
  interface and answer `stand within 6 blocks of the interface.` otherwise. `status` stays open: it
  is a readout of this mod's own block.
- **A queue cap.** [`MAX_QUEUE`] = 8; a ninth request is refused with `Refusal::QueueFull`
  (`the queue is full (8 jobs)`), so the saved compound and the per-tick rebuild are bounded.
- **A real reservation.** `Engine::submit` now checks the new job against the slots **minus what
  the queued jobs still need** (`free_inputs` → `reserve`, the same greedy walk the deficit uses),
  and a job reserves only its *remaining* crafts (`crafts_needed - crafts`), so a craft releases
  its reservation as it consumes the real item and a cancel releases all of it.

**Pinned by** `the_queue_has_a_cap`, `a_queued_job_reserves_the_ingredients_it_still_needs` (two logs,
a job for eight planks, a second request refused with `have 0`) and
`a_reservation_is_released_as_the_job_crafts`; `a_player_must_be_within_reach_of_the_interface` for
the distance rule. One existing test,
`a_cancelled_job_never_consumes_anything`, now starts from four logs instead of two: with the
reservation, a second job can no longer claim the logs the first one reserved — that is the fixed
behaviour, and the test still proves what it was written for (a cancelled job consumes nothing).

### 3. The entry says `kind = "mod"` for a `[native]` package

The **package** was already consistent and is unchanged in this respect: `horizon-mod.toml` carries
`api = "0.2"` with a `[native]` section (game 26.3, horizon `0b14f8a…`, mixins `e59da90…`) and no
`kind` key — `kind` is derived from the package, and `cargo horizon-mod check` reports
`integratedcrafting 0.2.1 (native-mod, format native-1/0b14f8a…/e59da90…, for 26.3)`. The entry's
`kind` is the orchestrator's edit; the artefact and the manifest already agree.

### 4. Three defects the in-game run found (this port had never run in game)

1. **The game crashed at startup: `interface is attached to integratedcrafting:part_interface_crafting
   twice`.** `expand_block_entity_parts` copies a block entity's Part into the block table under the
   same name, so the extra `attach_block_part(block, "interface", …)` was refused and the refused
   list aborted the launch. **Fixed** by attaching the Part once (through the block-entity type);
   recorded in `src/lib.rs`.
2. **The interface's slots never reached the tick.** A command's `BlockEntityRef::set_item_slot`
   write was readable by the next command and invisible to `BlockEntityTickEvent.entity.items`, so
   the next tick wrote an empty container back and a feed was wiped. **Fixed** by keeping the 13
   slots in the block entity's **saved compound** (`ic_slots`, `ic_mirror`), which both a command
   and the tick read and write; the tick mirrors them into the container hoppers reach and folds an
   outside change back with `merge_container` (a slot whose container value differs from the mirror
   was changed by a hopper and wins). Also measured: a `replace_data` *inside* a transaction is
   rolled back by it, so the feed charges the player first and writes the payload outside.
3. **A `replace_data` that replaced the whole payload dropped the slots** — the request after a feed
   wiped the items because `save()` wrote a fresh compound holding only the queue. **Fixed** by
   `save_engine`, which writes the queue into the existing compound and leaves every other key
   alone.

Pinned by the in-game run (A: two logs fed, a job accepted, `craft 0/2`; B: the interface idles and
the planks reach the chest below) and by `the_saved_slots_round_trip`,
`an_external_change_to_the_container_wins_over_the_saved_slots`,
`a_first_tick_takes_the_container_as_it_stands`.

### 5. Upstream defect fixed: the recipe could never load at 26.3

The carried recipe wrote its ingredients in the 1.21.1 object form (`{"item": …}`, `{"tag": …}`). At
26.3 an ingredient object must name a `type`/`neoforge:ingredient_type`
(`crates/horizon-mod-vocab/src/__private/ingredient_parser.rs:186`), and a plain item is the
**string** form — vanilla's own `data/minecraft/recipe/glass_pane.json` writes `"#":
"minecraft:glass"`. Carried as-is, the engine logged `recipes: invalid ingredient (unresolved tag or
malformed input)` and the interface's own recipe never loaded. **Fixed**: the key map is now
`"C": "minecraft:crafting_table"`, `"I": "#c:ingots/iron"`, `"V"/"W": "integrateddynamics:cable"`,
`"M": "integrateddynamics:crystalized_menril_chunk"`, same pattern, same result.

### Ladder and in-game leg for 0.2.1

| Step | Command | Result |
|---|---|---|
| unit tests RED | `cargo test --lib` before the cap/reservation | `error[E0425]`/`E0599`, 14 errors — `evidence/integratedcrafting/review-red-queue.log` |
| unit tests | `cargo test --lib --offline` | **26 passed**, 0 failed (was 19; +7) |
| lints | `cargo clippy --all-targets --offline -- -D warnings` | exit 0, 0 warnings |
| formatting | `cargo fmt --check` | clean |
| package | `cargo horizon-mod build` twice | `40,878 B`, sha256 `0798935e…d479d4fc`, **byte-identical** |
| registry check | `cargo horizon-mod check integratedcrafting.hmod` | `native-mod, format native-1/0b14f8a…/e59da90…, for 26.3`, EXIT 0 |
| fits | `cargo horizon-mod fits integratedcrafting.hmod --build nightly` | `0b14f8a made for`, EXIT 0 |
| in-game | `ports/tools/launcher-game.sh run integratedcrafting` | **PASS** + `EXIT:0` — `evidence/integratedcrafting-e2e/e2e-launcher.log` |
| control | `GAME=$CONTROL_GAME` | `FAIL: the crafting interface was not placed` + `EXIT:1` — `…-control.log` |
| selftest | `SELFTEST=1` | `FAIL: A craft count … 'craft 7/9'` + `EXIT:1` — `…-selftest.log` |
| log gate | `check-e2e-logs.py integratedcrafting-e2e --strict` | `logs=3 ok=3 missing-exit=0 bad-exit=0` |

**Scope note:** the interface's slots keep an item id and a count, so a stack's components (a
damaged tool, an enchanted book) are not preserved through the job — the engine's model, unchanged
by this package and recorded under `## Known gaps`.

## Slice 2: the job dependency graph (2026-11-21) — package 0.2.2

**Pin:** Horizon `0a875da91dbb0fdcb7ba8c42c11341aa229af51a` (Nightly 2026-11-20) + horizon-mixins
`e59da9094f0c2fdda6c627fe9eff22dec6a906f0`; `package_format` 6, `content_api` 0.2. The manifest
moved from `0b14f8a`; `cargo horizon-mod update --horizon 0a875da…` reported *"2 of the 308 items it
uses changed"* — both are the local-type collision `fits` reports (below), so **no source change was
needed for the repin**. `.horizon/sdk` was re-seeded from `ports/_shared/sdk/0a875da/`; the copy
keeps the release's 1970 mtimes, which leaves a **stale `OUT_DIR`** for `horizon-mod-vocab` (nine
errors about `MobTickCost`, `#[raw_text]`, …); `touch`ing the seeded tree fixes it (paid for here).

**Deliverable:** `integratedcrafting.hmod`, **46,984 B**, SHA-256
`d69d48d54a4ec2f0ba3c937dfb92c8e5044c4d19d63a806d439bc99371082ebc`, 20 entries, no `Cargo.lock`.

### The gap this slice closes

`## Gaps (next slices)` item 4, verbatim from 0.2.1:

> 4. **The job dependency graph** — a job whose ingredient is itself craftable
>    (`CraftingJobDependencyGraph`, `PartialCraftingJobCalculation`) is not implemented: a request
>    that needs a crafted intermediate is refused as missing ingredients. This is the largest
>    remaining piece of the engine.

### What it does

- **`Engine::submit` plans the whole chain before it accepts anything.** A missing ingredient cell
  whose item is itself craftable becomes a dependency, planned first and stored with the job; the
  chain runs leaf first and the requested recipe last. A cell's candidate items are tried in their
  sorted order (the table's cells are sorted, so a saved job does not depend on a set's iteration
  order), and the first one whose own plan fits the slots wins: a log in the slots makes
  `minecraft:oak_planks` the plank to craft, not the alphabetically earlier acacia plank whose log
  the slots do not hold.
- **The reservation is a simulation of the queue.** The old "crafts left × recipe" reservation
  cannot describe a chain, so `free_inputs` walks every queued job's remaining steps in order,
  takes what they consume and adds what their dependency steps will produce. A queued chain
  therefore reserves the *leaf* items it still needs, and a dependency's output is not counted
  twice.
- **Execution is one step at a time.** A dependency's craft puts its product into the ingredient
  slots (where the step above spends it); the requested recipe's product goes to the result slots.
  The readout names the step (`step 1/2`) and the step's own craft count; the acceptance reply
  names the plan (`(crafts: 1x minecraft:oak_planks, 1x minecraft:stick)`).
- **Bounds:** `MAX_DEPENDENCY_DEPTH` = 8, and an output already on the chain is not followed
  (a recipe cycle is refused with the missing cell, not a hang). A dependency whose product cannot
  fit in the ingredient slots blocks with `ingredient slots full` and consumes nothing.
- **Saves:** a job is stored as its `steps` (recipe, cells, crafts, progress, sink, limit); a job
  compound written by 0.2.1 (one recipe at the top level) is read as a one-step job, so an upgrade
  keeps a running job.

### Upstream source read (analysis aid, clean-room rewrite)

Repository `https://github.com/CyclopsMC/IntegratedCrafting`, commit
`01effd6901581a0986a25e3a007ef0897d3ee6fa` (2026-10-31, shallow clone under `/tmp`, deleted after
this record). Files read: `core/CraftingHelpers.java` (`calculateCraftingJob`,
`calculateCraftingJobForRecipe`, `calculateCraftingJobDependency…`),
`core/PartialCraftingJobCalculation.java`, `api/crafting/CraftingJobDependencyGraph.java`.

Behaviours extracted and followed: a recipe's inputs are simulated against storage; every missing
ingredient is resolved recursively into a dependency job **before** the parent job is created; a
dependency failure aborts the whole calculation with the missing pieces; dependencies' surplus is
fed back into the simulation so sibling dependencies do not double-count; a recursive recipe throws
and is refused. Deviations from upstream, deliberately:

| Upstream | Here | Why |
|---|---|---|
| Dependencies are separate `CraftingJob`s linked in a network-wide graph, one job per unique recipe, amounts merged when the same recipe appears again | the chain is a list of `Step`s of the one job | there is no network to hold separate jobs; one request, one saved job, one queue entry (and the queue cap stays 8) |
| Orders are pulled from the network as the dependencies complete | a dependency's product is inserted into the interface's own ingredient slots | the network gap (item 1) |
| A dependency can itself carry further dependencies without a fixed bound | at most `MAX_DEPENDENCY_DEPTH` = 8 | the whole chain is saved in the block entity; a bound keeps that compound finite |
| Surplus from an over-producing dependency stays in the network | surplus stays in the ingredient slots (visible in the readout's slot data) | same inputs, same honesty a crafting table has |

### Red proofs

**Unit (before the planner existed), verbatim** — `sdk-test-red-dependency.log`:

```
test runtime::tests::a_request_crafts_a_craftable_intermediate_before_the_requested_recipe ... FAILED
thread 'runtime::tests::a_request_crafts_a_craftable_intermediate_before_the_requested_recipe' panicked at src/runtime.rs:1321:14:
one log makes four planks, and two of those make four sticks: MissingIngredients([Missing { candidates: ["minecraft:oak_planks"], needed: 2, have: 0 }])
test result: FAILED. 29 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out
```

**In game, the same phase against the pre-slice package** (0.2.1's engine at the new pin; log
`e2e-launcher-red-old-code.log`):

```
== G: one log and a request for four sticks runs the planks first ==
  G feed: Gave 1 [Oak Log] to Player62591 / Integrated Crafting: inserted 1 of 1
FAIL: G request: the reply did not contain 'job 1 accepted for 4 x minecraft:stick': Integrated Crafting: missing 2 x minecraft:acacia_planks|minecraft:bamboo_planks|minecraft:birch_planks|minecraft:cherry_planks|minecraft:crimson_planks|minecraft:dark_oak_planks|minecraft:jungle_planks|minecraft:mangrove_planks|minecraft:oak_planks|minecraft:pale_oak_planks|minecraft:poplar_planks|minecraft:spruce_planks|minecraft:warped_planks (have 0)
EXIT:1
```

### An in-game defect this slice found and fixed: the hopper duplicated the result

Phase B of the old run had left the evidence in its own log, unread because the assertion only
asked *whether* planks were in the chest, not *how many*:

```
  chest 300,99,0: 300, 99, 0 has the following block data: [{count: 12, Slot: 0b, id: "minecraft:oak_planks"}]
```

An eight-plank job left **twelve** planks. The cause: the block entity's container mirror
(`ic_mirror`) recorded the container's content *before* the tick wrote into it, and the result
slots were never merged back from the container at all — so every tick the interface re-inserted
the stack a hopper had just taken, and the hopper drained it again. The same refill showed in the
first run of phase I: a one-interface craft left **four** interfaces in the chest (`count: 4`, the
development run before the fix; the preserved red is phase B's twelve planks above).

**Fixed:** the tick now merges the result slots as well as the ingredient slots, against the state
the interface itself last wrote (`write_items` is given the slots it overwrote `ic_mirror` with;
the `feed` path writes only `ic_slots` and leaves the mirror alone). **Pinned by the exact chest
counts** in phases B (8), G (4) and I (1) — an assertion that fails on the old package and on any
future refill, and by `a_mirror_of_the_written_state_lets_a_hopper_take_a_result`.

### The carried recipe, proven in game

The package's one recipe names the parent port's items, so it needs a game with **both** packages.
`native-build` accepts "a folder of them", so the slice built one game carrying `integratedcrafting`
and `integrateddynamics` (both at the campaign pin) and ran the same script with `PARENT_ITEMS=1`:

```
== I: the carried recipe, crafted by the interface ==
  I feed cable: … Integrated Crafting: inserted 2 of 2
  I feed crafting table: … inserted 2 of 2
  I feed iron: … inserted 4 of 4
  I feed menril chunk: … inserted 1 of 1
  I request: Integrated Crafting: job 1 accepted for 1 x integratedcrafting:part_interface_crafting
  I status: Integrated Crafting: interface at 302,101,0: idle
  chest 302,99,0: 1 x integratedcrafting:part_interface_crafting
  I ingredients consumed: 302, 101, 0 has the following block data: [{}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}]
```

Before the fix to the phase (the first attempt fed 2 iron and 1 table), the reply itself proved the
recipe had loaded and resolved its tag: *"missing 4 x minecraft:iron_ingot (have 2); missing 2 x
minecraft:crafting_table (have 1)"* — four iron cells through `#c:ingots/iron`, two table cells.
The single-package leg skips phase I and says so (`I skipped: integrateddynamics:cable is absent
from this single-package game`).

### The loot table, measured

Phase H rolls the carried table with `/loot give @s loot
integratedcrafting:blocks/part_interface_crafting` → one interface, and with
`…:blocks/no_such_table` → nothing (the differential). See `dialect-flags.md`: the checker's two
flags on that file (`bonus_rolls`, float `rolls`) are not fatal at 26.3.

### `fits --build 0b14f8a` is red, and a proved false positive

`cargo horizon-mod fits integratedcrafting.hmod --build 0b14f8a` reports *"2 items changed"*:
`type:Item` matched from `CommandArgumentValue::Item(stack)` in `src/support.rs` and `type:Recipe`
matched from this crate's own pure `Recipe` type. Both surface entries are **byte-identical** in
`0a875da` and `0b14f8a` (hash `af9ca6ca…` each), and the `item` command argument works in every
leg. The proof, with the two surface hashes and the game transcript, is
`evidence/integratedcrafting/fits-other-FALSE-POSITIVE.md`.

### Ladder and in-game legs for 0.2.2

| Step | Command | Result | Evidence |
|---|---|---|---|
| unit tests RED | `cargo test --lib` before the planner | **5 failed**, 29 passed; the chain test refused with `MissingIngredients` | `sdk-test-red-dependency.log` |
| unit tests | `cargo test --lib --offline` | **37 passed**, 0 failed (was 26; +8 runtime, +3 glue) | `sdk-test.log` |
| lints | `cargo clippy --all-targets --offline -- -D warnings` | exit 0, 0 warnings | `final-clippy.log` |
| formatting | `cargo fmt --check` | exit 0 | `final-fmt.log` |
| package | `cargo horizon-mod build` twice, `rm -f Cargo.lock` before each | both **46,984 B**, `d69d48d5…082ebc`, `cmp` → `BYTE-IDENTICAL` | `sdk-build-repro.log` |
| registry check | `cargo horizon-mod check integratedcrafting.hmod` | `native-mod, format native-1/0a875da…/e59da90…, for 26.3`, EXIT 0 | `sdk-check.log` |
| fits (pin) | `cargo horizon-mod fits … --build 0a875da` | `0a875da  made for`, EXIT 0 | `sdk-fits.log` |
| fits (other) | `cargo horizon-mod fits … --build 0b14f8a` | `does not fit: 2 items changed` — the proved false positive, EXIT 1 | `sdk-fits-other.log`, `fits-other-FALSE-POSITIVE.md` |
| in-game | `ports/tools/launcher-game.sh run integratedcrafting` | **PASS** + `EXIT:0` (A–H, I skipped) | `e2e-launcher.log` |
| in-game + parent | both packages in one native build, `PARENT_ITEMS=1` | **PASS** + `EXIT:0` (A–I, recipe crafted) | `e2e-launcher-with-parent.log` |
| control | `GAME=$CONTROL_GAME` | `FAIL: the crafting interface was not placed` + `EXIT:1` | `e2e-launcher-control.log` |
| selftest A | `SELFTEST=1` | `FAIL: A craft count … 'craft 7/9'` + `EXIT:1` | `e2e-launcher-selftest.log` |
| selftest G | `SELFTEST=dependency` | `FAIL: G plan (selftest) …` + `EXIT:1` | `e2e-launcher-selftest-dependency.log` |
| log gate | `check-e2e-logs.py integratedcrafting-e2e --strict` | `logs=6 ok=6 missing-exit=0 bad-exit=0` | — |
| build == delivered | `cmp` of the built game's `src/` + manifests | `same src/lib.rs`, `src/runtime.rs`, `src/support.rs`, both manifests | `build-matches-delivered.log` |
| gate | `verify-sdk-package.py integratedcrafting` | **1/1** | — |

### Harness defects fixed in this slice (the e2e had been passing vacuously)

1. **Phase D's "job 1 accepted" matched phase A's stale chat line.** `ic()` read the last twelve
   chat lines, so an old reply could pass for a new one (the `botanypots` lesson). The script now
   reads only the chat events that arrived after the command (`ctl events --since`, with the
   sequence kept in a file because `ic` runs in a command substitution), and phase D asserts
   `job 2 accepted` — which is the reply the interface actually gives. The stale-chat version is
   what made the old leg look green on a reply it never received.
2. **"The chest holds planks" did not name a count**, which is how the duplication above survived.
   `expect_in_chest` now takes the expected count and fails if the chest ever holds more than the
   job made; the chest is read with a fresh `data get`.

### Coverage (family → state)

| Family (jar) | State |
|---|---|
| crafting-job engine: request → recipe, reservation, 20 ticks/craft, blocked reasons, queue cap, cancel | **tested** — 37 unit tests; e2e A–F |
| job dependency graph (craftable intermediate), depth cap, cycle refusal | **tested** — 8 unit tests + 3 glue; e2e G (one log → planks → 4 sticks, 2 planks left, log consumed) |
| commands `status`/`request`/`cancel`/`feed`, reach rule, no item creation | **tested** — e2e A–F/G; unit tests |
| block entity, 13-slot container, hopper faces, result handoff without duplication | **tested** — e2e A/B (exactly 8 planks in the chest), G (4 sticks), I (1 interface) |
| carried block, item, state, model and lang | **tested** — placed/read/given in e2e A; the item comes back through the loot roll (H) and the craft (I) |
| carried recipe `part_interface_crafting` (incl. `#c:ingots/iron`) | **tested** — e2e I, in a game with the parent package; 9 ingredients consumed |
| carried loot table | **tested** — e2e H, with an unknown-table differential |
| saved jobs, including a job saved by 0.2.1 | **tested** — `a_saved_queue_reads_back_as_it_was_written`, `a_saved_chain_reads_back_with_its_steps`, `a_job_saved_by_the_previous_package_reads_as_one_step` |
| network attachment, crafting writer/aspects, part screens, attuned/enumerated variants | **blocked / not ported** — gaps 1–3 |
| priorities, distributed jobs, several CPUs, blocking-mode settings, re-usable ingredients, fluids, energy, bound machine, other recipe types, advancements/info book | **not ported** — gaps 5–8 |

Note on the test count: 26 → 37 is +11 — eight runtime tests for the chain (five red first, three
refusal/bound tests) and three glue tests (the 0.2.1 save format, a saved chain, and the result
handoff policy). The runtime module holds 28 tests, the glue 9.

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
4. ~~**The job dependency graph** …~~ **done in slice 2** (2026-11-21): a request whose ingredient
   is itself craftable plans the chain (bounded at 8, cycles refused); see "Slice 2". What is left
   of it is the network form of it: separate jobs, one per unique recipe, merged amounts, priorities
   (gaps 1 and 5).
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

Skills: superpowers/using-superpowers,brainstorming,writing-plans,test-driven-development,verification-before-completion; BeautifulRust/rust-best-practices

## Verification (slice 1, package 0.2.0 — historical; 0.2.2 is in "Slice 2" and `VALIDATION.md`)

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

## What still needs an in-game run (slice 1's status — superseded by slice 2)

**Settled at 0.2.2:** the native half now runs in a launcher-built game (`tools/e2e.sh`, phases
A–I; `e2e-launcher.log`, `e2e-launcher-with-parent.log`). The three risks below were all measured
green: the block entity comes from the declared type and ticks; the 13 slots line up with
`event.entity.items` and the hopper faces; the model and texture load (a windowed run was still not
made — a headless run has no renderer, so the *look* remains unproven, only the state/model data).

The slice-1 text is kept for the record: **the content half is verified; the native half was not.** The content-only variant of this package
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
- a job reserves its remaining leaf ingredients (0.2.1), so an earlier queued job cannot take them;
  a blocked job still names the exact deficit when the slots do not have it;
- `minecraft:crafting` only; a `RecipeRule` type (stonecutting, smithing, special) resolves to no
  recipe and is refused as `no crafting recipe makes …`;
- the interface has no screen, no comparator output and no block state for `running`/`blocked`; its
  state is in the readout and in the saved compound;
- the ingredient item ids in a job's saved cells are sorted, so a job and its readout do not depend
  on a set's iteration order.

## Changelog for 0.2.2

> **Update `integratedcrafting` 0.2.2** (supersedes 0.2.1; 0.2.1 was never published on its own).
>
> - **The job dependency graph.** A request whose ingredient is itself craftable plans the chain and
>   runs it leaf first: one log becomes planks, the planks become four sticks, in one job. The
>   acceptance reply names the plan, the readout names the step (`step 1/2`), the chain is bounded
>   at eight steps, a recipe cycle is refused rather than followed, and a queued chain reserves the
>   leaf items it still needs.
> - **Fixed: a hopper under the interface duplicated the job's output.** An eight-plank job left
>   twelve planks in the chest and a one-interface craft left four interfaces, because the result
>   slots were never folded back from the container and the mirror held the pre-write state. The
>   tick now merges both halves against the state it wrote; the e2e asserts the exact chest counts.
> - **Proven in game**: the carried recipe (crafted through the interface, in a game with the
>   parent package), the carried loot table (rolled, with an unknown-table differential), the block
>   entity, the 13-slot container and the hopper handoff.
> - Repinned to Horizon `0a875da` (Nightly 2026-11-20) + mixins `e59da909…`; `api = "0.2"`,
>   `package_format` 6. No source change was needed for the repin.

## Changelog for the first publication

> First publication: no earlier PR. A flat SDK package (`api = "0.2"`, `[native]` pinned to
> `0b14f8a`/`e59da90`), version **0.2.0** — the campaign's package version, not the Java mod's
> 1.8.1. It carries the crafting-job engine as a pure, unit-tested `src/runtime.rs` (14 tests) plus
> the Crafting Interface block that drives it (5 glue tests), the readout and three working
> commands, and the interface's carried content. The network half is `blocked(...)` with the exact
> symbols it would need, and the crafting writer/attuned/enumerated variants are recorded as
> unported, not stubbed.
