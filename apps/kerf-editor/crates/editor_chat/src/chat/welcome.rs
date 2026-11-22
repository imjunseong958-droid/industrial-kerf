//! A conversation not yet begun ([`ChatView::empty`]): where to work (the
//! folders, the open one ticked, Open Folder… and New Project…), then the
//! latest other conversations here, or with none, things to ask
//! ([`try_asking`]).

use super::ChatView;
use editor_session::commands::RunCommand;
use editor_state::model::{self, SummaryShow};
use editor_state::store::NextStep;
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::theme::{self, motion as scene, text};
use editor_ui::ui;
use gpui::{div, prelude::*, px, AnimationExt, AnyElement, Context, ElementId};

/// How wide a new conversation's column of folders and conversations is.
pub use editor_ui::theme::layout::WELCOME_COLUMN;

/// The latest conversations a new one offers to go back to: those not
/// archived and not `open`, newest first, at most `most`.
pub fn recent_conversations<'a>(
    conversations: &'a [model::ConversationSummary],
    open: Option<&str>,
    most: usize,
) -> Vec<&'a model::ConversationSummary> {
    let mut recent: Vec<&model::ConversationSummary> = conversations
        .iter()
        .filter(|summary| !summary.archived && Some(summary.id.as_str()) != open)
        .collect();
    recent.sort_by(|a, b| b.updated.cmp(&a.updated));
    recent.truncate(most);
    recent
}

/// What a new conversation offers to ask: Kerf's next steps (each shows
/// its label and fills the composer with its request), else three requests
/// to start from; at most four.
pub fn try_asking(next_steps: &[NextStep]) -> Vec<NextStep> {
    let offered: Vec<NextStep> = next_steps
        .iter()
        .filter_map(|step| {
            let label = step.label.trim();
            let request = step.request.trim();
            (!label.is_empty())
                .then(|| NextStep::new(label, if request.is_empty() { label } else { request }))
        })
        .take(4)
        .collect();
    if !offered.is_empty() {
        return offered;
    }
    [
        "Make a 30-second cut from the best takes",
        "Tighten the pauses in the interview",
        "Add captions to the whole sequence",
    ]
    .map(NextStep::from)
    .to_vec()
}

impl ChatView {
    /// A conversation not yet begun: where to work (the folders, the open
    /// one ticked, Open Folder… and New Project…), then the latest other
    /// conversations here, or with none, things to ask.
    pub(super) fn empty(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .flex()
            .justify_center()
            .pt(px(96.0))
            .child(
                div()
                    .w(px(WELCOME_COLUMN))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S12))
                    .child(welcome_card(self.folder_rows(cx)))
                    .child(welcome_card(self.second_rows(cx))),
            )
            .with_animation("empty-rise", motion::animation(scene::RISE), |empty, t| {
                empty.opacity(t).mt(px(scene::PAGE_RISE * (1.0 - t)))
            })
            .into_any_element()
    }

    /// The first card's rows: the folders (at most four, the open one
    /// ticked), Open Folder… and New Project….
    fn folder_rows(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        use editor_sidebar::sidebar::Entry;
        let session = self.session.read(cx);
        let folders: Vec<_> = editor_sidebar::sidebar::folder_entries(
            &session.prefs.recent,
            session.project.as_deref(),
        )
        .into_iter()
        .filter(|entry| matches!(entry.entry, Entry::Open(_) | Entry::Command(_)))
        .collect();
        let mut shown_folders = 0;
        let mut folder_rows = Vec::new();
        for (index, entry) in folders.into_iter().enumerate() {
            if matches!(entry.entry, Entry::Open(_)) {
                shown_folders += 1;
                if shown_folders > 4 {
                    continue;
                }
            }
            // No line over the card's first row.
            if entry.separated && !folder_rows.is_empty() {
                folder_rows.push(
                    div()
                        .my(px(theme::space::S4))
                        .child(ui::hairline(true))
                        .into_any_element(),
                );
            }
            let chosen = entry.checked;
            let run = entry.entry.clone();
            let icon = entry.icon;
            let row = welcome_row(
                ElementId::Name(format!("welcome-folder-{index}").into()),
                icon,
                entry.words,
                chosen,
            )
            .on_click(cx.listener(move |_, _, window, cx| match &run {
                Entry::Open(folder) if !chosen => window.dispatch_action(
                    Box::new(editor_session::commands::OpenRecent {
                        path: folder.to_string_lossy().into_owned().into(),
                    }),
                    cx,
                ),
                Entry::Command(id) => window.dispatch_action(Box::new(RunCommand::new(id)), cx),
                _ => {}
            }));
            folder_rows.push(match entry.tip {
                Some(tip) => ui::tipped(
                    ElementId::Name(format!("welcome-folder-tip-{index}").into()),
                    ui::Tip::new(tip),
                    row.w_full(),
                )
                .into_any_element(),
                None => row.into_any_element(),
            });
        }
        folder_rows
    }

    /// The second card's rows: the latest other conversations here, each
    /// opened as it is pressed; with none, things to ask, each filling the
    /// composer with its request.
    fn second_rows(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        let recent: Vec<(String, String)> = recent_conversations(
            &session.state.conversations,
            session.conversation.as_deref(),
            3,
        )
        .into_iter()
        .map(|summary| (summary.id.clone(), summary.label().to_owned()))
        .collect();
        if recent.is_empty() {
            try_asking(&session.state.next_steps)
                .into_iter()
                .enumerate()
                .map(|(index, step)| {
                    let request = step.request;
                    welcome_row(
                        ElementId::Name(format!("suggest-{index}").into()),
                        Some(Icon::Sparkle),
                        step.label,
                        false,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.fill(request.clone(), window, cx)
                    }))
                    .into_any_element()
                })
                .collect()
        } else {
            recent
                .into_iter()
                .map(|(id, title)| {
                    let open = id.clone();
                    let row = welcome_row(
                        ElementId::Name(format!("welcome-conversation-{id}").into()),
                        None,
                        title.clone(),
                        false,
                    )
                    .w_full()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.session.update(cx, |session, cx| {
                            session.open_conversation(Some(open.clone()));
                            cx.notify();
                        });
                        this.focus_composer(window, cx);
                    }));
                    ui::tipped(
                        ElementId::Name(format!("welcome-conversation-tip-{id}").into()),
                        ui::Tip::new(title),
                        row,
                    )
                    .into_any_element()
                })
                .collect()
        }
    }
}

/// One of a new conversation's cards, holding `rows`.
fn welcome_card(rows: Vec<AnyElement>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .p(px(theme::space::S4))
        .rounded(px(theme::radius::PANEL))
        .bg(theme::card())
        .border_1()
        .border_color(theme::line())
        .children(rows)
}

/// A row of a new conversation's cards: an icon where it has one, its
/// words, a tick where it is the one open.
fn welcome_row(
    id: ElementId,
    icon: Option<Icon>,
    words: String,
    ticked: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(theme::space::S8))
        .h(px(32.0))
        .px(px(10.0))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .hover(|row| row.bg(theme::hover2()))
        .when_some(icon, |row, icon| row.child(icon.view(14.0, theme::ink3())))
        .child(
            ui::line(words, text::MENU_QUIET, theme::ink())
                .flex_1()
                .min_w_0(),
        )
        .when(ticked, |row| {
            row.child(Icon::Check.view(14.0, theme::ink2()))
        })
}
