//! Right-docked "Solution" panel: a **Solve** button computes a solution for the
//! current cube via the pure [`cubr_core::solver`] adapter and lists the moves; a
//! **Run** button animates that solution by enqueuing the moves onto the shared
//! [`MoveQueue`] (the existing animator drains it — no new animation code here).
//!
//! The step list re-renders live in the active [`ControlScheme`]: Standard
//! notation (`R`, `U'`, `F2`) or Beginner view-relative wording (`Front CW`),
//! the latter recomputed every frame against the current camera basis so it
//! tracks orbiting. The step currently animating during a Run is highlighted.
//!
//! The solver's (slow, ~85 MB) Korf pattern databases are loaded-or-generated
//! **once at startup, off-thread** via `AsyncComputeTaskPool`, so the window opens
//! immediately and the Solve button simply reports "Building solver tables..." until
//! they land. Each solve also runs **off-thread** (a guaranteed-optimal IDA* search
//! can take seconds on the deepest states): pressing Solve spawns a task and shows
//! "Solving...", and a repaint cancels any in-flight solve.
//!
//! A **Live** toggle turns the static list into a live-sort: it solves the
//! current state once, then tracks every applied move (any source) cheaply with
//! no solver — popping the front when the user plays the next step, or prepending
//! the move's inverse (merged same-face) when they deviate. A debounced,
//! cancellable, generation-guarded background re-solve restores optimality once
//! the user pauses.
//!
//! This is a pure presentation/input layer: it reads `Cube` + the camera basis,
//! ends at `MoveQueue` / `ApplyState`, and never reaches into the frozen engine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use cubr_core::solver;

use crate::camera::OrbitCamera;
use crate::cube::animation::ActiveMove;
use crate::cube::model::{Move, Turn};
use crate::cube::{ApplyState, Cube, MoveApplied, MoveQueue};
use crate::ui::{
    set_button_color, spawn_labeled_button, ControlScheme, BTN_HOVER, BTN_NORMAL, BTN_PRESSED,
    LABEL_COLOR, PANEL_BG,
};
use crate::view_relative::{describe, rel_label};

/// The ready [`solver::Solver`] (PDBs + prebuilt in-memory search tables), once the
/// background load/build completes (`None` while loading). `Arc`-wrapped so each
/// off-thread solve closure can hold a clone without borrowing any Bevy state, and so the
/// ~62 MB search tables are built once and shared by every solve.
#[derive(Resource, Default)]
struct SolverReady(Option<Arc<solver::Solver>>);

/// The in-flight startup PDB load/build. Removed once polled to completion.
#[derive(Resource)]
struct PdbBuildTask(Task<Arc<solver::Solver>>);

/// An in-flight off-thread solve. Present only while a solve is running; its `cancel`
/// flag is set (and the resource dropped) when a repaint supersedes it.
#[derive(Resource)]
struct SolveTask {
    task: Task<Result<Vec<Move>, solver::SolveError>>,
    cancel: Arc<AtomicBool>,
    /// `None` for a manual / initial solve (always applied if not superseded).
    /// `Some(epoch)` for a live background re-solve: the move-epoch at dispatch.
    /// The result is applied only if it still matches `LiveMode`'s current epoch
    /// (no move arrived since dispatch); otherwise it is discarded — the cheap
    /// relist already kept the list valid.
    live_epoch: Option<u64>,
}

/// Live-sort mode: a toggle that solves the current state once, then live-updates
/// the step list as the user makes moves (see the module docs).
#[derive(Resource, Default, Clone, Copy)]
enum LiveMode {
    /// Live tracking off — the panel behaves as the classic static Solve/Run list.
    #[default]
    Off,
    /// Live tracking on, with bookkeeping for the debounced re-solve.
    On {
        /// Move-epoch counter: bumped on every applied move while On. Tags each
        /// background re-solve so a stale result (a move landed after dispatch)
        /// can be discarded.
        epoch: u64,
        /// Elapsed-seconds timestamp of the last applied move, for the debounce.
        last_move: f32,
        /// A re-solve is owed (a move changed the list since the last dispatch).
        resolve_pending: bool,
    },
}

/// Debounce window (seconds) of no moves before a background re-solve fires.
const RESOLVE_DEBOUNCE: f32 = 0.25;

/// The current solution + run/UI state shown in the panel.
#[derive(Resource)]
struct Solution {
    /// The computed solution moves, in order.
    moves: Vec<Move>,
    /// What the status line shows.
    status: SolveStatus,
    /// Step (0-based) currently animating during a Run, for the highlight.
    current: Option<usize>,
    /// Moves enqueued for the active run; `0` = not running.
    run_total: usize,
}

impl Default for Solution {
    fn default() -> Self {
        // Start in `Loading`: the tables build in the background from `Startup`,
        // and the status flips to `Idle` when they land.
        Solution {
            moves: Vec::new(),
            status: SolveStatus::Loading,
            current: None,
            run_total: 0,
        }
    }
}

/// Status-line state for the Solution panel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum SolveStatus {
    /// Tables ready, no solution computed yet.
    #[default]
    Idle,
    /// Loading / generating the Korf pattern databases in the background.
    Loading,
    /// A solve is running off-thread.
    Solving,
    /// Last solve found a solution of this many moves.
    Solved(usize),
    /// The current cube is already solved (empty solution).
    AlreadySolved,
    /// The cube just reached the solved state — either Live tracking emptied the
    /// list or a Run finished playing the whole solution. Shows "Solved!".
    JustSolved,
    /// The current cube is a physically impossible / invalid state.
    Unsolvable,
}

// --- Markers ------------------------------------------------------------------

/// The "Solve" button.
#[derive(Component)]
struct SolveButton;

/// The "Run" button.
#[derive(Component)]
struct RunButton;

/// The "Live" toggle button.
#[derive(Component)]
struct LiveButton;

/// The status `Text` line.
#[derive(Component)]
struct StatusText;

/// The container holding the per-move step rows.
#[derive(Component)]
struct StepList;

/// One step row; remembers its 0-based index into `Solution.moves`.
#[derive(Component, Clone, Copy)]
struct StepRow {
    index: usize,
}

// --- Layout constants ---------------------------------------------------------

const SOLVE_BUTTON_WIDTH: f32 = 80.0;
const STEP_FONT_SIZE: f32 = 14.0;

// --- Pure helpers (unit-tested) -----------------------------------------------

/// Which step (0-based) is currently active, given the run length and how many
/// moves remain (queued + the one animating). Returns `None` when the run is over
/// or not running. `remaining` = `MoveQueue.len() + (active ? 1 : 0)`.
fn current_step(run_total: usize, remaining: usize) -> Option<usize> {
    if run_total == 0 || remaining == 0 {
        return None;
    }
    // `saturating_sub` guards the `remaining > run_total` case (a stray move
    // queued beyond the run): it clamps to step 0 rather than underflowing.
    Some(run_total.saturating_sub(remaining).min(run_total - 1))
}

/// The status-line string for a `SolveStatus`.
fn status_text(status: SolveStatus) -> String {
    match status {
        SolveStatus::Idle => "Press Solve".to_string(),
        SolveStatus::Loading => {
            "Building solver tables (first run can take a minute)...".to_string()
        }
        SolveStatus::Solving => "Solving...".to_string(),
        SolveStatus::Solved(n) => format!("Solved in {n} moves"),
        SolveStatus::AlreadySolved => "Already solved".to_string(),
        SolveStatus::JustSolved => "Solved!".to_string(),
        SolveStatus::Unsolvable => "Unsolvable state".to_string(),
    }
}

/// Combine two same-face moves into the single move equivalent to doing both in
/// order, by adding their clockwise quarter-turns mod 4: `0` cancels (`None`),
/// `1` → Cw, `2` → Double, `3` → Ccw. Only valid when both are the same face.
fn combine_same_face(face: crate::cube::model::Face, a: Turn, b: Turn) -> Option<Move> {
    let q =
        (Move { face, turn: a }.quarter_turns_cw() + Move { face, turn: b }.quarter_turns_cw()) % 4;
    let turn = match q {
        0 => return None,
        1 => Turn::Cw,
        2 => Turn::Double,
        _ => Turn::Ccw,
    };
    Some(Move { face, turn })
}

/// Cheaply update the remaining-step list after the user applies `mv`, with no
/// solver call:
/// - if `mv` equals the next step, **pop** the front (the user played it);
/// - otherwise **prepend `mv.inverse()`** so the list still solves the cube,
///   merging with the new front when it shares a face (`R` then `R'` cancel out;
///   `R` then `R` → `R2`; etc.). A same-face combine that nets to a no-op drops
///   both entries.
///
/// The result is always a valid solution for the post-move state, computed in
/// O(len). Optimality is later restored by the debounced background re-solve.
fn relist_after_move(remaining: &[Move], mv: Move) -> Vec<Move> {
    // Correct move: pop the matched front.
    if remaining.first() == Some(&mv) {
        return remaining[1..].to_vec();
    }

    let inv = mv.inverse();
    // Try to merge the prepended inverse with a same-face front.
    if let Some((first, rest)) = remaining.split_first() {
        if first.face == inv.face {
            let mut out = match combine_same_face(inv.face, inv.turn, first.turn) {
                Some(combined) => vec![combined],
                None => Vec::new(), // they cancel — drop both
            };
            out.extend_from_slice(rest);
            return out;
        }
    }

    // No merge: just prepend the inverse.
    let mut out = Vec::with_capacity(remaining.len() + 1);
    out.push(inv);
    out.extend_from_slice(remaining);
    out
}

// --- Plugin -------------------------------------------------------------------

/// Wires the Solution panel: background table build, Solve/Run handlers, run
/// progress tracking, and the live-rendered step list.
pub struct SolverPlugin;

impl Plugin for SolverPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SolverReady>()
            .init_resource::<Solution>()
            .init_resource::<LiveMode>()
            .add_systems(Startup, (start_table_build, spawn_steps_panel))
            .add_systems(
                Update,
                (
                    poll_table_build,
                    handle_solve_button,
                    handle_live_button,
                    live_track,
                    dispatch_live_resolve,
                    poll_solve_task,
                    handle_run_button,
                    track_run_progress,
                    clear_solution_on_repaint.run_if(on_message::<ApplyState>),
                    style_live_button,
                    sync_step_rows,
                    update_step_text,
                    update_status_text,
                ),
            );
    }
}

// --- Background PDB build ------------------------------------------------------

/// Kick off the (slow) Korf PDB load-or-generate plus the in-memory search-table build
/// off-thread on the async compute pool, so the window opens immediately. `Solution`
/// defaults to `Loading`. The tables are built once here (inside [`solver::Solver::new`])
/// and reused by every solve.
fn start_table_build(mut commands: Commands) {
    let task = AsyncComputeTaskPool::get()
        .spawn(async move { Arc::new(solver::Solver::new(solver::build_or_load_pdbs())) });
    commands.insert_resource(PdbBuildTask(task));
}

/// Poll the in-flight build once per frame; when it completes, move the [`solver::Solver`]
/// into `SolverReady`, drop the task resource, and flip a still-`Loading` status to `Idle`
/// (a solve/repaint that happened meanwhile is left untouched).
fn poll_table_build(
    mut commands: Commands,
    task: Option<ResMut<PdbBuildTask>>,
    mut ready: ResMut<SolverReady>,
    mut solution: ResMut<Solution>,
) {
    let Some(mut task) = task else {
        return;
    };
    if let Some(built) = block_on(future::poll_once(&mut task.0)) {
        ready.0 = Some(built);
        commands.remove_resource::<PdbBuildTask>();
        if solution.status == SolveStatus::Loading {
            solution.status = SolveStatus::Idle;
        }
    }
}

// --- Off-thread solve dispatch -------------------------------------------------

/// Spawn an off-thread IDA* solve of the current cube state and install the
/// `SolveTask` resource. `live_epoch` is `None` for a manual / initial solve, or
/// `Some(epoch)` for a live background re-solve (tagging it for the generation
/// guard in [`poll_solve_task`]). The closure is `Send + 'static` — it captures
/// only the `Arc<Solver>`, an owned `CubeState`, and the cancel flag.
fn spawn_solve(
    commands: &mut Commands,
    solver: &Arc<solver::Solver>,
    cube: &Cube,
    live_epoch: Option<u64>,
) {
    let solver = Arc::clone(solver);
    let state = cube.0.to_state();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = Arc::clone(&cancel);
    let task =
        AsyncComputeTaskPool::get().spawn(async move { solver.solve(&state, &cancel_clone) });
    commands.insert_resource(SolveTask {
        task,
        cancel,
        live_epoch,
    });
}

// --- Button handlers ----------------------------------------------------------

/// On press of Solve: if the PDBs are ready and no solve is already running, spawn
/// an off-thread IDA* solve of the current cube state and show "Solving..."; the
/// result is collected by [`poll_solve_task`]. If the PDBs are still loading, report
/// that. The `With<SolveButton>` filter keeps this query disjoint from the Run
/// handler. The spawned closure is `Send + 'static` — it captures only the
/// `Arc<Pdbs>`, an owned `CubeState`, and the `Arc<AtomicBool>` cancel flag (no
/// Bevy refs).
#[allow(clippy::type_complexity)]
fn handle_solve_button(
    mut commands: Commands,
    mut interactions: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<SolveButton>),
    >,
    ready: Res<SolverReady>,
    solve_task: Option<Res<SolveTask>>,
    cube: Res<Cube>,
    mut solution: ResMut<Solution>,
) {
    for (interaction, mut bg) in &mut interactions {
        if *interaction == Interaction::Pressed {
            if solve_task.is_some() {
                // A solve is already running; ignore the press.
            } else {
                match ready.0.as_ref() {
                    Some(solver) => {
                        spawn_solve(&mut commands, solver, &cube, None);
                        // A new solve supersedes any old run highlight.
                        solution.status = SolveStatus::Solving;
                        solution.current = None;
                        solution.run_total = 0;
                    }
                    None => solution.status = SolveStatus::Loading,
                }
            }
        }
        set_button_color(interaction, &mut bg);
    }
}

/// On press of Live: toggle `LiveMode`. Turning it **On** dispatches an initial
/// off-thread solve of the current state (reusing the same dispatch as Solve) and
/// shows "Solving..."; from then on `live_track` keeps the list current as moves
/// land. Turning it **Off** just stops tracking; the displayed list is left as-is.
/// The `With<LiveButton>` filter keeps this query disjoint from the other handlers.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn handle_live_button(
    mut commands: Commands,
    interactions: Query<&Interaction, (Changed<Interaction>, With<LiveButton>)>,
    ready: Res<SolverReady>,
    solve_task: Option<Res<SolveTask>>,
    cube: Res<Cube>,
    time: Res<Time>,
    mut live: ResMut<LiveMode>,
    mut solution: ResMut<Solution>,
) {
    for interaction in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *live {
            LiveMode::On { .. } => {
                // Turning Live off — keep the current list, just stop tracking.
                *live = LiveMode::Off;
            }
            LiveMode::Off => {
                // Turning Live on: arm tracking and kick off the initial solve so
                // the list starts optimal. The epoch starts at 0 with nothing
                // pending; `live_track` advances it as moves land.
                *live = LiveMode::On {
                    epoch: 0,
                    last_move: time.elapsed_secs(),
                    resolve_pending: false,
                };
                match ready.0.as_ref() {
                    Some(solver) if solve_task.is_none() => {
                        spawn_solve(&mut commands, solver, &cube, None);
                        solution.status = SolveStatus::Solving;
                        solution.current = None;
                        solution.run_total = 0;
                    }
                    Some(_) => { /* a solve is already in flight; it will land. */ }
                    None => solution.status = SolveStatus::Loading,
                }
            }
        }
    }
}

/// Color the Live toggle every frame: highlighted (`BTN_PRESSED`) while On,
/// otherwise normal/hover like the other buttons. Mirrors `ui::style_scheme_toggles`.
fn style_live_button(
    live: Res<LiveMode>,
    mut buttons: Query<(&Interaction, &mut BackgroundColor), With<LiveButton>>,
) {
    let on = matches!(*live, LiveMode::On { .. });
    for (interaction, mut bg) in &mut buttons {
        bg.0 = if on {
            BTN_PRESSED
        } else if *interaction == Interaction::Hovered {
            BTN_HOVER
        } else {
            BTN_NORMAL
        };
    }
}

/// Track every applied move while Live is on (reads `MoveApplied` — fired at the
/// single choke-point for moves from any source). For each move: cheaply relist
/// via [`relist_after_move`] (pop the matched front, or prepend the merged
/// inverse), bump the move-epoch, stamp the time + mark a re-solve pending (the
/// debounce in [`dispatch_live_resolve`] later restores optimality), and update
/// the status. Reaching an empty list shows "Solved!". Does not touch the
/// run-highlight while live — the shrinking list is the progress.
fn live_track(
    mut moves: MessageReader<MoveApplied>,
    time: Res<Time>,
    mut live: ResMut<LiveMode>,
    mut solution: ResMut<Solution>,
) {
    let LiveMode::On {
        epoch,
        last_move,
        resolve_pending,
    } = &mut *live
    else {
        moves.clear(); // not live: don't accumulate stale messages
        return;
    };

    let mut changed = false;
    for &MoveApplied(mv) in moves.read() {
        solution.moves = relist_after_move(&solution.moves, mv);
        *epoch = epoch.wrapping_add(1);
        *last_move = time.elapsed_secs();
        *resolve_pending = true;
        changed = true;
    }
    if !changed {
        return;
    }

    // Live progress lives in the shrinking list, not the Run highlight.
    solution.current = None;
    solution.run_total = 0;
    solution.status = if solution.moves.is_empty() {
        SolveStatus::JustSolved
    } else {
        SolveStatus::Solved(solution.moves.len())
    };
}

/// Debounced, cancellable, generation-guarded background re-solve. When Live is on
/// with a pending re-solve, the user has paused (`>= RESOLVE_DEBOUNCE` since the
/// last move), and no solve is in flight, dispatch ONE off-thread solve from the
/// current state tagged with the current epoch, and clear the pending flag. A new
/// move (via `live_track`) re-arms the flag and re-stamps the time, so at most one
/// solve is ever in flight and it is always for a settled state. The status is
/// left showing the current list (no "Solving..." flicker for background refines).
fn dispatch_live_resolve(
    mut commands: Commands,
    ready: Res<SolverReady>,
    solve_task: Option<Res<SolveTask>>,
    cube: Res<Cube>,
    time: Res<Time>,
    mut live: ResMut<LiveMode>,
) {
    let LiveMode::On {
        epoch,
        last_move,
        resolve_pending,
    } = &mut *live
    else {
        return;
    };
    if !*resolve_pending || time.elapsed_secs() - *last_move < RESOLVE_DEBOUNCE {
        return;
    }
    let Some(solver) = ready.0.as_ref() else {
        return; // tables not ready yet; try again next frame (flag stays set)
    };
    if let Some(task) = solve_task {
        // A solve is in flight. If it's a now-stale live re-solve (a later move
        // bumped the epoch), cancel it so the next frame dispatches fresh from the
        // settled state — keeping at most one solve in flight, always for the
        // latest pause. A manual/initial solve (`live_epoch == None`) is left to
        // finish. The pending flag stays set so we retry once it clears.
        if matches!(task.live_epoch, Some(e) if e != *epoch) {
            task.cancel.store(true, Ordering::Relaxed);
        }
        return;
    }
    spawn_solve(&mut commands, solver, &cube, Some(*epoch));
    *resolve_pending = false;
}

/// Poll the in-flight off-thread solve once per frame; when it completes, fold the
/// result into the `Solution` panel and drop the task resource. A `Cancelled` result
/// is ignored (a repaint already reset the panel).
fn poll_solve_task(
    mut commands: Commands,
    task: Option<ResMut<SolveTask>>,
    live: Res<LiveMode>,
    mut solution: ResMut<Solution>,
) {
    let Some(mut task) = task else {
        return;
    };
    let Some(result) = block_on(future::poll_once(&mut task.task)) else {
        return;
    };
    commands.remove_resource::<SolveTask>();

    match task.live_epoch {
        // Background live re-solve: apply the optimal list ONLY if no move landed
        // since dispatch (the epoch is unchanged and Live is still on). Otherwise
        // discard — the cheap relist already kept `solution.moves` valid. Never
        // touches the status if discarded.
        Some(dispatch_epoch) => {
            let current_epoch = match *live {
                LiveMode::On { epoch, .. } => Some(epoch),
                LiveMode::Off => None,
            };
            if current_epoch == Some(dispatch_epoch) {
                if let Ok(moves) = result {
                    solution.status = if moves.is_empty() {
                        SolveStatus::JustSolved
                    } else {
                        SolveStatus::Solved(moves.len())
                    };
                    solution.moves = moves;
                    solution.current = None;
                    solution.run_total = 0;
                }
                // An `Unsolvable`/`Cancelled` background result is silently dropped:
                // the live state is reachable by construction, so this only happens
                // if a repaint raced — the relisted list stays shown.
            }
        }
        // Manual / initial solve: behave as before. If a repaint (Reset / POST
        // /state) superseded this solve in the same frame, `clear_solution_on_repaint`
        // reset the status away from `Solving`; discard the now-stale result rather
        // than overwriting the repainted panel. The Update systems are unordered, so
        // guard on the status, not on run order.
        None => {
            let superseded = solution.status != SolveStatus::Solving;
            match result {
                Ok(moves) if !superseded => {
                    solution.status = if moves.is_empty() {
                        SolveStatus::AlreadySolved
                    } else {
                        SolveStatus::Solved(moves.len())
                    };
                    solution.moves = moves;
                    solution.current = None;
                    solution.run_total = 0;
                }
                Err(solver::SolveError::Unsolvable) if !superseded => {
                    solution.moves.clear();
                    solution.status = SolveStatus::Unsolvable;
                }
                // Cancelled, or superseded by a repaint: discard the result.
                _ => {}
            }
        }
    }
}

/// On press of Run: enqueue every solution move onto the shared `MoveQueue` and
/// arm run-progress tracking. The existing animator drains the queue one move at
/// a time. The `With<RunButton>` filter keeps this query disjoint.
#[allow(clippy::type_complexity)]
fn handle_run_button(
    mut interactions: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<RunButton>),
    >,
    mut queue: ResMut<MoveQueue>,
    mut solution: ResMut<Solution>,
) {
    for (interaction, mut bg) in &mut interactions {
        if *interaction == Interaction::Pressed && !solution.moves.is_empty() {
            for &m in &solution.moves {
                queue.0.push_back(m);
            }
            solution.run_total = solution.moves.len();
            solution.current = None;
        }
        set_button_color(interaction, &mut bg);
    }
}

// --- Run progress / highlight -------------------------------------------------

/// While a run is active, derive the highlighted step from how many moves are
/// still queued + the one animating, and clear the run when the queue drains.
///
/// Manual moves enqueued mid-run aren't a supported case; the highlight
/// self-corrects once the queue drains back to the run's tail.
fn track_run_progress(
    queue: Res<MoveQueue>,
    active: Res<ActiveMove>,
    mut solution: ResMut<Solution>,
) {
    if solution.run_total == 0 {
        return;
    }
    let remaining = queue.0.len() + if active.0.is_some() { 1 } else { 0 };
    solution.current = current_step(solution.run_total, remaining);
    if remaining == 0 {
        solution.run_total = 0;
        // The run finished and the cube is now solved: clear the list so the
        // just-played solution can't be Run again to scramble it, and celebrate.
        solution.moves.clear();
        solution.current = None;
        solution.status = SolveStatus::JustSolved;
    }
}

/// A full repaint (Reset or `POST /state`) invalidates the displayed solution, so
/// clear it and return the panel to `Idle`. This also turns Live **off**: the
/// repaint is a fresh, unrelated state, so live tracking of the prior solution no
/// longer applies. Any in-flight off-thread solve is now stale: set its cancel flag
/// (the detached task observes it and exits with `Cancelled`, which `poll_solve_task`
/// ignores) and drop the task resource. Gated on `ApplyState`.
fn clear_solution_on_repaint(
    mut commands: Commands,
    solve_task: Option<Res<SolveTask>>,
    mut live: ResMut<LiveMode>,
    mut solution: ResMut<Solution>,
) {
    solution.moves.clear();
    solution.current = None;
    solution.run_total = 0;
    solution.status = SolveStatus::Idle;
    *live = LiveMode::Off;
    if let Some(task) = solve_task {
        task.cancel.store(true, Ordering::Relaxed);
        commands.remove_resource::<SolveTask>();
    }
}

// --- Panel UI -----------------------------------------------------------------

/// Spawn the right-docked Solution panel: a Solve/Run header row, a status line,
/// and an (initially empty) step list, mirroring the left panel's chrome.
fn spawn_steps_panel(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(6.0),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            // Same trick as the left panel: give the background an `Interaction`
            // so the camera's `pointer_over_ui` guard ignores drags on the gaps.
            Interaction::default(),
        ))
        .with_children(|panel| {
            // 1. Header row: Solve + Run + Live (the live-sort toggle).
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|header| {
                    spawn_labeled_button(header, SOLVE_BUTTON_WIDTH, "Solve", SolveButton);
                    spawn_labeled_button(header, SOLVE_BUTTON_WIDTH, "Run", RunButton);
                    spawn_labeled_button(header, SOLVE_BUTTON_WIDTH, "Live", LiveButton);
                });

            // 2. Status line.
            panel.spawn((
                Text::new(""),
                TextFont {
                    font_size: STEP_FONT_SIZE,
                    ..default()
                },
                TextColor(LABEL_COLOR),
                StatusText,
            ));

            // 3. Step list (starts empty; filled by `sync_step_rows`).
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                StepList,
                Interaction::default(),
            ));
        });
}

// --- List rendering -----------------------------------------------------------

/// Rebuild the step rows only when the move count changes: despawn the existing
/// rows and spawn exactly `moves.len()` fresh `Text` rows under `StepList`. The
/// per-row text + highlight is set every frame by `update_step_text`.
fn sync_step_rows(
    mut commands: Commands,
    solution: Res<Solution>,
    list: Query<Entity, With<StepList>>,
    rows: Query<(Entity, &StepRow)>,
) {
    if !solution.is_changed() {
        return;
    }
    let row_count = rows.iter().count();
    if row_count == solution.moves.len() {
        return;
    }
    let Ok(list) = list.single() else {
        return;
    };

    // Despawn the stale rows.
    for (entity, _) in &rows {
        commands.entity(entity).despawn();
    }
    // Spawn one row per solution move.
    commands.entity(list).with_children(|parent| {
        for index in 0..solution.moves.len() {
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: STEP_FONT_SIZE,
                    ..default()
                },
                TextColor(LABEL_COLOR),
                StepRow { index },
            ));
        }
    });
}

/// Set each step row's label + highlight every frame (cheap: <= 23 short
/// strings). The Beginner label is recomputed against the live camera basis so
/// it tracks orbiting; the active step (during a Run) is highlighted.
fn update_step_text(
    solution: Res<Solution>,
    scheme: Res<ControlScheme>,
    orbit: Res<OrbitCamera>,
    mut rows: Query<(&StepRow, &mut Text, &mut TextColor)>,
) {
    for (row, mut text, mut color) in &mut rows {
        // Guard the one frame before `sync_step_rows` catches up to a shorter list.
        let Some(&mv) = solution.moves.get(row.index) else {
            continue;
        };
        let move_label = match *scheme {
            ControlScheme::Standard => mv.notation(),
            ControlScheme::Beginner => {
                let (rel, turn) = describe(orbit.basis(), mv);
                rel_label(rel, turn)
            }
        };
        *text = Text::new(format!("{}. {}", row.index + 1, move_label));
        color.0 = if Some(row.index) == solution.current {
            BTN_PRESSED
        } else {
            LABEL_COLOR
        };
    }
}

/// Mirror the current `SolveStatus` into the status line.
fn update_status_text(solution: Res<Solution>, mut text: Query<&mut Text, With<StatusText>>) {
    // The status changes only a handful of times per session; skip the per-frame
    // string rebuild when nothing changed. (`update_step_text` deliberately runs
    // every frame so Beginner labels track the orbiting camera.)
    if !solution.is_changed() {
        return;
    }
    if let Ok(mut text) = text.single_mut() {
        *text = Text::new(status_text(solution.status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_step_not_running() {
        assert_eq!(current_step(0, 0), None);
        assert_eq!(current_step(0, 5), None);
    }

    #[test]
    fn current_step_over() {
        assert_eq!(current_step(5, 0), None);
    }

    #[test]
    fn current_step_first_mid_last() {
        assert_eq!(current_step(5, 5), Some(0));
        assert_eq!(current_step(5, 3), Some(2));
        assert_eq!(current_step(5, 1), Some(4));
    }

    #[test]
    fn current_step_clamps_below_total() {
        // `remaining` larger than the run length (a stray extra queued move)
        // must never index past the last step.
        assert_eq!(current_step(5, 6), Some(0));
        assert_eq!(current_step(5, 100), Some(0));
    }

    #[test]
    fn status_text_maps_each_variant() {
        assert_eq!(status_text(SolveStatus::Idle), "Press Solve");
        assert_eq!(
            status_text(SolveStatus::Loading),
            "Building solver tables (first run can take a minute)..."
        );
        assert_eq!(status_text(SolveStatus::Solving), "Solving...");
        assert_eq!(status_text(SolveStatus::Solved(12)), "Solved in 12 moves");
        assert_eq!(status_text(SolveStatus::AlreadySolved), "Already solved");
        assert_eq!(status_text(SolveStatus::JustSolved), "Solved!");
        assert_eq!(status_text(SolveStatus::Unsolvable), "Unsolvable state");
    }

    // --- Live-sort relist helper ----------------------------------------------

    /// Parse a notation string into a `Move` for terse test fixtures.
    fn m(s: &str) -> Move {
        Move::parse(s).unwrap()
    }

    /// Parse a space-separated move list.
    fn list(s: &str) -> Vec<Move> {
        s.split_whitespace().map(m).collect()
    }

    #[test]
    fn relist_correct_move_pops_front() {
        // Playing the next step removes it from the list, leaving the tail intact.
        assert_eq!(relist_after_move(&list("R U F"), m("R")), list("U F"));
        // Down to the last step → empty (solved).
        assert_eq!(relist_after_move(&list("R"), m("R")), Vec::<Move>::new());
    }

    #[test]
    fn relist_wrong_move_prepends_inverse() {
        // A deviation onto a different face prepends the move's inverse so the list
        // still solves the post-move state.
        assert_eq!(relist_after_move(&list("R U"), m("F")), list("F' R U"));
        assert_eq!(relist_after_move(&list("R U"), m("D'")), list("D R U"));
    }

    #[test]
    fn relist_same_face_merge_combines() {
        // Prepended inverse merges with a same-face front by adding quarter-turns.
        // Front R, play R' → inverse is R; R (1) + front R (1) = 2 → R2.
        assert_eq!(relist_after_move(&list("R U"), m("R'")), list("R2 U"));
        // Front R, play R2 → inverse is R2; R2 (2) + front R (1) = 3 → Ccw.
        assert_eq!(relist_after_move(&list("R U"), m("R2")), list("R' U"));
        // Front R2, play R → inverse is R'; R' (3) + front R2 (2) = 5 ≡ 1 → Cw.
        assert_eq!(relist_after_move(&list("R2 U"), m("R")), list("R U"));
    }

    #[test]
    fn relist_user_undo_collapses() {
        // The classic self-undo: list starts R, the user plays R (correct) then
        // immediately R' (undo). First pop leaves the tail; then R''s inverse R
        // merges with the new front.
        let after_r = relist_after_move(&list("R U"), m("R"));
        assert_eq!(after_r, list("U"));
        // Now undo R: inverse is R, prepended onto [U] (different face) → R U,
        // i.e. the list grows back to require fixing the undo.
        assert_eq!(relist_after_move(&after_r, m("R'")), list("R U"));

        // And a same-face undo collapses: front R, play R' inverse R merges with R
        // → R2; play R' again inverse R merges with R2 → R; etc.
        assert_eq!(relist_after_move(&list("R'"), m("R")), list("R2"));
    }

    #[test]
    fn relist_empty_remaining_prepends_inverse() {
        // From a solved/empty list, any move just prepends its inverse.
        assert_eq!(relist_after_move(&[], m("R")), list("R'"));
        assert_eq!(relist_after_move(&[], m("U2")), list("U2"));
    }

    #[test]
    fn combine_same_face_arithmetic() {
        use crate::cube::model::Face;
        // Cw + Cw = Double; Cw + Cw + ... handled via the mod-4 sum.
        assert_eq!(
            combine_same_face(Face::R, Turn::Cw, Turn::Cw),
            Some(m("R2"))
        );
        // Cw + Ccw cancels.
        assert_eq!(combine_same_face(Face::R, Turn::Cw, Turn::Ccw), None);
        // Double + Double cancels.
        assert_eq!(combine_same_face(Face::R, Turn::Double, Turn::Double), None);
        // Double + Cw = Ccw (2 + 1 = 3).
        assert_eq!(
            combine_same_face(Face::R, Turn::Double, Turn::Cw),
            Some(m("R'"))
        );
    }

    // --- Headless `live_track` behavior ---------------------------------------

    /// Build a minimal app with just the resources + messages `live_track` reads,
    /// pre-seeded into `LiveMode::On` with a known solution, and the system added.
    fn live_app(moves: Vec<Move>) -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.add_message::<MoveApplied>();
        app.insert_resource(LiveMode::On {
            epoch: 0,
            last_move: 0.0,
            resolve_pending: false,
        });
        app.insert_resource(Solution {
            moves,
            status: SolveStatus::Solved(0),
            current: None,
            run_total: 0,
        });
        app.add_systems(Update, live_track);
        app
    }

    #[test]
    fn live_track_pops_correct_move() {
        let mut app = live_app(list("R U F"));
        app.world_mut().write_message(MoveApplied(m("R")));
        app.update();
        let solution = app.world().resource::<Solution>();
        assert_eq!(solution.moves, list("U F"));
        // Epoch bumped, re-solve armed.
        let LiveMode::On {
            epoch,
            resolve_pending,
            ..
        } = *app.world().resource::<LiveMode>()
        else {
            panic!("expected LiveMode::On");
        };
        assert_eq!(epoch, 1);
        assert!(resolve_pending);
    }

    #[test]
    fn live_track_prepends_inverse_on_wrong_move() {
        let mut app = live_app(list("R U"));
        app.world_mut().write_message(MoveApplied(m("F")));
        app.update();
        assert_eq!(app.world().resource::<Solution>().moves, list("F' R U"));
    }

    #[test]
    fn live_track_solved_status_when_empty() {
        let mut app = live_app(list("R"));
        app.world_mut().write_message(MoveApplied(m("R")));
        app.update();
        let solution = app.world().resource::<Solution>();
        assert!(solution.moves.is_empty());
        assert!(solution.status == SolveStatus::JustSolved);
    }

    /// When a Run drains (queue empty, nothing animating) `track_run_progress`
    /// clears the list so it can't be re-run, and shows the solved celebration.
    #[test]
    fn track_run_progress_clears_list_when_run_finishes() {
        let mut app = App::new();
        app.init_resource::<MoveQueue>();
        app.init_resource::<ActiveMove>();
        app.insert_resource(Solution {
            moves: list("R U F"),
            status: SolveStatus::Solved(3),
            current: Some(2),
            run_total: 3,
        });
        app.add_systems(Update, track_run_progress);
        app.update();
        let solution = app.world().resource::<Solution>();
        assert!(solution.moves.is_empty());
        assert_eq!(solution.run_total, 0);
        assert_eq!(solution.current, None);
        assert_eq!(solution.status, SolveStatus::JustSolved);
    }
}
