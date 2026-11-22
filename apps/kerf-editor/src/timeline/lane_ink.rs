//! What a lane draws over its clips: the waveform and room-tone greys
//! (`theme::waveform`), the rubber band and its keys and the bar a name
//! stands on (`theme::lane`), and the shade under a waveform's name.

use crate::theme::{lane, waveform};
use gpui::Rgba;

pub fn peak(room: bool) -> Rgba {
    if room {
        waveform::room_peak()
    } else {
        waveform::peak()
    }
}

pub fn rms(room: bool) -> Rgba {
    if room {
        waveform::room_rms()
    } else {
        waveform::rms()
    }
}

pub use lane::{keyframe as key, name_bar, name_shadow as shadow, rubber_band};
