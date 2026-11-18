//! The names a request hands things under, which a workflow's blanks take:
//! `{guide}`, `{ref2}`, `{signal}`, `{moves}`, `{inside}`.

/// The name a workflow reads where clips go under.
pub const MOVES: &str = "moves";

/// The blank that takes where clips go as a picture in motion, in place of points: a video
/// of the frames asked for, white, with each clip's box drawn on it in black where it is on
/// that frame, as a layout a model follows.
pub const MOVES_VIDEO: &str = "moves_video";

/// The blank that takes where clips go as five tracks of points a clip, its box's middle
/// and four points toward its corners.
pub const MOVES_BOX: &str = "moves_box";

/// The blanks that take points on a thing of the guide's first frame, and points off it, as a
/// `generate` line writes them (`inside="0.3,0.6 0.3,0.7"`, each 0 to 1 across and down the
/// frame): filled as text a node reads coordinates from, a list of `{"x": …, "y": …}` in
/// pixels of the size asked for.
pub const INSIDE: &str = "inside";
pub const OUTSIDE: &str = "outside";

/// Whether `name` is one a line gives points under.
pub fn names_points(name: &str) -> bool {
    name == INSIDE || name == OUTSIDE
}

/// What a guide beside the first is of, where `name` is one's: `depth` of `guide_depth`.
pub fn guide_named(name: &str) -> Option<&str> {
    name.strip_prefix("guide_").filter(|of| {
        !of.is_empty()
            && of
                .chars()
                .all(|letter| letter.is_ascii_lowercase() || letter.is_ascii_digit())
    })
}

/// Which reference `name` is, counting from 0: `ref` is the first, `ref2`
/// the second.
pub fn reference(name: &str) -> Option<usize> {
    match name.strip_prefix("ref")? {
        "" => Some(0),
        number => number
            .parse::<usize>()
            .ok()
            .filter(|number| *number >= 2)
            .map(|number| number - 1),
    }
}

/// Whether `name` is one a request gives a file under.
pub fn names_a_file(name: &str) -> bool {
    matches!(name, "guide" | "first" | "last")
        || reference(name).is_some()
        || guide_named(name).is_some()
}

/// The name a request's signal has, counting from 0: `signal` is the
/// first, `signal2` the second, as its references are named.
pub fn signal_named(index: usize) -> String {
    match index {
        0 => "signal".to_owned(),
        index => format!("signal{}", index + 1),
    }
}

/// The blank that takes a signal as a picture in motion, in place of numbers: a grey video
/// of the frames asked for, as light on each frame as the signal is then, for a node that
/// reads a mask a frame. `signal_video` for the first, `signal2_video` for the second.
pub fn signal_video_named(index: usize) -> String {
    format!("{}_video", signal_named(index))
}

/// Whether `name` is one a request gives a signal under, as numbers or as a video.
pub fn names_a_signal(name: &str) -> bool {
    let name = name.strip_suffix("_video").unwrap_or(name);
    match name.strip_prefix("signal") {
        Some("") => true,
        Some(number) => number.parse::<usize>().is_ok_and(|number| number >= 2),
        None => false,
    }
}

/// Whether `name` is one a request hands something under, a file, a
/// signal, where clips go, or points: a blank of that name is filled only by a
/// request that has it.
pub fn names_what_is_handed(name: &str) -> bool {
    names_a_file(name)
        || names_a_signal(name)
        || name == MOVES
        || name == MOVES_VIDEO
        || name == MOVES_BOX
        || names_points(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_says_what_a_request_hands_under_it() {
        for name in ["guide", "first", "last", "ref", "ref2", "ref12", "guide_depth"] {
            assert!(names_a_file(name) && names_what_is_handed(name), "{name}");
        }
        for name in ["ref1", "ref0", "refs", "guide_", "guide_Depth", "guides", "prompt"] {
            assert!(!names_a_file(name), "{name}");
        }
        assert_eq!((reference("ref"), reference("ref3"), reference("ref1")), (Some(0), Some(2), None));
        assert_eq!(guide_named("guide_ids"), Some("ids"));
        assert_eq!([signal_named(0), signal_named(1)], ["signal", "signal2"]);
        assert_eq!(signal_video_named(1), "signal2_video");
        for name in ["signal", "signal2", "signal_video", "signal3_video"] {
            assert!(names_a_signal(name) && names_what_is_handed(name), "{name}");
        }
        for name in ["signal1", "signal1_video", "signals", "video"] {
            assert!(!names_a_signal(name), "{name}");
        }
        for name in [MOVES, MOVES_VIDEO, MOVES_BOX, INSIDE, OUTSIDE] {
            assert!(names_what_is_handed(name) && !names_a_file(name), "{name}");
        }
        assert!(names_points("inside") && names_points("outside") && !names_points("insides"));
    }
}
