//! Hybrid Rubik's-cube solver: guaranteed-optimal Korf IDA* under a wall-clock budget,
//! with a near-optimal Kociemba two-phase fallback for the deep tail.
//!
//! No Bevy types or systems live here — this mirrors [`crate::core`] in being a pure,
//! fully unit-tested module. The Bevy layer (`solve_ui`, in the `cubr` binary)
//! loads/generates the
//! [`Pdbs`] off-thread and feeds states through [`solve`].
//!
//! ## The hybrid
//! Both public entry points ([`solve`] and [`Solver::solve`]) delegate to [`run_hybrid`],
//! which runs the guaranteed-optimal Korf IDA* ([`search::search`]) with a wall-clock
//! budget (default ~4 s, [`DEFAULT_KORF_BUDGET_MS`]). If Korf finishes within budget the
//! answer is the exact optimum; if the budget expires (the rare near-God's-number states),
//! it falls back to the near-optimal in-house two-phase ([`two_phase::solve`]) from the same
//! state, so even distance-17–20 cubes return a solution in milliseconds. An external cancel
//! is honoured throughout (-> [`SolveError::Cancelled`]).
//!
//! ## The facelet boundary
//! Parse/validate is now fully in-house. A [`CubeState`] is concatenated into a 54-char
//! URFDLB facelet string (via [`color_to_facelet_char`] / [`state_to_facelets`]) and that
//! string is converted straight to our [`coords::Cubies`] integer arrays — and validated
//! for physical solvability — by [`facelet::facelets_to_cubies`] (a port of kewb's
//! `cube/{facelet,cubie}.rs`; wrong color counts / bad parity become
//! [`SolveError::Unsolvable`]). All coordinate math, the pattern databases, and the search
//! run on our own arrays. `kewb` is no longer a runtime dependency — it is kept only as a
//! dev-dependency test oracle (the `facelet_conversion_matches_kewb` cross-check below and
//! the `compose_matches_kewb_*` guards in `coords`).
//!
//! The facelet alphabet is face *letters* `U R F D L B` laid out in face order
//! **U, R, F, D, L, B**, 9 facelets each, row-major. Our [`CubeState`] uses the same face
//! order (its struct fields) and the README per-face read order is already the
//! Kociemba-style layout (mirrored `B`), so the conversion is a straight concatenation
//! once each sticker color is mapped to the face letter of the face whose solved center is
//! that color (`W->U, R->R, G->F, Y->D, O->L, B->B`).

use crate::model::{CubeState, Face, Move, StickerColor};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

mod cache;
mod coords;
mod facelet;
mod pdb;
mod search;
mod two_phase;

pub use pdb::Pdbs;
use pdb::SearchTables;

/// Error from solving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolveError {
    /// The state is physically impossible / unparseable (wrong color counts, bad
    /// permutation/orientation parity, etc.).
    Unsolvable,
    /// The search was aborted via the cancel flag (e.g. a repaint superseded it).
    Cancelled,
}

/// Load the cached Korf PDBs, or generate them (SLOW, ~30–60 s on first run) and cache
/// them to disk. Pure (no Bevy); the caller runs this once, off-thread.
pub fn build_or_load_pdbs() -> Pdbs {
    let path = cache::cache_path();
    if let Some(p) = cache::load(&path) {
        return p;
    }
    let p = Pdbs::generate();
    let _ = cache::save(&path, &p); // best-effort cache; ignore write errors
    p
}

/// Solve `state` via the hybrid. Validates the state via the in-house facelet converter (an
/// impossible cube returns [`SolveError::Unsolvable`]), converts it to our coordinate
/// arrays, then runs the **hybrid** ([`run_hybrid`]): guaranteed-optimal Korf IDA* under a
/// ~4 s wall-clock budget, with a near-optimal two-phase fallback if that budget expires.
/// `cancel` aborts the search (-> [`SolveError::Cancelled`]). An already-solved state returns
/// `Ok(vec![])`.
///
/// This convenience entry point builds **both** in-memory table sets *per call* — the Korf
/// [`SearchTables`] (~0.3–0.6 s + ~62 MB) and the [`two_phase::TwoPhaseTables`]. For repeated
/// solves (the GUI's one-shot Solve button plus live re-solves) prefer [`Solver`], which
/// builds the tables once and reuses them.
pub fn solve(pdbs: &Pdbs, state: &CubeState, cancel: &AtomicBool) -> Result<Vec<Move>, SolveError> {
    let cubies = state_to_cubies(state)?;
    let tables = SearchTables::build();
    let tp = two_phase::TwoPhaseTables::build();
    run_hybrid(pdbs, &tables, &tp, &cubies, cancel)
}

/// A reusable solver: the [`Pdbs`] plus the prebuilt in-memory acceleration tables — the
/// Korf [`SearchTables`] and the [`two_phase::TwoPhaseTables`] used by the hybrid fallback.
/// Construct once (the tables build in ~0.3–0.6 s) and call [`Solver::solve`] for every solve
/// so the tables are amortised across solves rather than rebuilt each time. The on-disk PDB
/// format is unchanged — both table sets are purely in-memory.
pub struct Solver {
    pdbs: Pdbs,
    tables: SearchTables,
    tp: two_phase::TwoPhaseTables,
}

impl Solver {
    /// Build a solver from already-loaded [`Pdbs`], materialising both the dense Korf
    /// [`SearchTables`] and the [`two_phase::TwoPhaseTables`] once. Pure (no Bevy); the
    /// caller runs this once, off-thread.
    pub fn new(pdbs: Pdbs) -> Solver {
        let tables = SearchTables::build();
        let tp = two_phase::TwoPhaseTables::build();
        Solver { pdbs, tables, tp }
    }

    /// Solve `state` using the prebuilt tables. Same contract as the free [`solve`]
    /// (validate -> convert -> hybrid Korf-optimal-with-two-phase-fallback), but with no
    /// per-call table build.
    pub fn solve(&self, state: &CubeState, cancel: &AtomicBool) -> Result<Vec<Move>, SolveError> {
        let cubies = state_to_cubies(state)?;
        run_hybrid(&self.pdbs, &self.tables, &self.tp, &cubies, cancel)
    }
}

/// Convert `state` to our coordinate arrays via the in-house facelet converter, which
/// also validates physical solvability — a malformed or impossible cube becomes
/// [`SolveError::Unsolvable`]. Shared by [`solve`] and [`Solver::solve`].
fn state_to_cubies(state: &CubeState) -> Result<coords::Cubies, SolveError> {
    let facelets = state_to_facelets(state);
    facelet::facelets_to_cubies(&facelets).ok_or(SolveError::Unsolvable)
}

/// Default Korf time budget (ms) before the hybrid falls back to the two-phase solver.
const DEFAULT_KORF_BUDGET_MS: u64 = 4000;

/// Resolve the Korf time budget. `CUBR_KORF_BUDGET_MS`, if set to a parseable value,
/// overrides [`DEFAULT_KORF_BUDGET_MS`] (handy for tests/benches that want to force the
/// two-phase fallback quickly); absent / unparseable falls back to the default.
fn korf_budget_ms() -> u64 {
    std::env::var("CUBR_KORF_BUDGET_MS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_KORF_BUDGET_MS)
}

/// The hybrid solve both public entry points delegate to. Resolves the Korf budget from
/// [`korf_budget_ms`] and runs [`run_hybrid_budget`].
fn run_hybrid(
    pdbs: &Pdbs,
    tables: &SearchTables,
    tp: &two_phase::TwoPhaseTables,
    cubies: &coords::Cubies,
    cancel: &AtomicBool,
) -> Result<Vec<Move>, SolveError> {
    let budget = Duration::from_millis(korf_budget_ms());
    run_hybrid_budget(pdbs, tables, tp, cubies, cancel, budget)
}

/// The hybrid solve with an explicit Korf `budget`. Runs the guaranteed-optimal Korf IDA*
/// under the wall-clock budget enforced by a watchdog thread; the watchdog also honours
/// the external `cancel`. Outcomes:
/// - Korf finishes within budget  -> `Ok(moves)` (exact optimal).
/// - external `cancel` fired       -> `Err(Cancelled)`.
/// - budget expired (Korf cancelled, but not by the user) -> run the near-optimal
///   two-phase from the same state (still honouring `cancel`) and return that.
///
/// Taking `budget` as a parameter (rather than only reading the env) lets tests force the
/// fallback path with `Duration::ZERO` without mutating process-global environment state
/// (a `setenv`/`getenv` data race under the parallel test runner).
fn run_hybrid_budget(
    pdbs: &Pdbs,
    tables: &SearchTables,
    tp: &two_phase::TwoPhaseTables,
    cubies: &coords::Cubies,
    cancel: &AtomicBool,
    budget: Duration,
) -> Result<Vec<Move>, SolveError> {
    let korf_cancel = AtomicBool::new(false);
    let done = AtomicBool::new(false);

    // Watchdog: trip `korf_cancel` when the user cancels OR the budget elapses; exit as
    // soon as the Korf search reports `done`. Scoped so it joins before we read `korf`.
    let korf = std::thread::scope(|s| {
        s.spawn(|| {
            let start = Instant::now();
            loop {
                if done.load(Ordering::Relaxed) {
                    return;
                }
                if cancel.load(Ordering::Relaxed) || start.elapsed() >= budget {
                    korf_cancel.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let r = search::search(pdbs, tables, cubies, &korf_cancel);
        done.store(true, Ordering::Relaxed);
        r
    });

    if let Some(moves) = korf {
        return Ok(moves); // exact optimal within budget
    }
    // Korf returned None only because `korf_cancel` was tripped. Distinguish the cause.
    if cancel.load(Ordering::Relaxed) {
        return Err(SolveError::Cancelled);
    }
    // Budget expired -> near-optimal two-phase (which itself honours `cancel`).
    match two_phase::solve(tp, cubies, cancel) {
        Some(idxs) => Ok(idxs.into_iter().map(coords::index_to_move).collect()),
        None => Err(SolveError::Cancelled),
    }
}

/// Map a sticker color to the facelet *letter* (face letter) of the face whose
/// solved center is that color. The scheme is fixed by the README, so this is the
/// inverse of [`Face::solved_color`].
fn color_to_facelet_char(color: StickerColor) -> char {
    match color {
        StickerColor::W => 'U', // U center is white
        StickerColor::R => 'R', // R center is red
        StickerColor::G => 'F', // F center is green
        StickerColor::Y => 'D', // D center is yellow
        StickerColor::O => 'L', // L center is orange
        StickerColor::B => 'B', // B center is blue
    }
}

/// Produce the URFDLB facelet string (54 chars) consumed by [`facelet::facelets_to_cubies`].
/// Faces are concatenated in order U, R, F, D, L, B; within each face, indices 0..9 in our
/// row-major order (which is the README per-face read order). Each sticker becomes its face
/// letter.
fn state_to_facelets(state: &CubeState) -> String {
    // URFDLB facelet order (the Kociemba-style layout the converter expects).
    const FACE_ORDER: [Face; 6] = [Face::U, Face::R, Face::F, Face::D, Face::L, Face::B];
    let mut s = String::with_capacity(54);
    for face in FACE_ORDER {
        for &color in state.face(face) {
            s.push(color_to_facelet_char(color));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CubeCore;
    use coords::{apply, move_to_index, Cubies, SOLVED};
    use std::sync::atomic::AtomicBool;

    /// Tiny deterministic LCG (Numerical Recipes); no `rand` crate.
    fn lcg(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *seed
    }

    /// Apply `seq` to a fresh solved core and return its state.
    fn scrambled_state(seq: &[Move]) -> CubeState {
        let mut core = CubeCore::solved();
        for &m in seq {
            core.apply(m);
        }
        core.to_state()
    }

    /// The full pipeline used by `solve`: `CubeState` -> facelets -> in-house converter
    /// -> `Cubies`.
    fn state_through_facelets(state: &CubeState) -> Cubies {
        facelet::facelets_to_cubies(&state_to_facelets(state)).expect("valid")
    }

    /// Read a `kewb::CubieCube` into our `Cubies` arrays (dev-only test oracle). Mirrors
    /// the field-for-field copy the old kewb runtime path used.
    fn cubies_from_kewb(c: &kewb::CubieCube) -> Cubies {
        let mut out = SOLVED;
        for i in 0..8 {
            out.cp[i] = c.cp[i] as u8;
            out.co[i] = c.co[i];
        }
        for i in 0..12 {
            out.ep[i] = c.ep[i] as u8;
            out.eo[i] = c.eo[i];
        }
        out
    }

    // The solved state produces the canonical solved facelet string.
    #[test]
    fn solved_facelets_string_is_canonical() {
        let expected = "UUUUUUUUURRRRRRRRRFFFFFFFFFDDDDDDDDDLLLLLLLLLBBBBBBBBB";
        let actual = state_to_facelets(&CubeState::solved());
        assert_eq!(actual.len(), 54);
        assert_eq!(actual, expected);
    }

    // ★ Conversion-integrity guard (fast, no PDBs): the CubeState -> Cubies pipeline
    // must agree with applying the same scramble to `coords::SOLVED` via `coords::apply`
    // (the move model the PDBs / search use). This is the key integration check that a
    // facelet/parity error would catch without building any tables.
    #[test]
    fn conversion_pipeline_matches_coords_apply() {
        // Solved round-trips to SOLVED.
        assert_eq!(state_through_facelets(&CubeState::solved()), SOLVED);

        let mut seed = 0x1234_5678u32;
        for _ in 0..50 {
            // Build a deterministic random scramble as Vec<Move>, applying it both to a
            // CubeCore (for the facelet pipeline) and to `coords::SOLVED` (for the model).
            let len = 1 + (lcg(&mut seed) as usize % 30); // 1..=30 moves
            let mut scramble: Vec<Move> = Vec::with_capacity(len);
            let mut expected = SOLVED;
            for _ in 0..len {
                let idx = (lcg(&mut seed) as usize) % 18;
                let m = Move::ALL[idx];
                scramble.push(m);
                expected = apply(&expected, move_to_index(m));
            }

            let state = scrambled_state(&scramble);
            let through = state_through_facelets(&state);
            assert_eq!(
                through, expected,
                "pipeline diverged from coords::apply for scramble {scramble:?}"
            );
        }
    }

    // A physically impossible state is rejected as Unsolvable. Flip a single edge in
    // place (the U/F edge): color counts stay correct and the permutation is valid, but
    // the edge-orientation parity is now odd — physically unreachable, so the in-house
    // `facelet::facelets_to_cubies` validator rejects it. (A lone off-color sticker can
    // re-parse to a valid cubie and is NOT a reliable rejection.)
    #[test]
    fn impossible_state_is_unsolvable() {
        // README per-face read order: U index 7 is the U/F edge sticker on the U face;
        // F index 1 is the U/F edge sticker on the F face (both centers stay).
        let mut bad = CubeState::solved();
        assert_eq!(bad.U[7], StickerColor::W);
        assert_eq!(bad.F[1], StickerColor::G);
        bad.U[7] = StickerColor::G;
        bad.F[1] = StickerColor::W;

        // `solve` rejects the cube during in-house validation, before the search ever
        // reads the PDBs — so a cheap empty `Pdbs` (no slow generation) suffices here.
        let empty = Pdbs {
            corner: Vec::new(),
            edge_a: Vec::new(),
            edge_b: Vec::new(),
        };
        let cancel = AtomicBool::new(false);
        assert_eq!(
            solve(&empty, &bad, &cancel),
            Err(SolveError::Unsolvable),
            "an odd-parity edge flip must be rejected before the search"
        );
    }

    // ★ Cross-check the in-house facelet converter against kewb (the dev-dep oracle): for
    // the solved state and ~50 deterministic random scrambles, our
    // `facelet::facelets_to_cubies` must produce byte-for-byte the same `Cubies` as
    // parsing the same facelet string through kewb's FaceCube/CubieCube. This proves the
    // port is exact and the dropped runtime dependency changed nothing.
    #[test]
    fn facelet_conversion_matches_kewb() {
        let through_kewb = |state: &CubeState| -> Cubies {
            let facelets = state_to_facelets(state);
            let face = kewb::FaceCube::try_from(facelets.as_str()).expect("valid facelets");
            let cubie = kewb::CubieCube::try_from(&face).expect("solvable cube");
            cubies_from_kewb(&cubie)
        };

        // Solved state.
        assert_eq!(
            state_through_facelets(&CubeState::solved()),
            through_kewb(&CubeState::solved()),
            "solved state diverged from kewb"
        );

        let mut seed = 0x9E37_79B9u32;
        for _ in 0..50 {
            let len = 1 + (lcg(&mut seed) as usize % 30); // 1..=30 moves
            let mut scramble: Vec<Move> = Vec::with_capacity(len);
            for _ in 0..len {
                scramble.push(Move::ALL[(lcg(&mut seed) as usize) % 18]);
            }
            let state = scrambled_state(&scramble);
            assert_eq!(
                state_through_facelets(&state),
                through_kewb(&state),
                "facelet conversion diverged from kewb for scramble {scramble:?}"
            );
        }
    }

    // ★ Hybrid Korf-success wiring (fast, no real PDBs): the solved state short-circuits
    // inside the Korf `search` before it ever reads the PDBs, so a cheap empty `Pdbs`
    // suffices. This proves the Korf-success path of `run_hybrid` returns `Ok(vec![])`.
    #[test]
    fn hybrid_korf_success_on_solved() {
        let empty = Pdbs {
            corner: Vec::new(),
            edge_a: Vec::new(),
            edge_b: Vec::new(),
        };
        let never = AtomicBool::new(false);
        assert_eq!(solve(&empty, &CubeState::solved(), &never), Ok(vec![]));
    }

    // ★ Hybrid two-phase fallback wiring (fast): a correctly-sized all-zero `Pdbs` is a
    // valid, admissible zero heuristic (no real Korf PDBs needed). A `Duration::ZERO` Korf
    // budget cancels Korf immediately, so the hybrid must take the two-phase path. The
    // returned solution is checked to actually solve a deterministic deep scramble. We call
    // `run_hybrid_budget` directly so no process-global env var is touched (which would race
    // the parallel test runner's other `getenv` callers).
    #[test]
    fn hybrid_falls_back_to_two_phase() {
        // A correctly-sized all-zero PDB set: an admissible (zero) heuristic, so the Korf
        // search would still be correct — but the 0 ms budget cancels it before it runs.
        let zero = Pdbs {
            corner: vec![0u8; super::pdb::CORNER_SIZE.div_ceil(2)],
            edge_a: vec![0u8; super::pdb::EDGE_SIZE.div_ceil(2)],
            edge_b: vec![0u8; super::pdb::EDGE_SIZE.div_ceil(2)],
        };
        let tables = SearchTables::build();
        let tp = two_phase::TwoPhaseTables::build();

        // A deterministic deep scramble (25 moves) built via the LCG helper.
        let mut seed = 0x0BAD_F00Du32;
        let mut scramble: Vec<Move> = Vec::with_capacity(25);
        for _ in 0..25 {
            scramble.push(Move::ALL[(lcg(&mut seed) as usize) % 18]);
        }
        let state = scrambled_state(&scramble);
        let cubies = state_to_cubies(&state).expect("solvable scramble");

        let never = AtomicBool::new(false);
        let sol = run_hybrid_budget(
            &zero,
            &tables,
            &tp,
            &cubies,
            &never,
            std::time::Duration::ZERO,
        )
        .expect("two-phase fallback must return a solution");

        assert!(!sol.is_empty(), "deep scramble fallback solution was empty");
        assert!(sol.len() <= 24, "fallback solution too long: {}", sol.len());

        // Apply the scramble then the solution to a fresh core; it must be solved.
        let mut core = CubeCore::solved();
        for &m in scramble.iter().chain(&sol) {
            core.apply(m);
        }
        assert_eq!(
            core.to_state(),
            CubeState::solved(),
            "two-phase fallback solution did not solve the cube"
        );
    }

    // End-to-end (ignored: builds the full ~85 MB PDBs). The Korf path solves a shallow
    // 8-quarter-turn scramble optimally (<= 8); a tiny budget forces the two-phase fallback
    // on a deep ~28-move scramble, whose solution must still re-solve the cube.
    #[test]
    #[ignore = "builds the full ~85 MB PDBs; exercises both hybrid paths (slow; run in release)"]
    fn hybrid_end_to_end() {
        let pdbs = Pdbs::generate();
        let solver = Solver::new(pdbs);
        let cancel = AtomicBool::new(false);
        let solved = CubeState::solved();

        // Korf path: a shallow 8-quarter-turn scramble must solve optimally (<= 8).
        let reported: Vec<Move> = ["R", "U", "F", "L", "D", "B", "R", "U"]
            .iter()
            .map(|s| Move::parse(s).unwrap())
            .collect();
        let state = scrambled_state(&reported);
        let sol = solver.solve(&state, &cancel).unwrap();
        assert!(
            sol.len() <= 8,
            "Korf path: 8-quarter-turn scramble solved in {} moves (> 8)",
            sol.len()
        );
        let mut core = CubeCore::solved();
        for &m in reported.iter().chain(&sol) {
            core.apply(m);
        }
        assert_eq!(
            core.to_state(),
            solved,
            "Korf path: reported case not solved"
        );

        // Two-phase fallback path: a tiny budget on a deep ~28-move scramble. The returned
        // solution must still re-solve the cube (correctness, not optimality). We drive
        // `run_hybrid_budget` directly (via the solver's prebuilt tables) so no env var is
        // mutated — a 150 ms budget forces the fallback.
        let mut seed = 0xDEEF_2810u32;
        let mut scramble: Vec<Move> = Vec::with_capacity(28);
        for _ in 0..28 {
            scramble.push(Move::ALL[(lcg(&mut seed) as usize) % 18]);
        }
        let state = scrambled_state(&scramble);
        let cubies = state_to_cubies(&state).unwrap();
        let sol = run_hybrid_budget(
            &solver.pdbs,
            &solver.tables,
            &solver.tp,
            &cubies,
            &cancel,
            std::time::Duration::from_millis(150),
        )
        .unwrap();
        assert!(
            !sol.is_empty(),
            "fallback path: deep scramble solution empty"
        );
        let mut core = CubeCore::solved();
        for &m in scramble.iter().chain(&sol) {
            core.apply(m);
        }
        assert_eq!(
            core.to_state(),
            solved,
            "fallback path: deep scramble solution did not solve the cube"
        );
    }

    // End-to-end (ignored: builds the full ~85 MB PDBs). For a few scrambles, `solve`
    // returns a solution that reapplies to solved and is <= 20 moves; the reported
    // 8-quarter-turn case must solve in <= 8.
    #[test]
    #[ignore = "builds the full ~85 MB PDBs and runs optimal solves (slow; run in release)"]
    fn solve_end_to_end() {
        let pdbs = Pdbs::generate();
        let cancel = AtomicBool::new(false);
        let solved = CubeState::solved();

        // Already-solved -> empty solution.
        assert!(solve(&pdbs, &solved, &cancel).unwrap().is_empty());

        // The reported case: an 8-quarter-turn scramble must solve in <= 8.
        let reported: Vec<Move> = ["R", "U", "F", "L", "D", "B", "R", "U"]
            .iter()
            .map(|s| Move::parse(s).unwrap())
            .collect();
        let state = scrambled_state(&reported);
        let sol = solve(&pdbs, &state, &cancel).unwrap();
        assert!(
            sol.len() <= 8,
            "8-quarter-turn scramble solved in {} moves (> 8)",
            sol.len()
        );
        let mut core = CubeCore::solved();
        for &m in reported.iter().chain(&sol) {
            core.apply(m);
        }
        assert_eq!(core.to_state(), solved, "reported case not solved");

        // A few deterministic random scrambles: solution <= 20 and reapplies to solved.
        let mut seed = 0xC0DE_1234u32;
        for _ in 0..5 {
            let len = 8 + (lcg(&mut seed) as usize % 13); // 8..=20
            let mut scramble: Vec<Move> = Vec::with_capacity(len);
            for _ in 0..len {
                scramble.push(Move::ALL[(lcg(&mut seed) as usize) % 18]);
            }
            let state = scrambled_state(&scramble);
            let sol = solve(&pdbs, &state, &cancel).unwrap();
            assert!(
                sol.len() <= 20,
                "random scramble solved in {} (> 20)",
                sol.len()
            );
            let mut core = CubeCore::solved();
            for &m in scramble.iter().chain(&sol) {
                core.apply(m);
            }
            assert_eq!(core.to_state(), solved, "random scramble not solved");
        }
    }
}
