//! A camera in a mesh's file as the keyframes of a `gen=camera@1` clip.
//!
//! A set rebuilt from a shot comes as a glTF file with the camera that filmed it, and a
//! scene from a 3D program with the camera it was framed by. The compositor says where
//! such a camera is on each frame, the mesh being where its clip puts it
//! (`Compositor::mesh_camera`); this writes those places as `set` takes them, with no
//! more keys than say them.

use kerf_compose::CameraPlace;

/// How near a key's line must come to what it stands for: a ten-thousandth of the frame
/// for where the camera is, and a hundredth of a degree for how it is turned.
const PLACE: f64 = 1e-4;
const TURN: f64 = 0.01;

/// The words of a `set` line for a camera that is at `places` on the frames from
/// `first`, one a frame: each of `x`, `y`, `z`, `pan`, `tilt`, and `roll` as one number
/// where it holds still and as keyframes where it does not, and `fov`. With whether
/// any of them moves. A key's line comes within `coarse` times [`PLACE`] and [`TURN`] of what
/// it stands for: 1 for a camera a file holds.
pub(crate) fn params(
    places: &[CameraPlace],
    first: i64,
    fov: f64,
    coarse: f64,
) -> (String, bool) {
    let mut words = Vec::new();
    let mut moves = false;
    let mut word = |name: &str, values: Vec<f64>, tolerance: f64, digits: i32| {
        let by = 10f64.powi(digits);
        // Not `-0`, which is 0.
        let rounded = |value: f64| ((value * by).round() / by) + 0.0;
        let kept = kept(&values, tolerance * coarse);
        if kept.len() <= 1 {
            words.push(format!("{name}={}", rounded(values[0])));
        } else {
            moves = true;
            let keys: Vec<String> = kept
                .iter()
                .map(|index| format!("{}f:{}", first + *index as i64, rounded(values[*index])))
                .collect();
            words.push(format!("{name}={{{}}}", keys.join(" ")));
        }
    };
    let of = |read: fn(&CameraPlace) -> f64| places.iter().map(read).collect::<Vec<f64>>();
    word("x", of(|place| place.x), PLACE, 5);
    word("y", of(|place| place.y), PLACE, 5);
    word("z", of(|place| place.z), PLACE, 5);
    word("pan", unwound(of(|place| place.pan)), TURN, 4);
    word("tilt", of(|place| place.tilt), TURN, 4);
    word("roll", unwound(of(|place| place.roll)), TURN, 4);
    words.push(format!("fov={}", (fov * 1000.0).round() / 1000.0));
    (words.join(" "), moves)
}

/// Angles that go on past a half turn, and do not start again from the other side.
fn unwound(mut angles: Vec<f64>) -> Vec<f64> {
    for index in 1..angles.len() {
        let turns = ((angles[index] - angles[index - 1]) / 360.0).round();
        angles[index] -= turns * 360.0;
    }
    angles
}

/// Which of `values`, one a frame, to keep as keys so that straight lines between them
/// come within `tolerance` of every value: one where they are all the same.
fn kept(values: &[f64], tolerance: f64) -> Vec<usize> {
    let Some(last) = values.len().checked_sub(1) else {
        return Vec::new();
    };
    if values
        .iter()
        .all(|value| (value - values[0]).abs() <= tolerance)
    {
        return vec![0];
    }
    let mut keep = vec![false; values.len()];
    (keep[0], keep[last]) = (true, true);
    // Between two kept, the one furthest from their line is kept, until none is far.
    let mut spans = vec![(0, last)];
    while let Some((from, to)) = spans.pop() {
        let line = |index: usize| {
            let share = (index - from) as f64 / (to - from) as f64;
            values[from] + (values[to] - values[from]) * share
        };
        let furthest = (from + 1..to)
            .map(|index| (index, (values[index] - line(index)).abs()))
            .fold(
                (from, 0.0),
                |most, next| if next.1 > most.1 { next } else { most },
            );
        if furthest.1 > tolerance {
            keep[furthest.0] = true;
            spans.push((from, furthest.0));
            spans.push((furthest.0, to));
        }
    }
    (0..values.len()).filter(|index| keep[*index]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(x: f64, pan: f64) -> CameraPlace {
        CameraPlace {
            x,
            y: 0.25,
            z: -1.0,
            pan,
            tilt: 0.0,
            roll: 0.0,
            fov: Some(40.0),
        }
    }

    #[test]
    fn a_camera_that_holds_still_is_numbers_and_one_that_moves_is_keys() {
        let (still, moves) = params(&[place(0.5, 10.0); 4], 0, 28.6479, 1.0);
        assert_eq!(still, "x=0.5 y=0.25 z=-1 pan=10 tilt=0 roll=0 fov=28.648");
        assert!(!moves);
        // Evenly from one place to another is two keys, however many frames it takes; a
        // corner on the way is a third. Keys count from `first`.
        let places: Vec<CameraPlace> = (0..=10)
            .map(|frame| {
                let frame = f64::from(frame);
                place(frame / 10.0, if frame < 4.0 { frame } else { 4.0 })
            })
            .collect();
        let (moving, moves) = params(&places, 12, 40.0, 1.0);
        assert_eq!(
            moving,
            "x={12f:0 22f:1} y=0.25 z=-1 pan={12f:0 16f:4 22f:4} tilt=0 roll=0 fov=40"
        );
        assert!(moves);
    }

    #[test]
    fn a_turn_past_half_way_round_goes_on() {
        // 170, 178, -174, -166: it went on turning, and did not go back the long way.
        let places: Vec<CameraPlace> = [170.0, 178.0, -174.0, -166.0]
            .iter()
            .map(|pan| place(0.0, *pan))
            .collect();
        let (words, _) = params(&places, 0, 40.0, 1.0);
        assert!(words.contains("pan={0f:170 3f:194}"), "{words}");
    }
}
