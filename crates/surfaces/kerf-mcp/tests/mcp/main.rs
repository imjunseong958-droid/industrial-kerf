//! kerf-mcp's integration tests, one test binary, each former test file a module. carry, document_steps, harness, preset_steps, shot_graph and workflow_server stay binaries of their own: each sets environment variables (KERF_MODELS, KERF_COMFY and others) for the whole process, which tests running beside them would read.

mod cut_edges;
mod preset_looks;
mod qc;
mod qc_full;
