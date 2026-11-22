//! A media file: ffprobe's facts of it, and frames, filmstrips, and waveforms read from it
//! ([`probe`]); where its picture cuts and its sound pauses and starts ([`index`]); a smaller
//! copy to edit with ([`proxy`]); and the project's cache these are kept in.
//!
//! [`derivatives`] is that cache: what can be made again from a file, kept under the file's
//! identity and the options it was made with, so that an interrupted run is never taken for a
//! result.

pub mod derivatives;
pub mod index;
pub mod probe;
pub mod proxy;
