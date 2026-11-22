//! Industrial Kerf's editor, drawn with GPUI.
//!
//! The window keeps no editable copy of the project: kerfd owns it, the
//! bridge (`kerf-bridge`) carries commands to it and its events back, and
//! [`store::EditorState`] is what those events leave. Each region of the
//! window draws from that state and sends edits back as typed operations.
//!
//! The regions are the crates under `crates/`; this one puts the window and
//! the application together from them, and gives each of their modules out
//! here too, as `kerf_editor::theme` and the rest, for the program and its
//! tests.

pub mod app;
pub mod focus;
pub mod keymap;
pub mod workspace;

pub use editor_chat::{ask, chat};
pub use editor_finish::{export, finish};
pub use editor_inspector::inspector;
pub use editor_launcher::{cast, launcher, presets, settings};
pub use editor_making::{making, media};
pub use editor_session::{artwork, commands, palette, session};
pub use editor_sidebar::sidebar;
pub use editor_stage::stage;
pub use editor_state::{
    changes, chip, delivery, folders, links, model, names, prefs, preset_file, shortcuts, store,
};
pub use editor_timeline::timeline;
pub use editor_ui::{chrome, icons, input, motion, selectable, theme, ui};
