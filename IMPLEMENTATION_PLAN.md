# Stage 1 Implementation Plan — Orchestrated Build

> **Audience:** the main agent in a *fresh* session. You are an **orchestrator**, not a coder.
> Your job is to (a) brief subagents with the per-phase specs below, (b) verify their output
> against the acceptance gates, and (c) keep the build marching toward the goal. **Do not write
> implementation code yourself** beyond tiny glue/fixups when a subagent gets within one line of
> done. Hold the long-running context; delegate the typing.

The authoritative product spec — colors, coordinate system, per-face read order, JSON contract,
HTTP semantics — lives in **`README.md`**. This plan never overrides it; if they ever disagree,
`README.md` wins and you fix the plan.

---

## 0. Orchestration protocol (read first)

For every phase:

1. **Brief a subagent** (`Agent`, `subagent_type: general-purpose` unless noted). Paste the phase's
   *Subagent brief* verbatim, plus: "Read `README.md` and `IMPLEMENTATION_PLAN.md` first. Implement
   only the files listed. Do not modify files owned by other phases. Match the frozen interface in
   §1 exactly — other modules depend on it. Run the listed acceptance command yourself and report
   the output before returning."
2. **Verify** when it returns: run the phase's *Gate* commands yourself. Don't trust the report —
   re-run. Read the changed files and confirm they match the §1 contract and the repo conventions.
3. **If the gate fails:** send the subagent the exact error via `SendMessage` (same agent, keeps
   context) and have it fix. Re-verify. Only advance when green.
4. **Commit** after each green gate: `git add -A && git commit -m "<phase>: <summary>"`.

**Guardrails for you, the orchestrator:**
- Keep `TodoWrite`/tasks current: one task per phase, `in_progress` → `completed` on green gate.
- Never let a phase land red. A failing `cargo test`/`clippy`/`build` blocks the commit.
- The §1 interface is **frozen after Phase 1**. If a later phase truly needs an interface change,
  stop, change §1 here, then re-brief — don't let subagents silently diverge.
- Parallelism: phases marked *(parallel-safe)* touch disjoint files and may run as concurrent
  `Agent` calls. Everything else is sequential.

---

## 1. Frozen module interface (the contract)

This is what every module agrees on. Phase 1 implements it; later phases consume it unchanged.
Signatures are the contract; bodies are the subagent's to write.

### `src/cube/model.rs` — colors, faces, moves, JSON shape

```rust
use bevy::prelude::*;          // for IVec3, Color
use serde::{Serialize, Deserialize};

/// Sticker color. Serializes to its single-letter name ("W","Y","R","O","B","G").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum StickerColor { W, Y, R, O, B, G }

impl StickerColor {
    /// Render color (sRGB). White/Yellow/Red/Orange/Blue/Green tuned for a clean look.
    pub fn to_render_color(self) -> Color;
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Face { U, D, L, R, F, B }

impl Face {
    pub const ALL: [Face; 6] = [Face::U, Face::D, Face::L, Face::R, Face::F, Face::B];
    /// Outward normal in world space: U=+Y, D=-Y, R=+X, L=-X, F=+Z, B=-Z  (see README coords).
    pub fn normal(self) -> IVec3;
    /// Solved color: U=W, D=Y, F=G, B=B, R=R, L=O  (see README face table).
    pub fn solved_color(self) -> StickerColor;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Turn { Cw, Ccw, Double }   // (none), ', 2

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Move { pub face: Face, pub turn: Turn }

impl Move {
    /// All 18 standard moves, in README order: U U' U2 D D' D2 L L' L2 R R' R2 F F' F2 B B' B2.
    pub const ALL: [Move; 18];
    /// Parse one of the 18 notation strings; None for anything else.
    pub fn parse(s: &str) -> Option<Move>;
    /// Notation string, e.g. "R", "R'", "R2".
    pub fn notation(self) -> String;
    /// Rotation axis = self.face.normal().
    pub fn axis(self) -> IVec3;
    /// Number of clockwise (looking at the face from outside) quarter-turns: Cw=1, Ccw=3, Double=2.
    pub fn quarter_turns_cw(self) -> u8;
}

/// The exact JSON shape of POST /state (see README "Cube-state JSON contract").
/// Field order is irrelevant to serde; keys are U R F D L B.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct CubeState {
    pub U: [StickerColor; 9],
    pub R: [StickerColor; 9],
    pub F: [StickerColor; 9],
    pub D: [StickerColor; 9],
    pub L: [StickerColor; 9],
    pub B: [StickerColor; 9],
}

impl CubeState {
    pub fn solved() -> Self;            // all 9 of each face = that face's solved color
    pub fn face(&self, f: Face) -> &[StickerColor; 9];
    /// Non-fatal sanity check per README "Validation notes": 6 faces × 9, each color ×9.
    /// Returns warnings; never rejects (impossible states are allowed).
    pub fn sanity_warnings(&self) -> Vec<String>;
}
```

### `src/cube/core.rs` — the pure, render-free source of truth

Plain Rust + glam integer math. **No Bevy systems, no entities.** Single source of truth for both
geometry (where each cubie is / how it's turned) and color (what each sticker shows — including
physically impossible paint jobs, which `POST /state` must allow).

```rust
use bevy::math::IVec3;
use crate::cube::model::{CubeState, Move, StickerColor, Face};

/// 26 cubies (the hidden core at (0,0,0) is omitted). Integer rotation math only.
pub struct CubeCore { /* private: Vec<CoreCubie>, 26 entries */ }

impl CubeCore {
    pub fn solved() -> Self;

    /// Apply a move as an integer permutation+reorientation of the affected layer.
    /// Quarter turns applied quarter_turns_cw times. Pure geometry — colors ride along.
    pub fn apply(&mut self, m: Move);

    /// Repaint to an arbitrary state for POST /state: reset all cubies to home pose,
    /// then assign each visible sticker the given color. Represents impossible states fine.
    pub fn paint(&mut self, state: &CubeState);

    /// Read the current facelets in README per-face orientation (row-major, the index
    /// layout and per-face viewing rules in README "Per-face viewing orientation").
    pub fn to_state(&self) -> CubeState;

    /// For the renderer: snapshot of each cubie's current pose + visible stickers, so the
    /// Bevy layer can build/sync entities. `home` identifies the entity across moves.
    pub fn cubies(&self) -> &[CoreCubie];
    /// Indices into cubies() that lie in the layer this move turns (the 9 moving pieces).
    pub fn layer(&self, m: Move) -> Vec<usize>;
}

/// Read-only view the renderer consumes.
pub struct CoreCubie {
    pub home: IVec3,           // solved position; stable id for the entity
    pub pos: IVec3,            // current grid position, components in {-1,0,1}
    pub orient: [IVec3; 3],    // integer rotation matrix columns (local->world basis)
    // visible stickers: which outward local face shows which color
    pub stickers: Vec<(IVec3 /*local outward normal*/, StickerColor)>,
}
```

> **Why this shape:** geometry is integer-exact (no float drift, trivially testable), and because
> `stickers` are stored data rather than derived from `Face::solved_color`, `paint()` can show any
> arrangement the README requires. The Bevy layer is a pure mirror of `CubeCore`.

### Bevy-side resources / components / events (defined in their owning module, listed here so all phases agree)

```rust
// cube/mod.rs
#[derive(Resource)] pub struct Cube(pub CubeCore);              // the live core
#[derive(Resource, Default)] pub struct MoveQueue(pub std::collections::VecDeque<Move>);
#[derive(Event)] pub struct ApplyState(pub CubeState);          // request an instant repaint
#[derive(Event)] pub struct CoreChanged;                        // core mutated -> sync visuals

// cube/spawn.rs
#[derive(Component)] pub struct Cubie { pub home: IVec3 }       // links entity <-> core cubie
#[derive(Component)] pub struct Sticker { pub local_normal: IVec3 }
#[derive(Resource)] pub struct CubeMaterials { /* per-color + body handles */ }

// cube/animation.rs
#[derive(Resource, Default)] pub struct ActiveMove(pub Option<MoveAnim>); // None = idle
```

**Invariant enforced across phases:** whenever `ActiveMove` is `None`, the entity transforms and
sticker materials are *exactly* what `Cube(CubeCore)` says (positions on the integer grid, rotations
at exact multiples of 90°). Animation is the only thing allowed to show in-between poses, and it must
end by restoring this invariant.

### Plugin wiring (`src/main.rs`)

```rust
fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(/* window title "Cube", reasonable size */))
        .add_plugins((CubePlugin, CameraPlugin, UiPlugin, ApiPlugin))
        .run();
}
```
`CubePlugin` owns the cube resources/events + spawn + animation + the sync system. The others are
self-contained.

---

## 2. Phases

Each phase: **Owns** (files it may write) · **Subagent brief** · **Gate** (you verify).

### Phase 0 — Skeleton & frozen contract  *(sequential, do first)*
- **Owns:** all module files as *compiling stubs* + `main.rs` wiring.
  `src/main.rs`, `src/cube/{mod,model,core,spawn,animation}.rs`, `src/camera.rs`, `src/ui.rs`,
  `src/api/{mod,server,types}.rs`.
- **Subagent brief:** Create every file with the full §1 type/function **signatures**, the
  resources/components/events, and the four plugins (`CubePlugin`, `CameraPlugin`, `UiPlugin`,
  `ApiPlugin`) registered and empty/stubbed. Bodies may be `todo!()` or trivial no-ops, **but the
  crate must `cargo check` clean** (so stub functions that must return values return sensible
  defaults rather than `todo!()` where needed to compile). No behavior yet. This freezes the
  interface in §1.
- **Gate:** `cargo check` exits 0. You read every file and confirm signatures match §1 verbatim.

### Phase 1 — Pure cube core + model  *(sequential; the correctness keystone)*
- **Owns:** `src/cube/model.rs`, `src/cube/core.rs` (+ `#[cfg(test)]` tests in both).
- **Subagent brief:** Implement `model.rs` and `core.rs` fully per §1. Integer rotation matrices for
  ±90° about each axis; `apply` permutes the layer's cubies' `pos` and `orient`; `to_state` reads
  facelets in the **exact README per-face orientation** (mind the mirrored `B` face and the
  back→front / front→back row directions in the README table). Add unit tests that MUST pass:
  1. `CubeCore::solved().to_state() == CubeState::solved()` and equals the README solved example.
  2. **Direction anchor (pins clockwise):** from solved, after `U`, the Front top row is Red
     (`state.F[0..3] == [R,R,R]`) and the Left top row is Green (`state.L[0..3] == [G,G,G]`).
     *(U clockwise sends Front→Left, so Front receives Right=Red, Left receives Front=Green.)*
  3. Every quarter move has order 4 (`apply` ×4 == solved); every `2` move has order 2.
  4. `X` then `X'` == solved, for all six faces.
  5. Sexy move: `(R U R' U')` ×6 == solved.
  6. `paint` round-trip: `paint(s); to_state() == s` for the solved state **and** for a deliberately
     impossible state (e.g. all 54 = `W`) — proves impossible states are representable.
  7. `Move::parse`/`notation` round-trip over all 18; `parse` rejects junk; `CubeState` serde
     round-trips and the solved JSON matches the README example byte-for-key.
- **Gate:** `cargo test` exits 0 with all the above present and passing. You read the tests and
  confirm they actually assert items 1–7 (not weakened). **After green, §1 is frozen.**

### Phase 2 — Spawn cubies + materials + sync  *(sequential; needs Phase 1)*
- **Owns:** `src/cube/spawn.rs`, the sync system + plugin assembly in `src/cube/mod.rs`.
- **Subagent brief:** On startup, insert `Cube(CubeCore::solved())`, build `CubeMaterials` (one
  shared `StandardMaterial` per `StickerColor` + a dark "body" material), and spawn 26 cubie
  entities from `Cube.0.cubies()`: each a small rounded-ish cube (~0.95 unit) at `home`, with child
  `Sticker` quads on each visible local normal, colored from the core. Add a **sync system** that,
  on `CoreChanged` (and once at startup), sets every cubie entity's `Transform` from `pos`+`orient`
  and every sticker's material from the core color. Add a basic directional/ambient light. Keep the
  body plugin wiring (`CubePlugin`) here.
- **Gate:** `cargo run`, wait ~4s, `screencapture -x /tmp/cube_p2.png`, then `Read /tmp/cube_p2.png`
  and confirm a solved 3×3 cube is visible with correct face colors (white-ish up, green front per
  default camera). `cargo clippy -- -D warnings` clean. (Kill the run after.)

### Phase 3 — Orbit camera  *(parallel-safe with Phase 5)*
- **Owns:** `src/camera.rs`.
- **Subagent brief:** `CameraPlugin` spawns a `Camera3d` positioned by spherical coords
  (yaw, pitch, radius) looking at the origin, default angled so white-up/green-front shows. On
  left-mouse **drag**, update yaw/pitch from mouse motion (clamp pitch to avoid flipping); mouse
  wheel adjusts radius (clamped). **Ignore drags that begin while the pointer is over a UI node** so
  button clicks don't spin the camera (read Bevy UI `Interaction`, or skip when pointer is within
  the panel rect). Recompute the camera transform each frame.
- **Gate:** `cargo run`; visually confirm via screenshot that the cube renders; manual drag check is
  noted for the user. `cargo clippy -- -D warnings` clean.

### Phase 4 — Move animation + queue  *(sequential; needs Phase 2)*
- **Owns:** `src/cube/animation.rs` (+ its registration in `CubePlugin`).
- **Subagent brief:** Implement the consumer of `MoveQueue` and `ActiveMove`. When idle and the
  queue is non-empty: pop a `Move`, snapshot the layer entities (`Cube.0.layer(m)`), apply the move
  to the core immediately, emit nothing yet, and animate the *visual* from the pre-move pose to the
  new core pose over ~0.25s with smoothstep easing by rotating those cubies about `m.axis()` through
  the origin (`Transform::rotate_around`). On completion, fire `CoreChanged` so the sync system
  snaps entities exactly onto the (already-applied) core — restoring the §1 invariant. Exactly one
  move animates at a time; the rest wait in the queue. Also handle the `ApplyState` event: set
  `Cube.0.paint(state)`, clear any queue/active move, fire `CoreChanged` (instant, no animation).
  Add a temporary debug keybind (e.g. press `R` key → push `Move::parse("R")`) **only** to self-test,
  and remove or leave clearly behind a comment for Phase 5/6 to supersede.
- **Gate:** `cargo run`; trigger a move (debug key or by pushing to the queue); screenshot mid- and
  post-move and confirm a layer turned and the cube remains coherent (no torn geometry, snaps to
  grid). Re-run the Phase-1 `cargo test` to confirm core untouched. `cargo clippy -- -D warnings`.

### Phase 5 — UI move panel  *(parallel-safe with Phase 3)*
- **Owns:** `src/ui.rs`.
- **Subagent brief:** `UiPlugin` builds a native `bevy_ui` panel docked on one side: 18 buttons laid
  out as 6 rows (U D L R F B), each row `[X] [X'] [X2]`, labeled with the notation. Style for a
  clean game look (subtle panel bg, hover/press color feedback, readable font via the default).
  On press, push the corresponding `Move` onto `MoveQueue` (same queue Phase 4 drains). Buttons must
  register as UI `Interaction` so Phase 3's camera correctly ignores those clicks.
- **Gate:** `cargo run`; screenshot shows the labeled panel; clicking a button animates that move
  (visually confirm via screenshot after a click, or note manual check). `cargo clippy -D warnings`.

### Phase 6 — HTTP API  *(sequential; needs Phases 1 & 4)*
- **Owns:** `src/api/{mod,server,types}.rs`.
- **Subagent brief:** `ApiPlugin` spawns a `tiny_http` server on `127.0.0.1:3000` on a dedicated
  `std::thread` at startup. Communicate to Bevy via an `mpsc` channel: the server thread holds the
  `Sender`, Bevy holds the `Receiver` in a resource (drained each frame by a system). Endpoints:
  - `POST /move` `{"move":"R"}` → validate with `Move::parse` **on the server thread**; `400` +
    error text for an unknown move; otherwise send `Cmd::Move(mv)` and reply `200`.
  - `POST /state` `<CubeState JSON>` → `serde_json::from_str::<CubeState>` on the server thread;
    `400` + error on malformed body; otherwise send `Cmd::SetState(state)` and reply `200`.
    (Optionally include `sanity_warnings()` in the 200 body; never reject on them.)
  The drain system maps `Cmd::Move` → `MoveQueue.push_back`, `Cmd::SetState` → emit `ApplyState`.
  Validation on the server thread means no cross-thread response round-trip; the render loop never
  blocks. Define request types in `types.rs`.
- **Gate:** with `cargo run` live: `curl -s -o /dev/null -w "%{http_code}" -XPOST
  localhost:3000/move -d '{"move":"R"}'` → `200` and the cube animates; a bad move → `400`;
  `POST /state` with the README solved JSON → `200` and cube resets to solved instantly; malformed
  body → `400`. Screenshot to confirm visual effect. `cargo clippy -- -D warnings` clean.

### Phase 7 — Docs, polish, final review  *(sequential, last)*
- **Owns:** `README.md` ("Build & run" — drop "Not yet implemented", confirm `cargo run` opens the
  window + starts the API on :3000; verify the curl example works as written). Any small fixups.
- **Orchestrator actions (do these yourself / via review subagents):**
  - `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` all green.
  - Launch **3 `feature-dev:code-reviewer` subagents in parallel** (simplicity/DRY · bugs/correctness
    · conventions/architecture). Consolidate findings; fix high-severity ones (delegate fixes to a
    subagent, you verify).
  - Final manual smoke test (screenshot + a couple of curl calls). Confirm the README JSON contract
    matches actual `/state` behavior end-to-end.
- **Gate:** all checks green; README accurate; reviewers' high-severity issues resolved.

---

## 3. Your verification toolkit (commands you run)

```bash
cargo check                      # fast compile gate (Phase 0)
cargo test                       # core correctness (Phase 1; re-run after 4 & 6)
cargo clippy -- -D warnings      # lint gate (every phase after 1)
cargo fmt --check                # format gate (Phase 7)
cargo run                        # launch app (visual gates) — run in background, kill after
screencapture -x /tmp/cube.png   # macOS screenshot; then Read /tmp/cube.png to inspect
# API gates (app must be running):
curl -s -o /dev/null -w "%{http_code}" -XPOST localhost:3000/move  -d '{"move":"R"}'
curl -s -o /dev/null -w "%{http_code}" -XPOST localhost:3000/state -d @/tmp/solved.json
```
Notes: `cargo run` is GUI; launch it with `run_in_background: true`, sleep a few seconds, screenshot,
then kill the background shell. The first build is ~2–3 min cold; the cache is already warm.

---

## 4. Decisions already locked (don't re-litigate)

- **Lib stack:** native `bevy_ui` (panel) + `tiny_http` (API). Versions pinned in `Cargo.toml`:
  bevy 0.18, tiny_http 0.12, serde/serde_json 1.
- **Source of truth:** the pure `CubeCore` (integer math); Bevy entities mirror it. `/state` paints
  raw colors so impossible arrangements render.
- **Animation:** ~0.25s, smoothstep, one move at a time, queue-serialized.
- **Camera:** left-drag orbit (ignored over UI), wheel zoom.
- **Conventions:** Rust 2021, `cargo fmt` defaults, clippy-clean (`-D warnings`), module layout per
  README "Planned project structure". Commit per green phase.

### Decided in the setup session (don't re-ask)
- **Run mode: fully autonomous.** Execute phases 0→7 end-to-end without pausing for review. Stop
  only if a gate genuinely fails (and can't be fixed by re-briefing the subagent) or a real,
  unforeseen ambiguity arises that the README/plan doesn't answer. Report at the end with a summary
  and the visual-gate screenshots.
- **Commits: auto-commit each green phase.** After a phase passes its gate, `git add -A &&
  git commit -m "<phase>: <summary>"`. Work proceeds on `main` (the repo's existing branch); no
  feature branch. Use the standard Co-Authored-By trailer.
- **Allowlist:** `.claude/settings.json` already permits `git add`/`git commit` (added this session)
  alongside the cargo/curl/screencapture commands, so the autonomous run shouldn't stall on prompts.

### Environment note (toolchain PATH — already fixed)
- `cargo` lives at `~/.cargo/bin/cargo` (rustup; verified `cargo 1.96.0`). A stale `~/.zprofile`
  previously **discarded** `$PATH` via a hardcoded Python-installer line, wiping `~/.cargo/bin`. That
  line was fixed to append `:${PATH}` (with a load-bearing comment), so **freshly launched** login
  shells resolve `cargo` in both login and interactive modes. The fix only affects shells started
  *after* it — if `cargo` is "command not found" in your session, the Claude process was launched
  from a pre-fix shell; fully quit and relaunch the terminal app + Claude, then `cargo --version`
  should work. No `settings.json` `env`/PATH hack is needed.

## 5. Open choices a subagent may make (low-stakes, just be consistent)
- Exact sRGB values for the six colors (pick clean, slightly desaturated; orange clearly ≠ red/yellow).
- Cubie size/gap, sticker inset, light intensity, panel side (left or bottom) and exact styling.
- Easing curve specifics and default camera yaw/pitch/radius.
