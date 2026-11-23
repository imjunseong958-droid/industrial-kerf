//! kerf-bridge's integration tests, one test binary, each former test file a module. connection stays a binary of its own (it counts access violations with a process-wide exception handler on Windows, and CI runs it alone), as do making, which sets the process-wide models file, and sharing, which runs each test again in a child given its own HOME.

mod api;
mod delivery;
mod protocol;
mod wire;

use kerf_bridge::{Bridge, Command};
use serde_json::Value;
use std::time::Duration;

/// How long a test waits for its bridge's drawing to settle: far longer than bringing a GPU
/// up takes on any machine, so that a wait that runs out is a fault to report, not a slow
/// machine.
const SETTLES_WITHIN: Duration = Duration::from_secs(120);

/// Held with a bridge, in a field after it, by what opens one for a test: let go just after
/// the bridge, it waits for the frames the bridge's playback was still drawing and for the
/// GPU one of them had begun to bring up.
///
/// A program the compositor draws asks it for the GPU when it plays, whatever its size, and
/// when a frame of 640x360 or more is drawn, and the compositor then brings the GPU up on a
/// thread of its own. A test binary that exits under that thread, at work in the GPU's
/// libraries, now and then dies as it ends, after every test has passed.
struct Settles;

impl Drop for Settles {
    fn drop(&mut self) {
        let settled = kerf_bridge::drawing_settled(SETTLES_WITHIN);
        // A test that has failed says why itself; a second panic would end the process.
        if !std::thread::panicking() {
            assert!(
                settled,
                "a bridge's drawing had not settled {SETTLES_WITHIN:?} after the bridge was \
                 let go: a frame is still being drawn, or the GPU is still being brought up"
            );
        }
    }
}

/// Send `command` as the editor writes it, in JSON. One that does not read as a [`Command`]
/// is refused, as one the bridge cannot take is.
fn send(bridge: &mut Bridge, command: &Value) -> bool {
    serde_json::from_value::<Command>(command.clone()).is_ok_and(|command| bridge.send(command))
}

/// The next event, in JSON, as the editor reads it.
fn poll(bridge: &mut Bridge) -> Option<Value> {
    bridge
        .poll()
        .map(|event| serde_json::to_value(event).expect("an event writes as JSON"))
}
