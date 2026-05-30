// Phase 0 stubs: resources/events defined but not yet registered (Phase 2+).
#![allow(dead_code)]

use bevy::prelude::*;

pub mod animation;
pub mod core;
pub mod model;
pub mod spawn;

use self::core::CubeCore;
use self::model::{CubeState, Move};

/// The live cube core (single source of truth).
#[derive(Resource)]
pub struct Cube(pub CubeCore);

/// Pending moves to animate, one at a time.
#[derive(Resource, Default)]
pub struct MoveQueue(pub std::collections::VecDeque<Move>);

/// Request an instant repaint to an arbitrary state (POST /state).
#[derive(Event)]
pub struct ApplyState(pub CubeState);

/// The core was mutated -> sync the Bevy visuals.
#[derive(Event)]
pub struct CoreChanged;

/// Owns the cube resources/events + spawn + animation + the sync system.
pub struct CubePlugin;

impl Plugin for CubePlugin {
    fn build(&self, _app: &mut App) {
        // Phase 2+ wires resources, events, spawn, sync, and animation systems here.
        // Kept empty in Phase 0 so the crate compiles without behavior.
    }
}
