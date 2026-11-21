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
//!
//! The open folder's name opens the folders and the ways to another; each
//! other folder has its own menu (Remove from Sidebar). The account at the
//! foot says who chat runs on, and its menu has Sign Out while signed in to
//! Kerf. The edge, pressed once, puts the sidebar away after a double-click's
//! wait; pressed twice, its width goes back. Tab reaches the rows and
//! buttons from the window, with a ring; Enter or Space presses them,
//! Shift+F10 or the Menu key opens a row's menu, the arrows move in menus.

use crate::commands::{OpenRecent, RunCommand};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::model::{Conversation, ConversationSummary, Proposal, Speaker};
use crate::motion;
use crate::prefs::{Keymap, Provider};
use crate::session::{BridgeEvent, Session};
use crate::store::Change;
use crate::theme::{self, layout, motion as scene, text};
use crate::ui::{self, icon_button};
use gpui::{
    canvas, deferred, div, linear_color_stop, linear_gradient, prelude::*, px, AnimationExt,
    AnyElement, App, Bounds, ClickEvent, ClipboardItem, Context, DispatchPhase, Div, ElementId,
    Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, KeyUpEvent, Keystroke, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, NoAction, Pixels, Point, SharedString, Stateful,
    Subscription, Task, Window,
};
use serde_json::Value;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub enum SidebarEvent {
    Settings,
    Hide,
    /// Delete this conversation, once the person has said so.
    Delete(String),
    /// A field the sidebar had the keys in is done: the window takes them back.
    Done,
    /// One of its menus opened: the window puts its own away.
    MenuOpened,
    /// The account was pressed (`from_keys`: with Enter, Space, Shift+F10
    /// or the Menu key): the window opens the app's menu above it, or puts
    /// it away when it is open.
    AppMenu {
        from_keys: bool,
    },
    /// A conversation was chosen, or New conversation: the window gives
    /// the composer the keys, as Qt's did.
    Chosen,
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
                .any(|known| crate::app::same_folder(known, folder))
        {
            folders.push(folder.clone());
        }
    }
    folders
}

fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}

/// The menu of the open folder's name, as Qt's: New Conversation in it,
/// the folders (the open one ticked, each with its path as tip), Open
/// Folder… and New Project…, then the open folder in the file manager and
/// its path to copy.
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
        checked: open.is_some_and(|open| crate::app::same_folder(open, folder)),
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

/// Another folder's menu, as Qt's: a new conversation in it (its project
/// opened in place of this one), open it, show it, copy its path, or take
/// it off the sidebar, its files left alone.
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

/// Where a project folder keeps its conversations, as the bridge keeps
/// them (`.kerf/conversations/<id>.json`).
pub const CONVERSATIONS: &str = ".kerf/conversations";

/// Another folder's conversations, read from where its project keeps them
/// as the bridge lists them (no bridge of its own is asked): the latest
/// first, those archived left out (they are its own to bring back, once it
/// is open). A file that does not read, or is not a conversation's, is
/// left out.
pub fn folder_conversations(folder: &Path) -> Vec<ConversationSummary> {
    let Ok(entries) = std::fs::read_dir(folder.join(CONVERSATIONS)) else {
        return Vec::new();
    };
    let mut all: Vec<ConversationSummary> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|text| conversation_summary(&text))
        .filter(|conversation| !conversation.archived)
        .collect();
    all.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
    all
}

/// A conversation's file as the sidebar lists it; None for one that is
/// not a conversation's (its id is not one the bridge makes). Its turns
/// are only counted, not kept: a long conversation's file is large.
pub fn conversation_summary(text: &str) -> Option<ConversationSummary> {
    // An object, as the bridge writes it (a list would read as its fields
    // in turn).
    if !text.trim_start().starts_with('{') {
        return None;
    }
    let file: ConversationFile = serde_json::from_str(text).ok()?;
    let id = file.id.as_str()?;
    let valid = id.len() <= 80
        && id.starts_with("c-")
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if !valid {
        return None;
    }
    let text_of = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    Some(ConversationSummary {
        id: id.to_owned(),
        title: text_of(&file.title),
        created: text_of(&file.created),
        updated: text_of(&file.updated),
        archived: file.archived.as_bool().unwrap_or(false),
        count: file.items.0,
    })
}

/// What [`conversation_summary`] reads of a conversation's file: what is
/// not text, or not there, reads as nothing, and every other field is
/// passed over unread.
#[derive(serde::Deserialize)]
struct ConversationFile {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    title: Value,
    #[serde(default)]
    created: Value,
    #[serde(default)]
    updated: Value,
    #[serde(default)]
    archived: Value,
    #[serde(default)]
    items: Counted,
}

/// How many things a list holds, counted as it is read and none of them
/// kept; anything not a list holds none.
#[derive(Default)]
struct Counted(u32);

impl<'de> serde::Deserialize<'de> for Counted {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{IgnoredAny, MapAccess, SeqAccess};
        struct Count;
        impl<'de> serde::de::Visitor<'de> for Count {
            type Value = Counted;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("any value")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Counted, A::Error> {
                let mut count = 0u32;
                while seq.next_element::<IgnoredAny>()?.is_some() {
                    count = count.saturating_add(1);
                }
                Ok(Counted(count))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Counted, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Counted(0))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_i64<E>(self, _: i64) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_u64<E>(self, _: u64) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_f64<E>(self, _: f64) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_str<E>(self, _: &str) -> Result<Counted, E> {
                Ok(Counted(0))
            }
            fn visit_unit<E>(self) -> Result<Counted, E> {
                Ok(Counted(0))
            }
        }
        deserializer.deserialize_any(Count)
    }
}

/// Opens conversation `id` (None: a new one) in the window whose project
/// is `folder`, once what opens it there has run (`OpenRecent`, which
/// opens the folder's project in place of this window, or brings its own
/// window to the front).
pub fn open_conversation_in(folder: PathBuf, id: Option<String>, cx: &mut App) {
    cx.defer(move |cx| {
        for handle in cx.windows() {
            let Some(workspace) = handle.downcast::<crate::workspace::Workspace>() else {
                continue;
            };
            let _ = workspace.update(cx, |workspace, _, cx| {
                let session = workspace.session().clone();
                session.update(cx, |session, cx| {
                    let here = session
                        .project
                        .as_deref()
                        .is_some_and(|open| crate::app::same_folder(open, &folder));
                    if here {
                        session.open_conversation(id.clone());
                        cx.notify();
                    }
                });
            });
        }
    });
}

/// The recent folders with `folder` taken off, however written.
pub fn forget_folder(recent: &[PathBuf], folder: &Path) -> Vec<PathBuf> {
    recent
        .iter()
        .filter(|known| !crate::app::same_folder(known, folder))
        .cloned()
        .collect()
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

/// Who chat runs on, as the account at the sidebar's foot says it: the Kerf
/// account's email and plan while signed in; else Claude Code on "Your
/// Claude plan", Kerf "Not signed in", or "My own API key" with its model,
/// "Set up in Settings" while there is no key.
pub fn account_line(
    provider: Provider,
    cloud: Option<&Value>,
    has_key: bool,
    model: &str,
) -> (String, String) {
    if crate::settings::account_status(cloud).sign_out {
        let account = cloud.map(|cloud| &cloud["account"]);
        let email = account
            .and_then(|account| account["email"].as_str())
            .unwrap_or_default();
        let plan = account
            .and_then(|account| account["plan"].as_str())
            .filter(|plan| !plan.trim().is_empty())
            .map_or_else(|| "Kerf account".to_owned(), |plan| format!("{plan} plan"));
        return (email.to_owned(), plan);
    }
    match provider {
        Provider::ClaudeCode => ("Claude Code".into(), "Your Claude plan".into()),
        Provider::Cloud => ("Kerf".into(), "Not signed in".into()),
        Provider::Anthropic | Provider::Openai => {
            let model = model.trim();
            let below = if !has_key {
                "Set up in Settings".to_owned()
            } else if model.is_empty() {
                provider.label().to_owned()
            } else {
                crate::chat::model_label(model)
            };
            ("My own API key".into(), below)
        }
    }
}

// ------------------------------------------------------------ the app's menu

/// Choose the keymap the editor answers to (the app's menu, View ▸ Keyboard).
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct ChooseKeymap {
    pub keymap: Keymap,
}

/// Switch to another branch of the project's repository (View ▸ Sync).
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct SwitchBranch {
    pub name: SharedString,
}

// SignOut: out of the Kerf account (the app's menu, while signed in).
gpui::actions!(kerf, [SignOut]);

/// What a row of the app's menu does, chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum AppDoes {
    /// Nothing: words that say how something stands.
    Nothing,
    /// The registry's command (or one the window carries out) by its id.
    Command(String),
    SignOut,
    /// A mask tool on the picture.
    Mask(crate::stage::MaskTool),
    /// The viewer's zoom, a percent of the frame's own pixels; 0 fits it.
    Zoom(u32),
    Keymap(Keymap),
    Branch(String),
    /// A recent project, in place of this one.
    Recent(PathBuf),
    ClearRecent,
}

impl AppDoes {
    /// The action it dispatches from where the keys are (the window).
    pub fn action(&self) -> Option<Box<dyn gpui::Action>> {
        Some(match self {
            AppDoes::Nothing => return None,
            AppDoes::Command(id) => Box::new(RunCommand {
                id: id.clone().into(),
            }),
            AppDoes::SignOut => Box::new(SignOut),
            AppDoes::Mask(tool) => match tool {
                crate::stage::MaskTool::Rectangle => Box::new(crate::stage::RectangleMask),
                crate::stage::MaskTool::Ellipse => Box::new(crate::stage::EllipseMask),
                crate::stage::MaskTool::Pen => Box::new(crate::stage::PenMask),
            },
            AppDoes::Zoom(percent) => Box::new(crate::stage::SetZoom { percent: *percent }),
            AppDoes::Keymap(keymap) => Box::new(ChooseKeymap { keymap: *keymap }),
            AppDoes::Branch(name) => Box::new(SwitchBranch {
                name: name.clone().into(),
            }),
            AppDoes::Recent(path) => Box::new(OpenRecent {
                path: path.to_string_lossy().into_owned().into(),
            }),
            AppDoes::ClearRecent => Box::new(crate::commands::ClearRecent),
        })
    }
}

/// A row of the app's menu, as it shows: its words, icon and key, whether
/// it is ticked or greyed (with why in its tip), and the rows it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRow {
    pub words: String,
    pub icon: Option<Icon>,
    pub key: Option<String>,
    pub checked: bool,
    pub disabled: bool,
    pub tip: Option<String>,
    /// A line goes before it.
    pub separated: bool,
    pub does: AppDoes,
    /// A submenu's rows.
    pub rows: Vec<AppRow>,
}

impl AppRow {
    fn new(words: impl Into<String>, does: AppDoes) -> Self {
        Self {
            words: words.into(),
            icon: None,
            key: None,
            checked: false,
            disabled: false,
            tip: None,
            separated: false,
            does,
            rows: Vec::new(),
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

    fn submenu(words: impl Into<String>, rows: Vec<AppRow>) -> Self {
        Self {
            rows,
            ..Self::new(words, AppDoes::Nothing)
        }
    }

    /// Words that say how something stands: greyed, chosen by nothing.
    fn note(words: impl Into<String>) -> Self {
        Self {
            disabled: true,
            ..Self::new(words, AppDoes::Nothing)
        }
    }
}

/// What the app's menu is built from: how the window and its commands
/// stand ([`app_rows`]).
pub struct AppMenuInput<'a> {
    /// Signed in to Kerf: Sign Out is offered.
    pub signed_in: bool,
    pub keymap: Keymap,
    /// The registry's commands in the keymap in use ([`crate::commands::catalog`]),
    /// and its menus in order.
    pub commands: &'a [crate::commands::Command],
    pub menus: &'a [String],
    /// Why each command cannot run now (`kerf_bridge::commands_states`).
    pub states: &'a Value,
    /// The checkable commands that are on.
    pub checked: &'a std::collections::BTreeSet<String>,
    /// The viewer's zoom (0 fits).
    pub zoom: f32,
    /// The project's Git standing (the last `collab` event's `status`),
    /// with why the last sync failed under [`SYNC_FAILURE`] when it did.
    pub collab: Option<&'a Value>,
    pub revision: u64,
    /// The system draws the registry's menus in a menu bar (macOS):
    /// they are not repeated here.
    pub menu_bar: bool,
    pub recent: &'a [PathBuf],
    pub current: Option<&'a Path>,
}

/// The row for the registry's command `id`, in `words` (its own label
/// when None): its key, ticked while on, greyed with why while it cannot
/// run.
fn command_row(input: &AppMenuInput, id: &str, words: Option<&str>) -> AppRow {
    let command = input.commands.iter().find(|command| command.id == id);
    let reason = crate::palette::unavailable(input.states, id).map(str::to_owned);
    AppRow {
        key: command
            .and_then(|command| command.keys.first())
            .map(|key| crate::commands::shown_keys(key)),
        checked: command.is_some_and(|command| command.checkable) && input.checked.contains(id),
        disabled: reason.is_some(),
        tip: reason,
        ..AppRow::new(
            words
                .map(str::to_owned)
                .or_else(|| command.map(|command| command.label.clone()))
                .unwrap_or_else(|| id.to_owned()),
            AppDoes::Command(id.to_owned()),
        )
    }
}

/// The row for the picture's mask tool `tool`, with its key: ticked while
/// it is out, greyed with why while none can be (no picture clip chosen
/// that the picture can place), as the states and ticks say by its own id
/// ([`crate::commands::mask_tool_id`]).
fn mask_row(input: &AppMenuInput, tool: crate::stage::MaskTool) -> AppRow {
    let id = crate::commands::mask_tool_id(tool);
    let reason = crate::palette::unavailable(input.states, id).map(str::to_owned);
    AppRow {
        key: Some(crate::commands::shown_keys(tool.shortcut())),
        checked: input.checked.contains(id),
        disabled: reason.is_some(),
        tip: reason,
        ..AppRow::new(tool.label(), AppDoes::Mask(tool))
    }
}

/// The app's menu, as Qt's account menu had it: Settings…, Account…; Media
/// and Import…; View ▸ (the panes, Follow Kerf, the mask
/// tools, Safe Zones, Viewer Zoom ▸, Keyboard ▸, Sync ▸); where there is no
/// menu bar, the registry's menus (File ▸ … Help ▸); Sign Out while signed
/// in to Kerf.
pub fn app_rows(input: &AppMenuInput) -> Vec<AppRow> {
    let mut rows = vec![
        command_row(input, "app.preferences", Some("Settings…")).icon(Icon::Gear),
        AppRow::new("Account…", AppDoes::Command("app.account".into())).icon(Icon::Lock),
        command_row(input, "window.project", Some("Media"))
            .icon(Icon::Media)
            .separated(),
        command_row(input, "file.import", Some("Import…")).icon(Icon::Import),
        AppRow::submenu("View", view_rows(input)).icon(Icon::Eye),
    ];
    if !input.menu_bar {
        let mut first = true;
        for menu in input.menus {
            let menu_rows = registry_rows(input, menu);
            if menu_rows.is_empty() {
                continue;
            }
            let mut row = AppRow::submenu(menu.clone(), menu_rows);
            row.separated = std::mem::take(&mut first);
            rows.push(row);
        }
    }
    if input.signed_in {
        rows.push(AppRow::new("Sign Out", AppDoes::SignOut).separated());
    }
    rows
}

/// View ▸: the panes and Follow Kerf, the mask tools, Safe Zones and the
/// zoom, the keyboard's choice and the sync.
fn view_rows(input: &AppMenuInput) -> Vec<AppRow> {
    let mut rows = vec![
        command_row(input, "view.sidebar", Some("Sidebar")),
        command_row(input, "view.picture", Some("Picture")),
        command_row(input, "view.timeline", Some("Timeline")),
        command_row(input, "view.followKerf", Some("Follow Kerf")),
    ];
    for (index, tool) in crate::stage::MaskTool::ALL.into_iter().enumerate() {
        let mut row = mask_row(input, tool).icon(tool.icon());
        row.separated = index == 0;
        rows.push(row);
    }
    rows.push(command_row(input, "view.safeZones", Some("Safe Zones")).separated());
    let zooms = crate::stage::ZOOMS
        .into_iter()
        .map(|zoom| AppRow {
            checked: (zoom - input.zoom).abs() < 1e-3,
            ..AppRow::new(
                crate::stage::zoom_words(zoom),
                AppDoes::Zoom((zoom * 100.0).round() as u32),
            )
        })
        .collect();
    rows.push(AppRow::submenu("Viewer Zoom", zooms));
    let keymaps = Keymap::ALL
        .into_iter()
        .map(|keymap| AppRow {
            checked: keymap == input.keymap,
            ..AppRow::new(keymap.label(), AppDoes::Keymap(keymap))
        })
        .collect();
    rows.push(AppRow::submenu("Keyboard", keymaps).separated());
    rows.push(AppRow::submenu("Sync", sync_rows(input)));
    rows
}

/// What a sync's standing says over its rows, as Qt's: the revision, then
/// how far it is from its upstream ("Revision 22 · 1 commit to pull").
pub fn sync_summary(collab: Option<&Value>, revision: u64) -> String {
    let revision = format!("Revision {revision}");
    let Some(status) = collab.filter(|status| status["repository"].as_bool() == Some(true)) else {
        return format!("{revision} · not shared yet");
    };
    let count = |name: &str| status[name].as_i64().unwrap_or(0).max(0);
    let (behind, ahead) = (count("behind"), count("ahead"));
    let mut parts = vec![revision];
    let plural = |n: i64, one: &str, many: &str| {
        if n == 1 {
            one.to_owned()
        } else {
            many.replace("{}", &n.to_string())
        }
    };
    if behind > 0 {
        parts.push(plural(behind, "1 commit to pull", "{} commits to pull"));
    }
    if ahead > 0 {
        parts.push(plural(ahead, "1 commit to push", "{} commits to push"));
    }
    if behind == 0 && ahead == 0 {
        parts.push("up to date".into());
    }
    parts.join(" · ")
}

/// Where the window puts why the last sync failed (until one succeeds)
/// in the standing it gives the app's menu ([`AppMenuInput::collab`]):
/// the Sync submenu says so ([`with_sync_failure`]).
pub const SYNC_FAILURE: &str = "syncFailure";

/// `collab` (the project's Git standing) with why the last sync failed,
/// when it did, for the app's menu ([`SYNC_FAILURE`]).
pub fn with_sync_failure(collab: Option<&Value>, failure: Option<&str>) -> Option<Value> {
    let mut status = collab.cloned()?;
    if let (Some(failure), Some(fields)) = (failure, status.as_object_mut()) {
        fields.insert(SYNC_FAILURE.to_owned(), Value::from(failure));
    }
    Some(status)
}

/// What the Sync submenu warns of under its summary, as Qt's: "A merge is
/// unfinished; finish it with git" while one is, else "Sync failed: <why>"
/// after a sync that failed; none when nothing went wrong.
pub fn sync_problem(collab: Option<&Value>) -> Option<String> {
    let status = collab?;
    if status["conflicted"].as_bool() == Some(true) {
        return Some("A merge is unfinished; finish it with git".to_owned());
    }
    status[SYNC_FAILURE]
        .as_str()
        .filter(|failure| !failure.trim().is_empty())
        .map(|failure| format!("Sync failed: {failure}"))
}

/// View ▸ Sync: how it stands; the branches (this one ticked; another
/// switches to it) and New Branch…; Sync Now, Live Sync, Commit… and Pull
/// Request…. A project that is no repository yet offers Publish only.
fn sync_rows(input: &AppMenuInput) -> Vec<AppRow> {
    let mut rows = vec![AppRow::note(sync_summary(input.collab, input.revision))];
    let status = input
        .collab
        .filter(|status| status["repository"].as_bool() == Some(true));
    // What went wrong, when something did (Qt's buildSyncMenu): an
    // unfinished merge, else why the last sync failed.
    if let Some(problem) = sync_problem(input.collab) {
        rows.push(AppRow {
            icon: Some(Icon::Dot),
            // All of it, where the row cuts it to one line.
            tip: Some(problem.clone()),
            ..AppRow::note(problem)
        });
    }
    let Some(status) = status else {
        rows.push(command_row(input, "collab.publish", None).separated());
        return rows;
    };
    let current = status["branch"].as_str().unwrap_or_default().to_owned();
    let mut branches: Vec<String> = Vec::new();
    if !current.is_empty() {
        branches.push(current.clone());
    }
    for branch in status["branches"].as_array().into_iter().flatten() {
        if let Some(branch) = branch.as_str().filter(|branch| !branch.is_empty()) {
            if !branches.iter().any(|known| known == branch) {
                branches.push(branch.to_owned());
            }
        }
    }
    let switch_reason =
        crate::palette::unavailable(input.states, "collab.switchBranch").map(str::to_owned);
    for (index, branch) in branches.into_iter().enumerate() {
        let here = branch == current;
        rows.push(AppRow {
            checked: here,
            disabled: !here && switch_reason.is_some(),
            tip: (!here).then(|| switch_reason.clone()).flatten(),
            separated: index == 0,
            ..AppRow::new(
                branch.clone(),
                if here {
                    AppDoes::Nothing
                } else {
                    AppDoes::Branch(branch)
                },
            )
        });
    }
    rows.push(command_row(input, "collab.newBranch", None));
    rows.push(command_row(input, "collab.sync", None).separated());
    rows.push(command_row(input, "collab.live", None));
    rows.push(command_row(input, "collab.commit", None));
    rows.push(command_row(input, "collab.pullRequest", None));
    rows
}

/// The registry's menu `menu` as rows, a line between its groups; File ▸
/// Open Recent after Open Project, and the mask tools at View's end, as the
/// menu bar has them.
fn registry_rows(input: &AppMenuInput, menu: &str) -> Vec<AppRow> {
    let mut rows = Vec::new();
    let mut group = None;
    for command in input.commands.iter().filter(|command| command.menu == menu) {
        let mut row = command_row(input, &command.id, None);
        // Switch Branch needs a branch: it opens on the others, as Qt's
        // did, greyed with why while there are none to go to.
        if command.id == "collab.switchBranch" {
            row = AppRow {
                rows: other_branches(input),
                does: AppDoes::Nothing,
                ..row
            };
        }
        row.separated = group.is_some_and(|group| group != command.group);
        group = Some(command.group);
        rows.push(row);
        if command.id == "file.openProject" {
            rows.push(AppRow::submenu("Open Recent", recent_rows(input)));
        }
    }
    if menu == "View" && !rows.is_empty() {
        for (index, tool) in crate::stage::MaskTool::ALL.into_iter().enumerate() {
            let mut row = mask_row(input, tool);
            row.separated = index == 0;
            rows.push(row);
        }
    }
    rows
}

/// Collaborate ▸ Switch Branch: the project's branches other than the one
/// it is on, each switching to it (Qt's buildBranchMenu); "No Other
/// Branches" when there are none.
fn other_branches(input: &AppMenuInput) -> Vec<AppRow> {
    let current = input
        .collab
        .and_then(|status| status["branch"].as_str())
        .unwrap_or_default();
    let mut rows: Vec<AppRow> = Vec::new();
    for branch in input
        .collab
        .and_then(|status| status["branches"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|branch| !branch.is_empty() && *branch != current)
    {
        if !rows.iter().any(|row| row.words == branch) {
            rows.push(AppRow::new(branch, AppDoes::Branch(branch.to_owned())));
        }
    }
    if rows.is_empty() {
        rows.push(AppRow::note(crate::finish::NO_OTHER_BRANCHES));
    }
    rows
}

/// File ▸ Open Recent: each recent project (the one open here ticked, one
/// whose folder is gone greyed), "No Recent Projects" when there are none,
/// then Clear Menu.
fn recent_rows(input: &AppMenuInput) -> Vec<AppRow> {
    let recent = crate::commands::recent_rows(input.recent, input.current);
    let mut rows: Vec<AppRow> = recent
        .iter()
        .map(|row| AppRow {
            checked: row.current,
            disabled: row.missing,
            tip: Some(if row.missing {
                crate::palette::RECENT_GONE.to_owned()
            } else {
                row.path.display().to_string()
            }),
            ..AppRow::new(row.name.clone(), AppDoes::Recent(row.path.clone()))
        })
        .collect();
    if rows.is_empty() {
        rows.push(AppRow::note("No Recent Projects"));
    }
    rows.push(AppRow {
        disabled: recent.is_empty(),
        ..AppRow::new("Clear Menu", AppDoes::ClearRecent).separated()
    });
    rows
}

/// `rows` as the kit's menu rows: each, chosen, dispatches what it does
/// from where the keys go back to once the menu has gone.
pub fn app_menu_items(rows: &[AppRow]) -> Vec<ui::MenuItem> {
    rows.iter()
        .map(|row| {
            let mut item = ui::MenuItem::new(row.words.clone())
                .checked(row.checked)
                .disabled(row.disabled);
            if let Some(icon) = row.icon {
                item = item.icon(icon);
            }
            if let Some(key) = &row.key {
                item = item.key(key.clone());
            }
            if let Some(tip) = &row.tip {
                item = item.tip(tip.clone());
            }
            if row.separated {
                item = item.separated();
            }
            if !row.rows.is_empty() {
                item = item.submenu(app_menu_items(&row.rows));
            }
            let does = row.does.clone();
            item.on_choose(move |window, cx| {
                if let Some(action) = does.action() {
                    window.dispatch_action(action, cx);
                }
            })
        })
        .collect()
}

/// Who chat runs on in `session`'s window, as the account at the foot says
/// it ([`account_line`]), and whether it is signed in to Kerf.
pub fn account_of(session: &Session, cx: &App) -> (String, String, bool) {
    let cloud = crate::app::cloud(cx);
    let has_key = session.ai_access(cloud).as_deref() != Some("noKey");
    let (title, below) = account_line(
        session.prefs.provider,
        cloud,
        has_key,
        session.prefs.model(),
    );
    let signed_in = crate::settings::account_status(cloud).sign_out;
    (title, below, signed_in)
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

// ------------------------------------------------------------ keys

gpui::actions!(
    sidebar,
    [
        FocusNext,
        FocusPrevious,
        OpenMenu,
        RenameRow,
        DeleteRow,
        MenuUp,
        MenuDown,
        MenuChoose,
        MenuClose,
        Unfold,
        Fold,
    ]
);

/// The sidebar's key context: Tab goes from one of its rows and buttons
/// to the next, Shift-Tab back.
pub const CONTEXT: &str = "Sidebar";
/// What in it takes the keys: Enter and Space press it, Shift+F10 or the
/// Menu key opens its menu.
pub const ITEM: &str = "SidebarItem";
/// A conversation's row, where F2 renames it and Delete or Backspace
/// deletes it (after asking).
pub const ROW: &str = "SidebarRow";
/// One of its menus, open: the arrows move in it (Tab and Shift-Tab too,
/// as in Qt's menus), Enter or Space chooses, Esc closes it.
pub const MENU: &str = "SidebarMenu";
/// What a conversation's row answers in: [`ITEM`] and [`ROW`] at once.
pub const ROW_KEYS: &str = "SidebarItem SidebarRow";
/// Another folder's row, where Right unfolds its conversations and Left
/// folds them.
pub const FOLDER: &str = "SidebarFolder";
/// What another folder's row answers in: [`ITEM`] and [`FOLDER`].
pub const FOLDER_KEYS: &str = "SidebarItem SidebarFolder";

/// The sidebar's keys, as [`bind_keys`] binds them. Enter and Space on a
/// row or button run nothing of the window's (Play, a render): GPUI
/// presses what has the keys as they come up.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("tab", FocusNext, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(CONTEXT)),
        KeyBinding::new("enter", NoAction, Some(ITEM)),
        KeyBinding::new("space", NoAction, Some(ITEM)),
        KeyBinding::new("shift-f10", OpenMenu, Some(ITEM)),
        KeyBinding::new("menu", OpenMenu, Some(ITEM)),
        KeyBinding::new("f2", RenameRow, Some(ROW)),
        KeyBinding::new("delete", DeleteRow, Some(ROW)),
        KeyBinding::new("backspace", DeleteRow, Some(ROW)),
        KeyBinding::new("up", MenuUp, Some(MENU)),
        KeyBinding::new("down", MenuDown, Some(MENU)),
        KeyBinding::new("shift-tab", MenuUp, Some(MENU)),
        KeyBinding::new("tab", MenuDown, Some(MENU)),
        KeyBinding::new("enter", MenuChoose, Some(MENU)),
        KeyBinding::new("space", MenuChoose, Some(MENU)),
        KeyBinding::new("escape", MenuClose, Some(MENU)),
        KeyBinding::new("right", Unfold, Some(FOLDER)),
        KeyBinding::new("left", Fold, Some(FOLDER)),
    ]
}

/// Binds the sidebar's keys, unless they are bound. Binding a keymap
/// clears every binding, so Settings calls this again when it changes one,
/// and every sidebar as it opens.
pub fn bind_keys(cx: &mut App) {
    let bound = Keystroke::parse("shift-f10").is_ok_and(|keystroke| {
        cx.all_bindings_for_input(&[keystroke])
            .iter()
            .any(|binding| binding.action().partial_eq(&OpenMenu))
    });
    if !bound {
        cx.bind_keys(key_bindings());
    }
}

/// Whether a press on the sidebar's edge at `now` is the second of a
/// double-click: the platform counts it so, or the press before it was let
/// go less than [`scene::DOUBLE_CLICK`] ago.
pub fn is_second_press(last_click: Option<Instant>, now: Instant, click_count: usize) -> bool {
    click_count >= 2
        || last_click.is_some_and(|at| now.saturating_duration_since(at) < scene::DOUBLE_CLICK)
}

/// How many times the light over a working title is drawn in a pass
/// (Qt's titleShimmerSteps: 30 a second, the title alone).
pub use crate::theme::motion::TITLE_SHIMMER_STEPS;

/// `phase` (0..1 of a pass) held to the step it is in.
pub fn shimmer_step(phase: f32) -> f32 {
    let steps = TITLE_SHIMMER_STEPS as f32;
    (phase.clamp(0.0, 1.0) * steps).floor().min(steps - 1.0) / steps
}

/// What the sidebar's edge's tooltip says.
pub const EDGE_TIP: &str = "Drag to resize · click to hide · double-click to reset";

// ------------------------------------------------------------ the view

/// Which of the sidebar's menus is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarMenu {
    /// A conversation's, by its id.
    Row(String),
    /// The open folder's name: the folders, and the ways to another.
    Folder,
    /// Another folder's, by its path.
    Other(PathBuf),
}

/// A press on the sidebar's edge, while it is held.
struct EdgePress {
    from: Point<Pixels>,
    /// It moved past a press: it is a drag.
    moved: bool,
    /// The second press of a double-click: the width went back.
    second: bool,
}

pub struct SidebarView {
    session: Entity<Session>,
    /// The conversations' list, for its fades.
    list_scroll: gpui::ScrollHandle,
    archived_open: bool,
    /// How many times the archived ones were unfolded: each unfolding
    /// staggers them in afresh.
    unfoldings: u64,
    /// The menu open, if one is, and the item of it the arrows are on.
    menu: Option<SidebarMenu>,
    menu_chosen: Option<usize>,
    /// An open menu holds the keys; it gives them back to what had them
    /// (`menu_return`) when it closes from the keys or by a choice.
    menu_focus: FocusHandle,
    menu_return: Option<FocusHandle>,
    /// Where a right-click opened the menu open, in the window: it opens
    /// at the pointer there, as Qt's did. None: anchored under its row
    /// (its ⋯ button, the keys).
    menu_at: Option<Point<Pixels>>,
    /// Keys to give back once the menu that had them is gone.
    refocus: Option<FocusHandle>,
    /// An item was chosen from the keys: the menu, gone, holds them until
    /// that key comes up, then gives them back to what had them (None:
    /// nothing did). Given back at once, the key coming up would press
    /// what had them, which opens the menu again or the folder it was for.
    give_back_on_key_up: Option<Option<FocusHandle>>,
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
    /// The other folders unfolded, and their conversations as read when
    /// they were (None while they are being read, away from the window);
    /// those of them showing all their conversations.
    unfolded: HashMap<PathBuf, Option<Vec<ConversationSummary>>>,
    others_shown_all: HashSet<PathBuf>,
    /// Every conversation the list has held, and whether it has been
    /// listed at all: what was there at first shows at once.
    seen: HashSet<String>,
    listed: bool,
    /// Conversations new to the list, and when they came.
    arrivals: HashMap<String, Instant>,
    /// The sidebar is in the window: put away, its titles' light rests.
    shown: bool,
    /// The app's menu is open above the account, which stays held.
    app_menu_open: bool,
    /// Draws the working titles' light again, a step at a time, while one
    /// shows.
    shimmer_tick: Option<Task<()>>,
    /// A press held on the edge; the last press on it let go where it was,
    /// and the wait before that press puts the sidebar away (dropped when
    /// a second press comes).
    edge_press: Option<EdgePress>,
    edge_clicked: Option<Instant>,
    edge_hide: Option<Task<()>>,
    /// Where the open folder's heading was laid out last, in the window.
    heading_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    _subscriptions: Vec<Subscription>,
}

impl SidebarView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
        let search = cx.new(|cx| TextInput::new(cx, "Search").with_style(text::SIDE));
        let rename = cx.new(|cx| TextInput::new(cx, "Conversation title").with_style(text::SIDE));
        let search_focus = search.read(cx).focus_handle(cx);
        let rename_focus = rename.read(cx).focus_handle(cx);
        let menu_focus = cx.focus_handle();
        let subscriptions = vec![
            // A menu that loses the keys to something else goes; one
            // waiting for a key to come up gives them back if the window
            // goes to the back first.
            cx.on_blur(&menu_focus, window, |this, window, cx| {
                if this.give_back_on_key_up.is_some() {
                    if window.is_window_active() {
                        this.give_back_on_key_up = None;
                    } else {
                        this.give_back_keys(window, cx);
                    }
                } else {
                    this.close_menus(cx);
                }
            }),
            // A letter typed in an open menu goes to its next item that
            // starts with it, as in the kit's menus; nothing else typed
            // there reaches the window under it.
            {
                let this = cx.weak_entity();
                let focus = menu_focus.clone();
                cx.intercept_keystrokes(move |event, window, cx| {
                    if !focus.is_focused(window) {
                        return;
                    }
                    // The menu's own keys are bound in MENU (it has no
                    // submenus, nor Home and End); a letter goes to its item;
                    // anything else, a window's shortcut with Ctrl, ⌘ or Alt
                    // among them, does nothing while it is open.
                    let keystroke = &event.keystroke;
                    let key = match ui::menu_key(keystroke) {
                        ui::MenuKey::Own
                            if matches!(
                                keystroke.key.as_str(),
                                "up" | "down" | "enter" | "space" | "escape" | "tab"
                            ) =>
                        {
                            return;
                        }
                        ui::MenuKey::Own => ui::MenuKey::Held,
                        key => key,
                    };
                    let open = this
                        .update(cx, |this, cx| {
                            let open = this.menu.is_some();
                            if let ui::MenuKey::Letter(letter) = key {
                                this.jump_in_menu(letter, cx);
                            }
                            open
                        })
                        .unwrap_or(false);
                    if open {
                        cx.stop_propagation();
                    }
                })
            },
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, change: &Change, cx| {
                if *change == Change::Conversations {
                    this.note_arrivals(true, cx);
                }
            }),
            // Kerf's account, as the foot says it.
            cx.subscribe(&session, |_, _, event: &BridgeEvent, cx| {
                if event.0["type"] == "cloud" {
                    crate::app::hear_cloud(&event.0, cx);
                    cx.notify();
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
            // Tab from the window itself (nothing in it holding the keys)
            // comes into the sidebar; a field, a menu or what is over the
            // window keeps its own Tab.
            {
                let this = cx.weak_entity();
                let handle = window.window_handle();
                cx.intercept_keystrokes(move |event, window, cx| {
                    let keystroke = &event.keystroke;
                    let modifiers = keystroke.modifiers;
                    if window.window_handle() != handle
                        || keystroke.key != "tab"
                        || modifiers.control
                        || modifiers.alt
                        || modifiers.platform
                        || modifiers.function
                    {
                        return;
                    }
                    let at_window = window
                        .context_stack()
                        .last()
                        .is_none_or(|context| context.contains(crate::commands::WORKSPACE));
                    let shown = this.read_with(cx, |this, _| this.shown).unwrap_or(false);
                    if at_window && shown {
                        if modifiers.shift {
                            window.focus_prev();
                        } else {
                            window.focus_next();
                        }
                        cx.stop_propagation();
                    }
                })
            },
        ];
        let mut sidebar = Self {
            session,
            list_scroll: gpui::ScrollHandle::new(),
            archived_open: false,
            unfoldings: 0,
            menu: None,
            menu_chosen: None,
            menu_focus,
            menu_return: None,
            menu_at: None,
            refocus: None,
            give_back_on_key_up: None,
            shimmer_since: Instant::now(),
            search,
            searching: false,
            query: String::new(),
            rename,
            renaming: None,
            show_all: false,
            unfolded: HashMap::new(),
            others_shown_all: HashSet::new(),
            seen: HashSet::new(),
            listed: false,
            arrivals: HashMap::new(),
            shown: true,
            app_menu_open: false,
            shimmer_tick: None,
            edge_press: None,
            edge_clicked: None,
            edge_hide: None,
            heading_bounds: Rc::new(Cell::new(None)),
            _subscriptions: subscriptions,
        };
        sidebar.note_arrivals(false, cx);
        sidebar.unfold_as_left(cx);
        sidebar
    }

    /// The open folder's heading (the project's row) as last laid out, in
    /// the window's coordinates: where a finished export's frame lands
    /// (`Workspace::fly_export_home`).
    pub fn project_row_bounds(&self) -> Option<Bounds<Pixels>> {
        self.heading_bounds.get()
    }

    /// Whether the sidebar is in the window; put away, it asks for no
    /// frames of its own.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown != shown {
            self.shown = shown;
            // Put away, it takes its menus with it.
            if !shown {
                self.close_menus(cx);
            }
            cx.notify();
        }
    }

    /// Whether the app's menu is open above the account (the window holds
    /// it): the account stays held meanwhile.
    pub fn set_app_menu_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.app_menu_open != open {
            self.app_menu_open = open;
            cx.notify();
        }
    }

    /// Whether one of its menus is open (a conversation's, the folder's).
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Puts away whichever of its menus is open (a row's, a folder's);
    /// whether one was. The keys go back to what had them.
    pub fn close_menus(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.menu.take().is_some();
        if open {
            self.menu_chosen = None;
            self.refocus = self.menu_return.take();
            cx.notify();
        }
        open
    }

    /// The menu open is `menu`.
    fn menu_is(&self, menu: &SidebarMenu) -> bool {
        self.menu.as_ref() == Some(menu)
    }

    /// Opens `menu` (another open one goes), taking the keys so the arrows
    /// move in it; from the keys (`keyboard`), its first item is chosen.
    fn open_menu(
        &mut self,
        menu: SidebarMenu,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = self.entries_of(&menu, cx);
        self.menu = Some(menu);
        self.menu_at = None;
        self.menu_chosen = if keyboard {
            menu_step(&entries, None, true)
        } else {
            None
        };
        // One opening in place of a menu just closed (a right-click on
        // another row) keeps the keys' way back of the first.
        let earlier = self.refocus.take();
        self.give_back_on_key_up = None;
        if !self.menu_focus.is_focused(window) {
            self.menu_return = window.focused(cx);
        } else if self.menu_return.is_none() {
            self.menu_return = earlier;
        }
        window.focus(&self.menu_focus);
        cx.emit(SidebarEvent::MenuOpened);
        cx.notify();
    }

    /// A row's open `menu`, `width` wide: at the pointer where a
    /// right-click opened it (kept `S8` inside the window), else
    /// under the row's right end, `top` below the row's top.
    fn placed_menu(
        &self,
        menu: &SidebarMenu,
        top: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = self.menu_view(menu, None, width, cx);
        match self.menu_at {
            Some(at) => deferred(
                gpui::anchored()
                    .position(at)
                    .snap_to_window_with_margin(px(theme::space::S8))
                    .child(view),
            )
            .with_priority(1)
            .into_any_element(),
            None => deferred(div().absolute().top(px(top)).right(px(8.0)).child(view))
                .with_priority(1)
                .into_any_element(),
        }
    }

    /// `menu` opens, or closes when it is the one open.
    fn toggle_menu(
        &mut self,
        menu: SidebarMenu,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu_is(&menu) {
            self.close_menus(cx);
        } else {
            self.open_menu(menu, keyboard, window, cx);
        }
    }

    /// A press outside the menu put it away: what was pressed has the keys
    /// if it takes them, else they go back to what had them.
    fn menu_pressed_out(&mut self, cx: &mut Context<Self>) {
        self.close_menus(cx);
    }

    /// What the menu `menu` holds now.
    fn entries_of(&self, menu: &SidebarMenu, cx: &App) -> Vec<MenuEntry> {
        let session = self.session.read(cx);
        match menu {
            SidebarMenu::Row(id) => {
                let archived = session
                    .state
                    .conversations
                    .iter()
                    .find(|conversation| &conversation.id == id)
                    .is_some_and(|conversation| conversation.archived);
                let working = session.state.work.busy()
                    && (session.state.work.conversation.as_deref() == Some(id.as_str())
                        || (session.state.work.conversation.is_none()
                            && session.conversation.as_deref() == Some(id.as_str())));
                row_entries(id, archived, working)
            }
            SidebarMenu::Folder => {
                folder_entries(&session.prefs.recent, session.project.as_deref())
            }
            SidebarMenu::Other(folder) => other_folder_entries(folder),
        }
    }

    fn step_menu(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.clone() else {
            return;
        };
        let entries = self.entries_of(&menu, cx);
        self.menu_chosen = menu_step(&entries, self.menu_chosen, down);
        cx.notify();
    }

    /// A letter typed in the open menu: its next item that starts with it
    /// and can be chosen is lit.
    fn jump_in_menu(&mut self, letter: char, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.clone() else {
            return;
        };
        let entries = self.entries_of(&menu, cx);
        let words: Vec<&str> = entries.iter().map(|entry| entry.words.as_str()).collect();
        let enabled: Vec<bool> = entries.iter().map(|entry| !entry.disabled).collect();
        if let Some(at) = ui::menu_letter(&words, &enabled, self.menu_chosen, letter) {
            self.menu_chosen = Some(at);
            cx.notify();
        }
    }

    fn choose_in_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.clone() else {
            return;
        };
        let entries = self.entries_of(&menu, cx);
        if let Some(entry) = self
            .menu_chosen
            .and_then(|at| entries.get(at))
            .filter(|entry| !entry.disabled)
        {
            self.run_entry(entry.entry.clone(), true, window, cx);
        }
    }

    /// The key that chose in a menu came up (or the window went to the
    /// back first): the keys go back to what had them, if the menu still
    /// holds them.
    fn give_back_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(back) = self.give_back_on_key_up.take() else {
            return;
        };
        if self.menu_focus.is_focused(window) {
            match back {
                Some(back) => window.focus(&back),
                None => window.blur(),
            }
        }
        cx.notify();
    }

    /// Esc in an open menu: it goes, and the keys go back.
    fn menu_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let back = self.menu_return.clone();
        self.close_menus(cx);
        self.refocus = None;
        if let Some(back) = back {
            window.focus(&back);
        }
    }

    /// Does what a menu's item says (`from_keys`: chosen with Enter or
    /// Space); the menu goes first. Where nothing else took the keys, they
    /// go back to what had them: from the keys, once the key comes up.
    fn run_entry(
        &mut self,
        entry: Entry,
        from_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        self.menu_chosen = None;
        let back = self.menu_return.take();
        match entry {
            Entry::Rename(id) => self.begin_rename(&id, window, cx),
            Entry::Archive(id, archived) => self
                .session
                .update(cx, |session, _| session.archive_conversation(&id, archived)),
            Entry::Delete(id) => cx.emit(SidebarEvent::Delete(id)),
            Entry::Command(id) => {
                window.dispatch_action(Box::new(RunCommand { id: id.into() }), cx)
            }
            Entry::SignOut => window.dispatch_action(Box::new(SignOut), cx),
            Entry::Open(folder) => {
                let here = self
                    .session
                    .read(cx)
                    .project
                    .as_deref()
                    .is_some_and(|open| crate::app::same_folder(open, &folder));
                if !here {
                    window.dispatch_action(
                        Box::new(OpenRecent {
                            path: folder.to_string_lossy().into_owned().into(),
                        }),
                        cx,
                    );
                }
            }
            Entry::Reveal(folder) => cx.reveal_path(&folder),
            Entry::CopyPath(folder) => {
                cx.write_to_clipboard(ClipboardItem::new_string(folder.display().to_string()))
            }
            Entry::Forget(folder) => {
                let project = self.session.update(cx, |session, cx| {
                    session.prefs.recent = forget_folder(&session.prefs.recent, &folder);
                    session.save_prefs();
                    cx.notify();
                    session.project.clone()
                });
                crate::commands::refresh_menus(project.as_deref(), cx);
            }
            Entry::NewConversation(folder) => self.open_in(folder, None, window, cx),
            Entry::OpenConversation(folder, id) => self.open_in(folder, Some(id), window, cx),
        }
        if self.menu_focus.is_focused(window) {
            if from_keys {
                self.give_back_on_key_up = Some(back);
            } else if let Some(back) = back {
                window.focus(&back);
            }
        }
        cx.notify();
    }

    /// Opens conversation `id` (None: a new one) of `folder`: here, at
    /// once; another folder's project opens in place of this window first.
    fn open_in(
        &mut self,
        folder: PathBuf,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let here = self
            .session
            .read(cx)
            .project
            .as_deref()
            .is_some_and(|open| crate::app::same_folder(open, &folder));
        if here {
            self.session.update(cx, |session, cx| {
                session.open_conversation(id);
                cx.notify();
            });
            cx.emit(SidebarEvent::Chosen);
            return;
        }
        window.dispatch_action(
            Box::new(OpenRecent {
                path: folder.to_string_lossy().into_owned().into(),
            }),
            cx,
        );
        open_conversation_in(folder, id, cx);
    }

    /// Remembers the conversations listed (`listed`: as the bridge listed
    /// them); those new to a list already shown fade in.
    fn note_arrivals(&mut self, listed: bool, cx: &mut Context<Self>) {
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

    /// Edits the title of conversation `id` where it is.
    pub fn begin_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self
            .session
            .read(cx)
            .state
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .map(|conversation| {
                (
                    conversation.title.clone(),
                    conversation.label().to_owned(),
                    conversation.archived,
                )
            })
        else {
            return;
        };
        let (title, label, archived) = title;
        if self.renaming.is_some() {
            self.finish_rename(true, false, cx);
        }
        // Its row comes into sight to be renamed in: unfolded, unfiltered.
        if archived && !self.archived_open {
            self.archived_open = true;
            self.unfoldings += 1;
        }
        if !matches(&label, &self.query) {
            self.search.update(cx, |field, cx| field.clear(cx));
            self.query.clear();
        }
        self.menu = None;
        self.menu_return = None;
        self.renaming = Some(id.to_owned());
        // The whole title is chosen, to type over.
        self.rename.update(cx, |field, cx| {
            field.set_text(title, cx);
            field.choose_all(cx);
        });
        self.rename.read(cx).focus(window);
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

    // ------------------------------------------------------------ the edge

    /// A press on the edge: the second of a double-click puts the width
    /// back, and stops the first from putting the sidebar away.
    fn edge_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let second = is_second_press(self.edge_clicked, Instant::now(), event.click_count);
        if second {
            self.edge_hide = None;
            self.edge_clicked = None;
            self.session.update(cx, |session, cx| {
                session.prefs.sidebar_width = layout::SIDEBAR;
                session.save_prefs();
                cx.notify();
            });
        }
        self.edge_press = Some(EdgePress {
            from: event.position,
            moved: false,
            second,
        });
        cx.notify();
    }

    /// The pointer moved with the edge held: past a press, the sidebar is
    /// as wide as the pointer is far in, with no motion.
    fn edge_moved(&mut self, at: Point<Pixels>, window_width: Pixels, cx: &mut Context<Self>) {
        let Some(press) = self.edge_press.as_mut() else {
            return;
        };
        if !press.moved && f32::from((at.x - press.from.x).abs()) < layout::DRAG_START {
            return;
        }
        press.moved = true;
        self.edge_hide = None;
        self.edge_clicked = None;
        let most = layout::sidebar_most(f32::from(window_width));
        let width = f32::from(at.x).clamp(layout::SIDEBAR_LEAST, most);
        self.session.update(cx, |session, cx| {
            if (session.prefs.sidebar_width - width).abs() > f32::EPSILON {
                session.prefs.sidebar_width = width;
                cx.notify();
            }
        });
        cx.notify();
    }

    /// The edge let go: a drag keeps its width; a press waits
    /// `DOUBLE_CLICK` for a second, and with none puts the sidebar away.
    fn edge_released(&mut self, cx: &mut Context<Self>) {
        let Some(press) = self.edge_press.take() else {
            return;
        };
        if press.moved {
            self.session.read(cx).save_prefs();
        } else if !press.second {
            self.edge_clicked = Some(Instant::now());
            self.edge_hide = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(scene::DOUBLE_CLICK).await;
                let _ = this.update(cx, |this, cx| {
                    this.edge_clicked = None;
                    cx.emit(SidebarEvent::Hide);
                });
            }));
        }
        cx.notify();
    }

    /// The sidebar's edge: dragged, it sets how wide the sidebar is;
    /// pressed, it puts it away; pressed twice, the width goes back.
    /// Nothing shows until the pointer is over it; then a line, in the
    /// accent while it is dragged.
    fn edge(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pressed = self.edge_press.is_some();
        let dragging = self.edge_press.as_ref().is_some_and(|press| press.moved);
        let line = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(layout::SIDEBAR_GRIP / 2.0 - 1.0))
            .w(px(2.0))
            .bg(if dragging {
                theme::accent()
            } else {
                theme::lineh()
            })
            .when(!pressed, |line| {
                line.invisible()
                    .group_hover("sidebar-edge", |line| line.visible())
            });
        let mut edge = div()
            .id("sidebar-edge")
            .group("sidebar-edge")
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(-layout::SIDEBAR_GRIP / 2.0))
            .w(px(layout::SIDEBAR_GRIP))
            .cursor_col_resize()
            .child(line)
            .child(ui::tip_zone(
                "sidebar-edge-tip",
                (!pressed).then(|| ui::Tip::new(EDGE_TIP)),
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| this.edge_down(event, cx)),
            )
            // Let go over it or anywhere else.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.edge_released(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.edge_released(cx)),
            );
        if pressed {
            // While it is held, the pointer is followed wherever it goes.
            let this = cx.weak_entity();
            edge = edge.child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble {
                                let width = window.viewport_size().width;
                                let _ = this.update(cx, |this, cx| {
                                    this.edge_moved(event.position, width, cx)
                                });
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            );
        }
        edge
    }

    // ------------------------------------------------------------ menus

    /// The menu `menu` as it shows, `width` wide: its header if it has one,
    /// its items, the one the arrows are on lit.
    fn menu_view(
        &self,
        menu: &SidebarMenu,
        header: Option<SharedString>,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entries = self.entries_of(menu, cx);
        let mut panel = menu_panel()
            .id("sidebar-menu")
            .w(px(width))
            .track_focus(&self.menu_focus)
            .key_context(MENU)
            .on_action(cx.listener(|this, _: &MenuUp, _, cx| this.step_menu(false, cx)))
            .on_action(cx.listener(|this, _: &MenuDown, _, cx| this.step_menu(true, cx)))
            .on_action(
                cx.listener(|this, _: &MenuChoose, window, cx| this.choose_in_menu(window, cx)),
            )
            .on_action(cx.listener(|this, _: &MenuClose, window, cx| this.menu_escape(window, cx)))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.menu_pressed_out(cx)));
        if let Some(header) = header {
            panel = panel.child(div().px(px(10.0)).pt(px(4.0)).pb(px(6.0)).child(ui::line(
                header,
                text::META,
                theme::ink3(),
            )));
        }
        for (index, entry) in entries.into_iter().enumerate() {
            if entry.separated {
                panel = panel.child(menu_separator());
            }
            let chosen = self.menu_chosen == Some(index);
            let mut item = menu_item(
                ElementId::Name(format!("sidebar-menu-{index}").into()),
                entry.words.clone(),
                entry.icon,
                entry.danger,
                entry.disabled,
            )
            .when(chosen, |item| item.bg(theme::popup_hover()))
            .when(entry.checked, |item| {
                item.child(div().flex_1())
                    .child(Icon::Check.view(14.0, theme::ink2()))
            });
            if !entry.disabled {
                let run = entry.entry.clone();
                item = item.on_click(cx.listener(move |this, _, window, cx| {
                    this.run_entry(run.clone(), false, window, cx)
                }));
            }
            // A folder's path, the kit's tip.
            panel = panel.child(ui::tip_on(item, entry.tip.clone().map(ui::Tip::new)));
        }
        // It rises into place as it fades in, as the kit's menus do.
        let rise = scene::MENU_RISE;
        panel
            .relative()
            .with_animation(
                ElementId::Name(format!("sidebar-menu-{menu:?}").into()),
                motion::animation(scene::FADE),
                move |menu, t| menu.opacity(t).top(px(rise * (1.0 - t))),
            )
            .into_any_element()
    }

    /// What takes the keys in the sidebar (in key context `context`, an
    /// [`ITEM`]): Tab reaches it, a ring shows round it while it has them,
    /// and Enter or Space presses it.
    fn keyed(element: Stateful<Div>, context: &'static str) -> Stateful<Div> {
        element
            .tab_index(0)
            .key_context(context)
            .border_1()
            .border_color(theme::with_alpha(theme::accent(), 0.0))
            .focus(|style| style.border_color(theme::accent()))
            // A press of the pointer leaves the keys where they were.
            .capture_any_mouse_down(|_, window, _| window.prevent_default())
    }

    fn row(
        &self,
        conversation: &ConversationSummary,
        index: usize,
        state: RowState,
        now: i64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let session = self.session.read(cx);
        let chosen = session.conversation.as_deref() == Some(conversation.id.as_str());
        let working = state == RowState::Working;
        let id = conversation.id.clone();
        let menu = SidebarMenu::Row(id.clone());
        let menu_open = self.menu_is(&menu);
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
        if working && !motion::reduced() && self.shown {
            // A band of light passing over the title, then resting; drawn
            // again TITLE_SHIMMER_STEPS times a pass, not every frame.
            let phase = shimmer_step(motion::phase(self.shimmer_since, scene::TITLE_SHIMMER));
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

        let row = Self::keyed(
            div().id(ElementId::Name(format!("conversation-{id}").into())),
            ROW_KEYS,
        )
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
        .child(words)
        .when_some(dot.filter(|_| !editing), |row, color| {
            row.child(ui::dot(color, 6.0))
        })
        // The whole title and when it last changed, the kit's tip, under
        // what the row holds (its ⋯ keeps its press).
        .child(ui::tip_zone(
            ElementId::Name(format!("conversation-tip-{id}").into()),
            (!editing && !menu_open).then(|| ui::Tip::new(tip.clone())),
        ))
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
                                let menu = menu.clone();
                                move |this, _: &MouseDownEvent, window, cx| {
                                    this.toggle_menu(menu.clone(), false, window, cx)
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
        // A right-click opens its menu too; Shift+F10 or the Menu key with
        // the row holding the keys.
        .on_mouse_down(
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
            let id = id.clone();
            move |this, _: &RenameRow, window, cx| this.begin_rename(&id, window, cx)
        }))
        .on_action(cx.listener({
            let id = id.clone();
            move |_, _: &DeleteRow, _, cx| {
                // Not while Kerf works in it; asked first, over the window.
                if !working {
                    cx.emit(SidebarEvent::Delete(id.clone()));
                }
            }
        }))
        .on_click(cx.listener({
            let id = id.clone();
            move |this, _, _, cx| {
                if this.renaming.as_deref() == Some(id.as_str()) {
                    return;
                }
                this.menu_pressed_out(cx);
                this.session.update(cx, |session, cx| {
                    session.open_conversation(Some(id.clone()));
                    cx.notify();
                });
                cx.emit(SidebarEvent::Chosen);
            }
        }));
        let with_menu = div().relative().child(row).when(menu_open, |wrapper| {
            // Over the rows after it, and past the list's edge.
            wrapper.child(self.placed_menu(&menu, 30.0, 180.0, cx))
        });
        let arrived = self
            .arrivals
            .get(&id)
            .filter(|at| at.elapsed() < scene::APPEAR)
            .copied();
        if conversation.archived {
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

    /// The open folder's name over its conversations, a chevron after it:
    /// pressed, the folders and the ways to another.
    fn folder_heading(&self, project: String, cx: &mut Context<Self>) -> impl IntoElement {
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

    /// Another folder worked on here: pressed, it opens in place of this
    /// project; its ⋯ or a right-click has its menu; its chevron (Right and
    /// Left from the keys) unfolds and folds its conversations, read from
    /// where its project keeps them, each opening its project on it.
    fn other_folder(&self, index: usize, folder: PathBuf, cx: &mut Context<Self>) -> AnyElement {
        let name = folder_name(&folder);
        let menu = SidebarMenu::Other(folder.clone());
        let menu_open = self.menu_is(&menu);
        let unfolded = self.unfolded.get(&folder);
        let fold = folder.clone();
        let chevron = div()
            .id(ElementId::Name(format!("folder-fold-{index}").into()))
            .flex_none()
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme::radius::SMALL))
            .hover(|chevron| chevron.bg(theme::hover2()))
            .child(
                if unfolded.is_some() {
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
            }));
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
        .child(
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
                ),
        )
        .on_mouse_down(
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
            let folder = folder.clone();
            move |this, _: &Unfold, _, cx| this.fold(&folder, Some(false), cx)
        }))
        .on_action(cx.listener({
            let folder = folder.clone();
            move |this, _: &Fold, _, cx| this.fold(&folder, Some(true), cx)
        }))
        .on_click(cx.listener({
            let folder = folder.clone();
            move |this, _, window, cx| {
                // Another folder opens in a window of its own, in place of this one.
                this.run_entry(Entry::Open(folder.clone()), false, window, cx)
            }
        }));
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

    /// `folder`'s conversations, read again (unfolding it, `Some(false)`),
    /// or put away (folding it, `Some(true)`); None: the other way. They
    /// are read away from the window (a folder can keep many long
    /// conversations), and show once they are, unless it was folded again.
    ///
    /// The fold is kept as it was left, for the next launch, as Qt's
    /// sidebar kept it (`sidebar/folded`): for the folders still listed only.
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

    /// The other folders left unfolded by hand (here, or in the Qt editor
    /// before it), unfolded again as the sidebar is built.
    fn unfold_as_left(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let current = session.project.clone();
        let unfolded: Vec<PathBuf> = session
            .prefs
            .recent
            .iter()
            .filter(|folder| {
                !current
                    .as_deref()
                    .is_some_and(|current| crate::app::same_folder(current, folder))
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
        let mut list = div().flex().flex_col().gap(px(2.0));
        if conversations.is_empty() {
            list = list.child(div().pl(px(42.0)).py(px(4.0)).child(ui::label(
                "No conversations yet",
                text::SIDE_NOTE,
                theme::ink3(),
            )));
        }
        for (at, conversation) in conversations.iter().take(count).enumerate() {
            let title: SharedString = conversation.label().to_owned().into();
            let tip: SharedString = match parse_time(&conversation.updated) {
                Some(when) => format!("{title} · {}", relative_time(when, now)).into(),
                None => title.clone(),
            };
            let open = Entry::OpenConversation(folder.to_path_buf(), conversation.id.clone());
            let row = Self::keyed(
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
            .on_click(cx.listener(move |this, _, window, cx| {
                this.run_entry(open.clone(), false, window, cx)
            }));
            list = list.child(row);
        }
        if let Some(more) = more {
            let words = match more {
                More::Show(count) => format!("Show {count} more"),
                More::Less => "Show less".to_owned(),
            };
            let folder = folder.to_path_buf();
            list = list.child(
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
                })),
            );
        }
        list.into_any_element()
    }

    /// The account at the foot: a disc with its first letter, who chat
    /// runs on, and under it the plan or the model. Pressed, the app's menu
    /// opens above it.
    fn account(&self, title: String, below: String, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.app_menu_open;
        let letter = title
            .chars()
            .find(|ch| ch.is_alphanumeric())
            .map(|ch| ch.to_uppercase().collect::<String>())
            .unwrap_or_default();
        Self::keyed(
            trigger(
                "account",
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.close_menus(cx);
                    cx.emit(SidebarEvent::AppMenu { from_keys: false });
                }),
            ),
            ITEM,
        )
        .flex()
        .items_center()
        .gap(px(8.0))
        .flex_1()
        .min_w_0()
        .h(px(36.0))
        .px(px(6.0))
        .rounded(px(theme::radius::CARD))
        .cursor_pointer()
        .when(open, |account| account.bg(theme::raised()))
        .hover(|account| account.bg(theme::raised()))
        .child(
            theme::type_style(div(), text::CONTROL_STRONG)
                .flex_none()
                .size(px(24.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::on())
                .text_color(theme::ink2())
                .child(letter),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(ui::line(title, text::CONTROL_STRONG, theme::ink2()))
                .child(ui::line(below, text::META, theme::ink3())),
        )
        .on_action(cx.listener(|this, _: &OpenMenu, _, cx| {
            this.close_menus(cx);
            cx.emit(SidebarEvent::AppMenu { from_keys: true });
        }))
        .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
            if event.is_keyboard() {
                this.close_menus(cx);
                cx.emit(SidebarEvent::AppMenu { from_keys: true });
            }
        }))
    }
}

impl Render for SidebarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.arrivals.retain(|_, at| at.elapsed() < scene::APPEAR);
        // A menu that had the keys has gone: they go back where they were.
        if let Some(back) = self.refocus.take() {
            if self.menu_focus.is_focused(window) {
                cx.defer_in(window, move |_, window, _| window.focus(&back));
            }
        }
        // What a menu's choice did took the keys: they stay there.
        if self.give_back_on_key_up.is_some() && !self.menu_focus.is_focused(window) {
            self.give_back_on_key_up = None;
        }
        // A menu gone by a key still down holds the keys, unseen, until it
        // comes up; Enter and Space press nothing meanwhile.
        let key_down_after_choice = self.give_back_on_key_up.is_some().then(|| {
            div()
                .absolute()
                .size_0()
                .track_focus(&self.menu_focus)
                .key_context(ITEM)
                .on_key_up(
                    cx.listener(|this, _: &KeyUpEvent, window, cx| this.give_back_keys(window, cx)),
                )
        });
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
        // The light over a working title steps on while the sidebar shows.
        let shimmering = busy && self.shown && !motion::reduced();
        if shimmering && self.shimmer_tick.is_none() {
            self.shimmer_tick = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(scene::TITLE_SHIMMER / TITLE_SHIMMER_STEPS)
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }));
        } else if !shimmering {
            self.shimmer_tick = None;
        }
        let open_conversation = session.state.open_conversation.clone();
        let proposals = session.state.snapshot.proposals.clone();
        let settled = session.state.settled.clone();
        // Who chat runs on, at the foot.
        let (account_title, account_below, _) = account_of(session, cx);
        // The other folders worked on here, newest first.
        let current = session.project.clone();
        let others: Vec<PathBuf> = session
            .prefs
            .recent
            .iter()
            .filter(|project| {
                !current
                    .as_deref()
                    .is_some_and(|current| crate::app::same_folder(current, project))
                    && project.is_dir()
            })
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
        let heading = self.folder_heading(project, cx);
        let other_rows: Vec<AnyElement> = if query.is_empty() {
            others
                .into_iter()
                .enumerate()
                .map(|(index, folder)| self.other_folder(index, folder, cx))
                .collect()
        } else {
            Vec::new()
        };
        let account = self.account(account_title, account_below, cx);
        let edge = self.edge(cx);

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::sidebar())
            .border_r_1()
            .border_color(theme::line())
            .key_context(CONTEXT)
            .on_action(cx.listener(|_, _: &FocusNext, window, _| window.focus_next()))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, _| window.focus_prev()))
            .children(key_down_after_choice)
            .child(
                div()
                    .h(px(layout::TOOLBAR))
                    .flex_none()
                    .flex()
                    .items_center()
                    // On macOS outside full screen the window buttons sit
                    // here, over the sidebar (Qt's toolbar_wiring), so
                    // Hide sidebar starts past them, and the row's empty
                    // stretch moves and zooms the window.
                    .pl(px(crate::app::title_bar_lead(window)))
                    .pr(px(8.0))
                    .gap(px(4.0))
                    .child(
                        icon_button("hide-sidebar", Icon::Sidebar)
                            .tooltip("Hide sidebar")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::Hide))),
                    )
                    .child(crate::app::title_bar_stretch(div().flex_1().h_full())),
            )
            .child(
                div()
                    .px(px(8.0))
                    .pb(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        Self::keyed(div().id("new-conversation"), ITEM)
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
                                cx.emit(SidebarEvent::Chosen);
                            })),
                    )
                    .child(self.search_row(cx)),
            )
            .child(heading)
            .child(
                div()
                    .id("conversations")
                    .track_scroll(&self.list_scroll)
                    .flex_1()
                    .min_h_0()
                    .pt(px(4.0))
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
                            Self::keyed(div().id("show-more"), ITEM)
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
                    .when(!other_rows.is_empty(), |list| {
                        list.child(div().px(px(18.0)).pt(px(18.0)).pb(px(4.0)).child(ui::label(
                            "Other folders",
                            text::META,
                            theme::ink3(),
                        )))
                        .children(other_rows)
                    })
                    .when(archived_count > 0, |list| {
                        list.child(
                            Self::keyed(div().id("archived"), ITEM)
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
            // Over the list: a fade where a row is cut at its top, and at
            // its foot while there is more.
            .child(ui::scroll_fade(&self.list_scroll, theme::sidebar()))
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
                    .child(account)
                    // A Tab stop of the kit's itself, with its ring.
                    .child(
                        icon_button("settings", Icon::Gear)
                            .tooltip("Settings")
                            .key(crate::commands::shown_keys("Ctrl+,"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::Settings))),
                    ),
            )
            .child(edge)
            .on_mouse_down(MouseButton::Right, |_, _, _| {})
    }
}
