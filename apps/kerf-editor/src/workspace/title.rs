//! The toolbar's title: the open conversation's name, its menu (Rename,
//! Archive, Delete…, Agent settings…, the models, the playhead's stamp,
//! Activity), its rename in place, Activity under it, and its tip saying when
//! the project was edited ([`title_tip`], [`edited_ago`]).

use super::toolbar::toolbar_stop;
use super::Workspace;
use editor_launcher::settings::Page;
use editor_sidebar::sidebar;
use editor_state::model::SummaryShow;
use editor_state::store::Connection;
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, layout, motion as scene, text};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, AnimationExt, AnyElement, ClickEvent, Context, DismissEvent, Div,
    MouseDownEvent, Rgba, Window,
};
use std::time::{Duration, Instant};

impl Workspace {
    /// The toolbar's title: the open conversation's name (the field it
    /// becomes while it is renamed there), pressed for its menu and its tip
    /// saying when the project was edited; after it, what stands in the way,
    /// in red, else the project.
    pub(super) fn title_row(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let session = self.session.read(cx);
        let project = session.project_name();
        // The title is the open conversation's name, the project after it;
        // with none open, "New conversation" (or the first line of the
        // request under way).
        let conversation = session.conversation.as_ref().and_then(|id| {
            session
                .state
                .conversations
                .iter()
                .find(|conversation| &conversation.id == id)
                .map(|conversation| conversation.label().to_owned())
                .or_else(|| {
                    session
                        .state
                        .open_conversation
                        .as_ref()
                        .filter(|conversation| &conversation.id == id)
                        .map(|conversation| conversation.title.trim().to_owned())
                        .filter(|title| !title.is_empty())
                })
        });
        let title = conversation.unwrap_or_else(|| session.state.conversation_title());
        let subtitle: Option<(String, Rgba)> = match self.standing_problem(session, cx) {
            Some(problem) => Some((problem, theme::neg())),
            None => Some((project.clone(), theme::ink3())),
        };
        // The title's tip: the project and its sequence after a
        // conversation's name, when it was edited and at which revision.
        let now = now_seconds();
        let title_tip = title_tip(
            &self.window_title,
            true,
            session.revision(),
            self.revision_time,
            now,
        );
        // Its "Edited 2 min ago" turns over by itself.
        if let Some(next) = self
            .revision_time
            .and_then(|then| edited_ago_changes(then, now))
        {
            let wait = Duration::from_secs(u64::try_from(next - now).unwrap_or(0));
            self.ages.at(Instant::now() + wait, cx);
        }
        let renaming = self.renaming_title.is_some();
        let title_menu = self.title_menu.is_some();
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .pl(px(theme::space::S8))
            .min_w_0()
            .child(if renaming {
                // Renamed where it is named: the field takes the title's place.
                div()
                    .w(px(TITLE_FIELD))
                    .h(px(theme::height::H28))
                    .flex()
                    .items_center()
                    .ml(px(-5.0))
                    .px(px(theme::space::S4))
                    .rounded(px(theme::radius::SMALL))
                    .bg(theme::card())
                    .border_1()
                    .border_color(theme::accent())
                    .child(self.title_field.clone())
                    .into_any_element()
            } else {
                self.title_button(title, title_tip, title_menu, window, cx)
            })
            .when_some(subtitle, |row, (words, color)| {
                row.child(ui::line(words, text::CONTROL, color).min_w_0())
            })
    }

    /// The toolbar's `title` as a Tab stop that opens the conversation's
    /// menu (`title_menu`: open now), and its tip, `title_tip`: the project
    /// and when it was edited.
    fn title_button(
        &self,
        title: String,
        title_tip: String,
        title_menu: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        // Pressed, what can be done with the conversation;
        // pressed twice, its name becomes a field.
        let title = toolbar_stop(
            ui::capture_trigger(
                "toolbar-title",
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.close_app_menu(window, cx);
                    this.activity_open = false;
                    this.sidebar
                        .update(cx, |sidebar, cx| sidebar.close_menus(cx));
                    if event.click_count >= 2 {
                        this.close_title_menu(window, cx);
                        this.begin_title_rename(window, cx);
                    } else {
                        this.toggle_title_menu(false, window, cx);
                    }
                    cx.notify();
                }),
            ),
            &self.title_stop,
            theme::radius::CONTROL,
            true,
            window,
        )
        // From the keys, its menu.
        .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
            if event.is_keyboard() {
                this.close_app_menu(window, cx);
                this.toggle_title_menu(true, window, cx);
                cx.notify();
            }
        }))
        .flex()
        .items_center()
        .gap(px(theme::space::S4))
        .min_w_0()
        .h(px(theme::height::H28))
        .pl(px(theme::space::S4))
        .pr(px(theme::space::S2))
        .ml(px(-4.0))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .when(title_menu, |title| title.bg(theme::raised()))
        .hover(|title| title.bg(theme::raised()))
        .child(ui::line(title, text::TITLE, theme::ink()))
        .child(Icon::ChevronDown.view(14.0, theme::ink3()));
        // Its tip, over all of it, so that it still gives way
        // in a narrow window; its menu says all the tip
        // would, so the tip waits while it is open.
        title
            .when(!title_menu && !title_tip.is_empty(), |title| {
                title.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .child(ui::tipped(
                            "toolbar-title-tip",
                            ui::Tip::new(title_tip.clone()),
                            div().w_full().h(px(theme::height::H28)),
                        )),
                )
            })
            .into_any_element()
    }

    /// Renames the open conversation where it is named: in the sidebar
    /// while it shows, else in the toolbar.
    pub(super) fn rename_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.session.read(cx).conversation.clone() else {
            return;
        };
        if self.sidebar_open {
            self.sidebar
                .update(cx, |sidebar, cx| sidebar.begin_rename(&id, window, cx));
        } else {
            self.begin_title_rename(window, cx);
        }
    }

    /// The toolbar's title becomes a field holding it.
    fn begin_title_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let Some(id) = session.conversation.clone() else {
            return;
        };
        let Some(title) = session
            .state
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| conversation.title.clone())
        else {
            return;
        };
        self.close_title_menu(window, cx);
        self.renaming_title = Some(id);
        // The whole title is chosen, to type over.
        self.title_field.update(cx, |field, cx| {
            field.set_text(title, cx);
            field.choose_all(cx);
        });
        self.title_field.read(cx).focus(window);
        cx.notify();
    }

    /// Ends the toolbar's rename: `keep` sends the new title (an empty or
    /// unchanged one is not sent).
    pub(super) fn finish_title_rename(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(id) = self.renaming_title.take() else {
            return;
        };
        if keep {
            let typed = self.title_field.read(cx).text().to_owned();
            let old = self
                .session
                .read(cx)
                .state
                .conversations
                .iter()
                .find(|conversation| conversation.id == id)
                .map(|conversation| conversation.title.clone());
            if let Some(title) = old.and_then(|old| sidebar::renamed(&old, &typed)) {
                self.session
                    .update(cx, |session, _| session.rename_conversation(&id, &title));
            }
        }
        cx.notify();
    }

    /// The conversation's menu, under its title (the kit's): Rename,
    /// Archive, Delete…, Agent settings… with the model in use, the models
    /// to choose between, the playhead's stamp, Activity. Open, it goes.
    fn toggle_title_menu(&mut self, from_keys: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.title_menu.is_some() {
            self.close_title_menu(window, cx);
            return;
        }
        let items = self.title_menu_items(cx);
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx).width(TITLE_MENU_WIDTH);
            if from_keys {
                menu.from_keys()
            } else {
                menu
            }
        });
        cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
            if this.title_menu.as_ref() == Some(&menu) {
                this.title_menu = None;
                cx.notify();
            }
        })
        .detach();
        self.title_menu = Some(menu);
        cx.notify();
    }

    /// Puts the conversation's menu away, the keys back where they were.
    pub(super) fn close_title_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.title_menu.take() {
            menu.update(cx, |menu, cx| menu.close(window, cx));
            cx.notify();
        }
    }

    fn title_menu_items(&self, cx: &mut Context<Self>) -> Vec<ui::MenuItem> {
        let this = cx.entity().downgrade();
        let session = self.session.read(cx);
        let id = session.conversation.clone();
        let connected = session.state.connection == Connection::Connected;
        let summary = id.as_ref().and_then(|id| {
            session
                .state
                .conversations
                .iter()
                .find(|conversation| &conversation.id == id)
        });
        let kept = summary.is_some() && connected;
        let archived = summary.is_some_and(|summary| summary.archived);
        let working = id
            .as_deref()
            .is_some_and(|id| self.conversation_working(id, cx));
        let can_delete = kept && !working;
        // The models the connection offers to choose between (Claude
        // Code's), and the one in use.
        let provider = session.prefs.provider;
        let configured = session.prefs.model().to_owned();
        let model = if configured.is_empty() {
            "default".to_owned()
        } else {
            configured.clone()
        };
        let choices = editor_chat::chat::model_choices(provider, &configured);
        let choosable = choices.len() > 1 && provider == editor_state::prefs::Provider::ClaudeCode;
        // A request carries the playhead's time while the picture is open.
        let stamp_offered = self.panel_open;
        let stamped = session.prefs.send_playhead;
        let mut items = vec![
            ui::MenuItem::new("Rename").disabled(!kept).on_choose({
                let this = this.clone();
                move |window, cx| {
                    let _ = this.update(cx, |this, cx| this.begin_title_rename(window, cx));
                }
            }),
            ui::MenuItem::new(if archived { "Unarchive" } else { "Archive" })
                .icon(Icon::Archive)
                .disabled(!kept)
                .on_choose({
                    let session = self.session.clone();
                    let id = id.clone();
                    move |_, cx| {
                        if let Some(id) = id.clone() {
                            session.update(cx, |session, cx| {
                                session.archive_conversation(&id, !archived);
                                cx.notify();
                            });
                        }
                    }
                }),
            ui::MenuItem::new("Delete…")
                .icon(Icon::Trash)
                .danger()
                .disabled(!can_delete)
                .on_choose({
                    let this = this.clone();
                    let id = id.clone();
                    move |window, cx| {
                        if let Some(id) = id.clone() {
                            let _ = this.update(cx, |this, cx| this.confirm_delete(id, window, cx));
                        }
                    }
                }),
            ui::MenuItem::new("Agent settings…")
                .icon(Icon::Sparkle)
                .separated()
                // The model in use, where a menu writes a key.
                .key(editor_state::prefs::model_label(&model))
                .on_choose({
                    let this = this.clone();
                    move |window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.show_settings(Some(Page::Chat), window, cx)
                        });
                    }
                }),
        ];
        if choosable {
            let rows = choices
                .into_iter()
                .map(|choice| {
                    let session = self.session.clone();
                    ui::MenuItem::new(editor_state::prefs::model_label(&choice))
                        .checked(choice == model)
                        .on_choose(move |_, cx| {
                            session.update(cx, |session, cx| {
                                session.prefs.claude_code_model = choice.clone();
                                session.save_prefs();
                                cx.notify();
                            });
                        })
                })
                .collect();
            items.push(ui::MenuItem::new("Model").submenu(rows));
        }
        if stamp_offered {
            let session = self.session.clone();
            items.push(
                ui::MenuItem::new("Send playhead time")
                    .checked(stamped)
                    .on_choose(move |_, cx| {
                        session.update(cx, |session, cx| {
                            session.prefs.send_playhead = !session.prefs.send_playhead;
                            session.save_prefs();
                            cx.notify();
                        });
                    }),
            );
        }
        items.push(ui::MenuItem::new("Activity").on_choose(move |_, cx| {
            let _ = this.update(cx, |this, cx| {
                this.activity_open = true;
                cx.notify();
            });
        }));
        items
    }

    /// Activity, under the title: the newest of what the window and Kerf
    /// did, a line each. A press outside or Esc puts it away.
    pub(super) fn activity_view(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let lines: Vec<(String, String)> = self
            .session
            .read(cx)
            .activity
            .latest(editor_session::session::ACTIVITY_SHOWN)
            .map(|(kind, text)| (kind.to_uppercase(), text.to_owned()))
            .collect();
        sidebar::menu_panel()
            .w(px(ACTIVITY_WIDTH))
            .px(px(theme::space::S12))
            .py(px(10.0))
            .flex()
            .flex_col()
            .gap(px(theme::space::S6))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.activity_open = false;
                cx.notify();
            }))
            .child(ui::line("Activity", text::META_STRONG, theme::ink3()))
            .when(lines.is_empty(), |panel| {
                panel.child(ui::label(
                    "Nothing yet: imports, exports and previews are listed here.",
                    text::CONTROL,
                    theme::ink3(),
                ))
            })
            .children(lines.into_iter().map(|(kind, words)| {
                div()
                    .flex()
                    .gap(px(theme::space::S8))
                    .child(
                        ui::line(kind, text::MONO_SMALL, theme::ink3())
                            .w(px(ACTIVITY_KIND))
                            .flex_none(),
                    )
                    .child(
                        ui::label(words, text::CONTROL, theme::ink2())
                            .flex_1()
                            .min_w_0(),
                    )
            }))
            .relative()
            .with_animation("activity", motion::animation(scene::FADE), |panel, t| {
                panel.opacity(t).top(px(scene::MENU_RISE * (1.0 - t)))
            })
    }
}

/// Seconds since 1970, now.
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// How long ago `then` was at `now` (seconds since 1970): "just now",
/// "4 min ago", "2 h ago" the same day, "yesterday", "3 days ago", then the
/// date ("4 Mar 2026"). Days, and the date, are the person's own: on the
/// calendar of the system's time zone ([`edited_ago_in`]).
pub fn edited_ago(then: i64, now: i64) -> String {
    edited_ago_in(then, now, &jiff::tz::TimeZone::system())
}

/// [`edited_ago`] on the calendar of `zone`.
pub fn edited_ago_in(then: i64, now: i64, zone: &jiff::tz::TimeZone) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let seconds = (now - then).max(0);
    if seconds < 60 {
        return "just now".into();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes} min ago");
    }
    let hours = minutes / 60;
    let days = local_day(now, zone) - local_day(then, zone);
    if hours < 24 && days == 0 {
        return format!("{hours} h ago");
    }
    if days <= 1 {
        return "yesterday".into();
    }
    if days < 7 {
        return format!("{days} days ago");
    }
    let (year, month, day) = civil(local_day(then, zone));
    format!("{day} {} {year}", MONTHS[(month - 1).clamp(0, 11) as usize])
}

/// When (seconds since 1970) what [`edited_ago`] says of `then` at `now`
/// changes next: at the next minute or hour it counts, or at the next
/// midnight on the system's calendar; None once it names the date.
pub fn edited_ago_changes(then: i64, now: i64) -> Option<i64> {
    edited_ago_changes_in(then, now, &jiff::tz::TimeZone::system())
}

/// [`edited_ago_changes`] on the calendar of `zone`.
pub fn edited_ago_changes_in(then: i64, now: i64, zone: &jiff::tz::TimeZone) -> Option<i64> {
    let seconds = (now - then).max(0);
    if seconds < 3600 {
        return Some(then + (seconds / 60 + 1) * 60);
    }
    let days = local_day(now, zone) - local_day(then, zone);
    if days >= 7 {
        return None;
    }
    let midnight = jiff::Timestamp::from_second(now)
        .ok()
        .and_then(|at| at.to_zoned(zone.clone()).tomorrow().ok())
        .and_then(|tomorrow| tomorrow.start_of_day().ok())
        .map(|midnight| midnight.timestamp().as_second());
    let hour = (days == 0 && seconds < 86_400).then(|| then + (seconds / 3600 + 1) * 3600);
    match (hour, midnight) {
        (Some(hour), Some(midnight)) => Some(hour.min(midnight)),
        (hour, midnight) => hour.or(midnight),
    }
}

/// The day (since 1970-01-01) that `seconds` since 1970 falls on in
/// `zone`: on its calendar, its offset from UTC then (summer time or not)
/// counted in. A time out of the calendar's reach counts as UTC.
pub fn local_day(seconds: i64, zone: &jiff::tz::TimeZone) -> i64 {
    let offset = jiff::Timestamp::from_second(seconds)
        .map(|at| zone.to_offset(at).seconds())
        .unwrap_or(0);
    (seconds + i64::from(offset)).div_euclid(86_400)
}

/// The date of a day since 1970-01-01: year, month, day.
fn civil(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days - era * 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// The toolbar title's tooltip: when the project was edited and at which
/// revision ("Edited 2 min ago · rev 22", "rev 22" with no time), after the
/// project and its sequence (`window_title`) when the title is a
/// conversation's.
pub fn title_tip(
    window_title: &str,
    conversation: bool,
    revision: u64,
    edited: Option<i64>,
    now: i64,
) -> String {
    let about = match edited {
        Some(then) => format!("Edited {} · rev {revision}", edited_ago(then, now)),
        None => format!("rev {revision}"),
    };
    if conversation && !window_title.is_empty() {
        format!("{window_title} · {about}")
    } else {
        about
    }
}

/// The title's menu: room for Agent settings… and the model after it.
const TITLE_MENU_WIDTH: f32 = 260.0;

/// Activity's width, and its column of kinds.
const ACTIVITY_WIDTH: f32 = 380.0;
const ACTIVITY_KIND: f32 = 64.0;
/// The toolbar's title field, while a conversation is renamed there.
const TITLE_FIELD: f32 = 280.0;

/// What opens under the toolbar's title (its menu, Activity), at `left`
/// in the window.
pub(super) fn under_title(left: f32, child: impl IntoElement) -> Div {
    div()
        .absolute()
        .top(px(layout::TOOLBAR - 2.0))
        .left(px(left))
        .child(child)
}
