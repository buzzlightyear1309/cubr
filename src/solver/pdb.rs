//! The three Korf pattern databases (one corner PDB + two 6-edge PDBs) and the
//! max-heuristic that combines them. Pure, no Bevy.
//!
//! Nothing here is wired into the live solver yet (Unit K2 — generation, heuristic,
//! and the cache primitives in [`crate::solver::cache`]). The existing kewb two-phase
//! `solve` keeps working; allow `dead_code` at the module level so the `-D warnings`
//! gate stays green until Unit K4 consumes this.
//!
//! ## Index formulas
//! - **Corner** (`CORNER_SIZE = 40320·2187`): `perm_rank8(cp)·2187 + corner_ori_rank(co)`.
//! - **Edge, per group of 6** (`EDGE_SIZE = 665280·64`): rank only the six group
//!   members as an ordered injection `[6]→[12]` over their *slots*, and read each
//!   member's orientation at *its slot*. The other six edges are indistinguishable
//!   blanks and never enter the index: `partial_perm_rank(slots)·64 + edge_ori_rank(oris)`.
//!
//! ## Edge orientation transition (used by the fast neighbor closure)
//! kewb's compose puts the content of source-slot `b.ep[i]` into result-slot `i`,
//! adding `b.eo[i]` to its orientation. So for move `mv`: the edge sitting in slot `s`
//! lands in result-slot `t` where `MOVE_CUBES[mv].ep[t] == s` (call this `inv_ep[mv][s]`),
//! and its new orientation is `(old + MOVE_CUBES[mv].eo[t]) % 2`. The neighbor-consistency
//! test proves this matches the ground-truth full-cube `apply`.

// See note above: exercised by tests but not yet wired into the live solver.
#![allow(dead_code)]

use super::coords::{
    apply, corner_ori_rank, corner_ori_unrank, edge_ori_rank, edge_ori_unrank, get_nibble,
    partial_perm_rank, partial_perm_unrank, perm_rank8, perm_unrank8, Cubies, MOVE_CUBES, SOLVED,
};

/// Corner PDB size: `40320` corner permutations × `2187` corner orientations.
pub(crate) const CORNER_SIZE: usize = 88_179_840;
/// Per-group edge PDB size: `665280` ordered 6-of-12 placements × `64` orientations.
pub(crate) const EDGE_SIZE: usize = 42_577_920;

/// Edge group A: ids BL BR FR FL UB UR.
const GROUP_A: [u8; 6] = [0, 1, 2, 3, 4, 5];
/// Edge group B: ids UF UL DF DR DB DL.
const GROUP_B: [u8; 6] = [6, 7, 8, 9, 10, 11];

// --- Index functions (Cubies -> PDB index) ---

/// Corner PDB index of `c`.
pub(crate) fn corner_index(c: &Cubies) -> u32 {
    perm_rank8(&c.cp) * 2187 + corner_ori_rank(&c.co) as u32
}

/// Edge PDB index of `c` for the given group of six edge ids.
///
/// Builds the inverse edge map `inv[ep[p]] = p` (the slot currently holding each edge
/// id), then for each group member `group[j]` reads its slot and the orientation *at
/// that slot*. Only the six members are ranked.
fn edge_index(c: &Cubies, group: &[u8; 6]) -> u32 {
    // inv[id] = slot p such that ep[p] == id.
    let mut inv = [0u8; 12];
    for (p, &id) in c.ep.iter().enumerate() {
        inv[id as usize] = p as u8;
    }
    let mut slots = [0u8; 6];
    let mut oris = [0u8; 6];
    for j in 0..6 {
        let slot = inv[group[j] as usize];
        slots[j] = slot;
        oris[j] = c.eo[slot as usize];
    }
    partial_perm_rank(&slots) * 64 + edge_ori_rank(&oris) as u32
}

/// Edge PDB index for group A (ids 0..6).
pub(crate) fn edge_index_a(c: &Cubies) -> u32 {
    edge_index(c, &GROUP_A)
}

/// Edge PDB index for group B (ids 6..12).
pub(crate) fn edge_index_b(c: &Cubies) -> u32 {
    edge_index(c, &GROUP_B)
}

// --- Fast neighbour transitions (validated by the consistency test) ---

/// Per-move corner sub-coordinate transition tables, built once.
struct CornerMoveTables {
    /// `perm[mv][rank]` = new corner-permutation rank after move `mv`.
    perm: Vec<[u16; 18]>,
    /// `ori[mv][rank]` = new corner-orientation rank after move `mv`.
    ori: Vec<[u16; 18]>,
}

impl CornerMoveTables {
    fn build() -> CornerMoveTables {
        // Corner permutation transitions over all 40320 ranks.
        let mut perm = vec![[0u16; 18]; 40320];
        for (rank, row) in perm.iter_mut().enumerate() {
            let cp = perm_unrank8(rank as u32);
            for (mv, slot) in row.iter_mut().enumerate() {
                let move_cp = MOVE_CUBES[mv].cp;
                let mut res = [0u8; 8];
                for i in 0..8 {
                    res[i] = cp[move_cp[i] as usize];
                }
                *slot = perm_rank8(&res) as u16;
            }
        }
        // Corner orientation transitions over all 2187 ranks (identity permutation,
        // solved edges — orientation evolves independently of permutation).
        let mut ori = vec![[0u16; 18]; 2187];
        for (rank, row) in ori.iter_mut().enumerate() {
            let co = corner_ori_unrank(rank as u16);
            let c = Cubies {
                cp: SOLVED.cp,
                co,
                ep: SOLVED.ep,
                eo: SOLVED.eo,
            };
            for (mv, slot) in row.iter_mut().enumerate() {
                let next = apply(&c, mv);
                *slot = corner_ori_rank(&next.co);
            }
        }
        CornerMoveTables { perm, ori }
    }

    /// Corner-index neighbour: `idx` factorises as `perm*2187 + ori`, each evolving
    /// independently.
    fn neighbor(&self, idx: u32, mv: usize) -> u32 {
        let perm = (idx / 2187) as usize;
        let ori = (idx % 2187) as usize;
        self.perm[perm][mv] as u32 * 2187 + self.ori[ori][mv] as u32
    }
}

/// Per-move edge transition data (shared by both groups — the slot/orientation
/// transition is group-independent; only the index formula differs, and that is the
/// same `partial_perm`/`edge_ori` rank applied to the moved slots).
struct EdgeMoveTables {
    /// `inv_ep[mv][s]` = result-slot `t` with `MOVE_CUBES[mv].ep[t] == s` (where the
    /// content of source-slot `s` lands under move `mv`).
    inv_ep: [[u8; 12]; 18],
    /// `flip[mv]` = `MOVE_CUBES[mv].eo` (orientation delta added at each result-slot).
    flip: [[u8; 12]; 18],
}

impl EdgeMoveTables {
    fn build() -> EdgeMoveTables {
        let mut inv_ep = [[0u8; 12]; 18];
        let mut flip = [[0u8; 12]; 18];
        for mv in 0..18 {
            let ep = MOVE_CUBES[mv].ep;
            for (t, &s) in ep.iter().enumerate() {
                inv_ep[mv][s as usize] = t as u8;
            }
            flip[mv] = MOVE_CUBES[mv].eo;
        }
        EdgeMoveTables { inv_ep, flip }
    }

    /// Edge-index neighbour for either group: `idx` factorises as `pos*64 + ori`.
    fn neighbor(&self, idx: u32, mv: usize) -> u32 {
        let pos = idx / 64;
        let ori = (idx % 64) as u8;
        let slots = partial_perm_unrank(pos);
        let oris = edge_ori_unrank(ori);
        let mut new_slots = [0u8; 6];
        let mut new_oris = [0u8; 6];
        for j in 0..6 {
            let t = self.inv_ep[mv][slots[j] as usize];
            new_slots[j] = t;
            new_oris[j] = (oris[j] + self.flip[mv][t as usize]) % 2;
        }
        partial_perm_rank(&new_slots) * 64 + edge_ori_rank(&new_oris) as u32
    }
}

// --- The three PDBs + heuristic ---

/// The three Korf pattern databases, nibble-packed (`0xF` sentinel removed after a
/// complete build). `Vec<u8>` only, so `Pdbs` is trivially `Send + Sync`.
pub(crate) struct Pdbs {
    pub corner: Vec<u8>,
    pub edge_a: Vec<u8>,
    pub edge_b: Vec<u8>,
}

impl Pdbs {
    /// Build all three databases from scratch. SLOW (~1–3 min single-threaded; the
    /// corner PDB is the long pole). The caller caches the result to disk (Unit K4).
    pub(crate) fn generate() -> Pdbs {
        use super::coords::build_pdb;

        let corner_tabs = CornerMoveTables::build();
        let edge_tabs = EdgeMoveTables::build();

        let corner = build_pdb(CORNER_SIZE, corner_index(&SOLVED), |idx, out| {
            for mv in 0..18usize {
                out.push(corner_tabs.neighbor(idx, mv));
            }
        });
        let edge_a = build_pdb(EDGE_SIZE, edge_index_a(&SOLVED), |idx, out| {
            for mv in 0..18usize {
                out.push(edge_tabs.neighbor(idx, mv));
            }
        });
        let edge_b = build_pdb(EDGE_SIZE, edge_index_b(&SOLVED), |idx, out| {
            for mv in 0..18usize {
                out.push(edge_tabs.neighbor(idx, mv));
            }
        });

        Pdbs {
            corner,
            edge_a,
            edge_b,
        }
    }

    /// Admissible heuristic: the max of the three PDB lower bounds for `c`.
    pub(crate) fn h(&self, c: &Cubies) -> u8 {
        let hc = get_nibble(&self.corner, corner_index(c));
        let ha = get_nibble(&self.edge_a, edge_index_a(c));
        let hb = get_nibble(&self.edge_b, edge_index_b(c));
        hc.max(ha).max(hb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver::coords::build_pdb;

    /// Tiny deterministic LCG (Numerical Recipes), no `rand` crate.
    fn lcg(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *seed
    }

    /// Apply a deterministic random reachable sequence to SOLVED.
    fn random_reachable(seed: &mut u32, len: usize) -> Cubies {
        let mut c = SOLVED;
        for _ in 0..len {
            c = apply(&c, (lcg(seed) as usize) % 18);
        }
        c
    }

    // --- 1. Solved indices. ---

    #[test]
    fn solved_indices() {
        assert_eq!(corner_index(&SOLVED), 0, "corner solved index must be 0");
        assert_eq!(edge_index_a(&SOLVED), 0, "edge A solved index must be 0");
        // Group B members sit at slots 6..11 (nonzero rank); just confirm it is stable.
        let b1 = edge_index_b(&SOLVED);
        let b2 = edge_index_b(&SOLVED);
        assert_eq!(b1, b2, "edge B solved index must round-trip");
        assert_ne!(b1, 0, "edge B solved index is expected to be nonzero");
        assert!((b1 as usize) < EDGE_SIZE);
    }

    // --- 2. Index in range over random reachable states. ---

    #[test]
    fn indices_in_range() {
        let mut seed = 0xC0FF_EE00u32;
        for _ in 0..10_000 {
            let len = 1 + (lcg(&mut seed) as usize % 30);
            let c = random_reachable(&mut seed, len);
            assert!(
                (corner_index(&c) as usize) < CORNER_SIZE,
                "corner index out of range"
            );
            assert!(
                (edge_index_a(&c) as usize) < EDGE_SIZE,
                "edge A index out of range"
            );
            assert!(
                (edge_index_b(&c) as usize) < EDGE_SIZE,
                "edge B index out of range"
            );
        }
    }

    // --- 3. THE guard: fast neighbour funcs equal the ground-truth full-cube transition. ---

    #[test]
    fn neighbor_consistency() {
        let corner_tabs = CornerMoveTables::build();
        let edge_tabs = EdgeMoveTables::build();

        let mut seed = 0x1357_9BDFu32;
        for _ in 0..5_000 {
            let len = 1 + (lcg(&mut seed) as usize % 30);
            let c = random_reachable(&mut seed, len);
            let ci = corner_index(&c);
            let ai = edge_index_a(&c);
            let bi = edge_index_b(&c);
            for mv in 0..18usize {
                let moved = apply(&c, mv);
                assert_eq!(
                    corner_tabs.neighbor(ci, mv),
                    corner_index(&moved),
                    "corner neighbour mismatch (mv={mv})"
                );
                assert_eq!(
                    edge_tabs.neighbor(ai, mv),
                    edge_index_a(&moved),
                    "edge A neighbour mismatch (mv={mv})"
                );
                assert_eq!(
                    edge_tabs.neighbor(bi, mv),
                    edge_index_b(&moved),
                    "edge B neighbour mismatch (mv={mv})"
                );
            }
        }
    }

    // --- 4. Small real BFS via build_pdb cross-checked vs brute Cubies BFS. ---
    //
    // Build the corner-orientation-only PDB (2187 entries) using the fast corner
    // neighbour over a closure that holds ori fixed (perm always solved), and confirm it
    // agrees with a brute BFS in Cubies space. This exercises build_pdb + the corner
    // sub-coordinate transition together, sub-second.
    #[test]
    fn small_corner_ori_pdb_matches_brute() {
        let corner_tabs = CornerMoveTables::build();
        // Index space: corner-orientation rank (perm fixed solved -> idx = ori).
        let pdb = build_pdb(2187, corner_ori_rank(&SOLVED.co) as u32, |idx, out| {
            // idx is a pure ori rank; the fast table operates on ori independently.
            for mv in 0..18usize {
                out.push(corner_tabs.ori[idx as usize][mv] as u32);
            }
        });
        for idx in 0..2187u32 {
            assert_ne!(get_nibble(&pdb, idx), 0xF, "0xF left at {idx}");
        }

        // Brute BFS over Cubies, dedup by corner-ori rank.
        let mut dist = vec![u8::MAX; 2187];
        let start = corner_ori_rank(&SOLVED.co) as usize;
        dist[start] = 0;
        let mut frontier = vec![SOLVED];
        let mut depth = 0u8;
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for c in &frontier {
                for mv in 0..18usize {
                    let nc = apply(c, mv);
                    let r = corner_ori_rank(&nc.co) as usize;
                    if dist[r] == u8::MAX {
                        dist[r] = depth + 1;
                        next.push(nc);
                    }
                }
            }
            frontier = next;
            depth += 1;
        }
        for (idx, &b) in dist.iter().enumerate() {
            assert_ne!(b, u8::MAX, "brute left {idx} unreached");
            assert_eq!(
                get_nibble(&pdb, idx as u32),
                b,
                "distance mismatch at {idx}"
            );
        }
    }

    // --- 5. Full corner PDB (ignored: tens of seconds). ---

    #[test]
    #[ignore = "full corner PDB build is slow (tens of seconds)"]
    fn full_corner_pdb_build() {
        let corner_tabs = CornerMoveTables::build();
        let p = build_pdb(CORNER_SIZE, corner_index(&SOLVED), |idx, out| {
            for mv in 0..18usize {
                out.push(corner_tabs.neighbor(idx, mv));
            }
        });
        let mut max = 0u8;
        for idx in 0..CORNER_SIZE as u32 {
            let v = get_nibble(&p, idx);
            assert_ne!(
                v, 0xF,
                "0xF left at corner idx {idx} (space not fully reachable)"
            );
            max = max.max(v);
        }
        assert_eq!(get_nibble(&p, 0), 0, "corner solved entry must be 0");
        assert_eq!(max, 11, "corner PDB max distance must be 11");
    }

    // --- Optional: full edge-A PDB (ignored). ---

    #[test]
    #[ignore = "full edge PDB build is slow (tens of seconds)"]
    fn full_edge_a_pdb_build() {
        let edge_tabs = EdgeMoveTables::build();
        let p = build_pdb(EDGE_SIZE, edge_index_a(&SOLVED), |idx, out| {
            for mv in 0..18usize {
                out.push(edge_tabs.neighbor(idx, mv));
            }
        });
        for idx in 0..EDGE_SIZE as u32 {
            assert_ne!(get_nibble(&p, idx), 0xF, "0xF left at edge-A idx {idx}");
        }
        assert_eq!(get_nibble(&p, 0), 0, "edge-A solved entry must be 0");
    }
}
