# Validation — integratedcrafting 0.2.2 (slice 2)

Pin: Horizon `0a875da91dbb0fdcb7ba8c42c11341aa229af51a` (Nightly 2026-11-20) + horizon-mixins
`e59da9094f0c2fdda6c627fe9eff22dec6a906f0`. Every build, test and package step ran through
`ports/tools/build-slot.sh` (`BUILD_LOCAL=1`, `CARGO_INCREMENTAL=0`, `rm -f Cargo.lock` before each
package build), no bare `cargo`, no `--target-dir`. Logs are in
`ports/_shared/evidence/integratedcrafting/` and `ports/_shared/evidence/integratedcrafting-e2e/`.

**Delivered:** `integratedcrafting.hmod`, **46,984 B**, SHA-256
`d69d48d54a4ec2f0ba3c937dfb92c8e5044c4d19d63a806d439bc99371082ebc`, 20 entries, no `Cargo.lock`.

## The ladder

| # | Step | Command | Result | Evidence |
|---|---|---|---|---|
| 1 | unit tests, **red** | `cargo test --lib` with the chain tests, before the planner | **5 failed / 29 passed**; the chain request refused with `MissingIngredients([Missing { candidates: ["minecraft:oak_planks"], needed: 2, have: 0 }])` | `sdk-test-red-dependency.log` |
| 2 | unit tests, green | `cargo test --lib --offline` | **37 passed / 0 failed** (26 at 0.2.1) | `sdk-test.log` |
| 3 | lints | `cargo clippy --all-targets --offline -- -D warnings` | exit 0, 0 warnings | `final-clippy.log` |
| 4 | format | `cargo fmt --check` | exit 0 | `final-fmt.log` |
| 5 | package | `cargo horizon-mod build` twice, `Cargo.lock` removed before each | both **46,984 B**, `d69d48d5…082ebc`, `cmp` → `BYTE-IDENTICAL` | `sdk-build-repro.log` |
| 6 | registry check | `cargo horizon-mod check integratedcrafting.hmod` | `native-mod, format native-1/0a875da…/e59da90…, for 26.3`, EXIT 0 | `sdk-check.log` |
| 7 | fits, pin | `cargo horizon-mod fits … --build 0a875da` | `0a875da  made for`, EXIT 0 | `sdk-fits.log` |
| 8 | fits, other | `cargo horizon-mod fits … --build 0b14f8a` | `does not fit: 2 items changed` — the proved local-type false positive, EXIT 1 | `sdk-fits-other.log`, `fits-other-FALSE-POSITIVE.md` |
| 9 | SDK gate | `python3 ports/tools/verify-sdk-package.py integratedcrafting` | **1/1** (package, runtime byte-identical, no lock, tests, clippy, check, fits, skills) | — |
| 10 | dialect gate | `check_261_dialect.py integratedcrafting.hmod` | recipe clean; two unverified loot spellings (`bonus_rolls`, float `rolls`) flagged and **measured accepted** in game | `dialect-flags.md` |

## In-game legs (launcher-built game at the pin)

`ports/tools/launcher-game.sh run integratedcrafting` builds the game from this port's manifest and
sources; `build-matches-delivered.log` proves the built game's `src/lib.rs`, `src/runtime.rs`,
`src/support.rs` and both manifests are byte-identical to the delivered tree.

| Leg | Command | Result | Evidence |
|---|---|---|---|
| green (this package) | `launcher-game.sh run integratedcrafting` | **PASS** + `EXIT:0` — A–H, phase I skipped (no parent item) | `e2e-launcher.log` |
| green (with the parent package) | both packages in one `native-build`, `PARENT_ITEMS=1` | **PASS** + `EXIT:0` — A–I; the carried recipe crafted, 9 ingredients consumed, 1 interface in the chest | `e2e-launcher-with-parent.log` |
| control | the same Nightly **without** the package (`GAME=$CONTROL_GAME`) | `FAIL: the crafting interface was not placed` + `EXIT:1` | `e2e-launcher-control.log` |
| selftest A | `SELFTEST=1` (expects `craft 7/9`) | `FAIL: A craft count …` + `EXIT:1` | `e2e-launcher-selftest.log` |
| selftest G | `SELFTEST=dependency` (expects a plan the engine cannot make) | `FAIL: G plan (selftest): …` + `EXIT:1` | `e2e-launcher-selftest-dependency.log` |
| pre-slice package (red) | the 0.2.1 code at the pin, same script | `FAIL: G request: … missing 2 x …planks… (have 0)` + `EXIT:1` | `e2e-launcher-red-old-code.log` |
| log gate | `check-e2e-logs.py integratedcrafting-e2e --strict` | `logs=6 ok=6 missing-exit=0 bad-exit=0` | — |

Every assertion reads chat **events after the command** (`ctl events --since`, sequence in a file),
never the last N lines; the control and both selftests end `EXIT:1`, so the scenario can fail.

## What the in-game leg proves

- **Phases A/B:** two logs → eight planks in two crafts; the chest below holds **exactly 8** (the
  count is the assertion: an earlier build left 12, the duplication fixed in this slice).
- **Phase C:** a stick request with no planks is refused with the cell's deficit and its
  candidates.
- **Phase D:** a cancelled job consumes nothing; the reply is the fresh `job 2 accepted` (the old
  script matched phase A's stale line — fixed).
- **Phase E:** an item no recipe makes is refused by name.
- **Phase F:** feeding items the player does not hold creates nothing.
- **Phase G:** one log → planks → **exactly 4 sticks**; the acceptance reply names the two-step
  plan; the result slot readback shows 2 planks left and no log.
- **Phase H:** the loot table rolls **exactly 1** interface; an unknown table rolls nothing.
- **Phase I** (parent package present): the carried recipe loads with `#c:ingots/iron`, is
  requested through the interface, consumes all nine ingredients and puts **exactly 1** interface in
  the chest.

## Why the new tests are fail-capable

- **Chain:** one log + four sticks — a refusal is the old engine's answer, and the test asserts the
  planks step's product *in the ingredient slots* before the stick step consumes it, the log's
  absence, and the two leftover planks.
- **Depth cap:** a ten-link chain with its leaf supplied is refused — a graph without a cap would
  craft it.
- **Cycle:** `a ← b ← a` is refused with the missing cell, not a hang.
- **Input full:** an intermediate whose product cannot fit blocks with `ingredient slots full` and
  the leaf log is still there.
- **Reservation:** a queued chain reserves the only log; a second request is refused with `have 0`.
- **Result handoff:** the mirror test shows a hopper's removal wins (7 left, then 0) — the policy
  whose absence duplicated the output.
- **Save compatibility:** a job compound in 0.2.1's shape reads as one step; a chain round-trips
  with its progress and its `step 1/2` readout.

## What this validation does not cover

- The **look** of the block: a headless/`--headless` run has no renderer, so the carried model and
  texture are proven as data (the state resolves, the model loads) but not visually. A windowed
  `--offscreen` + `ctl screenshot` run is the remaining check if the look becomes the point.
- Anything upstream does over the **network** (gaps 1 and 5): the interface is standalone here.
