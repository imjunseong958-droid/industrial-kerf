//! kerf-compose's integration tests, one test binary: each former test file is a module, so the compositor and wgpu link once.

mod catalog_cost;
mod compose;
mod deep;
mod depth_gpu;
mod expressions;
mod fill;
mod footage;
mod frame_cache;
mod frame_key;
mod gpu;
mod grades;
mod hdr;
mod lands;
mod mesh;
mod motion;
mod multicam;
mod parent;
mod parent_depth;
mod passes;
mod retime;
mod soft_masks;
mod strokes;
mod three_d;
