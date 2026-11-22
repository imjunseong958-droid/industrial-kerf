//! What the window says over its rooms: the bars under the toolbar, each
//! until what it says is dealt with (kerfd gone, project.kerf changed on
//! disk, the last error, a failed sync), the toolbar's red line of what
//! stands in the way ([`standing_problem`]), and the notices over the
//! conversation. An error is read as a person reads it ([`readable_error`]),
//! its first line in the bar ([`first_line`]).

use super::Workspace;
use editor_session::session::Session;
use editor_state::store::Connection;
use editor_ui::motion;
use editor_ui::theme::{self, layout, motion as scene, text};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, AnimationExt, AnyElement, App, Context, Rgba, SharedString, Window,
};
use kerf_bridge::Command;

/// The last error, kept in a bar under the toolbar until it is dismissed.
pub(super) struct ErrorBar {
    /// As the bridge said it, to tell its notice from the rest.
    pub(super) raw: String,
    /// As a person reads it.
    pub(super) text: String,
    /// A failed turn's: the next turn takes it away.
    pub(super) from_agent: bool,
    /// A failed sync's ("Sync failed: <why>"): a later sync, started or
    /// done, takes it away.
    pub(super) from_sync: bool,
    /// Its place among the errors, so a new one fades in afresh.
    pub(super) key: u64,
}

impl Workspace {
    /// What stands in the way, as the toolbar's line says it in red
    /// ([`standing_problem`]).
    pub(super) fn standing_problem(&self, session: &Session, cx: &App) -> Option<String> {
        let collab = self.collab.read(cx);
        standing_problem(&StandingInput {
            has_project: session.has_project(),
            connection: &session.state.connection,
            error: self.error.as_ref().map(|error| error.text.as_str()),
            conflicted: collab.conflicted(),
            sync_failure: collab.standing_failure(),
        })
    }

    /// The bars under the toolbar, each until what it says is dealt with:
    /// kerfd gone, project.kerf changed on disk, the last error.
    pub(super) fn bars(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        let mut bars = Vec::new();
        if let (true, Connection::Failed(reason)) =
            (session.has_project(), &session.state.connection)
        {
            bars.push(bar(
                "bar-disconnected".into(),
                theme::neg(),
                format!("Project disconnected. {reason}").into(),
                None,
                vec![ui::button("bar-reconnect", "Reconnect")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.reconnect();
                            cx.notify();
                        })
                    }))
                    .into_any_element()],
                None,
            ));
        }
        if session.state.snapshot.projection_status == "diverged" {
            bars.push(bar(
                "bar-diverged".into(),
                theme::warn(),
                "project.kerf changed on disk.".into(),
                Some(
                    "Load it to bring in edits made outside Kerf (a git pull, a text editor), \
                     or keep the editor's version and write it over the file."
                        .into(),
                ),
                vec![
                    ui::button("bar-keep-editor", "Keep Editor Version")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, _| {
                                session.send(Command::ProjectionRepair);
                            })
                        }))
                        .into_any_element(),
                    ui::button("bar-load-disk", "Load from Disk")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, _| {
                                session.send(Command::ProjectionImport);
                            })
                        }))
                        .into_any_element(),
                ],
                None,
            ));
        }
        if let Some(error) = &self.error {
            // Its first line; the whole of a longer one (an import's ffmpeg
            // arguments and stderr) in its tooltip.
            let line = first_line(&error.text);
            let whole = (line != error.text.trim()).then(|| error.text.clone().into());
            bars.push(bar(
                format!("bar-error-{}", error.key).into(),
                theme::neg(),
                line.to_owned().into(),
                None,
                vec![ui::button("bar-dismiss", "Dismiss")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.error = None;
                        cx.notify();
                    }))
                    .into_any_element()],
                whole,
            ));
        }
        bars
    }

    pub(super) fn notices(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut notices = self.session.read(cx).state.notices.clone();
        // What the error bar holds is said there, not again here.
        if let Some(error) = &self.error {
            notices.retain(|notice| !(notice.error && notice.text == error.raw));
        }
        let bottom = notice_bottom(
            f32::from(window.viewport_size().height),
            self.chat.read(cx).composer_top(),
        );
        div()
            .absolute()
            .bottom(px(bottom))
            .left_0()
            .right_0()
            .px(px(24.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(theme::space::S6))
            .children(notices.into_iter().map(|notice| {
                let color = if notice.error {
                    theme::neg()
                } else {
                    theme::pos()
                };
                let key = notice.id;
                theme::type_style(div(), text::CONTROL)
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .max_w(px(layout::NOTICE_MOST))
                    .px(px(theme::space::S12))
                    .py(px(theme::space::S8))
                    .rounded(px(theme::radius::CARD))
                    .bg(theme::popup())
                    .border_1()
                    .border_color(theme::line2())
                    .shadow_lg()
                    .text_color(theme::ink())
                    .child(div().flex_none().child(ui::dot(color, 6.0)))
                    // Long words (a failed push's ssh line) wrap inside it.
                    .child(div().flex_1().min_w_0().child(notice.text.clone()))
                    .with_animation(
                        ("notice", key),
                        motion::animation(scene::APPEAR),
                        |notice, t| notice.opacity(t),
                    )
            }))
    }
}

/// How far up from the window's foot (a window `height` points tall) the
/// notices stand: `NOTICE_GAP` above the composer with what stands over it
/// (its top at `composer_top`, as last laid out), so that they never cover
/// Kerf's next steps or what waits to be asked; `NOTICE_FOOT` before the
/// composer has been laid out.
pub fn notice_bottom(height: f32, composer_top: Option<f32>) -> f32 {
    match composer_top {
        Some(top) => (height - top).max(0.0) + layout::NOTICE_GAP,
        None => layout::NOTICE_FOOT,
    }
}

/// What the window says when changing a conversation (renaming,
/// archiving, deleting or opening it) failed.
pub fn conversation_error(message: &str) -> String {
    format!("Couldn't change the conversation · {message}")
}

/// What the bridge answers a command sent while kerfd cannot be reached.
pub(super) const DISCONNECTED: &str = "Runtime is disconnected; reconnect to continue";

/// Whether the error bar takes the error `raw` with kerfd's connection as
/// it is: not "Runtime is disconnected" (the bridge's answer to a command
/// sent while kerfd could not be reached) while it cannot be, since the
/// disconnected bar ("Project disconnected", Reconnect) or the toolbar's
/// "Connecting…" says so already.
pub fn bar_takes_error(raw: &str, connection: &Connection) -> bool {
    raw != DISCONNECTED || matches!(connection, Connection::Connected)
}

/// A failed sync as its bar and the toolbar's line say it.
pub fn sync_failed(why: &str) -> String {
    format!("Sync failed: {}", readable_error(why))
}

/// What the toolbar's red line weighs.
pub struct StandingInput<'a> {
    pub has_project: bool,
    pub connection: &'a Connection,
    /// The error bar's words, while it shows.
    pub error: Option<&'a str>,
    /// A merge is unfinished.
    pub conflicted: bool,
    /// Why the last sync failed, while no sync has started since.
    pub sync_failure: Option<&'a str>,
}

/// What stands in the way, as the toolbar's line says it in red, the first of:
/// kerfd gone ("Offline") or coming ("Connecting…"); the error bar's first
/// line; an unfinished merge; a failed sync, after its bar is dismissed, until
/// a later sync starts or succeeds. None with no project, or nothing in the
/// way.
pub fn standing_problem(input: &StandingInput) -> Option<String> {
    if !input.has_project {
        return None;
    }
    match input.connection {
        Connection::Failed(_) => Some("Offline".into()),
        Connection::Connecting => Some("Connecting…".into()),
        Connection::Connected => input
            .error
            .map(|error| first_line(error).to_owned())
            .or_else(|| {
                input
                    .conflicted
                    .then(|| "A merge is unfinished; finish it with git".to_owned())
            })
            .or_else(|| {
                input
                    .sync_failure
                    .map(|why| first_line(&sync_failed(why)).to_owned())
            }),
    }
}

/// A bar's height under the toolbar.
pub(super) const BAR: f32 = theme::height::H40;

/// A bar under the toolbar: its colour in the dot only, the bar and its
/// words neutral; what it says, then its buttons. It fades in where it is.
fn bar(
    id: SharedString,
    color: Rgba,
    title: SharedString,
    detail: Option<SharedString>,
    buttons: Vec<AnyElement>,
    tip: Option<SharedString>,
) -> AnyElement {
    let tip_id = SharedString::from(format!("{id}-tip"));
    let bar = div()
        .id(id.clone())
        .w_full()
        .h(px(BAR))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(theme::space::S8))
        .px(px(theme::space::S12))
        .bg(theme::chrome())
        .border_b_1()
        .border_color(theme::line())
        .child(ui::dot(color, 7.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S8))
                .flex_1()
                .min_w_0()
                .child(
                    ui::line(title, text::CONTROL_STRONG, theme::ink())
                        .min_w_0()
                        .when(detail.is_some(), |title| title.flex_none()),
                )
                .when_some(detail, |row, detail| {
                    row.child(
                        ui::line(detail, text::CONTROL, theme::ink2())
                            .flex_1()
                            .min_w_0(),
                    )
                }),
        )
        .children(buttons)
        .with_animation(id, motion::animation(scene::FADE), |bar, t| bar.opacity(t));
    // The whole of what it says, where its line cuts it.
    match tip {
        Some(tip) => ui::tipped(tip_id, ui::Tip::new(tip), bar).into_any_element(),
        None => bar.into_any_element(),
    }
}

/// The first line of what `text` says, trimmed (the whole of it when it
/// has one): what the error bar and the toolbar's line show of an error.
pub fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

/// Whether an error the bridge sends stays in the error bar: not what a
/// view answers in place (a making, a shot's graph, a recipe, a workflow,
/// the Cast dialog).
pub fn kept_in_bar(command: &str) -> bool {
    !(matches!(
        command,
        "media_make" | "media_carry" | "takes_read" | "cast"
    ) || command.starts_with("recipe")
        || command.starts_with("shot_")
        || command.starts_with("workflow"))
}

/// An error as a person reads it: an engine diagnostic's severity and code
/// ("error[ripple.blocked]: ", "warning[x.y] 3:14: ") are for tools.
pub fn readable_error(message: &str) -> String {
    let Some(rest) = message
        .strip_prefix("error[")
        .or_else(|| message.strip_prefix("warning["))
    else {
        return message.to_owned();
    };
    let Some(close) = rest.find(']') else {
        return message.to_owned();
    };
    let code = &rest[..close];
    if code.is_empty()
        || !code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return message.to_owned();
    }
    let after = &rest[close + 1..];
    // An optional " line:column".
    let position = |text: &str| -> Option<usize> {
        let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
        let tail = text.strip_prefix(' ')?;
        let line = digits(tail);
        let tail = tail.get(line..)?.strip_prefix(':').filter(|_| line > 0)?;
        let column = digits(tail);
        (column > 0).then_some(1 + line + 1 + column)
    };
    let after = match position(after) {
        Some(skip) if after[skip..].starts_with(": ") => &after[skip..],
        _ => after,
    };
    match after.strip_prefix(": ") {
        Some(text) if !text.trim().is_empty() => text.to_owned(),
        _ => message.to_owned(),
    }
}
