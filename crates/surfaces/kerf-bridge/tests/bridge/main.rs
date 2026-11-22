//! kerf-bridge's integration tests, one test binary, each former test file a module. connection stays a binary of its own (it counts access violations with a process-wide exception handler on Windows, and CI runs it alone), as do making, which sets the process-wide models file, and sharing, which runs each test again in a child given its own HOME.

mod api;
mod delivery;
mod protocol;
mod wire;

use kerf_bridge::{Bridge, Command};
use serde_json::Value;

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
