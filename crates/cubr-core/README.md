# cubr-core

[![Crates.io](https://img.shields.io/crates/v/cubr-core.svg)](https://crates.io/crates/cubr-core)
[![Docs.rs](https://docs.rs/cubr-core/badge.svg)](https://docs.rs/cubr-core)

The pure, **Bevy-free** heart of [`cubr`](https://github.com/buzzlightyear1309/cubr): an
integer-math 3×3×3 Rubik's cube model and a **guaranteed-optimal** Korf IDA\* solver, with no
rendering dependency.

- **`core`** — [`CubeCore`], the single source of truth: 26 cubies as integer position/orientation
  vectors, the 18 face turns applied as exact integer permutations. No floating point.
- **`model`** — [`StickerColor`], [`Face`], [`Move`] (parse/notation), and [`CubeState`], the
  serde JSON shape (six faces × nine row-major stickers).
- **`solver`** — a **hybrid**. The primary engine is a Richard Korf style iterative-deepening
  A\* guided by three additive pattern databases (one over all eight corners, two over disjoint
  six-edge groups), combined as a max-of-three admissible heuristic; because the heuristic never
  overestimates, every solution it returns is a provably shortest move sequence (≤ 20 face
  turns — God's number). Korf runs under a wall-clock budget (~4 s); for the rare
  near-God's-number states that would exceed it, the solver falls back to a self-contained
  Kociemba two-phase search that returns a short, near-optimal solution in milliseconds, so a
  solve never stalls.

## Example

```rust
use cubr_core::core::CubeCore;
use cubr_core::model::Move;
use cubr_core::solver::{build_or_load_pdbs, solve};
use std::sync::atomic::AtomicBool;

// Scramble a solved cube.
let mut cube = CubeCore::solved();
for m in ["R", "U", "F'", "L2"] {
    cube.apply(Move::parse(m).unwrap());
}

// Build (or load the cached) pattern databases — ~85 MB, cached under
// ~/.cache/cubr/ on first run, then loaded in well under a second.
let pdbs = build_or_load_pdbs();
let cancel = AtomicBool::new(false);

// Solve via the hybrid: guaranteed-optimal Korf within the wall-clock budget,
// with a near-optimal two-phase fallback for the rare deep states. `cancel`
// (an AtomicBool) can abort a long search from another thread; an
// already-solved cube returns an empty Vec.
let solution = solve(&pdbs, &cube.to_state(), &cancel).unwrap();
println!("solution: {} moves", solution.len());
```

## Notes

- The half-turn metric (HTM): the 18 moves are the six faces × {90° CW, 90° CCW, 180°}.
- The color scheme and per-face read order are fixed by the
  [`cubr` README](https://github.com/buzzlightyear1309/cubr#cube-state-format) — white `U` up,
  green `F` front (standard Western / BOY).
- Facelet parsing and physical-solvability validation are now fully **in-house**; all the
  coordinate math, the pattern databases, and the search are local to this crate. `kewb` is kept
  only as a dev-only test oracle (a cross-check in the test suite), not a runtime dependency.

For the interactive 3D app (drag-to-turn, animated optimal solves, an HTTP control API) built on
top of this crate, see [`cubr`](https://github.com/buzzlightyear1309/cubr).

## License

[MIT](https://github.com/buzzlightyear1309/cubr/blob/main/LICENSE)
