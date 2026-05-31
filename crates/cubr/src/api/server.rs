// HTTP server thread (tiny_http). Runs on a dedicated std::thread, validates
// requests here (so no cross-thread response round-trip is needed), and hands
// validated `Cmd`s to Bevy over the mpsc channel.

use std::io::Read;
use std::sync::mpsc::Sender;

use tiny_http::{Method, Response, Server};

use crate::api::types::{Cmd, MoveRequest};
use crate::cube::model::{CubeState, Move};

/// Cap on the request body we'll read. A valid `/move` body is ~12 bytes and a
/// `/state` body ~350 bytes, so anything past this is invalid and will simply
/// fail JSON parsing (-> 400). Bounds the read so a huge body can't exhaust RAM.
const MAX_BODY_BYTES: u64 = 64 * 1024;

/// Bind 127.0.0.1:3000 and serve forever. On bind failure, log and return so
/// the thread exits cleanly (the app keeps running without the API).
pub fn run(tx: Sender<Cmd>) {
    let server = match Server::http("127.0.0.1:3000") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cube API: failed to bind 127.0.0.1:3000: {e}");
            return;
        }
    };

    for mut request in server.incoming_requests() {
        // Read the body up front; on a read error treat it as empty so the
        // route handlers produce a clean 400 rather than panicking.
        let mut body = String::new();
        let _ = request
            .as_reader()
            .take(MAX_BODY_BYTES)
            .read_to_string(&mut body);

        let response = match (request.method(), request.url()) {
            (&Method::Post, "/move") => handle_move(&tx, &body),
            (&Method::Post, "/state") => handle_state(&tx, &body),
            _ => Response::from_string("not found").with_status_code(404),
        };

        // Ignore the result: a dropped client must not crash the server thread.
        let _ = request.respond(response);
    }
}

/// Parse `{"move":"R"}`, validate the move string, and enqueue it.
fn handle_move(tx: &Sender<Cmd>, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let req: MoveRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Response::from_string(format!("invalid /move JSON: {e}")).with_status_code(400);
        }
    };
    match Move::parse(&req.mv) {
        Some(mv) => {
            let _ = tx.send(Cmd::Move(mv));
            Response::from_string("ok")
        }
        None => Response::from_string(format!("unknown move: {:?}", req.mv)).with_status_code(400),
    }
}

/// Parse a full `CubeState` and request an instant repaint. Sanity warnings are
/// surfaced in the 200 body but never cause a rejection (impossible states are
/// allowed per the README).
fn handle_state(tx: &Sender<Cmd>, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let state: CubeState = match serde_json::from_str(body) {
        Ok(s) => s,
        Err(e) => {
            return Response::from_string(format!("invalid /state JSON: {e}"))
                .with_status_code(400);
        }
    };
    let warnings = state.sanity_warnings();
    let _ = tx.send(Cmd::SetState(state));
    let body = serde_json::json!({ "warnings": warnings }).to_string();
    Response::from_string(body)
}
