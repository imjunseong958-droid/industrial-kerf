//! The kit every region is built from: buttons, icon buttons, a switch, a
//! segmented control, a menu, tips, chips and labels, set in the editor's
//! tokens, as Qt's ui/components gave every view.
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
//! pressed. A tip shows after `TIP_DELAY`, fading in over `FADE`; once one
//! has shown, the next one under the pointer shows at once. Under Reduce
//! Motion all of it is simply there.

use crate::icons::Icon;
use crate::motion::{self, Tween};
use crate::theme::{self, height, layout, motion as scene, radius, text, TypeStyle};
use gpui::{
    actions, canvas, deferred, div, linear_color_stop, linear_gradient, point, prelude::*, px,
    size, AnimationExt, AnyElement, AnyView, App, Bounds, ClickEvent, Component, Context,
    DismissEvent, Div, ElementId, Entity, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement,
    KeyBinding, KeyContext, KeyDownEvent, Keymap, Keystroke, MouseButton, MouseDownEvent, NoAction,
    Pixels, Point, Rgba, ScrollHandle, SharedString, Size, Stateful, Subscription, Task, TextRun,
    WeakFocusHandle, Window,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

// ---------------------------------------------------------------- keys

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
/// An open [`Menu`]'s: the arrows move in it (Tab and Shift-Tab too, as in
/// Qt's menus), Right opens a row's submenu and Left closes it, Enter or
/// Space chooses, Esc closes it and nothing else.
pub const MENU: &str = "KitMenu";

/// The kit's keys. `input::bind_keys` binds them with the field's, which
/// `commands::bind` binds first whenever it binds a keymap.
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

/// Tab (`forward`) or Shift-Tab from what has the keys: on to the window's
/// next tab stop, or back, passing over what is folded away to nothing
/// ([`note_shown`]). Where what holds it binds Tab itself (the launcher's
/// own stops, the sidebar's), it goes on to that instead. Inside a panel
/// open over the window ([`PANEL`], or one named in [`trap_tab`]) it goes
/// round that panel only; with nothing there to go to, the keys stay where
/// they were. A field, every control of the kit, and a panel itself
/// ([`follow_tab`]) call this.
pub fn tab_to(forward: bool, window: &mut Window, cx: &mut App) {
    let keymap = cx.key_bindings();
    let stack = window.context_stack();
    let taken = tab_is_taken(&keymap.borrow(), &stack, forward);
    if taken {
        cx.propagate();
        return;
    }
    let trap = TAB_TRAP.with(|trap| trap.borrow().as_ref().and_then(WeakFocusHandle::upgrade));
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
/// band's) rather than stopping there. Once is enough:
/// [`crate::input::bind_keys`] calls it each time a keymap is bound.
pub fn follow_tab(cx: &mut App) {
    if FOLLOWING_TAB.with(|following| following.replace(true)) {
        return;
    }
    cx.intercept_keystrokes(|event, window, cx| {
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
            tab_goes_on_from(&keymap, &event.context_stack, forward)
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
/// own box, from where the sidebar takes it.
pub fn tab_goes_on_from(keymap: &Keymap, stack: &[KeyContext], forward: bool) -> bool {
    let at_window = stack
        .last()
        .is_none_or(|context| context.contains(crate::commands::WORKSPACE));
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

/// The most stops Tab passes over looking for one inside a panel.
const TAB_MOST: usize = 512;

thread_local! {
    /// What Tab keeps inside while a panel is open over the window.
    static TAB_TRAP: RefCell<Option<WeakFocusHandle>> = const { RefCell::new(None) };
}

/// While a panel one goes into is open over the window, Tab and Shift-Tab
/// from a field or a control of the kit go round what is inside it only,
/// never to the window behind. A panel with a key context of [`PANELS`]
/// needs nothing more; another can track `within` on its box
/// (`track_focus`) and set it here as it opens, and None as it closes.
pub fn trap_tab(within: Option<&FocusHandle>) {
    TAB_TRAP.with(|trap| *trap.borrow_mut() = within.map(FocusHandle::downgrade));
}

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
            || action.partial_eq(&crate::input::FocusNext)
            || action.partial_eq(&crate::input::FocusPrevious)
            || action.partial_eq(&crate::input::Indent)
            || action.partial_eq(&crate::input::Outdent))
    })
}

// ---------------------------------------------------------------- words

/// Words in a type style and colour.
pub fn label(words: impl Into<SharedString>, style: TypeStyle, color: Rgba) -> gpui::Div {
    theme::type_style(div(), style)
        .text_color(color)
        .child(words.into())
}

/// Words that keep to one line, cut with an ellipsis.
pub fn line(words: impl Into<SharedString>, style: TypeStyle, color: Rgba) -> gpui::Div {
    label(words, style, color)
        .whitespace_nowrap()
        .overflow_hidden()
        .text_ellipsis()
}

/// A hairline, across or down.
pub fn hairline(across: bool) -> gpui::Div {
    let line = div().flex_none().bg(theme::line());
    if across {
        line.h(px(1.0)).w_full()
    } else {
        line.w(px(1.0)).h_full()
    }
}

// ---------------------------------------------------------------- tips

/// What a control's tip says: its name (or why it cannot be pressed now),
/// and its key, set after the words in mono and dimmer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tip {
    pub words: SharedString,
    pub key: Option<SharedString>,
}

impl Tip {
    pub fn new(words: impl Into<SharedString>) -> Self {
        Self {
            words: words.into(),
            key: None,
        }
    }

    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The key, if there is one.
    pub fn maybe_key(mut self, key: Option<impl Into<SharedString>>) -> Self {
        self.key = key.map(Into::into);
        self
    }

    /// One line of it: a tip does not wrap.
    fn one_line(&self) -> Self {
        let flat = |words: &SharedString| -> SharedString {
            if words.contains('\n') {
                words.replace('\n', " ").into()
            } else {
                words.clone()
            }
        };
        Self {
            words: flat(&self.words),
            key: self.key.as_ref().map(flat),
        }
    }
}

/// The tip a control shows: while it can be pressed, its name; greyed,
/// why not (else its name). Its key either way.
pub fn tip_for(
    name: Option<&SharedString>,
    key: Option<&SharedString>,
    disabled: bool,
    reason: Option<&SharedString>,
) -> Option<Tip> {
    let words = if disabled { reason.or(name) } else { name }?;
    Some(Tip::new(words.clone()).maybe_key(key.cloned()))
}

/// The card a tip is drawn on, `width` wide (or as wide as its words).
fn tip_card(tip: &Tip, width: Option<f32>) -> Div {
    theme::type_style(div(), text::CONTROL)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(theme::space::S8))
        .h(px(layout::TIP_HEIGHT))
        .px(px(theme::space::S8))
        .when_some(width, |card, width| card.w(px(width)))
        .rounded(px(radius::CONTROL))
        .bg(theme::card())
        .border_1()
        .border_color(theme::line2())
        .text_color(theme::ink())
        .whitespace_nowrap()
        .shadow_md()
        .child(tip.words.clone())
        .when_some(tip.key.clone(), |card, key| {
            card.child(label(key, text::MONO_SMALL, theme::ink3()))
        })
}

/// How wide `words` are set in `style`.
fn measure(words: &SharedString, style: TypeStyle, window: &Window) -> f32 {
    if words.is_empty() {
        return 0.0;
    }
    let font = if style.mono {
        theme::mono_font(style.weight)
    } else {
        theme::ui_font(style.weight)
    };
    let run = TextRun {
        len: words.len(),
        font,
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(words.clone(), px(style.px), &[run], None);
    f32::from(line.width).ceil()
}

/// How big `tip`'s card is.
fn tip_size(tip: &Tip, window: &Window) -> Size<Pixels> {
    let mut width = theme::space::S8 * 2.0 + measure(&tip.words, text::CONTROL, window) + 2.0;
    if let Some(key) = &tip.key {
        width += theme::space::S8 + measure(key, text::MONO_SMALL, window);
    }
    size(px(width), px(layout::TIP_HEIGHT))
}

/// Where a tip `tip` big goes for a control at `control` in a window
/// `window` big, as Qt placed it: centred on the control, `TIP_EDGE` in
/// from the window's sides at least, `TIP_GAP` under it, or over it when
/// the control is low in the window.
pub fn tip_origin(
    control: Bounds<Pixels>,
    tip: Size<Pixels>,
    window: Size<Pixels>,
) -> Point<Pixels> {
    let (width, edge) = (f32::from(tip.width), layout::TIP_EDGE);
    let centred =
        (f32::from(control.left()) + f32::from(control.size.width) / 2.0 - width / 2.0).round();
    let left = centred
        .min(f32::from(window.width) - edge - width)
        .max(edge);
    let above = f32::from(control.bottom()) > layout::TIP_FLIP_BELOW
        && f32::from(control.top()) > layout::TIP_FLIP_ABOVE;
    let top = if above {
        f32::from(control.top()) - layout::TIP_GAP - f32::from(tip.height)
    } else {
        f32::from(control.bottom()) + layout::TIP_GAP
    };
    point(px(left), px(top.round()))
}

/// Whether a tip asked for now shows at once: one shows still
/// (`showing`), or the last went less than `TIP_QUICK` before `now`.
pub fn tip_is_quick(showing: usize, last_hidden: Option<Instant>, now: Instant) -> bool {
    showing > 0
        || last_hidden.is_some_and(|at| now.saturating_duration_since(at) < scene::TIP_QUICK)
}

thread_local! {
    /// How many tips show now, and when the last one went.
    static TIPS: Cell<(usize, Option<Instant>)> = const { Cell::new((0, None)) };
}

/// What a control keeps while it shows: its keys, its hover, its tip.
pub struct Control {
    focus: FocusHandle,
    hover: Tween,
    /// Enter or Space went down while it had the keys: coming up, they press it.
    armed: bool,
    tip: TipState,
}

#[derive(Default)]
struct TipState {
    hovered: bool,
    /// Since when it shows, and whether it faded in (not when it came at
    /// once after another).
    shown: Option<(Instant, bool)>,
    /// Since when it goes, fading out.
    hiding: Option<Instant>,
    waiting: Option<Task<()>>,
    /// Where the control was laid out last.
    bounds: Option<Bounds<Pixels>>,
}

impl Control {
    fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
            hover: Tween::at(0.0),
            armed: false,
            tip: TipState::default(),
        }
    }

    /// The pointer came over it or left it: its hover fades, and its tip
    /// (`tipped`) waits to show, shows at once, or goes.
    fn set_hovered(&mut self, hovered: bool, tipped: bool, cx: &mut Context<Self>) {
        self.hover.go(if hovered { 1.0 } else { 0.0 }, scene::HOVER);
        self.tip.hovered = hovered;
        if !hovered || !tipped {
            self.hide_tip();
            return;
        }
        if self.tip.shown.is_some() {
            return;
        }
        let now = Instant::now();
        let (showing, last) = TIPS.with(Cell::get);
        if tip_is_quick(showing, last, now) {
            self.show_tip(false, now);
            return;
        }
        self.tip.waiting = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(scene::TIP_DELAY).await;
            let _ = this.update(cx, |control, cx| {
                if control.tip.hovered && control.tip.shown.is_none() {
                    control.show_tip(true, Instant::now());
                    cx.notify();
                }
            });
        }));
    }

    fn show_tip(&mut self, fade: bool, now: Instant) {
        self.tip.waiting = None;
        self.tip.hiding = None;
        self.tip.shown = Some((now, fade));
        TIPS.with(|tips| {
            let (showing, last) = tips.get();
            tips.set((showing + 1, last));
        });
    }

    fn hide_tip(&mut self) {
        self.tip.waiting = None;
        if self.tip.shown.take().is_some() {
            let now = Instant::now();
            self.tip.hiding = Some(now);
            TIPS.with(|tips| {
                let (showing, _) = tips.get();
                tips.set((showing.saturating_sub(1), Some(now)));
            });
        }
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        self.hide_tip();
    }
}

/// The state of the control `id`, kept for as long as it shows.
fn control(id: &ElementId, window: &mut Window, cx: &mut App) -> Entity<Control> {
    window.use_keyed_state(id.clone(), cx, |_, cx| Control::new(cx))
}

/// How far a fade begun `since` has got, 0..1, eased; none under Reduce Motion.
fn faded(since: Instant, now: Instant) -> f32 {
    let duration = motion::duration(scene::FADE);
    if duration.is_zero() {
        return 1.0;
    }
    let t = now.saturating_duration_since(since).as_secs_f32() / duration.as_secs_f32();
    motion::ease(t.clamp(0.0, 1.0))
}

/// `element` with its hover followed in `state` (for its fade) and its tip
/// `tip` shown where the pointer rests on it.
fn hovered(
    element: Stateful<Div>,
    state: &Entity<Control>,
    tip: Option<Tip>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let tipped = tip.is_some();
    let hover_state = state.clone();
    let mut element = element.on_hover(move |hovered, _, cx| {
        hover_state.update(cx, |control, cx| {
            control.set_hovered(*hovered, tipped, cx);
            cx.notify();
        })
    });
    let Some(tip) = tip else {
        return element;
    };
    // A press puts the tip away, as Qt's did.
    let press_state = state.clone();
    element = element.on_mouse_down(MouseButton::Left, move |_, _, cx| {
        press_state.update(cx, |control, cx| {
            control.hide_tip();
            cx.notify();
        })
    });
    // Where the control is, for the tip to stand by.
    let bounds_state = state.clone();
    element = element.child(
        canvas(
            move |bounds, _, cx| {
                bounds_state.update(cx, |control, _| control.tip.bounds = Some(bounds))
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    );
    match tip_layer(state, &tip, window, cx) {
        Some(layer) => element.child(layer),
        None => element,
    }
}

/// The tip over everything, while it shows or fades out.
fn tip_layer(
    state: &Entity<Control>,
    tip: &Tip,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let now = Instant::now();
    let (shown, hiding, bounds) = {
        let control = state.read(cx);
        (control.tip.shown, control.tip.hiding, control.tip.bounds)
    };
    let opacity = match (shown, hiding) {
        (Some((since, true)), _) => faded(since, now),
        (Some((_, false)), _) => 1.0,
        (None, Some(since)) => {
            let gone = faded(since, now);
            if gone >= 1.0 {
                state.update(cx, |control, _| control.tip.hiding = None);
                return None;
            }
            1.0 - gone
        }
        (None, None) => return None,
    };
    if opacity < 1.0 || hiding.is_some() {
        window.request_animation_frame();
    }
    let tip = tip.one_line();
    let tip_size = tip_size(&tip, window);
    let origin = tip_origin(bounds?, tip_size, window.viewport_size());
    Some(
        deferred(
            gpui::anchored()
                .position(origin)
                .child(tip_card(&tip, Some(f32::from(tip_size.width))).opacity(opacity)),
        )
        .with_priority(3)
        .into_any_element(),
    )
}

/// `child` with a tip: its name and key, shown as a control of the kit's
/// is (after a rest, or at once after another; fading). The toolbar's
/// buttons that are not of the kit, and rows that say more on a rest, use it.
pub fn tipped(id: impl Into<ElementId>, tip: Tip, child: impl IntoElement) -> Tipped {
    Tipped {
        id: id.into(),
        tip,
        child: child.into_any_element(),
    }
}

#[derive(IntoElement)]
pub struct Tipped {
    id: ElementId,
    tip: Tip,
    child: AnyElement,
}

impl RenderOnce for Tipped {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = control(&self.id, window, cx);
        let element = div()
            .id(self.id)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .child(self.child);
        hovered(element, &state, Some(self.tip), window, cx)
    }
}

/// What a control says when the pointer rests on it, as GPUI's own
/// `tooltip` shows it (after its own wait): its words, fading in.
pub struct Tooltip(pub SharedString);

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        fading_card(Tip::new(self.0.clone()))
    }
}

/// A tip with its key, as GPUI's own `tooltip` shows it: see [`tooltip`].
pub struct TipView(pub Tip);

impl Render for TipView {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        fading_card(self.0.clone())
    }
}

fn fading_card(tip: Tip) -> impl IntoElement {
    tip_card(&tip.one_line(), None).with_animation(
        "kit-tip",
        motion::animation(scene::FADE),
        |card, t| card.opacity(t),
    )
}

/// What to give an element's `tooltip` for `tip`: its words and its key.
pub fn tooltip(tip: Tip) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| TipView(tip.clone())).into()
}

// ---------------------------------------------------------------- keys on a control

/// `element`, a control of the kit keeping `state`: Tab reaches it (unless
/// `disabled`, not `focusable`, or folded away to nothing), a ring shows
/// just inside its edge while it has the keys, Space or Enter presses it,
/// and `on_click` runs on a press of the pointer or the keys. A press of
/// the pointer leaves the keys where they were. On a control `filled`
/// with the accent the ring goes round it outside ([`ring_around`]).
fn keyed(
    element: Stateful<Div>,
    state: &Entity<Control>,
    context: &'static str,
    focusable: bool,
    disabled: bool,
    corner: f32,
    filled: bool,
    window: &Window,
    cx: &App,
    on_click: Option<Handler>,
) -> Stateful<Div> {
    let focus = state.read(cx).focus.clone();
    let mut element = element
        .relative()
        .key_context(context)
        // The pointer leaves the keys where they were: the ring is the keyboard's.
        .capture_any_mouse_down(|_, window, _| window.prevent_default())
        .on_action(|_: &FocusNext, window, cx| tab_to(true, window, cx))
        .on_action(|_: &FocusPrevious, window, cx| tab_to(false, window, cx));
    if focusable && !disabled {
        element = element.track_focus(&focus);
        if focus.is_focused(window) {
            element = element.child(if filled {
                ring_around(corner)
            } else {
                ring(corner)
            });
        }
        // Folded away to nothing, Tab passes it over.
        let noted = focus.clone();
        element = element.child(
            canvas(
                move |bounds, window, _| note_shown(&noted, bounds, window),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
    }
    if disabled {
        return element;
    }
    let arm = state.clone();
    element = element.on_key_down(move |event: &KeyDownEvent, _, cx| {
        let keystroke = &event.keystroke;
        if matches!(keystroke.key.as_str(), "enter" | "space") && !keystroke.modifiers.modified() {
            arm.update(cx, |control, _| control.armed = true);
        }
    });
    if let Some(handler) = on_click {
        let armed = state.clone();
        element = element.on_click(move |event, window, cx| {
            // The keys press only what they went down on: an Enter that
            // chose in a menu does not press what has the keys after it.
            if event.is_keyboard()
                && !armed.update(cx, |control, _| std::mem::take(&mut control.armed))
            {
                return;
            }
            handler(event, window, cx)
        });
    }
    element
}

/// The ring round what has the keys: `FOCUS_RING` of the accent just
/// inside its edge, its corners `corner` round.
pub fn ring(corner: f32) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(px(corner))
        .border(px(layout::FOCUS_RING))
        .border_color(theme::accent())
}

/// The ring round what is filled with the accent, where [`ring`] would not
/// show: `FOCUS_RING` of the accent `FOCUS_RING_OUT` outside its edge,
/// round a control whose corners are `corner` round.
pub fn ring_around(corner: f32) -> Div {
    let out = -(layout::FOCUS_RING_OUT + layout::FOCUS_RING);
    div()
        .absolute()
        .top(px(out))
        .left(px(out))
        .right(px(out))
        .bottom(px(out))
        .rounded(px(corner - out))
        .border(px(layout::FOCUS_RING))
        .border_color(theme::accent())
}

// ---------------------------------------------------------------- buttons

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The one thing to do: the accent.
    Primary,
    /// An outline on the chrome.
    Secondary,
    /// Words alone until the pointer is over it.
    Ghost,
    /// What cannot be undone.
    Danger,
}

/// A button with words and an optional icon before them.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    icon: Option<Icon>,
    kind: Kind,
    disabled: bool,
    small: bool,
    focusable: bool,
    tooltip: Option<SharedString>,
    key: Option<SharedString>,
    reason: Option<SharedString>,
    on_click: Option<Handler>,
}

pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button {
        id: id.into(),
        label: label.into(),
        icon: None,
        kind: Kind::Secondary,
        disabled: false,
        small: false,
        focusable: true,
        tooltip: None,
        key: None,
        reason: None,
        on_click: None,
    }
}

impl Button {
    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    pub fn primary(self) -> Self {
        self.kind(Kind::Primary)
    }

    pub fn ghost(self) -> Self {
        self.kind(Kind::Ghost)
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    /// Whether Tab reaches it: it does when it has an `on_click` (one
    /// without, a menu's trigger takes the press, is no stop), unless a
    /// view keeps its own stops.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    /// What its tip says while it can be pressed.
    pub fn tooltip(mut self, words: impl Into<SharedString>) -> Self {
        self.tooltip = Some(words.into());
        self
    }

    /// Its key, in its tip after the words ("⌘↩", "Esc").
    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Why it cannot be pressed now: its tip while it is greyed.
    pub fn reason(mut self, words: impl Into<SharedString>) -> Self {
        self.reason = Some(words.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

/// A button's fill, its fill under the pointer, its ink and its ink under
/// the pointer, and its outline.
fn button_colours(kind: Kind) -> (Rgba, Rgba, Rgba, Rgba, Option<Rgba>) {
    match kind {
        Kind::Primary => (
            theme::accent(),
            theme::accent_hover(),
            theme::accent_text(),
            theme::accent_text(),
            None,
        ),
        Kind::Secondary => (
            theme::raised(),
            theme::hover2(),
            theme::ink(),
            theme::ink(),
            Some(theme::line2()),
        ),
        Kind::Ghost => (
            theme::with_alpha(theme::raised(), 0.0),
            theme::raised(),
            theme::ink2(),
            theme::ink(),
            None,
        ),
        Kind::Danger => (
            theme::raised(),
            theme::hover2(),
            theme::neg(),
            theme::neg(),
            Some(theme::line2()),
        ),
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = control(&self.id, window, cx);
        let (fill, hover, ink, hover_ink, outline) = button_colours(self.kind);
        // The hover fades over HOVER; a greyed button has none.
        let lit = if self.disabled {
            0.0
        } else {
            state.read(cx).hover.sample(window)
        };
        let ink = if self.disabled {
            theme::dis()
        } else {
            theme::mix(ink, hover_ink, lit)
        };
        let tall = if self.small { height::H24 } else { height::H28 };
        let mut button = theme::type_style(div(), text::CONTROL_STRONG)
            .id(self.id.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .h(px(tall))
            .px(px(if self.small { 8.0 } else { 12.0 }))
            .rounded(px(radius::CONTROL))
            .bg(theme::mix(fill, hover, lit))
            .text_color(ink)
            .whitespace_nowrap();
        if let Some(outline) = outline {
            button = button.border_1().border_color(outline);
        }
        if let Some(icon) = self.icon {
            button = button.child(icon.view(14.0, ink));
        }
        button = button.child(self.label);
        if !self.disabled {
            button = button.cursor_pointer();
        }
        let tip = tip_for(
            self.tooltip.as_ref(),
            self.key.as_ref(),
            self.disabled,
            self.reason.as_ref(),
        );
        // Only what its press does can the keys press.
        let focusable = self.focusable && self.on_click.is_some();
        let button = keyed(
            button,
            &state,
            CONTROL,
            focusable,
            self.disabled,
            radius::CONTROL,
            self.kind == Kind::Primary,
            window,
            cx,
            self.on_click,
        );
        hovered(button, &state, tip, window, cx)
    }
}

/// A square button holding an icon.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: Icon,
    size: f32,
    on: bool,
    disabled: bool,
    focusable: bool,
    color: Option<Rgba>,
    tooltip: Option<SharedString>,
    key: Option<SharedString>,
    reason: Option<SharedString>,
    on_click: Option<Handler>,
}

pub fn icon_button(id: impl Into<ElementId>, icon: Icon) -> IconButton {
    IconButton {
        id: id.into(),
        icon,
        size: height::H28,
        on: false,
        disabled: false,
        focusable: true,
        color: None,
        tooltip: None,
        key: None,
        reason: None,
        on_click: None,
    }
}

impl IconButton {
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Shows it held down: what it toggles is on.
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Whether Tab reaches it: it does when it has an `on_click` (one
    /// without, a menu's trigger takes the press, is no stop), unless a
    /// view keeps its own stops.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    pub fn color(mut self, color: Rgba) -> Self {
        self.color = Some(color);
        self
    }

    /// Its name, in its tip.
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Its key, in its tip after its name ("Space", "⌘,").
    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Why it cannot be pressed now: its tip while it is greyed.
    pub fn reason(mut self, words: impl Into<SharedString>) -> Self {
        self.reason = Some(words.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = control(&self.id, window, cx);
        let lit = if self.disabled {
            0.0
        } else {
            state.read(cx).hover.sample(window)
        };
        let ink = if self.disabled {
            theme::dis()
        } else if self.on {
            theme::ink()
        } else {
            self.color.unwrap_or(theme::ink2())
        };
        let (fill, hover) = if self.on {
            (theme::on(), theme::hover2())
        } else {
            (theme::with_alpha(theme::raised(), 0.0), theme::raised())
        };
        let mut button = div()
            .id(self.id.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(self.size))
            .rounded(px(radius::CONTROL))
            .bg(theme::mix(fill, hover, lit))
            .child(self.icon.view(16.0, ink));
        if !self.disabled {
            button = button.cursor_pointer();
        }
        let tip = tip_for(
            self.tooltip.as_ref(),
            self.key.as_ref(),
            self.disabled,
            self.reason.as_ref(),
        );
        // Only what its press does can the keys press.
        let focusable = self.focusable && self.on_click.is_some();
        let button = keyed(
            button,
            &state,
            CONTROL,
            focusable,
            self.disabled,
            radius::CONTROL,
            false,
            window,
            cx,
            self.on_click,
        );
        hovered(button, &state, tip, window, cx)
    }
}

// ---------------------------------------------------------------- switch

/// A switch. Pressed, its knob slides to the other end and its track takes
/// or loses the accent over `segment`; set before it shows, it is already
/// in place; under Reduce Motion it is there at once. Tab reaches it, and
/// Space presses it.
pub fn switch(
    id: impl Into<SharedString>,
    on: bool,
    toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Switch {
    Switch {
        id: id.into(),
        on,
        disabled: false,
        focusable: true,
        tooltip: None,
        toggle: Rc::new(toggle),
    }
}

#[derive(IntoElement)]
pub struct Switch {
    id: SharedString,
    on: bool,
    disabled: bool,
    focusable: bool,
    tooltip: Option<SharedString>,
    toggle: Rc<dyn Fn(bool, &mut Window, &mut App)>,
}

impl Switch {
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Whether Tab reaches it (it does unless a view keeps its own stops).
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    pub fn tooltip(mut self, words: impl Into<SharedString>) -> Self {
        self.tooltip = Some(words.into());
        self
    }
}

/// Where a switch's knob is: where it was set when it first showed, then
/// sliding to each new setting.
struct Knob {
    on: bool,
    at: Tween,
}

/// A switch drawn `on`: its knob slides there over `SEGMENT` if it was set
/// the other way (where it was first drawn, it is already in place).
fn knob_to(knob: &mut Knob, on: bool) {
    if knob.on != on {
        knob.on = on;
        knob.at.go(if on { 1.0 } else { 0.0 }, scene::SEGMENT);
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = ElementId::Name(self.id.clone());
        let state = control(&id, window, cx);
        let on = self.on;
        let knob = window.use_keyed_state(id.clone(), cx, |_, _| Knob {
            on,
            at: Tween::at(if on { 1.0 } else { 0.0 }),
        });
        let at = knob.update(cx, |knob, _| {
            knob_to(knob, on);
            knob.at.clone()
        });
        let at = at.sample(window);
        let track = theme::mix(theme::track(), theme::accent(), at);
        let body = div()
            .relative()
            .w(px(34.0))
            .h(px(20.0))
            .rounded_full()
            .bg(track)
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .left(px(2.0 + 14.0 * at))
                    .size(px(16.0))
                    .rounded_full()
                    .bg(theme::ink())
                    .shadow_sm(),
            );
        let toggle = self.toggle.clone();
        let element = div()
            .id(id)
            .flex_none()
            .rounded_full()
            .when(!self.disabled, |element| element.cursor_pointer())
            .when(self.disabled, |element| element.opacity(0.4))
            .child(body);
        let element = keyed(
            element,
            &state,
            CONTROL,
            self.focusable,
            self.disabled,
            10.0,
            // On, its track is the accent.
            on,
            window,
            cx,
            Some(Rc::new(
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| toggle(!on, window, cx),
            )),
        );
        let tip = self.tooltip.map(Tip::new);
        hovered(element, &state, tip, window, cx)
    }
}

// ---------------------------------------------------------------- segmented

/// A segmented control: one of a few, the chosen one filled. Choosing
/// another slides the fill to it over `SEGMENT` (at once under Reduce
/// Motion, and in place when it first shows). Tab reaches it, and the
/// arrows choose. Each segment can say more in a tip ([`Segmented::tips`])
/// and be greyed when it cannot be chosen ([`Segmented::disabled`]).
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: impl Into<SharedString>,
    options: &[(T, &'static str)],
    chosen: T,
    choose: impl Fn(T, &mut Window, &mut App) + 'static,
) -> Segmented<T> {
    Segmented {
        id: id.into(),
        options: options
            .iter()
            .map(|(value, words)| (*value, SharedString::from(*words)))
            .collect(),
        chosen,
        choose: Rc::new(choose),
        tips: Vec::new(),
        disabled: Vec::new(),
        focusable: true,
    }
}

pub struct Segmented<T> {
    id: SharedString,
    options: Vec<(T, SharedString)>,
    chosen: T,
    choose: Rc<dyn Fn(T, &mut Window, &mut App)>,
    tips: Vec<Option<SharedString>>,
    disabled: Vec<T>,
    focusable: bool,
}

impl<T: Copy + PartialEq + 'static> Segmented<T> {
    /// What each segment's tip says, in the options' order (Settings'
    /// permission segments carry their summaries).
    pub fn tips(mut self, tips: &[&str]) -> Self {
        self.tips = tips
            .iter()
            .map(|tip| (!tip.is_empty()).then(|| SharedString::from(tip.to_string())))
            .collect();
        self
    }

    /// Greys `value`'s segment: it cannot be chosen now.
    pub fn disabled(mut self, value: T) -> Self {
        self.disabled.push(value);
        self
    }

    /// Whether Tab reaches it (it does unless a view keeps its own stops).
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }
}

impl<T: Copy + PartialEq + 'static> IntoElement for Segmented<T> {
    type Element = Component<Self>;

    fn into_element(self) -> Self::Element {
        Component::new(self)
    }
}

/// The step from segment `from` to the next one that can be chosen,
/// forward or back; none past either end.
pub fn segment_step(enabled: &[bool], from: usize, forward: bool) -> Option<usize> {
    if forward {
        (from + 1..enabled.len()).find(|at| enabled[*at])
    } else {
        (0..from.min(enabled.len())).rev().find(|at| enabled[*at])
    }
}

/// Where a segmented control's fill is: on the segment chosen when it
/// last stood still, sliding from where it was to a newly chosen one.
struct Fill {
    chosen: Option<usize>,
    /// Its span (left, width) as it began to slide, and its slide.
    from: Option<(f32, f32)>,
    slide: Tween,
    /// Each segment's span, from the track's inner left, as laid out last.
    spans: Vec<(f32, f32)>,
}

impl Fill {
    /// Its span now, `t` of the way from where it began to the chosen one.
    fn span(&self, t: f32) -> Option<(f32, f32)> {
        let to = *self.spans.get(self.chosen?)?;
        let from = self.from.unwrap_or(to);
        Some((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t))
    }
}

/// Inside the track: its padding, the segments' height.
const TRACK_PAD: f32 = 2.0;
const SEGMENT_TALL: f32 = 24.0;

impl<T: Copy + PartialEq + 'static> RenderOnce for Segmented<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = ElementId::Name(self.id.clone());
        let state = control(&id, window, cx);
        let chosen = self
            .options
            .iter()
            .position(|(value, _)| *value == self.chosen);
        let enabled: Rc<Vec<bool>> = Rc::new(
            self.options
                .iter()
                .map(|(value, _)| !self.disabled.contains(value))
                .collect(),
        );
        let fill = window.use_keyed_state(id.clone(), cx, |_, _| Fill {
            chosen,
            from: None,
            slide: Tween::at(1.0),
            spans: Vec::new(),
        });
        // A new choice slides the fill from where it is now.
        let slide = fill.update(cx, |fill, _| {
            if fill.chosen != chosen {
                let t = fill.slide.value();
                fill.from = fill.span(t);
                fill.chosen = chosen;
                fill.slide = Tween::at(0.0);
                fill.slide.go(1.0, scene::SEGMENT);
            }
            fill.slide.clone()
        });
        let t = slide.sample(window);
        let span = if slide.running() {
            fill.read(cx).span(t)
        } else {
            None
        };
        let slid = span.is_some();
        let options = self.options.clone();
        let count = options.len();
        let spans_into = fill.clone();
        let mut track = div()
            .relative()
            .flex()
            .flex_none()
            .p(px(TRACK_PAD))
            .rounded(px(radius::CONTROL + 1.0))
            .bg(theme::raised())
            .on_children_prepainted(move |bounds, _, cx| {
                // The segments come after the fill when it slides.
                let segments = &bounds[bounds.len().saturating_sub(count)..];
                let Some(first) = segments.first() else {
                    return;
                };
                let left = f32::from(first.left());
                let spans = segments
                    .iter()
                    .map(|b| (f32::from(b.left()) - left, f32::from(b.size.width)))
                    .collect();
                spans_into.update(cx, |fill, _| fill.spans = spans);
            });
        if let Some((left, width)) = span {
            track = track.child(
                div()
                    .absolute()
                    .top(px(TRACK_PAD))
                    .left(px(TRACK_PAD + left))
                    .w(px(width))
                    .h(px(SEGMENT_TALL))
                    .rounded(px(radius::SMALL + 1.0))
                    .bg(theme::on()),
            );
        }
        for (index, (value, words)) in options.into_iter().enumerate() {
            let selected = Some(index) == chosen;
            let can = enabled[index];
            let choose = self.choose.clone();
            let segment_id = ElementId::Name(format!("{}-{index}", self.id).into());
            let segment_state = control(&segment_id, window, cx);
            let ink = if !can {
                theme::dis()
            } else if selected {
                theme::ink()
            } else {
                theme::ink2()
            };
            let segment = theme::type_style(
                div(),
                if selected {
                    text::CONTROL_STRONG
                } else {
                    text::CONTROL
                },
            )
            .id(segment_id)
            .relative()
            .px(px(10.0))
            .h(px(SEGMENT_TALL))
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .rounded(px(radius::SMALL + 1.0))
            .text_color(ink)
            .when(selected && !slid, |segment| segment.bg(theme::on()))
            .child(words.clone())
            // As wide as its words are when chosen, so choosing does not
            // change its width.
            .child(
                theme::type_style(div(), text::CONTROL_STRONG)
                    .h(px(0.0))
                    .overflow_hidden()
                    .child(words),
            )
            .when(can, |segment| {
                segment
                    .cursor_pointer()
                    .when(!selected, |segment| {
                        segment.hover(|style| style.text_color(theme::ink()))
                    })
                    .on_click(move |_, window, cx| choose(value, window, cx))
            });
            let tip = self.tips.get(index).cloned().flatten().map(Tip::new);
            track = track.child(hovered(segment, &segment_state, tip, window, cx));
        }
        let (forward, back) = (self.choose.clone(), self.choose.clone());
        let values: Rc<Vec<T>> = Rc::new(self.options.iter().map(|(value, _)| *value).collect());
        let (forward_values, back_values) = (values.clone(), values);
        let (forward_enabled, back_enabled) = (enabled.clone(), enabled);
        let from = chosen.unwrap_or(0);
        let element = div()
            .id(id)
            .flex_none()
            .rounded(px(radius::CONTROL + 1.0))
            .child(track)
            .on_action(move |_: &SegmentNext, window, cx| {
                let next = if chosen.is_none() {
                    forward_enabled.iter().position(|can| *can)
                } else {
                    segment_step(&forward_enabled, from, true)
                };
                if let Some(next) = next {
                    forward(forward_values[next], window, cx);
                }
            })
            .on_action(move |_: &SegmentPrevious, window, cx| {
                if let Some(next) = segment_step(&back_enabled, from, false) {
                    back(back_values[next], window, cx);
                }
            });
        keyed(
            element,
            &state,
            CONTROL_SEGMENTED,
            self.focusable,
            false,
            radius::CONTROL + 1.0,
            false,
            window,
            cx,
            None,
        )
    }
}

// ---------------------------------------------------------------- menu

/// One row of a [`Menu`]: its words, and what it does chosen; or a
/// submenu of rows that opens beside it.
#[derive(Clone)]
pub struct MenuItem {
    words: SharedString,
    icon: Option<Icon>,
    key: Option<SharedString>,
    checked: bool,
    disabled: bool,
    danger: bool,
    separated: bool,
    tip: Option<SharedString>,
    submenu: Vec<MenuItem>,
    run: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl MenuItem {
    pub fn new(words: impl Into<SharedString>) -> Self {
        Self {
            words: words.into(),
            icon: None,
            key: None,
            checked: false,
            disabled: false,
            danger: false,
            separated: false,
            tip: None,
            submenu: Vec::new(),
            run: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Its key, quiet at the row's end.
    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// A tick at its end: what is in use.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Greyed: it cannot be chosen now (its tip may say why).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// In brick: what cannot be undone.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// A line before it, between its group and the one before.
    pub fn separated(mut self) -> Self {
        self.separated = true;
        self
    }

    pub fn tip(mut self, words: impl Into<SharedString>) -> Self {
        self.tip = Some(words.into());
        self
    }

    /// Rows that open beside it (Right, or the pointer resting on it).
    pub fn submenu(mut self, items: Vec<MenuItem>) -> Self {
        self.submenu = items;
        self
    }

    /// What choosing it does, once the menu has gone and given the keys back.
    pub fn on_choose(mut self, run: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.run = Some(Rc::new(run));
        self
    }

    pub fn words(&self) -> &SharedString {
        &self.words
    }

    /// Whether it can be chosen (or opened, with a submenu).
    pub fn enabled(&self) -> bool {
        !self.disabled
    }
}

/// The next row of `enabled` from `from` (none lit: the first or last),
/// round past either end; none when no row can be chosen.
pub fn menu_step(enabled: &[bool], from: Option<usize>, down: bool) -> Option<usize> {
    let count = enabled.len();
    if count == 0 {
        return None;
    }
    let start = match (from, down) {
        (None, true) => count - 1,
        (None, false) => 0,
        (Some(at), _) => at.min(count - 1),
    };
    (1..=count)
        .map(|step| {
            if down {
                (start + step) % count
            } else {
                (start + count - step % count) % count
            }
        })
        .find(|at| enabled[*at])
}

/// The row a typed `letter` goes to: the next after `from`, round past the
/// end, whose words start with it (any case), that can be chosen.
pub fn menu_letter(
    words: &[&str],
    enabled: &[bool],
    from: Option<usize>,
    letter: char,
) -> Option<usize> {
    let count = words.len().min(enabled.len());
    if count == 0 {
        return None;
    }
    let letter = letter.to_lowercase().collect::<String>();
    let start = from.map_or(count - 1, |at| at.min(count - 1));
    (1..=count).map(|step| (start + step) % count).find(|at| {
        enabled[*at]
            && words[*at]
                .trim_start()
                .to_lowercase()
                .starts_with(letter.as_str())
    })
}

static MENU_SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A menu: rows of words to choose from, with keys, submenus and ticks. It
/// takes the keys as it opens and gives them back as it goes; the arrows
/// light a row at once, Enter or Space chooses it, a letter goes to the
/// next row starting with it, Right opens a submenu and Left or Esc closes
/// it, and Esc (or a press outside) closes the menu and does nothing else.
/// It fades in over `FADE`, rising `MENU_RISE`.
///
/// Whoever opens it holds the entity, places it (in a `deferred`
/// `anchored`, or an absolute box), and drops it on its [`DismissEvent`].
pub struct Menu {
    focus: FocusHandle,
    items: Vec<MenuItem>,
    header: Option<SharedString>,
    width: f32,
    /// The row lit at each level open: the menu's own, then each submenu's.
    path: Vec<Option<usize>>,
    /// What had the keys before it opened.
    back: Option<FocusHandle>,
    serial: usize,
    /// Where it and its submenus are laid out, to know a press outside;
    /// and where they were the frame before, to know which way a submenu
    /// has room to open.
    panels: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    laid: Vec<Bounds<Pixels>>,
    closed: bool,
    _letters: Subscription,
}

impl EventEmitter<DismissEvent> for Menu {}

impl Focusable for Menu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Menu {
    /// A menu of `items`, taking the keys from what has them.
    pub fn new(items: Vec<MenuItem>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let back = window.focused(cx);
        window.focus(&focus);
        let letters = {
            let this = cx.weak_entity();
            let focus = focus.clone();
            cx.intercept_keystrokes(move |event, window, cx| {
                if !focus.contains_focused(window, cx) {
                    return;
                }
                let keystroke = &event.keystroke;
                let modifiers = keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
                    return;
                }
                // The menu's own keys are bound in MENU.
                if matches!(
                    keystroke.key.as_str(),
                    "up" | "down"
                        | "left"
                        | "right"
                        | "home"
                        | "end"
                        | "enter"
                        | "space"
                        | "escape"
                        | "tab"
                ) {
                    return;
                }
                // A letter goes to its row; no other key typed while the menu
                // is open reaches the window (Backspace, a command's letter).
                let mut chars = keystroke.key.chars();
                if let (Some(letter), None) = (chars.next(), chars.next()) {
                    if letter.is_alphanumeric() {
                        let _ = this.update(cx, |menu, cx| menu.jump(letter, cx));
                    }
                }
                cx.stop_propagation();
            })
        };
        Self {
            focus,
            items,
            header: None,
            width: layout::MENU_LEAST,
            path: vec![None],
            back,
            serial: MENU_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            panels: Rc::default(),
            laid: Vec::new(),
            closed: false,
            _letters: letters,
        }
    }

    /// Quiet words over its rows (who chat runs on).
    pub fn header(mut self, words: impl Into<SharedString>) -> Self {
        self.header = Some(words.into());
        self
    }

    /// How wide it is (at least `MENU_LEAST`).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(layout::MENU_LEAST);
        self
    }

    /// Opened from the keys: its first row that can be chosen is lit.
    pub fn from_keys(mut self) -> Self {
        let enabled: Vec<bool> = self.items.iter().map(MenuItem::enabled).collect();
        self.path = vec![menu_step(&enabled, None, true)];
        self
    }

    /// The rows at `level`.
    fn rows(&self, level: usize) -> &[MenuItem] {
        let mut rows = self.items.as_slice();
        for lit in self.path.iter().take(level) {
            match lit.and_then(|at| rows.get(at)) {
                Some(item) => rows = item.submenu.as_slice(),
                None => return &[],
            }
        }
        rows
    }

    fn deepest(&self) -> usize {
        self.path.len() - 1
    }

    fn enabled(&self, level: usize) -> Vec<bool> {
        self.rows(level).iter().map(MenuItem::enabled).collect()
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        let level = self.deepest();
        let lit = menu_step(&self.enabled(level), self.path[level], down);
        self.path[level] = lit;
        cx.notify();
    }

    fn end(&mut self, last: bool, cx: &mut Context<Self>) {
        let level = self.deepest();
        self.path[level] = menu_step(&self.enabled(level), None, !last);
        cx.notify();
    }

    fn jump(&mut self, letter: char, cx: &mut Context<Self>) {
        let level = self.deepest();
        let rows = self.rows(level);
        let words: Vec<&str> = rows.iter().map(|item| item.words.as_ref()).collect();
        let enabled: Vec<bool> = rows.iter().map(MenuItem::enabled).collect();
        if let Some(at) = menu_letter(&words, &enabled, self.path[level], letter) {
            self.path[level] = Some(at);
            cx.notify();
        }
    }

    /// Opens the lit row's submenu, its first row lit (`keys`) or none.
    fn open_submenu(&mut self, keys: bool, cx: &mut Context<Self>) -> bool {
        let level = self.deepest();
        let Some(item) = self.path[level].and_then(|at| self.rows(level).get(at)) else {
            return false;
        };
        if item.submenu.is_empty() || item.disabled {
            return false;
        }
        let enabled: Vec<bool> = item.submenu.iter().map(MenuItem::enabled).collect();
        self.path.push(if keys {
            menu_step(&enabled, None, true)
        } else {
            None
        });
        cx.notify();
        true
    }

    /// Closes the deepest submenu; false when none is open.
    fn close_submenu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.path.len() <= 1 {
            return false;
        }
        self.path.pop();
        cx.notify();
        true
    }

    /// The pointer is over row `at` of `level`: it is lit, and a submenu
    /// of it opens.
    fn point_at(&mut self, level: usize, at: usize, cx: &mut Context<Self>) {
        if level >= self.path.len() {
            return;
        }
        if self.path[level] == Some(at) && self.path.len() > level + 1 {
            return;
        }
        self.path.truncate(level + 1);
        self.path[level] = Some(at);
        let opens = self
            .rows(level)
            .get(at)
            .is_some_and(|item| !item.submenu.is_empty() && !item.disabled);
        if opens {
            self.path.push(None);
        }
        cx.notify();
    }

    fn choose_lit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let level = self.deepest();
        if let Some(at) = self.path[level] {
            self.choose(level, at, true, window, cx);
        }
    }

    /// Row `at` of `level` chosen: its submenu opens, or the menu goes,
    /// the keys go back, and then what it does runs.
    fn choose(
        &mut self,
        level: usize,
        at: usize,
        keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.rows(level).get(at).cloned() else {
            return;
        };
        if item.disabled {
            return;
        }
        if !item.submenu.is_empty() {
            self.path.truncate(level + 1);
            self.path[level] = Some(at);
            self.open_submenu(keys, cx);
            return;
        }
        self.close(window, cx);
        if let Some(run) = item.run {
            run(window, cx);
        }
    }

    /// It goes: the keys go back to what had them, if it still has them.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        self.closed = true;
        if self.focus.contains_focused(window, cx) {
            match self.back.take() {
                Some(back) => window.focus(&back),
                None => window.blur(),
            }
        }
        cx.emit(DismissEvent);
        cx.notify();
    }

    /// The panel of `level`'s rows.
    fn panel(&self, level: usize, window: &Window, cx: &mut Context<Self>) -> Div {
        let rows = self.rows(level).to_vec();
        let lit = self.path.get(level).copied().flatten();
        let panels = self.panels.clone();
        let mut panel = menu_panel().flex().flex_col().w(px(self.width)).child(
            canvas(
                move |bounds, _, _| panels.borrow_mut().push(bounds),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        if level == 0 {
            if let Some(header) = self.header.clone() {
                panel = panel.child(div().px(px(10.0)).pt(px(4.0)).pb(px(6.0)).child(line(
                    header,
                    text::META,
                    theme::ink3(),
                )));
            }
        }
        let viewport = window.viewport_size();
        for (at, item) in rows.into_iter().enumerate() {
            if item.separated {
                panel = panel.child(menu_separator());
            }
            let is_lit = lit == Some(at);
            let open = is_lit && self.path.len() > level + 1 && !item.submenu.is_empty();
            let mut row = menu_item(
                ElementId::Name(format!("kit-menu-{}-{level}-{at}", self.serial).into()),
                item.words.clone(),
                item.icon,
                item.danger,
                item.disabled,
            )
            .relative()
            .when(is_lit, |row| row.bg(theme::popup_hover()))
            .when(
                item.checked || item.key.is_some() || !item.submenu.is_empty(),
                |row| row.child(div().flex_1()),
            )
            .when_some(item.key.clone(), |row, key| {
                row.child(label(key, text::MENU_QUIET, theme::ink3()))
            })
            .when(item.checked, |row| {
                row.child(Icon::Check.view(14.0, theme::ink2()))
            })
            .when(!item.submenu.is_empty(), |row| {
                row.child(Icon::ChevronRight.view(
                    14.0,
                    if item.disabled {
                        theme::dis()
                    } else {
                        theme::ink3()
                    },
                ))
            })
            .on_hover(cx.listener(move |menu, hovered: &bool, _, cx| {
                if *hovered {
                    menu.point_at(level, at, cx);
                }
            }));
            if let Some(tip) = item.tip.clone() {
                row = row.tooltip(tooltip(Tip::new(tip)));
            }
            if !item.disabled {
                row = row.on_click(cx.listener(move |menu, event: &ClickEvent, window, cx| {
                    menu.choose(level, at, event.is_keyboard(), window, cx)
                }));
            }
            if open {
                // Beside the row, to the right unless the window ends first.
                let room = self.laid.get(level).is_none_or(|bounds| {
                    f32::from(bounds.right()) + self.width <= f32::from(viewport.width)
                });
                let sub = self.panel(level + 1, window, cx);
                // Its first row level with this one; its edge on the menu's.
                let beside = self.width - 6.0;
                // Kept inside the window: one that would run past its foot
                // is raised until it fits.
                row = row.child(
                    div()
                        .absolute()
                        .top(px(-5.0))
                        .when(room, |place| place.left(px(beside)))
                        .when(!room, |place| place.right(px(beside)))
                        .child(
                            gpui::anchored()
                                .snap_to_window_with_margin(px(layout::TIP_EDGE))
                                .child(sub),
                        ),
                );
            }
            panel = panel.child(row);
        }
        panel
    }
}

impl Render for Menu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Where the panels were, kept; they are laid out again this frame.
        self.laid = std::mem::take(&mut *self.panels.borrow_mut());
        let panel = self.panel(0, window, cx);
        let panels = self.panels.clone();
        let rise = scene::MENU_RISE;
        panel
            .id(ElementId::Name(format!("kit-menu-{}", self.serial).into()))
            .relative()
            .track_focus(&self.focus)
            .key_context(MENU)
            .on_action(cx.listener(|menu, _: &MenuUp, _, cx| menu.step(false, cx)))
            .on_action(cx.listener(|menu, _: &MenuDown, _, cx| menu.step(true, cx)))
            .on_action(cx.listener(|menu, _: &MenuFirst, _, cx| menu.end(false, cx)))
            .on_action(cx.listener(|menu, _: &MenuLast, _, cx| menu.end(true, cx)))
            .on_action(cx.listener(|menu, _: &MenuChoose, window, cx| menu.choose_lit(window, cx)))
            .on_action(cx.listener(|menu, _: &MenuOpen, _, cx| {
                menu.open_submenu(true, cx);
            }))
            .on_action(cx.listener(|menu, _: &MenuBack, _, cx| {
                menu.close_submenu(cx);
            }))
            .on_action(cx.listener(|menu, _: &MenuClose, window, cx| {
                if !menu.close_submenu(cx) {
                    menu.close(window, cx);
                }
            }))
            .on_mouse_down_out(
                cx.listener(move |menu, event: &MouseDownEvent, window, cx| {
                    // A press on one of its submenus is not outside it.
                    let inside = panels
                        .borrow()
                        .iter()
                        .any(|bounds| bounds.contains(&event.position));
                    if !inside {
                        menu.close(window, cx);
                    }
                }),
            )
            .with_animation(
                ElementId::Name(format!("kit-menu-open-{}", self.serial).into()),
                motion::animation(scene::FADE),
                move |panel, t| panel.opacity(t).top(px(rise * (1.0 - t))),
            )
    }
}

/// What opens from a press: a panel of rows a step up from what it opens over.
pub fn menu_panel() -> Div {
    div()
        .p(px(4.0))
        .rounded(px(radius::CARD))
        .bg(theme::popup())
        .border_1()
        .border_color(theme::line2())
        .shadow_lg()
        .occlude()
}

/// One row of a menu: its icon (or the room for one), its words. What
/// cannot be undone is in brick; what cannot be done now is in the ink of
/// what cannot be pressed, and takes no press.
pub fn menu_item(
    id: impl Into<ElementId>,
    words: impl Into<SharedString>,
    icon: Option<Icon>,
    danger: bool,
    disabled: bool,
) -> Stateful<Div> {
    let ink = if disabled {
        theme::dis()
    } else if danger {
        theme::neg()
    } else {
        theme::ink()
    };
    let glyph = if disabled {
        theme::dis()
    } else if danger {
        theme::neg()
    } else {
        theme::ink2()
    };
    theme::type_style(div(), text::MENU_ITEM)
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.0))
        .h(px(layout::MENU_ROW))
        .px(px(10.0))
        .rounded(px(radius::CONTROL))
        .text_color(ink)
        .whitespace_nowrap()
        .when(!disabled, |item| item.cursor_pointer())
        .child(match icon {
            Some(icon) => icon.view(14.0, glyph).into_any_element(),
            None => div().size(px(14.0)).flex_none().into_any_element(),
        })
        .child(words.into())
}

/// A line between a menu's groups.
pub fn menu_separator() -> Div {
    div().my(px(4.0)).mx(px(6.0)).h(px(1.0)).bg(theme::line())
}

/// A press that toggles what it opens (a menu), taken before anything under
/// the pointer has it, so the menu it closes does not take it as a press
/// outside and open again.
pub fn trigger(
    id: impl Into<ElementId>,
    press: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .capture_any_mouse_down(move |event: &MouseDownEvent, window, cx| {
            if event.button == MouseButton::Left {
                cx.stop_propagation();
                press(event, window, cx);
            }
        })
}

// ---------------------------------------------------------------- scroll fades

/// Whether a list that scrolls with no bar shows its fades: at its top
/// while a row is cut by the top edge (any scroll, for painted content
/// with no rows), at its foot while there is more below. `scrolled` is how
/// far it is scrolled down, `most` the furthest it can be, `top` where the
/// list's top is, and `rows` each row's top and bottom as laid out
/// unscrolled.
pub fn scroll_fades(scrolled: f32, most: f32, top: f32, rows: &[(f32, f32)]) -> (bool, bool) {
    let edge = top + scrolled;
    let cut = scrolled > 0.0
        && (rows.is_empty()
            || rows
                .iter()
                .any(|(row_top, row_bottom)| *row_top < edge - 1.0 && *row_bottom > edge + 1.0));
    let more = scrolled < most - 1.0;
    (cut, more)
}

/// The fades over a list scrolled by `handle` that shows no bar: `TOP_FADE`
/// of `ground` at its top while a row is cut there, `BOTTOM_FADE` at its
/// foot while there is more ([`scroll_fades`]). Put it after the list, in
/// a box of the list's place (`relative`); it takes no press.
pub fn scroll_fade(handle: &ScrollHandle, ground: Rgba) -> impl IntoElement {
    let handle = handle.clone();
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = handle.bounds();
            let scrolled = -f32::from(handle.offset().y);
            let most = f32::from(handle.max_offset().height);
            let rows: Vec<(f32, f32)> = (0..handle.children_count())
                .filter_map(|at| handle.bounds_for_item(at))
                .map(|row| (f32::from(row.top()), f32::from(row.bottom())))
                .collect();
            let (cut, more) = scroll_fades(scrolled, most, f32::from(view.top()), &rows);
            let solid = ground;
            let clear = theme::with_alpha(ground, 0.0);
            if cut {
                let top = Bounds::new(
                    view.origin,
                    size(view.size.width, px(layout::TOP_FADE).min(view.size.height)),
                );
                window.paint_quad(gpui::fill(
                    top,
                    linear_gradient(
                        180.0,
                        linear_color_stop(solid, 0.0),
                        linear_color_stop(clear, 1.0),
                    ),
                ));
            }
            if more {
                let tall = px(layout::BOTTOM_FADE).min(view.size.height);
                let bottom = Bounds::new(
                    point(view.left(), view.bottom() - tall),
                    size(view.size.width, tall),
                );
                window.paint_quad(gpui::fill(
                    bottom,
                    linear_gradient(
                        180.0,
                        linear_color_stop(clear, 0.0),
                        linear_color_stop(solid, 1.0),
                    ),
                ));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

// ---------------------------------------------------------------- panels

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

// ---------------------------------------------------------------- small things

/// A small rounded chip of words.
pub fn chip(words: impl Into<SharedString>, color: Rgba) -> gpui::Div {
    theme::type_style(div(), text::META)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .h(px(20.0))
        .px(px(8.0))
        .rounded(px(10.0))
        .bg(theme::raised())
        .text_color(color)
        .child(words.into())
}

/// A filled dot, as a status says.
pub fn dot(color: Rgba, size: f32) -> gpui::Div {
    div().flex_none().size(px(size)).rounded_full().bg(color)
}

/// A thin progress bar, 0..1.
pub fn progress(amount: f64, color: Rgba) -> gpui::Div {
    let amount = amount.clamp(0.0, 1.0) as f32;
    div()
        .h(px(3.0))
        .w_full()
        .rounded_full()
        .bg(theme::track())
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(gpui::relative(amount)),
        )
}

/// A colour as GPUI's Hsla.
pub fn hsla(color: Rgba) -> Hsla {
    color.into()
}
