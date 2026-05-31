//! Guaranteed-optimal Korf IDA* solver.
//!
//! No Bevy types or systems live here — this mirrors [`crate::core`] in being a pure,
//! fully unit-tested module. The Bevy layer (`solve_ui`, in the `cubr` binary)
//! loads/generates the
//! [`Pdbs`] off-thread and feeds states through [`solve`].
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

/// Solve `state` optimally. Validates the state via the in-house facelet converter (an
/// impossible cube returns [`SolveError::Unsolvable`]), converts it to our coordinate
/// arrays, then runs the
/// guaranteed-optimal IDA* search. `cancel` aborts the search
/// (-> [`SolveError::Cancelled`]). An already-solved state returns `Ok(vec![])`.
///
/// This convenience entry point builds the in-memory [`SearchTables`] *per call* (~0.3–0.6
/// s + ~62 MB). For repeated solves (the GUI's one-shot Solve button plus live re-solves)
/// prefer [`Solver`], which builds the tables once and reuses them.
pub fn solve(pdbs: &Pdbs, state: &CubeState, cancel: &AtomicBool) -> Result<Vec<Move>, SolveError> {
    let cubies = state_to_cubies(state)?;
    let tables = SearchTables::build();
    run_search(pdbs, &tables, &cubies, cancel)
}

/// A reusable solver: the [`Pdbs`] plus the prebuilt [`SearchTables`] acceleration
/// structure. Construct once (the tables build in ~0.3–0.6 s) and call [`Solver::solve`]
/// for every solve so the tables are amortised across solves rather than rebuilt each
/// time. The on-disk PDB format is unchanged — [`SearchTables`] is purely in-memory.
pub struct Solver {
    pdbs: Pdbs,
    tables: SearchTables,
}

impl Solver {
    /// Build a solver from already-loaded [`Pdbs`], materialising the dense
    /// [`SearchTables`] once. Pure (no Bevy); the caller runs this once, off-thread.
    pub fn new(pdbs: Pdbs) -> Solver {
        let tables = SearchTables::build();
        Solver { pdbs, tables }
    }

    /// Solve `state` optimally using the prebuilt tables. Same contract as the free
    /// [`solve`] (validate -> convert -> guaranteed-optimal IDA*), but with no per-call
    /// table build.
    pub fn solve(&self, state: &CubeState, cancel: &AtomicBool) -> Result<Vec<Move>, SolveError> {
        let cubies = state_to_cubies(state)?;
        run_search(&self.pdbs, &self.tables, &cubies, cancel)
    }
}

/// Convert `state` to our coordinate arrays via the in-house facelet converter, which
/// also validates physical solvability — a malformed or impossible cube becomes
/// [`SolveError::Unsolvable`]. Shared by [`solve`] and [`Solver::solve`].
fn state_to_cubies(state: &CubeState) -> Result<coords::Cubies, SolveError> {
    let facelets = state_to_facelets(state);
    facelet::facelets_to_cubies(&facelets).ok_or(SolveError::Unsolvable)
}

/// Run the guaranteed-optimal coordinate search and map cancellation to
/// [`SolveError::Cancelled`]. Shared by [`solve`] and [`Solver::solve`] (DRY).
fn run_search(
    pdbs: &Pdbs,
    tables: &SearchTables,
    cubies: &coords::Cubies,
    cancel: &AtomicBool,
) -> Result<Vec<Move>, SolveError> {
    match search::search(pdbs, tables, cubies, cancel) {
        Some(moves) => Ok(moves),
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
