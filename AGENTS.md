# AGENTS.md

Guidance for coding agents working in this repository. **Read [`CLAUDE.md`](CLAUDE.md) first**: it has the architecture (the `bevy_ecs` simulation behind the `World` facade, the one chained schedule and its real order, `SpawnId` ordering, the determinism rules and the map pin), the commands and toolchain, the rendering and level pipelines, the progression and content tables, and the testing strategy. [`STYLE.md`](STYLE.md) governs any player-visible string, and [`README.md`](README.md) / [`TERMINALS.md`](TERMINALS.md) are the player-facing and terminal-support docs.

## Rules for every agent

- **Do not compile or run.** No `cargo` (build, check, test, clippy, run, fmt), no `rustc`, no `rustfmt`, no game binaries, bot batches or capture harnesses. The coordinator merges the batch into the main checkout, compiles **once**, and fixes the breakage there. Write the code and the tests you would have run, and report exactly what is unverified.
- **Never edit the map pin** (`src/game/levelgen/mod.rs` `TRACE_HASHES`) to make a failure pass. A deliberate generator change moves it, in the same change, with the reason recorded in the module docs. (There are no scripted-trace goldens any more: `src/game/ecs/harness.rs` is deleted.)
- **Touch only the files your task names.** If you must add a line to a shared file (an enum variant, a match arm, a `mod` line, a table row), re-read it immediately before the edit, insert once at the end of the list, and never reformat, reorder or rewrite the surrounding code.
- **Commit on your own branch in your own worktree.** Exclude `.review*` files from commits. Never touch `main` or another worker's branch. Do not compile.
- **Gameplay and design decisions need no approval.** Make them and list them in your report.
- **Report, then stop.** Finish with what you built (file:line), the decisions you made, every file you touched, what is unverified or may not compile, the tests the coordinator must run, and the expected movement of the map pin.

## Spinning up workers

When spinning up workers, use `/orchestration` to split the work across parallel child agents. They must show up under the **same workspace in Orca** as the coordinator, so the owner can tab-tile them in one window.

Finished workers are cleared from Orca once they report, so a worker's lifetime is its dispatch: report when the task is done and do not start new work after it.
