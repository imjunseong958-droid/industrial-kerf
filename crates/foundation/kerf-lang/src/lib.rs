//! The text form of Kerf projects.
//!
//! A project is a `.kerf` document: the canonical file that people, agents,
//! and version control all read. Every change is a script of edit
//! operations in the same vocabulary. Both directions are deterministic.

mod captions;
mod diff;
mod doc;
mod facts;
mod lex;
mod making;
pub mod preset;
mod reference;
mod script;
mod sound;
mod text;

pub use captions::{
    caption_characters, caption_script, is_a_sound_note, to_srt, to_vtt, Word, CAPTION_GAP_FRAMES,
    READING_SPEED, SHORT_CAPTION_SIXTHS,
};
pub use diff::diff_text;
pub use doc::{parse_document, print_document, Document, PrintOptions, VERSION};
pub use facts::{
    airs, Barred, Breaks, Facts, NoFacts, Place, Snap, Snaps, Speech, BEAT, WORD_SLACK,
};
pub use making::print_step;
pub use reference::{fx_reference, property_reference, reference};
pub use script::{
    apply_script, apply_script_with, print_op, print_ops, sequence_position, Applied,
};
pub use sound::{EdgeSound, Heard, Sound};
pub use text::Clock;
