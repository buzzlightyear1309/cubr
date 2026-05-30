//! Pure two-phase (Kociemba) solver adapter around the `kewb` crate.
//!
//! No Bevy types or systems live here — this mirrors `cube::core` in being a pure,
//! fully unit-tested module. The Bevy layer (a later unit) caches the `DataTable`
//! produced by [`build_tables`] off-thread and feeds states through [`solve`].
//!
//! ## Mapping to `kewb`
//! `kewb` works in a facelet alphabet of face *letters* `U R F D L B` (its
//! [`kewb::Color`]) laid out in face order **U, R, F, D, L, B**, 9 facelets each,
//! row-major. Our [`CubeState`] uses the same face order (its struct fields) and the
//! README per-face read order is already the Kociemba-style layout (mirrored `B`),
//! so the conversion is a straight concatenation once each sticker color is mapped to
//! the face letter of the face whose solved center is that color
//! (`W->U, R->R, G->F, Y->D, O->L, B->B`). The solve-and-reapply tests verify this
//! end-to-end: any per-face mirror/rotation error would make a real scramble fail.

use crate::cube::model::{CubeState, Face, Move, StickerColor, Turn};

pub use kewb::DataTable;

/// Error from solving. `Unsolvable` covers physically impossible / unparseable states
/// (wrong color counts, bad permutation/orientation parity, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolveError {
    Unsolvable,
}

/// Maximum solution length we ask the two-phase solver for. Kociemba's algorithm
/// finds a solution within 23 moves for any solvable cube at this bound.
const MAX_SOLUTION_LEN: u8 = 23;

/// Generate the two-phase move + pruning tables. SLOW (seconds) — the caller runs
/// this once, off-thread, and caches the handle. Pure (no Bevy).
pub fn build_tables() -> DataTable {
    DataTable::default()
}

/// Solve `state` with the two-phase algorithm. Returns the solution as our absolute
/// [`Move`]s (low move count, `<= 23`). An already-solved state returns an empty vec
/// (`Ok(vec![])`). A physically impossible / invalid state returns
/// `Err(SolveError::Unsolvable)`.
pub fn solve(tables: &DataTable, state: &CubeState) -> Result<Vec<Move>, SolveError> {
    let facelets = state_to_facelets(state);
    let face_cube =
        kewb::FaceCube::try_from(facelets.as_str()).map_err(|_| SolveError::Unsolvable)?;
    let cubie_cube = kewb::CubieCube::try_from(&face_cube).map_err(|_| SolveError::Unsolvable)?;

    let mut solver = kewb::Solver::new(tables, MAX_SOLUTION_LEN, None);
    let solution = solver.solve(cubie_cube).ok_or(SolveError::Unsolvable)?;

    Ok(solution
        .get_all_moves()
        .into_iter()
        .map(from_kewb_move)
        .collect())
}

/// Map a sticker color to the `kewb` facelet *letter* (face letter) of the face whose
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

/// Produce the `kewb` URFDLB facelet string (54 chars). Faces are concatenated in
/// order U, R, F, D, L, B; within each face, indices 0..9 in our row-major order
/// (which is the README per-face read order). Each sticker becomes its face letter.
fn state_to_facelets(state: &CubeState) -> String {
    // kewb's facelet order is U, R, F, D, L, B (its `Color`/`FaceCube` ordering).
    const KEWB_FACE_ORDER: [Face; 6] = [Face::U, Face::R, Face::F, Face::D, Face::L, Face::B];
    let mut s = String::with_capacity(54);
    for face in KEWB_FACE_ORDER {
        for &color in state.face(face) {
            s.push(color_to_facelet_char(color));
        }
    }
    s
}

/// Map a `kewb` move variant to our absolute [`Move`]. The plain variant is a CW
/// quarter-turn, the `2` variant is a 180° turn, and the `3` variant is CCW (prime).
fn from_kewb_move(m: kewb::Move) -> Move {
    use kewb::Move as K;
    let (face, turn) = match m {
        K::U => (Face::U, Turn::Cw),
        K::U2 => (Face::U, Turn::Double),
        K::U3 => (Face::U, Turn::Ccw),
        K::D => (Face::D, Turn::Cw),
        K::D2 => (Face::D, Turn::Double),
        K::D3 => (Face::D, Turn::Ccw),
        K::R => (Face::R, Turn::Cw),
        K::R2 => (Face::R, Turn::Double),
        K::R3 => (Face::R, Turn::Ccw),
        K::L => (Face::L, Turn::Cw),
        K::L2 => (Face::L, Turn::Double),
        K::L3 => (Face::L, Turn::Ccw),
        K::F => (Face::F, Turn::Cw),
        K::F2 => (Face::F, Turn::Double),
        K::F3 => (Face::F, Turn::Ccw),
        K::B => (Face::B, Turn::Cw),
        K::B2 => (Face::B, Turn::Double),
        K::B3 => (Face::B, Turn::Ccw),
    };
    Move { face, turn }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cube::core::CubeCore;
    use std::sync::OnceLock;

    /// Build the (slow) two-phase tables exactly once for the whole test module.
    fn tables() -> &'static DataTable {
        static TABLES: OnceLock<DataTable> = OnceLock::new();
        TABLES.get_or_init(build_tables)
    }

    /// Parse a scramble sequence of notation strings into our `Move`s.
    fn scramble(seq: &[&str]) -> Vec<Move> {
        seq.iter()
            .map(|s| Move::parse(s).unwrap_or_else(|| panic!("bad move {s}")))
            .collect()
    }

    /// Apply `seq` to a fresh solved core and return its state.
    fn scrambled_state(seq: &[Move]) -> CubeState {
        let mut core = CubeCore::solved();
        for &m in seq {
            core.apply(m);
        }
        core.to_state()
    }

    // Test 1: the solved state produces the canonical kewb solved facelet string.
    #[test]
    fn solved_facelets_string_is_canonical() {
        let expected = "UUUUUUUUURRRRRRRRRFFFFFFFFFDDDDDDDDDLLLLLLLLLBBBBBBBBB";
        let actual = state_to_facelets(&CubeState::solved());
        assert_eq!(actual.len(), 54);
        assert_eq!(actual, expected);
    }

    // Test 2: kewb move -> our Move mapping, covering all six faces and all turns.
    #[test]
    fn kewb_move_mapping_covers_all_faces() {
        use kewb::Move as K;
        let cw = Turn::Cw;
        let ccw = Turn::Ccw;
        let dbl = Turn::Double;

        assert_eq!(
            from_kewb_move(K::U),
            Move {
                face: Face::U,
                turn: cw
            }
        );
        assert_eq!(
            from_kewb_move(K::U2),
            Move {
                face: Face::U,
                turn: dbl
            }
        );
        assert_eq!(
            from_kewb_move(K::U3),
            Move {
                face: Face::U,
                turn: ccw
            }
        );

        assert_eq!(
            from_kewb_move(K::D),
            Move {
                face: Face::D,
                turn: cw
            }
        );
        assert_eq!(
            from_kewb_move(K::D2),
            Move {
                face: Face::D,
                turn: dbl
            }
        );
        assert_eq!(
            from_kewb_move(K::D3),
            Move {
                face: Face::D,
                turn: ccw
            }
        );

        assert_eq!(
            from_kewb_move(K::R),
            Move {
                face: Face::R,
                turn: cw
            }
        );
        assert_eq!(
            from_kewb_move(K::R2),
            Move {
                face: Face::R,
                turn: dbl
            }
        );
        assert_eq!(
            from_kewb_move(K::R3),
            Move {
                face: Face::R,
                turn: ccw
            }
        );

        assert_eq!(
            from_kewb_move(K::L),
            Move {
                face: Face::L,
                turn: cw
            }
        );
        assert_eq!(
            from_kewb_move(K::L2),
            Move {
                face: Face::L,
                turn: dbl
            }
        );
        assert_eq!(
            from_kewb_move(K::L3),
            Move {
                face: Face::L,
                turn: ccw
            }
        );

        assert_eq!(
            from_kewb_move(K::F),
            Move {
                face: Face::F,
                turn: cw
            }
        );
        assert_eq!(
            from_kewb_move(K::F2),
            Move {
                face: Face::F,
                turn: dbl
            }
        );
        assert_eq!(
            from_kewb_move(K::F3),
            Move {
                face: Face::F,
                turn: ccw
            }
        );

        assert_eq!(
            from_kewb_move(K::B),
            Move {
                face: Face::B,
                turn: cw
            }
        );
        assert_eq!(
            from_kewb_move(K::B2),
            Move {
                face: Face::B,
                turn: dbl
            }
        );
        assert_eq!(
            from_kewb_move(K::B3),
            Move {
                face: Face::B,
                turn: ccw
            }
        );
    }

    // Test 3: solve-and-reapply on known scrambles — the key correctness gate.
    #[test]
    fn solve_and_reapply_known_scrambles() {
        let solved = CubeState::solved();

        // Empty scramble: already solved -> empty solution.
        let empty: Vec<Move> = vec![];
        let solution = solve(tables(), &scrambled_state(&empty)).unwrap();
        assert!(solution.is_empty(), "solved cube should need no moves");

        let cases: &[&[&str]] = &[
            &["R", "U", "R'", "U'"],
            &[
                "R", "U", "F", "L", "D", "B", "R'", "U2", "F'", "L2", "D'", "B2",
            ],
            &[
                "F", "R", "U", "R'", "U'", "F'", "R", "U2", "R'", "U2", "R", "U'", "R'", "U'", "F",
                "F", "U", "R", "U'", "R'",
            ],
        ];

        for case in cases {
            let seq = scramble(case);
            let state = scrambled_state(&seq);

            let solution = solve(tables(), &state).unwrap();
            assert!(
                solution.len() <= MAX_SOLUTION_LEN as usize,
                "solution for {case:?} too long: {}",
                solution.len()
            );

            // Reapply the solution to the SAME scramble and assert solved.
            let mut core = CubeCore::solved();
            for &m in &seq {
                core.apply(m);
            }
            for &m in &solution {
                core.apply(m);
            }
            assert_eq!(core.to_state(), solved, "scramble {case:?} not solved");
        }
    }

    // Test 4: deterministic pseudo-random scrambles (tiny LCG, no `rand` crate).
    #[test]
    fn solve_and_reapply_random_scrambles() {
        let solved = CubeState::solved();

        // Numerical Recipes LCG; deterministic for a fixed seed.
        let mut seed: u32 = 0x1234_5678;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };

        for _ in 0..20 {
            let mut core = CubeCore::solved();
            for _ in 0..20 {
                let m = Move::ALL[(next() as usize) % Move::ALL.len()];
                core.apply(m);
            }
            let state = core.to_state();

            let solution = solve(tables(), &state).unwrap();
            assert!(
                solution.len() <= MAX_SOLUTION_LEN as usize,
                "random solution too long: {}",
                solution.len()
            );

            for &m in &solution {
                core.apply(m);
            }
            assert_eq!(core.to_state(), solved, "random scramble not solved");
        }
    }

    // Test 5: a physically impossible state is rejected as Unsolvable.
    #[test]
    fn impossible_state_is_unsolvable() {
        // Flip a single edge in place: swap the two stickers of the UF edge (its U
        // facelet and its F facelet). Color counts stay correct and the permutation
        // is valid, but the edge-orientation parity is now odd — which is physically
        // impossible to reach on a real cube, so kewb's CubieCube conversion rejects
        // it (a robust unsolvable case; a lone off-color sticker can re-parse to a
        // valid cubie and is NOT a reliable rejection).
        //
        // README per-face read order: U index 7 is the U/F edge sticker on the U
        // face; F index 1 is the U/F edge sticker on the F face (both centers stay).
        let mut bad = CubeState::solved();
        assert_eq!(bad.U[7], StickerColor::W);
        assert_eq!(bad.F[1], StickerColor::G);
        bad.U[7] = StickerColor::G;
        bad.F[1] = StickerColor::W;

        assert_eq!(solve(tables(), &bad), Err(SolveError::Unsolvable));
    }
}
