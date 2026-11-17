//! A workflow: the file `kerf-comfy` runs for a model kept on a ComfyUI
//! server, a ComfyUI graph with blanks and what the model it runs makes
//! ([`workflow`]), the workflows that come with Kerf, and the names a
//! recipe's request hands things under ([`names`]).
//!
//! It is a crate of its own so that what reads a workflow without running
//! one reads it the same way: `kerf-comfy` (`tools/kerf-comfy`), which fills
//! its blanks and queues it; the editor's bridge, which shows it as the
//! graph behind a model and sets a value in it ([`detail`]); and an agent's
//! tools, which can say and set what a person can.
//!
//! Nothing here asks a server, and nothing of ComfyUI is linked: ComfyUI is
//! another program, asked over HTTP by `kerf-comfy` when a shot is made.
//! This reads and writes a file.

pub mod detail;
pub mod names;
pub mod workflow;
