//! Motion graphics drawn from numbers: what a document can say in a line and a frame can draw alone,
//! with no state carried from the frame before, so any frame can be drawn first.

pub mod particles;
pub mod shape;

pub use particles::Emitter;
pub use shape::{Figure, Shape};
