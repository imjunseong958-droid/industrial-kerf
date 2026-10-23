//! The text form of Kerf projects.
//!
//! A project is a `.kerf` document: the canonical file that people, agents,
//! and version control all read. Every change is a script of edit
//! operations in the same vocabulary. Both directions are deterministic.

mod captions;
mod diff;
mod doc;
mod lex;
pub mod preset;
mod reference;
mod script;
mod text;

pub use captions::{caption_script, Word};
pub use diff::diff_text;
pub use doc::{parse_document, print_document, Document, PrintOptions, VERSION};
pub use reference::reference;
pub use script::{apply_script, print_op, print_ops, Applied};
pub use text::Clock;
