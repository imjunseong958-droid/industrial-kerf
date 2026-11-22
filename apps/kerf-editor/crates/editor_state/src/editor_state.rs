//! The editor's data, which names no gpui: the project as the bridge's
//! events leave it ([`store::EditorState`]), the person's settings
//! ([`prefs::Prefs`]), kerf:// links, and what the views work out from them.

pub mod book;
pub mod changes;
pub mod chip;
pub mod delivery;
pub mod folders;
pub mod links;
pub mod model;
pub mod names;
pub mod prefs;
pub mod preset_file;
pub mod shortcuts;
pub mod store;
pub mod tokens;
pub mod track_heights;
