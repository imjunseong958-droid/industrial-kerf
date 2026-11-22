//! The kit every region is built from: buttons, icon buttons, a switch, a
//! segmented control, a menu, tips, chips and labels, set in the editor's
//! tokens.
//!
//! What takes a press takes the keys too: Tab reaches it (Shift-Tab goes
//! back), a ring of the accent shows just inside its edge while it has
//! them (outside it, on what is filled with the accent), and Space or
//! Enter presses it. A button that does nothing pressed itself (what holds
//! it opens a menu on the press) is no stop, nor is a control folded away
//! to nothing. A press of the pointer leaves the keys where they were, so
//! the ring shows only for the keyboard. A button's hover fades in and out
//! over `HOVER`. A control's tip names it
//! and its key (in mono, dimmer); a greyed one says why it cannot be
//! pressed. A tip shows after `TIP_DELAY`, fading in over `FADE`, and fades
//! out as it goes; once one has shown (the kit's or GPUI's own), the next
//! one under the pointer shows at once. It says what its control says as
//! it is drawn, is drawn over everything (a menu's rows' tips too), and
//! goes at once when a panel opens over its control. Under Reduce Motion
//! all of it is simply there.

mod button;
mod control;
mod icon_button;
mod marks;
mod menu;
mod panel;
mod scroll_fade;
mod segmented;
mod switch;
mod tab;
mod tip;
mod words;

pub use button::*;
pub use control::*;
pub use icon_button::*;
pub use marks::*;
pub use menu::*;
pub use panel::*;
pub use scroll_fade::*;
pub use segmented::*;
pub use switch::*;
pub use tab::*;
pub use tip::*;
pub use words::*;

use gpui::{actions, App, ClickEvent, KeyBinding, NoAction, Window};
use std::rc::Rc;

/// What a press of a control runs.
type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

actions!(
    kit,
    [
        FocusNext,
        FocusPrevious,
        SegmentPrevious,
        SegmentNext,
        MenuUp,
        MenuDown,
        MenuFirst,
        MenuLast,
        MenuChoose,
        MenuClose,
        MenuOpen,
        MenuBack,
    ]
);

/// The key context of a control of the kit that has the keys: Space and
/// Enter press it (as they come up) and run nothing of the window's; Tab
/// and Shift-Tab go on.
pub const CONTROL: &str = "KitControl";
/// A segmented control's, inside [`CONTROL`]: the arrows choose.
pub const SEGMENTED: &str = "KitSegmented";
/// What a segmented control answers in: [`CONTROL`] and [`SEGMENTED`].
pub const CONTROL_SEGMENTED: &str = "KitControl KitSegmented";
/// An open [`Menu`]'s: the arrows move in it (Tab and Shift-Tab too, so
/// the keys stay in a menu while it is open), Right opens a row's submenu
/// and Left closes it, Enter or Space chooses, Esc closes it and nothing
/// else.
pub const MENU: &str = "KitMenu";

/// The kit's keys. `keymap::bind` binds them with the field's, first
/// whenever it binds a keymap.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", NoAction, Some(CONTROL)),
        KeyBinding::new("space", NoAction, Some(CONTROL)),
        KeyBinding::new("tab", FocusNext, Some(CONTROL)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(CONTROL)),
        KeyBinding::new("left", SegmentPrevious, Some(SEGMENTED)),
        KeyBinding::new("right", SegmentNext, Some(SEGMENTED)),
        KeyBinding::new("up", MenuUp, Some(MENU)),
        KeyBinding::new("down", MenuDown, Some(MENU)),
        KeyBinding::new("shift-tab", MenuUp, Some(MENU)),
        KeyBinding::new("tab", MenuDown, Some(MENU)),
        KeyBinding::new("home", MenuFirst, Some(MENU)),
        KeyBinding::new("end", MenuLast, Some(MENU)),
        KeyBinding::new("enter", MenuChoose, Some(MENU)),
        KeyBinding::new("space", MenuChoose, Some(MENU)),
        KeyBinding::new("escape", MenuClose, Some(MENU)),
        KeyBinding::new("right", MenuOpen, Some(MENU)),
        KeyBinding::new("left", MenuBack, Some(MENU)),
    ]
}
