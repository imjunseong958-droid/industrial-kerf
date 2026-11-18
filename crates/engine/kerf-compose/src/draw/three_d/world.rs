//! Clips in depth as they are in the world at a moment, for one who measures a scene and
//! draws none of it.
//!
//! What stands on what is not in a frame: a foot a hundredth of the frame over a floor is a
//! pixel or two, and behind the floor it is no pixel at all. It is in the numbers a frame is
//! drawn from, which are the ones here: a clip's plane ([`Plane::of`]), the plane behind it
//! where it has a `depth`, and a mesh's triangles where its file's animation has them then
//! ([`mesh::in_the_world`]). Nothing is drawn to say them, and no picture is read, so they
//! are the same whatever size the sequence is.
//!
//! The world here is the one of this module's top, by the frame's height and not its pixels:
//! x to the right, y down, z away from the viewer, from the middle of the frame at `z=0`.

use super::*;

/// What a clip in depth is, in the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// A flat picture: one rectangle.
    Flat,
    /// A picture with a `depth`: the box between its front and its back.
    Solid,
    /// A mesh: its triangles.
    Mesh,
}

/// A clip in depth as it is in the world at a moment.
pub struct InDepth {
    pub clip: String,
    pub form: Form,
    /// Its corners: a flat clip's four, going round; a solid's eight, its front's four and
    /// then its back's, each behind the one it is of; every corner of a mesh's triangles.
    pub corners: Vec<[f64; 3]>,
    /// Whether it fills what its corners go round. A shape that is no plain rectangle, a
    /// solid with a bevel, and a mesh do not: they are within it.
    pub filled: bool,
    /// How far down the world one more of its own `y` puts it. Other than 1 where a parent
    /// has scaled it, and none where a parent has turned it, so that its own `y` moves it
    /// across as well.
    pub fall: Option<f64>,
    /// Its sides, a flat clip's four and a solid's twelve, each by its two corners. A mesh's
    /// are its triangles'.
    sides: Vec<[u32; 2]>,
    triangles: Vec<[u32; 3]>,
    surface: mesh::Standing,
}

impl InDepth {
    /// Where a ray from `from` along `way`, a unit long, first meets its surface, no further
    /// than `reach`: how far along, and the way the surface faces there, a unit long, toward
    /// either side of it. A surface the ray runs along is not met.
    pub fn meets(&self, from: [f64; 3], way: [f64; 3], reach: f64) -> Option<(f64, [f64; 3])> {
        self.surface.first(from, way, reach)
    }

    /// Its edges, each by its two ends: where it goes through another clip, one of these
    /// goes through that clip's surface.
    pub fn edges(&self) -> Vec<[[f64; 3]; 2]> {
        let ends = |[from, to]: [u32; 2]| [self.corners[from as usize], self.corners[to as usize]];
        if self.form != Form::Mesh {
            return self.sides.iter().copied().map(ends).collect();
        }
        // Two triangles share an edge, which is one edge.
        let mut sides: Vec<[u32; 2]> = self
            .triangles
            .iter()
            .flat_map(|[a, b, c]| [[*a, *b], [*b, *c], [*c, *a]])
            .map(|[from, to]| [from.min(to), from.max(to)])
            .collect();
        sides.sort_unstable();
        sides.dedup();
        sides.into_iter().map(ends).collect()
    }
}

/// What a sequence has in depth at a moment ([`crate::Compositor::in_depth`]).
#[derive(Default)]
pub struct World {
    /// Its clips in depth that draw, from the lowest track up. A camera, a light, a null, a
    /// signal, and an adjustment layer are none of them.
    pub clips: Vec<InDepth>,
    /// Clips in depth whose outline is their picture's own and not their numbers': a title, a
    /// nested sequence, media with alpha, particles, and a clip its masks cut. They are
    /// somewhere within their frame, and where is not read.
    pub unread: Vec<String>,
    /// The clips that draw out of depth, which are nowhere in the world.
    pub flat: Vec<String>,
}

/// The clip of `plan` at clip time `time` in the world, where `to_output` puts it: a mesh,
/// or a picture that `held` says where in its own pixels it is, and whether it fills that.
pub(in crate::draw) fn of(
    scene: &Scene,
    plan: &ClipPlan,
    to_output: &Affine,
    time: Time,
    held: Option<(Rect, bool)>,
    fall: Option<f64>,
) -> InDepth {
    let clip = plan.clip.id.clone();
    // Frame heights, where the scene is in pixels.
    let by = 1.0 / scene.height.max(1.0);
    let Some((rect, filled)) = held else {
        let (points, triangles) = mesh::in_the_world(scene, plan, to_output, time);
        let corners: Vec<Vector> = points.iter().map(|point| times(*point, by)).collect();
        return InDepth {
            surface: surface(&clip, &corners, &triangles),
            clip: clip.to_string(),
            form: Form::Mesh,
            corners,
            filled: false,
            fall,
            sides: Vec::new(),
            triangles,
        };
    };
    let size = (f64::from(plan.size.0), f64::from(plan.size.1));
    let front = Plane::of(scene, plan, size, to_output, time);
    let deep = depth_of(plan, time, scene.height);
    let mut corners: Vec<Vector> = front.corners(rect).to_vec();
    let (mut sides, mut triangles) = (
        vec![[0, 1], [1, 2], [2, 3], [3, 0]],
        vec![[0, 1, 2], [0, 2, 3]],
    );
    if deep > 0.0 {
        corners.extend(front.pushed(deep).corners(rect));
        sides.extend([[4, 5], [5, 6], [6, 7], [7, 4]]);
        triangles.extend([[4, 5, 6], [4, 6, 7]]);
        // And from each corner of the front to the one behind it, with the side between.
        for corner in 0..4u32 {
            let next = (corner + 1) % 4;
            sides.push([corner, corner + 4]);
            triangles.extend([[corner, next, next + 4], [corner, next + 4, corner + 4]]);
        }
    }
    let corners: Vec<Vector> = corners.iter().map(|corner| times(*corner, by)).collect();
    InDepth {
        surface: surface(&clip, &corners, &triangles),
        clip: clip.to_string(),
        form: if deep > 0.0 { Form::Solid } else { Form::Flat },
        corners,
        // A bevel takes a solid's edges in.
        filled: filled && bevel_of(plan, time, scene.height) <= 0.0,
        fall,
        sides,
        triangles,
    }
}

fn surface(clip: &Id, corners: &[Vector], triangles: &[[u32; 3]]) -> mesh::Standing {
    let corners = triangles
        .iter()
        .map(|triangle| triangle.map(|corner| corners[corner as usize]))
        .collect();
    mesh::Standing::over(clip.clone(), 1.0, corners)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_meets_the_nearest_of_a_surface_and_says_how_it_faces() {
        // Two floors, one a unit under the other, each two triangles a unit across.
        let floor = |y: f64| {
            let corner = |x: f64, z: f64| [x, y, z];
            [
                [corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0)],
                [corner(0.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0)],
            ]
        };
        let both: Vec<[Vector; 3]> = floor(2.0).into_iter().chain(floor(1.0)).collect();
        let standing = mesh::Standing::over(Id::new("c").unwrap(), 1.0, both);
        let down = [0.0, 1.0, 0.0];
        // From above both, the upper one; from between them, the lower; from on one, that one.
        let (along, facing) = standing.first([0.5, 0.0, 0.5], down, 10.0).expect("it is under");
        assert!((along - 1.0).abs() < 1e-12 && facing[1].abs() == 1.0, "{along} {facing:?}");
        let (along, _) = standing.first([0.5, 1.5, 0.5], down, 10.0).expect("it is under");
        assert!((along - 0.5).abs() < 1e-12, "{along}");
        let (along, _) = standing.first([0.5, 1.0, 0.5], down, 10.0).expect("it is on it");
        assert_eq!(along, 0.0);
        // Not past its reach, not beside it, not behind where it starts, and not along it.
        assert!(standing.first([0.5, 0.0, 0.5], down, 0.9).is_none());
        assert!(standing.first([1.5, 0.0, 0.5], down, 10.0).is_none());
        assert!(standing.first([0.5, 2.5, 0.5], down, 10.0).is_none());
        assert!(standing.first([-1.0, 1.0, 0.5], [1.0, 0.0, 0.0], 10.0).is_none());
        // A slope faces where it leans.
        let slope = vec![[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 1.0]]];
        let standing = mesh::Standing::over(Id::new("c").unwrap(), 1.0, slope);
        let (along, facing) = standing.first([0.2, -1.0, 0.3], down, 10.0).expect("it is under");
        assert!((along - 1.3).abs() < 1e-12, "{along}");
        assert!((facing[1].abs() - 0.5f64.sqrt()).abs() < 1e-12, "{facing:?}");
        // Nothing has no surface.
        let none = mesh::Standing::over(Id::new("c").unwrap(), 1.0, Vec::new());
        assert!(none.first([0.0; 3], down, 10.0).is_none());
    }
}
