//! A panel open over the window: the key contexts that mark one, and where
//! it sits.

use crate::theme::layout;
use gpui::{point, px, size, Bounds, KeyContext, Pixels, Size};

/// The key context that marks a panel open over the window: Tab from inside
/// it goes round it only. The panels that were made before it are known by
/// their own contexts ([`PANELS`]).
pub const PANEL: &str = "KitPanel";

/// The key contexts of the panels open over the window (Settings, Finish,
/// a preset's dialog, the command palette, a shot's plan or graph, a
/// question, the export popover), and [`PANEL`].
pub const PANELS: [&str; 9] = [
    PANEL,
    "Settings",
    "Finish",
    "PresetDialog",
    "CommandPalette",
    "Making",
    "Confirm",
    "KerfPrompt",
    "ExportPopover",
];

/// The innermost panel in `stack`, by the context that names it.
pub fn panel_of(stack: &[KeyContext]) -> Option<&'static str> {
    stack
        .iter()
        .rev()
        .find_map(|context| PANELS.iter().copied().find(|panel| context.contains(panel)))
}

/// Where a panel one goes into (Settings, Finish) sits in a window
/// `window` big: as big as `most`, or the window less `MODAL_MARGIN` all
/// round where that is smaller, centred across, and `MODAL_ABOVE` of the
/// height to spare above it. `Workspace::modal` places its panel here.
pub fn modal_bounds(most: Size<Pixels>, window: Size<Pixels>) -> Bounds<Pixels> {
    let margin = px(layout::MODAL_MARGIN * 2.0);
    let width = most.width.min(window.width - margin).max(Pixels::ZERO);
    let height = most.height.min(window.height - margin).max(Pixels::ZERO);
    let left = ((window.width - width) / 2.0).floor();
    let top = px((f32::from(window.height - height) * layout::MODAL_ABOVE).floor());
    Bounds::new(point(left, top), size(width, height))
}
