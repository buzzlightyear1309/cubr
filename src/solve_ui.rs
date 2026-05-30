//! Right-docked "Solution" panel: a **Solve** button computes a solution for the
//! current cube via the pure [`crate::solver`] adapter and lists the moves; a
//! **Run** button animates that solution by enqueuing the moves onto the shared
//! [`MoveQueue`] (the existing animator drains it — no new animation code here).
//!
//! The step list re-renders live in the active [`ControlScheme`]: Standard
//! notation (`R`, `U'`, `F2`) or Beginner view-relative wording (`Front CW`),
//! the latter recomputed every frame against the current camera basis so it
//! tracks orbiting. The step currently animating during a Run is highlighted.
//!
//! The solver's (slow) two-phase tables are built **once at startup, off-thread**
//! via `AsyncComputeTaskPool`, so the window opens immediately and the Solve
//! button simply reports "Building solver tables..." until they land.
//!
//! This is a pure presentation/input layer: it reads `Cube` + the camera basis,
//! ends at `MoveQueue` / `ApplyState`, and never reaches into the frozen engine.

use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

use crate::camera::OrbitCamera;
use crate::cube::animation::ActiveMove;
use crate::cube::model::Move;
use crate::cube::{ApplyState, Cube, MoveQueue};
use crate::solver;
use crate::ui::{
    set_button_color, spawn_labeled_button, ControlScheme, BTN_PRESSED, LABEL_COLOR, PANEL_BG,
};
use crate::view_relative::{describe, rel_label};

/// The solver tables, once the background build completes (`None` while loading).
#[derive(Resource, Default)]
struct SolveTables(Option<solver::DataTable>);

/// The in-flight startup table build. Removed once polled to completion.
#[derive(Resource)]
struct TableBuildTask(Task<solver::DataTable>);

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
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum SolveStatus {
    /// Tables ready, no solution computed yet.
    #[default]
    Idle,
    /// Building the two-phase tables in the background.
    Loading,
    /// Last solve found a solution of this many moves.
    Solved(usize),
    /// The current cube is already solved (empty solution).
    AlreadySolved,
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
        SolveStatus::Loading => "Building solver tables...".to_string(),
        SolveStatus::Solved(n) => format!("Solved in {n} moves"),
        SolveStatus::AlreadySolved => "Already solved".to_string(),
        SolveStatus::Unsolvable => "Unsolvable state".to_string(),
    }
}

// --- Plugin -------------------------------------------------------------------

/// Wires the Solution panel: background table build, Solve/Run handlers, run
/// progress tracking, and the live-rendered step list.
pub struct SolverPlugin;

impl Plugin for SolverPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SolveTables>()
            .init_resource::<Solution>()
            .add_systems(Startup, (start_table_build, spawn_steps_panel))
            .add_systems(
                Update,
                (
                    poll_table_build,
                    handle_solve_button,
                    handle_run_button,
                    track_run_progress,
                    clear_solution_on_repaint.run_if(on_message::<ApplyState>),
                    sync_step_rows,
                    update_step_text,
                    update_status_text,
                ),
            );
    }
}

// --- Background table build ---------------------------------------------------

/// Kick off the (slow) two-phase table build off-thread on the async compute
/// pool, so the window opens immediately. `Solution` defaults to `Loading`.
fn start_table_build(mut commands: Commands) {
    let task = AsyncComputeTaskPool::get().spawn(async move { solver::build_tables() });
    commands.insert_resource(TableBuildTask(task));
}

/// Poll the in-flight build once per frame; when it completes, move the tables
/// into `SolveTables`, drop the task resource, and flip a still-`Loading` status
/// to `Idle` (a solve/repaint that happened meanwhile is left untouched).
fn poll_table_build(
    mut commands: Commands,
    task: Option<ResMut<TableBuildTask>>,
    mut tables: ResMut<SolveTables>,
    mut solution: ResMut<Solution>,
) {
    let Some(mut task) = task else {
        return;
    };
    if let Some(built) = block_on(future::poll_once(&mut task.0)) {
        tables.0 = Some(built);
        commands.remove_resource::<TableBuildTask>();
        if solution.status == SolveStatus::Loading {
            solution.status = SolveStatus::Idle;
        }
    }
}

// --- Button handlers ----------------------------------------------------------

/// On press of Solve: if the tables are ready, solve the current cube state and
/// store the result; otherwise report that the tables are still building. The
/// `With<SolveButton>` filter keeps this query disjoint from the Run handler.
#[allow(clippy::type_complexity)]
fn handle_solve_button(
    mut interactions: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<SolveButton>),
    >,
    tables: Res<SolveTables>,
    cube: Res<Cube>,
    mut solution: ResMut<Solution>,
) {
    for (interaction, mut bg) in &mut interactions {
        if *interaction == Interaction::Pressed {
            match tables.0.as_ref() {
                Some(tables) => {
                    let state = cube.0.to_state();
                    solution.current = None;
                    solution.run_total = 0;
                    match solver::solve(tables, &state) {
                        Ok(moves) => {
                            solution.status = if moves.is_empty() {
                                SolveStatus::AlreadySolved
                            } else {
                                SolveStatus::Solved(moves.len())
                            };
                            solution.moves = moves;
                        }
                        Err(_) => {
                            solution.moves.clear();
                            solution.status = SolveStatus::Unsolvable;
                        }
                    }
                }
                None => solution.status = SolveStatus::Loading,
            }
        }
        set_button_color(interaction, &mut bg);
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
    }
}

/// A full repaint (Reset or `POST /state`) invalidates the displayed solution,
/// so clear it and return the panel to `Idle`. Gated on `ApplyState`.
fn clear_solution_on_repaint(mut solution: ResMut<Solution>) {
    solution.moves.clear();
    solution.current = None;
    solution.run_total = 0;
    solution.status = SolveStatus::Idle;
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
            // 1. Header row: Solve + Run.
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|header| {
                    spawn_labeled_button(header, SOLVE_BUTTON_WIDTH, "Solve", SolveButton);
                    spawn_labeled_button(header, SOLVE_BUTTON_WIDTH, "Run", RunButton);
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
            "Building solver tables..."
        );
        assert_eq!(status_text(SolveStatus::Solved(12)), "Solved in 12 moves");
        assert_eq!(status_text(SolveStatus::AlreadySolved), "Already solved");
        assert_eq!(status_text(SolveStatus::Unsolvable), "Unsolvable state");
    }
}
