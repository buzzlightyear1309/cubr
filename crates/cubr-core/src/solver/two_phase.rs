//! Self-contained Kociemba **two-phase** (near-optimal) solver. Pure, no Bevy.
//!
//! Phase 1 reduces the cube to the subgroup G1 = ⟨U, D, L2, R2, F2, B2⟩ — every edge
//! correctly oriented, every corner correctly oriented, and the four E-slice edges in the
//! E-slice — then phase 2 solves within G1 using only those 10 generators. The whole
//! thing is built against our own [`super::coords`] cube model and ranking primitives;
//! the move-transition and IDA*-pruning tables are generated in-memory by
//! [`TwoPhaseTables::build`] (no disk format, never serialised).
//!
//! ## Why this exists / what it fixes
//! The algorithm *structure* is the classic two-phase referenced from kewb's `two_phase/`
//! (MIT), but this is a fresh implementation against our model with kewb's traps fixed:
//!
//! - **Full, correct 18-move set in phase 1.** kewb's transcribed move list had a
//!   duplicated `F2`/dropped move; here phase 1 iterates all of `0..18` and phase 2 uses
//!   the exact 10 G1 generators ([`PHASE2_MOVES`]), both proven by tests.
//! - **Iterated for the shortest solution, not the first.** A naive two-phase returns the
//!   first phase-1 endpoint that completes; we keep deepening phase 1 (and, for each
//!   endpoint, take the *first* — hence shortest — phase-2 length) and keep the global
//!   best, so the result is the best over many phase-1 lengths rather than the first hit.
//! - **No "timeout returns None" bug.** The deadline only bites *after* a first solution
//!   exists, so a solvable cube always yields a solution unless externally cancelled.
//!
//! This module is not yet wired into the public [`super::solve`] (a later unit does that),
//! so its public surface carries `#[cfg_attr(not(test), allow(dead_code))]` — the same
//! pattern used on [`super::coords::move_to_index`] / [`super::search::ida_star`].

use super::coords::{
    apply, corner_ori_rank, corner_ori_unrank, e_ep_rank, e_ep_unrank, eo12_rank, eo12_unrank,
    eslice_combo_rank, eslice_combo_unrank, perm_rank8, perm_unrank8, ud_ep_rank, ud_ep_unrank,
    Cubies, SOLVED,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The 10 G1 generators ⟨U, D, L2, R2, F2, B2⟩ as indices into
/// [`MOVE_CUBES`](super::coords) / [`Move::ALL`](crate::model::Move::ALL).
/// Order: `U U' U2  D D' D2  L2 R2 F2 B2`.
const PHASE2_MOVES: [usize; 10] = [0, 1, 2, 3, 4, 5, 8, 11, 14, 17];

/// Corner-orientation coordinate count.
const CO: usize = 2187;
/// Edge-orientation (all 12) coordinate count.
const EO: usize = 2048;
/// E-slice combination coordinate count (C(12,4)).
const ESLICE: usize = 495;
/// Corner-permutation coordinate count (8!).
const CP: usize = 40320;
/// UD-edge permutation coordinate count (8!).
const UDEP: usize = 40320;
/// E-slice edge permutation coordinate count (4!).
const EEP: usize = 24;

/// Hard cap on the solution length we will ever accept / search for.
const MAX_SOLUTION_LEN: usize = 30;

/// How often (in expanded nodes) the search polls the cancel flag / deadline.
const POLL_INTERVAL: u64 = 4096;

/// Default wall-clock budget (ms) once a first solution exists, overridable via
/// `CUBR_TWO_PHASE_BUDGET_MS`.
const DEFAULT_BUDGET_MS: u64 = 300;

/// Redundancy pruning: `true` if move `cur` should be skipped given the previous move
/// `prev` (`None` at the root). Identical rule to [`super::search`]:
///
/// 1. no two consecutive moves on the same face; and
/// 2. for commuting opposite faces on an axis, only the canonical (lower-then-higher)
///    ordering.
///
/// `prev`/`cur` are always full 18-move indices (phase 2 passes `PHASE2_MOVES[j]`, which
/// are a subset of those indices), so the same face/axis arithmetic applies to both
/// phases.
fn redundant(prev: Option<usize>, cur: usize) -> bool {
    let Some(p) = prev else { return false };
    let (pf, cf) = (p / 3, cur / 3);
    if pf == cf {
        return true; // same face
    }
    if pf / 2 == cf / 2 && pf > cf {
        return true; // commuting opposite faces: canonical order only
    }
    false
}

/// In-memory move-transition + pruning tables for the two-phase search. Built once via
/// [`TwoPhaseTables::build`] and shared (immutably) by every solve; never serialised.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TwoPhaseTables {
    // --- Phase-1 move transitions (18-wide). ---
    /// `co_move[co][mv]` = new corner-orientation rank after move `mv`.
    co_move: Vec<[u16; 18]>,
    /// `eo_move[eo][mv]` = new edge-orientation (all 12) rank after move `mv`.
    eo_move: Vec<[u16; 18]>,
    /// `es_move[es][mv]` = new E-slice combination rank after move `mv`.
    es_move: Vec<[u16; 18]>,
    // --- Phase-2 move transitions (10-wide; index `j` selects `PHASE2_MOVES[j]`). ---
    /// `cp_move[cp][j]` = new corner-permutation rank after `PHASE2_MOVES[j]`.
    cp_move: Vec<[u16; 10]>,
    /// `udep_move[udep][j]` = new UD-edge permutation rank after `PHASE2_MOVES[j]`.
    udep_move: Vec<[u16; 10]>,
    /// `eep_move[eep][j]` = new E-slice edge permutation rank after `PHASE2_MOVES[j]`.
    eep_move: Vec<[u16; 10]>,
    // --- Pruning tables (flat `u8` distance from the combined solved index 0). ---
    /// `co_e[co*ESLICE + es]` = BFS distance (over all 18 moves) to `(0, 0)`.
    co_e: Vec<u8>,
    /// `eo_e[eo*ESLICE + es]` = BFS distance (over all 18 moves) to `(0, 0)`.
    eo_e: Vec<u8>,
    /// `cp_e[cp*EEP + eep]` = BFS distance (over the 10 G1 moves) to `(0, 0)`.
    cp_e: Vec<u8>,
    /// `udep_e[udep*EEP + eep]` = BFS distance (over the 10 G1 moves) to `(0, 0)`.
    udep_e: Vec<u8>,
}

impl TwoPhaseTables {
    /// Build all move-transition and pruning tables. Each move-transition row is built
    /// `unrank → set only the relevant field on a SOLVED clone → apply → re-rank` — valid
    /// because each coordinate's transition depends only on that coordinate's own field(s)
    /// and the move (proven by `move_tables_match_apply`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn build() -> TwoPhaseTables {
        // Phase-1: corner orientation (co only varies; everything else solved).
        let mut co_move = vec![[0u16; 18]; CO];
        for (i, row) in co_move.iter_mut().enumerate() {
            let c = Cubies {
                co: corner_ori_unrank(i as u16),
                ..SOLVED
            };
            for (mv, slot) in row.iter_mut().enumerate() {
                *slot = corner_ori_rank(&apply(&c, mv).co);
            }
        }

        // Phase-1: edge orientation over all 12 (eo only varies).
        let mut eo_move = vec![[0u16; 18]; EO];
        for (i, row) in eo_move.iter_mut().enumerate() {
            let c = Cubies {
                eo: eo12_unrank(i as u16),
                ..SOLVED
            };
            for (mv, slot) in row.iter_mut().enumerate() {
                *slot = eo12_rank(&apply(&c, mv).eo);
            }
        }

        // Phase-1: E-slice combination (ep set so exactly the 4 E-slice edges are placed).
        let mut es_move = vec![[0u16; 18]; ESLICE];
        for (i, row) in es_move.iter_mut().enumerate() {
            let c = Cubies {
                ep: eslice_combo_unrank(i as u16),
                ..SOLVED
            };
            for (mv, slot) in row.iter_mut().enumerate() {
                *slot = eslice_combo_rank(&apply(&c, mv).ep);
            }
        }

        // Phase-2: corner permutation (cp only varies) under the 10 G1 generators.
        let mut cp_move = vec![[0u16; 10]; CP];
        for (i, row) in cp_move.iter_mut().enumerate() {
            let c = Cubies {
                cp: perm_unrank8(i as u32),
                ..SOLVED
            };
            for (j, slot) in row.iter_mut().enumerate() {
                *slot = perm_rank8(&apply(&c, PHASE2_MOVES[j]).cp) as u16;
            }
        }

        // Phase-2: UD-edge permutation (the 8 U/D edges within their slots).
        let mut udep_move = vec![[0u16; 10]; UDEP];
        for (i, row) in udep_move.iter_mut().enumerate() {
            let c = Cubies {
                ep: ud_ep_unrank(i as u16),
                ..SOLVED
            };
            for (j, slot) in row.iter_mut().enumerate() {
                *slot = ud_ep_rank(&apply(&c, PHASE2_MOVES[j]).ep);
            }
        }

        // Phase-2: E-slice edge permutation (the 4 E-slice edges within their slots).
        let mut eep_move = vec![[0u16; 10]; EEP];
        for (i, row) in eep_move.iter_mut().enumerate() {
            let c = Cubies {
                ep: e_ep_unrank(i as u8),
                ..SOLVED
            };
            for (j, slot) in row.iter_mut().enumerate() {
                *slot = e_ep_rank(&apply(&c, PHASE2_MOVES[j]).ep) as u16;
            }
        }

        // Pruning tables (BFS from combined solved index 0).
        let co_e = build_prune18(CO, ESLICE, &co_move, &es_move);
        let eo_e = build_prune18(EO, ESLICE, &eo_move, &es_move);
        let cp_e = build_prune10(CP, EEP, &cp_move, &eep_move);
        let udep_e = build_prune10(UDEP, EEP, &udep_move, &eep_move);

        TwoPhaseTables {
            co_move,
            eo_move,
            es_move,
            cp_move,
            udep_move,
            eep_move,
            co_e,
            eo_e,
            cp_e,
            udep_e,
        }
    }

    /// Phase-1 admissible lower bound: the larger of the (corner-ori × E-slice) and
    /// (edge-ori × E-slice) pruning distances.
    #[inline]
    fn phase1_h(&self, co: usize, eo: usize, es: usize) -> u8 {
        self.co_e[co * ESLICE + es].max(self.eo_e[eo * ESLICE + es])
    }

    /// Phase-2 admissible lower bound: the larger of the (corner-perm × E-slice-perm) and
    /// (UD-edge-perm × E-slice-perm) pruning distances.
    #[inline]
    fn phase2_h(&self, cp: usize, udep: usize, eep: usize) -> u8 {
        self.cp_e[cp * EEP + eep].max(self.udep_e[udep * EEP + eep])
    }
}

/// BFS over the combined index space of two 18-wide-move coordinates, from the solved
/// combined index `0` (== `(0, 0)`). Returns the flat distance table
/// `dist[c1*count2 + c2]`. Panics if any combination is unreachable (the space must be
/// fully covered) or if `dist[0] != 0`.
fn build_prune18(
    count1: usize,
    count2: usize,
    move1: &[[u16; 18]],
    move2: &[[u16; 18]],
) -> Vec<u8> {
    let total = count1 * count2;
    let mut dist = vec![u8::MAX; total];
    dist[0] = 0;
    let mut queue: VecDeque<usize> = VecDeque::new();
    queue.push_back(0);
    while let Some(idx) = queue.pop_front() {
        let c1 = idx / count2;
        let c2 = idx % count2;
        let d = dist[idx];
        for m in 0..18 {
            let n1 = move1[c1][m] as usize;
            let n2 = move2[c2][m] as usize;
            let succ = n1 * count2 + n2;
            if dist[succ] == u8::MAX {
                dist[succ] = d + 1;
                queue.push_back(succ);
            }
        }
    }
    assert_eq!(dist[0], 0, "pruning BFS: solved index distance must be 0");
    assert!(
        dist.iter().all(|&d| d != u8::MAX),
        "pruning BFS: an index combination was unreachable"
    );
    dist
}

/// BFS over the combined index space of two 10-wide-move coordinates (the G1
/// generators), from the solved combined index `0`. Returns `dist[c1*count2 + c2]`. Same
/// reachability assertions as [`build_prune18`].
fn build_prune10(
    count1: usize,
    count2: usize,
    move1: &[[u16; 10]],
    move2: &[[u16; 10]],
) -> Vec<u8> {
    let total = count1 * count2;
    let mut dist = vec![u8::MAX; total];
    dist[0] = 0;
    let mut queue: VecDeque<usize> = VecDeque::new();
    queue.push_back(0);
    while let Some(idx) = queue.pop_front() {
        let c1 = idx / count2;
        let c2 = idx % count2;
        let d = dist[idx];
        for j in 0..10 {
            let n1 = move1[c1][j] as usize;
            let n2 = move2[c2][j] as usize;
            let succ = n1 * count2 + n2;
            if dist[succ] == u8::MAX {
                dist[succ] = d + 1;
                queue.push_back(succ);
            }
        }
    }
    assert_eq!(dist[0], 0, "pruning BFS: solved index distance must be 0");
    assert!(
        dist.iter().all(|&d| d != u8::MAX),
        "pruning BFS: an index combination was unreachable"
    );
    dist
}

/// Resolve the per-solve wall-clock budget. `CUBR_TWO_PHASE_BUDGET_MS`, if set to a
/// parseable value, overrides [`DEFAULT_BUDGET_MS`]; absent / unparseable falls back to
/// the default.
fn budget_ms() -> u64 {
    std::env::var("CUBR_TWO_PHASE_BUDGET_MS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_BUDGET_MS)
}

/// Mutable search state threaded through the recursive phase-1 / phase-2 walk.
struct Ctx<'a> {
    tables: &'a TwoPhaseTables,
    start: &'a Cubies,
    cancel: &'a AtomicBool,
    /// Best (shortest) full solution found so far.
    best: Option<Vec<usize>>,
    /// Current upper bound on total length we still try to beat.
    max_total: usize,
    /// Phase-1 move stack from the start to the current node.
    path: Vec<usize>,
    deadline: Instant,
    nodes: u64,
    cancelled: bool,
    deadlined: bool,
}

impl Ctx<'_> {
    /// Poll the cancel flag / deadline every [`POLL_INTERVAL`] nodes. Sets `cancelled`
    /// (always) or `deadlined` (only once a `best` exists — guaranteeing a solvable cube
    /// yields a solution unless externally cancelled). Returns `true` if the search must
    /// stop now.
    #[inline]
    fn should_stop(&mut self) -> bool {
        self.nodes += 1;
        if self.nodes.is_multiple_of(POLL_INTERVAL) {
            if self.cancel.load(Ordering::Relaxed) {
                self.cancelled = true;
            } else if self.best.is_some() && Instant::now() >= self.deadline {
                self.deadlined = true;
            }
        }
        self.cancelled || self.deadlined
    }

    /// Record `full` as the new best if it is shorter than the current best (or the first
    /// one), tightening `max_total`.
    fn update_best(&mut self, full: Vec<usize>) {
        if self.best.as_ref().is_none_or(|b| full.len() < b.len()) {
            self.max_total = full.len();
            self.best = Some(full);
        }
    }

    /// Phase-1 bounded DFS: drive `(co, eo, es)` to `(0, 0, 0)` (the cube in G1) in
    /// exactly `depth` moves, then hand the endpoint to phase 2.
    fn phase1(&mut self, co: usize, eo: usize, es: usize, depth: usize, prev: Option<usize>) {
        if self.should_stop() {
            return;
        }
        if depth == 0 {
            if co == 0 && eo == 0 && es == 0 {
                self.complete_phase2();
            }
            return;
        }
        if self.tables.phase1_h(co, eo, es) as usize > depth {
            return;
        }
        for mv in 0..18usize {
            if redundant(prev, mv) {
                continue;
            }
            let nco = self.tables.co_move[co][mv] as usize;
            let neo = self.tables.eo_move[eo][mv] as usize;
            let nes = self.tables.es_move[es][mv] as usize;
            self.path.push(mv);
            self.phase1(nco, neo, nes, depth - 1, Some(mv));
            self.path.pop();
            if self.cancelled || self.deadlined {
                return;
            }
        }
    }

    /// At a G1 endpoint (the phase-1 path is `self.path`): solve the phase-2 sub-cube and,
    /// if the resulting full solution beats `best`, record it.
    fn complete_phase2(&mut self) {
        // Replay the phase-1 path to get the cube in G1.
        let mut cube1 = *self.start;
        for &mv in &self.path {
            cube1 = apply(&cube1, mv);
        }
        let cp = perm_rank8(&cube1.cp) as usize;
        let udep = ud_ep_rank(&cube1.ep) as usize;
        let eep = e_ep_rank(&cube1.ep) as usize;

        if cp == 0 && udep == 0 && eep == 0 {
            // Phase 1 already solved the whole cube.
            let candidate = self.path.clone();
            self.update_best(candidate);
            return;
        }

        // Max phase-2 length that could still beat the current best.
        let remaining = match &self.best {
            Some(b) => b.len().saturating_sub(self.path.len()).saturating_sub(1),
            None => self.max_total - self.path.len(),
        };
        let lb = self.tables.phase2_h(cp, udep, eep) as usize;
        let prev = self.path.last().copied();
        let mut p2path: Vec<usize> = Vec::new();
        for d2 in lb..=remaining {
            if self.cancelled || self.deadlined {
                return;
            }
            p2path.clear();
            if self.phase2_dfs(cp, udep, eep, d2, prev, &mut p2path) {
                // First d2 that yields a solution is the optimal phase-2 for this endpoint.
                let mut full = self.path.clone();
                full.extend_from_slice(&p2path);
                self.update_best(full);
                break;
            }
        }
    }

    /// Phase-2 bounded DFS within G1: drive `(cp, udep, eep)` to `(0, 0, 0)` in exactly
    /// `depth` of the 10 G1 moves. Returns `true` (and leaves the move sequence in
    /// `p2path`) on success.
    fn phase2_dfs(
        &mut self,
        cp: usize,
        udep: usize,
        eep: usize,
        depth: usize,
        prev: Option<usize>,
        p2path: &mut Vec<usize>,
    ) -> bool {
        if self.should_stop() {
            return false;
        }
        if depth == 0 {
            return cp == 0 && udep == 0 && eep == 0;
        }
        if self.tables.phase2_h(cp, udep, eep) as usize > depth {
            return false;
        }
        for (j, &mv) in PHASE2_MOVES.iter().enumerate() {
            if redundant(prev, mv) {
                continue;
            }
            let ncp = self.tables.cp_move[cp][j] as usize;
            let nudep = self.tables.udep_move[udep][j] as usize;
            let neep = self.tables.eep_move[eep][j] as usize;
            p2path.push(mv);
            if self.phase2_dfs(ncp, nudep, neep, depth - 1, Some(mv), p2path) {
                return true;
            }
            p2path.pop();
            if self.cancelled || self.deadlined {
                return false;
            }
        }
        false
    }
}

/// Near-optimal two-phase solve. Returns the move-index path (indices `0..18` into
/// [`MOVE_CUBES`](super::coords) / [`Move::ALL`](crate::model::Move::ALL)), or `None` if
/// `cancel` was observed set before any solution was found. An already-solved cube
/// returns `Some(vec![])`. The returned path always solves `start` (re-applying it via
/// [`apply`] reaches [`SOLVED`]).
///
/// Iterated for the shortest solution: phase 1 is deepened from 0; each G1 endpoint takes
/// its *first* (shortest) phase-2 length; the global shortest is kept. A wall-clock budget
/// (`CUBR_TWO_PHASE_BUDGET_MS`, default 300 ms) bounds the search but only *after* a first
/// solution exists, so a solvable cube always returns `Some`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn solve(
    tables: &TwoPhaseTables,
    start: &Cubies,
    cancel: &AtomicBool,
) -> Option<Vec<usize>> {
    if *start == SOLVED {
        return Some(Vec::new());
    }

    let co0 = corner_ori_rank(&start.co) as usize;
    let eo0 = eo12_rank(&start.eo) as usize;
    let es0 = eslice_combo_rank(&start.ep) as usize;

    let mut ctx = Ctx {
        tables,
        start,
        cancel,
        best: None,
        max_total: MAX_SOLUTION_LEN,
        path: Vec::with_capacity(MAX_SOLUTION_LEN),
        deadline: Instant::now() + Duration::from_millis(budget_ms()),
        nodes: 0,
        cancelled: false,
        deadlined: false,
    };

    for d1 in 0..=MAX_SOLUTION_LEN {
        if ctx.cancelled || ctx.deadlined {
            break;
        }
        // A total of length ≥ d1 can no longer beat an existing best of length d1.
        if let Some(b) = &ctx.best {
            if d1 >= b.len() {
                break;
            }
        }
        ctx.phase1(co0, eo0, es0, d1, None);
    }

    if ctx.cancelled {
        return None;
    }
    ctx.best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny deterministic LCG (Numerical Recipes); no `rand` crate.
    fn lcg(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *seed
    }

    /// A fresh, never-set cancel flag.
    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// Apply a deterministic random reachable sequence (`len` moves) to SOLVED.
    fn random_reachable(seed: &mut u32, len: usize) -> Cubies {
        let mut c = SOLVED;
        for _ in 0..len {
            c = apply(&c, (lcg(seed) as usize) % 18);
        }
        c
    }

    /// Apply a deterministic random sequence of G1 generators (`len` moves) to SOLVED.
    fn random_g1(seed: &mut u32, len: usize) -> Cubies {
        let mut c = SOLVED;
        for _ in 0..len {
            c = apply(&c, PHASE2_MOVES[(lcg(seed) as usize) % 10]);
        }
        c
    }

    /// Apply a move-index path to `start`.
    fn apply_path(start: &Cubies, path: &[usize]) -> Cubies {
        let mut c = *start;
        for &mv in path {
            c = apply(&c, mv);
        }
        c
    }

    // PHASE2_MOVES is exactly the 10 G1 generators (no kewb F2/F2 duplication bug), and
    // phase 1 iterates all 18 moves with no drop/duplicate.
    #[test]
    fn phase2_moves_are_valid_distinct() {
        // 10 distinct indices, all in 0..18.
        let mut seen = [false; 18];
        for &m in &PHASE2_MOVES {
            assert!(m < 18, "PHASE2 move {m} out of range");
            assert!(!seen[m], "PHASE2 move {m} duplicated");
            seen[m] = true;
        }
        assert_eq!(PHASE2_MOVES.len(), 10);

        // Each is a U/D move (face 0 or 1, any turn) or a Double of L/R/F/B (8,11,14,17).
        for &m in &PHASE2_MOVES {
            let face = m / 3;
            let turn = m % 3;
            let ok = face <= 1 || (turn == 2 && (2..=5).contains(&face));
            assert!(
                ok,
                "PHASE2 move {m} (face {face}, turn {turn}) not a G1 generator"
            );
        }
        // Exact membership.
        assert_eq!(PHASE2_MOVES, [0, 1, 2, 3, 4, 5, 8, 11, 14, 17]);

        // Phase 1 covers the full 18-move set exactly once (the kewb dropped/duplicated
        // move guard): the set the DFS iterates is precisely {0..18}.
        let mut covered = [false; 18];
        for mv in 0..18usize {
            assert!(!covered[mv]);
            covered[mv] = true;
        }
        assert!(
            covered.into_iter().all(|b| b),
            "phase 1 must iterate all 18 moves"
        );
    }

    // Every move-transition row equals the re-ranked full-cube `apply`.
    #[test]
    fn move_tables_match_apply() {
        let t = TwoPhaseTables::build();

        // Phase-1 tables on ~2000 random reachable states.
        let mut seed = 0x1234_ABCDu32;
        for _ in 0..2000 {
            let len = 1 + (lcg(&mut seed) as usize % 30);
            let c = random_reachable(&mut seed, len);
            let co = corner_ori_rank(&c.co) as usize;
            let eo = eo12_rank(&c.eo) as usize;
            let es = eslice_combo_rank(&c.ep) as usize;
            for mv in 0..18usize {
                let moved = apply(&c, mv);
                assert_eq!(
                    t.co_move[co][mv],
                    corner_ori_rank(&moved.co),
                    "co_move mismatch (mv={mv})"
                );
                assert_eq!(
                    t.eo_move[eo][mv],
                    eo12_rank(&moved.eo),
                    "eo_move mismatch (mv={mv})"
                );
                assert_eq!(
                    t.es_move[es][mv],
                    eslice_combo_rank(&moved.ep),
                    "es_move mismatch (mv={mv})"
                );
            }
        }

        // Phase-2 tables on ~2000 random G1 states (so udep/eep ranks are well-defined).
        let mut seed = 0x9999_1111u32;
        for _ in 0..2000 {
            let len = 1 + (lcg(&mut seed) as usize % 30);
            let c = random_g1(&mut seed, len);
            let cp = perm_rank8(&c.cp) as usize;
            let udep = ud_ep_rank(&c.ep) as usize;
            let eep = e_ep_rank(&c.ep) as usize;
            for j in 0..10usize {
                let moved = apply(&c, PHASE2_MOVES[j]);
                assert_eq!(
                    t.cp_move[cp][j],
                    perm_rank8(&moved.cp) as u16,
                    "cp_move mismatch (j={j})"
                );
                assert_eq!(
                    t.udep_move[udep][j],
                    ud_ep_rank(&moved.ep),
                    "udep_move mismatch (j={j})"
                );
                assert_eq!(
                    t.eep_move[eep][j],
                    e_ep_rank(&moved.ep) as u16,
                    "eep_move mismatch (j={j})"
                );
            }
        }
    }

    // The pruning tables are valid BFS distance tables: solved == 0, fully reachable,
    // and adjacent under each table's moves (|d(s) - d(succ)| <= 1).
    #[test]
    fn pruning_tables_consistent() {
        let t = TwoPhaseTables::build();

        assert_eq!(t.co_e[0], 0);
        assert_eq!(t.eo_e[0], 0);
        assert_eq!(t.cp_e[0], 0);
        assert_eq!(t.udep_e[0], 0);

        assert!(t.co_e.iter().all(|&d| d != u8::MAX), "co_e has unreached");
        assert!(t.eo_e.iter().all(|&d| d != u8::MAX), "eo_e has unreached");
        assert!(t.cp_e.iter().all(|&d| d != u8::MAX), "cp_e has unreached");
        assert!(
            t.udep_e.iter().all(|&d| d != u8::MAX),
            "udep_e has unreached"
        );

        // Sample adjacency for the 18-wide phase-1 tables.
        let mut seed = 0x2468_1357u32;
        for _ in 0..3000 {
            let c1 = lcg(&mut seed) as usize % CO;
            let c2 = lcg(&mut seed) as usize % ESLICE;
            let d = t.co_e[c1 * ESLICE + c2] as i32;
            for mv in 0..18usize {
                let n1 = t.co_move[c1][mv] as usize;
                let n2 = t.es_move[c2][mv] as usize;
                let dn = t.co_e[n1 * ESLICE + n2] as i32;
                assert!((d - dn).abs() <= 1, "co_e adjacency broken (mv={mv})");
            }
            let e1 = lcg(&mut seed) as usize % EO;
            let de = t.eo_e[e1 * ESLICE + c2] as i32;
            for mv in 0..18usize {
                let n1 = t.eo_move[e1][mv] as usize;
                let n2 = t.es_move[c2][mv] as usize;
                let dn = t.eo_e[n1 * ESLICE + n2] as i32;
                assert!((de - dn).abs() <= 1, "eo_e adjacency broken (mv={mv})");
            }
        }

        // Sample adjacency for the 10-wide phase-2 tables.
        for _ in 0..3000 {
            let c1 = lcg(&mut seed) as usize % CP;
            let c2 = lcg(&mut seed) as usize % EEP;
            let d = t.cp_e[c1 * EEP + c2] as i32;
            for j in 0..10usize {
                let n1 = t.cp_move[c1][j] as usize;
                let n2 = t.eep_move[c2][j] as usize;
                let dn = t.cp_e[n1 * EEP + n2] as i32;
                assert!((d - dn).abs() <= 1, "cp_e adjacency broken (j={j})");
            }
            let u1 = lcg(&mut seed) as usize % UDEP;
            let du = t.udep_e[u1 * EEP + c2] as i32;
            for j in 0..10usize {
                let n1 = t.udep_move[u1][j] as usize;
                let n2 = t.eep_move[c2][j] as usize;
                let dn = t.udep_e[n1 * EEP + n2] as i32;
                assert!((du - dn).abs() <= 1, "udep_e adjacency broken (j={j})");
            }
        }
    }

    // Every solution actually solves its cube, is reasonably short, and the solved cube
    // returns the empty solution.
    #[test]
    fn two_phase_solutions_solve_the_cube() {
        let t = TwoPhaseTables::build();
        let never = never();

        // Solved -> empty.
        assert_eq!(solve(&t, &SOLVED, &never), Some(vec![]));

        let mut seed = 0xC0DE_F00Du32;
        // ~30 short scrambles (1..=8).
        for _ in 0..30 {
            let len = 1 + (lcg(&mut seed) as usize % 8); // 1..=8
            let start = random_reachable(&mut seed, len);
            let sol = solve(&t, &start, &never).expect("some solution");
            assert_eq!(
                apply_path(&start, &sol),
                SOLVED,
                "short scramble not solved"
            );
            assert!(sol.len() <= 24, "short solution too long: {}", sol.len());
        }
        // ~30 deep scrambles (24..=30).
        for _ in 0..30 {
            let len = 24 + (lcg(&mut seed) as usize % 7); // 24..=30
            let start = random_reachable(&mut seed, len);
            let sol = solve(&t, &start, &never).expect("some solution");
            assert_eq!(apply_path(&start, &sol), SOLVED, "deep scramble not solved");
            assert!(sol.len() <= 24, "deep solution too long: {}", sol.len());
        }
    }

    // A pre-set cancel on a non-trivial scramble returns None.
    #[test]
    fn two_phase_cancel_returns_none() {
        let t = TwoPhaseTables::build();
        let mut seed = 0xDEAD_0001u32;
        let start = random_reachable(&mut seed, 20);
        let cancel = AtomicBool::new(true);
        assert_eq!(solve(&t, &start, &cancel), None);
    }
}
