//! The sidebar: the project's conversations, the new one, a search over
//! them, the archived ones folded at the end, and at its foot the account,
//! which opens the app's menu, and Settings.
//!
//! A conversation Kerf works in has a band of light passing over its title
//! once each `TITLE_SHIMMER`, then resting; under Reduce Motion the title is
//! simply in ink. Done while you looked elsewhere, waiting for you, or
//! failed: a dot at the row's end, there at once. A conversation new to the
//! list fades in where it is; archived conversations fade in where they
//! are, one after another, as they unfold. A title is renamed where it is
//! (Rename… in its menu, or F2), and deleting one asks first, over the
//! window.

use crate::commands::RunCommand;
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::model::{Conversation, ConversationSummary, Proposal, Speaker};
use crate::motion;
use crate::session::Session;
use crate::store::Change;
use crate::theme::{self, layout, motion as scene, text};
use crate::ui::{self, icon_button};
use gpui::{
    deferred, div, linear_color_stop, linear_gradient, prelude::*, px, AnimationExt, App, Context,
    Div, ElementId, Entity, EventEmitter, Focusable, KeyDownEvent, MouseButton, MouseDownEvent,
    SharedString, Stateful, Subscription, Window,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub enum SidebarEvent {
    Settings,
    Hide,
    /// Delete this conversation, once the person has said so.
    Delete(String),
    /// A field the sidebar had the keys in is done: the window takes them back.
    Done,
}

impl EventEmitter<SidebarEvent> for SidebarView {}

/// How many of a long list show until "Show N more" is pressed.
pub const SHOWN_AT_FIRST: usize = 6;

/// How a conversation stands, as the dot at its row's end says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    /// Kerf works in it: its title has the light over it.
    Working,
    /// The open conversation has a proposal waiting for you.
    Review,
    /// The open conversation's last turn failed.
    Failed,
    /// A turn ended in it while you looked elsewhere.
    Done,
    Idle,
}

/// How the conversation `id` stands. Waiting and failing are said only of
/// the open one (`chosen`, as `open` holds it): a proposal of one of its
/// turns still pending, else its last answer an error.
pub fn row_state(
    id: &str,
    chosen: Option<&str>,
    working: bool,
    open: Option<&Conversation>,
    proposals: &[Proposal],
    settled: &[String],
) -> RowState {
    if working {
        return RowState::Working;
    }
    if let Some(open) = open.filter(|open| chosen == Some(id) && open.id == id) {
        let pending = |proposal: &str| {
            proposals
                .iter()
                .any(|known| known.proposal_id == proposal && known.is_pending())
        };
        if open
            .items
            .iter()
            .any(|turn| turn.proposal.as_deref().is_some_and(pending))
        {
            return RowState::Review;
        }
        let failed = open
            .items
            .iter()
            .rev()
            .find(|turn| turn.kind == Speaker::Kerf)
            .is_some_and(|turn| turn.error);
        if failed {
            return RowState::Failed;
        }
    }
    if settled.iter().any(|done| done == id) {
        RowState::Done
    } else {
        RowState::Idle
    }
}

/// Whether a title is found by what is typed in Search: anywhere in it,
/// ignoring case. Nothing typed finds every one.
pub fn matches(title: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty() || title.to_lowercase().contains(&query.to_lowercase())
}

/// The title a rename keeps: what was typed, its spaces run together and
/// trimmed. None when it is empty or the same as before.
pub fn renamed(old: &str, typed: &str) -> Option<String> {
    let title = typed.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty() && title != old).then_some(title)
}

/// The row after a long list's first few.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum More {
    /// "Show N more".
    Show(usize),
    /// "Show less".
    Less,
}

/// How many of `total` conversations show, and the row after them. A long
/// list shows its first [`SHOWN_AT_FIRST`] unless the rest were asked for,
/// it is searched, or the open one (at `open_at`) is past them.
pub fn shown(
    total: usize,
    searching: bool,
    show_all: bool,
    open_at: Option<usize>,
) -> (usize, Option<More>) {
    if total <= SHOWN_AT_FIRST || searching {
        return (total, None);
    }
    let mut count = total;
    if !show_all {
        count = SHOWN_AT_FIRST;
        if open_at.is_some_and(|at| at >= SHOWN_AT_FIRST) {
            count = total;
        }
    }
    let all = count == total;
    let more = if !all {
        Some(More::Show(total - count))
    } else if show_all {
        Some(More::Less)
    } else {
        None
    };
    (count, more)
}

/// Seconds since 1970 of an RFC 3339 time ("2026-11-20T03:49:08Z",
/// "2026-11-20T12:49:08.5+09:00"); None when it does not read.
pub fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = text.get(range)?;
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let bytes = text.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let zone = &rest[1..];
            if zone.len() != 5 || zone.as_bytes()[2] != b':' {
                return None;
            }
            let hours: i64 = zone[..2].parse().ok()?;
            let minutes: i64 = zone[3..].parse().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146_097 + of_cycle - 719_468
}

/// The date of a day since 1970-01-01: year, month, day.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
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
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// How long ago `when` was at `now` (both seconds since 1970): "Now",
/// "4m", "2h", "3d", then "Mar 4" this year and "Mar 2025" before.
pub fn relative_time(when: i64, now: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let seconds = (now - when).max(0);
    if seconds < 60 {
        return "Now".into();
    }
    if seconds < 3600 {
        return format!("{}m", seconds / 60);
    }
    if seconds < 86_400 {
        return format!("{}h", seconds / 3600);
    }
    if seconds < 7 * 86_400 {
        return format!("{}d", seconds / 86_400);
    }
    let (year, month, day) = civil_from_days(when.div_euclid(86_400));
    let (this_year, _, _) = civil_from_days(now.div_euclid(86_400));
    let month = MONTHS[(month - 1).clamp(0, 11) as usize];
    if year == this_year {
        format!("{month} {day}")
    } else {
        format!("{month} {year}")
    }
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

// ------------------------------------------------------------ menus

/// What opens from a press: a panel of items a step up from what it opens over.
pub fn menu_panel() -> Div {
    div()
        .p(px(4.0))
        .rounded(px(theme::radius::CARD))
        .bg(theme::popup())
        .border_1()
        .border_color(theme::line2())
        .shadow_lg()
        .occlude()
}

/// One item of a menu: its icon (or the room for one), its words. What
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
        .h(px(30.0))
        .px(px(10.0))
        .rounded(px(theme::radius::CONTROL))
        .text_color(ink)
        .when(!disabled, |item| {
            item.cursor_pointer()
                .hover(|item| item.bg(theme::popup_hover()))
        })
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

/// The app's menu, as the account at the sidebar's foot (or the toolbar's
/// ⋯ while the sidebar is away) opens it: whose account it is, then
/// Settings, the account, the help, and Quit. Each runs its command;
/// `close` puts the menu away first.
pub fn app_menu(header: SharedString, close: Rc<dyn Fn(&mut Window, &mut App)>) -> Div {
    let items: [(&'static str, &'static str, Option<Icon>); 6] = [
        ("app.preferences", "Settings…", Some(Icon::Gear)),
        ("app.account", "Account…", Some(Icon::Lock)),
        ("help.guide", "User Guide", Some(Icon::Search)),
        ("help.crashReports", "Crash Reports", Some(Icon::Folder)),
        ("help.about", "About", Some(Icon::Sparkle)),
        ("app.quit", "Quit", Some(Icon::Close)),
    ];
    let mut menu = menu_panel().w(px(220.0)).child(
        div().px(px(10.0)).pt(px(4.0)).pb(px(6.0)).child(ui::line(
            header,
            text::META,
            theme::ink3(),
        )),
    );
    for (index, (id, words, icon)) in items.into_iter().enumerate() {
        if id == "app.quit" {
            menu = menu.child(menu_separator());
        }
        let close = close.clone();
        menu = menu.child(
            menu_item(
                ElementId::Name(format!("app-menu-{index}").into()),
                words,
                icon,
                false,
                false,
            )
            .on_click(move |_, window, cx| {
                close(window, cx);
                window.dispatch_action(Box::new(RunCommand { id: id.into() }), cx);
            }),
        );
    }
    menu
}

/// A press that toggles what it opens, taken before anything under the
/// pointer has it, so the menu it closes does not take it as a press
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

// ------------------------------------------------------------ the view

pub struct SidebarView {
    session: Entity<Session>,
    archived_open: bool,
    /// How many times the archived ones were unfolded: each unfolding
    /// staggers them in afresh.
    unfoldings: u64,
    /// The row whose menu is open.
    menu: Option<String>,
    /// The account's menu is open.
    app_menu: bool,
    shimmer_since: Instant,
    search: Entity<TextInput>,
    /// The Search row is a field.
    searching: bool,
    query: String,
    rename: Entity<TextInput>,
    /// The conversation whose title is being edited.
    renaming: Option<String>,
    /// A long list shows all of itself.
    show_all: bool,
    /// Every conversation the list has held, and whether it has been
    /// listed at all: what was there at first shows at once.
    seen: HashSet<String>,
    listed: bool,
    /// Conversations new to the list, and when they came.
    arrivals: HashMap<String, Instant>,
    _subscriptions: Vec<Subscription>,
}

impl SidebarView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx, "Search").with_style(text::SIDE));
        let rename = cx.new(|cx| TextInput::new(cx, "Conversation title").with_style(text::SIDE));
        let search_focus = search.read(cx).focus_handle(cx);
        let rename_focus = rename.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, change: &Change, cx| {
                if *change == Change::Conversations {
                    this.note_arrivals(cx);
                }
            }),
            cx.subscribe_in(&search, window, |this, field, event: &InputEvent, _, cx| {
                match event {
                    InputEvent::Changed => {
                        this.query = field.read(cx).text().trim().to_owned();
                        cx.notify();
                    }
                    // Esc empties the field, then gives way to the Search row.
                    InputEvent::Cancel => this.search_escape(cx),
                    InputEvent::Submit => {}
                }
            }),
            // Left empty, the field gives way to the Search row again.
            cx.on_blur(&search_focus, window, |this, _, cx| {
                if this.search.read(cx).text().trim().is_empty() {
                    this.searching = false;
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &rename,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Submit => this.finish_rename(true, true, cx),
                    InputEvent::Cancel => this.finish_rename(false, true, cx),
                    InputEvent::Changed => {}
                },
            ),
            // Leaving the field keeps what was typed.
            cx.on_blur(&rename_focus, window, |this, _, cx| {
                this.finish_rename(true, false, cx)
            }),
        ];
        let mut sidebar = Self {
            session,
            archived_open: false,
            unfoldings: 0,
            menu: None,
            app_menu: false,
            shimmer_since: Instant::now(),
            search,
            searching: false,
            query: String::new(),
            rename,
            renaming: None,
            show_all: false,
            seen: HashSet::new(),
            listed: false,
            arrivals: HashMap::new(),
            _subscriptions: subscriptions,
        };
        sidebar.note_arrivals(cx);
        sidebar
    }

    /// Remembers the conversations listed; those new to a list already
    /// shown fade in.
    fn note_arrivals(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self
            .session
            .read(cx)
            .state
            .conversations
            .iter()
            .map(|conversation| conversation.id.clone())
            .collect();
        let first = !self.listed;
        let now = Instant::now();
        for id in ids {
            if self.seen.insert(id.clone()) && !first {
                self.arrivals.insert(id, now);
            }
        }
        if self.session.read(cx).state.has_snapshot || !self.seen.is_empty() {
            self.listed = true;
        }
    }

    /// Edits the title of conversation `id` where it is.
    pub fn begin_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self
            .session
            .read(cx)
            .state
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| conversation.title.clone())
        else {
            return;
        };
        if self.renaming.is_some() {
            self.finish_rename(true, false, cx);
        }
        self.menu = None;
        self.renaming = Some(id.to_owned());
        self.rename
            .update(cx, |field, cx| field.set_text(title, cx));
        self.rename.read(cx).focus(window);
        // The whole title is chosen, to type over, once the field is drawn.
        cx.on_next_frame(window, |_, window, cx| {
            cx.on_next_frame(window, |this, window, cx| {
                if this.renaming.is_some() {
                    window.dispatch_action(Box::new(crate::input::SelectAll), cx);
                }
            });
        });
        cx.notify();
    }

    /// Ends a rename: `keep` sends the new title (an empty or unchanged one
    /// is not sent); `give_back` hands the keys back to the window.
    fn finish_rename(&mut self, keep: bool, give_back: bool, cx: &mut Context<Self>) {
        let Some(id) = self.renaming.take() else {
            return;
        };
        if keep {
            let typed = self.rename.read(cx).text().to_owned();
            let old = self
                .session
                .read(cx)
                .state
                .conversations
                .iter()
                .find(|conversation| conversation.id == id)
                .map(|conversation| conversation.title.clone());
            if let Some(title) = old.and_then(|old| renamed(&old, &typed)) {
                self.session
                    .update(cx, |session, _| session.rename_conversation(&id, &title));
            }
        }
        if give_back {
            cx.emit(SidebarEvent::Done);
        }
        cx.notify();
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = true;
        self.search.read(cx).focus(window);
        cx.notify();
    }

    /// Esc in Search empties it, then gives way to the Search row.
    fn search_escape(&mut self, cx: &mut Context<Self>) {
        if self.search.read(cx).text().is_empty() {
            self.searching = false;
            cx.emit(SidebarEvent::Done);
            cx.notify();
        } else {
            self.search.update(cx, |field, cx| field.clear(cx));
        }
    }

    fn row(
        &self,
        conversation: &ConversationSummary,
        index: usize,
        state: RowState,
        now: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let session = self.session.read(cx);
        let chosen = session.conversation.as_deref() == Some(conversation.id.as_str());
        let working = state == RowState::Working;
        let id = conversation.id.clone();
        let menu_open = self.menu.as_deref() == Some(conversation.id.as_str());
        let editing = self.renaming.as_deref() == Some(conversation.id.as_str());
        let style = if chosen { text::SIDE_NAME } else { text::SIDE };
        let title: SharedString = conversation.label().to_owned().into();
        // The whole title, and when it last changed, a hover away.
        let tip: SharedString = match parse_time(&conversation.updated) {
            Some(when) => format!("{title} · {}", relative_time(when, now)).into(),
            None => title.clone(),
        };

        let mut words = ui::line(
            title,
            style,
            if chosen { theme::ink() } else { theme::ink2() },
        )
        .flex_1()
        .min_w_0();
        if working && !motion::reduced() {
            // A band of light passing over the title, then resting.
            let phase = motion::phase(self.shimmer_since, scene::TITLE_SHIMMER);
            window.request_animation_frame();
            if let Some(at) = motion::shimmer_at(phase, scene::TITLE_CROSS) {
                let light = theme::ink();
                let rest = theme::with_alpha(theme::ink2(), 0.0);
                words = words.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(gpui::relative(at - 0.15))
                        .w(gpui::relative(0.3))
                        .bg(linear_gradient(
                            90.0,
                            linear_color_stop(rest, 0.0),
                            linear_color_stop(theme::with_alpha(light, 0.22), 0.5),
                        ))
                        .opacity(0.9),
                );
                words = words.relative();
            }
        } else if working {
            words = words.text_color(theme::ink());
        }
        let words = if editing {
            // The field takes the title's place, its text where the title was.
            div()
                .flex_1()
                .min_w_0()
                .h(px(26.0))
                .flex()
                .items_center()
                .ml(px(-5.0))
                .px(px(4.0))
                .rounded(px(theme::radius::SMALL))
                .bg(theme::card())
                .border_1()
                .border_color(theme::accent())
                // Esc reaches the field's holder as a key, whatever the
                // keymap leaves bound in fields.
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.finish_rename(false, true, cx);
                    }
                }))
                .child(self.rename.clone())
                .into_any_element()
        } else {
            words.into_any_element()
        };
        let dot = match state {
            RowState::Working => Some(theme::activity::kerf()),
            RowState::Review | RowState::Done => Some(theme::accent_ink()),
            RowState::Failed => Some(theme::neg()),
            RowState::Idle => None,
        };

        let row = div()
            .id(ElementId::Name(format!("conversation-{id}").into()))
            .group("row")
            .relative()
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(theme::height::H32))
            .px(px(10.0))
            .mx(px(8.0))
            .rounded(px(theme::radius::CONTROL))
            .cursor_pointer()
            .when(chosen, |row| row.bg(theme::on()))
            .when(!chosen, |row| row.hover(|row| row.bg(theme::raised())))
            .when(!editing, |row| {
                let tip = tip.clone();
                row.tooltip(move |_, cx| cx.new(|_| ui::Tooltip(tip.clone())).into())
            })
            .child(words)
            .when_some(dot.filter(|_| !editing), |row, color| {
                row.child(ui::dot(color, 6.0))
            })
            .when(!editing, |row| {
                row.child(
                    div()
                        .invisible()
                        .group_hover("row", |more| more.visible())
                        .when(menu_open, |more| more.visible())
                        .child(
                            trigger(
                                ElementId::Name(format!("more-{id}").into()),
                                cx.listener({
                                    let id = id.clone();
                                    move |this, _: &MouseDownEvent, _, cx| {
                                        this.menu = if this.menu.as_deref() == Some(id.as_str()) {
                                            None
                                        } else {
                                            Some(id.clone())
                                        };
                                        cx.notify();
                                    }
                                }),
                            )
                            .child(
                                icon_button(
                                    ElementId::Name(format!("more-button-{id}").into()),
                                    Icon::More,
                                )
                                .size(22.0),
                            ),
                        ),
                )
            })
            // A right-click opens its menu too.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener({
                    let id = id.clone();
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.menu = Some(id.clone());
                        cx.notify();
                    }
                }),
            )
            .on_click(cx.listener({
                let id = id.clone();
                move |this, _, _, cx| {
                    if this.renaming.as_deref() == Some(id.as_str()) {
                        return;
                    }
                    this.menu = None;
                    this.session.update(cx, |session, cx| {
                        session.open_conversation(Some(id.clone()));
                        cx.notify();
                    });
                }
            }));
        let archived = conversation.archived;
        let with_menu = div().relative().child(row).when(menu_open, |wrapper| {
            // Over the rows after it, and past the list's edge.
            wrapper.child(deferred(self.row_menu(&id, archived, working, cx)).with_priority(1))
        });
        let arrived = self
            .arrivals
            .get(&id)
            .filter(|at| at.elapsed() < scene::APPEAR)
            .copied();
        if archived {
            // Unfolding, each fades in where it is, one after another.
            let delay = scene::UNFOLD_STAGGER.as_secs_f32() * index as f32;
            let total = scene::APPEAR.as_secs_f32() + delay;
            let key = self.unfoldings;
            with_menu
                .with_animation(
                    ElementId::Name(format!("unfold-{id}-{key}").into()),
                    motion::animation(std::time::Duration::from_secs_f32(total)),
                    move |row, t| {
                        let start = if total > 0.0 { delay / total } else { 0.0 };
                        let local = ((t - start) / (1.0 - start).max(0.001)).clamp(0.0, 1.0);
                        row.opacity(local)
                    },
                )
                .into_any_element()
        } else if arrived.is_some() {
            // New to the list: it fades in where it is.
            with_menu
                .with_animation(
                    ElementId::Name(format!("arrive-{id}").into()),
                    motion::animation(scene::APPEAR),
                    |row, t| row.opacity(t),
                )
                .into_any_element()
        } else {
            with_menu.into_any_element()
        }
    }

    fn row_menu(
        &self,
        id: &str,
        archived: bool,
        working: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rename_id = id.to_owned();
        let archive_id = id.to_owned();
        let delete_id = id.to_owned();
        menu_panel()
            .absolute()
            .top(px(30.0))
            .right(px(8.0))
            .w(px(180.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.menu = None;
                cx.notify();
            }))
            .child(
                menu_item(
                    ElementId::Name(format!("rename-{id}").into()),
                    "Rename…",
                    None,
                    false,
                    false,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.begin_rename(&rename_id, window, cx);
                })),
            )
            .child(
                menu_item(
                    ElementId::Name(format!("archive-{id}").into()),
                    if archived { "Unarchive" } else { "Archive" },
                    Some(Icon::Archive),
                    false,
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.menu = None;
                    this.session.update(cx, |session, _| {
                        session.archive_conversation(&archive_id, !archived)
                    });
                })),
            )
            .child(menu_separator())
            .child(
                // Not while Kerf works in it; asked first, over the window.
                menu_item(
                    ElementId::Name(format!("delete-{id}").into()),
                    "Delete…",
                    Some(Icon::Trash),
                    true,
                    working,
                )
                .when(!working, |item| {
                    item.on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        cx.emit(SidebarEvent::Delete(delete_id.clone()));
                        cx.notify();
                    }))
                }),
            )
            .with_animation("row-menu", motion::animation(scene::FADE), |menu, t| {
                menu.opacity(t)
            })
    }

    fn search_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(theme::height::H32))
            .px(px(10.0))
            .rounded(px(theme::radius::CONTROL));
        if self.searching || !self.query.is_empty() {
            shell
                .bg(theme::raised())
                .child(Icon::Search.view(16.0, theme::ink3()))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.search_escape(cx);
                    }
                }))
                .child(div().flex_1().min_w_0().child(self.search.clone()))
                .into_any_element()
        } else {
            shell
                .id("search-conversations")
                .cursor_pointer()
                .hover(|row| row.bg(theme::raised()))
                .child(Icon::Search.view(16.0, theme::ink()))
                .child(ui::line("Search", text::SIDE, theme::ink()))
                .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx)))
                .into_any_element()
        }
    }
}

impl Render for SidebarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.arrivals.retain(|_, at| at.elapsed() < scene::APPEAR);
        let session = self.session.read(cx);
        let project = session.project_name();
        let new_chosen = session.conversation.is_none();
        let chosen = session.conversation.clone();
        let conversations = session.state.conversations.clone();
        let query = self.query.clone();
        let (open, archived): (Vec<_>, Vec<_>) = conversations
            .into_iter()
            .filter(|conversation| matches(conversation.label(), &query))
            .partition(|c| !c.archived);
        let busy = session.state.work.busy();
        let working_in = session.state.work.conversation.clone();
        let open_conversation = session.state.open_conversation.clone();
        let proposals = session.state.snapshot.proposals.clone();
        let settled = session.state.settled.clone();
        let connection = session.state.connection.clone();
        let provider = session.prefs.provider.label();
        // The other folders worked on here, newest first.
        let current = session.project.clone();
        let others: Vec<std::path::PathBuf> = session
            .prefs
            .recent
            .iter()
            .filter(|project| Some(*project) != current.as_ref() && project.is_dir())
            .take(6)
            .cloned()
            .collect();
        let state_of = |conversation: &ConversationSummary| {
            let id = conversation.id.as_str();
            let working = busy
                && (working_in.as_deref() == Some(id)
                    || (working_in.is_none() && chosen.as_deref() == Some(id)));
            row_state(
                id,
                chosen.as_deref(),
                working,
                open_conversation.as_ref(),
                &proposals,
                &settled,
            )
        };
        let now = now_seconds();

        let open_at = open
            .iter()
            .position(|conversation| chosen.as_deref() == Some(conversation.id.as_str()));
        let (count, more) = shown(open.len(), !query.is_empty(), self.show_all, open_at);
        let rows: Vec<_> = open
            .iter()
            .take(count)
            .enumerate()
            .map(|(index, conversation)| {
                let state = state_of(conversation);
                self.row(conversation, index, state, now, window, cx)
                    .into_any_element()
            })
            .collect();
        let archived_rows: Vec<_> = if self.archived_open {
            archived
                .iter()
                .enumerate()
                .map(|(index, conversation)| {
                    let state = state_of(conversation);
                    self.row(conversation, index, state, now, window, cx)
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        let archived_count = archived.len();
        let nothing_found = !query.is_empty() && open.is_empty() && archived.is_empty();

        let (status_color, status) = match &connection {
            crate::store::Connection::Connected => (theme::pos(), "Connected".to_owned()),
            crate::store::Connection::Connecting => (theme::warn(), "Connecting…".to_owned()),
            crate::store::Connection::Failed(message) => (theme::neg(), message.clone()),
        };
        let app_menu_open = self.app_menu;
        let close_app_menu: Rc<dyn Fn(&mut Window, &mut App)> = {
            let this = cx.entity().downgrade();
            Rc::new(move |_, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.app_menu = false;
                    cx.notify();
                });
            })
        };

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::sidebar())
            .border_r_1()
            .border_color(theme::line())
            .child(
                div()
                    .h(px(layout::TOOLBAR))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(8.0))
                    .gap(px(4.0))
                    .child(
                        icon_button("hide-sidebar", Icon::Sidebar)
                            .tooltip("Hide sidebar")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::Hide))),
                    )
                    .child(div().flex_1()),
            )
            .child(
                div()
                    .px(px(8.0))
                    .pb(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .id("new-conversation")
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .h(px(theme::height::H32))
                            .px(px(10.0))
                            .rounded(px(theme::radius::CONTROL))
                            .cursor_pointer()
                            .when(new_chosen, |row| row.bg(theme::on()))
                            .when(!new_chosen, |row| row.hover(|row| row.bg(theme::raised())))
                            .child(Icon::Plus.view(16.0, theme::ink()))
                            .child(ui::line("New conversation", text::SIDE, theme::ink()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.update(cx, |session, cx| {
                                    session.open_conversation(None);
                                    cx.notify();
                                });
                            })),
                    )
                    .child(self.search_row(cx)),
            )
            .child(
                div()
                    .px(px(18.0))
                    .pt(px(8.0))
                    .pb(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(Icon::Folder.view(14.0, theme::ink3()))
                    .child(ui::line(project, text::SIDE_NOTE, theme::ink3())),
            )
            .child(
                div()
                    .id("conversations")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(rows)
                    .when_some(more, |list, more| {
                        let words = match more {
                            More::Show(count) => format!("Show {count} more"),
                            More::Less => "Show less".to_owned(),
                        };
                        list.child(
                            div()
                                .id("show-more")
                                .flex()
                                .items_center()
                                .h(px(theme::height::H28))
                                .px(px(10.0))
                                .mx(px(8.0))
                                .rounded(px(theme::radius::CONTROL))
                                .cursor_pointer()
                                .hover(|row| row.bg(theme::raised()))
                                .child(ui::line(words, text::SIDE_NOTE, theme::ink3()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_all = !this.show_all;
                                    cx.notify();
                                })),
                        )
                    })
                    .when(open.is_empty() && query.is_empty(), |list| {
                        list.child(div().px(px(18.0)).py(px(6.0)).child(ui::label(
                            "What you say to Kerf stays here.",
                            text::SIDE_NOTE,
                            theme::ink3(),
                        )))
                    })
                    .when(nothing_found, |list| {
                        list.child(div().px(px(18.0)).py(px(6.0)).child(ui::label(
                            "No conversations match",
                            text::SIDE_NOTE,
                            theme::ink3(),
                        )))
                    })
                    .when(!others.is_empty() && query.is_empty(), |list| {
                        list.child(div().px(px(18.0)).pt(px(18.0)).pb(px(4.0)).child(ui::label(
                            "Other folders",
                            text::META,
                            theme::ink3(),
                        )))
                        .children(others.into_iter().enumerate().map(|(index, project)| {
                            let name = project
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            div()
                                .id(ElementId::Name(format!("folder-{index}").into()))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .h(px(theme::height::H28))
                                .px(px(10.0))
                                .mx(px(8.0))
                                .rounded(px(theme::radius::CONTROL))
                                .cursor_pointer()
                                .hover(|row| row.bg(theme::raised()))
                                .child(Icon::Folder.view(14.0, theme::ink3()))
                                .child(ui::line(name, text::SIDE_NOTE, theme::ink2()))
                                .on_click(move |_, window, cx| {
                                    // Another folder opens in a window of its own, in place of this one.
                                    crate::app::open(Some(project.clone()), None, cx);
                                    window.remove_window();
                                })
                        }))
                    })
                    .when(archived_count > 0, |list| {
                        list.child(
                            div()
                                .id("archived")
                                .mt(px(10.0))
                                .mx(px(8.0))
                                .px(px(10.0))
                                .h(px(28.0))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .rounded(px(theme::radius::CONTROL))
                                .cursor_pointer()
                                .hover(|row| row.bg(theme::raised()))
                                .child(
                                    if self.archived_open {
                                        Icon::ChevronDown
                                    } else {
                                        Icon::ChevronRight
                                    }
                                    .view(12.0, theme::ink3()),
                                )
                                .child(ui::line(
                                    format!("Archived · {archived_count}"),
                                    text::SIDE_NOTE,
                                    theme::ink3(),
                                ))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.archived_open = !this.archived_open;
                                    this.unfoldings += 1;
                                    cx.notify();
                                })),
                        )
                        .children(archived_rows)
                    }),
            )
            // The account's menu, over the list just above the foot; drawn
            // before the foot, so a press on the foot is the foot's.
            .when(app_menu_open, |sidebar| {
                sidebar.child(
                    div()
                        .absolute()
                        .left(px(8.0))
                        .bottom(px(layout::SIDEBAR_FOOT + 4.0))
                        .child(
                            app_menu(provider.into(), close_app_menu.clone()).on_mouse_down_out(
                                cx.listener(|this, _, _, cx| {
                                    this.app_menu = false;
                                    cx.notify();
                                }),
                            ),
                        )
                        .with_animation("app-menu", motion::animation(scene::FADE), |menu, t| {
                            menu.opacity(t)
                        }),
                )
            })
            .child(
                div()
                    .h(px(layout::SIDEBAR_FOOT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(6.0))
                    .border_t_1()
                    .border_color(theme::line())
                    .child(
                        // The account: pressed, the app's menu opens above it.
                        trigger(
                            "account",
                            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                this.app_menu = !this.app_menu;
                                cx.notify();
                            }),
                        )
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .flex_1()
                        .min_w_0()
                        .h(px(36.0))
                        .px(px(6.0))
                        .rounded(px(theme::radius::CONTROL))
                        .cursor_pointer()
                        .when(app_menu_open, |account| account.bg(theme::raised()))
                        .hover(|account| account.bg(theme::raised()))
                        .child(ui::dot(status_color, 7.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(ui::line(provider, text::CONTROL_STRONG, theme::ink2()))
                                .child(ui::line(status, text::META, theme::ink3())),
                        ),
                    )
                    .child(
                        icon_button("settings", Icon::Gear)
                            .tooltip("Settings")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::Settings))),
                    ),
            )
            .on_mouse_down(MouseButton::Right, |_, _, _| {})
    }
}
