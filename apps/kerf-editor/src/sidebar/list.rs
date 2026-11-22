//! The project's conversations as the sidebar lists them: New
//! conversation and Search over them, how each stands (the dot at its
//! row's end), how many show ("Show N more"), the other folders, and the
//! archived ones folded at the end; those new to the list fade in.

use super::ages::now_seconds;
use super::keys::ITEM;
use super::{SidebarEvent, SidebarView};
use crate::icons::Icon;
use crate::model::{Conversation, ConversationSummary, Proposal, Speaker, SummaryShow, TurnShow};
use crate::theme::{self, motion as scene, text};
use crate::ui;
use gpui::{div, prelude::*, px, Context, Div, Window};
use std::time::Instant;

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
            .find(|turn| turn.speaker() == Speaker::Kerf)
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

/// What the row after a long list's first few says.
pub(super) fn more_words(more: More) -> String {
    match more {
        More::Show(count) => format!("Show {count} more"),
        More::Less => "Show less".to_owned(),
    }
}

/// A line in the list that says how it stands ("No conversations match").
fn list_note(words: &'static str) -> Div {
    div().px(px(18.0)).py(px(theme::space::S6)).child(ui::label(
        words,
        text::SIDE_NOTE,
        theme::ink3(),
    ))
}

impl SidebarView {
    /// Remembers the conversations listed (`listed`: as the bridge listed
    /// them); those new to a list already shown fade in, and those whose
    /// fade has run out are let go.
    pub(super) fn note_arrivals(&mut self, listed: bool, cx: &mut Context<Self>) {
        self.arrivals.retain(|_, at| at.elapsed() < scene::APPEAR);
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
        if listed || !self.seen.is_empty() {
            self.listed = true;
        }
    }

    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = true;
        self.search.read(cx).focus(window);
        cx.notify();
    }

    /// Esc in Search empties it, then gives way to the Search row.
    pub(super) fn search_escape(&mut self, cx: &mut Context<Self>) {
        if self.search.read(cx).text().is_empty() {
            self.searching = false;
            cx.emit(SidebarEvent::Done);
            cx.notify();
        } else {
            self.search.update(cx, |field, cx| field.clear(cx));
        }
    }

    /// New conversation, and Search under it.
    pub(super) fn top_rows(&self, new_chosen: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(theme::space::S8))
            .pb(px(theme::space::S8))
            .flex()
            .flex_col()
            .gap(px(theme::space::S2))
            .child(
                Self::keyed(div().id("new-conversation"), ITEM)
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
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
                        cx.emit(SidebarEvent::Chosen);
                    })),
            )
            .child(self.search_row(cx))
    }

    fn search_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = div()
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .h(px(theme::height::H32))
            .px(px(10.0))
            .rounded(px(theme::radius::CONTROL));
        if self.searching || !self.query.is_empty() {
            shell
                .bg(theme::raised())
                .child(Icon::Search.view(16.0, theme::ink3()))
                .child(div().flex_1().min_w_0().child(self.search.clone()))
                .into_any_element()
        } else {
            Self::keyed(shell.id("search-conversations"), ITEM)
                .cursor_pointer()
                .hover(|row| row.bg(theme::raised()))
                .child(Icon::Search.view(16.0, theme::ink()))
                .child(ui::line("Search", text::SIDE, theme::ink()))
                .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx)))
                .into_any_element()
        }
    }

    /// The conversations' list under the open folder's name: the open ones
    /// (the first few, unless the rest were asked for), a note when there
    /// are none or none match, the other folders while nothing is searched,
    /// and the archived ones folded at the end. It is drawn again as the age
    /// in a tip turns over.
    pub(super) fn conversations(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
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
        let archived_shown = archived.iter().filter(|_| self.archived_open);
        self.follow_ages(open.iter().chain(archived_shown), now, cx);

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
                self.row(conversation, index, state, now, cx)
                    .into_any_element()
            })
            .collect();
        let archived_rows: Vec<_> = if self.archived_open {
            archived
                .iter()
                .enumerate()
                .map(|(index, conversation)| {
                    let state = state_of(conversation);
                    self.row(conversation, index, state, now, cx)
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        let archived_count = archived.len();
        let nothing_found = !query.is_empty() && open.is_empty() && archived.is_empty();
        let other_rows = self.other_folders(&query, cx);
        div()
            .id("conversations")
            .track_scroll(&self.list_scroll)
            .flex_1()
            .min_h_0()
            .pt(px(theme::space::S4))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(theme::space::S2))
            .children(rows)
            .when_some(more, |list, more| list.child(Self::show_more(more, cx)))
            .when(open.is_empty() && query.is_empty(), |list| {
                list.child(list_note("What you say to Kerf stays here."))
            })
            .when(nothing_found, |list| {
                list.child(list_note("No conversations match"))
            })
            .when(!other_rows.is_empty(), |list| {
                list.child(
                    div()
                        .px(px(18.0))
                        .pt(px(18.0))
                        .pb(px(theme::space::S4))
                        .child(ui::label("Other folders", text::META, theme::ink3())),
                )
                .children(other_rows)
            })
            .when(archived_count > 0, |list| {
                list.child(self.archived_toggle(archived_count, cx))
                    .children(archived_rows)
            })
    }

    /// The row after a long list's first few: pressed, the rest show, or
    /// go again.
    fn show_more(more: More, cx: &Context<Self>) -> impl IntoElement {
        Self::keyed(div().id("show-more"), ITEM)
            .flex()
            .items_center()
            .h(px(theme::height::H28))
            .px(px(10.0))
            .mx(px(theme::space::S8))
            .rounded(px(theme::radius::CONTROL))
            .cursor_pointer()
            .hover(|row| row.bg(theme::raised()))
            .child(ui::line(more_words(more), text::SIDE_NOTE, theme::ink3()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_all = !this.show_all;
                cx.notify();
            }))
    }

    /// "Archived · N" at the list's end: pressed, the archived ones unfold,
    /// or fold.
    fn archived_toggle(&self, count: usize, cx: &Context<Self>) -> impl IntoElement {
        Self::keyed(div().id("archived"), ITEM)
            .mt(px(10.0))
            .mx(px(theme::space::S8))
            .px(px(10.0))
            .h(px(28.0))
            .flex()
            .items_center()
            .gap(px(theme::space::S6))
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
                format!("Archived · {count}"),
                text::SIDE_NOTE,
                theme::ink3(),
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                this.archived_open = !this.archived_open;
                this.unfoldings += 1;
                cx.notify();
            }))
    }
}
