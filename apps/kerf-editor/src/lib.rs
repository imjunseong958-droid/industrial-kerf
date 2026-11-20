//! Industrial Kerf's editor, drawn with GPUI.
//!
//! The window keeps no editable copy of the project: kerfd owns it, the
//! bridge (`kerf-bridge`) carries commands to it and its events back, and
//! [`store::EditorState`] is what those events leave. Each region of the
//! window draws from that state and sends edits back as typed operations.

pub mod app;
pub mod artwork;
pub mod cast;
pub mod chat;
pub mod commands;
pub mod export;
pub mod finish;
pub mod icons;
pub mod input;
pub mod inspector;
pub mod launcher;
pub mod links;
pub mod making;
pub mod media;
pub mod model;
pub mod motion;
pub mod names;
pub mod palette;
pub mod prefs;
pub mod presets;
pub mod session;
pub mod settings;
pub mod sidebar;
pub mod stage;
pub mod store;
pub mod theme;
pub mod timeline;
pub mod ui;
pub mod workspace;
