//! kerf-mcp's integration tests, one test binary, each former test file a module. carry, document_steps, harness, preset_steps and shot_graph stay binaries of their own: each sets a process-wide setting (the generation file) that tests running beside them would read.

mod cut_edges;
mod preset_looks;
mod qc;
mod qc_full;
mod stdio;
