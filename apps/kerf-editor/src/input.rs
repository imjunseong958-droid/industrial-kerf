//! A text field: one line, or several that wrap (the composer). It takes
//! the platform's input method, so Hangul and other composed text arrive
//! as they are written. Enter submits; Shift+Enter starts a new line where
//! the field has several. A field of several lines given a most height
//! grows with its lines up to it, then scrolls, keeping the cursor in view.

use crate::theme::{self, TypeStyle};
use gpui::{
    actions, div, fill, point, prelude::*, px, size, App, AvailableSpace, Bounds, ClipboardItem,
    ContentMask, Context, CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Rgba, ScrollWheelEvent,
    SharedString, Style, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Submit,
        Newline,
        WordLeft,
        WordRight,
        DeleteWordLeft,
        Cancel,
    ]
);

/// The key context of a field. The window's keymap binds each plain key
/// of its commands (Esc, Backspace, Left, a letter) to nothing here, so
/// typing it in a field never runs the command.
pub const CONTEXT: &str = "TextInput";

/// The key context the field's own keys are bound in: one inside
/// [`CONTEXT`], so they come before what the keymap unbinds there, and
/// Backspace, the arrows and Esc still do what they do in a field.
pub const KEYS: &str = "TextInputKeys";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(key_bindings());
}

/// The field's keys, as [`bind_keys`] binds them.
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(KEYS);
    vec![
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("secondary-v", Paste, context),
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-x", Cut, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("enter", Submit, context),
        KeyBinding::new("shift-enter", Newline, context),
        KeyBinding::new("alt-left", WordLeft, context),
        KeyBinding::new("alt-right", WordRight, context),
        KeyBinding::new("ctrl-left", WordLeft, context),
        KeyBinding::new("ctrl-right", WordRight, context),
        KeyBinding::new("alt-backspace", DeleteWordLeft, context),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, context),
        KeyBinding::new("escape", Cancel, context),
        // In the composer, what Kerf does without asking goes round.
        KeyBinding::new(
            "shift-tab",
            crate::chat::CyclePermissions,
            Some(format!("{} > {KEYS}", crate::chat::COMPOSER_CONTEXT).as_str()),
        ),
    ]
}

/// What the field says to whoever holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Changed,
    Submit,
    Cancel,
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    multiline: bool,
    style: TypeStyle,
    reading: bool,
    secret: bool,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    layout: Option<Layout>,
    is_selecting: bool,
    /// The most a field of several lines grows to before it scrolls.
    max_height: Option<Pixels>,
    /// How far its text is scrolled up, past its top.
    scroll: Pixels,
    /// The cursor moved or the text changed: the next drawing scrolls to
    /// keep the cursor in view. A wheel scrolls away from it.
    reveal: bool,
}

/// Where the text was drawn last, to find the index under the pointer.
#[derive(Clone)]
struct Layout {
    lines: Vec<WrappedLine>,
    line_height: Pixels,
    /// The field's own bounds: what of the text shows.
    bounds: Bounds<Pixels>,
    /// How far the text was scrolled up in them.
    scroll: Pixels,
    /// The height of all its lines.
    height: Pixels,
}

impl EventEmitter<InputEvent> for TextInput {}

impl TextInput {
    pub fn new(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: placeholder.into(),
            multiline: false,
            style: theme::text::BODY,
            reading: false,
            secret: false,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            layout: None,
            is_selecting: false,
            max_height: None,
            scroll: Pixels::ZERO,
            reveal: false,
        }
    }

    /// Grows with its lines up to `height`, then scrolls (a field of
    /// several lines).
    pub fn max_height(mut self, height: Pixels) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Several lines, wrapped, set for reading: the composer.
    pub fn multiline(mut self) -> Self {
        self.multiline = true;
        self.reading = true;
        self
    }

    /// Shows dots in place of what is typed: a key.
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    pub fn with_style(mut self, style: TypeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = text.into();
        if !self.multiline {
            self.content = self.content.replace('\n', " ");
        }
        let end = self.content.len();
        self.selected_range = end..end;
        self.marked_range = None;
        self.reveal = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        // One line has nowhere to go up to: what holds it may (a list under it).
        if !self.multiline {
            cx.propagate();
            return;
        }
        self.move_vertically(-1.0, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if !self.multiline {
            cx.propagate();
            return;
        }
        self.move_vertically(1.0, cx);
    }

    fn move_vertically(&mut self, direction: f32, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        let Some(position) = self.position_for_index(&layout, self.cursor_offset()) else {
            return;
        };
        let target = point(
            position.x,
            position.y + layout.line_height * direction + layout.line_height / 2.0,
        );
        let offset = if target.y < Pixels::ZERO {
            0
        } else {
            self.index_for_local(&layout, target)
        };
        self.move_to(offset, cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word(self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word(self.cursor_offset()), cx);
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.choose_all(cx)
    }

    /// Chooses all the text, as Select All does, without the field needing
    /// to be in the frame drawn.
    pub fn choose_all(&mut self, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.content[..self.cursor_offset()]
            .rfind('\n')
            .map_or(0, |at| at + 1);
        self.move_to(start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let end = self.content[cursor..]
            .find('\n')
            .map_or(self.content.len(), |at| cursor + at);
        self.move_to(end, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        // While text is being composed, Enter belongs to the input method.
        if self.marked_range.is_some() {
            return;
        }
        cx.emit(InputEvent::Submit);
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.replace_text_in_range(None, "\n", window, cx);
        } else {
            cx.emit(InputEvent::Submit);
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.is_selecting = true;
        let offset = self.index_for_mouse_position(event.position);
        if event.click_count >= 2 {
            self.selected_range = self.previous_word(offset)..self.next_word(offset);
            cx.notify();
            return;
        }
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    /// A wheel over a field that scrolls moves its text, and goes no
    /// further while it can; at either end it goes on to what holds it.
    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let most = (layout.height - layout.bounds.size.height).max(Pixels::ZERO);
        if most <= Pixels::ZERO {
            return;
        }
        let delta = event.delta.pixel_delta(layout.line_height).y;
        let scroll = (self.scroll - delta).clamp(Pixels::ZERO, most);
        if scroll == self.scroll {
            return;
        }
        self.scroll = scroll;
        self.reveal = false;
        cx.stop_propagation();
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = if self.multiline {
                text
            } else {
                text.replace('\n', " ")
            };
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.secret {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.secret {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.reveal = true;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.reveal = true;
        cx.notify()
    }

    /// What is drawn: the text, or dots for a secret.
    fn shown(&self) -> String {
        if self.secret {
            "•".repeat(self.content.chars().count())
        } else {
            self.content.clone()
        }
    }

    /// The byte offset in what is drawn of `offset` in the content.
    fn shown_offset(&self, offset: usize) -> usize {
        if self.secret {
            self.content[..offset].chars().count() * '•'.len_utf8()
        } else {
            offset
        }
    }

    fn content_offset(&self, shown: usize) -> usize {
        if self.secret {
            let chars = shown / '•'.len_utf8();
            self.content
                .char_indices()
                .nth(chars)
                .map_or(self.content.len(), |(at, _)| at)
        } else {
            shown.min(self.content.len())
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = self.layout.clone() else {
            return 0;
        };
        if self.content.is_empty() {
            return 0;
        }
        let local = position - layout.bounds.origin + point(Pixels::ZERO, layout.scroll);
        if local.y < Pixels::ZERO {
            return 0;
        }
        self.index_for_local(&layout, local)
    }

    /// The content offset nearest a point within the drawn text.
    fn index_for_local(&self, layout: &Layout, local: Point<Pixels>) -> usize {
        let mut top = Pixels::ZERO;
        let mut start = 0;
        for line in &layout.lines {
            let height = line.size(layout.line_height).height;
            if local.y < top + height {
                let within = point(local.x, local.y - top);
                let index = match line.closest_index_for_position(within, layout.line_height) {
                    Ok(index) | Err(index) => index,
                };
                return self.content_offset(start + index);
            }
            top += height;
            start += line.len() + 1;
        }
        self.content.len()
    }

    /// Where `offset` is drawn, from the text's top left.
    fn position_for_index(&self, layout: &Layout, offset: usize) -> Option<Point<Pixels>> {
        let shown = self.shown_offset(offset);
        let mut top = Pixels::ZERO;
        let mut start = 0;
        for line in &layout.lines {
            let end = start + line.len();
            if shown <= end {
                let at = line.position_for_index(shown - start, layout.line_height)?;
                return Some(point(at.x, top + at.y));
            }
            top += line.size(layout.line_height).height;
            start = end + 1;
        }
        None
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    fn previous_word(&self, offset: usize) -> usize {
        self.content
            .unicode_word_indices()
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_word(&self, offset: usize) -> usize {
        self.content
            .unicode_word_indices()
            .find_map(|(idx, word)| (idx + word.len() > offset).then_some(idx + word.len()))
            .unwrap_or(self.content.len())
    }

    fn font(&self) -> gpui::Font {
        if self.reading {
            theme::prose_font(self.style.weight)
        } else if self.style.mono {
            theme::mono_font(self.style.weight)
        } else {
            theme::ui_font(self.style.weight)
        }
    }

    fn font_size(&self) -> Pixels {
        if self.reading {
            px(theme::text::READING.px + theme::prose_step())
        } else {
            px(self.style.px)
        }
    }

    fn line_height(&self) -> Pixels {
        if self.reading {
            px(theme::text::READING.line + theme::prose_step() * 1.5)
        } else {
            px(self.style.line)
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_text = if self.multiline {
            new_text.to_owned()
        } else {
            new_text.replace('\n', " ")
        };
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let range = range.start.min(self.content.len())..range.end.min(self.content.len());
        self.content =
            self.content[0..range.start].to_owned() + &new_text + &self.content[range.end..];
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.selection_reversed = false;
        self.marked_range.take();
        self.reveal = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let range = range.start.min(self.content.len())..range.end.min(self.content.len());
        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| {
                let within = new_text
                    .char_indices()
                    .scan(0usize, |utf16, (at, ch)| {
                        let here = *utf16;
                        *utf16 += ch.len_utf16();
                        Some((here, at))
                    })
                    .collect::<Vec<_>>();
                let to_utf8 = |utf16: usize| {
                    within
                        .iter()
                        .find(|(index, _)| *index >= utf16)
                        .map_or(new_text.len(), |(_, at)| *at)
                };
                range.start + to_utf8(range_utf16.start)..range.start + to_utf8(range_utf16.end)
            })
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.reveal = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.clone()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = self.position_for_index(&layout, range.start)?;
        let end = self.position_for_index(&layout, range.end).unwrap_or(start);
        let origin = bounds.origin - point(Pixels::ZERO, layout.scroll);
        Some(Bounds::from_corners(
            origin + start,
            origin + point(end.x.max(start.x + px(1.0)), end.y + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.layout.clone()?;
        let local = point - layout.bounds.origin + gpui::point(Pixels::ZERO, layout.scroll);
        Some(self.offset_to_utf16(self.index_for_local(&layout, local)))
    }
}

/// Draws the field's text, its selection and its cursor.
struct TextElement {
    input: Entity<TextInput>,
    color: Rgba,
}

struct Prepaint {
    lines: Vec<WrappedLine>,
    quads: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    /// How far the text is scrolled up, and the height of all its lines.
    scroll: Pixels,
    height: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Where a field `visible` tall scrolled `scroll` up goes to show a line
/// of `line_height` whose top is `top` in its text: no further than it must.
pub fn scroll_to_show(scroll: Pixels, top: Pixels, line_height: Pixels, visible: Pixels) -> Pixels {
    if top < scroll {
        top
    } else if top + line_height > scroll + visible {
        (top + line_height - visible).max(Pixels::ZERO)
    } else {
        scroll
    }
}

/// The runs of `text` in `font`, the part being composed underlined.
fn runs(text: &str, font: gpui::Font, color: Rgba, marked: Option<&Range<usize>>) -> Vec<TextRun> {
    let run = TextRun {
        len: text.len(),
        font,
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let Some(marked) = marked.filter(|marked| marked.end <= text.len()) else {
        return vec![run];
    };
    [
        TextRun {
            len: marked.start,
            ..run.clone()
        },
        TextRun {
            len: marked.end - marked.start,
            underline: Some(UnderlineStyle {
                color: Some(run.color),
                thickness: px(1.0),
                wavy: false,
            }),
            ..run.clone()
        },
        TextRun {
            len: text.len() - marked.end,
            ..run
        },
    ]
    .into_iter()
    .filter(|run| run.len > 0)
    .collect()
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        let line_height = input.line_height();
        if !input.multiline {
            style.size.height = line_height.into();
            return (window.request_layout(style, [], cx), ());
        }
        let text = if input.content.is_empty() {
            input.placeholder.to_string()
        } else {
            input.shown()
        };
        let font = input.font();
        let font_size = input.font_size();
        let most = input.max_height.map(|most| most.max(line_height));
        let layout = window.request_measured_layout(style, move |known, available, window, _cx| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let runs = runs(&text, font.clone(), theme::ink(), None);
            let lines = window
                .text_system()
                .shape_text(text.clone().into(), font_size, &runs, width, None)
                .map(|lines| {
                    lines.into_iter().fold(Pixels::ZERO, |sum, line| {
                        sum + line.size(line_height).height
                    })
                })
                .unwrap_or(line_height);
            let height = lines.max(line_height);
            size(
                width.unwrap_or(px(100.0)),
                most.map_or(height, |most| height.min(most)),
            )
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let line_height = input.line_height();
        let empty = input.content.is_empty();
        let (text, color) = if empty {
            (input.placeholder.to_string(), theme::ink3())
        } else {
            (input.shown(), self.color)
        };
        let marked = input
            .marked_range
            .as_ref()
            .map(|range| input.shown_offset(range.start)..input.shown_offset(range.end));
        let runs = runs(
            &text,
            input.font(),
            color,
            if empty { None } else { marked.as_ref() },
        );
        let wrap = input.multiline.then_some(bounds.size.width);
        let lines: Vec<WrappedLine> = window
            .text_system()
            .shape_text(text.into(), input.font_size(), &runs, wrap, None)
            .map(|lines| lines.into_iter().collect())
            .unwrap_or_default();
        let height = lines.iter().fold(Pixels::ZERO, |sum, line| {
            sum + line.size(line_height).height
        });
        let layout = Layout {
            lines: lines.clone(),
            line_height,
            bounds,
            scroll: Pixels::ZERO,
            height,
        };
        // Taller than the field: scrolled, and to the cursor when it moved.
        let most = (height - bounds.size.height).max(Pixels::ZERO);
        let mut scroll = input.scroll;
        if input.reveal {
            if let Some(at) = input.position_for_index(&layout, input.cursor_offset()) {
                scroll = scroll_to_show(scroll, at.y, line_height, bounds.size.height);
            }
        }
        let scroll = scroll.clamp(Pixels::ZERO, most);
        let origin = bounds.origin - point(Pixels::ZERO, scroll);
        let mut quads = Vec::new();
        let mut cursor = None;
        let selection = input.selected_range.clone();
        if empty {
            cursor = Some(fill(
                Bounds::new(bounds.origin, size(px(1.5), line_height)),
                theme::accent_ink(),
            ));
        } else if selection.is_empty() {
            if let Some(at) = input.position_for_index(&layout, input.cursor_offset()) {
                cursor = Some(fill(
                    Bounds::new(origin + at, size(px(1.5), line_height)),
                    theme::accent_ink(),
                ));
            }
        } else if let (Some(start), Some(end)) = (
            input.position_for_index(&layout, selection.start),
            input.position_for_index(&layout, selection.end),
        ) {
            let wash = theme::with_alpha(theme::accent(), 0.45);
            if start.y == end.y {
                quads.push(fill(
                    Bounds::from_corners(
                        origin + start,
                        origin + point(end.x, end.y + line_height),
                    ),
                    wash,
                ));
            } else {
                // The first line to its end, the lines between whole, the last from its start.
                quads.push(fill(
                    Bounds::from_corners(
                        origin + start,
                        origin + point(bounds.size.width, start.y + line_height),
                    ),
                    wash,
                ));
                if end.y > start.y + line_height {
                    quads.push(fill(
                        Bounds::from_corners(
                            origin + point(Pixels::ZERO, start.y + line_height),
                            origin + point(bounds.size.width, end.y),
                        ),
                        wash,
                    ));
                }
                quads.push(fill(
                    Bounds::from_corners(
                        origin + point(Pixels::ZERO, end.y),
                        origin + point(end.x, end.y + line_height),
                    ),
                    wash,
                ));
            }
        }
        Prepaint {
            lines,
            quads,
            cursor,
            scroll,
            height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus_handle, line_height, multiline) = {
            let input = self.input.read(cx);
            (
                input.focus_handle.clone(),
                input.line_height(),
                input.multiline,
            )
        };
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let scroll = prepaint.scroll;
        // What scrolls past a field of several lines is not drawn.
        let mask = multiline.then_some(ContentMask { bounds });
        window.with_content_mask(mask, |window| {
            for quad in prepaint.quads.drain(..) {
                window.paint_quad(quad);
            }
            let mut top = bounds.origin.y - scroll;
            for line in &prepaint.lines {
                let _ = line.paint(
                    point(bounds.origin.x, top),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                top += line.size(line_height).height;
            }
            if focus_handle.is_focused(window) {
                if let Some(cursor) = prepaint.cursor.take() {
                    window.paint_quad(cursor);
                }
            }
        });
        let lines = std::mem::take(&mut prepaint.lines);
        let height = prepaint.height;
        self.input.update(cx, |input, _cx| {
            input.scroll = scroll;
            input.reveal = false;
            input.layout = Some(Layout {
                lines,
                line_height,
                bounds,
                scroll,
                height,
            });
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The field's own keys answer inside it, below what the keymap
        // leaves unbound in fields.
        let keys = div()
            .flex()
            .w_full()
            .key_context(KEYS)
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(TextElement {
                input: cx.entity(),
                color: theme::ink(),
            });
        div().flex().w_full().key_context(CONTEXT).child(keys)
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
