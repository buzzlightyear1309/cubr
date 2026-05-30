//! Pure foundation for the Korf solver: a `u8`-array cube model, move composition
//! that reproduces kewb's `Mul` exactly, generic factorial-base ranking/unranking
//! primitives, and a generic nibble-packed BFS pattern-database generator.
//!
//! The kewb crate is used only as the *reference* in tests; the runtime math is
//! entirely on our own arrays.
//!
//! ## Cube model and parity convention
//! We match the kewb cubie model exactly so that pattern databases and the eventual
//! IDA* heuristic stay admissible. The corner / edge identities and orientation
//! conventions are kewb's (`kewb::cube::cubie`):
//! - Corners `0..8`: UBL UBR UFR UFL DFL DFR DBR DBL.
//! - Edges `0..12`: BL BR FR FL UB UR UF UL DF DR DB DL.
//! - `co ∈ 0..3`, `eo ∈ 0..2`.
//!
//! kewb's `CubieCube * CubieCube` (`Mul`) is:
//! ```text
//! res.cp[i] = a.cp[b.cp[i]];  res.co[i] = (a.co[b.cp[i]] + b.co[i]) % 3;
//! res.ep[i] = a.ep[b.ep[i]];  res.eo[i] = (a.eo[b.ep[i]] + b.eo[i]) % 2;
//! ```
//! and `cube.apply_move(m) == cube * MOVE`, i.e. the moved state is `compose(cube, MOVE)`
//! (the cube on the *left*). Our [`apply`] reproduces that order; the `compose_matches_kewb`
//! test is the guard.

use crate::cube::model::{Face, Move, Turn};

/// Cube state on flat `u8` arrays (kewb's cubie convention; see module docs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Cubies {
    /// Corner permutation, values `0..8`.
    pub cp: [u8; 8],
    /// Corner orientation, values `0..3`.
    pub co: [u8; 8],
    /// Edge permutation, values `0..12`.
    pub ep: [u8; 12],
    /// Edge orientation, values `0..2`.
    pub eo: [u8; 12],
}

/// The solved cube: identity permutations, zero orientation.
pub(crate) const SOLVED: Cubies = Cubies {
    cp: [0, 1, 2, 3, 4, 5, 6, 7],
    co: [0, 0, 0, 0, 0, 0, 0, 0],
    ep: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    eo: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
};

// --- The six base (CW quarter-turn) move-cubes, transcribed from kewb `moves.rs`. ---
// Corner ids:  UBL=0 UBR=1 UFR=2 UFL=3 DFL=4 DFR=5 DBR=6 DBL=7
// Edge ids:    BL=0 BR=1 FR=2 FL=3 UB=4 UR=5 UF=6 UL=7 DF=8 DR=9 DB=10 DL=11

/// U (clockwise) — kewb `U_MOVE`.
const U_MOVE: Cubies = Cubies {
    // cp: [UFL, UBL, UBR, UFR, DFL, DFR, DBR, DBL]
    cp: [3, 0, 1, 2, 4, 5, 6, 7],
    co: [0, 0, 0, 0, 0, 0, 0, 0],
    // ep: [BL, BR, FR, FL, UL, UB, UR, UF, DF, DR, DB, DL]
    ep: [0, 1, 2, 3, 7, 4, 5, 6, 8, 9, 10, 11],
    eo: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
};

/// D (clockwise) — kewb `D_MOVE`.
const D_MOVE: Cubies = Cubies {
    // cp: [UBL, UBR, UFR, UFL, DBL, DFL, DFR, DBR]
    cp: [0, 1, 2, 3, 7, 4, 5, 6],
    co: [0, 0, 0, 0, 0, 0, 0, 0],
    // ep: [BL, BR, FR, FL, UB, UR, UF, UL, DL, DF, DR, DB]
    ep: [0, 1, 2, 3, 4, 5, 6, 7, 11, 8, 9, 10],
    eo: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
};

/// R (clockwise) — kewb `R_MOVE`.
const R_MOVE: Cubies = Cubies {
    // cp: [UBL, UFR, DFR, UFL, DFL, DBR, UBR, DBL]
    cp: [0, 2, 5, 3, 4, 6, 1, 7],
    co: [0, 1, 2, 0, 0, 1, 2, 0],
    // ep: [BL, UR, DR, FL, UB, FR, UF, UL, DF, BR, DB, DL]
    ep: [0, 5, 9, 3, 4, 2, 6, 7, 8, 1, 10, 11],
    eo: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
};

/// L (clockwise) — kewb `L_MOVE`.
const L_MOVE: Cubies = Cubies {
    // cp: [DBL, UBR, UFR, UBL, UFL, DFR, DBR, DFL]
    cp: [7, 1, 2, 0, 3, 5, 6, 4],
    co: [2, 0, 0, 1, 2, 0, 0, 1],
    // ep: [DL, BR, FR, UL, UB, UR, UF, BL, DF, DR, DB, FL]
    ep: [11, 1, 2, 7, 4, 5, 6, 0, 8, 9, 10, 3],
    eo: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
};

/// F (clockwise) — kewb `F_MOVE`.
const F_MOVE: Cubies = Cubies {
    // cp: [UBL, UBR, UFL, DFL, DFR, UFR, DBR, DBL]
    cp: [0, 1, 3, 4, 5, 2, 6, 7],
    co: [0, 0, 1, 2, 1, 2, 0, 0],
    // ep: [BL, BR, UF, DF, UB, UR, FL, UL, FR, DR, DB, DL]
    ep: [0, 1, 6, 8, 4, 5, 3, 7, 2, 9, 10, 11],
    eo: [0, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, 0],
};

/// B (clockwise) — kewb `B_MOVE`.
const B_MOVE: Cubies = Cubies {
    // cp: [UBR, DBR, UFR, UFL, DFL, DFR, DBL, UBL]
    cp: [1, 6, 2, 3, 4, 5, 7, 0],
    co: [1, 2, 0, 0, 0, 0, 1, 2],
    // ep: [UB, DB, FR, FL, BR, UR, UF, UL, DF, DR, BL, DL]
    ep: [4, 10, 2, 3, 1, 5, 6, 7, 8, 9, 0, 11],
    eo: [1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0],
};

/// Compose two cube states, reproducing kewb's `CubieCube::Mul` exactly:
/// `res.cp[i] = a.cp[b.cp[i]]`, `res.co[i] = (a.co[b.cp[i]] + b.co[i]) % 3`, and the
/// edge analogues. With `a` the existing state and `b` a move-cube, this is one move
/// applied (see [`apply`]).
pub(crate) fn compose(a: &Cubies, b: &Cubies) -> Cubies {
    let mut res = SOLVED;
    for i in 0..8 {
        let bi = b.cp[i] as usize;
        res.cp[i] = a.cp[bi];
        res.co[i] = (a.co[bi] + b.co[i]) % 3;
    }
    for i in 0..12 {
        let bi = b.ep[i] as usize;
        res.ep[i] = a.ep[bi];
        res.eo[i] = (a.eo[bi] + b.eo[i]) % 2;
    }
    res
}

/// The 18 move-cubes in our [`Move::ALL`] order: per face `Cw, Ccw, Double`, faces in
/// the order U D L R F B. Quarter-turn squares/cubes are derived from the CW bases.
pub(crate) const MOVE_CUBES: [Cubies; 18] = {
    // const-fn-free derivation: build each face's triple from its CW base.
    let u = U_MOVE;
    let u2 = compose_const(&u, &u);
    let u3 = compose_const(&u2, &u);
    let d = D_MOVE;
    let d2 = compose_const(&d, &d);
    let d3 = compose_const(&d2, &d);
    let l = L_MOVE;
    let l2 = compose_const(&l, &l);
    let l3 = compose_const(&l2, &l);
    let r = R_MOVE;
    let r2 = compose_const(&r, &r);
    let r3 = compose_const(&r2, &r);
    let f = F_MOVE;
    let f2 = compose_const(&f, &f);
    let f3 = compose_const(&f2, &f);
    let b = B_MOVE;
    let b2 = compose_const(&b, &b);
    let b3 = compose_const(&b2, &b);
    [
        u, u3, u2, // U  U'  U2
        d, d3, d2, // D  D'  D2
        l, l3, l2, // L  L'  L2
        r, r3, r2, // R  R'  R2
        f, f3, f2, // F  F'  F2
        b, b3, b2, // B  B'  B2
    ]
};

/// `const`-context twin of [`compose`] (a `const fn` so [`MOVE_CUBES`] can derive the
/// quarter-turn powers at compile time). Identical math.
const fn compose_const(a: &Cubies, b: &Cubies) -> Cubies {
    let mut res = SOLVED;
    let mut i = 0;
    while i < 8 {
        let bi = b.cp[i] as usize;
        res.cp[i] = a.cp[bi];
        res.co[i] = (a.co[bi] + b.co[i]) % 3;
        i += 1;
    }
    let mut i = 0;
    while i < 12 {
        let bi = b.ep[i] as usize;
        res.ep[i] = a.ep[bi];
        res.eo[i] = (a.eo[bi] + b.eo[i]) % 2;
        i += 1;
    }
    res
}

/// Apply move `move_idx` (index into [`MOVE_CUBES`] / [`Move::ALL`]) to `c`.
///
/// Order matches kewb's `cube.apply_move(m) == cube * MOVE`, i.e. the existing state `c`
/// is the *left* (`a`) operand and the move-cube the *right* (`b`).
pub(crate) fn apply(c: &Cubies, move_idx: usize) -> Cubies {
    compose(c, &MOVE_CUBES[move_idx])
}

/// Map one of our [`Move`]s to its [`MOVE_CUBES`] index (mirrors [`Move::ALL`]).
// Used only by tests (the conversion-integrity guard and the search cross-checks);
// the live solver path goes CubeState -> kewb -> Cubies and never needs this mapping.
#[allow(dead_code)]
pub(crate) fn move_to_index(m: Move) -> usize {
    let face = match m.face {
        Face::U => 0,
        Face::D => 1,
        Face::L => 2,
        Face::R => 3,
        Face::F => 4,
        Face::B => 5,
    };
    let turn = match m.turn {
        Turn::Cw => 0,
        Turn::Ccw => 1,
        Turn::Double => 2,
    };
    face * 3 + turn
}

/// Inverse of [`move_to_index`].
pub(crate) fn index_to_move(i: usize) -> Move {
    Move::ALL[i]
}

// --- Generic ranking / unranking (factorial-base / falling-factorial). ---

/// Lehmer (factorial-base) rank of a permutation of `0..8`. Range `0..40320`.
pub(crate) fn perm_rank8(p: &[u8; 8]) -> u32 {
    // Standard Lehmer code: for each position, count how many later elements are smaller.
    const FACT: [u32; 8] = [5040, 720, 120, 24, 6, 2, 1, 1]; // 7! .. 0!
    let mut rank = 0u32;
    for i in 0..8 {
        let mut smaller = 0u32;
        for j in (i + 1)..8 {
            if p[j] < p[i] {
                smaller += 1;
            }
        }
        rank += smaller * FACT[i];
    }
    rank
}

/// Inverse of [`perm_rank8`]: factorial-base unrank to a permutation of `0..8`.
pub(crate) fn perm_unrank8(rank: u32) -> [u8; 8] {
    const FACT: [u32; 8] = [5040, 720, 120, 24, 6, 2, 1, 1]; // 7! .. 0!
    let mut avail: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
    let mut len = 8usize;
    let mut rank = rank;
    let mut out = [0u8; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        let f = FACT[i];
        let idx = (rank / f) as usize;
        rank %= f;
        *slot = avail[idx];
        // remove avail[idx], preserving order of the rest
        for k in idx..(len - 1) {
            avail[k] = avail[k + 1];
        }
        len -= 1;
    }
    out
}

/// Rank of a corner-orientation vector (`co ∈ 0..3` per corner). Only the first 7 are
/// free; the 8th is fixed by the parity `Σ co ≡ 0 (mod 3)`. Range `0..2187`.
pub(crate) fn corner_ori_rank(co: &[u8; 8]) -> u16 {
    let mut rank = 0u16;
    for &c in co.iter().take(7) {
        rank = rank * 3 + c as u16;
    }
    rank
}

/// Inverse of [`corner_ori_rank`]: reconstructs all 8 entries, the 8th chosen so the
/// total orientation is `≡ 0 (mod 3)`.
pub(crate) fn corner_ori_unrank(rank: u16) -> [u8; 8] {
    let mut out = [0u8; 8];
    let mut rank = rank;
    let mut sum = 0u16;
    for slot in out.iter_mut().take(7).rev() {
        let d = (rank % 3) as u8;
        *slot = d;
        rank /= 3;
        sum += d as u16;
    }
    out[7] = ((3 - (sum % 3)) % 3) as u8;
    out
}

/// Rank of an ordered injection `[6] -> [12]` (`slots[j] ∈ 0..12`, all distinct), i.e. a
/// "permutation of a 6-subset" of 12. Falling-factorial / Lehmer over the remaining pool.
/// Range `0..665280` (= 12·11·10·9·8·7).
pub(crate) fn partial_perm_rank(slots: &[u8; 6]) -> u32 {
    let mut rank = 0u32;
    for (i, &s) in slots.iter().enumerate() {
        // count how many slots strictly less than `s` are still unused (i.e. not chosen
        // by an earlier position)
        let mut smaller = s as u32;
        for &prev in &slots[..i] {
            if prev < s {
                smaller -= 1;
            }
        }
        // base = 12 - i remaining items; multiply running rank then add
        let base = (12 - i) as u32;
        rank = rank * base + smaller;
    }
    rank
}

/// Inverse of [`partial_perm_rank`].
pub(crate) fn partial_perm_unrank(rank: u32) -> [u8; 6] {
    // Decode digits from the last position back to the first (mixed radix 7,8,9,10,11,12).
    let mut digits = [0u32; 6];
    let mut rank = rank;
    for i in (0..6).rev() {
        let base = (12 - i) as u32;
        digits[i] = rank % base;
        rank /= base;
    }
    // Map each "kth still-available" digit to an actual 0..12 value.
    let mut avail: Vec<u8> = (0..12).collect();
    let mut out = [0u8; 6];
    for i in 0..6 {
        let idx = digits[i] as usize;
        out[i] = avail[idx];
        avail.remove(idx);
    }
    out
}

/// Rank of a 6-bit edge-orientation vector (`bits ∈ 0..2`). Range `0..64`.
pub(crate) fn edge_ori_rank(bits: &[u8; 6]) -> u8 {
    let mut rank = 0u8;
    for &b in bits.iter() {
        rank = (rank << 1) | (b & 1);
    }
    rank
}

/// Inverse of [`edge_ori_rank`].
pub(crate) fn edge_ori_unrank(rank: u8) -> [u8; 6] {
    let mut out = [0u8; 6];
    for (j, slot) in out.iter_mut().enumerate() {
        *slot = (rank >> (5 - j)) & 1;
    }
    out
}

// --- Generic nibble-packed BFS-by-sweep pattern-DB generator. ---

/// Read the nibble at index `idx` from a packed blob.
pub(crate) fn get_nibble(blob: &[u8], idx: u32) -> u8 {
    let byte = blob[(idx >> 1) as usize];
    if idx & 1 == 0 {
        byte & 0x0F
    } else {
        byte >> 4
    }
}

/// Write the 4-bit value `val` at index `idx` into a packed blob.
pub(crate) fn set_nibble(blob: &mut [u8], idx: u32, val: u8) {
    let bi = (idx >> 1) as usize;
    let byte = blob[bi];
    if idx & 1 == 0 {
        blob[bi] = (byte & 0xF0) | (val & 0x0F);
    } else {
        blob[bi] = (byte & 0x0F) | (val << 4);
    }
}

/// Build a nibble-packed distance table of `size` entries by BFS from `solved_index`.
///
/// `neighbors(idx, out)` must push the up-to-18 successor indices of `idx` into `out`
/// (the closure is responsible for clearing/using `out` as a scratch buffer; we clear
/// it before each call). Unvisited entries are left at the sentinel `0xF`; real
/// distances are `< 15`. Asserting that no `0xF` remains is the caller's job (some index
/// spaces here are not fully reachable — e.g. orientation-only coordinates are, but
/// callers must know their own space).
pub(crate) fn build_pdb(
    size: usize,
    solved_index: u32,
    neighbors: impl Fn(u32, &mut Vec<u32>),
) -> Vec<u8> {
    let mut blob = vec![0xFFu8; size.div_ceil(2)];
    set_nibble(&mut blob, solved_index, 0);

    let mut scratch: Vec<u32> = Vec::with_capacity(18);
    let mut depth: u8 = 0;
    loop {
        let mut wrote = 0u64;
        for idx in 0..size as u32 {
            if get_nibble(&blob, idx) != depth {
                continue;
            }
            scratch.clear();
            neighbors(idx, &mut scratch);
            for &succ in &scratch {
                if get_nibble(&blob, succ) == 0xF {
                    set_nibble(&mut blob, succ, depth + 1);
                    wrote += 1;
                }
            }
        }
        if wrote == 0 {
            break;
        }
        depth += 1;
    }
    blob
}

#[cfg(test)]
mod tests {
    use super::*;
    use kewb::{CubieCube, Move as KMove};

    /// kewb `Move` equivalent of our [`MOVE_CUBES`] index (test cross-check only).
    fn kewb_move(idx: usize) -> KMove {
        use KMove::*;
        // Our order: per face Cw, Ccw, Double; faces U D L R F B.
        const KS: [KMove; 18] = [
            U, U3, U2, // U  U'  U2
            D, D3, D2, // D  D'  D2
            L, L3, L2, // L  L'  L2
            R, R3, R2, // R  R'  R2
            F, F3, F2, // F  F'  F2
            B, B3, B2, // B  B'  B2
        ];
        KS[idx]
    }

    fn kewb_to_cubies(c: &CubieCube) -> Cubies {
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

    // --- 1. Ranking bijections (exhaustive). ---

    #[test]
    fn perm8_roundtrip_and_bijection() {
        let mut seen = vec![false; 40320];
        for rank in 0..40320u32 {
            let p = perm_unrank8(rank);
            // valid permutation of 0..8
            let mut bits = 0u32;
            for &v in &p {
                assert!(v < 8);
                bits |= 1 << v;
            }
            assert_eq!(bits, 0xFF, "unrank({rank}) is not a permutation: {p:?}");
            let back = perm_rank8(&p);
            assert_eq!(back, rank, "perm8 roundtrip failed at {rank}");
            assert!(!seen[rank as usize], "rank {rank} produced twice");
            seen[rank as usize] = true;
        }
        assert!(seen.into_iter().all(|b| b), "perm8 not a bijection");
    }

    #[test]
    fn corner_ori_roundtrip_and_parity() {
        let mut seen = vec![false; 2187];
        for rank in 0..2187u16 {
            let co = corner_ori_unrank(rank);
            for &v in &co {
                assert!(v < 3);
            }
            let sum: u16 = co.iter().map(|&v| v as u16).sum();
            assert_eq!(sum % 3, 0, "unrank({rank}) violates parity: {co:?}");
            let back = corner_ori_rank(&co);
            assert_eq!(back, rank, "corner_ori roundtrip failed at {rank}");
            assert!(!seen[rank as usize]);
            seen[rank as usize] = true;
        }
        assert!(seen.into_iter().all(|b| b));
    }

    #[test]
    fn partial_perm_roundtrip_and_bijection() {
        const N: u32 = 665280; // 12*11*10*9*8*7
        let mut seen = vec![false; N as usize];
        for rank in 0..N {
            let slots = partial_perm_unrank(rank);
            // injection: all distinct, all < 12
            for a in 0..6 {
                assert!(slots[a] < 12);
                for b in (a + 1)..6 {
                    assert_ne!(
                        slots[a], slots[b],
                        "unrank({rank}) not injective: {slots:?}"
                    );
                }
            }
            let back = partial_perm_rank(&slots);
            assert_eq!(
                back, rank,
                "partial_perm roundtrip failed at {rank}: {slots:?}"
            );
            assert!(!seen[rank as usize]);
            seen[rank as usize] = true;
        }
        assert!(seen.into_iter().all(|b| b), "partial_perm not a bijection");
    }

    #[test]
    fn edge_ori_roundtrip_and_bijection() {
        let mut seen = vec![false; 64];
        for rank in 0..64u8 {
            let bits = edge_ori_unrank(rank);
            for &b in &bits {
                assert!(b < 2);
            }
            let back = edge_ori_rank(&bits);
            assert_eq!(back, rank, "edge_ori roundtrip failed at {rank}");
            assert!(!seen[rank as usize]);
            seen[rank as usize] = true;
        }
        assert!(seen.into_iter().all(|b| b));
    }

    // --- 2. Compose matches kewb EXACTLY (the parity guard). ---

    #[test]
    fn compose_matches_kewb_single_moves() {
        for i in 0..18usize {
            let ours = apply(&SOLVED, i);
            let theirs = kewb_to_cubies(&CubieCube::default().apply_move(kewb_move(i)));
            assert_eq!(
                ours,
                theirs,
                "single move {i} ({:?}) diverges",
                kewb_move(i)
            );
        }
    }

    #[test]
    fn compose_matches_kewb_random_sequences() {
        // Tiny deterministic LCG (Numerical Recipes), no `rand` crate.
        let mut seed: u32 = 0xDEAD_BEEF;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };

        for _ in 0..50 {
            let mut ours = SOLVED;
            let mut theirs = CubieCube::default();
            let len = 5 + (next() as usize % 26); // 5..=30 moves
            for _ in 0..len {
                let idx = (next() as usize) % 18;
                ours = apply(&ours, idx);
                theirs = theirs.apply_move(kewb_move(idx));
            }
            let theirs = kewb_to_cubies(&theirs);
            assert_eq!(ours.cp, theirs.cp, "cp diverged");
            assert_eq!(ours.co, theirs.co, "co diverged");
            assert_eq!(ours.ep, theirs.ep, "ep diverged");
            assert_eq!(ours.eo, theirs.eo, "eo diverged");
        }
    }

    #[test]
    fn move_index_mapping_is_consistent() {
        for i in 0..18usize {
            assert_eq!(move_to_index(index_to_move(i)), i);
            assert_eq!(index_to_move(i), Move::ALL[i]);
        }
    }

    // --- 3. Corner-permutation move tables are full permutations of their index space. ---

    #[test]
    fn corner_perm_move_tables_are_permutations() {
        // For each move, applying the move's corner permutation to every cp and
        // re-ranking must hit each of the 40320 ranks exactly once.
        for (mv, move_cube) in MOVE_CUBES.iter().enumerate() {
            let move_cp = move_cube.cp;
            let mut visited = vec![false; 40320];
            for rank in 0..40320u32 {
                let cp = perm_unrank8(rank);
                // apply the move's corner permutation: res.cp[i] = cp[move_cp[i]]
                let mut res = [0u8; 8];
                for i in 0..8 {
                    res[i] = cp[move_cp[i] as usize];
                }
                let new_rank = perm_rank8(&res);
                assert!(
                    !visited[new_rank as usize],
                    "move {mv}: rank {new_rank} hit twice (not a permutation)"
                );
                visited[new_rank as usize] = true;
            }
            assert!(
                visited.into_iter().all(|b| b),
                "move {mv}: corner-perm table is not a full permutation"
            );
        }
    }

    // --- 4. BFS engine end-to-end on the corner-orientation-only space (size 2187). ---

    #[test]
    fn build_pdb_corner_orientation_matches_brute_bfs() {
        // PDB via build_pdb: index space is the corner-orientation rank.
        let pdb = build_pdb(2187, corner_ori_rank(&SOLVED.co) as u32, |idx, out| {
            let co = corner_ori_unrank(idx as u16);
            let c = Cubies {
                cp: SOLVED.cp,
                co,
                ep: SOLVED.ep,
                eo: SOLVED.eo,
            };
            for mv in 0..18usize {
                let next = apply(&c, mv);
                out.push(corner_ori_rank(&next.co) as u32);
            }
        });

        // No sentinel left; solved == 0.
        for idx in 0..2187u32 {
            assert_ne!(get_nibble(&pdb, idx), 0xF, "0xF left at idx {idx}");
        }
        assert_eq!(get_nibble(&pdb, corner_ori_rank(&SOLVED.co) as u32), 0);

        // Independent brute-force BFS directly in Cubies space, dedup by corner-ori rank.
        let mut dist = vec![u8::MAX; 2187];
        let start = corner_ori_rank(&SOLVED.co) as usize;
        dist[start] = 0;
        let mut frontier = vec![SOLVED];
        let mut depth = 0u8;
        while !frontier.is_empty() {
            let mut next_frontier = Vec::new();
            for c in &frontier {
                for mv in 0..18usize {
                    let nc = apply(c, mv);
                    let r = corner_ori_rank(&nc.co) as usize;
                    if dist[r] == u8::MAX {
                        dist[r] = depth + 1;
                        next_frontier.push(nc);
                    }
                }
            }
            frontier = next_frontier;
            depth += 1;
        }

        // Every entry agrees; max distances match.
        let mut max_pdb = 0u8;
        let mut max_brute = 0u8;
        for (idx, &b) in dist.iter().enumerate() {
            let p = get_nibble(&pdb, idx as u32);
            assert_ne!(b, u8::MAX, "brute BFS left {idx} unreached");
            assert_eq!(p, b, "distance mismatch at idx {idx}: pdb={p} brute={b}");
            max_pdb = max_pdb.max(p);
            max_brute = max_brute.max(b);
        }
        assert_eq!(max_pdb, max_brute, "max distance mismatch");
    }
}
