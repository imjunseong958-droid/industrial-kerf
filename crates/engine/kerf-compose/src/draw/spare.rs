//! Pictures done with, kept for whatever next wants one of their size.
//!
//! A frame is drawn on a canvas the size of the frame, and a clip of one color is a picture that
//! size too: 133 MB each at 3840x2160. Memory new from the system is paid for a page at a time
//! as it is first written, and with the system's allocator a picture that large is given back
//! to the system when it is dropped, so a frame of one flat color spent most of its time being
//! handed memory: 185 ms, where a picture kept and written over makes it 35 ms. Under mimalloc,
//! which keeps such memory itself, the same frame went from 76 ms to 35 ms.
//!
//! What is kept is what was in it, and whoever takes one says what it is to hold: every pixel
//! of it is written before it is handed over, so a picture from here is the picture a new one
//! would be. Every drawing in the process shares them, up to [`MOST_BYTES`].

use super::*;
use std::sync::Mutex;

/// How many bytes of pictures are kept: three the size of a 3840x2160 frame.
const MOST_BYTES: usize = 512 << 20;

/// The least bytes of a picture worth keeping: smaller ones cost little to be handed.
const LEAST_BYTES: usize = 1 << 20;

static KEPT: Mutex<Vec<Image>> = Mutex::new(Vec::new());

fn kept() -> std::sync::MutexGuard<'static, Vec<Image>> {
    KEPT.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn bytes(picture: &Image) -> usize {
    std::mem::size_of_val(picture.px.as_slice())
}

/// One kept of `w` by `h`, holding whatever it held.
fn taken(w: usize, h: usize) -> Option<Image> {
    let mut kept = kept();
    let at = kept
        .iter()
        .position(|picture| (picture.w, picture.h) == (w, h))?;
    Some(kept.swap_remove(at))
}

/// A clear picture of `w` by `h`, as [`Image::new`] makes one.
pub(super) fn clear(w: usize, h: usize) -> Image {
    match taken(w, h) {
        Some(mut picture) => {
            picture.fill([0.0; 4]);
            picture
        }
        None => Image::new(w, h),
    }
}

/// A picture of `w` by `h` of one premultiplied `color`, as [`Image::filled`] makes one.
pub(super) fn filled(w: usize, h: usize, color: [f32; 4]) -> Image {
    match taken(w, h) {
        Some(mut picture) => {
            picture.fill(color);
            picture
        }
        None => Image::filled(w, h, color),
    }
}

/// `picture` is done with: kept for the next that wants one of its size, the oldest kept making
/// room for it.
pub(super) fn keep(picture: Image) {
    let size = bytes(&picture);
    if !(LEAST_BYTES..=MOST_BYTES).contains(&size) || picture.px.len() != picture.w * picture.h * 4
    {
        return;
    }
    let mut kept = kept();
    kept.push(picture);
    while kept.iter().map(bytes).sum::<usize>() > MOST_BYTES {
        kept.remove(0);
    }
}

/// [`keep`], of a picture nothing else holds: one that is also in a store of frames, or shared
/// with another pass, is theirs.
pub(super) fn keep_if_alone(picture: Arc<Image>) {
    if let Ok(picture) = Arc::try_unwrap(picture) {
        keep(picture);
    }
}
