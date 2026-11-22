//! Where the window's keys go: settled on the timeline while it shows, else
//! with the window, never away from a field, a menu or what is open over the
//! window ([`settle_keys`]); what Esc does first, ahead of every Esc the
//! window binds ([`escape_goes`]); and each command's keys in the keymap in
//! use, for the toolbar's tips.

use super::asked::CONFIRM;
use super::Workspace;
use crate::sidebar;
use crate::store::Connection;
use crate::ui;
use gpui::{App, Context, Focusable, Subscription, Window};

impl Workspace {
    /// Esc, before the timeline's (Reject Proposal, or letting the chosen
    /// clip go) or a field's has it ([`escape_goes`]): an open menu goes,
    /// wherever the keys are (a menu's press leaves them where they were);
    /// else the preview render its card shows stops; else the Media band is
    /// put away.
    pub(super) fn escape_first(window: &Window, cx: &mut Context<Self>) -> Subscription {
        let this = cx.weak_entity();
        let handle = window.window_handle();
        cx.intercept_keystrokes(move |event, window, cx| {
            let keystroke = &event.keystroke;
            if window.window_handle() != handle
                || keystroke.key != "escape"
                || keystroke.modifiers.modified()
            {
                return;
            }
            let taken = this
                .update(cx, |this, cx| {
                    let mut at = this.escape_at(&event.context_stack, cx);
                    // The conversation's own intercept may have put
                    // its menu away on this Esc already: it was the
                    // menu's, not a render's or the band's.
                    at.menu_open |= this.chat.update(cx, |chat, _| chat.take_escape());
                    match escape_goes(at) {
                        EscapeGoes::On => false,
                        EscapeGoes::CloseMenus => this.close_menus(window, cx),
                        EscapeGoes::StopRender => {
                            this.stop_render(cx);
                            true
                        }
                        EscapeGoes::PutBandAway => {
                            this.set_media(false, cx);
                            true
                        }
                    }
                })
                .unwrap_or(false);
            if taken {
                cx.stop_propagation();
            }
        })
    }

    /// Whether the panel shows the timeline, with kerfd there to answer
    /// its keys: where the keys belong when nothing else holds them.
    fn timeline_in_view(&self, cx: &App) -> bool {
        self.panel_open
            && self.timeline_shown.target() > 0.5
            && self.modal.is_none()
            && self.session.read(cx).state.connection == Connection::Connected
    }

    /// Settles the keys where they belong: on the timeline while the panel
    /// shows it, since its keys (Space, Home and End, the arrows, S, Delete, M,
    /// the tools) work only from it; never away from a field being typed in, a
    /// menu, or what is open over the window. A region folded away gives back
    /// the keys it held. Called as the window first comes to the front, as what
    /// is over it or the Media band closes, when Esc leaves a field, and when
    /// nothing holds them.
    pub fn settle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        // The Media band put away keeps nothing of the keys (it stays laid
        // out, no height; what leaves the window gives them up by itself).
        if !self.media_open && self.media.focus_handle(cx).contains_focused(window, cx) {
            window.focus(&self.focus_handle);
        }
        let timeline = self.timeline.focus_handle(cx);
        let holder = match window.focused(cx) {
            None => Holder::Nothing,
            Some(focused) if focused == self.focus_handle => Holder::Nothing,
            Some(focused) if focused == timeline => Holder::Timeline,
            Some(_) if holds_keys_itself(&window.context_stack()) => Holder::Busy,
            Some(_) => Holder::Other,
        };
        match settle_keys(self.timeline_in_view(cx), holder) {
            KeysGo::Timeline => self.timeline.read(cx).focus(window),
            KeysGo::Window => window.focus(&self.focus_handle),
            KeysGo::Stay => {}
        }
    }

    /// The keys settle once this frame is drawn: what folds away now is
    /// still laid out where it was.
    pub(super) fn settle_soon(&mut self, cx: &mut Context<Self>) {
        self.settle_soon = true;
        cx.notify();
    }

    /// Whether `id` answers plain Esc in the keymap in use.
    pub(super) fn bound_to_escape(&self, id: &str) -> bool {
        self.keys.1.get(id).is_some_and(|keys| {
            keys.iter()
                .any(|key| crate::shortcuts::keystroke(key).as_deref() == Some("escape"))
        })
    }

    /// Reads each command's keys again when the keymap has changed.
    pub(super) fn refresh_keys(&mut self, cx: &App) {
        let keymap = self.session.read(cx).prefs.keymap.wire();
        if self.keys.0 == keymap {
            return;
        }
        let keys = crate::commands::catalog(keymap)
            .into_iter()
            .map(|command| (command.id, command.keys))
            .collect();
        self.keys = (keymap.to_owned(), keys);
    }

    /// Command `id`'s first key as the system writes it ("⌘E", "Ctrl+E").
    pub(super) fn shown_key(&self, id: &str) -> Option<String> {
        self.keys
            .1
            .get(id)
            .and_then(|keys| keys.first())
            .map(|key| crate::shortcuts::shown_keys(key))
    }

    /// Esc in the window (outside a field, the timeline, and whatever is
    /// open over the window): stops the preview render its card shows,
    /// else puts the Media band away.
    pub(super) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_menus(window, cx) {
            return;
        }
        if self.render_card_shows(cx) {
            self.stop_render(cx);
        } else if self.media_open {
            self.set_media(false, cx);
        }
    }

    /// Whether the preview render's card shows over the picture: the stage
    /// draws it ([`StageView::render_card_shown`]) and the picture is in.
    ///
    /// [`StageView::render_card_shown`]: crate::stage::StageView::render_card_shown
    fn render_card_shows(&self, cx: &App) -> bool {
        self.panel_open
            && self.picture_shown.target() > 0.5
            && self.stage.read(cx).render_card_shown(cx)
    }

    /// Where the keys are (`stack`, their key contexts) and what stands
    /// open, as Esc is pressed ([`escape_goes`]).
    fn escape_at(&self, stack: &[gpui::KeyContext], cx: &App) -> EscapeAt {
        let has = |name: &str| stack.iter().any(|context| context.contains(name));
        EscapeAt {
            menu_has_keys: has(ui::MENU),
            menu_open: self.title_menu.is_some()
                || self.app_menu.is_some()
                || self.activity_open
                || self.sidebar.read(cx).menu_open()
                || self.chat.read(cx).menu_open(),
            in_field: has(crate::input::CONTEXT),
            // The chip's questions count as dialogs: their own Close has
            // Esc, keys in them or not.
            in_dialog: self.modal.is_some()
                || self.export.is_some()
                || ui::panel_of(stack).is_some()
                || has(crate::palette::CONTEXT)
                || has(CONFIRM)
                || self.collab.read(cx).asking(cx),
            on_picture: has(crate::stage::CONTEXT),
            rendering: self.render_card_shows(cx),
            band_open: self.media_open,
        }
    }

    /// Stops the preview rendering as the render card's Stop does: its card
    /// goes at once, done with until that render ends
    /// ([`StageView::stop_render`]).
    ///
    /// [`StageView::stop_render`]: crate::stage::StageView::stop_render
    fn stop_render(&mut self, cx: &mut Context<Self>) {
        self.stage.update(cx, |stage, cx| stage.stop_render(cx));
    }
}

/// What holds the keys as the window settles them ([`settle_keys`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holder {
    /// Nothing, or the window's own box.
    Nothing,
    /// The timeline.
    Timeline,
    /// A field being typed in, a menu, or a panel open over the window
    /// ([`holds_keys_itself`]).
    Busy,
    /// Something else that takes the keys: a button, a row, the picture.
    Other,
}

/// Where the keys go as the window settles them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeysGo {
    Timeline,
    /// The window's own box: its keys (the palette, the mask tools) work.
    Window,
    /// Where they are.
    Stay,
}

/// Where the keys go as the window settles them: to the timeline while it is in
/// view (`timeline_in_view`), unless someone is typing, choosing in a menu, or
/// in a panel over the window; with no timeline in view they stay where they
/// are, and the timeline folded away gives them back to the window.
pub fn settle_keys(timeline_in_view: bool, holder: Holder) -> KeysGo {
    match holder {
        Holder::Busy => KeysGo::Stay,
        Holder::Timeline if timeline_in_view => KeysGo::Stay,
        Holder::Timeline => KeysGo::Window,
        Holder::Nothing | Holder::Other if timeline_in_view => KeysGo::Timeline,
        Holder::Nothing => KeysGo::Window,
        Holder::Other => KeysGo::Stay,
    }
}

/// Whether what has the keys (its key contexts, outermost first) keeps
/// them as the window settles them: a field being typed in, a menu, or a
/// panel open over the window.
pub fn holds_keys_itself(stack: &[gpui::KeyContext]) -> bool {
    let busy = [
        crate::input::CONTEXT,
        crate::ui::MENU,
        sidebar::MENU,
        crate::palette::CONTEXT,
        CONFIRM,
    ];
    ui::panel_of(stack).is_some()
        || stack
            .iter()
            .any(|context| busy.iter().any(|name| context.contains(name)))
}

/// Where the keys are as Esc is pressed, and what stands open, for
/// [`escape_goes`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EscapeAt {
    /// One of the kit's menus has the keys: it closes itself (its deepest
    /// submenu first).
    pub menu_has_keys: bool,
    /// One of the window's own menus is open (the title's, Activity, the
    /// app's, the sidebar's), whether or not it has the keys.
    pub menu_open: bool,
    /// A field is being typed in: its own Esc goes first.
    pub in_field: bool,
    /// What one goes into is over the window, with the keys or not:
    /// Settings, the palette, a question, a shot's graph, Export's popover.
    pub in_dialog: bool,
    /// The picture has the keys (a mask tool out, a mask chosen).
    pub on_picture: bool,
    /// The preview render's card shows over the picture
    /// ([`render_card_shows`]): Esc stops a render only while its card is
    /// in sight, never one the person cannot see.
    ///
    /// [`render_card_shows`]: super::render_card_shows
    pub rendering: bool,
    /// The Media band is open.
    pub band_open: bool,
}

/// What Esc does first, ahead of every Esc the window binds (the
/// timeline's Reject Proposal and its letting a clip go among them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeGoes {
    /// Nothing here: it goes on to what has the keys (a menu, a field, a
    /// dialog, the picture), then to the window's own bindings.
    On,
    /// The open menus close.
    CloseMenus,
    /// The preview rendering stops.
    StopRender,
    /// The Media band is put away.
    PutBandAway,
}

/// What Esc does first: an open menu goes first; then a preview rendering
/// stops (not from a field, a menu or a dialog, whose own Esc comes
/// first); then the Media band closes (not from a field, the picture or a
/// dialog); else it goes on to whatever has the keys.
pub fn escape_goes(at: EscapeAt) -> EscapeGoes {
    if at.menu_has_keys {
        return EscapeGoes::On;
    }
    if at.menu_open {
        return EscapeGoes::CloseMenus;
    }
    if at.in_field || at.in_dialog {
        return EscapeGoes::On;
    }
    if at.rendering {
        return EscapeGoes::StopRender;
    }
    if at.band_open && !at.on_picture {
        return EscapeGoes::PutBandAway;
    }
    EscapeGoes::On
}
