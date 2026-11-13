//! A camera a model found, read from the file it was written to.
//!
//! A model that reads a picture back says where the camera was: on each frame of a shot, or
//! for the one frame a set was rebuilt from. There is no format for it. What comes is the
//! model's answer as it is printed, tensors
//! as text: `'extrinsics': tensor(…)`, three or four rows of four numbers a frame, the way
//! from the world to the camera with x right, y down, and z ahead; and `'intrinsics':
//! tensor(…)`, three rows of three a frame, the lens in pixels of the picture the model ran
//! on. A file with a lens and no place is of a camera that did not move. A file that is one
//! number is a lens alone: how much it sees down its picture, in degrees.
//!
//! This reads such a file into where the camera is on each frame in the space of a mesh's
//! file ([`kerf_compose::FileView`]), the camera of one frame being at that file's origin:
//! a set rebuilt from that frame is then seen as the shot saw it.

use kerf_compose::FileView;

type Vector = [f64; 3];

/// A camera on each frame of a shot.
#[derive(Debug)]
pub(crate) struct Found {
    pub views: Vec<FileView>,
    /// How much each frame's lens sees down its picture, in degrees.
    pub fovs: Vec<f64>,
    /// The frame the file counts its places from: the one whose camera is at the file's own
    /// origin, not turned. A model that reads frames together as views of one scene picks
    /// one of them, not always the first, and what it rebuilds of any of them is in that
    /// one's space. None where no frame is there, or the file has no places.
    pub anchor: Option<usize>,
}

impl Found {
    /// The lens of the middle of them, by size: a camera clip has one, and what a model
    /// finds of a lens wavers frame by frame.
    pub(crate) fn lens(&self) -> f64 {
        let mut sorted = self.fovs.clone();
        sorted.sort_by(f64::total_cmp);
        sorted[sorted.len() / 2]
    }

    /// Where the camera is `frames` frames in, which may be between two: the last of them
    /// from there on.
    pub(crate) fn at(&self, frames: f64) -> FileView {
        let last = self.views.len() - 1;
        let from = (frames.max(0.0).floor() as usize).min(last);
        let (one, next) = (self.views[from], self.views[(from + 1).min(last)]);
        let share = (frames - from as f64).clamp(0.0, 1.0);
        let between = |one: Vector, next: Vector| {
            [0, 1, 2].map(|axis| one[axis] + (next[axis] - one[axis]) * share)
        };
        FileView {
            eye: between(one.eye, next.eye),
            right: unit(between(one.right, next.right)),
            forward: unit(between(one.forward, next.forward)),
        }
    }
}

/// The camera `text` holds, that of frame `origin` being at a mesh file's origin and one of
/// the text's units being `scale` of the mesh file's.
pub(crate) fn read(text: &str, origin: usize, scale: f64) -> Result<Found, String> {
    if let Ok(fov) = text.trim().parse::<f64>() {
        return lens_alone(fov);
    }
    let places = tensor(text, "extrinsics")?;
    let lenses = tensor(text, "intrinsics")?;
    if places.is_none() && lenses.is_none() {
        return Err("it holds no `extrinsics` and no `intrinsics`, and is not one number: it is no camera a model wrote, and no lens in degrees".to_owned());
    }
    let fovs = match &lenses {
        Some(lenses) => lenses
            .iter()
            .map(|lens| {
                // Half the picture's height over how long the lens is: its middle is the lens's.
                let (long, half) = match lens.as_slice() {
                    [_, down, _] if down.len() == 3 => (down[1], down[2]),
                    _ => (0.0, 0.0),
                };
                if long > 0.0 && half > 0.0 {
                    Ok(2.0 * (half / long).atan().to_degrees())
                } else {
                    Err(
                        "its `intrinsics` are not three rows of three a frame, with a lens in them"
                            .to_owned(),
                    )
                }
            })
            .collect::<Result<Vec<f64>, String>>()?,
        None => return Err("it holds no `intrinsics`, so no lens".to_owned()),
    };
    let Some(places) = places else {
        return Ok(Found {
            views: vec![FileView::ORIGIN; fovs.len()],
            fovs,
            anchor: None,
        });
    };
    if places.len() != fovs.len() {
        return Err(format!(
            "it has {} places and {} lenses",
            places.len(),
            fovs.len()
        ));
    }
    // Each as what turns the world to the camera, and what then moves it.
    let cameras = places
        .iter()
        .map(|place| match place.as_slice() {
            [across, down, ahead, ..] if [across, down, ahead].iter().all(|row| row.len() == 4) => {
                let row = |row: &Vec<f64>| [row[0], row[1], row[2]];
                Ok((
                    [row(across), row(down), row(ahead)],
                    [across[3], down[3], ahead[3]],
                ))
            }
            _ => Err("its `extrinsics` are not three rows of four a frame".to_owned()),
        })
        .collect::<Result<Vec<([Vector; 3], Vector)>, String>>()?;
    let anchor = cameras.iter().position(|(turn, moved)| {
        let still = moved.iter().all(|side| side.abs() < 1e-6);
        still && (0..3).all(|row| (0..3).all(|column| {
            (turn[row][column] - f64::from(u8::from(row == column))).abs() < 1e-6
        }))
    });
    let Some((turn, moved)) = cameras.get(origin) else {
        return Err(format!(
            "it has {} frames, from 0, and no frame {origin}",
            cameras.len()
        ));
    };
    let dot =
        |one: Vector, other: Vector| one[0] * other[0] + one[1] * other[1] + one[2] * other[2];
    // Into the space of frame `origin`'s camera, and from its y down and z ahead to a
    // file's y up and z toward the viewer.
    let way = |of: Vector| [dot(turn[0], of), -dot(turn[1], of), -dot(turn[2], of)];
    let place = |of: Vector| {
        let turned = way(of);
        [
            turned[0] + moved[0],
            turned[1] - moved[1],
            turned[2] - moved[2],
        ]
    };
    let views = cameras
        .iter()
        .map(|(rows, by)| {
            // Where the camera is in the world is what its turn and move bring to nothing.
            let eye = [0, 1, 2].map(|axis| {
                -(rows[0][axis] * by[0] + rows[1][axis] * by[1] + rows[2][axis] * by[2])
            });
            FileView {
                eye: place(eye).map(|side| side * scale + 0.0),
                right: unit(way(rows[0])),
                forward: unit(way(rows[2])),
            }
        })
        .collect();
    Ok(Found {
        views,
        fovs,
        anchor,
    })
}

fn lens_alone(fov: f64) -> Result<Found, String> {
    if !(fov > 0.0 && fov < 180.0) {
        return Err(format!(
            "{fov} is no lens: it is degrees down the picture, between 0 and 180"
        ));
    }
    Ok(Found {
        views: vec![FileView::ORIGIN],
        fovs: vec![fov],
        anchor: None,
    })
}

fn unit(of: Vector) -> Vector {
    let length = (of[0] * of[0] + of[1] * of[1] + of[2] * of[2])
        .sqrt()
        .max(1e-300);
    of.map(|side| side / length)
}

/// The tensor `text` prints under `name`, as its matrices, each its rows: none where the
/// text has no such name.
fn tensor(text: &str, name: &str) -> Result<Option<Vec<Vec<Vec<f64>>>>, String> {
    let Some(named) = [format!("'{name}'"), format!("\"{name}\"")]
        .iter()
        .find_map(|quoted| text.find(quoted.as_str()).map(|at| at + quoted.len()))
    else {
        return Ok(None);
    };
    let rest = &text[named..];
    let Some(open) = rest.find('[') else {
        return Err(format!("`{name}` is followed by no numbers"));
    };
    // Rows are the innermost brackets, and a matrix those that hold rows.
    let (mut matrices, mut rows, mut row, mut number) =
        (Vec::new(), Vec::new(), Vec::new(), String::new());
    let mut depth = 0usize;
    let ended = |number: &mut String, row: &mut Vec<f64>| -> Result<(), String> {
        if number.is_empty() {
            return Ok(());
        }
        if number.starts_with("...") {
            return Err(format!("`{name}` is cut short, as PyTorch prints more than a thousand numbers: it holds 83 frames at most"));
        }
        let read = number
            .parse()
            .map_err(|_| format!("`{name}` has `{number}` where a number is"))?;
        row.push(read);
        number.clear();
        Ok(())
    };
    for letter in rest[open..].chars() {
        match letter {
            '[' => depth += 1,
            ']' => {
                ended(&mut number, &mut row)?;
                if !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                } else if !rows.is_empty() {
                    matrices.push(std::mem::take(&mut rows));
                }
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            ',' => ended(&mut number, &mut row)?,
            letter if letter.is_whitespace() => ended(&mut number, &mut row)?,
            letter => number.push(letter),
        }
    }
    if depth != 0 {
        return Err(format!("`{name}` does not end"));
    }
    // One matrix alone has no bracket about its rows but the tensor's own.
    if !rows.is_empty() {
        matrices.push(rows);
    }
    Ok(Some(matrices))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(one: Vector, other: Vector) -> bool {
        (0..3).all(|axis| (one[axis] - other[axis]).abs() < 1e-6)
    }

    /// As a model's answer is printed: a camera that stands still, one a
    /// unit to its right and a unit nearer, and one turned a quarter round to its right.
    const PRINTED: &str = "{'depth': tensor([[[0.5000, 0.5100, 0.5200,  ..., 0.9000]]]),\n 'extrinsics': tensor([[[[ 1.0000e+00,  0.0000e+00,  0.0000e+00,  0.0000e+00],\n          [ 0.0000e+00,  1.0000e+00,  0.0000e+00,  0.0000e+00],\n          [ 0.0000e+00,  0.0000e+00,  1.0000e+00,  0.0000e+00]],\n\n         [[ 1.0000e+00,  0.0000e+00,  0.0000e+00, -1.0000e+00],\n          [ 0.0000e+00,  1.0000e+00,  0.0000e+00,  0.0000e+00],\n          [ 0.0000e+00,  0.0000e+00,  1.0000e+00, -1.0000e+00]],\n\n         [[ 0.0000e+00,  0.0000e+00, -1.0000e+00,  0.0000e+00],\n          [ 0.0000e+00,  1.0000e+00,  0.0000e+00,  0.0000e+00],\n          [ 1.0000e+00,  0.0000e+00,  0.0000e+00,  0.0000e+00]]]], device='cuda:0'),\n 'intrinsics': tensor([[[[400.,   0., 252.],\n          [  0., 400., 142.],\n          [  0.,   0.,   1.]],\n\n         [[400.,   0., 252.],\n          [  0., 390., 142.],\n          [  0.,   0.,   1.]],\n\n         [[400.,   0., 252.],\n          [  0., 410., 142.],\n          [  0.,   0.,   1.]]]])}";

    #[test]
    fn a_printed_path_is_read_into_a_files_space() {
        let found = read(PRINTED, 0, 1.0).expect("a path");
        assert_eq!(found.views.len(), 3);
        // The first is where the file counts from.
        assert_eq!(found.anchor, Some(0));
        assert!(
            near(found.views[0].eye, [0.0; 3]) && near(found.views[0].forward, [0.0, 0.0, -1.0])
        );
        // A unit to the right and a unit ahead, which in a file is its z backward.
        assert!(
            near(found.views[1].eye, [1.0, 0.0, -1.0]),
            "{:?}",
            found.views[1]
        );
        assert!(near(found.views[1].forward, [0.0, 0.0, -1.0]));
        // What is ahead of the third is the world's x: it looks to the first one's right.
        assert!(
            near(found.views[2].forward, [1.0, 0.0, 0.0]),
            "{:?}",
            found.views[2]
        );
        assert!(
            near(found.views[2].right, [0.0, 0.0, 1.0]),
            "{:?}",
            found.views[2]
        );
        // The lens of the middle one by size, of a picture 284 tall.
        let lens = 2.0 * (142.0f64 / 400.0).atan().to_degrees();
        assert!((found.lens() - lens).abs() < 1e-9);
        // Half way between two frames is half way between their places.
        assert!(near(found.at(0.5).eye, [0.5, 0.0, -0.5]));
        assert!(near(found.at(9.0).forward, [1.0, 0.0, 0.0]));
    }

    #[test]
    fn the_frame_a_set_is_of_is_at_the_origin_and_a_scale_is_kept() {
        // From the second frame's camera, the first is a unit to the left and a unit behind,
        // and at twice the size two.
        let found = read(PRINTED, 1, 2.0).expect("a path");
        assert!(near(found.views[1].eye, [0.0; 3]), "{:?}", found.views[1]);
        assert!(
            near(found.views[0].eye, [-2.0, 0.0, 2.0]),
            "{:?}",
            found.views[0]
        );
        // From the third's, which looks along x, the first is where it was and looks to
        // its left.
        let found = read(PRINTED, 2, 1.0).expect("a path");
        assert!(near(found.views[2].forward, [0.0, 0.0, -1.0]));
        assert!(
            near(found.views[0].forward, [-1.0, 0.0, 0.0]),
            "{:?}",
            found.views[0]
        );
        assert!(read(PRINTED, 3, 1.0).unwrap_err().contains("no frame 3"));
        // Whichever frame is put at the origin, the file counts from the one it counts from,
        // and a file whose every camera is somewhere counts from none.
        assert_eq!(found.anchor, Some(0));
        let moved = PRINTED.replacen("0.0000e+00],\n          [ 0.0000e+00,  1.0000e+00", "5.0000e-01],\n          [ 0.0000e+00,  1.0000e+00", 1);
        assert_ne!(moved, PRINTED);
        assert_eq!(read(&moved, 0, 1.0).expect("a path").anchor, None);
    }

    #[test]
    fn a_lens_alone_is_a_camera_that_stands_at_the_origin() {
        let found = read(" 41.5\n", 0, 1.0).expect("a lens");
        assert_eq!(found.views, [FileView::ORIGIN]);
        assert_eq!(found.lens(), 41.5);
        // A lens for each frame and no place is a camera that did not move.
        let found = read(
            "{'intrinsics': tensor([[[1.0, 0.0, 0.5], [0.0, 1.2, 0.5], [0.0, 0.0, 1.0]]])}",
            0,
            1.0,
        )
        .expect("a lens");
        assert_eq!(found.views, [FileView::ORIGIN]);
        assert!((found.lens() - 2.0 * (0.5f64 / 1.2).atan().to_degrees()).abs() < 1e-9);
        assert!(read("200", 0, 1.0).is_err());
        assert!(read("a camera", 0, 1.0)
            .unwrap_err()
            .contains("no camera a model wrote"));
    }

    #[test]
    fn a_path_cut_short_is_refused() {
        let cut = "{'extrinsics': tensor([[[[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],\n ...,\n [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]]]]), 'intrinsics': tensor([[[[1., 0., 1.], [0., 1., 1.], [0., 0., 1.]]]])}";
        assert!(read(cut, 0, 1.0).unwrap_err().contains("cut short"));
    }
}
