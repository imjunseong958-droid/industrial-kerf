//! kerf-mcp's integration tests, one test binary, each former test file a module. carry, document_steps, harness, preset_steps, shot_graph and workflow_server stay binaries of their own: each sets process-wide settings (the models file, the variables model programs get) that tests running beside them would read.

mod cut_edges;
mod preset_looks;
mod qc;
mod qc_full;
