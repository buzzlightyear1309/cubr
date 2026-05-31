# cubr

**An interactive 3×3×3 Rubik's cube in Rust + [Bevy](https://bevyengine.org/)** — drag to turn the
layers, solve it in the fewest possible moves, and drive it over a small local HTTP API.

![cubr — an optimal solve, animated](https://raw.githubusercontent.com/buzzlightyear1309/cubr/main/docs/demo.gif)

`cubr` renders a fully interactive 3×3×3 cube, animates all 18 face turns, computes the
**guaranteed-optimal** (fewest-move) solution for any reachable state — Korf's IDA\* with pattern
databases, every solution ≤ 20 turns — and exposes `POST /move` / `POST /state` on `localhost:3000`.

The pure, Bevy-free cube model + solver live in the separate
[`cubr-core`](https://crates.io/crates/cubr-core) crate.

## Install & run

Prebuilt binaries for **macOS / Linux / Windows** are on the
[Releases page](https://github.com/buzzlightyear1309/cubr/releases) — download, unpack, run `cubr`.
This is the recommended route (no toolchain needed).

Or build from source with cargo (compiles Bevy, takes a few minutes):

```bash
cargo install cubr
cubr
```

Either way this opens the cube window and starts the HTTP API on `localhost:3000`. On first launch
the solver builds its pattern databases (~85 MB) once and caches them under `~/.cache/cubr/`;
later launches load them in well under a second.

```bash
# apply a move while the app is running:
curl -X POST localhost:3000/move -H 'Content-Type: application/json' -d '{"move":"R"}'
```

See the [project README](https://github.com/buzzlightyear1309/cubr#readme) for the controls, the
solving details, and the full HTTP API + cube-state JSON contract.

## License

[MIT](https://github.com/buzzlightyear1309/cubr/blob/main/LICENSE)
