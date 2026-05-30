use bevy::prelude::*;

pub mod server;
pub mod types;

/// Runs the tiny_http server on its own thread and bridges to Bevy via mpsc. Phase 6 implements it.
pub struct ApiPlugin;

impl Plugin for ApiPlugin {
    fn build(&self, _app: &mut App) {
        // Phase 6 spawns the server thread and adds the channel-drain system.
    }
}
