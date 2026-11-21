//! kerf-bridge's integration tests, one test binary, each former test file a module. connection stays a binary of its own (it counts access violations with a process-wide exception handler on Windows, and CI runs it alone), as do making and sharing, which set environment variables for the whole process.

mod api;
mod delivery;
mod protocol;
