//! Lightweight, dependency-minimal editing primitives.
//!
//! This crate intentionally has no ffmpeg, native runtime, or vision
//! dependencies. It is the stable boundary shared by the native engine and
//! higher-level clients for deterministic timeline compilation.

pub mod compile;
pub mod compile_v1;
pub mod overlay;
