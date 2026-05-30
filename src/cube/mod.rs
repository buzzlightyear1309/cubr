// Cube plugin: owns the core resources/events and (Phase 2) the spawn + sync +
// lighting wiring. Animation (Phase 4) registers itself separately later.

use bevy::prelude::*;

pub mod animation;
pub mod core;
pub mod model;
pub mod spawn;

use self::animation::{animate_move, apply_state, start_move, ActiveMove};
use self::core::CubeCore;
use self::model::{CubeState, Move};
use self::spawn::{spawn_cubies, sync_visuals, CubeMaterials};

/// The live cube core (single source of truth).
#[derive(Resource)]
pub struct Cube(pub CubeCore);

/// Pending moves to animate, one at a time.
#[derive(Resource, Default)]
pub struct MoveQueue(pub std::collections::VecDeque<Move>);

/// Request an instant repaint to an arbitrary state (POST /state).
///
/// A buffered message (Bevy 0.18 renamed buffered "events" to "messages";
/// `#[derive(Event)]` is now the observer/trigger path, not this one). Read with
/// `MessageReader<ApplyState>`, registered via `add_message`.
#[derive(Message)]
pub struct ApplyState(pub CubeState);

/// The core was mutated -> sync the Bevy visuals. Buffered message (see above).
#[derive(Message)]
pub struct CoreChanged;

/// Owns the cube resources/events + spawn + the sync system. (Animation is
/// wired by Phase 4; the orbit camera + ambient fill by Phase 3's CameraPlugin.)
pub struct CubePlugin;

impl Plugin for CubePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Cube(CubeCore::solved()))
            .init_resource::<MoveQueue>()
            // Phase 4: tracks the in-flight layer turn (None = idle).
            .init_resource::<ActiveMove>()
            // `CubeMaterials` needs `Assets<StandardMaterial>`, so build it via
            // `FromWorld` at `init_resource` time (assets plugin is already up).
            .init_resource::<CubeMaterials>()
            // Buffered messages in 0.18 are registered with `add_message`.
            .add_message::<CoreChanged>()
            .add_message::<ApplyState>()
            // Startup order: materials already exist (resource above), the cube
            // exists, so spawn then do one sync to land on the integer grid.
            .add_systems(Startup, spawn_lighting)
            .add_systems(Startup, (spawn_cubies, sync_visuals).chain())
            // Phase 4: drive the move queue + animation. `start_move` pops/applies
            // and snapshots; `animate_move` eases the visual and fires
            // `CoreChanged` on completion; `apply_state` handles instant repaints.
            .add_systems(Update, (start_move, animate_move, apply_state).chain())
            // On every later mutation, snap visuals back onto the core. Ordered
            // after the writers so a `CoreChanged` emitted this frame is seen
            // the same frame — entities snap onto the grid with no float lag.
            .add_systems(
                Update,
                sync_visuals
                    .run_if(on_message::<CoreChanged>)
                    .after(animate_move)
                    .after(apply_state),
            );
    }
}

/// Basic lighting: a key directional light. Ambient fill is attached to the
/// camera (Bevy 0.18 made `AmbientLight` a per-camera component).
fn spawn_lighting(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 6000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(5.0, 8.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
