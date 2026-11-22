//! Tab and Shift-Tab across the window from a field or a control of the kit:
//! on to the next stop, or back, passing over what is folded away to nothing,
//! going round a panel open over the window only, and on from a panel's or a
//! region's own box where nothing binds Tab.

use super::{panel_of, tab_trap, FocusNext, FocusPrevious};
use gpui::{
    App, Bounds, FocusHandle, KeyContext, Keymap, Keystroke, Pixels, Point, WeakFocusHandle, Window,
};
use std::cell::{Cell, RefCell};

/// Tab (`forward`) or Shift-Tab from what has the keys: on to the window's
/// next tab stop, or back, passing over what is folded away to nothing
/// ([`note_shown`]). Where what holds it binds Tab itself (the launcher's
/// own stops, the sidebar's), it goes on to that instead. Inside a panel
/// open over the window ([`PANEL`](super::PANEL), or one named in
/// [`trap_tab`](super::trap_tab)) it goes round that panel only; with
/// nothing there to go to, the keys stay where they were. A field, every
/// control of the kit, and a panel itself ([`follow_tab`]) call this.
pub fn tab_to(forward: bool, window: &mut Window, cx: &mut App) {
    let keymap = cx.key_bindings();
    let stack = window.context_stack();
    let taken = tab_is_taken(&keymap.borrow(), &stack, forward);
    if taken {
        cx.propagate();
        return;
    }
    let trap = tab_trap();
    let panel = panel_of(&stack);
    let start = window.focused(cx);
    let mut first = None;
    for _ in 0..TAB_MOST {
        if forward {
            window.focus_next();
        } else {
            window.focus_prev();
        }
        let now = window.focused(cx);
        if now == start {
            return;
        }
        // Round every stop once and none would do.
        if first.is_some() && now == first {
            break;
        }
        if first.is_none() {
            first.clone_from(&now);
        }
        if now.as_ref().is_some_and(is_put_away) {
            continue;
        }
        let inside = match (&trap, panel) {
            (Some(trap), _) => trap.contains_focused(window, cx),
            (None, Some(panel)) => panel_of(&window.context_stack()) == Some(panel),
            (None, None) => true,
        };
        if inside {
            return;
        }
    }
    match start {
        Some(start) => window.focus(&start),
        None => window.blur(),
    }
}

thread_local! {
    /// Whether Tab where nothing binds it is followed ([`follow_tab`]).
    static FOLLOWING_TAB: Cell<bool> = const { Cell::new(false) };
    /// The fields and controls of the kit folded away to nothing when they
    /// were last laid out ([`note_shown`]).
    static PUT_AWAY: RefCell<Vec<WeakFocusHandle>> = const { RefCell::new(Vec::new()) };
}

/// Has Tab and Shift-Tab go on from what has the keys where nothing binds
/// them ([`tab_goes_on_from`]) as they do from a field or a control of the
/// kit ([`tab_to`]): into a panel open over the window from the panel
/// itself (Settings, Finish, a question, the export popover open with the
/// keys on their own box), and on from a region's own box (the Media
/// band's) rather than stopping there. `window_context` is the key context
/// of the window's own box, where it is left alone ([`tab_goes_on_from`]).
/// Once is enough: `keymap::bind` calls it each time it binds a keymap.
pub fn follow_tab(window_context: &'static str, cx: &mut App) {
    if FOLLOWING_TAB.with(|following| following.replace(true)) {
        return;
    }
    cx.intercept_keystrokes(move |event, window, cx| {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if keystroke.key != "tab"
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
        {
            return;
        }
        let forward = !modifiers.shift;
        let goes_on = {
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            tab_goes_on_from(&keymap, &event.context_stack, forward, window_context)
        };
        if goes_on {
            tab_to(forward, window, cx);
            cx.stop_propagation();
        }
    })
    .detach();
}

/// Whether Tab (or Shift-Tab, not `forward`) from what holds the keys
/// with `stack` goes on as from a control of the kit ([`follow_tab`]):
/// nothing binds it there ([`tab_is_unbound`]), and it is not the window's
/// own box (key context `window_context`), from where the sidebar takes it.
pub fn tab_goes_on_from(
    keymap: &Keymap,
    stack: &[KeyContext],
    forward: bool,
    window_context: &str,
) -> bool {
    let at_window = stack
        .last()
        .is_none_or(|context| context.contains(window_context));
    !at_window && tab_is_unbound(keymap, stack, forward)
}

/// Whether Tab (or Shift-Tab, not `forward`) with `stack` holding the keys
/// is bound to nothing at all.
pub fn tab_is_unbound(keymap: &Keymap, stack: &[KeyContext], forward: bool) -> bool {
    let Ok(key) = Keystroke::parse(if forward { "tab" } else { "shift-tab" }) else {
        return false;
    };
    keymap.bindings_for_input(&[key], stack).0.is_empty()
}

/// Where a field or a control of the kit holding `focus` was laid out
/// (`bounds`, inside what clips it now): folded away to nothing (in a box
/// put away to no height, as the Media band is, or slid off the window, as
/// the sidebar is), Tab passes it over ([`is_folded_away`]); scrolled out
/// of sight in a list, it is still reached. The kit's controls and the
/// field call this as they are laid out.
pub fn note_shown(focus: &FocusHandle, bounds: Bounds<Pixels>, window: &Window) {
    let shown = Bounds::new(Point::default(), window.viewport_size());
    let away = is_folded_away(bounds, window.content_mask().bounds, shown);
    PUT_AWAY.with(|put_away| {
        let mut put_away = put_away.borrow_mut();
        let at = put_away.iter().position(|weak| weak == &focus.downgrade());
        match (at, away) {
            (Some(at), false) => {
                put_away.swap_remove(at);
            }
            (None, true) => {
                put_away.retain(|weak| weak.upgrade().is_some());
                put_away.push(focus.downgrade());
            }
            _ => {}
        }
    });
}

/// Whether a control laid out at `bounds` inside a clip of `mask`, in a
/// window showing `window`, is folded away to nothing: it has no size; a
/// box round it was put away to none, which leaves a clip of no room at
/// all; or nothing but the window clips it and it lies wholly outside it
/// (a region slid off its edge). Scrolled wholly out of a list it is not:
/// the clip it is in is then the list's, or less than none (one that ends
/// before it begins).
pub fn is_folded_away(
    bounds: Bounds<Pixels>,
    mask: Bounds<Pixels>,
    window: Bounds<Pixels>,
) -> bool {
    let sized = bounds.size.width > Pixels::ZERO && bounds.size.height > Pixels::ZERO;
    let shut = mask.size.width == Pixels::ZERO || mask.size.height == Pixels::ZERO;
    let off_window = mask == window && !bounds.intersects(&window);
    !sized || shut || off_window
}

/// Whether `focus` was folded away to nothing when last laid out.
fn is_put_away(focus: &FocusHandle) -> bool {
    let weak = focus.downgrade();
    PUT_AWAY.with(|put_away| put_away.borrow().iter().any(|away| away == &weak))
}

/// The most stops Tab passes over looking for one inside a panel.
const TAB_MOST: usize = 512;

/// Whether Tab (or Shift-Tab, not `forward`) with `stack` holding the keys
/// is bound to something other than going on from a field or a control of
/// the kit.
pub fn tab_is_taken(keymap: &Keymap, stack: &[KeyContext], forward: bool) -> bool {
    let Ok(key) = Keystroke::parse(if forward { "tab" } else { "shift-tab" }) else {
        return false;
    };
    let (bindings, _) = keymap.bindings_for_input(&[key], stack);
    // A code field's own Tab (it types an indent) is no stop of a view's:
    // Ctrl+Tab leaves the field from there.
    bindings.iter().any(|binding| {
        let action = binding.action();
        !(action.partial_eq(&FocusNext)
            || action.partial_eq(&FocusPrevious)
            || action.partial_eq(&crate::input::Indent)
            || action.partial_eq(&crate::input::Outdent))
    })
}
