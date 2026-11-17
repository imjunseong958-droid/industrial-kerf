//! A shot as a graph, for whoever shows or changes one: the editor's bridge,
//! for a person, and an agent's tools (`kerf-mcp`), for an agent.
//!
//! What a person sees in the editor's graph an agent reads with `look`, and
//! what a person sets there an agent sets with `edit`, so both go through
//! the one reading and the one rule here:
//!
//! - [`graph`]: a media's recipe laid out as its steps and what they read
//!   ([`kerf_engine::recipe::graph`]), each model saying which program's
//!   workflow it is run with, where it is run with one.
//! - [`workflow`]: the workflow behind a model, every node and input of it,
//!   and a value in it set in a copy of the shot's own that the step's line
//!   names with `workflow=`.
//! - [`takes`]: a shot's takes held to its guide by where its subjects are,
//!   to choose among them by.
//!
//! The two differ only in where a change is written. A person's is written
//! in place, as a text editor writes it. An agent's is never written over
//! anything: its recipe is the next version beside the one before
//! ([`kerf_engine::recipe::version`]) and its workflow a new copy, which a
//! proposal moves the media to.
//!
//! Nothing here runs a recipe or asks a server.

pub mod graph;
pub mod takes;
pub mod workflow;
