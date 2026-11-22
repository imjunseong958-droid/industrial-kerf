//! A menu's rows as data ([`MenuItem`]: words, a key, a tick, a submenu, what
//! choosing does), and how the keys move among them: the row the arrows or a
//! letter light, what stays lit as the rows change, and how tall they stand.

use crate::icons::Icon;
use crate::theme::layout;
use gpui::{App, SharedString, Window};
use std::rc::Rc;

/// What choosing a row of a [`Menu`](super::Menu) does
/// ([`MenuItem::on_choose`]).
type MenuHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// One row of a [`Menu`](super::Menu): its words, and what it does chosen;
/// or a submenu of rows that opens beside it.
#[derive(Clone)]
pub struct MenuItem {
    pub(super) words: SharedString,
    pub(super) icon: Option<Icon>,
    pub(super) key: Option<SharedString>,
    pub(super) checked: bool,
    pub(super) disabled: bool,
    pub(super) danger: bool,
    pub(super) separated: bool,
    pub(super) tip: Option<SharedString>,
    pub(super) submenu: Vec<MenuItem>,
    pub(super) run: Option<MenuHandler>,
}

impl MenuItem {
    pub fn new(words: impl Into<SharedString>) -> Self {
        Self {
            words: words.into(),
            icon: None,
            key: None,
            checked: false,
            disabled: false,
            danger: false,
            separated: false,
            tip: None,
            submenu: Vec::new(),
            run: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Its key, quiet at the row's end.
    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// A tick at its end: what is in use.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Greyed: it cannot be chosen now (its tip may say why).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// In brick: what cannot be undone.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// A line before it, between its group and the one before.
    pub fn separated(mut self) -> Self {
        self.separated = true;
        self
    }

    pub fn tip(mut self, words: impl Into<SharedString>) -> Self {
        self.tip = Some(words.into());
        self
    }

    /// Rows that open beside it (Right, or the pointer resting on it).
    pub fn submenu(mut self, items: Vec<MenuItem>) -> Self {
        self.submenu = items;
        self
    }

    /// What choosing it does, once the menu has gone and given the keys back.
    pub fn on_choose(mut self, run: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.run = Some(Rc::new(run));
        self
    }

    pub fn words(&self) -> &SharedString {
        &self.words
    }

    /// Whether it can be chosen (or opened, with a submenu).
    pub fn enabled(&self) -> bool {
        !self.disabled
    }

    /// The rows of its submenu (none for a row that does something).
    pub fn rows(&self) -> &[MenuItem] {
        &self.submenu
    }
}

/// The next row of `enabled` from `from` (none lit: the first or last),
/// round past either end; none when no row can be chosen.
pub fn menu_step(enabled: &[bool], from: Option<usize>, down: bool) -> Option<usize> {
    let count = enabled.len();
    if count == 0 {
        return None;
    }
    let start = match (from, down) {
        (None, true) => count - 1,
        (None, false) => 0,
        (Some(at), _) => at.min(count - 1),
    };
    (1..=count)
        .map(|step| {
            if down {
                (start + step) % count
            } else {
                (start + count - step % count) % count
            }
        })
        .find(|at| enabled[*at])
}

/// The row a typed `letter` goes to: the next after `from`, round past the
/// end, whose words start with it (any case), that can be chosen.
pub fn menu_letter(
    words: &[&str],
    enabled: &[bool],
    from: Option<usize>,
    letter: char,
) -> Option<usize> {
    let count = words.len().min(enabled.len());
    if count == 0 {
        return None;
    }
    let letter = letter.to_lowercase().collect::<String>();
    let start = from.map_or(count - 1, |at| at.min(count - 1));
    (1..=count).map(|step| (start + step) % count).find(|at| {
        enabled[*at]
            && words[*at]
                .trim_start()
                .to_lowercase()
                .starts_with(letter.as_str())
    })
}

/// How tall a menu's panel of `items` is: its rows (`MENU_ROW` each), the
/// lines between their groups, its padding and its edge.
pub fn menu_height(items: &[MenuItem]) -> f32 {
    let separators = items.iter().filter(|item| item.separated).count() as f32;
    layout::MENU_PAD * 2.0
        + 2.0
        + items.len() as f32 * layout::MENU_ROW
        + separators * layout::MENU_SEPARATOR
}

/// The rows at `level` of `items` with `path` lit.
pub(super) fn rows_at<'a>(
    items: &'a [MenuItem],
    path: &[Option<usize>],
    level: usize,
) -> &'a [MenuItem] {
    let mut rows = items;
    for lit in path.iter().take(level) {
        match lit.and_then(|at| rows.get(at)) {
            Some(item) => rows = item.submenu.as_slice(),
            None => return &[],
        }
    }
    rows
}

/// What of `path` still holds over `items` (a menu's rows changed while it
/// was open): each level's lit row while it is there and can be chosen,
/// and the submenus open under it while it still has one.
pub fn kept_path(items: &[MenuItem], path: &[Option<usize>]) -> Vec<Option<usize>> {
    let mut kept: Vec<Option<usize>> = Vec::new();
    let mut rows = items;
    for (level, lit) in path.iter().enumerate() {
        let lit = lit.filter(|at| rows.get(*at).is_some_and(MenuItem::enabled));
        kept.push(lit);
        let opens = lit
            .and_then(|at| rows.get(at))
            .filter(|item| !item.submenu.is_empty());
        match opens {
            Some(item) if level + 1 < path.len() => rows = item.submenu.as_slice(),
            _ => break,
        }
    }
    if kept.is_empty() {
        kept.push(None);
    }
    kept
}
