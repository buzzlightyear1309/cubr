//! In-house facelet → [`Cubies`](super::coords::Cubies) converter + physical-solvability
//! validator. This replaces the former `kewb` runtime dependency: a 54-char URFDLB
//! facelet string is parsed straight into our integer cubie arrays and validated, with
//! `kewb` retained only as a dev-only test oracle.
//!
//! ## Attribution
//! The tables ([`CORNER_FACELET`], [`EDGE_FACELET`], [`CORNER_COLOR`], [`EDGE_COLOR`]) and
//! the parse/validate algorithm are a verbatim port of `kewb`'s `cube/facelet.rs` and
//! `cube/cubie.rs` (`CubieCube::try_from(&FaceCube)` + `is_solvable`). `kewb` is MIT —
//! see <https://crates.io/crates/kewb>. The cubie identity / orientation conventions are
//! kewb's, so this produces *identical* [`Cubies`](super::coords::Cubies) to the old kewb
//! path (guarded by the `facelet_conversion_matches_kewb` test in `super`).
//!
//! ## Facelet alphabet
//! Color codes are kewb's `Color` order: `U=0, R=1, F=2, D=3, L=4, B=5`. The 54 facelet
//! indices are laid out in face order **U, R, F, D, L, B** (9 each, row-major):
//! `U=0..9, R=9..18, F=18..27, D=27..36, L=36..45, B=45..54`.

use super::coords::{Cubies, SOLVED};

/// Map the 8 corner positions (UBL, UBR, UFR, UFL, DFL, DFR, DBR, DBL) to their 3
/// facelet indices. Ported from kewb `CORNER_FACELET`.
const CORNER_FACELET: [[usize; 3]; 8] = [
    [0, 36, 47],  // UBL
    [2, 45, 11],  // UBR
    [8, 9, 20],   // UFR
    [6, 18, 38],  // UFL
    [27, 44, 24], // DFL
    [29, 26, 15], // DFR
    [35, 17, 51], // DBR
    [33, 53, 42], // DBL
];

/// Map the 12 edge positions (BL, BR, FR, FL, UB, UR, UF, UL, DF, DR, DB, DL) to their 2
/// facelet indices. Ported from kewb `EDGE_FACELET`.
const EDGE_FACELET: [[usize; 2]; 12] = [
    [50, 39], // BL
    [48, 14], // BR
    [23, 12], // FR
    [21, 41], // FL
    [1, 46],  // UB
    [5, 10],  // UR
    [7, 19],  // UF
    [3, 37],  // UL
    [28, 25], // DF
    [32, 16], // DR
    [34, 52], // DB
    [30, 43], // DL
];

/// Map the 8 corner positions to their 3 facelet colors (codes `U=0,R=1,F=2,D=3,L=4,B=5`).
/// Ported from kewb `CORNER_COLOR`.
const CORNER_COLOR: [[u8; 3]; 8] = [
    [0, 4, 5], // UBL = U L B
    [0, 5, 1], // UBR = U B R
    [0, 1, 2], // UFR = U R F
    [0, 2, 4], // UFL = U F L
    [3, 4, 2], // DFL = D L F
    [3, 2, 1], // DFR = D F R
    [3, 1, 5], // DBR = D R B
    [3, 5, 4], // DBL = D B L
];

/// Map the 12 edge positions to their 2 facelet colors. Ported from kewb `EDGE_COLOR`.
const EDGE_COLOR: [[u8; 2]; 12] = [
    [5, 4], // BL = B L
    [5, 1], // BR = B R
    [2, 1], // FR = F R
    [2, 4], // FL = F L
    [0, 5], // UB = U B
    [0, 1], // UR = U R
    [0, 2], // UF = U F
    [0, 4], // UL = U L
    [3, 2], // DF = D F
    [3, 1], // DR = D R
    [3, 5], // DB = D B
    [3, 4], // DL = D L
];

/// Parse the 54-char URFDLB facelet string, convert to [`Cubies`], and validate physical
/// solvability. Returns `None` if the string is malformed (wrong length / bad char /
/// unidentifiable cubie) or the cube is physically impossible (bad color counts,
/// permutation-parity mismatch, Σco≢0 mod 3, or Σeo≢0 mod 2).
pub(crate) fn facelets_to_cubies(s: &str) -> Option<Cubies> {
    // 1. Length must be 54; each char must map to a color code U/R/F/D/L/B.
    if s.len() != 54 {
        return None;
    }
    let mut face = [0u8; 54];
    for (i, c) in s.chars().enumerate() {
        face[i] = match c {
            'U' => 0,
            'R' => 1,
            'F' => 2,
            'D' => 3,
            'L' => 4,
            'B' => 5,
            _ => return None,
        };
    }

    // 2. Robustness (beyond kewb): each of the 6 color codes must appear exactly 9 times.
    let mut counts = [0u8; 6];
    for &col in &face {
        counts[col as usize] += 1;
    }
    if counts.iter().any(|&n| n != 9) {
        return None;
    }

    let mut out = SOLVED;

    // 3. Corners: identify the cubie at each corner position and its orientation.
    for i in 0..8 {
        // Find the orientation `ori` whose facelet shows U(0) or D(3).
        let mut ori = None;
        for (o, &fi) in CORNER_FACELET[i].iter().enumerate() {
            if face[fi] == 0 || face[fi] == 3 {
                ori = Some(o);
                break;
            }
        }
        let ori = ori?;
        let col1 = face[CORNER_FACELET[i][(ori + 1) % 3]];
        let col2 = face[CORNER_FACELET[i][(ori + 2) % 3]];

        let mut found = None;
        for (j, cc) in CORNER_COLOR.iter().enumerate() {
            if col1 == cc[1] && col2 == cc[2] {
                found = Some(j);
                break;
            }
        }
        let j = found?;
        out.cp[i] = j as u8;
        out.co[i] = ori as u8;
    }

    // 4. Edges: identify the cubie at each edge position and its orientation.
    for i in 0..12 {
        let f0 = face[EDGE_FACELET[i][0]];
        let f1 = face[EDGE_FACELET[i][1]];
        let mut found = None;
        for (j, ec) in EDGE_COLOR.iter().enumerate() {
            if f0 == ec[0] && f1 == ec[1] {
                found = Some((j as u8, 0u8));
                break;
            }
            if f0 == ec[1] && f1 == ec[0] {
                found = Some((j as u8, 1u8));
                break;
            }
        }
        let (j, eo) = found?;
        out.ep[i] = j;
        out.eo[i] = eo;
    }

    // 5. Validate physical solvability (port of kewb `is_solvable`).
    if !is_solvable(&out) {
        return None;
    }

    Some(out)
}

/// Port of kewb's `CubieCube::is_solvable`: no duplicate permutation entries,
/// corner/edge permutation parities agree, Σco ≡ 0 (mod 3), and Σeo ≡ 0 (mod 2).
fn is_solvable(c: &Cubies) -> bool {
    if has_duplicates(&c.cp) || has_duplicates(&c.ep) {
        return false;
    }
    let corner_parity = perm_swaps(&c.cp) % 2;
    let edge_parity = perm_swaps(&c.ep) % 2;
    if corner_parity != edge_parity {
        return false;
    }
    let co_sum: u32 = c.co.iter().map(|&v| v as u32).sum();
    let eo_sum: u32 = c.eo.iter().map(|&v| v as u32).sum();
    co_sum.is_multiple_of(3) && eo_sum.is_multiple_of(2)
}

/// True if any value repeats in `perm`.
fn has_duplicates(perm: &[u8]) -> bool {
    for i in 0..perm.len() {
        for j in (i + 1)..perm.len() {
            if perm[i] == perm[j] {
                return true;
            }
        }
    }
    false
}

/// Number of swaps to sort `perm` into identity order (its permutation parity counter),
/// matching kewb's `count_corner_perm` / `count_edge_perm`.
fn perm_swaps(perm: &[u8]) -> u32 {
    let mut p: Vec<u8> = perm.to_vec();
    let mut count = 0u32;
    for i in 0..p.len() {
        if p[i] as usize != i {
            if let Some(j) = (i + 1..p.len()).find(|&j| p[j] as usize == i) {
                p.swap(i, j);
                count += 1;
            }
        }
    }
    count
}
