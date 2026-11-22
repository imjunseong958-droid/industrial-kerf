//! What the sidebar's menus hold: a conversation's (Rename…, Archive,
//! Delete…), the open folder's (New Conversation, the folders, Open
//! Folder…, New Project…, Show in Files, Copy Path) and another folder's
//! (New Conversation, Open Project, Show in Files, Copy Path, Remove from
//! Sidebar); and where the arrows go among them.

use super::folders::folder_name;
use crate::icons::Icon;
use std::path::{Path, PathBuf};

/// What an item of one of the sidebar's menus does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A conversation's title becomes a field.
    Rename(String),
    /// A conversation archived (true) or brought back.
    Archive(String, bool),
    /// A conversation deleted, once the person has said so.
    Delete(String),
    /// The registry's command by its id.
    Command(&'static str),
    /// Out of the Kerf account.
    SignOut,
    /// The project in a folder, in place of this window's.
    Open(PathBuf),
    /// A folder in the system's file manager.
    Reveal(PathBuf),
    /// A folder's path on the clipboard.
    CopyPath(PathBuf),
    /// A folder off the recent ones, its files left alone.
    Forget(PathBuf),
    /// A new conversation in a folder: here, or in its project, opened in
    /// place of this one.
    NewConversation(PathBuf),
    /// One of another folder's conversations, by its id, in its project
    /// opened in place of this one.
    OpenConversation(PathBuf, String),
}

/// An item of one of the sidebar's menus, as it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub entry: Entry,
    pub words: String,
    pub icon: Option<Icon>,
    /// What cannot be undone: in brick.
    pub danger: bool,
    /// What cannot be done now: shown, and not chosen.
    pub disabled: bool,
    /// The one chosen among its like: ticked at its end.
    pub checked: bool,
    /// What a hover over it says (a folder's path).
    pub tip: Option<String>,
    /// A line goes before it.
    pub separated: bool,
}

impl MenuEntry {
    fn new(entry: Entry, words: impl Into<String>) -> Self {
        Self {
            entry,
            words: words.into(),
            icon: None,
            danger: false,
            disabled: false,
            checked: false,
            tip: None,
            separated: false,
        }
    }

    fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    fn separated(mut self) -> Self {
        self.separated = true;
        self
    }
}

/// A conversation's menu: Rename…, Archive (Unarchive), and Delete…, which
/// is off while Kerf works in it.
pub fn row_entries(id: &str, archived: bool, working: bool) -> Vec<MenuEntry> {
    vec![
        MenuEntry::new(Entry::Rename(id.to_owned()), "Rename…"),
        MenuEntry::new(
            Entry::Archive(id.to_owned(), !archived),
            if archived { "Unarchive" } else { "Archive" },
        )
        .icon(Icon::Archive),
        MenuEntry {
            danger: true,
            disabled: working,
            ..MenuEntry::new(Entry::Delete(id.to_owned()), "Delete…")
                .icon(Icon::Trash)
                .separated()
        },
    ]
}

/// What the system's file manager is called here.
pub fn reveal_words() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(target_os = "windows") {
        "Show in Explorer"
    } else {
        "Show in Files"
    }
}

/// The folders to work in, as the folder's menu lists them: the open one
/// first, then the recent ones still there, each once.
pub fn recent_folders(recent: &[PathBuf], open: Option<&Path>) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = open.map(Path::to_path_buf).into_iter().collect();
    for folder in recent {
        if folder.is_dir()
            && !folders
                .iter()
                .any(|known| crate::folders::same_folder(known, folder))
        {
            folders.push(folder.clone());
        }
    }
    folders
}

/// The menu of the open folder's name: New Conversation in it, the
/// folders (the open one ticked, each with its path as tip), Open Folder…
/// and New Project…, then the open folder in the file manager and its
/// path to copy.
pub fn folder_entries(recent: &[PathBuf], open: Option<&Path>) -> Vec<MenuEntry> {
    let folders = recent_folders(recent, open);
    let mut entries: Vec<MenuEntry> = open
        .map(|open| {
            MenuEntry::new(
                Entry::NewConversation(open.to_path_buf()),
                "New Conversation",
            )
            .icon(Icon::Plus)
        })
        .into_iter()
        .collect();
    let first_folder = entries.len();
    entries.extend(folders.iter().map(|folder| MenuEntry {
        checked: open.is_some_and(|open| crate::folders::same_folder(open, folder)),
        tip: Some(folder.display().to_string()),
        ..MenuEntry::new(Entry::Open(folder.clone()), folder_name(folder)).icon(Icon::Folder)
    }));
    if first_folder > 0 && entries.len() > first_folder {
        entries[first_folder].separated = true;
    }
    let mut open_folder = MenuEntry::new(Entry::Command("file.openProject"), "Open Folder…");
    if !entries.is_empty() {
        open_folder = open_folder.separated();
    }
    entries.push(open_folder);
    entries.push(MenuEntry::new(
        Entry::Command("file.newProject"),
        "New Project…",
    ));
    if let Some(open) = open {
        entries.push(MenuEntry::new(Entry::Reveal(open.to_path_buf()), reveal_words()).separated());
        entries.push(MenuEntry::new(
            Entry::CopyPath(open.to_path_buf()),
            "Copy Path",
        ));
    }
    entries
}

/// Another folder's menu: a new conversation in it (its project opened in
/// place of this one), open it, show it, copy its path, or take it off the
/// sidebar, its files left alone.
pub fn other_folder_entries(folder: &Path) -> Vec<MenuEntry> {
    vec![
        MenuEntry::new(
            Entry::NewConversation(folder.to_path_buf()),
            "New Conversation",
        )
        .icon(Icon::Plus),
        MenuEntry::new(Entry::Open(folder.to_path_buf()), "Open Project"),
        MenuEntry::new(Entry::Reveal(folder.to_path_buf()), reveal_words()).separated(),
        MenuEntry::new(Entry::CopyPath(folder.to_path_buf()), "Copy Path"),
        MenuEntry::new(Entry::Forget(folder.to_path_buf()), "Remove from Sidebar").separated(),
    ]
}

/// The item the arrows go to from `from` in `entries`: the next (or the
/// one before) that can be chosen, round from the end; None when none can.
pub fn menu_step(entries: &[MenuEntry], from: Option<usize>, down: bool) -> Option<usize> {
    let count = entries.len();
    if count == 0 {
        return None;
    }
    let start = match from {
        Some(at) => at.min(count - 1),
        None if down => count - 1,
        None => 0,
    };
    (1..=count)
        .map(|step| {
            if down {
                (start + step) % count
            } else {
                (start + count * 2 - step) % count
            }
        })
        .find(|at| !entries[*at].disabled)
}
