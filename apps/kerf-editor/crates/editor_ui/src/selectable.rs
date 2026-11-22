//! Words that can be chosen with the pointer, so a request or an answer
//! can be copied: a drag chooses from where it went down to where the
//! pointer is, a double-click a word, a triple-click a line, and a press
//! with Shift stretches what is chosen to it. What is chosen is washed in
//! the accent; ⌘C (Ctrl+C) copies it ([`Selections::copied`], which the
//! view that holds them answers). A press anywhere else lets it go.
//!
//! The pieces of one run (an answer's paragraphs, list items, headings and
//! code) are chosen as one: a drag goes on from one piece into the next,
//! and what is copied keeps the breaks between them. Links in the words (a
//! timecode Kerf wrote) still answer a press that does not drag.

use crate::input::{self, Unit};
use crate::theme;
use gpui::{
    fill, point, px, App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, GlobalElementId,
    Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, SharedString, StyledText, TextLayout, TextRun, Window,
};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// A place in a run: which piece, and the byte in its words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Spot {
    pub piece: usize,
    pub index: usize,
}

impl Spot {
    pub fn new(piece: usize, index: usize) -> Self {
        Self { piece, index }
    }
}

/// What is chosen: in which run, from where it is anchored to where the
/// pointer took it, and what the press chose first (a word, a line), kept
/// whole as a drag after it goes on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    pub run: SharedString,
    pub anchor: Spot,
    pub head: Spot,
    pub unit: Unit,
    pub first: (Spot, Spot),
}

impl Chosen {
    /// What a press at `at` chose first: by `unit` (a double-click a word),
    /// `range` of piece `at.piece`'s words.
    pub fn pressed(
        run: impl Into<SharedString>,
        piece: usize,
        range: Range<usize>,
        unit: Unit,
    ) -> Self {
        let first = (Spot::new(piece, range.start), Spot::new(piece, range.end));
        Self {
            run: run.into(),
            anchor: first.0,
            head: first.1,
            unit,
            first,
        }
    }

    /// From where it starts to where it ends, in reading order.
    pub fn span(&self) -> (Spot, Spot) {
        if self.head < self.anchor {
            (self.head, self.anchor)
        } else {
            (self.anchor, self.head)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The part of piece `piece`, `len` bytes long, it covers: None where it
    /// covers none of it.
    pub fn within(&self, piece: usize, len: usize) -> Option<Range<usize>> {
        within(self.span(), piece, len)
    }

    /// The pointer at `here`, the unit there spanning `here_unit`: by
    /// letters the head goes there; by words or lines, what the press chose
    /// first stays whole and the unit at the pointer joins it.
    pub fn drag_to(&mut self, here: Spot, here_unit: (Spot, Spot)) {
        if self.unit == Unit::Letter {
            self.head = here;
        } else {
            let (anchor, head) = stretch(self.first, here_unit);
            self.anchor = anchor;
            self.head = head;
        }
    }
}

/// The part of piece `piece`, `len` bytes long, that the span from `start`
/// to `end` (in reading order) covers; None where it covers none of it.
pub fn within((start, end): (Spot, Spot), piece: usize, len: usize) -> Option<Range<usize>> {
    if piece < start.piece || piece > end.piece {
        return None;
    }
    let from = if piece == start.piece {
        start.index.min(len)
    } else {
        0
    };
    let to = if piece == end.piece {
        end.index.min(len)
    } else {
        len
    };
    (to > from).then_some(from..to)
}

/// A drag from what a press chose first (`first`, a word or a line) to the
/// unit at the pointer (`here`): where it is anchored and its head. Going
/// back, it is anchored at the first's end and reaches the start of the
/// unit there; going on, from the first's start to the end of the unit.
pub fn stretch(first: (Spot, Spot), here: (Spot, Spot)) -> (Spot, Spot) {
    if here.0 < first.0 {
        (first.1, here.0)
    } else {
        (first.0, here.1.max(first.1))
    }
}

/// The words copied of a run's pieces (each its number, its words, and what
/// stands between it and the piece before it) from `start` to `end`.
pub fn copied(pieces: &[(usize, &str, &str)], (start, end): (Spot, Spot)) -> String {
    let mut words = String::new();
    let mut any = false;
    let mut ordered: Vec<&(usize, &str, &str)> = pieces.iter().collect();
    ordered.sort_by_key(|(piece, _, _)| *piece);
    for (piece, text, before) in ordered {
        let Some(range) = within((start, end), *piece, text.len()) else {
            continue;
        };
        let Some(part) = text.get(range) else {
            continue;
        };
        if any {
            words.push_str(before);
        }
        words.push_str(part);
        any = true;
    }
    words
}

/// A piece laid out this frame: its run, number, place, what of it takes
/// the pointer (its place as far as what holds it shows it, under nothing
/// drawn over it), words, and what stands between it and the piece before
/// it when they are copied.
struct Laid {
    run: SharedString,
    piece: usize,
    bounds: Bounds<Pixels>,
    hitbox: Hitbox,
    words: SharedString,
    before: &'static str,
}

/// What is chosen in a view's words, and where its pieces were laid out:
/// one for the view, shared by its pieces ([`selectable`]).
#[derive(Default)]
pub struct Selections {
    chosen: Option<Chosen>,
    dragging: bool,
    /// Where a press went down, for one that does not drag to be a link's.
    pressed: Option<(SharedString, Spot)>,
    laid: Vec<Laid>,
    /// What a right-click on words opens: the view's menu of them (Copy,
    /// Select All), at the pointer, for the run pressed.
    on_menu: Option<MenuOpener>,
}

/// Opens the words' menu for a run, at a place in the window.
pub type MenuOpener = Rc<dyn Fn(SharedString, gpui::Point<Pixels>, &mut Window, &mut App)>;

/// The selections a view's pieces share.
pub type Shared = Rc<RefCell<Selections>>;

impl Selections {
    pub fn shared() -> Shared {
        Rc::new(RefCell::new(Self::default()))
    }

    /// What a right-click on its words opens ([`MenuOpener`]).
    pub fn on_menu(&mut self, open: MenuOpener) {
        self.on_menu = Some(open);
    }

    /// Chooses all the words of `run` as they are laid out now (Select
    /// All on its menu); whether there were any.
    pub fn choose_all(&mut self, run: &SharedString) -> bool {
        let mut pieces = self.laid.iter().filter(|laid| &laid.run == run);
        let Some(first) = pieces.next() else {
            return false;
        };
        let (mut low, mut high) = (first, first);
        for laid in pieces {
            if laid.piece < low.piece {
                low = laid;
            }
            if laid.piece > high.piece {
                high = laid;
            }
        }
        let span = (
            Spot::new(low.piece, 0),
            Spot::new(high.piece, high.words.len()),
        );
        if span.0 == span.1 {
            return false;
        }
        self.chosen = Some(Chosen {
            run: run.clone(),
            anchor: span.0,
            head: span.1,
            unit: Unit::Letter,
            first: span,
        });
        self.dragging = false;
        self.pressed = None;
        true
    }

    /// The view draws again: its pieces say again where they are.
    pub fn begin_frame(&mut self) {
        self.laid.clear();
    }

    pub fn chosen(&self) -> Option<&Chosen> {
        self.chosen.as_ref().filter(|chosen| !chosen.is_empty())
    }

    /// Lets go of what is chosen; whether anything was.
    pub fn clear(&mut self) -> bool {
        self.dragging = false;
        self.chosen.take().is_some()
    }

    /// The words chosen, as they are copied: None with nothing chosen.
    pub fn copied(&self) -> Option<String> {
        let chosen = self.chosen()?;
        let pieces: Vec<(usize, &str, &str)> = self
            .laid
            .iter()
            .filter(|laid| laid.run == chosen.run)
            .map(|laid| (laid.piece, laid.words.as_ref(), laid.before))
            .collect();
        let words = copied(&pieces, chosen.span());
        (!words.is_empty()).then_some(words)
    }

    /// Whether the pointer is on any piece laid out where it shows: not on
    /// a part a scrolled list cuts off (under the composer, the toolbar),
    /// nor under what is drawn over it and takes the pointer (a panel over
    /// the window, a menu).
    fn on_a_piece(&self, window: &Window) -> bool {
        self.laid.iter().any(|laid| laid.hitbox.is_hovered(window))
    }

    /// The piece of `run` the pointer at `y` reaches during a drag: the
    /// last whose top is above it, else the first.
    fn reached(&self, run: &SharedString, y: Pixels) -> Option<usize> {
        let mut pieces: Vec<&Laid> = self.laid.iter().filter(|laid| &laid.run == run).collect();
        pieces.sort_by_key(|laid| laid.piece);
        pieces
            .iter()
            .rev()
            .find(|laid| laid.bounds.top() <= y)
            .or_else(|| pieces.first())
            .map(|laid| laid.piece)
    }
}

/// A piece of words that can be chosen: piece `piece` of run `run` in the
/// view whose selections are `shared`. Set in the type style of what holds
/// it, unless [`Selectable::runs`] says otherwise.
pub fn selectable(
    shared: &Shared,
    id: impl Into<ElementId>,
    run: impl Into<SharedString>,
    piece: usize,
    words: impl Into<SharedString>,
) -> Selectable {
    let words = words.into();
    Selectable {
        id: id.into(),
        run: run.into(),
        piece,
        before: "\n\n",
        text: StyledText::new(words.clone()),
        words,
        links: Vec::new(),
        on_link: None,
        shared: shared.clone(),
    }
}

/// What a press on one of a piece's links does, given the link's number.
type LinkHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub struct Selectable {
    id: ElementId,
    run: SharedString,
    piece: usize,
    before: &'static str,
    words: SharedString,
    text: StyledText,
    links: Vec<Range<usize>>,
    on_link: Option<LinkHandler>,
    shared: Shared,
}

impl Selectable {
    /// Its words set in `runs` (their lengths add up to the words').
    pub fn runs(mut self, runs: Vec<TextRun>) -> Self {
        self.text = StyledText::new(self.words.clone()).with_runs(runs);
        self
    }

    /// What stands between it and the piece before it when both are
    /// copied: a blank line unless said (a list's next item, a line).
    pub fn before(mut self, before: &'static str) -> Self {
        self.before = before;
        self
    }

    /// Links in its words: a press that goes down and up in one of
    /// `ranges` without dragging runs `on_link` with its number.
    pub fn links(
        mut self,
        ranges: Vec<Range<usize>>,
        on_link: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.links = ranges;
        self.on_link = Some(Rc::new(on_link));
        self
    }
}

/// The link of `links` at byte `index`, if one is.
fn link_at(links: &[Range<usize>], index: usize) -> Option<usize> {
    links.iter().position(|range| range.contains(&index))
}

/// Where a press or the pointer is in the words: the byte nearest it.
fn index_at(layout: &TextLayout, position: gpui::Point<Pixels>) -> usize {
    match layout.index_for_position(position) {
        Ok(index) | Err(index) => index,
    }
}

impl IntoElement for Selectable {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Selectable {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.text
            .prepaint(None, inspector_id, bounds, state, window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        self.shared.borrow_mut().laid.push(Laid {
            run: self.run.clone(),
            piece: self.piece,
            bounds,
            hitbox: hitbox.clone(),
            words: self.words.clone(),
            before: self.before,
        });
        hitbox
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.text.layout().clone();
        // Over a link the hand, else the text cursor.
        let over_link = hitbox.is_hovered(window)
            && link_at(&self.links, index_at(&layout, window.mouse_position())).is_some()
            && layout.index_for_position(window.mouse_position()).is_ok();
        window.set_cursor_style(
            if over_link {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            },
            hitbox,
        );
        // What is chosen of it, washed in the accent under the words.
        let chosen = self
            .shared
            .borrow()
            .chosen()
            .filter(|chosen| chosen.run == self.run)
            .and_then(|chosen| chosen.within(self.piece, self.words.len()));
        if let Some(range) = chosen {
            paint_chosen(&layout, bounds, range, window);
        }
        self.text
            .paint(None, inspector_id, bounds, state, &mut (), window, cx);
        self.listen(&layout, hitbox, window);
    }
}

impl Selectable {
    fn listen(&self, layout: &TextLayout, hitbox: &Hitbox, window: &mut Window) {
        // A press anywhere but on words where they show lets what is
        // chosen go: on the composer over words scrolled under it too.
        let shared = self.shared.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            let mut selections = shared.borrow_mut();
            if !selections.on_a_piece(window) && selections.clear() {
                window.refresh();
            }
        });
        // A press on it: a place, a word, a line; with Shift, what is
        // chosen stretches to it.
        let (shared, run, piece, words) = (
            self.shared.clone(),
            self.run.clone(),
            self.piece,
            self.words.clone(),
        );
        let (pressed_layout, pressed_hitbox) = (layout.clone(), hitbox.clone());
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !pressed_hitbox.is_hovered(window)
            {
                return;
            }
            let index = index_at(&pressed_layout, event.position);
            let mut selections = shared.borrow_mut();
            let here = Spot::new(piece, index);
            let stretching = event.modifiers.shift
                && selections
                    .chosen
                    .as_ref()
                    .is_some_and(|chosen| chosen.run == run);
            if stretching {
                if let Some(chosen) = selections.chosen.as_mut() {
                    chosen.head = here;
                    chosen.unit = Unit::Letter;
                }
                selections.pressed = None;
            } else {
                let unit = Unit::of_clicks(event.click_count);
                let range = input::unit_at(&words, index, unit);
                selections.chosen = Some(Chosen::pressed(run.clone(), piece, range, unit));
                selections.pressed = (unit == Unit::Letter).then(|| (run.clone(), here));
            }
            selections.dragging = true;
            window.refresh();
        });
        // A right-click on it: the view's menu of its words, at the
        // pointer; what is chosen stays chosen.
        let (shared, run) = (self.shared.clone(), self.run.clone());
        let menu_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Right
                || !menu_hitbox.is_hovered(window)
            {
                return;
            }
            let open = shared.borrow().on_menu.clone();
            if let Some(open) = open {
                cx.stop_propagation();
                open(run.clone(), event.position, window, cx);
            }
        });
        // A drag: the piece the pointer reaches takes the head there.
        let (shared, run, piece, words) = (
            self.shared.clone(),
            self.run.clone(),
            self.piece,
            self.words.clone(),
        );
        let moved_layout = layout.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let mut selections = shared.borrow_mut();
            if !selections.dragging {
                return;
            }
            if event.pressed_button != Some(MouseButton::Left) {
                selections.dragging = false;
                return;
            }
            let ours = selections
                .chosen
                .as_ref()
                .is_some_and(|chosen| chosen.run == run);
            if !ours || selections.reached(&run, event.position.y) != Some(piece) {
                return;
            }
            let bounds = moved_layout.bounds();
            let index = if event.position.y > bounds.bottom() {
                words.len()
            } else if event.position.y < bounds.top() {
                0
            } else {
                index_at(&moved_layout, event.position)
            };
            let unit = selections
                .chosen
                .as_ref()
                .map_or(Unit::Letter, |chosen| chosen.unit);
            let here_unit = input::unit_at(&words, index, unit);
            let here = Spot::new(piece, index);
            let unit_span = (
                Spot::new(piece, here_unit.start),
                Spot::new(piece, here_unit.end),
            );
            if let Some(chosen) = selections.chosen.as_mut() {
                let was = chosen.clone();
                chosen.drag_to(here, unit_span);
                if *chosen != was {
                    window.refresh();
                }
            }
        });
        // Let go: a press that did not drag is a place, which chooses
        // nothing, or a link's.
        let (shared, run, piece) = (self.shared.clone(), self.run.clone(), self.piece);
        let (up_layout, up_hitbox) = (layout.clone(), hitbox.clone());
        let links = self.links.clone();
        let on_link = self.on_link.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            let link = {
                let mut selections = shared.borrow_mut();
                selections.dragging = false;
                if selections.chosen.as_ref().is_some_and(Chosen::is_empty) {
                    selections.chosen = None;
                }
                let pressed = selections
                    .pressed
                    .clone()
                    .filter(|(at_run, at)| *at_run == run && at.piece == piece);
                if pressed.is_none() {
                    return;
                }
                selections.pressed = None;
                let still = selections.chosen.is_none();
                pressed.and_then(|(_, at)| {
                    let up = up_hitbox
                        .is_hovered(window)
                        .then(|| up_layout.index_for_position(event.position).ok())
                        .flatten()?;
                    let link = link_at(&links, at.index)?;
                    (still && link_at(&links, up) == Some(link)).then_some(link)
                })
            };
            window.refresh();
            if let (Some(link), Some(on_link)) = (link, on_link.as_ref()) {
                on_link(link, window, cx);
            }
        });
    }
}

/// The accent's wash under `range` of the words `layout` laid out in
/// `bounds`: the first line from its start to the edge, the lines between
/// whole, the last up to its end.
fn paint_chosen(
    layout: &TextLayout,
    bounds: Bounds<Pixels>,
    range: Range<usize>,
    window: &mut Window,
) {
    let (Some(start), Some(end)) = (
        layout.position_for_index(range.start),
        layout.position_for_index(range.end),
    ) else {
        return;
    };
    let line = layout.line_height();
    let wash = theme::with_alpha(theme::accent(), CHOSEN_WASH);
    let right = bounds.right();
    let left = bounds.left();
    if start.y == end.y {
        window.paint_quad(fill(
            Bounds::from_corners(start, point(end.x, end.y + line)),
            wash,
        ));
        return;
    }
    window.paint_quad(fill(
        Bounds::from_corners(start, point(right, start.y + line)),
        wash,
    ));
    if end.y > start.y + line {
        window.paint_quad(fill(
            Bounds::from_corners(point(left, start.y + line), point(right, end.y)),
            wash,
        ));
    }
    window.paint_quad(fill(
        Bounds::from_corners(
            point(left, end.y),
            point(end.x.max(left + px(1.0)), end.y + line),
        ),
        wash,
    ));
}

/// How strong the accent is under what is chosen, as a field's selection.
const CHOSEN_WASH: f32 = 0.45;
