# Integrated Crafting — a Horizon port

**Integrated Crafting** is a Minecraft mod. Craft stuff in Integrated Dynamics networks. This repository is a **port of it to [Horizon](https://github.com/horizon-rust/horizon)**, the Rust engine for Minecraft Java 26.3. It carries the complete source of the port plus the content files it ships, and installs as a single `.hmod` package.

Upstream: [CyclopsMC/IntegratedCrafting](https://github.com/CyclopsMC/IntegratedCrafting) — licence **MIT**.

| | |
|---|---|
| Upstream mod | Integrated Crafting ([source](https://github.com/CyclopsMC/IntegratedCrafting)) |
| Ported to | Horizon 0b14f8a, Minecraft Java 26.3, mod API `0.2` |
| Package | `integratedcrafting.hmod` (install it with the Horizon launcher) |
| Licence | **MIT** — the upstream mod's licence; see [Licence and credit](#licence-and-credit) |

## What this port implements

- **`integratedcrafting:part_interface_crafting`** — a part-shaped block (the carried part model:
- **The engine** (`src/runtime.rs`, 14 tests, no game types): a request resolves to a recipe and
- **The tick** (`src/support.rs`): rebuilds the engine from the compound, advances it once, writes
- **`/integratedcrafting status <pos>`** — the readout: the queue, the running job's craft and
- **`/integratedcrafting request|cancel|feed`** — submit a job, drop every unfinished job, and put
- **Content**: the interface's texture (byte-identical), model geometry (the jar's ten cubes), a
- `struct Cable` with `#[mod_api::horizon_part] impl BlockPart` (`block_signal`, `is_signal_source`,
- A redstone **channel** in the cable's saved compound under the key `value` (0..15) and a `status`

## Not yet ported

- **`blocked(no-parent-network-attachment-api)`** — the network half: a part on
- **`blocked(no-variable-card-layer)`** — the Crafting Writer part and every aspect the mod ships
- **The part screens** — upstream has five containers and their screens
- **The job dependency graph** — a job whose ingredient is itself craftable
- **Priorities, distributed jobs over several interfaces, several crafting CPUs, blocking-mode
- **The bound machine** — inserting the inputs into the block the interface faces through the item
- **Recipe types other than `minecraft:crafting`** (`smelting`, `stonecutting`, `smithing` are in
- **Advancements, the info book, the gametests** (7 advancements, 2 XMLs, 47 KB of gametests) are

## Installing

1. Install the [Horizon launcher](https://github.com/horizon-rust/horizon/releases) (0.2.70 or newer).
2. Add `integratedcrafting.hmod` to the instance's mods.
3. The package is pinned to a specific Horizon build; the launcher checks that pin.

## Licence and credit

Integrated Crafting was created by its upstream authors; the code, models and assets in the original mod
remain theirs and are used here under **MIT**. The Rust in this repository is a clean-room
rewrite for Horizon, not a translation of the original source.

- `LICENSE` — the upstream licence text.
- `LICENSE-NOTICE.md` — what this port carries and where it came from.
- The upstream project: https://github.com/CyclopsMC/IntegratedCrafting

## Technical details

```
horizon-mod.toml   package manifest (id, version, api, [native] pin)
Cargo.toml         the crate; depends on `horizon-mod-api` from the fetched SDK
src/               lib.rs (the one `impl Hooks`), runtime.rs (pure logic + tests), glue
assets/ data/      the content the package ships
tools/             content generators and the e2e scenario
```

- Build: `cargo horizon-mod build` (packages; it does not compile against the game).
- Test: `cargo test`, `cargo clippy --all-targets -- -D warnings`.
- In-game: `ports/tools/launcher-game.sh run integratedcrafting` in the campaign tree, or install the
  `.hmod` and play; the harness scenario is `tools/e2e.sh`.

This repository is generated from the campaign's port at `ports/integratedcrafting/`; the port's own
`notes.md` is the full record (decisions, traps, evidence).
