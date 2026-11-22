//! The tools: what a press or a drag on a clip does (the registry's
//! `tool.*`), what each is called, and the pointer each shows over a clip.

use gpui::CursorStyle;

/// What a press or a drag on a clip does (the registry's `tool.*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Chooses, moves and trims.
    #[default]
    Selection,
    /// Splits the clip where it is pressed.
    Razor,
    /// Trims, closing or opening the gap after.
    Ripple,
    /// Moves the cut between two clips.
    Roll,
    /// Shows other media in the same span.
    Slip,
    /// Moves a clip between its neighbours.
    Slide,
}

impl Tool {
    pub const ALL: [Tool; 6] = [
        Tool::Selection,
        Tool::Razor,
        Tool::Ripple,
        Tool::Roll,
        Tool::Slip,
        Tool::Slide,
    ];

    /// Its command in the registry, whose keys choose it.
    pub fn command(self) -> &'static str {
        match self {
            Tool::Selection => "tool.selection",
            Tool::Razor => "tool.razor",
            Tool::Ripple => "tool.ripple",
            Tool::Roll => "tool.roll",
            Tool::Slip => "tool.slip",
            Tool::Slide => "tool.slide",
        }
    }

    /// The tool command `id` chooses.
    pub fn from_command(id: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.command() == id)
    }

    /// What it is called on the strip and in its tip.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Selection => "Selection",
            Tool::Razor => "Razor",
            Tool::Ripple => "Ripple Edit",
            Tool::Roll => "Rolling Edit",
            Tool::Slip => "Slip",
            Tool::Slide => "Slide",
        }
    }

    /// A clip's edges take a drag.
    pub fn trims_edges(self) -> bool {
        matches!(self, Tool::Selection | Tool::Ripple | Tool::Roll)
    }

    /// A clip's body takes a drag.
    pub fn drags_bodies(self) -> bool {
        matches!(self, Tool::Selection | Tool::Slip | Tool::Slide)
    }

    /// The pointer over a clip, on one of its edges or not: it shows what
    /// the tool would do there.
    pub fn cursor(self, edge: bool) -> CursorStyle {
        if self == Tool::Razor {
            CursorStyle::Crosshair
        } else if edge && self.trims_edges() {
            if self == Tool::Roll {
                CursorStyle::ResizeColumn
            } else {
                CursorStyle::ResizeLeftRight
            }
        } else if self.drags_bodies() {
            if self == Tool::Selection {
                CursorStyle::OpenHand
            } else {
                CursorStyle::ResizeLeftRight
            }
        } else {
            CursorStyle::Arrow
        }
    }
}
