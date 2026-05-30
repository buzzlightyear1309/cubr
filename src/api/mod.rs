use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;

use bevy::prelude::*;

pub mod server;
pub mod types;

use self::types::Cmd;
use crate::cube::{ApplyState, MoveQueue};

/// Holds the receiving end of the server-thread channel. `mpsc::Receiver` is
/// `Send` but not `Sync`; Bevy resources must be `Send + Sync`, so wrap it in a
/// `Mutex` (only the single drain system ever touches it).
#[derive(Resource)]
struct CmdReceiver(Mutex<Receiver<Cmd>>);

/// Runs the tiny_http server on its own thread and bridges to Bevy via mpsc.
pub struct ApiPlugin;

impl Plugin for ApiPlugin {
    fn build(&self, app: &mut App) {
        // `build` runs once: create the channel and spawn the (detached)
        // server thread that owns the `Sender`. Bevy keeps the `Receiver`.
        let (tx, rx) = mpsc::channel::<Cmd>();
        std::thread::spawn(move || server::run(tx));

        app.insert_resource(CmdReceiver(Mutex::new(rx)))
            .add_systems(Update, drain_commands);
    }
}

/// Drain every command received since last frame and apply it to the world.
/// Non-blocking: `try_recv` returns immediately once the queue is empty.
fn drain_commands(
    receiver: Res<CmdReceiver>,
    mut queue: ResMut<MoveQueue>,
    mut apply: MessageWriter<ApplyState>,
) {
    let Ok(rx) = receiver.0.lock() else {
        return; // poisoned mutex: skip this frame rather than crash
    };
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            Cmd::Move(m) => queue.0.push_back(m),
            Cmd::SetState(s) => {
                apply.write(ApplyState(s));
            }
        }
    }
}
