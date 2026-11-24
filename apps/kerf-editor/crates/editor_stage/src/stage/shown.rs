//! What the picture shows over the chosen clip: its masks and motion path
//! as they land on the picture (`Shown`), what a press on them hits, what a
//! press follows until its button comes up (`Drag`) and what a mask tool
//! has drawn so far (`Drawn`); and whether the picture shows the program at
//! all, so that they can be drawn on ([`shows_program`]).

use super::overlay::{self, Affine, MotionPath, Outline, Projective, Pt};
use super::picture::{apart, to_pt};
use super::tools::MaskTool;
use super::StageView;
use editor_session::session::Session;
use gpui::{point, Bounds, Pixels};

/// How near the pointer must be to a handle, a point or an outline to take it.
pub(super) const REACH: f64 = 6.0;

/// Whether the picture shows the program, so the clip chosen on it can be
/// drawn over and drawn on (its masks, its path, the mask tools): not while
/// a first cut proposed on an empty program stands in for it, nor while a
/// proposal's preview renders or shows. A proposal waiting over a program
/// with clips leaves the picture the program's, as kerfd plays the
/// document, and its masks and path can be drawn on while the proposal
/// waits.
pub fn shows_program(state: &editor_state::store::EditorState, program_has_clips: bool) -> bool {
    let cut_stands_in = state.snapshot.proposal_timeline.is_some() && !program_has_clips;
    let preview_shows = state.preview.proposal.is_some()
        && matches!(state.preview.status.as_str(), "running" | "ready" | "live");
    !cut_stands_in && !preview_shows
}

/// The chosen clip's masks and motion path, as the picture shows them.
#[derive(Clone, Debug)]
pub(super) struct Shown {
    pub(super) clip: String,
    pub(super) placement: overlay::Placement,
    pub(super) masks: Vec<overlay::Mask>,
    pub(super) path: Option<Vec<Pt>>,
    /// How far along its path the clip is now.
    pub(super) along: f64,
    /// Seconds into the program it is shown at.
    pub(super) position: f64,
    /// A path in depth that no one map shows, sampled where the clip's
    /// anchor goes (`overlay::sampled`), its points where a drag has them.
    pub(super) sampled: Option<overlay::Sampled>,
}

impl Shown {
    /// Whether the picture's clicks are its own: there is something to drag.
    /// A path is only here while some of it can be seen (`Stage::overlays`).
    pub(super) fn edits(&self) -> bool {
        self.edits_masks() || self.path.is_some()
    }

    /// Whether its masks take the picture's clicks: it has some, and its
    /// picture can be seen. Turned edge on or behind the camera, a click on
    /// the picture plays as it would with none.
    pub(super) fn edits_masks(&self) -> bool {
        !self.masks.is_empty() && self.placement.masks_seen()
    }

    pub(super) fn aspect(&self) -> f64 {
        let (width, height) = self.placement.canvas;
        width.max(1.0) / height.max(1.0)
    }

    /// Fractions of the clip's picture to the window, with the canvas drawn
    /// at `picture`; none when the clip cannot be seen.
    pub(super) fn to_window(&self, picture: Bounds<Pixels>) -> Option<Projective> {
        let (width, height) = self.placement.canvas;
        let (x, y) = to_pt(picture.origin);
        let (across, down) = to_pt(point(picture.size.width, picture.size.height));
        self.placement.picture.shows().then(|| {
            self.placement
                .picture
                .then(Affine::scale(
                    across / width.max(1.0),
                    down / height.max(1.0),
                ))
                .then(Affine::translate(x, y))
        })
    }

    /// A point of the clip's motion path to the window; none when no one
    /// map shows it.
    pub(super) fn path_to_window(&self, picture: Bounds<Pixels>) -> Option<Projective> {
        let (x, y) = to_pt(picture.origin);
        let (across, down) = to_pt(point(picture.size.width, picture.size.height));
        self.placement.path.shows().then(|| {
            self.placement
                .path
                .then(Affine::scale(across, down))
                .then(Affine::translate(x, y))
        })
    }

    /// Where the path's point `index` is in the window: on its sampled
    /// curve, or as its map shows it; none out of sight.
    pub(super) fn point_place(&self, picture: Bounds<Pixels>, index: usize) -> Option<Pt> {
        if let Some(sampled) = &self.sampled {
            return sampled
                .handles
                .get(index)
                .copied()
                .flatten()
                .map(|share| share_to_window(picture, share));
        }
        let at = *self.path.as_ref()?.get(index)?;
        let map = self.path_to_window(picture)?;
        map.in_sight(at).then(|| map.apply(at))
    }

    /// With what a drag holds in place of what the snapshot says.
    fn holding(&mut self, drag: &Drag) {
        match drag {
            Drag::Point { clip, now, .. } if *clip == self.clip => self.path = Some(now.clone()),
            Drag::Mask {
                clip, name, now, ..
            } if *clip == self.clip => {
                if let Some(mask) = self.masks.iter_mut().find(|mask| mask.name == *name) {
                    mask.outline = now.clone();
                }
            }
            Drag::Added { clip, mask }
                if *clip == self.clip && self.masks.iter().all(|has| has.name != mask.name) =>
            {
                self.masks.push(mask.clone());
            }
            _ => {}
        }
    }

    /// The handle of the mask `chosen` under `at`.
    pub(super) fn handle_at(
        &self,
        picture: Bounds<Pixels>,
        chosen: Option<&str>,
        at: Pt,
    ) -> Option<usize> {
        let mask = self
            .masks
            .iter()
            .find(|mask| Some(mask.name.as_str()) == chosen)?;
        let map = self.to_window(picture)?;
        square_at(&mask.outline.handles(), map, at)
    }

    /// The mask under `at`: the chosen one first, then the last drawn.
    pub(super) fn mask_at(
        &self,
        picture: Bounds<Pixels>,
        chosen: Option<&str>,
        at: Pt,
    ) -> Option<String> {
        let map = self.to_window(picture)?;
        let hit = |mask: &overlay::Mask| {
            let (line, closed) = mask.outline.line();
            // What is out of sight is not there to press.
            let closed = closed && line.iter().all(|point| map.in_sight(*point));
            let line: Vec<Pt> = line
                .into_iter()
                .filter(|point| map.in_sight(*point))
                .map(|point| map.apply(point))
                .collect();
            (closed && overlay::inside(&line, at))
                || overlay::distance_to_line(&line, closed, at) <= REACH
        };
        if let Some(mask) = self
            .masks
            .iter()
            .find(|mask| Some(mask.name.as_str()) == chosen)
            .filter(|mask| hit(mask))
        {
            return Some(mask.name.clone());
        }
        self.masks
            .iter()
            .rev()
            .find(|mask| hit(mask))
            .map(|mask| mask.name.clone())
    }

    /// The point of the motion path under `at`.
    pub(super) fn point_at(&self, picture: Bounds<Pixels>, at: Pt) -> Option<usize> {
        let count = self.path.as_ref()?.len();
        (0..count).rev().find(|index| {
            self.point_place(picture, *index)
                .is_some_and(|(x, y)| (x - at.0).abs() <= REACH && (y - at.1).abs() <= REACH)
        })
    }

    /// How far along the path's curve `at` is, in spans, when it is on it.
    /// A sampled path's curve takes no point.
    pub(super) fn curve_at(&self, picture: Bounds<Pixels>, at: Pt) -> Option<f64> {
        if self.sampled.is_some() {
            return None;
        }
        let points = self.path.as_ref()?;
        let map = self.path_to_window(picture)?;
        let samples = MotionPath::new(points.clone(), self.aspect()).samples();
        let mut nearest = REACH;
        let mut found = None;
        for (index, pair) in samples.windows(2).enumerate() {
            // In depth, what is on or behind the camera breaks the curve.
            if !map.in_sight(pair[0]) || !map.in_sight(pair[1]) {
                continue;
            }
            let (from, to) = (map.apply(pair[0]), map.apply(pair[1]));
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length = dx * dx + dy * dy;
            let t = if length > 0.0 {
                (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let distance = apart((from.0 + dx * t, from.1 + dy * t), at);
            if distance <= nearest {
                nearest = distance;
                found = Some((index as f64 + t) / overlay::SAMPLES_PER_SPAN as f64);
            }
        }
        found
    }
}

/// A place in fractions of the canvas, drawn at `picture`, in the window.
pub(super) fn share_to_window(picture: Bounds<Pixels>, (x, y): Pt) -> Pt {
    let (left, top) = to_pt(picture.origin);
    let (across, down) = to_pt(point(picture.size.width, picture.size.height));
    (left + x * across, top + y * down)
}

/// A place in the window as fractions of the canvas drawn at `picture`.
pub(super) fn window_to_share(picture: Bounds<Pixels>, (x, y): Pt) -> Pt {
    let (left, top) = to_pt(picture.origin);
    let (across, down) = to_pt(point(picture.size.width, picture.size.height));
    ((x - left) / across.max(1.0), (y - top) / down.max(1.0))
}

/// The square of `points`, mapped by `map`, under `at`: the last drawn
/// first, and none out of sight.
fn square_at(points: &[Pt], map: Projective, at: Pt) -> Option<usize> {
    points.iter().rposition(|point| {
        if !map.in_sight(*point) {
            return false;
        }
        let (x, y) = map.apply(*point);
        (x - at.0).abs() <= REACH && (y - at.1).abs() <= REACH
    })
}

/// A press on the picture's overlays, followed until its button comes up.
#[derive(Clone, Debug)]
pub(super) enum Drag {
    /// A mask by a handle, or moved whole (no handle).
    Mask {
        clip: String,
        name: String,
        handle: Option<usize>,
        /// Where the press was, in fractions of the clip's picture.
        from: Pt,
        original: Outline,
        now: Outline,
        pressed: Pt,
        moved: bool,
    },
    /// A point of the motion path.
    Point {
        clip: String,
        index: usize,
        before: Vec<Pt>,
        now: Vec<Pt>,
        /// From the pointer to the point, so the point does not jump to it.
        held: Pt,
        pressed: Pt,
        moved: bool,
    },
    /// A rectangle or an ellipse being dragged out by its tool, corner to
    /// corner, in fractions of the clip's picture.
    Draw {
        clip: String,
        tool: MaskTool,
        from: Pt,
        to: Pt,
    },
    /// Not a drag: a mask just drawn, shown until kerfd's snapshot has it.
    Added { clip: String, mask: overlay::Mask },
}

/// What a mask tool has drawn so far, as the picture shows it.
#[derive(Clone, Debug, Default)]
pub(super) struct Drawn {
    /// A rectangle or an ellipse being dragged out.
    pub(super) outline: Option<Outline>,
    /// The pen's corners, in fractions of the clip's picture.
    pub(super) pen: Vec<Pt>,
    /// Where the pointer is, in the window, for the pen's next side.
    pub(super) pointer: Option<Pt>,
}

impl StageView {
    /// The chosen clip's masks and motion path where the picture shows the
    /// program ([`shows_program`]); none while a proposal's cut or its
    /// preview stands in for it.
    /// Its path only while some of it can be seen: a path in depth no one
    /// map shows is sampled where the clip's anchor goes, its points where a
    /// drag has them; one all on or behind the camera is no path to show
    /// or edit, unless one of its points is in hand.
    pub(super) fn overlays(&self, session: &Session) -> Option<Shown> {
        let state = &session.state;
        if !shows_program(state, session.timeline().has_clips()) {
            return None;
        }
        let clip = session.selection.first()?;
        let placement = overlay::placement(&self.program, clip, state.position)?;
        let (data, _) = overlay::clip_in(&self.program, clip)?;
        let mut shown = Shown {
            clip: clip.clone(),
            placement,
            masks: overlay::masks_of(data),
            path: overlay::path_in(data),
            along: overlay::property_at(&data.props, "along", placement.clip_time, 0.0),
            position: state.position,
            sampled: None,
        };
        for drag in [
            self.held.as_ref().map(|(drag, _, _)| drag),
            self.drag.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            shown.holding(drag);
        }
        if !shown.placement.path.shows() {
            shown.sampled = overlay::sampled(
                &self.program,
                &shown.clip,
                shown.position,
                shown.path.as_deref(),
            );
        }
        let in_hand = matches!(&self.drag, Some(Drag::Point { clip, .. }) if *clip == shown.clip);
        let seen = shown.sampled.is_some()
            || shown
                .path
                .as_deref()
                .is_some_and(|points| shown.placement.path_seen(points));
        if !seen && !in_hand {
            shown.path = None;
        }
        Some(shown)
    }

    /// The overlays as last drawn: what a press and the pointer hit.
    pub(super) fn shown(&self) -> Option<std::rc::Rc<Shown>> {
        self.shown.borrow().clone()
    }

    /// Follows what the picture shows over the chosen clip as it draws
    /// (`shown`): a picture that cannot be seen cannot be drawn on (the tool
    /// goes down, and what was being drawn is let go); with no clip shown at
    /// all, whatever was in hand goes too; with nothing to take a press, the
    /// cursor is the stage's own again.
    pub(super) fn follow_shown(&mut self, shown: Option<&Shown>) {
        if shown.is_none() {
            self.drag = None;
        }
        if !shown.is_some_and(|shown| shown.placement.masks_seen()) {
            self.tool = None;
            self.end_drawing();
        }
        if self.tool.is_none() && shown.is_none_or(|shown| !shown.edits()) {
            self.cursor = None;
        }
    }
}
