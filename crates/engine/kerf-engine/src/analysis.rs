//! Readings of a project's picture and sound: what a sequence shows as it draws, how a take
//! holds beside its guide, what a recording says and how it lines up with another, and whether a
//! program is fit to deliver ([`qc`], [`loudness`]).
//!
//! The modules whose names do not say it:
//!
//! - [`shows`]: what shows of each clip on a frame: where it is, how much of the frame it has,
//!   and what is in front of it.
//! - [`stands`]: what each clip in depth stands on, measured in the scene with nothing drawn.
//! - [`eye`]: where the eye is either side of a cut, the thing it was on as one shot ends and
//!   the thing it is given as the next starts.
//! - [`follows`]: how well a take's depth followed its guide's, clip by clip.
//! - [`placed`]: where a take has a clip's subject, beside where its guide has it.
//! - [`unheard`]: where a recording sounds like speech and its [`transcription`] has no words.
//! - [`sync`]: where one recording's sound falls against another's, to line cameras up by it.

pub mod eye;
pub mod follows;
pub mod loudness;
pub mod placed;
pub mod qc;
pub mod shows;
pub mod stands;
pub mod sync;
pub mod transcription;
pub mod unheard;
