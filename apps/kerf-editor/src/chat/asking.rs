//! Asking Kerf: why nothing can be asked now ([`Refusal`]), what waits
//! while Kerf works ([`Queued`]), and the view's requests as they go, wait
//! for the turn under way, go again, stop, and come back to the composer
//! when a turn ends troubled; and the Ask popover.

use super::{ChatView, PLACEHOLDER};
use crate::session::Focused;
use crate::store::Connection;
use gpui::{prelude::*, Context, SharedString, Window};

/// A request written while Kerf works, joined to the one already waiting.
pub fn join_queued(waiting: Option<&str>, text: &str) -> String {
    let text = text.trim();
    match waiting.map(str::trim).filter(|waiting| !waiting.is_empty()) {
        Some(waiting) if !text.is_empty() => format!("{waiting}\n\n{text}"),
        Some(waiting) => waiting.to_owned(),
        None => text.to_owned(),
    }
}

/// What waits for Kerf to finish: the requests written while it works,
/// joined as one, and the playhead time the latest of them was written at;
/// while all that waits is one Ask again, also what that request went about
/// the first time ([`Queued::push_again`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queued(Option<Waiting>);

#[derive(Clone, Debug, PartialEq)]
struct Waiting {
    text: String,
    at: f64,
    about: Option<Focused>,
}

impl Queued {
    /// `text`, written with the playhead at `at`, joins what waits, and the
    /// whole goes with `at`, about the sequence there as the composer's
    /// requests go; nothing written leaves the queue as it is.
    pub fn push(&mut self, text: &str, at: f64) {
        if text.trim().is_empty() {
            return;
        }
        let text = join_queued(self.text(), text);
        self.0 = Some(Waiting {
            text,
            at,
            about: None,
        });
    }

    /// Ask again pressed while Kerf works, with the playhead at `at`: `text`
    /// waits, and while it is all that waits it goes about `about` when the
    /// turn ends (what it went about when this window first sent it, the
    /// Ask popover's clip or scope), else as [`Queued::push`]'s requests
    /// go. Joined with another request, it goes as they do; pressed again
    /// while the same words wait, nothing changes.
    pub fn push_again(&mut self, text: &str, at: f64, about: Option<Focused>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match &self.0 {
            None => {
                self.0 = Some(Waiting {
                    text: text.to_owned(),
                    at,
                    about,
                })
            }
            Some(waiting) if waiting.text == text => {}
            Some(_) => self.push(text, at),
        }
    }

    /// The request waiting, if one is.
    pub fn text(&self) -> Option<&str> {
        self.0.as_ref().map(|waiting| waiting.text.as_str())
    }

    /// What the request waiting goes about, when it is one Ask again of
    /// words this window sent before; None for the sequence at its time.
    pub fn about(&self) -> Option<&Focused> {
        self.0.as_ref().and_then(|waiting| waiting.about.as_ref())
    }

    /// The request waiting and its playhead time, taken to be sent: nothing
    /// waits after.
    pub fn take(&mut self) -> Option<(String, f64)> {
        self.0.take().map(|waiting| (waiting.text, waiting.at))
    }

    /// Nothing waits any more (Cancel).
    pub fn clear(&mut self) {
        self.0 = None;
    }
}

/// Why the composer cannot ask: what it says in its placeholder's place,
/// and the reason a hover over it (or Return) gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub placeholder: &'static str,
    pub reason: String,
}

/// Why nothing can be asked now: no folder open, kerfd not connected, or
/// (while Kerf is not working) a connection that cannot take requests,
/// `ask_reason` being the registry's reason for `ai.ask` ("Add your API
/// key in Settings → Chat", the Kerf account's). None when a request can
/// go.
pub fn refusal(
    has_project: bool,
    disconnected: bool,
    busy: bool,
    ask_reason: Option<&str>,
) -> Option<Refusal> {
    let refused = |placeholder, reason: &str| {
        Some(Refusal {
            placeholder,
            reason: reason.to_owned(),
        })
    };
    if !has_project {
        refused(
            "Open a folder to ask Kerf",
            "Open a folder or start a project first",
        )
    } else if disconnected {
        refused("Connect to ask Kerf", "Connect to start editing")
    } else {
        match ask_reason.filter(|reason| !busy && !reason.trim().is_empty()) {
            Some(reason) => refused("Set up Kerf in Settings to ask", reason),
            None => None,
        }
    }
}

/// The registry's reason Ask cannot run on a connection whose access is
/// `access` (`noKey`, or a Kerf account's standing such as `signedOut`),
/// whatever else holds; None for `ready`.
pub fn access_reason(access: &str) -> Option<String> {
    let states =
        kerf_bridge::commands_states(&serde_json::json!({"connected": true, "aiAccess": access}));
    states["ai.ask"].as_str().map(str::to_owned)
}

impl ChatView {
    /// Opens the Ask popover over `anchor` (in the window: the chosen clip
    /// on the timeline, else the timeline at the playhead), with the
    /// playhead's time as its title; its request goes about the chosen
    /// clip or the whole sequence ([`crate::ask::AskView`]). The window
    /// calls it for Ask Kerf… (`ai.ask`, ⌘J). A second call while it is
    /// open moves it.
    pub fn open_ask(
        &mut self,
        anchor: gpui::Bounds<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.clone();
        let ask = cx.new(|cx| crate::ask::AskView::new(session, anchor, window, cx));
        cx.subscribe(&ask, |this, ask, _: &gpui::DismissEvent, cx| {
            if this.ask.as_ref() == Some(&ask) {
                this.ask = None;
                cx.notify();
            }
        })
        .detach();
        ask.read(cx).focus(window, cx);
        self.ask = Some(ask);
        cx.notify();
    }

    /// Whether the Ask popover is open.
    pub fn asking(&self) -> bool {
        self.ask.is_some()
    }

    /// The request waiting for Kerf to finish, if one is.
    pub fn queued(&self) -> Option<&str> {
        self.queued.text()
    }

    /// Why the composer cannot ask now, as last worked out; None when it can.
    pub(super) fn cannot_ask(&self) -> Option<&Refusal> {
        self.refusal.as_ref()
    }

    /// Works out again whether a request can go, and says so in the
    /// composer's placeholder. The connection's access (its key, Kerf's
    /// account) is asked of the registry only when it changes.
    pub(super) fn refresh_placeholder(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let busy = session.state.work.busy();
        let cloud = crate::session::cloud(cx);
        let access = (!busy).then(|| session.ai_access(cloud)).flatten();
        if self.access.as_ref() != Some(&access) {
            self.ask_reason = access.as_deref().and_then(access_reason);
            self.access = Some(access);
        }
        self.refusal = refusal(
            session.has_project(),
            // Connecting counts as not connected: a request has nowhere
            // to go until kerfd is ready.
            session.state.connection != Connection::Connected,
            busy,
            self.ask_reason.as_deref(),
        );
        let placeholder: SharedString = self
            .refusal
            .as_ref()
            .map_or(PLACEHOLDER, |refused| refused.placeholder)
            .into();
        if placeholder != self.placeholder {
            self.placeholder = placeholder.clone();
            self.composer
                .update(cx, |composer, cx| composer.set_placeholder(placeholder, cx));
        }
    }

    /// Sends what is written; while Kerf works, it waits for the turn to end,
    /// joining what already waits.
    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        if self.cannot_ask().is_some() {
            // A key or a choice saved in another window since counts.
            self.session
                .update(cx, |session, _| session.prefs.refresh());
            self.access = None;
            self.refresh_placeholder(cx);
        }
        if let Some(refused) = self.cannot_ask() {
            let reason = refused.reason.clone();
            self.session.update(cx, |session, cx| {
                session.state.notify(&reason, true);
                cx.notify();
            });
            return;
        }
        let session = self.session.read(cx);
        if session.state.work.busy() {
            // It goes with the playhead time the latest request was written at.
            let at = session.state.position;
            self.queued.push(&text, at);
            self.composer.update(cx, |composer, cx| composer.clear(cx));
            self.focus_composer(window, cx);
            cx.notify();
            return;
        }
        if self.ask(&text, None, cx) {
            self.composer.update(cx, |composer, cx| composer.clear(cx));
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Asks Kerf `text`, stamped `at` (the playhead time it was written at;
    /// None for now).
    fn ask(&mut self, text: &str, at: Option<f64>, cx: &mut Context<Self>) -> bool {
        let sent = self.session.update(cx, |session, cx| {
            let at = at.unwrap_or(session.state.position);
            let sent = session.ask_at(text, at);
            cx.notify();
            sent
        });
        if sent {
            self.scroll.scroll_to_bottom();
        }
        sent
    }

    /// Asks Kerf `text` about what `about` went about
    /// ([`crate::session::Session::ask_as`]).
    fn ask_as(&mut self, text: &str, about: Focused, cx: &mut Context<Self>) -> bool {
        let sent = self.session.update(cx, |session, cx| {
            let sent = session.ask_as(text, about);
            cx.notify();
            sent
        });
        if sent {
            self.scroll.scroll_to_bottom();
        }
        sent
    }

    /// Sends `text` again about what it was about the first time (the Ask
    /// popover's clip or scope, or the sequence frame it was asked at,
    /// [`crate::session::Session::ask_again`]): at once, or once the turn
    /// under way ends, still about that ([`Queued::push_again`]).
    pub(super) fn ask_again(&mut self, text: String, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if session.state.work.busy() {
            let at = session.state.position;
            let about = session.asked().of(&text).cloned();
            self.queued.push_again(&text, at, about);
        } else {
            let sent = self.session.update(cx, |session, cx| {
                let sent = session.ask_again(&text);
                cx.notify();
                sent
            });
            if sent {
                self.scroll.scroll_to_bottom();
            }
        }
        cx.notify();
    }

    /// Stops Kerf's turn: the work row says "Stopping…" until it ends, and
    /// what waits goes back in the composer then.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.session.read(cx).state.work.busy() {
            self.stopping = true;
        }
        self.session.update(cx, |session, _| session.stop());
        cx.notify();
    }

    /// The turn ended: what waited goes now, unless the turn was stopped or
    /// failed, when it goes back in the composer to send or change.
    pub(super) fn turn_ended(&mut self, cx: &mut Context<Self>) {
        let stopped = std::mem::take(&mut self.stopping);
        self.clock = None;
        let about = self.queued.about().cloned();
        let Some((text, at)) = self.queued.take() else {
            cx.notify();
            return;
        };
        // A stopped turn says "completed" once it ends: `stopping` keeps it.
        let troubled = {
            let session = self.session.read(cx);
            stopped
                || matches!(session.state.work.state.as_str(), "aborted" | "failed")
                || session.state.connection != Connection::Connected
        };
        // An Ask again that waited alone goes about what it went about.
        let sent = !troubled
            && match about {
                Some(about) => self.ask_as(&text, about, cx),
                None => self.ask(&text, Some(at), cx),
            };
        if sent {
            cx.notify();
            return;
        }
        let written = self.composer.read(cx).text().to_owned();
        let back = join_queued(Some(&text), &written);
        self.composer
            .update(cx, |composer, cx| composer.set_text(back, cx));
        cx.notify();
    }
}
