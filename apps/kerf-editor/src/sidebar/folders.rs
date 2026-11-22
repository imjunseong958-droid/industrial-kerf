//! The folders in the sidebar: the open folder's name over its
//! conversations (pressed, the folders and the ways to another; where a
//! finished export lands), and the other folders worked on here, each
//! opening its project in place of this one, with its menu and its
//! conversations unfolded under it as they were left.

use super::ages::{now_seconds, parse_time, relative_time};
use super::conversation_files::folder_conversations;
use super::entries::Entry;
use super::keys::{Fold, OpenMenu, Unfold, FOLDER_KEYS, ITEM};
use super::list::{more_words, shown, More};
use super::menus::trigger;
use super::{SidebarMenu, SidebarView};
use crate::icons::Icon;
use crate::model::{ConversationSummary, SummaryShow};
use crate::theme::{self, text};
use crate::ui::{self, icon_button};
use gpui::{
    canvas, deferred, div, prelude::*, px, AnyElement, ClickEvent, Context, Div, ElementId,
    MouseButton, MouseDownEvent, SharedString, Stateful,
};
use std::path::{Path, PathBuf};

/// The recent folders with `folder` taken off, however written.
pub fn forget_folder(recent: &[PathBuf], folder: &Path) -> Vec<PathBuf> {
    recent
        .iter()
        .filter(|known| !crate::folders::same_folder(known, folder))
        .cloned()
        .collect()
}

/// A folder's name, else all of its path.
pub(super) fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}

impl SidebarView {
    /// The open folder's name over its conversations, a chevron after it:
    /// pressed, the folders and the ways to another.
    pub(super) fn folder_heading(
        &self,
        project: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open = self.menu_is(&SidebarMenu::Folder);
        let heading = Self::keyed(
            trigger(
                "folder-heading",
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.toggle_menu(SidebarMenu::Folder, false, window, cx)
                }),
            ),
            ITEM,
        )
        .mx(px(8.0))
        .mt(px(4.0))
        .px(px(10.0))
        .h(px(theme::height::H28))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .when(open, |heading| heading.bg(theme::raised()))
        .hover(|heading| heading.bg(theme::raised()))
        .child(Icon::Folder.view(14.0, theme::ink3()))
        .child(ui::line(project, text::SIDE_NOTE, theme::ink3()).min_w_0())
        .child(Icon::ChevronDown.view(12.0, theme::ink3()))
        .relative()
        .child(ui::tip_zone(
            "folder-heading-tip",
            (!open).then(|| ui::Tip::new("Folders")),
        ))
        .on_action(cx.listener(|this, _: &OpenMenu, window, cx| {
            this.open_menu(SidebarMenu::Folder, true, window, cx)
        }))
        .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
            if event.is_keyboard() {
                this.toggle_menu(SidebarMenu::Folder, true, window, cx);
            }
        }));
        let bounds = self.heading_bounds.clone();
        div()
            .relative()
            .child(heading)
            // Where the project's row is, for a finished export to land on.
            .child(
                canvas(
                    move |placed, _, _| bounds.set(Some(placed)),
                    |_, _, _, _| {},
                )
                .absolute()
                .top(px(4.0))
                .left(px(8.0))
                .right(px(8.0))
                .h(px(theme::height::H28)),
            )
            .when(open, |wrapper| {
                wrapper.child(
                    deferred(
                        div()
                            .absolute()
                            .top(px(34.0))
                            .left(px(8.0))
                            .child(self.menu_view(&SidebarMenu::Folder, None, 240.0, cx)),
                    )
                    .with_priority(1),
                )
            })
    }

    /// The other folders worked on here, newest first, at most six, while
    /// nothing is searched (`query`).
    pub(super) fn other_folders(&self, query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        let current = session.project.clone();
        let others: Vec<PathBuf> = session
            .prefs
            .recent
            .iter()
            .filter(|project| {
                !current
                    .as_deref()
                    .is_some_and(|current| crate::folders::same_folder(current, project))
                    && project.is_dir()
            })
            .take(6)
            .cloned()
            .collect();
        if query.is_empty() {
            others
                .into_iter()
                .enumerate()
                .map(|(index, folder)| self.other_folder(index, folder, cx))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Another folder worked on here: pressed, it opens in place of this
    /// project; its ⋯ or a right-click has its menu; its chevron (Right and
    /// Left from the keys) unfolds and folds its conversations, read from
    /// where its project keeps them, each opening its project on it.
    fn other_folder(&self, index: usize, folder: PathBuf, cx: &mut Context<Self>) -> AnyElement {
        let name = folder_name(&folder);
        let menu = SidebarMenu::Other(folder.clone());
        let menu_open = self.menu_is(&menu);
        let unfolded = self.unfolded.get(&folder);
        let chevron = Self::fold_chevron(index, &folder, unfolded.is_some(), cx);
        let row = Self::keyed(
            div().id(ElementId::Name(format!("folder-{index}").into())),
            FOLDER_KEYS,
        )
        .group("folder-row")
        .relative()
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(theme::height::H28))
        .pl(px(6.0))
        .pr(px(10.0))
        .mx(px(8.0))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .hover(|row| row.bg(theme::raised()))
        .when(menu_open, |row| row.bg(theme::raised()))
        .child(chevron)
        .child(Icon::Folder.view(14.0, theme::ink3()))
        .child(
            ui::line(name, text::SIDE_NOTE, theme::ink2())
                .flex_1()
                .min_w_0(),
        )
        // Its path, the kit's tip, under its ⋯ and its chevron.
        .child(ui::tip_zone(
            ElementId::Name(format!("folder-tip-{index}").into()),
            (!menu_open).then(|| ui::Tip::new(folder.display().to_string())),
        ))
        .child(Self::folder_more(index, &menu, menu_open, cx));
        let row = Self::folder_answering(row, &folder, &menu, cx);
        // Nothing under it until its conversations are read.
        let conversations = unfolded
            .and_then(Option::as_deref)
            .map(|conversations| self.folder_conversation_rows(index, &folder, conversations, cx));
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(row)
            .children(conversations)
            .when(menu_open, |wrapper| {
                wrapper.child(self.placed_menu(&menu, 28.0, 200.0, cx))
            })
            .into_any_element()
    }

    /// Another folder's chevron: pressed, its conversations unfold under
    /// it, or fold; its own press is not the row's, which opens the folder.
    fn fold_chevron(
        index: usize,
        folder: &Path,
        unfolded: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let fold = folder.to_path_buf();
        div()
            .id(ElementId::Name(format!("folder-fold-{index}").into()))
            .flex_none()
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme::radius::SMALL))
            .hover(|chevron| chevron.bg(theme::hover2()))
            .child(
                if unfolded {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                }
                .view(12.0, theme::ink3()),
            )
            // Its own press folds and unfolds; the row's opens the folder.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.fold(&fold, None, cx);
            }))
    }

    /// Another folder's ⋯, seen while the pointer is over its row or its
    /// menu is open: pressed, the menu opens, or closes.
    fn folder_more(index: usize, menu: &SidebarMenu, menu_open: bool, cx: &Context<Self>) -> Div {
        div()
            .invisible()
            .group_hover("folder-row", |more| more.visible())
            .when(menu_open, |more| more.visible())
            .child(
                trigger(
                    ElementId::Name(format!("folder-more-{index}").into()),
                    cx.listener({
                        let menu = menu.clone();
                        move |this, _: &MouseDownEvent, window, cx| {
                            this.toggle_menu(menu.clone(), false, window, cx)
                        }
                    }),
                )
                .child(
                    icon_button(
                        ElementId::Name(format!("folder-more-button-{index}").into()),
                        Icon::More,
                    )
                    .size(20.0),
                ),
            )
    }

    /// What another folder's row answers: a right-click opens its menu at
    /// the pointer, Shift+F10 or the Menu key under the row; Right unfolds
    /// its conversations and Left folds them; a press opens the folder in
    /// a window of its own, in place of this one.
    fn folder_answering(
        row: Stateful<Div>,
        folder: &Path,
        menu: &SidebarMenu,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        row.on_mouse_down(
            MouseButton::Right,
            cx.listener({
                let menu = menu.clone();
                move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(menu.clone(), false, window, cx);
                    // At the pointer, kept inside the window.
                    this.menu_at = Some(event.position);
                }
            }),
        )
        .on_action(cx.listener({
            let menu = menu.clone();
            move |this, _: &OpenMenu, window, cx| this.open_menu(menu.clone(), true, window, cx)
        }))
        .on_action(cx.listener({
            let folder = folder.to_path_buf();
            move |this, _: &Unfold, _, cx| this.fold(&folder, Some(false), cx)
        }))
        .on_action(cx.listener({
            let folder = folder.to_path_buf();
            move |this, _: &Fold, _, cx| this.fold(&folder, Some(true), cx)
        }))
        .on_click(cx.listener({
            let folder = folder.to_path_buf();
            move |this, _, window, cx| {
                // Another folder opens in a window of its own, in place of this one.
                this.run_entry(Entry::Open(folder.clone()), false, window, cx)
            }
        }))
    }

    /// `folder`'s conversations, read again (unfolding it, `Some(false)`),
    /// or put away (folding it, `Some(true)`); None: the other way. They
    /// are read away from the window (a folder can keep many long
    /// conversations), and show once they are, unless it was folded again.
    ///
    /// The fold is kept as it was left, so the next launch shows the
    /// sidebar the same way: for the folders still listed only.
    fn fold(&mut self, folder: &Path, fold: Option<bool>, cx: &mut Context<Self>) {
        let folded = !self.unfolded.contains_key(folder);
        let now_folded = fold.unwrap_or(!folded);
        if now_folded {
            self.unfolded.remove(folder);
            self.others_shown_all.remove(folder);
        } else {
            self.unfold_reading(folder, cx);
        }
        self.session.update(cx, |session, _| {
            session.prefs.fold_by_hand(folder, now_folded);
            session.save_prefs();
        });
        cx.notify();
    }

    /// Unfolds `folder`, reading its conversations away from the window;
    /// they show once they are read, unless it was folded again. What was
    /// read before stays until the new reading is in.
    fn unfold_reading(&mut self, folder: &Path, cx: &mut Context<Self>) {
        self.unfolded.entry(folder.to_path_buf()).or_insert(None);
        let folder = folder.to_path_buf();
        let reading = cx.background_executor().spawn({
            let folder = folder.clone();
            async move { folder_conversations(&folder) }
        });
        cx.spawn(async move |this, cx| {
            let read = reading.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(shown) = this.unfolded.get_mut(&folder) {
                    *shown = Some(read);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The other folders left unfolded by hand (here, or in the editor
    /// before 0.1.5 (Qt), whose folds carry over so they open as they were
    /// left), unfolded again as the sidebar is built.
    pub(super) fn unfold_as_left(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let current = session.project.clone();
        let unfolded: Vec<PathBuf> = session
            .prefs
            .recent
            .iter()
            .filter(|folder| {
                !current
                    .as_deref()
                    .is_some_and(|current| crate::folders::same_folder(current, folder))
                    && session.prefs.folder_folded(folder) == Some(false)
                    && folder.is_dir()
            })
            .cloned()
            .collect();
        for folder in unfolded {
            self.unfold_reading(&folder, cx);
        }
    }

    /// Another folder's conversations, unfolded under it: the latest first,
    /// the first few unless the rest were asked for ("Show N more"), "No
    /// conversations yet" with none. Each opens its project on it.
    fn folder_conversation_rows(
        &self,
        index: usize,
        folder: &Path,
        conversations: &[ConversationSummary],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = now_seconds();
        let show_all = self.others_shown_all.contains(folder);
        let (count, more) = shown(conversations.len(), false, show_all, None);
        self.follow_ages(conversations.iter().take(count), now, cx);
        let mut list = div().flex().flex_col().gap(px(2.0));
        if conversations.is_empty() {
            list = list.child(div().pl(px(42.0)).py(px(4.0)).child(ui::label(
                "No conversations yet",
                text::SIDE_NOTE,
                theme::ink3(),
            )));
        }
        for (at, conversation) in conversations.iter().take(count).enumerate() {
            list = list.child(Self::folder_conversation_row(
                index,
                at,
                folder,
                conversation,
                now,
                cx,
            ));
        }
        if let Some(more) = more {
            list = list.child(Self::folder_more_row(index, folder, more, cx));
        }
        list.into_any_element()
    }

    /// One of another folder's conversations (`at` among them, under the
    /// folder at `index`): its title, and in its tip its title and age at
    /// `now`; pressed, its project opens on it.
    fn folder_conversation_row(
        index: usize,
        at: usize,
        folder: &Path,
        conversation: &ConversationSummary,
        now: i64,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let title: SharedString = conversation.label().to_owned().into();
        let tip: SharedString = match parse_time(&conversation.updated) {
            Some(when) => format!("{title} · {}", relative_time(when, now)).into(),
            None => title.clone(),
        };
        let open = Entry::OpenConversation(folder.to_path_buf(), conversation.id.clone());
        Self::keyed(
            div().id(ElementId::Name(
                format!("folder-{index}-conversation-{at}").into(),
            )),
            ITEM,
        )
        .relative()
        .flex()
        .items_center()
        .h(px(theme::height::H28))
        .pl(px(34.0))
        .pr(px(10.0))
        .mx(px(8.0))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .hover(|row| row.bg(theme::raised()))
        .child(
            ui::line(title, text::SIDE, theme::ink2())
                .flex_1()
                .min_w_0(),
        )
        .child(ui::tip_zone(
            ElementId::Name(format!("folder-{index}-conversation-tip-{at}").into()),
            Some(ui::Tip::new(tip)),
        ))
        .on_click(
            cx.listener(move |this, _, window, cx| this.run_entry(open.clone(), false, window, cx)),
        )
    }

    /// The row after another folder's first few conversations: pressed, the
    /// rest show, or go again.
    fn folder_more_row(
        index: usize,
        folder: &Path,
        more: More,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let words = more_words(more);
        let folder = folder.to_path_buf();
        Self::keyed(
            div().id(ElementId::Name(format!("folder-{index}-more").into())),
            ITEM,
        )
        .flex()
        .items_center()
        .h(px(theme::height::H28))
        .pl(px(34.0))
        .mx(px(8.0))
        .rounded(px(theme::radius::CONTROL))
        .cursor_pointer()
        .hover(|row| row.bg(theme::raised()))
        .child(ui::line(words, text::SIDE_NOTE, theme::ink3()))
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.others_shown_all.remove(&folder) {
                this.others_shown_all.insert(folder.clone());
            }
            cx.notify();
        }))
    }
}
