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
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
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
    /// The probe bailed early because a stop flag was observed set — either the caller's
    /// `cancel` flag, or (in the parallel driver) the shared `found` flag once another
    /// worker has produced a solution at the current threshold. The two are folded into
    /// one variant: in both cases the answer this subtree could still yield is no longer
    /// wanted, so the search unwinds immediately.
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
///
/// `found` is the shared early-bail flag: every [`CANCEL_CHECK_INTERVAL`] expanded nodes
/// the probe checks `found.load() || cancel.load()` and, if either is set, unwinds fast
/// with [`Dfs::Cancelled`]. In the parallel driver `found` is raised the instant any
/// worker produces a solution at the current threshold, so sibling subtrees stop grinding
/// immediately instead of running to completion before the join. The single-threaded
/// driver passes a never-set `found`, so this is a no-op there. Bailing only ever
/// *discards* in-flight work — it never changes which threshold first yields a solution,
/// so optimality is preserved.
#[allow(clippy::too_many_arguments)]
fn dfs_coord(
    node: Coord,
    solved: Coord,
    g: u8,
    threshold: u8,
    prev_move: Option<usize>,
    pdbs: &Pdbs,
    tables: &SearchTables,
    found: &AtomicBool,
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
    if nodes.is_multiple_of(CANCEL_CHECK_INTERVAL)
        && (found.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed))
    {
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
            found,
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
    // The single-threaded path has no peer worker to short-circuit it, so `found` stays
    // unset and the early-bail in `dfs_coord` reduces to the plain `cancel` poll.
    let found = AtomicBool::new(false);
    let mut threshold = pdbs.h_index(start.ci, start.ai, start.bi);
    let mut path: Vec<usize> = Vec::new();
    let mut nodes: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match dfs_coord(
            start, solved, 0, threshold, None, pdbs, tables, &found, cancel, &mut path, &mut nodes,
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

/// One unit of fine-grained parallel work: a depth-2 node of the search tree, tagged with
/// the two-move prefix that reaches it. The parallel driver hands these out one at a time
/// so the cost of a heavy subtree falls on a single puller rather than bounding the whole
/// iteration (as the old coarse round-robin-over-18-roots split did).
#[derive(Clone, Copy)]
struct Task {
    /// The coordinate after applying `prefix[0]` then `prefix[1]` to the start.
    coord: Coord,
    /// The two move indices `[m1, m2]` that reach `coord` from the start.
    prefix: [usize; 2],
    /// The previous move (`prefix[1]`), for the redundancy pruning of `coord`'s children.
    prev_move: usize,
}

/// What the single-threaded "expand the top two levels" pass produced for one threshold.
enum Expansion {
    /// `start`/a depth-1 node *is* solved within the threshold — the optimal path is known
    /// without touching the frontier (carries the move-index path).
    Solved(Vec<usize>),
    /// The frontier of admissible depth-2 nodes to search this threshold. `best_next` holds
    /// the smallest f-value of any node *pruned* at depth 0 or 1 (folded in exactly as
    /// `ida_coord_single` would), so the frontier search only needs to min into it further.
    Frontier { tasks: Vec<Task>, best_next: u8 },
}

/// Mirror exactly what [`dfs_coord`] / [`ida_coord_single`] do at depths 0 and 1 for the
/// given `threshold`, single-threaded and cheap (≤ 18·~15 nodes):
///
/// - **Depth 0** (`g = 0`, `f = h(start)`): if `f > threshold` the whole search is pruned —
///   return [`Expansion::Frontier`] with `best_next = f` and no tasks. (`start == solved`
///   is handled by the caller before the loop, matching `ida_coord_single`.)
/// - **Depth 1** (`g = 1`) for each legal root `m1`: `c1 = start.neighbor(m1)`,
///   `f = 1 + h(c1)`. If `f > threshold`, fold `f` into `best_next` (this depth-1 node is
///   pruned; its depth-2 children are *not* in play this threshold). Else if `c1 == solved`
///   the optimum is `[m1]` (`1 <= threshold` here since `f = 1 <= threshold`). Otherwise the
///   node is f-admissible, so enqueue each of its legal depth-2 children as a [`Task`].
///
/// This makes the parallel driver's `best_next` identical to the single-threaded sequence,
/// which is what keeps the parallel optimum equal to the single-threaded optimum (an
/// overshooting threshold could otherwise return a suboptimal solution).
fn expand_top_two_levels(
    start: Coord,
    solved: Coord,
    threshold: u8,
    roots: &[usize],
    pdbs: &Pdbs,
    tables: &SearchTables,
) -> Expansion {
    // Depth 0: prune the entire search if even the start's heuristic exceeds the threshold.
    let hstart = pdbs.h_index(start.ci, start.ai, start.bi);
    if hstart > threshold {
        return Expansion::Frontier {
            tasks: Vec::new(),
            best_next: hstart,
        };
    }

    let mut best_next = INF;
    let mut tasks: Vec<Task> = Vec::new();
    for &m1 in roots {
        let c1 = start.neighbor(tables, m1);
        let f1 = 1u8.saturating_add(pdbs.h_index(c1.ci, c1.ai, c1.bi));
        if f1 > threshold {
            // Depth-1 node pruned: contributes its f as a next-threshold candidate; its
            // depth-2 children do not enter the frontier this threshold.
            best_next = best_next.min(f1);
            continue;
        }
        if c1 == solved {
            // Optimal one-move solution (f1 = 1 <= threshold).
            return Expansion::Solved(vec![m1]);
        }
        // f-admissible depth-1 node: expand its legal depth-2 children into tasks.
        for m2 in 0..18usize {
            if redundant(Some(m1), m2) {
                continue;
            }
            let c2 = c1.neighbor(tables, m2);
            tasks.push(Task {
                coord: c2,
                prefix: [m1, m2],
                prev_move: m2,
            });
        }
    }
    Expansion::Frontier { tasks, best_next }
}

/// Multi-threaded coordinate IDA* with a **depth-2 frontier** and **dynamic work-pulling**.
///
/// Each threshold iteration:
/// 1. Single-threaded, build the frontier of admissible depth-2 nodes via
///    [`expand_top_two_levels`] (which also handles the length-0/1 solutions exactly and
///    seeds `best_next` with every depth-0/1 prune, so the per-threshold `best_next` matches
///    [`ida_coord_single`] precisely — this is what preserves optimality).
/// 2. A [`std::thread::scope`] pool of `threads` workers each `fetch_add(1)` task indices
///    from a shared [`AtomicUsize`] cursor until exhausted. Because tasks ≫ threads
///    (~200+), a heavy subtree is pulled by one worker while the others race ahead through
///    the rest — far better balanced than the old static split. Each task DFS-searches from
///    its depth-2 coordinate with `g = 2` and `path` pre-seeded to its two-move prefix.
/// 3. On a [`Dfs::Found`], the first writer raises `found` and stores the returned path
///    (already complete, since `path` was seeded with the prefix). All workers poll `found`
///    and bail. Every task's [`Dfs::Min`] is reduced into a shared `best_next`
///    ([`AtomicU8::fetch_min`]).
///
/// All workers share the single `threshold`, so the first threshold at which *any* task
/// finds a solution is the optimum — returning that path is optimal.
fn ida_coord_frontier(
    start: Coord,
    solved: Coord,
    roots: &[usize],
    threads: usize,
    pdbs: &Pdbs,
    tables: &SearchTables,
    cancel: &AtomicBool,
) -> Option<Vec<usize>> {
    let mut threshold = pdbs.h_index(start.ci, start.ai, start.bi);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }

        // Step 1: build this threshold's frontier (also resolves length-0/1 optima and
        // seeds best_next with the depth-0/1 prunes, matching ida_coord_single exactly).
        let (tasks, base_next) =
            match expand_top_two_levels(start, solved, threshold, roots, pdbs, tables) {
                Expansion::Solved(path) => return Some(path),
                Expansion::Frontier { tasks, best_next } => (tasks, best_next),
            };

        // Step 2 + 3: dynamic work-pulling over the frontier.
        let found = AtomicBool::new(false);
        let best_next = AtomicU8::new(base_next);
        let cursor = AtomicUsize::new(0);
        let solution: Mutex<Option<Vec<usize>>> = Mutex::new(None);

        std::thread::scope(|s| {
            for _ in 0..threads {
                let found = &found;
                let best_next = &best_next;
                let cursor = &cursor;
                let solution = &solution;
                let tasks = &tasks;
                s.spawn(move || {
                    let mut nodes: u64 = 0;
                    let mut path: Vec<usize> = Vec::with_capacity(threshold as usize + 1);
                    loop {
                        if found.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        let Some(task) = tasks.get(i) else {
                            return; // frontier exhausted
                        };
                        path.clear();
                        path.extend_from_slice(&task.prefix);
                        match dfs_coord(
                            task.coord,
                            solved,
                            2,
                            threshold,
                            Some(task.prev_move),
                            pdbs,
                            tables,
                            found,
                            cancel,
                            &mut path,
                            &mut nodes,
                        ) {
                            Dfs::Found(p) => {
                                // First writer wins; any solution at this threshold is
                                // optimal, so lengths are not compared. `p` already carries
                                // the two-move prefix (path was seeded with it).
                                if !found.swap(true, Ordering::Relaxed) {
                                    *solution.lock().unwrap() = Some(p);
                                }
                                return;
                            }
                            Dfs::Cancelled => return,
                            Dfs::Min(m) => {
                                best_next.fetch_min(m, Ordering::Relaxed);
                            }
                        }
                    }
                });
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
            // No node exceeded the threshold anywhere: the space is exhausted with no
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
    let threads = solver_threads();

    let idxs = if threads <= 1 || roots.len() <= 1 {
        ida_coord_single(start_coord, solved, pdbs, tables, cancel)
    } else {
        ida_coord_frontier(start_coord, solved, &roots, threads, pdbs, tables, cancel)
    };
    idxs.map(|idxs| idxs.into_iter().map(index_to_move).collect())
}

/// Resolve the worker-thread count for the parallel driver. `CUBR_SOLVER_THREADS`, if set
/// to a parseable positive integer, overrides it (handy for benchmarking / reproducibility
/// and for forcing the single-threaded path with `1`); absent, unparseable, or `0` falls
/// back to [`std::thread::available_parallelism`] (then `1` if even that is unavailable).
fn solver_threads() -> usize {
    if let Ok(s) = std::env::var("CUBR_SOLVER_THREADS") {
        if let Ok(n) = s.trim().parse::<usize>() {
            if n >= 1 {
                return n;
            }
        }
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
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

    /// The depth-2 frontier parallel driver must return solutions of the SAME length as the
    /// single-threaded driver (its reference optimum) for medium-depth states, and both must
    /// actually re-solve the cube. This is the optimality guard for the dynamic-balancing
    /// rewrite: a per-threshold `best_next` that diverged from the single-threaded sequence
    /// could let an overshooting threshold return a suboptimal answer; equal lengths across
    /// ~20 states prove it does not.
    #[test]
    #[ignore = "builds the full ~85 MB PDBs and runs ~20 medium-depth optimal solves on both \
                drivers (slow; run in release with --ignored)"]
    fn parallel_matches_single_optimum() {
        let pdbs = Pdbs::generate();
        let tables = SearchTables::build();
        let solved = Coord::of(&SOLVED);
        // All 18 legal root moves (the root prunes nothing) — same set the driver builds.
        let roots: Vec<usize> = (0..18usize).filter(|&mv| !redundant(None, mv)).collect();
        // Force real fan-out independent of the host's core count.
        let threads = 8usize;

        let mut seed = 0xBEEF_5A17u32;
        for trial in 0..20 {
            // Medium-depth scrambles (12..=16 moves) — deep enough to exercise the
            // multi-iteration frontier, shallow enough to stay tractable for both drivers.
            let len = 12 + (lcg(&mut seed) as usize % 5); // 12..=16
            let mut cubies = SOLVED;
            for _ in 0..len {
                cubies = apply(&cubies, (lcg(&mut seed) as usize) % 18);
            }
            let start = Coord::of(&cubies);

            let c1 = never();
            let c2 = never();
            let single = ida_coord_single(start, solved, &pdbs, &tables, &c1).expect("solvable");
            let parallel = ida_coord_frontier(start, solved, &roots, threads, &pdbs, &tables, &c2)
                .expect("solvable");

            assert_eq!(
                single.len(),
                parallel.len(),
                "trial {trial}: parallel length {} != single optimum {} (scramble len {len})",
                parallel.len(),
                single.len(),
            );
            // Both paths must drive the state back to solved.
            assert_eq!(
                apply_path(&cubies, &single),
                SOLVED,
                "trial {trial}: single-threaded solution did not solve the state"
            );
            assert_eq!(
                apply_path(&cubies, &parallel),
                SOLVED,
                "trial {trial}: parallel solution did not solve the state"
            );
        }
    }
}
