//! The editor's design tokens ([`theme`], [`icons`], [`motion`]) and the kit
//! every region is built from ([`ui`], [`input`], [`selectable`], [`chrome`]).
//! The kit stays generic: the app's own contexts and actions come to it as
//! parameters.

pub mod chrome;
pub mod icons;
pub mod input;
pub mod motion;
pub mod selectable;
pub mod theme;
pub mod ui;
