# CLAUDE.md — `cube`

Interactive 3×3×3 Rubik's cube in **Rust + Bevy 0.18**, with a local HTTP control API. This is
**Stage 1** of a three-stage project (Stage 2: Python CV app feeds state via the API; Stage 3: Rust
solver). See `README.md` §intro for the staging.

## The spec is the README — and it's binding
`README.md` is the source of truth for the **cube-state JSON contract, color scheme, coordinate
system, per-face read order, and HTTP semantics**. Other stages depend on these exactly. If code and
README disagree, the README wins. Don't change the contract casually — downstream tools rely on it.

## Building Stage 1: read `IMPLEMENTATION_PLAN.md`
The build is structured as an **orchestrated, verification-gated** effort: the main agent acts as an
orchestrator that briefs subagents to write each module and then verifies against acceptance gates,
rather than writing code itself. `IMPLEMENTATION_PLAN.md` contains the frozen module interface (§1),
the per-phase subagent briefs, and the gates. Start there.

## Commands
```bash
cargo run                     # opens the cube window + starts the API on localhost:3000
cargo test                    # pure cube-core correctness (no rendering needed)
cargo clippy -- -D warnings   # lint gate — keep it clean
cargo fmt                     # format (defaults)
# quick API smoke test (app running):
curl -XPOST localhost:3000/move  -H 'Content-Type: application/json' -d '{"move":"R"}'
curl -XPOST localhost:3000/state -H 'Content-Type: application/json' -d @state.json
```
First cold build of Bevy is ~2–3 min; the cache is warm after that. `cargo run` is a GUI app — when
verifying it from an agent, launch in the background, screenshot with `screencapture -x`, then kill.

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

## Architecture (target)
```
src/
├── main.rs        # App + plugin wiring: CubePlugin, CameraPlugin, UiPlugin, ApiPlugin
├── cube/
│   ├── core.rs    # PURE integer-math cube (source of truth) — no Bevy, fully unit-tested
│   ├── model.rs   # StickerColor, Face, Move (parse/notation), CubeState (serde JSON shape)
│   ├── spawn.rs   # 26 cubie entities + sticker children; sync visuals <- core
│   └── animation.rs # MoveQueue consumer; ~0.25s smoothstep layer turns, one at a time
├── camera.rs      # orbit camera: left-drag (ignored over UI) + wheel zoom
├── ui.rs          # native bevy_ui: 18 move buttons -> MoveQueue
└── api/           # tiny_http on its own thread + mpsc channel -> Bevy (non-blocking)
```
**Key invariant:** the pure `CubeCore` is the single source of truth (geometry *and* color); Bevy
entities mirror it. When no move is animating, transforms sit exactly on the integer grid / 90°
multiples. `POST /state` paints raw sticker colors, so physically impossible arrangements render
(the README requires this).

## Conventions
- Rust 2021, `cargo fmt` defaults, clippy clean under `-D warnings`.
- Lib stack is locked: native `bevy_ui` + `tiny_http` + serde. Don't swap to egui/axum.
- Commit per completed, green-gated phase.

## Tooling notes (this machine)
- Rust stable via rustup; `~/.cargo/bin` is on PATH via `.zprofile` (a fix was needed — the
  hard-coded PATH there was overriding `~/.cargo/env`). New terminals resolve `cargo` fine.
- System linker is Xcode Command Line Tools `clang` — verified working.

## RustRover setup (optional, editor/debugger only — not required to build)
RustRover does **not** do the linking (the Rust toolchain + system `clang` do, already working). It
adds completion, navigation, inline Clippy, and an LLDB debugger. To wire it up:
1. **Open** the `cube` folder (File ▸ Open → select the dir with `Cargo.toml`); let it index.
2. **Toolchain:** Settings ▸ Languages & Frameworks ▸ Rust → confirm *Toolchain location* is
   `~/.cargo/bin` (auto-detected).
3. **Std navigation:** install std sources for go-to-definition into `std`:
   `rustup component add rust-src` (RustRover may also offer this via a prompt).
4. **Optional power:** Settings ▸ Rust ▸ External Linters → enable **Clippy** for richer on-the-fly
   warnings; enable *Rustfmt* on save. Create a **Cargo** run config for `run`/`test`; the bundled
   LLDB gives breakpoint debugging with no extra setup.
   (Optional, skippable: a faster linker like `lld` for quicker Bevy dev builds — not required.)
