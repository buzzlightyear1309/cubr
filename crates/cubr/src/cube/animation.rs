// Phase 4: the move-animation system. Consumes `MoveQueue` one move at a time.
//
// Model: the pure `CubeCore` is the single source of truth and is mutated
// *immediately* when a move starts. The visual layer then animates from the
// stored pre-move poses to the (already-applied) post-move core pose by rotating
// the moving cubies about the move's axis through the world origin. On
// completion we fire `CoreChanged`, and `sync_visuals` (gated on it) snaps the
// entities exactly onto the integer grid, restoring the §1 invariant.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use crate::cube::model::{Move, Turn};
use crate::cube::ApplyState;
use crate::cube::{CoreChanged, Cube, MoveApplied, MoveQueue};

/// Duration of a single layer turn, in seconds (~the spec's 0.25s).
const MOVE_DURATION: f32 = 0.25;

/// The currently animating move, or `None` when idle.
#[derive(Resource, Default)]
pub struct ActiveMove(pub Option<MoveAnim>);

/// In-flight animation state for a single layer turn.
pub struct MoveAnim {
    /// The move being animated (its core mutation is already applied).
    pub mv: Move,
    /// The participating cubie entities, each paired with the world `Transform`
    /// it had *before* the move was applied. Each frame we re-derive the
    /// animated pose from this fixed pre-move pose, so float error can't
    /// accumulate and no per-frame "previous angle" bookkeeping is needed.
    pub entities: Vec<(Entity, Transform)>,
    /// Elapsed animation time, in seconds.
    pub elapsed: f32,
}

/// Smoothstep easing: 0->0, 1->1, zero slope at both ends.
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Total signed angle (radians) to rotate about the (outward, unit) axis for a
/// right-handed rotation. Clockwise-as-seen-from-outside is -90° about the
/// outward normal, matching the core's `rot_cw`. ±PI land identically, so the
/// sign of a double turn is irrelevant.
fn total_angle(turn: Turn) -> f32 {
    match turn {
        Turn::Cw => -FRAC_PI_2,
        Turn::Ccw => FRAC_PI_2,
        Turn::Double => PI,
    }
}

/// Start a move when idle and the queue is non-empty: snapshot the moving layer
/// entities (by their *pre-move* homes), apply the move to the core immediately,
/// and stash the pre-move transforms for the animator. Guarded so exactly one
/// move animates at a time; the rest wait in the queue.
pub fn start_move(
    mut active: ResMut<ActiveMove>,
    mut queue: ResMut<MoveQueue>,
    mut cube: ResMut<Cube>,
    cubies: Query<(Entity, &super::spawn::Cubie, &Transform)>,
    mut move_applied: MessageWriter<MoveApplied>,
) {
    if active.0.is_some() {
        return; // a move is already animating
    }
    let Some(m) = queue.0.pop_front() else {
        return; // nothing queued
    };

    // Homes of the cubies in the moving layer, computed BEFORE applying the
    // move (afterwards `layer` would report the new occupants).
    let homes: Vec<IVec3> = cube
        .0
        .layer(m)
        .iter()
        .map(|&i| cube.0.cubies()[i].home)
        .collect();

    // Resolve those homes to the matching Bevy entities, capturing each one's
    // current (pre-move) transform as the animation's fixed starting pose.
    let entities: Vec<(Entity, Transform)> = cubies
        .iter()
        .filter(|(_, cubie, _)| homes.contains(&cubie.home))
        .map(|(entity, _, transform)| (entity, *transform))
        .collect();

    // Mutate the source of truth now. We do NOT fire CoreChanged yet — that
    // would snap the visuals instantly; instead we animate toward this pose and
    // fire CoreChanged on completion.
    cube.0.apply(m);
    // Announce the applied move at this single choke-point so the live-sort step
    // list can track every move (any source) without polling the queue. Does not
    // affect the animation in any way.
    move_applied.write(MoveApplied(m));

    active.0 = Some(MoveAnim {
        mv: m,
        entities,
        elapsed: 0.0,
    });
}

/// Advance the active animation each frame. Sets each moving cubie's transform
/// to its pre-move pose rotated by the eased angle about the move axis through
/// the origin. On completion, clears `ActiveMove` and fires `CoreChanged` so the
/// sync system lands the entities exactly on the integer grid.
pub fn animate_move(
    time: Res<Time>,
    mut active: ResMut<ActiveMove>,
    mut transforms: Query<&mut Transform>,
    mut core_changed: MessageWriter<CoreChanged>,
) {
    let Some(anim) = active.0.as_mut() else {
        return;
    };

    anim.elapsed += time.delta_secs();
    let t = (anim.elapsed / MOVE_DURATION).clamp(0.0, 1.0);
    let eased = smoothstep(t);

    let axis = anim.mv.axis().as_vec3(); // already unit length
    let angle = eased * total_angle(anim.mv.turn);
    let rotation = Quat::from_axis_angle(axis, angle);

    // The cube center is the world origin, so the layer pivots about Vec3::ZERO.
    for &(entity, pre_move) in &anim.entities {
        if let Ok(mut transform) = transforms.get_mut(entity) {
            *transform = pre_move;
            transform.rotate_around(Vec3::ZERO, rotation);
        }
    }

    if t >= 1.0 {
        active.0 = None;
        // sync_visuals (gated on CoreChanged) snaps entities onto the already-
        // applied core pose — we deliberately don't set final transforms here.
        core_changed.write(CoreChanged);
    }
}

/// Handle `POST /state` repaints: paint the core to the requested state, drop
/// any queued / in-flight move, and fire `CoreChanged` for an instant repaint
/// (no animation).
pub fn apply_state(
    mut events: MessageReader<ApplyState>,
    mut cube: ResMut<Cube>,
    mut queue: ResMut<MoveQueue>,
    mut active: ResMut<ActiveMove>,
    mut core_changed: MessageWriter<CoreChanged>,
) {
    // Apply only the latest requested state (intermediate ones would be
    // overwritten anyway); `.read().last()` still fully drains the reader so
    // events aren't reprocessed next frame.
    if let Some(ApplyState(state)) = events.read().last() {
        cube.0.paint(state);
        queue.0.clear();
        active.0 = None;
        core_changed.write(CoreChanged);
    }
}
