# Validation — integratedcrafting 0.2.0

Every build, test and package step ran through `ports/tools/build-slot.sh` (`BUILD_LOCAL=1`,
`CARGO_INCREMENTAL=0`, `rm -f Cargo.lock` before each package build). Logs are in
`ports/_shared/evidence/integratedcrafting/`.

## The ladder

| # | Step | Command | Result | Evidence |
|---|---|---|---|---|
| 1 | engine only, red | `cargo test` with `src/lib.rs` = `pub mod runtime;` | 14 failed / 0 passed, each `not yet implemented` | `sdk-test-runtime-only-red.log` |
| 2 | engine only, green | the same, before any glue existed | 14 passed / 0 failed | `sdk-test-runtime-only.log` |
| 3 | whole crate | `cargo test` | **19 passed / 0 failed** (14 engine + 5 glue) | `sdk-test.log`, `ic-final-test.log` |
| 4 | lints | `cargo clippy --all-targets -- -D warnings` | exit 0, 0 warnings | `sdk-clippy.log`, `ic-final-clippy.log` |
| 5 | format | `cargo fmt --check` | exit 0 (one `cargo fmt` after the two layout nits in `sdk-fmt-first-run.log`) | `ic-final-fmt.log` |
| 6 | package | `cargo horizon-mod build` | `target/horizon-mod/integratedcrafting.hmod` | `sdk-build.log`, `ic-final-build.log` |
| 7 | reproducible | build twice, `sha256sum` + `cmp` | both `aab6b6fd…5da70`, `BYTE-IDENTICAL`; a later rebuild is `SAME-AS-DELIVERED` | `sdk-build-repro.log`, `ic-final-build.log` |
| 8 | registry check | `cargo horizon-mod check integratedcrafting.hmod` | exit 0 (text in `notes.md`) | `sdk-check.log` |
| 9 | fits | `cargo horizon-mod fits integratedcrafting.hmod --build nightly` | exit 0, `0b14f8a  made for` | `sdk-fits.log` |
| 10 | content loads | shared unmodded game at the pin + a content-only build of the package; `setblock`/`give` work, and the same run **without** the package answers `Unknown block type` | not vacuous | `content-load-transcript.txt`, `content-load-launch.log`, `content-load-control-launch.log` |
| 11 | native e2e | `tools/e2e.sh` | **not run** — needs `horizon-launcher`, absent here | (none, by design) |

Step 10 compiled nothing (published shared binaries + a content-only package), so it took no build
slot. It proves the **content** half; the **native** half (Parts, block entity, the engine in a
world) is step 11 and is unrun.

## Why the tests are fail-capable

The engine's failures are real output states, not assertions about themselves:

- **complete job**: two logs → eight planks, inputs empty, output slot holding 8, job gone;
- **missing ingredient**: refused at submission with `Missing 1 x minecraft:oak_planks (have 1)`;
- **over-subscription**: 100 planks from three logs → `needed 25, have 3`;
- **partial progress**: after *n* of 20 ticks the readout says `progress n/20` and **nothing** has
  been consumed (`assert_eq!` on the slots, not on the counter);
- **cancellation**: two jobs, `cancel`, ingredients still in the slots, `tick` returns nothing;
- **output cannot fit**: result slots full of another item → `blocked: output full`, and the log
  still holds its item;
- **unblock**: freeing the slot makes the same job finish on the next tick;
- **two cells, one item**: the accounting test fails if the engine checks cells independently;
- **readout**: two tests pin the exact strings, so a wording change fails the suite.

## What this validation does not cover

The in-game behaviour of the native half: the block entity ticking, the container's slots lining up
with `event.entity.items`, the hoppers, and `tools/e2e.sh`. `notes.md` names each as a risk, with
the one-line fix if the block entity turns out not to come from the declared type.
