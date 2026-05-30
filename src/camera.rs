use bevy::prelude::*;

/// Orbit camera: left-drag rotate (ignored over UI) + wheel zoom. Phase 3 implements it.
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, _app: &mut App) {
        // Phase 3 spawns the Camera3d and adds the orbit/zoom systems.
    }
}
