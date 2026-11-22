//! The mask tools: a rectangle or an ellipse dragged out corner to corner,
//! or the pen's corners clicked (closed on its first, or left open by
//! Return), drawn as a new mask on the chosen picture clip. A tool out has
//! the picture's presses and keys, and the picture holds still under it.

use super::overlay::{self, Outline, Pt};
use super::picture::apart;
use super::shown::{Drag, Drawn, Shown, REACH};
use super::{editing::DRAG_START, ensure_keys, CancelDrawing, FinishPen, StageView};
use crate::icons::Icon;
use gpui::{App, Bounds, Context, Pixels, Window};

/// A tool that draws a new mask on the chosen picture clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskTool {
    Rectangle,
    Ellipse,
    Pen,
}

impl MaskTool {
    pub const ALL: [MaskTool; 3] = [MaskTool::Rectangle, MaskTool::Ellipse, MaskTool::Pen];

    /// Its name, as its button's tip and the app's menu say it.
    pub fn label(self) -> &'static str {
        match self {
            MaskTool::Rectangle => "Rectangle mask",
            MaskTool::Ellipse => "Ellipse mask",
            MaskTool::Pen => "Pen mask",
        }
    }

    /// What to do with it, while it is out.
    pub fn hint(self) -> &'static str {
        match self {
            MaskTool::Rectangle | MaskTool::Ellipse => "Drag on the picture to draw it",
            MaskTool::Pen => {
                "Click its corners, then the first again to close it (Return leaves it open)"
            }
        }
    }

    /// Its key, as the registry writes keys.
    pub fn shortcut(self) -> &'static str {
        match self {
            MaskTool::Rectangle => "Shift+R",
            MaskTool::Ellipse => "Shift+E",
            MaskTool::Pen => "Shift+P",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            MaskTool::Rectangle => Icon::MaskRectangle,
            MaskTool::Ellipse => Icon::MaskEllipse,
            MaskTool::Pen => Icon::Pen,
        }
    }

    /// Its button's id in the transport.
    pub(super) fn id(self) -> &'static str {
        match self {
            MaskTool::Rectangle => "mask-rectangle",
            MaskTool::Ellipse => "mask-ellipse",
            MaskTool::Pen => "mask-pen",
        }
    }
}

/// How the picture's mask tools stand among a menu's
/// [`crate::commands::MenuStates`] and the app menu's states and ticks:
/// each by an id of its own, as if it were a command of the registry's
/// (which does not carry them).
pub fn mask_tool_id(tool: MaskTool) -> &'static str {
    match tool {
        MaskTool::Rectangle => "mask.rectangle",
        MaskTool::Ellipse => "mask.ellipse",
        MaskTool::Pen => "mask.pen",
    }
}

impl StageView {
    /// Whether there is a picture clip chosen that a mask tool can draw on:
    /// one the picture places where it can be seen, not turned edge on or
    /// behind the camera (an invertible placement), on the program shown
    /// rather than a proposal. The mask tools are greyed in the menus while
    /// it is false and none is out ([`StageView::mask_tool`]).
    pub fn can_draw(&self, cx: &App) -> bool {
        self.overlays(self.session.read(cx))
            .is_some_and(|shown| shown.placement.masks_seen())
    }

    /// The mask tool out, if one is.
    pub fn mask_tool(&self) -> Option<MaskTool> {
        self.tool
    }

    /// Takes out `tool`, or puts it down if it is out (⇧R ⇧E ⇧P, the
    /// transport's tools). False, with a notice saying why, when no picture
    /// clip is chosen to draw on.
    pub fn toggle_tool(
        &mut self,
        tool: MaskTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.tool == Some(tool) {
            self.set_tool(None, window, cx);
            return true;
        }
        if !self.can_draw(cx) {
            self.session.update(cx, |session, cx| {
                session.state.notify(crate::commands::NO_MASK_CLIP, false);
                cx.notify();
            });
            return false;
        }
        self.set_tool(Some(tool), window, cx);
        true
    }

    /// Takes out `tool` (None puts the one out down): what was being drawn
    /// is let go, and the picture takes the keys and holds still.
    pub fn set_tool(
        &mut self,
        tool: Option<MaskTool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tool = tool.filter(|_| self.can_draw(cx));
        if tool == self.tool {
            return;
        }
        self.end_drawing();
        self.tool = tool;
        if tool.is_some() {
            ensure_keys(cx);
            self.end_skim(cx);
            window.focus(&self.focus_handle);
        }
        cx.notify();
    }

    /// Lets go of a rectangle or ellipse being dragged out and of the
    /// pen's corners.
    pub(super) fn end_drawing(&mut self) {
        if matches!(self.drag, Some(Drag::Draw { .. })) {
            self.drag = None;
        }
        self.pen.clear();
        self.pointer = None;
    }

    /// Whether a mask is being drawn: dragged out, or the pen's corners.
    pub(super) fn drawing(&self) -> bool {
        !self.pen.is_empty() || matches!(self.drag, Some(Drag::Draw { .. }))
    }

    /// What a mask tool has drawn so far, as the picture shows it.
    pub(super) fn in_hand(&self) -> Drawn {
        Drawn {
            outline: match &self.drag {
                Some(Drag::Draw { tool, from, to, .. }) => {
                    Some(overlay::dragged(*tool == MaskTool::Ellipse, *from, *to))
                }
                _ => None,
            },
            pen: self.pen.clone(),
            pointer: self.pointer,
        }
    }

    /// Whether a press at `at` (in the window) is on the pen's first corner
    /// with enough corners to close the outline.
    fn pen_closes(&self, shown: &Shown, picture: Bounds<Pixels>, at: Pt) -> bool {
        let Some(map) = shown.to_window(picture) else {
            return false;
        };
        self.pen.len() >= 3
            && self.pen.first().is_some_and(|first| {
                let (x, y) = map.apply(*first);
                map.in_sight(*first) && (x - at.0).abs() <= REACH && (y - at.1).abs() <= REACH
            })
    }

    /// A press with a mask tool out: a corner of the pen (its first again
    /// closes it), or the start of a rectangle or an ellipse.
    pub(super) fn press_tool(
        &mut self,
        tool: MaskTool,
        shown: &Shown,
        picture: Bounds<Pixels>,
        at: Pt,
        cx: &mut Context<Self>,
    ) {
        // A press past the horizon, where none of the picture is, draws nothing.
        let Some(map) = shown.to_window(picture) else {
            return;
        };
        let Some(on) = map.point_in_sight(at) else {
            return;
        };
        match tool {
            MaskTool::Pen => {
                if self.pen_closes(shown, picture, at) {
                    self.finish_pen(true, cx);
                    return;
                }
                // A second press of a double-click is not another corner.
                let again = self
                    .pen
                    .last()
                    .is_some_and(|last| apart(map.apply(*last), at) < DRAG_START);
                if !again {
                    self.pen.push(on);
                }
                self.pointer = Some(at);
            }
            MaskTool::Rectangle | MaskTool::Ellipse => {
                self.drag = Some(Drag::Draw {
                    clip: shown.clip.clone(),
                    tool,
                    from: on,
                    to: on,
                });
            }
        }
        cx.notify();
    }

    /// The pen's corners become a mask: closed on its first corner (three
    /// or more), or left open by Return (two or more). False when there are
    /// too few.
    fn finish_pen(&mut self, closed: bool, cx: &mut Context<Self>) -> bool {
        if self.pen.len() < if closed { 3 } else { 2 } {
            return false;
        }
        let Some(clip) = self.shown().map(|shown| shown.clip.clone()) else {
            return false;
        };
        let points = std::mem::take(&mut self.pen);
        self.pointer = None;
        self.add_mask(&clip, Outline::Points { points, closed }, cx);
        true
    }

    /// Adds `outline` to `clip` as a new mask, named the first of `k1`,
    /// `k2`, … the clip does not use. It shows, chosen, at once; the tool
    /// is put down.
    pub(super) fn add_mask(&mut self, clip: &str, outline: Outline, cx: &mut Context<Self>) {
        let mut taken = overlay::clip_in(&self.program, clip)
            .map(|(data, _)| overlay::mask_names(data))
            .unwrap_or_default();
        if let Some((Drag::Added { clip: held, mask }, _, _)) = &self.held {
            if held == clip {
                taken.push(mask.name.clone());
            }
        }
        let name = overlay::new_mask_name(&taken);
        let op = overlay::mask_add(clip, &name, &outline);
        self.tool = None;
        self.chosen_point = None;
        self.choose_mask(Some(name.clone()), cx);
        let mask = overlay::Mask {
            name,
            mode: "add".to_owned(),
            outline,
        };
        self.send_held(
            Drag::Added {
                clip: clip.to_owned(),
                mask,
            },
            op,
            cx,
        );
        cx.notify();
    }

    /// Esc: lets go of what is being drawn, else puts the tool down; with
    /// neither, the window's Esc has it.
    pub(super) fn cancel_drawing(
        &mut self,
        _: &CancelDrawing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drawing() {
            self.end_drawing();
            cx.notify();
        } else if self.tool.is_some() {
            self.set_tool(None, window, cx);
        } else {
            cx.propagate();
        }
    }

    /// Return: the pen's corners become an open outline, as Stroke draws along.
    pub(super) fn finish_pen_open(
        &mut self,
        _: &FinishPen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.finish_pen(false, cx) {
            cx.propagate();
        }
    }
}
