//! A text field: one line, or several that wrap (the composer), or code
//! (several lines in mono, as a settings file is written). It takes the
//! platform's input method, so Hangul and other composed text arrive as
//! they are written. Enter submits; Shift+Enter starts a new line where the
//! field has several. In code, Enter starts a new line and ⌘↩ / Ctrl+Enter
//! submits. A field of several lines given a most height grows with its
//! lines up to it, then scrolls, keeping the cursor in view; one line
//! longer than its field scrolls sideways to keep the cursor in view.
//!
//! It keeps its own typing to take back: ⌘Z / Ctrl+Z undoes it, ⌘⇧Z /
//! Ctrl+Shift+Z and Ctrl+Y redo it, and while a field has the keys they
//! never reach the project's Undo. Tab and Shift-Tab leave it for the next
//! or the one before, as everything in the kit does; in code, which is
//! written indented, Tab indents (Shift-Tab takes it back) and Ctrl+Tab and
//! Ctrl+Shift+Tab leave it.

use crate::theme::{self, TypeStyle};
use crate::ui::{FocusNext, FocusPrevious};
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
        SubmitCode,
        Newline,
        WordLeft,
        WordRight,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteToLineStart,
        SelectUp,
        SelectDown,
        SelectHome,
        SelectEnd,
        SelectWordLeft,
        SelectWordRight,
        TextStart,
        TextEnd,
        SelectTextStart,
        SelectTextEnd,
        Undo,
        Redo,
        Cancel,
        Indent,
        Outdent,
    ]
);

/// A row of a field's edit menu, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditRow {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

impl EditRow {
    pub fn words(self) -> &'static str {
        match self {
            EditRow::Undo => "Undo",
            EditRow::Redo => "Redo",
            EditRow::Cut => "Cut",
            EditRow::Copy => "Copy",
            EditRow::Paste => "Paste",
            EditRow::Delete => "Delete",
            EditRow::SelectAll => "Select All",
        }
    }

    /// Its key as the registry writes one (shown in the system's signs by
    /// `shortcuts::shown_keys`); Delete shows none: it is named for its key.
    pub fn keys(self) -> Option<&'static str> {
        match self {
            EditRow::Undo => Some("Ctrl+Z"),
            EditRow::Redo => Some("Ctrl+Shift+Z"),
            EditRow::Cut => Some("Ctrl+X"),
            EditRow::Copy => Some("Ctrl+C"),
            EditRow::Paste => Some("Ctrl+V"),
            EditRow::Delete => None,
            EditRow::SelectAll => Some("Ctrl+A"),
        }
    }
}

/// What a field can do as its edit menu opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditCan {
    pub undo: bool,
    pub redo: bool,
    /// Some of its words are chosen.
    pub chosen: bool,
    /// A secret's words are never cut or copied.
    pub secret: bool,
    /// The clipboard holds words.
    pub paste: bool,
    /// It holds no words.
    pub empty: bool,
    /// All its words are chosen.
    pub all_chosen: bool,
}

/// A field's edit menu: each row, and whether it can be chosen now.
pub fn edit_rows(can: EditCan) -> Vec<(EditRow, bool)> {
    let copyable = can.chosen && !can.secret;
    vec![
        (EditRow::Undo, can.undo),
        (EditRow::Redo, can.redo),
        (EditRow::Cut, copyable),
        (EditRow::Copy, copyable),
        (EditRow::Paste, can.paste),
        (EditRow::Delete, can.chosen),
        (EditRow::SelectAll, !can.empty && !can.all_chosen),
    ]
}

/// The key context of a field. The window's keymap binds each plain key
/// of its commands (Esc, Backspace, Left, a letter) to nothing here, so
/// typing it in a field never runs the command.
pub const CONTEXT: &str = "TextInput";

/// The key context the field's own keys are bound in: one inside
/// [`CONTEXT`], so they come before what the keymap unbinds there, and
/// Backspace, the arrows and Esc still do what they do in a field.
pub const KEYS: &str = "TextInputKeys";

/// The key context a code field adds to [`KEYS`], where ⌘↩ / Ctrl+Enter
/// submits (Enter there starts a new line), Tab indents, Shift-Tab takes
/// an indent back, and Ctrl+Tab and Ctrl+Shift+Tab leave the field.
pub const CODE: &str = "CodeInput";

/// What Tab types in code: the indent of the JSON Settings writes.
pub const INDENT: &str = "  ";

/// What a code field's own keys answer in: [`KEYS`] and [`CODE`] at once.
pub const CODE_KEYS: &str = "TextInputKeys CodeInput";

/// The field's keys, as `keymap::bind` binds them first whenever it
/// binds a keymap. ⌘ arrows and ⌘⌫ are macOS's; Ctrl+Home and Ctrl+End
/// the text's ends elsewhere.
pub fn key_bindings() -> Vec<KeyBinding> {
    key_bindings_for(cfg!(target_os = "macos"))
}

/// The field's keys on macOS (`mac`) or elsewhere.
pub fn key_bindings_for(mac: bool) -> Vec<KeyBinding> {
    let context = Some(KEYS);
    let mut bindings = vec![
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("secondary-v", Paste, context),
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-x", Cut, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("shift-home", SelectHome, context),
        KeyBinding::new("shift-end", SelectEnd, context),
        KeyBinding::new("enter", Submit, context),
        KeyBinding::new("shift-enter", Newline, context),
        KeyBinding::new("alt-left", WordLeft, context),
        KeyBinding::new("alt-right", WordRight, context),
        KeyBinding::new("ctrl-left", WordLeft, context),
        KeyBinding::new("ctrl-right", WordRight, context),
        KeyBinding::new("shift-alt-left", SelectWordLeft, context),
        KeyBinding::new("shift-alt-right", SelectWordRight, context),
        KeyBinding::new("shift-ctrl-left", SelectWordLeft, context),
        KeyBinding::new("shift-ctrl-right", SelectWordRight, context),
        KeyBinding::new("alt-backspace", DeleteWordLeft, context),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, context),
        KeyBinding::new("alt-delete", DeleteWordRight, context),
        KeyBinding::new("ctrl-delete", DeleteWordRight, context),
        // The typing's own undo, before the project's (the registry's
        // Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y).
        KeyBinding::new("secondary-z", Undo, context),
        KeyBinding::new("secondary-shift-z", Redo, context),
        KeyBinding::new("secondary-y", Redo, context),
        KeyBinding::new("escape", Cancel, context),
        KeyBinding::new("tab", FocusNext, context),
        KeyBinding::new("shift-tab", FocusPrevious, context),
    ];
    if mac {
        bindings.extend([
            KeyBinding::new("cmd-left", Home, context),
            KeyBinding::new("cmd-right", End, context),
            KeyBinding::new("cmd-up", TextStart, context),
            KeyBinding::new("cmd-down", TextEnd, context),
            KeyBinding::new("shift-cmd-left", SelectHome, context),
            KeyBinding::new("shift-cmd-right", SelectEnd, context),
            KeyBinding::new("shift-cmd-up", SelectTextStart, context),
            KeyBinding::new("shift-cmd-down", SelectTextEnd, context),
            KeyBinding::new("cmd-backspace", DeleteToLineStart, context),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-home", TextStart, context),
            KeyBinding::new("ctrl-end", TextEnd, context),
            KeyBinding::new("shift-ctrl-home", SelectTextStart, context),
            KeyBinding::new("shift-ctrl-end", SelectTextEnd, context),
        ]);
    }
    bindings.extend([
        // In code, Enter is a new line, and this is what sends it.
        KeyBinding::new("secondary-enter", SubmitCode, Some(CODE)),
        // In code Tab types the indent Settings writes its JSON with;
        // Ctrl+Tab is the way out.
        KeyBinding::new("tab", Indent, Some(CODE)),
        KeyBinding::new("shift-tab", Outdent, Some(CODE)),
        KeyBinding::new("ctrl-tab", FocusNext, Some(CODE)),
        KeyBinding::new("ctrl-shift-tab", FocusPrevious, Some(CODE)),
    ]);
    bindings
}

/// What the field says to whoever holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Changed,
    Submit,
    Cancel,
}

/// What a field held at one moment, to go back to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub text: String,
    pub selection: Range<usize>,
    pub reversed: bool,
}

/// What an edit was, for keeping a run of them as one step back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Letters typed (or composed) where the cursor was; typed over a
    /// selection, they start a run of their own.
    Typing,
    /// Backspace, Delete, or a word or a line taken.
    Deleting,
    /// A paste, a cut, a new line: a step of its own.
    Other,
}

/// A field's own undo and redo. Typing in a run (each letter where the
/// last left the cursor) is one step back, and so is deleting in a run;
/// moving the cursor ([`History::break_run`]) or any other edit starts a
/// new step. A new edit forgets what was undone.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The run open now: its kind and where its last edit left the cursor.
    run: Option<(EditKind, usize)>,
}

impl History {
    /// The most steps kept back.
    pub const MOST: usize = 200;

    /// An edit of `kind` is about to change `before`, leaving the cursor
    /// at `cursor`.
    pub fn record(&mut self, before: Snapshot, kind: EditKind, cursor: usize) {
        let joins = match (self.run, kind) {
            (Some((EditKind::Typing, at)), EditKind::Typing) => {
                before.selection.is_empty() && before.selection.start == at
            }
            (Some((EditKind::Deleting, at)), EditKind::Deleting) => {
                before.selection.start == at || before.selection.end == at
            }
            _ => false,
        };
        if !joins {
            self.undo.push(before);
            if self.undo.len() > Self::MOST {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.run = (kind != EditKind::Other).then_some((kind, cursor));
    }

    /// The cursor moved: the next edit is a step of its own.
    pub fn break_run(&mut self) {
        self.run = None;
    }

    /// What to go back to from `current`, if anything was done.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let back = self.undo.pop()?;
        self.redo.push(current);
        self.run = None;
        Some(back)
    }

    /// What was undone last, to do again from `current`.
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let again = self.redo.pop()?;
        self.undo.push(current);
        self.run = None;
        Some(again)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The kind of an edit that puts `new_text` in place of `replaced` (a
/// range `replaced` long): a letter or two typed (over a selection too,
/// which starts a run of its own), a deletion, or anything else.
pub fn edit_kind(replaced: usize, new_text: &str) -> EditKind {
    if new_text.is_empty() {
        if replaced == 0 {
            EditKind::Other
        } else {
            EditKind::Deleting
        }
    } else if !new_text.contains('\n') && new_text.chars().count() <= 2 {
        EditKind::Typing
    } else {
        EditKind::Other
    }
}

/// What a press selects by, and a drag after it extends by: letters, or
/// (after a double-click) words, or (a triple-click) lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Unit {
    #[default]
    Letter,
    Word,
    Line,
}

impl Unit {
    pub fn of_clicks(count: usize) -> Unit {
        match count {
            0 | 1 => Unit::Letter,
            2 => Unit::Word,
            _ => Unit::Line,
        }
    }
}

/// The word, the run of spaces or the mark at `offset` in `text`, as a
/// double-click chooses it: what is just after the offset, or at the end
/// what is just before it.
pub fn word_at(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    let mut last = 0..0;
    for (at, part) in text.split_word_bound_indices() {
        let range = at..at + part.len();
        if part == "\n" {
            if offset == at && last.end == at && !last.is_empty() {
                return last;
            }
            last = range;
            continue;
        }
        if range.contains(&offset) {
            return range;
        }
        last = range;
    }
    if last.end == offset && text[last.clone()] != *"\n" {
        last
    } else {
        offset..offset
    }
}

/// The line `offset` is on in `text`, without its line break, as a
/// triple-click chooses it.
pub fn line_at(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    let start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |at| offset + at);
    start..end
}

/// What Shift-Tab takes back in code with the cursor at `cursor`: up to
/// an [`INDENT`] of the spaces its line starts with. None when the line
/// starts with none.
pub fn outdent_range(text: &str, cursor: usize) -> Option<Range<usize>> {
    let start = line_at(text, cursor).start;
    let spaces = text[start..]
        .bytes()
        .take(INDENT.len())
        .take_while(|byte| *byte == b' ')
        .count();
    (spaces > 0).then(|| start..start + spaces)
}

/// The unit of `text` at `offset`.
pub fn unit_at(text: &str, offset: usize, unit: Unit) -> Range<usize> {
    match unit {
        Unit::Letter => {
            let offset = offset.min(text.len());
            offset..offset
        }
        Unit::Word => word_at(text, offset),
        Unit::Line => line_at(text, offset),
    }
}

/// A drag from a press that chose `anchor` (by `unit`) to `offset`: the
/// range from the anchor to the whole unit at the pointer, and whether the
/// cursor is at its start (the drag went back).
pub fn extend_by(
    text: &str,
    anchor: Range<usize>,
    offset: usize,
    unit: Unit,
) -> (Range<usize>, bool) {
    let here = unit_at(text, offset, unit);
    if here.start < anchor.start {
        (here.start..anchor.end, true)
    } else {
        (anchor.start..here.end.max(anchor.end), false)
    }
}

/// Where a one-line field `visible` wide, scrolled `scroll` sideways,
/// goes to show the cursor at `cursor` in text `width` wide: no further
/// than it must, never past the text's end, never before its start.
pub fn scroll_x_to_show(scroll: f32, cursor: f32, width: f32, visible: f32) -> f32 {
    let caret = CURSOR_WIDTH;
    let most = (width + caret - visible).max(0.0);
    let scroll = if cursor < scroll {
        cursor
    } else if cursor + caret > scroll + visible {
        cursor + caret - visible
    } else {
        scroll
    };
    scroll.clamp(0.0, most)
}

/// Points wide the cursor is drawn.
const CURSOR_WIDTH: f32 = 1.5;

pub struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    multiline: bool,
    style: TypeStyle,
    reading: bool,
    /// Code: mono in the field's own size, Enter a new line.
    code: bool,
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
    /// How far one line is scrolled sideways, past its start.
    scroll_x: Pixels,
    /// The cursor moved or the text changed: the next drawing scrolls to
    /// keep the cursor in view. A wheel scrolls away from it.
    reveal: bool,
    /// Its own typing, to take back and do again.
    history: History,
    /// What the press that is held chose, and by what, so a drag after a
    /// double-click goes by words.
    press: Option<(Range<usize>, Unit)>,
    /// The kind of the edit the next change is (a paste, a cut), when it
    /// is not what the change itself says.
    next_kind: Option<EditKind>,
    /// Its edit menu while open (a right-click), and where: the kit's,
    /// which holds the keys and gives them back to the field.
    edit_menu: Option<(Entity<crate::ui::Menu>, Point<Pixels>)>,
}

/// Where the text was drawn last, to find the index under the pointer.
#[derive(Clone)]
struct Layout {
    lines: Vec<WrappedLine>,
    line_height: Pixels,
    /// The field's own bounds: what of the text shows.
    bounds: Bounds<Pixels>,
    /// How far the text was scrolled up in them, and sideways.
    scroll: Pixels,
    scroll_x: Pixels,
    /// The height of all its lines.
    height: Pixels,
}

impl EventEmitter<InputEvent> for TextInput {}

impl TextInput {
    pub fn new(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        Self {
            // Tab reaches every field.
            focus_handle: cx.focus_handle().tab_stop(true),
            content: String::new(),
            placeholder: placeholder.into(),
            multiline: false,
            style: theme::text::BODY,
            reading: false,
            code: false,
            secret: false,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            layout: None,
            is_selecting: false,
            max_height: None,
            scroll: Pixels::ZERO,
            scroll_x: Pixels::ZERO,
            reveal: false,
            history: History::default(),
            press: None,
            next_kind: None,
            edit_menu: None,
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

    /// Several lines of code: mono in the field's own size (see
    /// [`TextInput::with_style`]), not the reading face; Enter starts a new
    /// line and ⌘↩ / Ctrl+Enter submits. Settings' MCP servers are written
    /// in one.
    pub fn code(mut self) -> Self {
        self.multiline = true;
        self.reading = false;
        self.code = true;
        self
    }

    /// Whether Enter starts a new line here rather than submitting.
    pub fn is_code(&self) -> bool {
        self.code
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
        // Text put in from outside starts the field's history afresh.
        self.history.clear();
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    /// Whether the field has typing to take back, and to do again.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
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
        if let Some(offset) = self.vertical_target(direction) {
            self.move_to(offset, cx);
        }
    }

    /// Where the cursor goes a line up (`direction` -1) or down (1).
    fn vertical_target(&self, direction: f32) -> Option<usize> {
        let layout = self.layout.clone()?;
        let position = self.position_for_index(&layout, self.cursor_offset())?;
        let target = point(
            position.x,
            position.y + layout.line_height * direction + layout.line_height / 2.0,
        );
        Some(if target.y < Pixels::ZERO {
            0
        } else {
            self.index_for_local(&layout, target)
        })
    }

    /// Shift+Up: a line up, choosing; in one line, to its start.
    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.multiline {
            self.vertical_target(-1.0).unwrap_or(0)
        } else {
            0
        };
        self.select_to(target, cx);
    }

    /// Shift+Down: a line down, choosing; in one line, to its end.
    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let target = if self.multiline {
            self.vertical_target(1.0).unwrap_or(self.content.len())
        } else {
            self.content.len()
        };
        self.select_to(target, cx);
    }

    fn line_start(&self) -> usize {
        line_at(&self.content, self.cursor_offset()).start
    }

    fn line_end(&self) -> usize {
        line_at(&self.content, self.cursor_offset()).end
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_start(), cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_end(), cx);
    }

    fn text_start(&mut self, _: &TextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn text_end(&mut self, _: &TextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_text_start(&mut self, _: &SelectTextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_text_end(&mut self, _: &SelectTextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_word(self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_word(self.cursor_offset()), cx);
    }

    fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.extend_to(self.next_word(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    /// ⌘⌫: what is before the cursor on its line goes.
    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.extend_to(self.line_start(), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    /// What the field holds now, to come back to.
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.content.clone(),
            selection: self.selected_range.clone(),
            reversed: self.selection_reversed,
        }
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.content = snapshot.text;
        let len = self.content.len();
        self.selected_range = snapshot.selection.start.min(len)..snapshot.selection.end.min(len);
        self.selection_reversed = snapshot.reversed;
        self.marked_range = None;
        self.reveal = true;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    /// Takes back the field's last step of typing. With nothing to take
    /// back it does nothing, and the project's Undo still does not run.
    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.is_some() {
            return;
        }
        let current = self.snapshot();
        if let Some(back) = self.history.undo(current) {
            self.restore(back, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.is_some() {
            return;
        }
        let current = self.snapshot();
        if let Some(again) = self.history.redo(current) {
            self.restore(again, cx);
        }
    }

    /// Tab leaves the field for what comes next, unless what holds it
    /// takes Tab itself (the launcher's own stops).
    fn focus_next(&mut self, _: &FocusNext, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::tab_to(true, window, cx);
    }

    fn focus_previous(&mut self, _: &FocusPrevious, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::tab_to(false, window, cx);
    }

    /// Tab in code: the indent is typed where the cursor is.
    fn indent(&mut self, _: &Indent, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.is_some() {
            return;
        }
        self.replace_text_in_range(None, INDENT, window, cx);
    }

    /// Shift-Tab in code: the cursor's line gives back an indent.
    fn outdent(&mut self, _: &Outdent, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.is_some() {
            return;
        }
        let Some(taken) = outdent_range(&self.content, self.cursor_offset()) else {
            return;
        };
        let keep = |offset: usize| {
            if offset >= taken.end {
                offset - taken.len()
            } else {
                offset.min(taken.start)
            }
        };
        let (chosen, reversed) = (self.selected_range.clone(), self.selection_reversed);
        self.next_kind = Some(EditKind::Other);
        self.replace_text_in_range(Some(self.range_to_utf16(&taken)), "", window, cx);
        self.selected_range = keep(chosen.start)..keep(chosen.end);
        self.selection_reversed = reversed && !self.selected_range.is_empty();
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
            self.extend_to(self.previous_word(self.cursor_offset()), cx)
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
        self.choose_all(cx);
        self.share_primary(cx);
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
            self.extend_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.extend_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        // While text is being composed, Enter belongs to the input method.
        if self.marked_range.is_some() {
            return;
        }
        if self.code {
            self.replace_text_in_range(None, "\n", window, cx);
        } else {
            cx.emit(InputEvent::Submit);
        }
    }

    fn submit_code(&mut self, _: &SubmitCode, _: &mut Window, cx: &mut Context<Self>) {
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
        let unit = Unit::of_clicks(event.click_count);
        if unit != Unit::Letter {
            // A double-click chooses the word, a triple-click the line;
            // dragging on goes by them.
            let chosen = unit_at(&self.content, offset, unit);
            self.selected_range = chosen.clone();
            self.selection_reversed = false;
            self.press = Some((chosen, unit));
            self.history.break_run();
            self.reveal = true;
            cx.notify();
            return;
        }
        self.press = None;
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let chose = self.is_selecting;
        self.is_selecting = false;
        self.press = None;
        if chose {
            self.share_primary(cx);
        }
    }

    /// On Linux, what the pointer chose is the primary selection, which a
    /// middle-click pastes elsewhere, here or in another app; never a
    /// secret's.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn share_primary(&self, cx: &mut Context<Self>) {
        if self.secret || self.selected_range.is_empty() {
            return;
        }
        if let Some(words) = self.content.get(self.selected_range.clone()) {
            cx.write_to_primary(ClipboardItem::new_string(words.to_owned()));
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    fn share_primary(&self, _: &mut Context<Self>) {}

    /// A middle-click on Linux pastes the primary selection where it
    /// went down.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn on_middle_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = cx.read_from_primary().and_then(|item| item.text()) else {
            return;
        };
        window.focus(&self.focus_handle);
        let offset = self.index_for_mouse_position(event.position);
        self.move_to(offset, cx);
        let text = if self.multiline {
            text
        } else {
            text.replace('\n', " ")
        };
        self.next_kind = Some(EditKind::Other);
        self.replace_text_in_range(None, &text, window, cx);
        cx.stop_propagation();
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    fn on_middle_down(&mut self, _: &MouseDownEvent, _: &mut Window, _: &mut Context<Self>) {}

    /// A right-click: the field takes the keys, a press outside what is
    /// chosen puts the cursor there (inside it keeps it), and its edit
    /// menu opens at the pointer ([`edit_rows`]).
    fn on_right_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        let offset = self.index_for_mouse_position(event.position);
        let range = &self.selected_range;
        if range.is_empty() || offset < range.start || offset > range.end {
            self.move_to(offset, cx);
        }
        let can = EditCan {
            undo: self.history.can_undo(),
            redo: self.history.can_redo(),
            chosen: !self.selected_range.is_empty(),
            secret: self.secret,
            paste: cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|text| !text.is_empty()),
            empty: self.content.is_empty(),
            all_chosen: self.selected_range == (0..self.content.len()),
        };
        let field = cx.entity().downgrade();
        let items: Vec<crate::ui::MenuItem> = edit_rows(can)
            .into_iter()
            .map(|(row, enabled)| {
                let field = field.clone();
                let mut item = crate::ui::MenuItem::new(row.words()).disabled(!enabled);
                if let Some(keys) = row.keys() {
                    item = item.key(editor_state::shortcuts::shown_keys(keys));
                }
                if matches!(row, EditRow::Cut | EditRow::SelectAll) {
                    item = item.separated();
                }
                item.on_choose(move |window, cx| {
                    let _ = field.update(cx, |field, cx| field.run_edit(row, window, cx));
                })
            })
            .collect();
        let menu = cx.new(|cx| crate::ui::Menu::new(items, window, cx));
        cx.subscribe(&menu, |this, menu, _: &gpui::DismissEvent, cx| {
            if this
                .edit_menu
                .as_ref()
                .is_some_and(|(open, _)| *open == menu)
            {
                this.edit_menu = None;
                cx.notify();
            }
        })
        .detach();
        self.edit_menu = Some((menu, event.position));
        cx.stop_propagation();
        cx.notify();
    }

    /// What a row of the edit menu does, as its key would.
    fn run_edit(&mut self, row: EditRow, window: &mut Window, cx: &mut Context<Self>) {
        match row {
            EditRow::Undo => self.undo(&Undo, window, cx),
            EditRow::Redo => self.redo(&Redo, window, cx),
            EditRow::Cut => self.cut(&Cut, window, cx),
            EditRow::Copy => self.copy(&Copy, window, cx),
            EditRow::Paste => self.paste(&Paste, window, cx),
            EditRow::Delete => {
                if !self.selected_range.is_empty() {
                    self.next_kind = Some(EditKind::Other);
                    self.replace_text_in_range(None, "", window, cx);
                }
            }
            EditRow::SelectAll => self.select_all(&SelectAll, window, cx),
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_selecting {
            return;
        }
        let offset = self.index_for_mouse_position(event.position);
        match self.press.clone() {
            Some((anchor, unit)) => {
                let (range, reversed) = extend_by(&self.content, anchor, offset, unit);
                self.selected_range = range;
                self.selection_reversed = reversed;
                self.reveal = true;
                cx.notify();
            }
            None => self.select_to(offset, cx),
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
            self.next_kind = Some(EditKind::Other);
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
            self.next_kind = Some(EditKind::Other);
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.reveal = true;
        self.history.break_run();
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    /// Shift with a move, or a drag: the selection's moving end goes to
    /// `offset`, and the next edit is a step of its own, as after any move
    /// of the cursor (deleting what was chosen never joins the Backspaces
    /// before it).
    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.history.break_run();
        self.extend_to(offset, cx);
    }

    /// The selection's moving end goes to `offset`, leaving the run of
    /// edits open: Backspace and Delete choose what they take this way.
    fn extend_to(&mut self, offset: usize, cx: &mut Context<Self>) {
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
        let local = position - layout.bounds.origin + point(layout.scroll_x, layout.scroll);
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
        } else if self.style.mono || self.code {
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
        let kind = self
            .next_kind
            .take()
            .unwrap_or_else(|| edit_kind(range.len(), &new_text));
        // What ends a composition was kept as it began.
        if self.marked_range.is_none() && !(range.is_empty() && new_text.is_empty()) {
            let before = self.snapshot();
            self.history
                .record(before, kind, range.start + new_text.len());
        }
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
        // A composition is typing, kept as it begins.
        if self.marked_range.is_none() {
            let before = self.snapshot();
            let kind = if range.is_empty() {
                EditKind::Typing
            } else {
                EditKind::Other
            };
            self.history
                .record(before, kind, range.start + new_text.len());
        }
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
        let origin = bounds.origin - point(layout.scroll_x, layout.scroll);
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
        let local = point - layout.bounds.origin + gpui::point(layout.scroll_x, layout.scroll);
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
    /// How far the text is scrolled up and sideways, and the height of
    /// all its lines.
    scroll: Pixels,
    scroll_x: Pixels,
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
        // Folded away to nothing, Tab passes it over.
        crate::ui::note_shown(&input.focus_handle, bounds, window);
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
            scroll_x: Pixels::ZERO,
            height,
        };
        // Taller than the field: scrolled, and to the cursor when it moved.
        let most = (height - bounds.size.height).max(Pixels::ZERO);
        let mut scroll = input.scroll;
        let cursor_at = input.position_for_index(&layout, input.cursor_offset());
        if input.reveal
            && let Some(at) = cursor_at
        {
            scroll = scroll_to_show(scroll, at.y, line_height, bounds.size.height);
        }
        let scroll = scroll.clamp(Pixels::ZERO, most);
        // One line wider than the field: sideways, to the cursor when it
        // moved, and never past the text's end.
        let scroll_x = if input.multiline || empty {
            Pixels::ZERO
        } else {
            let width = lines
                .first()
                .map_or(Pixels::ZERO, |line| line.size(line_height).width);
            // Not revealing, it is held where it was, inside what the text
            // allows now.
            let shown = match cursor_at {
                Some(at) if input.reveal => at.x,
                _ => input.scroll_x,
            };
            px(scroll_x_to_show(
                f32::from(input.scroll_x),
                f32::from(shown),
                f32::from(width),
                f32::from(bounds.size.width),
            ))
        };
        let origin = bounds.origin - point(scroll_x, scroll);
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
            scroll_x,
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
        let scroll_x = prepaint.scroll_x;
        // What scrolls past the field is not drawn: above and below a field
        // of several lines, beside one line (its glyphs keep their room
        // above and below).
        let mask = if multiline {
            bounds
        } else {
            Bounds::from_corners(
                point(bounds.left(), bounds.top() - line_height),
                point(bounds.right(), bounds.bottom() + line_height),
            )
        };
        window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
            for quad in prepaint.quads.drain(..) {
                window.paint_quad(quad);
            }
            let mut top = bounds.origin.y - scroll;
            for line in &prepaint.lines {
                if let Err(error) = line.paint(
                    point(bounds.origin.x - scroll_x, top),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                ) {
                    log::warn!("a text field's line did not draw: {error}");
                }
                top += line.size(line_height).height;
            }
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        let lines = std::mem::take(&mut prepaint.lines);
        let height = prepaint.height;
        self.input.update(cx, |input, _cx| {
            input.scroll = scroll;
            input.scroll_x = scroll_x;
            input.reveal = false;
            input.layout = Some(Layout {
                lines,
                line_height,
                bounds,
                scroll,
                scroll_x,
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
            .key_context(if self.code { CODE_KEYS } else { KEYS })
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
            .on_action(cx.listener(Self::submit_code))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::text_start))
            .on_action(cx.listener(Self::text_end))
            .on_action(cx.listener(Self::select_text_start))
            .on_action(cx.listener(Self::select_text_end))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_middle_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(TextElement {
                input: cx.entity(),
                color: theme::ink(),
            });
        // The edit menu at the pointer, kept inside the window; beside the
        // field's keys, not under them, so its own keys come first.
        let menu = self.edit_menu.as_ref().map(|(menu, at)| {
            gpui::deferred(
                gpui::anchored()
                    .position(*at)
                    .snap_to_window_with_margin(px(theme::space::S8))
                    .child(menu.clone()),
            )
            .with_priority(2)
        });
        div()
            .flex()
            .w_full()
            .child(div().flex().w_full().key_context(CONTEXT).child(keys))
            .children(menu)
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
