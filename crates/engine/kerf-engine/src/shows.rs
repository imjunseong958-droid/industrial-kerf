//! What shows of each clip on a frame: where it is, how much of the frame
//! it has, and what is in front of it.
//!
//! A scene written as text, clips in depth under a camera, can be wrong in
//! ways its text does not show: a floor drawn over what stands on it, a
//! subject gone past the frame's edge, a wall with its back to the camera.
//! This reads a frame as an eye would, without one. It draws the frame's ids
//! pass ([`kerf_compose::Pass::Ids`]), which says which clip each pixel is
//! of, and then each clip by itself, the others made clear, which says where
//! that clip would be with nothing before it. What the two differ by is what
//! hides it.
//!
//! It is of the clips of one sequence, a nested sequence being one clip, on a
//! small frame, so it reads to about a hundredth of the frame.

use crate::error::Result;
use kerf_compose::{ComposeOptions, Compositor, Frame, Pass};
use kerf_model::{Catalog, Param, Project, Time, Value};

/// The height of the frames read, in pixels, where the sequence is as tall.
const HEIGHT: f64 = 216.0;

/// How far a byte of a pixel may be from a clip's color and still be that clip's.
const NEAR: u8 = 6;

/// The most clips read one by one on a frame.
pub const MOST_CLIPS: usize = 32;

/// What shows of one clip on a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub clip: String,
    /// Where it would be with nothing before it: left, top, right, bottom,
    /// in shares of the frame. `None` when none of it is in the frame.
    pub at: Option<[f64; 4]>,
    /// The share of the frame it would have with nothing before it.
    pub area: f64,
    /// The share of the frame it has, as the frame is drawn.
    pub shows: f64,
    /// The share of it that other clips hide.
    pub hidden: f64,
    /// Which clips hide it, and the share of it each hides, the most first.
    pub by: Vec<(String, f64)>,
    /// The edges of the frame it reaches: `left`, `top`, `right`, `bottom`.
    /// A clip that reaches one may go on past it.
    pub edges: Vec<&'static str>,
    /// How opaque it is, where that is not wholly: a clip that lets what is
    /// behind it through is read here as not showing.
    pub opacity: Option<f64>,
}

/// What shows on a frame of a sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct Showing {
    /// The clips that draw at that moment, from the lowest track up.
    pub clips: Vec<Shown>,
    /// The share of the frame no clip has.
    pub empty: f64,
    /// Clips left out, where there are more than [`MOST_CLIPS`].
    pub unread: Vec<String>,
}

fn compositor(project: &Project, sequence: &str) -> Result<Compositor> {
    Ok(Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some(sequence.to_owned()),
            pass: Pass::Ids,
            ..ComposeOptions::default()
        },
    )?)
}

fn rounded(share: f64) -> f64 {
    (share * 1000.0).round() / 1000.0
}

/// What shows of each clip of `sequence` (the first when `None`) of `project`
/// at sequence time `at`. Media is read from where the project's paths say,
/// so resolve them first.
pub fn showing(project: &Project, sequence: Option<&str>, at: Time) -> Result<Showing> {
    let target = crate::program::program_sequence(project, sequence)?;
    let id = target.id.to_string();
    let whole = compositor(project, &id)?;
    let colors = whole.ids();
    let scale = (HEIGHT / f64::from(target.height.max(1))).min(1.0);
    let frame = whole.frame(at, scale);
    let pixels = (frame.width * frame.height).max(1) as usize;
    // Which clip each pixel is of, by its place among `colors`.
    let of: Vec<Option<usize>> = frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            colors
                .iter()
                .position(|(_, color)| color.iter().zip(pixel).all(|(a, b)| a.abs_diff(*b) <= NEAR))
        })
        .collect();
    let empty = frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[..3].iter().all(|channel| *channel <= NEAR))
        .count();

    // The clips of the sequence that draw now, from the lowest track up.
    let drawing: Vec<&kerf_model::Clip> = target
        .tracks
        .iter()
        .filter(|track| track.enabled)
        .flat_map(|track| &track.clips)
        .filter(|clip| clip.enabled && clip.start <= at && at < clip.end())
        .filter(|clip| colors.iter().any(|(told, _)| *told == clip.id.as_str()))
        .collect();
    let (read, unread) = drawing.split_at(drawing.len().min(MOST_CLIPS));

    let mut clips = Vec::with_capacity(read.len());
    for clip in read {
        let place = colors
            .iter()
            .position(|(told, _)| *told == clip.id.as_str())
            .expect("it has a color");
        // The frame with the others clear: they still move what is parented to them.
        let mut alone = project.clone();
        let own = alone
            .sequences
            .iter_mut()
            .find(|sequence| sequence.id == id.as_str())
            .expect("the sequence is there");
        for other in own
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
            .filter(|other| other.id != clip.id)
        {
            other
                .props
                .insert("opacity".to_owned(), Param::Static(Value::Number(0.0)));
        }
        let there: Frame = compositor(&alone, &id)?.frame(at, scale);
        let (w, h) = (there.width as usize, there.height as usize);
        let (mut area, mut seen) = (0usize, 0usize);
        let (mut left, mut top, mut right, mut bottom) = (w, h, 0usize, 0usize);
        let mut hidden_by = vec![0usize; colors.len()];
        for (index, pixel) in there.rgba.as_chunks::<4>().0.iter().enumerate() {
            if pixel[..3].iter().all(|channel| *channel <= NEAR) {
                continue;
            }
            area += 1;
            let (x, y) = (index % w, index / w);
            left = left.min(x);
            right = right.max(x + 1);
            top = top.min(y);
            bottom = bottom.max(y + 1);
            match of.get(index).copied().flatten() {
                Some(shown) if shown == place => seen += 1,
                Some(other) => hidden_by[other] += 1,
                None => {}
            }
        }
        let share = |count: usize| rounded(count as f64 / pixels as f64);
        let of_it = |count: usize| rounded(count as f64 / area.max(1) as f64);
        let mut by: Vec<(String, f64)> = hidden_by
            .iter()
            .enumerate()
            .filter(|(_, count)| **count * 50 >= area.max(1))
            .map(|(other, count)| (colors[other].0.clone(), of_it(*count)))
            .collect();
        by.sort_by(|a, b| b.1.total_cmp(&a.1));
        let opacity = clip
            .props
            .get("opacity")
            .and_then(|opacity| opacity.value_at(at - clip.start).as_number())
            .filter(|opacity| *opacity < 0.999);
        let mut edges = Vec::new();
        if area > 0 {
            for (reached, name) in [
                (left == 0, "left"),
                (top == 0, "top"),
                (right == w, "right"),
                (bottom == h, "bottom"),
            ] {
                if reached {
                    edges.push(name);
                }
            }
        }
        clips.push(Shown {
            clip: clip.id.to_string(),
            at: (area > 0).then(|| {
                [
                    rounded(left as f64 / w as f64),
                    rounded(top as f64 / h as f64),
                    rounded(right as f64 / w as f64),
                    rounded(bottom as f64 / h as f64),
                ]
            }),
            area: share(area),
            shows: share(seen),
            hidden: if area > 0 {
                rounded(1.0 - seen as f64 / area as f64)
            } else {
                0.0
            },
            by,
            edges,
            opacity,
        });
    }
    Ok(Showing {
        clips,
        empty: rounded(empty as f64 / pixels as f64),
        unread: unread.iter().map(|clip| clip.id.to_string()).collect(),
    })
}
