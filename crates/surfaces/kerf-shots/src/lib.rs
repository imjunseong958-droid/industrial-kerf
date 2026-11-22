//! A shot as a graph, for whoever shows or changes one: the editor's bridge,
//! for a person, and an agent's tools (`kerf-mcp`), for an agent.
//!
//! What a person sees in the editor's graph an agent reads with `look`, and
//! what a person sets there an agent sets with `edit`, so both go through
//! the one reading and the one rule here:
//!
//! - [`graph`]: the steps that make a media laid out with what they read
//!   ([`kerf_engine::recipe::graph()`]), each model saying which program's
//!   workflow it is run with, where it is run with one.
//! - [`workflow`]: the workflow behind a model, every node and input of it,
//!   and a value in it set in a copy of the shot's own that the step's line
//!   names with `workflow=`.
//! - [`signals`]: a signal's keys read from the onsets of a sound.
//! - [`takes`]: a shot's takes held to its guide by where its subjects are,
//!   to choose among them by.
//! - [`tracks`]: where a thing goes and the matte of a thing, read from
//!   footage: a clip that follows a point, and a mask that holds to an object.
//!
//! A change to a shot's steps is the same for both: `step` lines of an edit
//! of the document, which undo takes back. The two differ only in a
//! workflow's copy. A person's is made once and then changed in place; an
//! agent's is never written over, each a new copy the step then names.
//!
//! Nothing here runs a step or asks a server.

pub mod graph;
pub mod signals;
pub mod takes;
pub mod tracks;
pub mod workflow;
