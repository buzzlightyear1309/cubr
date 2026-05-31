//! Guaranteed-optimal IDA* search for the Korf solver (Unit K3). Pure, no Bevy.
//!
//! Iterative-deepening A* over the 18 absolute moves, driven by an admissible
//! heuristic (the max-of-three pattern databases from [`super::pdb`], or any other
//! admissible lower bound — including the zero heuristic, which turns the search into a
//! plain iterative-deepening DFS that is still optimal). Optimality holds for **any**
//! admissible `h`; `h` only changes how much of the tree we prune, never the length of
//! the answer.

use super::coords::{apply, index_to_move, Cubies, SOLVED};
use super::pdb::{corner_index, edge_index_a, edge_index_b, Pdbs, SearchTables};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

/// How often (in expanded nodes) the DFS polls the cancel flag.
const CANCEL_CHECK_INTERVAL: u64 = 8192;

/// Redundancy pruning: `true` if move `cur` should be skipped given the previous move
/// `prev` (`None` at the root). Both rules are proven never to remove an optimal path:
///
/// 1. No two consecutive moves on the **same face** — they collapse to a single
///    face turn already covered by a different move index.
/// 2. For the two **commuting opposite faces** on an axis (U/D, L/R, F/B), only the
///    canonical ordering is allowed: forbid the higher-numbered face immediately after
///    the lower-numbered one (so e.g. `U` then `D` is allowed but `D` then `U` is not).
fn redundant(prev: Option<usize>, cur: usize) -> bool {
    let Some(p) = prev else { return false };
    let (pf, cf) = (p / 3, cur / 3);
    if pf == cf {
        return true; // (1) same face
    }
    if pf / 2 == cf / 2 && pf > cf {
        return true; // (2) commuting opposite faces: canonical order only
    }
    false
}

/// Sentinel for "no f-value exceeded the threshold" (an effectively infinite bound).
const INF: u8 = u8::MAX;

/// Outcome of a bounded depth-first probe.
enum Dfs {
    /// A solution was found; carries the move-index path from the search root.
    Found(Vec<usize>),
    /// No solution within the threshold; carries the smallest f-value that exceeded it
    /// (the next threshold to try), or [`INF`] if no child exceeded it / there were no
    /// children to expand.
    Min(u8),
    /// The cancel flag was observed set; the search is aborting.
    Cancelled,
}

/// IDA* optimal search. `h` is an **admissible** heuristic (a lower bound on the number
/// of moves needed to solve). Returns the optimal move-index sequence, or `None` if
/// `cancel` was set (a validated, solvable cube always yields `Some`).
///
/// Optimality holds for any admissible `h`, including `|_| 0`; `h` only affects speed.
///
/// This is the **reference oracle**: the production path is the faster coordinate-space
/// search below, but this `Cubies`-driven IDA* (especially with the zero heuristic) is
/// the trusted optimal-distance baseline the cross-check tests compare against, so it is
/// kept verbatim. It is only reachable from tests now.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn ida_star(
    start: &Cubies,
    h: impl Fn(&Cubies) -> u8,
    cancel: &AtomicBool,
) -> Option<Vec<usize>> {
    if *start == SOLVED {
        return Some(Vec::new());
    }

    let mut threshold = h(start);
    let mut path: Vec<usize> = Vec::new();
    let mut nodes: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match dfs(start, 0, threshold, None, &h, cancel, &mut path, &mut nodes) {
            Dfs::Found(p) => return Some(p),
            Dfs::Cancelled => return None,
            Dfs::Min(next) => {
                // No exceeded f at all: the space below `start` is exhausted without a
                // solution. For a solvable cube this never happens (it is always
                // reachable), so guard against an infinite loop rather than spinning.
                if next == INF || next <= threshold {
                    return None;
                }
                threshold = next;
            }
        }
    }
}

/// Bounded DFS. `g` is the cost so far, `threshold` the current f-bound, `prev_move` the
/// previous move index (for redundancy pruning). `path` holds the move indices from the
/// root to the current node; it is restored on return. Returns the search outcome.
#[cfg_attr(not(test), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
fn dfs(
    node: &Cubies,
    g: u8,
    threshold: u8,
    prev_move: Option<usize>,
    h: &impl Fn(&Cubies) -> u8,
    cancel: &AtomicBool,
    path: &mut Vec<usize>,
    nodes: &mut u64,
) -> Dfs {
    let f = g.saturating_add(h(node));
    if f > threshold {
        return Dfs::Min(f);
    }
    if *node == SOLVED {
        return Dfs::Found(path.clone());
    }

    *nodes += 1;
    if nodes.is_multiple_of(CANCEL_CHECK_INTERVAL) && cancel.load(Ordering::Relaxed) {
        return Dfs::Cancelled;
    }

    let mut min = INF;
    for mv in 0..18usize {
        if redundant(prev_move, mv) {
            continue;
        }
        let child = apply(node, mv);
        path.push(mv);
        let outcome = dfs(&child, g + 1, threshold, Some(mv), h, cancel, path, nodes);
        path.pop();
        match outcome {
            Dfs::Found(p) => return Dfs::Found(p),
            Dfs::Cancelled => return Dfs::Cancelled,
            Dfs::Min(m) => min = min.min(m),
        }
    }
    Dfs::Min(min)
}

// ---------------------------------------------------------------------------
// Fast production search: incremental coordinates + dense tables + multicore.
// ---------------------------------------------------------------------------
//
// The reference `ida_star` above re-derives all three PDB indices from the full
// `Cubies` at every node (and is kept verbatim as the optimality oracle). The
// production path below instead carries the *triple* `(corner_index, edge_index_a,
// edge_index_b)` — a complete, injective encoding of the cube — and advances it with
// `SearchTables` table lookups, so the hot loop never touches a `Cubies` or re-ranks.
// It also fans the legal root moves across threads. Both paths are optimal (admissible
// `max`-of-three heuristic + IDA*); this one is just dramatically faster on deep solves.

/// The search coordinate: a complete encoding of the cube as its three PDB indices.
/// `(0, 0, edge_index_b(&SOLVED))` is solved.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Coord {
    ci: u32,
    ai: u32,
    bi: u32,
}

impl Coord {
    /// The coordinate of `c`.
    fn of(c: &Cubies) -> Coord {
        Coord {
            ci: corner_index(c),
            ai: edge_index_a(c),
            bi: edge_index_b(c),
        }
    }

    /// The successor under move `mv`, via the dense transition tables (no `Cubies`).
    #[inline]
    fn neighbor(&self, tables: &SearchTables, mv: usize) -> Coord {
        Coord {
            ci: tables.corner_neighbor(self.ci, mv),
            ai: tables.edge_neighbor(self.ai, mv),
            bi: tables.edge_neighbor(self.bi, mv),
        }
    }
}

/// Bounded coordinate-space DFS, shared by the single- and multi-threaded drivers.
/// Identical control flow to [`dfs`] but operating on [`Coord`] with the dense-table
/// heuristic and transitions. `path` is the move-index stack from the search root.
#[allow(clippy::too_many_arguments)]
fn dfs_coord(
    node: Coord,
    solved: Coord,
    g: u8,
    threshold: u8,
    prev_move: Option<usize>,
    pdbs: &Pdbs,
    tables: &SearchTables,
    cancel: &AtomicBool,
    path: &mut Vec<usize>,
    nodes: &mut u64,
) -> Dfs {
    let f = g.saturating_add(pdbs.h_index(node.ci, node.ai, node.bi));
    if f > threshold {
        return Dfs::Min(f);
    }
    if node == solved {
        return Dfs::Found(path.clone());
    }

    *nodes += 1;
    if nodes.is_multiple_of(CANCEL_CHECK_INTERVAL) && cancel.load(Ordering::Relaxed) {
        return Dfs::Cancelled;
    }

    let mut min = INF;
    for mv in 0..18usize {
        if redundant(prev_move, mv) {
            continue;
        }
        let child = node.neighbor(tables, mv);
        path.push(mv);
        let outcome = dfs_coord(
            child,
            solved,
            g + 1,
            threshold,
            Some(mv),
            pdbs,
            tables,
            cancel,
            path,
            nodes,
        );
        path.pop();
        match outcome {
            Dfs::Found(p) => return Dfs::Found(p),
            Dfs::Cancelled => return Dfs::Cancelled,
            Dfs::Min(m) => min = min.min(m),
        }
    }
    Dfs::Min(min)
}

/// Single-threaded coordinate IDA* (fallback when there is no usable parallelism or only
/// one legal root move). Returns the optimal move-index path, or `None` if cancelled.
fn ida_coord_single(
    start: Coord,
    solved: Coord,
    pdbs: &Pdbs,
    tables: &SearchTables,
    cancel: &AtomicBool,
) -> Option<Vec<usize>> {
    if start == solved {
        return Some(Vec::new());
    }
    let mut threshold = pdbs.h_index(start.ci, start.ai, start.bi);
    let mut path: Vec<usize> = Vec::new();
    let mut nodes: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match dfs_coord(
            start, solved, 0, threshold, None, pdbs, tables, cancel, &mut path, &mut nodes,
        ) {
            Dfs::Found(p) => return Some(p),
            Dfs::Cancelled => return None,
            Dfs::Min(next) => {
                if next == INF || next <= threshold {
                    return None;
                }
                threshold = next;
            }
        }
    }
}

/// Per-threshold result of probing a single root-move subset.
enum RootProbe {
    /// A solution was found at the current threshold (carries the full move-index path).
    Found(Vec<usize>),
    /// Exhausted the threshold with no solution. The next-threshold candidate has already
    /// been folded into the shared `best_next` atomic, so it is not carried here.
    Min,
    /// Aborted (the shared `found`/`cancel` flag was observed set).
    Stopped,
}

/// Probe one root-move subset for `threshold`: expand each assigned root move and DFS
/// below it. Bails early if another thread already found a solution (`found`) or the
/// caller cancelled. Reduces the next-threshold candidate into `best_next` (atomic min).
#[allow(clippy::too_many_arguments)]
fn probe_roots(
    roots: &[usize],
    start: Coord,
    solved: Coord,
    threshold: u8,
    pdbs: &Pdbs,
    tables: &SearchTables,
    found: &AtomicBool,
    best_next: &AtomicU8,
    cancel: &AtomicBool,
) -> RootProbe {
    // Root node: f = h(start) (g = 0). If it already exceeds the threshold, this subset
    // contributes that f as its next-threshold candidate and nothing else.
    let hstart = pdbs.h_index(start.ci, start.ai, start.bi);
    if hstart > threshold {
        best_next.fetch_min(hstart, Ordering::Relaxed);
        return RootProbe::Min;
    }

    let mut min = INF;
    let mut nodes: u64 = 0;
    let mut path: Vec<usize> = Vec::with_capacity(threshold as usize + 1);
    for &mv in roots {
        if found.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
            return RootProbe::Stopped;
        }
        let child = start.neighbor(tables, mv);
        path.clear();
        path.push(mv);
        match dfs_coord(
            child,
            solved,
            1,
            threshold,
            Some(mv),
            pdbs,
            tables,
            cancel,
            &mut path,
            &mut nodes,
        ) {
            Dfs::Found(p) => return RootProbe::Found(p),
            Dfs::Cancelled => return RootProbe::Stopped,
            Dfs::Min(m) => min = min.min(m),
        }
    }
    // Fold this subset's next-threshold candidate into the shared min.
    if min != INF {
        best_next.fetch_min(min, Ordering::Relaxed);
    }
    RootProbe::Min
}

/// Multi-threaded coordinate IDA*. The legal root moves are partitioned into disjoint
/// subsets, one per thread; each thread DFS-searches its subset for the *current*
/// threshold. The threshold rises only when **all** threads exhaust it without a
/// solution, so the first threshold at which any thread reports a solution is the
/// optimum — returning any path found at that threshold is optimal.
fn ida_coord_parallel(
    start: Coord,
    solved: Coord,
    roots: &[usize],
    threads: usize,
    pdbs: &Pdbs,
    tables: &SearchTables,
    cancel: &AtomicBool,
) -> Option<Vec<usize>> {
    // Round-robin the root moves into `threads` disjoint chunks (balances the heavy
    // first roots across workers better than contiguous slicing).
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); threads];
    for (i, &mv) in roots.iter().enumerate() {
        buckets[i % threads].push(mv);
    }
    buckets.retain(|b| !b.is_empty());

    let mut threshold = pdbs.h_index(start.ci, start.ai, start.bi);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }

        let found = AtomicBool::new(false);
        let best_next = AtomicU8::new(INF);
        let solution: Mutex<Option<Vec<usize>>> = Mutex::new(None);

        std::thread::scope(|s| {
            let handles: Vec<_> = buckets
                .iter()
                .map(|bucket| {
                    let found = &found;
                    let best_next = &best_next;
                    let solution = &solution;
                    s.spawn(move || {
                        match probe_roots(
                            bucket, start, solved, threshold, pdbs, tables, found, best_next,
                            cancel,
                        ) {
                            RootProbe::Found(p) => {
                                // First writer wins; any solution at this threshold is
                                // optimal, so we don't compare lengths.
                                if !found.swap(true, Ordering::Relaxed) {
                                    *solution.lock().unwrap() = Some(p);
                                }
                            }
                            RootProbe::Min | RootProbe::Stopped => {}
                        }
                    })
                })
                .collect();
            for h in handles {
                let _ = h.join();
            }
        });

        if let Some(p) = solution.into_inner().unwrap() {
            return Some(p);
        }
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let next = best_next.load(Ordering::Relaxed);
        if next == INF || next <= threshold {
            // No child exceeded the threshold anywhere: the space is exhausted with no
            // solution (cannot happen for a validated solvable cube, but guard anyway).
            return None;
        }
        threshold = next;
    }
}

/// The single internal entry point both public surfaces delegate to (DRY). Runs the fast
/// coordinate-space IDA* — multicore when there is real parallelism and more than one
/// legal root move, single-threaded otherwise — and maps the move indices to
/// [`Move`](crate::model::Move)s. Returns `None` if `cancel` was observed set.
pub(crate) fn search(
    pdbs: &Pdbs,
    tables: &SearchTables,
    start: &Cubies,
    cancel: &AtomicBool,
) -> Option<Vec<crate::model::Move>> {
    let start_coord = Coord::of(start);
    let solved = Coord::of(&SOLVED);
    if start_coord == solved {
        return Some(Vec::new());
    }

    // Legal root moves under the redundancy pruning (root forbids nothing, so all 18).
    let roots: Vec<usize> = (0..18usize).filter(|&mv| !redundant(None, mv)).collect();
    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let threads = parallelism.min(roots.len());

    let idxs = if threads <= 1 || roots.len() <= 1 {
        ida_coord_single(start_coord, solved, pdbs, tables, cancel)
    } else {
        ida_coord_parallel(start_coord, solved, &roots, threads, pdbs, tables, cancel)
    };
    idxs.map(|idxs| idxs.into_iter().map(index_to_move).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CubeCore;
    use crate::model::{CubeState, Move};
    use std::collections::HashMap;

    /// A fresh, never-set cancel flag.
    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// Tiny deterministic LCG (Numerical Recipes); no `rand` crate.
    fn lcg(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *seed
    }

    /// Hashable key for a `Cubies` (it derives `Eq` but not `Hash`).
    type Key = ([u8; 8], [u8; 8], [u8; 12], [u8; 12]);
    fn key(c: &Cubies) -> Key {
        (c.cp, c.co, c.ep, c.eo)
    }

    /// Apply a move-index path to `start` and return the resulting `Cubies`.
    fn apply_path(start: &Cubies, path: &[usize]) -> Cubies {
        let mut c = *start;
        for &mv in path {
            c = apply(&c, mv);
        }
        c
    }

    /// Brute-force BFS from SOLVED recording the true optimal distance for every state
    /// reachable within `max_depth` moves. Keyed on the `Cubies` tuple.
    fn brute_bfs(max_depth: u8) -> HashMap<Key, u8> {
        let mut dist: HashMap<Key, u8> = HashMap::new();
        dist.insert(key(&SOLVED), 0);
        let mut frontier = vec![SOLVED];
        let mut depth = 0u8;
        while depth < max_depth && !frontier.is_empty() {
            let mut next = Vec::new();
            for c in &frontier {
                for mv in 0..18usize {
                    let nc = apply(c, mv);
                    let k = key(&nc);
                    if let std::collections::hash_map::Entry::Vacant(e) = dist.entry(k) {
                        e.insert(depth + 1);
                        next.push(nc);
                    }
                }
            }
            frontier = next;
            depth += 1;
        }
        dist
    }

    // --- 1. Trivial cases. ---

    #[test]
    fn solved_needs_no_moves() {
        let c = never();
        assert_eq!(ida_star(&SOLVED, |_| 0, &c), Some(vec![]));
    }

    #[test]
    fn single_move_solves_in_one() {
        // Every single move applied to SOLVED is one move from solved (its inverse).
        for mv in 0..18usize {
            let scrambled = apply(&SOLVED, mv);
            let c = never();
            let sol = ida_star(&scrambled, |_| 0, &c).expect("solvable");
            assert_eq!(sol.len(), 1, "move {mv} should solve in exactly 1");
            assert_eq!(apply_path(&scrambled, &sol), SOLVED, "move {mv} not solved");
        }
    }

    // --- 2. ★ Optimality cross-check with the ZERO heuristic (no PDBs needed). ---

    #[test]
    fn zero_heuristic_matches_brute_optimal_distances() {
        const D: u8 = 5;
        let dist = brute_bfs(D);

        // Sample states spread across depths 0..=D and confirm IDA* (zero heuristic,
        // pure IDDFS) returns exactly the brute-BFS optimal distance, and that the
        // returned path actually solves the state.
        //
        // Bucket the reached states by depth so the sample spans every depth.
        let mut by_depth: Vec<Vec<Key>> = vec![Vec::new(); (D + 1) as usize];
        for (k, &d) in &dist {
            by_depth[d as usize].push(*k);
        }

        let mut seed = 0x5EED_1234u32;
        let mut checked = 0usize;
        // Take a spread: up to ~50 from each depth bucket → ~300 total.
        for (d, bucket) in by_depth.iter().enumerate() {
            let want = d as u8;
            let take = bucket.len().min(50);
            for _ in 0..take {
                let pick = (lcg(&mut seed) as usize) % bucket.len();
                let k = bucket[pick];
                let state = Cubies {
                    cp: k.0,
                    co: k.1,
                    ep: k.2,
                    eo: k.3,
                };
                let c = never();
                let sol = ida_star(&state, |_| 0, &c).expect("solvable");
                assert_eq!(
                    sol.len() as u8,
                    want,
                    "zero-heuristic length != brute optimal at depth {want}"
                );
                assert_eq!(
                    apply_path(&state, &sol),
                    SOLVED,
                    "returned solution does not solve the state (depth {want})"
                );
                checked += 1;
            }
        }
        assert!(checked >= 200, "expected a broad sample, only {checked}");
    }

    // --- 3. Cross-check the emitted Move labels against the trusted engine. ---

    #[test]
    fn emitted_moves_solve_the_core_engine() {
        let mut seed = 0xABCD_0001u32;
        for _ in 0..20 {
            // Build a scramble of length <= 6 as Vec<Move>.
            let len = 1 + (lcg(&mut seed) as usize % 6); // 1..=6
            let mut scramble: Vec<Move> = Vec::with_capacity(len);
            let mut cubies = SOLVED;
            let mut core = CubeCore::solved();
            for _ in 0..len {
                let idx = (lcg(&mut seed) as usize) % 18;
                let m = Move::ALL[idx];
                scramble.push(m);
                cubies = apply(&cubies, idx);
                core.apply(m);
            }

            // Solve the Cubies with the zero heuristic, emit our Moves.
            let cancel = never();
            let sol_idx = ida_star(&cubies, |_| 0, &cancel).expect("solvable");
            let sol: Vec<Move> = sol_idx.iter().map(|&i| index_to_move(i)).collect();

            // The emitted solution must drive the independent engine back to solved.
            for &m in &sol {
                core.apply(m);
            }
            assert_eq!(
                core.to_state(),
                CubeState::solved(),
                "emitted solution failed to solve the engine for scramble {scramble:?}"
            );
        }
    }

    // --- 4. Pruning sanity: redundant never blocks all moves, and respects the rules. ---

    #[test]
    fn redundant_root_allows_everything() {
        for mv in 0..18usize {
            assert!(!redundant(None, mv), "root must allow move {mv}");
        }
    }

    #[test]
    fn redundant_never_blocks_all_moves() {
        // For any previous move there must remain at least one legal continuation,
        // otherwise the search would dead-end incorrectly.
        for prev in 0..18usize {
            let allowed = (0..18usize)
                .filter(|&mv| !redundant(Some(prev), mv))
                .count();
            assert!(allowed > 0, "prev {prev} blocks every move");
        }
    }

    #[test]
    fn redundant_rules_are_exact() {
        // Rule 1: same face always blocked. Rule 2: opposite faces on an axis allow
        // exactly one ordering. faces: 0=U 1=D 2=L 3=R 4=F 5=B; axis = face/2.
        for p in 0..18usize {
            for cur in 0..18usize {
                let (pf, cf) = (p / 3, cur / 3);
                let want = if pf == cf {
                    true
                } else if pf / 2 == cf / 2 {
                    pf > cf // higher face after lower on same axis is blocked
                } else {
                    false
                };
                assert_eq!(
                    redundant(Some(p), cur),
                    want,
                    "redundant({p},{cur}) wrong (pf={pf} cf={cf})"
                );
            }
        }
        // Concretely: U then D allowed (0<1), D then U blocked.
        assert!(!redundant(Some(0), 3)); // U, D
        assert!(redundant(Some(3), 0)); // D, U
    }

    // --- 5. Full-PDB optimality (ignored: PDB build + deep solves are slow). ---

    #[test]
    #[ignore = "builds the full ~85 MB PDBs and runs deep optimal solves (slow; run in release)"]
    fn full_pdb_optimality() {
        let pdbs = Pdbs::generate();
        let tables = SearchTables::build();

        // The user's reported case: an 8-quarter-turn scramble (no doubles) must solve
        // in <= 8 moves and the solution must actually solve it.
        let scramble = ["R", "U", "F", "L", "D", "B", "R", "U"];
        let mut cubies = SOLVED;
        let mut core = CubeCore::solved();
        for s in scramble {
            let m = Move::parse(s).unwrap();
            cubies = apply(&cubies, crate::solver::coords::move_to_index(m));
            core.apply(m);
        }
        let cancel = never();
        let sol = search(&pdbs, &tables, &cubies, &cancel).expect("solvable");
        assert!(
            sol.len() <= 8,
            "8-quarter-turn scramble solved in {} moves (> 8)",
            sol.len()
        );
        for &m in &sol {
            core.apply(m);
        }
        assert_eq!(
            core.to_state(),
            CubeState::solved(),
            "8-quarter-turn solution did not solve the engine"
        );

        // Several random scrambles (length 12..=20): solution <= 20 and solves.
        let mut seed = 0x0DDB_0A11u32;
        for _ in 0..5 {
            let len = 12 + (lcg(&mut seed) as usize % 9); // 12..=20
            let mut cubies = SOLVED;
            let mut core = CubeCore::solved();
            for _ in 0..len {
                let idx = (lcg(&mut seed) as usize) % 18;
                cubies = apply(&cubies, idx);
                core.apply(Move::ALL[idx]);
            }
            let cancel = never();
            let sol = search(&pdbs, &tables, &cubies, &cancel).expect("solvable");
            assert!(
                sol.len() <= 20,
                "random scramble solved in {} (> 20)",
                sol.len()
            );
            for &m in &sol {
                core.apply(m);
            }
            assert_eq!(
                core.to_state(),
                CubeState::solved(),
                "random scramble solution did not solve the engine"
            );
        }
    }

    /// Sanity that the PDB heuristic does not change the optimum: for shallow random
    /// states the full-PDB `search` length equals the zero-heuristic `ida_star` length.
    #[test]
    #[ignore = "builds the full PDBs; compares PDB-guided vs zero-heuristic optimum on shallow states"]
    fn full_pdb_matches_zero_heuristic_on_shallow_states() {
        let pdbs = Pdbs::generate();
        let tables = SearchTables::build();
        let mut seed = 0xFACE_0F01u32;
        for _ in 0..30 {
            // Shallow enough that the zero-heuristic IDDFS oracle stays quick, but a touch
            // deeper than before so the cross-check spans more of the tree.
            let len = 1 + (lcg(&mut seed) as usize % 9); // shallow: 1..=9
            let mut cubies = SOLVED;
            for _ in 0..len {
                cubies = apply(&cubies, (lcg(&mut seed) as usize) % 18);
            }
            let c1 = never();
            let c2 = never();
            let pdb_len = search(&pdbs, &tables, &cubies, &c1)
                .expect("solvable")
                .len();
            let zero_len = ida_star(&cubies, |_| 0, &c2).expect("solvable").len();
            assert_eq!(
                pdb_len, zero_len,
                "PDB heuristic changed the optimum (pdb={pdb_len} zero={zero_len})"
            );
        }
    }
}
