//! What a clip's drawing reads of its place in a sequence: how far its
//! transitions reach, and the seed its effects draw their chance from.

use kerf_model::{Clip, Time, Track};

/// How far the clip's transitions reach before and after it.
pub fn handles(track: &Track, clip: &Clip) -> (Time, Time) {
    track
        .transitions
        .iter()
        .fold((Time::ZERO, Time::ZERO), |(pre, post), t| {
            (
                if t.to == clip.id {
                    pre.max(t.duration)
                } else {
                    pre
                },
                if t.from == clip.id {
                    post.max(t.duration)
                } else {
                    post
                },
            )
        })
}

/// The seed of a clip's effects: FNV-1a of its id, so random effects differ
/// between clips and are the same in a render, in playback, and in a
/// preview.
pub fn seed(id: &str) -> u32 {
    id.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_differ_between_clips_and_hold_for_one() {
        assert_eq!(seed("c1"), seed("c1"));
        assert_ne!(seed("c1"), seed("c2"));
    }
}
