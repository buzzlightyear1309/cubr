// Phase 0 stubs: resource/struct defined but not yet driven (Phase 4).
#![allow(dead_code)]

use bevy::prelude::*;

use crate::cube::model::Move;

/// The currently animating move, or `None` when idle.
#[derive(Resource, Default)]
pub struct ActiveMove(pub Option<MoveAnim>);

/// In-flight animation state for a single layer turn. Phase 4 fills this in.
#[allow(dead_code)]
pub struct MoveAnim {
    /// The move being animated.
    pub mv: Move,
    /// Entities (cubies) participating in this turn.
    pub entities: Vec<Entity>,
    /// Elapsed animation time, in seconds.
    pub elapsed: f32,
}
