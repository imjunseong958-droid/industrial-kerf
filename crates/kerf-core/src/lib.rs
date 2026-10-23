//! Lightweight, dependency-minimal editing primitives.
//!
//! This crate intentionally has no ffmpeg, native runtime, or vision
//! dependencies. It is the stable boundary shared by the native engine and
//! higher-level clients for deterministic program compilation.

pub mod overlay;
pub mod preview;
pub mod program;
pub mod raster;
