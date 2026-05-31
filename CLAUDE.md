# CLAUDE.md — `cubr`

Interactive 3×3×3 Rubik's cube in **Rust + Bevy 0.18**, with a guaranteed-optimal solver and a local
HTTP control API. The cube engine, the 18 moves, the animation, the solver, and the HTTP API are built
and working; further work builds on them. See `README.md` for build/run and the HTTP API contract.

## The spec is the README — and it's binding
`README.md` is the source of truth for the **cube-state JSON contract, color scheme, coordinate
system, per-face read order, and HTTP semantics**. External API clients depend on these exactly. If
code and README disagree, the README wins. Don't change the contract casually — clients rely on it.

## How we work here: orchestrate, don't hand-code
**Default behaviour for any non-trivial task in this repo.** The main agent acts as an
**orchestrator**, not a coder: you hold the long-running context and keep the work marching toward the
goal, but you brief subagents to do the actual typing and verify their output against acceptance
gates. Don't write implementation code yourself beyond tiny glue/fixups when a subagent gets within a
line of done. Trivial one-liners and pure doc/chore edits are fine to do directly — reach for the
orchestration loop whenever the work is a real feature, phase, refactor, or non-obvious bug.

For each unit of work:

1. **Plan & freeze the contract first.** Decide the module/function interface and the **acceptance
   gate** — the exact commands that prove it works — *before* delegating. For larger efforts, write it
   down as a plan under `docs/plans/` (git-ignored). Keep `TodoWrite` current: one task per unit,
   `in_progress` → `completed` only on a green gate.
2. **Brief a subagent** (`Agent`, `subagent_type: general-purpose` unless a specialized one fits).
   Tell it: read `README.md` and `CLAUDE.md` first; implement only the listed files; don't modify
   files owned by another unit; match the agreed interface exactly — other modules depend on it; run
   the acceptance command itself and report the output before returning.
3. **Verify yourself — don't trust the report.** Re-run the gate commands (`cargo test`, `cargo clippy
   -- -D warnings`, `cargo build`, the GUI-screenshot + `curl` smoke checks). Read the changed files
   and confirm they match the contract and the repo conventions.
4. **If the gate fails,** send the *same* subagent the exact error via `SendMessage` (keeps its
   context) and have it fix. Re-verify. Only advance when green.
5. **Commit** after each green gate: `git add -A && git commit -m "<summary>"` — one commit per
   completed, green unit. Never let a unit land red: a failing test/clippy/build blocks the commit.

**Guardrails:**
- The interface is **frozen** once agreed. If a later unit truly needs to change it, stop, amend the
  written interface/plan first, then re-brief — don't let subagents silently diverge.
- **Parallelism:** units that touch disjoint files may run as concurrent `Agent` calls; anything
  sharing files runs sequentially.
- The pure `CubeCore`, the 18 absolute `Move`s, `MoveQueue`, the animation system, and the
  `POST /move` / `POST /state` JSON contract are the **frozen engine** the renderer and API clients
  depend on. New
  work is a presentation/input layer that ends at `MoveQueue` / `ApplyState`; don't reach into the
  engine or change the contract just to add a feature.

## Commands
```bash
cargo run -p cubr             # opens the cube window + starts the API on localhost:3000
cargo test                    # pure cube-core correctness (no rendering); covers BOTH crates
cargo clippy --workspace -- -D warnings   # lint gate — keep it clean
cargo fmt                     # format (defaults)
# quick API smoke test (app running):
curl -XPOST localhost:3000/move  -H 'Content-Type: application/json' -d '{"move":"R"}'
curl -XPOST localhost:3000/state -H 'Content-Type: application/json' -d @state.json
```
The repo is a Cargo **workspace** (virtual manifest at the root; members `crates/cubr-core` and
`crates/cubr`, no `default-members`). `cargo run` needs `-p cubr` to pick the binary; `cargo build`
/ `cargo clippy` take `--workspace`; a bare `cargo test` covers both crates' tests.

First cold build of Bevy is ~2–3 min; the cache is warm after that. `cargo run -p cubr` is a GUI app
— when verifying it from an agent, launch in the background, screenshot with `screencapture -x`, then
kill.

## Working efficiently in this repo (avoid permission stalls)
- **Read files with the `Read`/`Grep`/`Glob` tools — not Bash.** Don't shell out to `cat`, `head`,
  `tail`, `sed`, or `grep` just to view a file; those go through Bash and prompt for permission,
  interrupting the flow. The dedicated tools are pre-approved and give cleaner output.
- **Don't `cd` into the project.** The working directory is already this repo, and `cd <abs path>`
  in a compound command can itself trigger a permission prompt. Use absolute paths with the tools,
  or run Bash commands from the existing cwd.
- A pre-approved allowlist for the common safe commands here (cargo check/test/build/clippy/fmt/run,
  read-only git, search/read utils, `curl` to `localhost:3000`, `screencapture`) lives in
  `.claude/settings.json`. If a routine command still prompts, add it there rather than re-approving
  it every session.

## Architecture
Virtual Cargo workspace: `cubr-core` (pure library, no Bevy) + `cubr` (the Bevy binary, depends on it).
```
Cargo.toml         # [workspace]: members crates/cubr-core, crates/cubr; profiles live HERE (root-only)
crates/
├── cubr-core/     # LIBRARY — pure, no Bevy (glam, serde, kewb)
│   └── src/
│       ├── lib.rs   # pub mod core; pub mod model; pub mod solver;
│       ├── core.rs  # PURE integer-math cube (source of truth) — no Bevy, fully unit-tested (uses glam::IVec3)
│       ├── model.rs # StickerColor, Face, Move (parse/notation), CubeState (serde JSON shape) — no to_render_color
│       └── solver/  # guaranteed-optimal Korf solver: coords + ranking (coords.rs), corner+2 edge PDBs + cache (pdb.rs/cache.rs), IDA* (search.rs); CubeState -> optimal Vec<Move> (kewb kept only for facelet parse/validate). Public: solve, build_or_load_pdbs, Pdbs, SolveError
└── cubr/          # BINARY — the Bevy app
    └── src/
        ├── main.rs        # App + plugin wiring: CubePlugin, CameraPlugin, UiPlugin, ApiPlugin, MeshPickingPlugin, SwipePlugin, SolverPlugin
        ├── cube/
        │   ├── mod.rs     # CubePlugin + resources/messages; `pub use cubr_core::{core, model};` keeps crate::cube::{core,model}::* paths working
        │   ├── spawn.rs   # 26 cubie entities + sticker children; sync visuals <- core; render_color() free fn (StickerColor -> Bevy Color)
        │   └── animation.rs # MoveQueue consumer; ~0.25s smoothstep layer turns, one at a time
        ├── camera.rs      # re-basing turntable orbit (pole follows the tumble, smooth re-level, `L` levels) + wheel zoom; OrbitCamera::basis()
        ├── geom.rs        # small shared geometry helper (best_by_dot: nearest direction by dot product)
        ├── view_relative.rs # pure view-relative mapping: RelFace + relative_move(basis) -> Move, describe (inverse) + rel_label (Beginner wording)
        ├── swipe.rs       # mesh-picking swipe/flick — drag a visible layer to turn it -> MoveQueue
        ├── ui.rs          # native bevy_ui: Standard/Beginner scheme toggle, move grids + Reset -> MoveQueue
        ├── solve_ui.rs    # SolverPlugin: right Solve/Run panel; off-thread PDB build + off-thread solve (uses cubr_core::solver); scheme-aware step list -> MoveQueue
        └── api/           # tiny_http on its own thread + mpsc channel -> Bevy (non-blocking)
```
**Key invariant:** the pure `CubeCore` is the single source of truth (geometry *and* color); Bevy
entities mirror it. When no move is animating, transforms sit exactly on the integer grid / 90°
multiples. `POST /state` paints raw sticker colors, so physically impossible arrangements render
(the README requires this).

## Conventions
- Rust 2021, `cargo fmt` defaults, clippy clean under `-D warnings`.
- Lib stack is locked: native `bevy_ui` + `tiny_http` + serde. Don't swap to egui/axum.
- Commit per completed, green-gated unit of work.

## Tooling notes (this machine)
- Rust stable via rustup; `~/.cargo/bin` is on PATH via `.zprofile` (a fix was needed — the
  hard-coded PATH there was overriding `~/.cargo/env`). New terminals resolve `cargo` fine.
- System linker is Xcode Command Line Tools `clang` — verified working.
