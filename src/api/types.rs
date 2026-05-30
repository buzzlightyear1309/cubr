// Request/response types and the `Cmd` channel enum bridging the HTTP server
// thread to the Bevy world.

use serde::Deserialize;

use crate::cube::model::{CubeState, Move};

/// Body of `POST /move`: `{"move":"R"}`. The JSON key is `move`, a Rust
/// keyword, so the field is renamed.
#[derive(Deserialize)]
pub struct MoveRequest {
    #[serde(rename = "move")]
    pub mv: String,
}

/// A validated command sent from the server thread over the mpsc channel and
/// applied to the Bevy world by the drain system.
///
/// `CubeState` is ~54 bytes, so the size disparity between variants is small
/// and no boxing is needed.
pub enum Cmd {
    Move(Move),
    SetState(CubeState),
}
