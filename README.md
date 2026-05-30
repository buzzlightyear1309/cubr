# Cube

A Rubik's cube project in three stages:

1. **Cube game (this repo, Stage 1)** — a Rust + [Bevy](https://bevyengine.org/) app that renders an interactive 3×3×3 cube, animates moves, and exposes a small local HTTP API for external control.
2. **Python CV app (Stage 2)** — a computer-vision app that reads a physical cube and feeds its state into this game via the HTTP API.
3. **Solver (Stage 3)** — a Rust solver, integrated into the workflow to produce move sequences for a given state.

This README documents Stage 1 and, critically, the **cube-state JSON contract** that Stages 2 and 3 rely on. Stage 1 is built and working; the sections below are both its spec and the binding contract the later stages depend on.

---

## Features

- Fully rendered 3×3×3 cube in 3D with correct face colors.
- Orbit camera: rotate the view by left-dragging, zoom with the mouse wheel.
- All 18 standard moves: `U U' U2  D D' D2  L L' L2  R R' R2  F F' F2  B B' B2`.
- Smooth animation when a move is applied.
- A UI panel with a button for each move.
- A lightweight HTTP API on `localhost:3000`:
  - `POST /move` — apply a single animated move.
  - `POST /state` — set the entire cube to an arbitrary state instantly (no animation).

---

## Conventions

These conventions are the source of truth. The renderer, the move engine, the HTTP API, and any external client (Python CV app, solver) must all agree on them.

### Faces

The six faces use standard single-letter names:

| Letter | Face   | Solved color   |
|--------|--------|----------------|
| `U`    | Up     | White  (`W`)   |
| `D`    | Down   | Yellow (`Y`)   |
| `F`    | Front  | Green  (`G`)   |
| `B`    | Back   | Blue   (`B`)   |
| `R`    | Right  | Red    (`R`)   |
| `L`    | Left   | Orange (`O`)   |

This is the standard Western / BOY scheme: **white up, green front**. With white on top and green facing you, red is on the right, orange on the left, blue at the back, yellow on the bottom.

### Colors

Stickers are described with single-letter color codes, independent of face position (so a scrambled cube can be expressed):

| Code | Color  |
|------|--------|
| `W`  | White  |
| `Y`  | Yellow |
| `R`  | Red    |
| `O`  | Orange |
| `B`  | Blue   |
| `G`  | Green  |

### Coordinate system (for the implementation)

Right-handed axes, cube centered at the origin, each cubie one unit:

- `+X` → Right (`R` face), `-X` → Left (`L` face)
- `+Y` → Up (`U` face),    `-Y` → Down (`D` face)
- `+Z` → Front (`F` face), `-Z` → Back (`B` face)

A clockwise face turn (`U`, `R`, `F`, …) is clockwise **as seen looking directly at that face from outside the cube**. A `'` (prime) is counter-clockwise; a `2` suffix is a 180° turn.

---

## Cube-state JSON contract

This is the exact shape used by `POST /state`. It is also the canonical representation a solver or CV client should produce/consume.

A cube state is a JSON object with one key per face. Each value is an array of **9 color codes** in **row-major order** (top-left → top-right, then down row by row) as the face is viewed in its **standard orientation**, defined below.

### Per-face viewing orientation

To make "row-major" unambiguous, each face is read while holding the cube in the standard orientation (white `U` on top, green `F` toward you). For each face, index `0` is the top-left sticker and index `8` is the bottom-right sticker, where "top" and "left" are:

| Face | Read while looking at it from… | Index 0 (top-left) is toward… | Rows go… | Columns go… |
|------|--------------------------------|-------------------------------|----------|-------------|
| `U`  | above (`+Y`, looking down)     | back-left  (`-X`, `-Z`)       | back → front | left → right |
| `D`  | below (`-Y`, looking up)       | front-left (`-X`, `+Z`)       | front → back | left → right |
| `F`  | the front (`+Z`)               | top-left   (`+Y`, `-X`)       | top → bottom | left → right |
| `B`  | the back (`-Z`)                | top-right* (`+Y`, `+X`)       | top → bottom | right → left |
| `R`  | the right (`+X`)               | top-front  (`+Y`, `+Z`)       | top → bottom | front → back |
| `L`  | the left (`-X`)                | top-back   (`+Y`, `-Z`)       | top → bottom | back → front |

\* `B` is read as if you walked around to look at it head-on, so its left/right are mirrored relative to `F`. This matches the common Kociemba-style facelet layout, which the Stage 3 solver expects.

Index layout within every face:

```
0 1 2
3 4 5
6 7 8
```

Index `4` is always the center sticker, which fixes that face's solved color.

### Solved-cube example

```json
{
  "U": ["W","W","W","W","W","W","W","W","W"],
  "R": ["R","R","R","R","R","R","R","R","R"],
  "F": ["G","G","G","G","G","G","G","G","G"],
  "D": ["Y","Y","Y","Y","Y","Y","Y","Y","Y"],
  "L": ["O","O","O","O","O","O","O","O","O"],
  "B": ["B","B","B","B","B","B","B","B","B"]
}
```

### Validation notes (for `POST /state`)

The endpoint sets the displayed stickers exactly as given, so a client can show any arrangement — including physically impossible ones — without the app rejecting it. Recommended (non-fatal) sanity checks the implementation may surface as warnings:

- Exactly 6 face keys (`U D F B R L`), each with exactly 9 entries.
- Each entry is one of `W Y R O B G`.
- Each color appears exactly 9 times across all 54 stickers.

---

## HTTP API

Server listens on `http://localhost:3000`. JSON in, JSON out.

### `POST /move`

Apply a single move, animated.

Request body:

```json
{ "move": "R" }
```

`move` is one of the 18 move strings: `U U' U2 D D' D2 L L' L2 R R' R2 F F' F2 B B' B2`.

Response: `200 OK` on a valid move; `400` with an error message for an unknown move string.

### `POST /state`

Set the full cube state instantly, no animation. Used by the CV app to mirror a physical cube.

Request body: the [cube-state JSON](#cube-state-json-contract) object above.

Response: `200 OK` on success; `400` with an error message if the body is malformed.

---

## Move notation

| Suffix | Meaning                        |
|--------|--------------------------------|
| (none) | 90° clockwise (facing the face)|
| `'`    | 90° counter-clockwise          |
| `2`    | 180°                           |

Faces: `U` (up), `D` (down), `L` (left), `R` (right), `F` (front), `B` (back).

---

## Project structure

Stage 1 is laid out as:

```
cube/
├── Cargo.toml
├── README.md
└── src/
    ├── main.rs          # Bevy app + plugin wiring
    ├── cube/            # cube model, move engine, spawning, animation
    ├── camera.rs        # orbit camera controls
    ├── ui.rs            # move-button panel
    └── api/             # HTTP server + request/response types + Bevy bridge
```

The HTTP server runs on its own thread and hands commands to Bevy over a channel, so the render loop stays non-blocking.

---

## Build & run

```bash
cargo run
```

This opens the cube window and starts the HTTP API on `localhost:3000`. In the window you can orbit the view by left-dragging, zoom with the mouse wheel, and apply any move with the on-screen button panel.

Apply a move from the command line (animated):

```bash
curl -X POST localhost:3000/move -H 'Content-Type: application/json' -d '{"move":"R"}'
```

Set the entire cube to an arbitrary state instantly (no animation) — the body is the [cube-state JSON](#cube-state-json-contract):

```bash
curl -X POST localhost:3000/state -H 'Content-Type: application/json' -d @state.json
```

Run the cube-core test suite (no rendering needed):

```bash
cargo test
```
