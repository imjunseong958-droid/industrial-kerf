//! The Media band: the project's media, between the picture and the
//! timeline. It comes down out from under the picture when opened (the
//! workspace moves it).
//!
//! Its strip has the segmented "Media N | Index", whose fill slides from
//! one to the other, then Search, the List/Grid view, Import (the
//! registry's `file.import`, as File ▸ Import Media…) and a close button.
//! Search opens a "Search media" field under the strip that keeps the
//! media whose name holds what is typed; Esc empties and closes it.
//!
//! The Media page is one row of tiles on the well, or a list. A tile shows
//! its media's picture: its poster fitted at its own shape (over a
//! checkerboard when its pixel format has alpha), a sound's waveform, or a
//! film or sound symbol while the artwork comes ([`editor_session::artwork`]); its
//! length in frames ("9s20f"); a dot for media not downloaded or outside
//! the project; "Offline" in red for a file that is gone; and a dashed
//! "Not made" hole for a shot not made yet. Under its name a line says what
//! it is doing or what it is ("Making take 2 · 0:41", "Queued · 1 ahead",
//! "Made · shot · take 1 of 2 · on V1"). A tile being made has amber
//! stripes moving across it; one waiting its turn on the server a dashed
//! amber outline. The arrow keys move the chosen media, and Return puts it
//! in at the playhead, as a double-click does.
//!
//! The Index page lists the chosen media's shots, silences and loudest
//! moments from the engine's media index. Roles, a column at the band's
//! end, has a row for each role of the program's tracks, whose box turns
//! all of them on or off, and Edit roles… to set a role for each track.
//!
//! A tile's menu (a right-click, or its ⋯ on hover) has Insert and
//! Overwrite, Replace Media…, Carry Steps into the Document…, Open Graph,
//! Make… and Make Several or Stop Making, its Takes, and Show in Folder;
//! the empty bin's has Import Media…, Consolidate Media and Cast and
//! Styles…, which a tile's menu ends with too. Each item says in its
//! tooltip what it does, or why it cannot. They are the kit's menus: they
//! take the keys while open (the arrows, Return, a letter, Esc are
//! theirs, not the band's).

use crate::making::{MakeMedia, OpenShotGraph};
use editor_launcher::cast::{CastView, OpenCast};
use editor_session::artwork::{self, paint_waveform};
use editor_session::commands::RunCommand;
use editor_session::session::Session;
use editor_state::model::{self, Source, TimelineShow, Track, TrackKind};
use editor_state::store::{Badge, MediaActivity};
use editor_ui::icons::Icon;
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::motion::{self, Tween};
use editor_ui::theme::{self, motion as scene, text};
use editor_ui::ui::{self, icon_button};
use gpui::{
    anchored, canvas, deferred, div, img, point, prelude::*, px, quad, AnimationExt, AnyElement,
    AnyView, App, BorderStyle, Bounds, Context, Corners, DismissEvent, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, MouseButton, MouseDownEvent, ObjectFit,
    PathBuilder, PathPromptOptions, Pixels, Point, SharedString, Subscription, TextRun, Window,
};
use kerf_bridge::Command;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// A tile's thumb and the gap between tiles; the search row under the
/// strip; Roles' column; a row of the list, and of the Index.
pub use editor_ui::theme::layout::MEDIA_THUMB as THUMB;
use editor_ui::theme::layout::{
    MEDIA_INDEX_ROW as INDEX_ROW, MEDIA_LIST_ROW as LIST_ROW, MEDIA_ROLES_WIDTH as ROLES_WIDTH,
    MEDIA_SEARCH_ROW as SEARCH_ROW, MEDIA_TILE_GAP as TILE_GAP,
};
/// From the thumb to its name, and the name's and the status's lines.
const NAME_TOP: f32 = 4.0;
const LINE: f32 = 16.0;
/// The well's padding at its left and above the tiles (none while the
/// search row takes the room).
const SIDE_PADDING: f32 = 16.0;
const TOP_PADDING: f32 = 12.0;
/// Roles: its rows.
const ROLE_ROW: f32 = 20.0;
/// The checkerboard under a picture with alpha.
const CHECKER: f32 = 8.0;

/// The roles kerfd names, in the order the Roles list offers them.
pub const ENGINE_ROLES: [&str; 5] = ["video", "titles", "dialogue", "music", "effects"];

/// Amber stripes across `bounds`, `phase` (0..1) of the way to the next:
/// what is being made, on its tile, its graph's takes and its clips.
pub(crate) fn paint_stripes(window: &mut Window, bounds: Bounds<Pixels>, phase: f32) {
    let space = scene::STRIPE_SPACE;
    let shift = phase * space;
    let height = f32::from(bounds.size.height);
    let width = f32::from(bounds.size.width);
    let mut x = -height + shift;
    while x < width {
        let mut path = PathBuilder::stroke(px(space * 0.45));
        path.move_to(point(
            bounds.origin.x + px(x),
            bounds.origin.y + bounds.size.height,
        ));
        path.line_to(point(bounds.origin.x + px(x + height), bounds.origin.y));
        if let Ok(path) = path.build() {
            window.paint_path(path, theme::activity::gpu_stripe());
        }
        x += space;
    }
}

// ------------------------------------------------------------ what is said

/// A length as a tile's badge says it, in frames at `fps`: "9s20f" under a
/// minute, "1m05s" under an hour, else "1h02m".
pub fn badge_text(seconds: f64, fps: f64) -> String {
    let rate = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        24.0
    };
    let per_second = (rate.round() as i64).max(1);
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    let frames = (seconds * rate).round() as i64;
    let whole = frames / per_second;
    if whole < 60 {
        format!("{whole}s{:02}f", frames % per_second)
    } else if whole < 3600 {
        format!("{}m{:02}s", whole / 60, whole % 60)
    } else {
        format!("{}h{:02}m", whole / 3600, (whole / 60) % 60)
    }
}

/// The line under a tile's name: what `source` is doing (being made,
/// queued, not made yet, offline), wanting making again (`wants`, why),
/// else what it is, which take, and where it is used (`used_on`, track
/// badges): "Made · shot · take 1 of 2 · on V1", "Video · on V1 V2",
/// "Sound · outside the project". `now` is when it is said.
pub fn status_text(
    source: &Source,
    activity: &MediaActivity,
    badge: Option<Badge>,
    wants: Option<&str>,
    used_on: &[String],
    now: Instant,
) -> String {
    let on = (!used_on.is_empty()).then(|| format!("on {}", used_on.join(" ")));
    let joined = |mut parts: Vec<String>| {
        parts.extend(on.clone());
        parts.join(" · ")
    };
    match activity {
        MediaActivity::Making { take, since } => {
            let clock =
                editor_state::chip::elapsed(now.saturating_duration_since(*since).as_secs());
            return if *take > 0 {
                format!("Making take {take} · {clock}")
            } else {
                format!("Making · {clock}")
            };
        }
        MediaActivity::Queued { ahead } => {
            return match ahead {
                Some(ahead) if *ahead > 0 => format!("Queued · {ahead} ahead"),
                _ => "Queued for making".to_owned(),
            };
        }
        MediaActivity::NotMade => return "Not made yet".to_owned(),
        MediaActivity::Offline => return "Offline · file missing".to_owned(),
        MediaActivity::Ready => {}
    }
    match badge {
        Some(Badge::Offline) => {
            return if source.is_made() {
                "Not made yet".to_owned()
            } else {
                "Offline · file missing".to_owned()
            };
        }
        Some(Badge::NotDownloaded) => return joined(vec![Badge::NotDownloaded.words().to_owned()]),
        _ => {}
    }
    if let Some(why) = wants {
        return format!("Wants making · {why}");
    }
    if source.is_made() {
        let mut parts = vec!["Made".to_owned(), "shot".to_owned()];
        if source.takes.is_empty() {
            parts.push("take 1 of 1".to_owned());
        } else if let Some(chosen) = source.chosen_take() {
            parts.push(format!("take {chosen} of {}", source.takes.len()));
        }
        return joined(parts);
    }
    let what = if source.is_picture() {
        if source.duration > 0.0 {
            "Video"
        } else {
            "Still"
        }
    } else {
        "Sound"
    };
    let mut parts = vec![what.to_owned()];
    if badge == Some(Badge::External) {
        parts.push("outside the project".to_owned());
    }
    joined(parts)
}

/// The ink a status line is in: amber while it is made or queued, the
/// brick for a file gone, a step up for what wants a look, else quiet.
pub fn status_ink(activity: &MediaActivity, status: &str) -> gpui::Rgba {
    if matches!(
        activity,
        MediaActivity::Making { .. } | MediaActivity::Queued { .. }
    ) {
        theme::activity::gpu()
    } else if status.starts_with("Offline") {
        theme::neg()
    } else if status.starts_with("Wants") || status.starts_with("Not made") {
        theme::ink2()
    } else {
        theme::ink3()
    }
}

/// A tile's tooltip: its name and length as a timecode, its file, what a
/// double-click does, what it is doing, and what its badge means.
pub fn tile_tip(
    name: &str,
    source: &Source,
    fps: f64,
    status: &str,
    badge: Option<Badge>,
) -> String {
    let mut lines = vec![format!(
        "{name} · {}",
        model::timecode(source.duration, fps)
    )];
    if !status.is_empty() {
        lines.push(status.to_owned());
    }
    if !source.path.is_empty() {
        lines.push(source.path.clone());
    }
    lines.push("Double-click to insert at the playhead".to_owned());
    if let Some(badge) = badge {
        lines.push(badge.tip().to_owned());
    }
    lines.join("\n")
}

/// Whether a media named `name` is kept by the search `query`: by name, as
/// typed, ignoring case.
pub fn matches_search(name: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty() || name.to_lowercase().contains(&query.to_lowercase())
}

/// The media `step` places on from `current` among `ids` (the arrow keys):
/// the first or the last when none is chosen, held at either end.
pub fn step_choice(ids: &[String], current: Option<&str>, step: i32) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let at = current.and_then(|current| ids.iter().position(|id| id == current));
    let next = match at {
        None if step < 0 => ids.len() - 1,
        None => 0,
        Some(at) => (at as i64 + i64::from(step)).clamp(0, ids.len() as i64 - 1) as usize,
    };
    Some(ids[next].clone())
}

// ------------------------------------------------------------------ roles

/// A role of the program's tracks, as a Roles row shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleGroup {
    /// The role as the engine names it, lower case ("dialogue"), or the kind.
    pub key: String,
    /// "Dialogue".
    pub name: String,
    /// Its tracks' ids, in the timeline's order.
    pub tracks: Vec<String>,
    /// Every track of the role is on.
    pub enabled: bool,
}

fn kind_word(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
        TrackKind::Subtitle => "subtitle",
    }
}

/// A role's name as a person reads it: "dialogue" is "Dialogue"; a
/// subtitle track with none is "Captions", another its kind's.
pub fn role_name(role: &str, kind: &str) -> String {
    let mut name = role.trim().to_owned();
    if name.is_empty() {
        name = if kind == "subtitle" {
            "captions".to_owned()
        } else {
            kind.to_owned()
        };
    }
    let mut letters = name.chars();
    match letters.next() {
        None => "Other".to_owned(),
        Some(first) => first.to_uppercase().chain(letters).collect(),
    }
}

/// The Roles rows for `tracks`: one per role, tracks with none under their
/// kind (subtitles under Captions), in the order they first come.
pub fn role_groups(tracks: &[Track]) -> Vec<RoleGroup> {
    let mut groups: Vec<RoleGroup> = Vec::new();
    for track in tracks {
        let kind = kind_word(track.kind);
        let role = track.role.as_deref().unwrap_or_default().trim();
        let key = if role.is_empty() {
            if kind == "subtitle" {
                "captions".to_owned()
            } else {
                kind.to_owned()
            }
        } else {
            role.to_lowercase()
        };
        let at = match groups.iter().position(|group| group.key == key) {
            Some(at) => at,
            None => {
                groups.push(RoleGroup {
                    key,
                    name: role_name(role, kind),
                    tracks: Vec::new(),
                    enabled: true,
                });
                groups.len() - 1
            }
        };
        groups[at].tracks.push(track.id.clone());
        groups[at].enabled &= track.enabled;
    }
    groups
}

/// Track badges with what runs on as a range, "V1–V12 A1", so that a
/// sequence of many tracks stays inside the column. Two in a row are as
/// short said one by one.
pub fn badge_runs(badges: &[String]) -> String {
    let split = |badge: &str| -> Option<(char, u32)> {
        let mut chars = badge.chars();
        let letter = chars.next()?;
        Some((letter, chars.as_str().parse().ok()?))
    };
    let mut runs = Vec::new();
    let mut from = 0;
    while from < badges.len() {
        let mut to = from;
        while to + 1 < badges.len() {
            match (split(&badges[to]), split(&badges[to + 1])) {
                (Some((letter, number)), Some((next_letter, next)))
                    if letter == next_letter && next == number + 1 =>
                {
                    to += 1
                }
                _ => break,
            }
        }
        if to - from >= 2 {
            runs.push(format!("{}–{}", badges[from], badges[to]));
        } else {
            runs.extend(badges[from..=to].iter().cloned());
        }
        from = to + 1;
    }
    runs.join(" ")
}

/// The edit turning each of `tracks` on or off.
pub fn enable_ops(tracks: &[String], enabled: bool) -> Vec<Value> {
    tracks
        .iter()
        .map(|track| json!({"op": "set-track", "track": track, "enabled": enabled}))
        .collect()
}

/// The edit giving each track its role (`""` gives back its kind's).
pub fn role_ops(changes: &[(String, String)]) -> Vec<Value> {
    changes
        .iter()
        .map(|(track, role)| json!({"op": "set-track", "track": track, "role": role}))
        .collect()
}

/// The roles Edit roles offers: the engine's, then the timeline's own.
pub fn role_choices(tracks: &[Track]) -> Vec<String> {
    let mut choices: Vec<String> = ENGINE_ROLES
        .iter()
        .map(|role| role_name(role, ""))
        .collect();
    for track in tracks {
        let role = track.role.as_deref().unwrap_or_default().trim();
        if role.is_empty() {
            continue;
        }
        let name = role_name(role, "");
        if !choices
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&name))
        {
            choices.push(name);
        }
    }
    choices
}

/// A track's kind's own role, as Edit roles names it ("Dialogue" for sound).
pub fn default_role(kind: TrackKind) -> String {
    let role = match kind {
        TrackKind::Audio => "dialogue",
        TrackKind::Video => "video",
        TrackKind::Subtitle => "",
    };
    role_name(role, kind_word(kind))
}

/// What Edit roles sends for a track whose field says `typed`: a listed
/// role by its engine name (lower case), another as typed, nothing for its
/// kind's default; None when that is what it has (`before`, lower case,
/// empty for the default).
pub fn role_change(typed: &str, choices: &[String], before: &str) -> Option<String> {
    let typed = typed.trim();
    let typed = typed.strip_suffix("(default)").map_or(typed, str::trim);
    let role = match choices
        .iter()
        .find(|choice| choice.eq_ignore_ascii_case(typed))
    {
        Some(choice) => choice.to_lowercase(),
        None => typed.to_owned(),
    };
    (!role.eq_ignore_ascii_case(before)).then_some(role)
}

// ------------------------------------------------------------------ index

/// A row of the Index page.
#[derive(Clone, Debug, PartialEq)]
pub enum IndexRow {
    /// "3 shots", "1 silence", "Loudest moments".
    Header(String),
    /// "Shot 2", "00:00:04:00–00:00:09:12", and the second it starts at.
    Row { name: String, meta: String, at: f64 },
}

/// The Index page's rows for a media's `media_index` answer, its times at
/// the media's own `fps`: its shots (the engine's ranges, else the spans
/// between its cuts), its silences and its loudest moments.
pub fn index_rows(index: &Value, duration: f64, fps: f64) -> Vec<IndexRow> {
    let tc = |seconds: f64| model::timecode(seconds, fps);
    let mut rows = Vec::new();
    let mut shots: Vec<(f64, f64)> = index["ranges"]
        .as_array()
        .map(|ranges| {
            ranges
                .iter()
                .filter_map(|range| Some((range["start"].as_f64()?, range["end"].as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    if shots.is_empty() {
        let mut cuts: Vec<f64> = index["cuts"]
            .as_array()
            .map(|cuts| cuts.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        cuts.sort_by(f64::total_cmp);
        let mut start = 0.0;
        for cut in &cuts {
            if *cut > start {
                shots.push((start, *cut));
            }
            start = *cut;
        }
        if !cuts.is_empty() && duration > start {
            shots.push((start, duration));
        }
    }
    let count = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    if !shots.is_empty() {
        rows.push(IndexRow::Header(count(shots.len(), "shot", "shots")));
        for (number, (start, end)) in shots.iter().enumerate() {
            rows.push(IndexRow::Row {
                name: format!("Shot {}", number + 1),
                meta: format!("{}–{}", tc(*start), tc(*end)),
                at: *start,
            });
        }
    }
    let silences: Vec<(f64, f64)> = index["silences"]
        .as_array()
        .map(|spans| {
            spans
                .iter()
                .filter_map(|span| Some((span[0].as_f64()?, span[1].as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    if !silences.is_empty() {
        rows.push(IndexRow::Header(count(
            silences.len(),
            "silence",
            "silences",
        )));
        for (number, (start, end)) in silences.iter().enumerate() {
            rows.push(IndexRow::Row {
                name: format!("Silence {}", number + 1),
                meta: format!("{}–{}", tc(*start), tc(*end)),
                at: *start,
            });
        }
    }
    let loudest: Vec<f64> = index["loudest"]
        .as_array()
        .map(|times| times.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    if !loudest.is_empty() {
        rows.push(IndexRow::Header("Loudest moments".to_owned()));
        for (number, at) in loudest.iter().enumerate() {
            rows.push(IndexRow::Row {
                name: format!("Loudest {}", number + 1),
                meta: tc(*at),
                at: *at,
            });
        }
    }
    rows
}

/// Where the program shows second `at` of media `media`: the first clip
/// of it whose source range holds that second, at the place in the clip
/// it shows it. None where none does.
pub fn program_time_of(timeline: &model::Timeline, media: &str, at: f64) -> Option<f64> {
    timeline
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .filter(|clip| clip.source_id.as_deref() == Some(media) && clip.duration > 0.0)
        .find_map(|clip| {
            let first = clip.source_at(0.0)?;
            let last = clip.source_at(clip.duration)?;
            let (low, high) = if first <= last {
                (first, last)
            } else {
                (last, first)
            };
            if at < low || at > high || (last - first).abs() < 1e-9 {
                return None;
            }
            Some(clip.timeline_start + (at - first) / (last - first) * clip.duration)
        })
}

// -------------------------------------------------------------- the band

/// The band's two pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Media,
    Index,
}

/// A menu while open: of a media, or of the empty bin; where it was asked
/// for; and the kit's menu itself, which has the keys while it is open.
struct OpenMenu {
    media: Option<String>,
    at: Point<Pixels>,
    menu: Entity<ui::Menu>,
}

/// Make Several ▸: two, three or four takes of the same shot to choose
/// among, each with every seed of its steps one on from the last.
fn several(media: &str, connected: bool) -> ui::MenuItem {
    let rows = [2u32, 3, 4]
        .into_iter()
        .map(|rolls| {
            let media: SharedString = media.to_owned().into();
            ui::MenuItem::new(format!("{rolls} Takes…"))
                .tip(format!(
                    "Make {rolls} takes of {media}, each with every seed of its steps one on from the last; the command is shown first"
                ))
                .on_choose(move |window, cx| {
                    window.dispatch_action(
                        Box::new(MakeMedia {
                            media: media.clone(),
                            rolls,
                        }),
                        cx,
                    );
                })
        })
        .collect();
    ui::MenuItem::new("Make Several")
        .icon(Icon::Sparkle)
        .disabled(!connected)
        .tip(if connected {
            "Several takes of the same shot to choose among"
        } else {
            "Kerf is not connected"
        })
        .submenu(rows)
}

/// The key context the band's tiles answer in.
const CONTEXT: &str = "MediaBand";

// The band's own keys, while it has them: the arrow keys move the chosen
// media, Return or Enter puts it in at the playhead, and Esc puts away its
// menu or its search before the window's Esc (which puts the band away).
gpui::actions!(media_band, [PreviousMedia, NextMedia, InsertMedia, PutAway]);

/// Binds the band's keys, again whenever a keymap bound since cleared them.
fn bind_keys(cx: &mut App) {
    if editor_launcher::settings::escape_bound(cx, &PutAway) {
        return;
    }
    cx.bind_keys([
        gpui::KeyBinding::new("left", PreviousMedia, Some(CONTEXT)),
        gpui::KeyBinding::new("up", PreviousMedia, Some(CONTEXT)),
        gpui::KeyBinding::new("right", NextMedia, Some(CONTEXT)),
        gpui::KeyBinding::new("down", NextMedia, Some(CONTEXT)),
        gpui::KeyBinding::new("enter", InsertMedia, Some(CONTEXT)),
        gpui::KeyBinding::new("escape", PutAway, Some(CONTEXT)),
    ]);
}

pub struct MediaBand {
    session: Entity<Session>,
    focus: FocusHandle,
    menu: Option<OpenMenu>,
    tab: Tab,
    /// The segmented control's fill: 0 under Media, 1 under Index.
    tab_fill: Tween,
    list: bool,
    searching: bool,
    search: Entity<TextInput>,
    query: String,
    /// A dialog the band shows over the window itself, where the
    /// workspace does not take [`OpenCast`] or [`EditRoles`].
    hosted: Option<(AnyView, u64)>,
    hosts: u64,
    /// Roles' column, for its fades.
    roles_scroll: gpui::ScrollHandle,
    /// Each command's first key in the keymap they were read for.
    keys: (String, HashMap<String, String>),
    _subscriptions: Vec<Subscription>,
}

gpui::actions!(kerf, [EditRoles]);

impl MediaBand {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx, "Search media").with_style(text::CONTROL));
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &search,
                window,
                |this, field, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Changed => {
                            this.query = field.read(cx).text().to_owned();
                            cx.notify();
                        }
                        // Esc empties the field and puts it away.
                        InputEvent::Cancel => this.close_search(window, cx),
                        InputEvent::Submit => window.focus(&this.focus),
                    }
                },
            ),
        ];
        Self {
            session,
            focus: cx.focus_handle().tab_stop(true),
            menu: None,
            tab: Tab::Media,
            tab_fill: Tween::at(0.0),
            list: false,
            searching: false,
            search,
            query: String::new(),
            hosted: None,
            hosts: 0,
            roles_scroll: gpui::ScrollHandle::new(),
            keys: (String::new(), HashMap::new()),
            _subscriptions: subscriptions,
        }
    }

    /// Find in Bin (`bin.search`, ⌘F): the Media page, its search field
    /// shown and the cursor in it. The workspace calls it once the band is
    /// open.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = true;
        self.set_tab(Tab::Media, cx);
        self.search.update(cx, |search, cx| search.choose_all(cx));
        self.search.read(cx).focus(window);
        cx.notify();
    }

    /// Empties the search field and puts it away; the tiles have the keys.
    pub fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.searching = false;
        self.query.clear();
        self.search.update(cx, |search, cx| search.clear(cx));
        window.focus(&self.focus);
        cx.notify();
    }

    /// Switch Bin View (`bin.toggleView`, ⌘⌥2): tiles or a list. The
    /// workspace calls it.
    pub fn toggle_view(&mut self, cx: &mut Context<Self>) {
        self.list = !self.list;
        cx.notify();
    }

    /// Shows the Media page or the Index; the fill slides to it.
    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.tab_fill
            .go(if tab == Tab::Index { 1.0 } else { 0.0 }, scene::SEGMENT);
        if tab == Tab::Index {
            self.ask_index(cx);
        }
        cx.notify();
    }

    fn ask_index(&mut self, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            if let Some(media) = session.chosen_media.clone() {
                session.ask_media_index(&media);
            }
        });
    }

    /// Chooses media `id`, as a press or an arrow key does.
    fn choose(&mut self, id: &str, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.chosen_media = Some(id.to_owned());
            cx.notify();
        });
        if self.tab == Tab::Index {
            self.ask_index(cx);
        }
        cx.notify();
    }

    /// Command `id`'s first key as the system writes it ("⌘I", "Ctrl+I").
    fn shown_key(&mut self, id: &str, cx: &App) -> Option<String> {
        let keymap = self.session.read(cx).prefs.keymap.wire();
        if self.keys.0 != keymap {
            let keys = editor_session::commands::catalog(keymap)
                .into_iter()
                .filter_map(|command| {
                    let first = command.keys.first()?.clone();
                    Some((command.id, editor_state::shortcuts::shown_keys(&first)))
                })
                .collect();
            self.keys = (keymap.to_owned(), keys);
        }
        self.keys.1.get(id).cloned()
    }

    /// Runs registry command `id` as the window does (its reason said when
    /// it cannot run).
    fn run(id: &'static str, window: &mut Window, cx: &mut App) {
        window.dispatch_action(Box::new(RunCommand::new(id)), cx);
    }

    /// Swaps the file behind `media` for one a person picks: every clip
    /// that uses it keeps its timing.
    fn replace(&mut self, media: String, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = editor_launcher::presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                // It asks "Replace m2". GPUI's chooser takes no filter and no
                // folder to start in, so it neither narrows to media files nor
                // opens in the media's own folder.
                prompt: Some(format!("Replace {media}").into()),
            },
        );
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let answer = chosen.await;
            let _ = session.update(cx, |session, cx| {
                let path = match answer {
                    editor_launcher::presets::Picked::Paths(paths) => paths.into_iter().next(),
                    editor_launcher::presets::Picked::Failed(error) => {
                        session
                            .state
                            .notify(&format!("No file chooser could open: {error}"), true);
                        None
                    }
                    editor_launcher::presets::Picked::Unusable(words) => {
                        session.state.notify(&words, true);
                        None
                    }
                    editor_launcher::presets::Picked::Nothing => None,
                };
                // Any file: kerfd reads it and refuses, in its own words, one
                // that is no media.
                if let Some(path) = path {
                    session.send(Command::MediaReplace {
                        media: media.clone(),
                        path: path.to_string_lossy().into_owned(),
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Has media `media` be its take `path` again: one undoable replace.
    fn use_take(&mut self, media: &str, path: &str, cx: &mut Context<Self>) {
        let command = Command::MediaReplace {
            media: media.to_owned(),
            path: path.to_owned(),
        };
        self.session.update(cx, |session, cx| {
            session.send(command);
            cx.notify();
        });
    }

    /// Opens the menu of `media` (None: the bin's own) where it was asked
    /// for, the kit's: it takes the keys from the band, so the band's own
    /// (the arrows, Return) do nothing under it, and gives them back as it
    /// goes.
    fn open_menu(
        &mut self,
        media: Option<&str>,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((title, items)) = self.menu_items(media, cx) else {
            return;
        };
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx).width(theme::layout::MEDIA_MENU_WIDTH);
            match title {
                Some(title) => menu.header(title),
                None => menu,
            }
        });
        cx.subscribe_in(&menu, window, |this, menu, _: &DismissEvent, _, cx| {
            if this.menu.as_ref().is_some_and(|open| &open.menu == menu) {
                this.menu = None;
                cx.notify();
            }
        })
        .detach();
        self.menu = Some(OpenMenu {
            media: media.map(str::to_owned),
            at,
            menu,
        });
        cx.notify();
    }

    /// The menu open goes, the keys back where they were.
    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.menu.take() {
            open.menu.update(cx, |menu, cx| menu.close(window, cx));
            cx.notify();
        }
    }

    /// Cast and Styles…, over the window.
    fn show_cast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_action_available(&OpenCast, cx) {
            window.dispatch_action(Box::new(OpenCast), cx);
            return;
        }
        let view = cx.new(|cx| CastView::new(self.session.clone(), window, cx));
        self.host(view, window, cx);
    }

    /// Edit roles…, over the window.
    fn edit_roles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_action_available(&EditRoles, cx) {
            window.dispatch_action(Box::new(EditRoles), cx);
            return;
        }
        let view = cx.new(|cx| RolesView::new(self.session.clone(), window, cx));
        self.host(view, window, cx);
    }

    /// Shows `view` over the window from the band, where the workspace
    /// does not open it; it gives the window back by [`DismissEvent`].
    fn host<V: Render + Focusable + EventEmitter<DismissEvent>>(
        &mut self,
        view: Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = view.entity_id();
        cx.subscribe_in(
            &view,
            window,
            move |this, _, _: &DismissEvent, window, cx| {
                if this
                    .hosted
                    .as_ref()
                    .is_some_and(|(open, _)| open.entity_id() == id)
                {
                    this.hosted = None;
                    window.focus(&this.focus);
                    cx.notify();
                }
            },
        )
        .detach();
        // The menu that asked for it goes first (it gave the keys back).
        self.menu = None;
        window.focus(&view.focus_handle(cx));
        self.hosts += 1;
        self.hosted = Some((view.into(), self.hosts));
        cx.notify();
    }

    /// The file behind a media, where the project keeps it.
    fn file_of(&self, source: &Source, cx: &App) -> Option<PathBuf> {
        if source.path.is_empty() {
            return None;
        }
        let path = PathBuf::from(&source.path);
        if path.is_absolute() {
            return Some(path);
        }
        self.session
            .read(cx)
            .project
            .as_ref()
            .map(|project| project.join(path))
    }

    /// The media shown, in order: those whose name the search keeps.
    fn shown(&self, cx: &App) -> Vec<(Source, String)> {
        let timeline = self.session.read(cx).timeline();
        timeline
            .sources
            .iter()
            .map(|source| (source.clone(), timeline.media_name(&source.id)))
            .filter(|(_, name)| matches_search(name, &self.query))
            .collect()
    }

    /// The arrow keys: the chosen media `step` places on among those shown.
    fn step(&mut self, step: i32, cx: &mut Context<Self>) {
        if self.hosted.is_some() {
            return;
        }
        let ids: Vec<String> = self
            .shown(cx)
            .into_iter()
            .map(|(source, _)| source.id)
            .collect();
        let current = self.session.read(cx).chosen_media.clone();
        if let Some(next) = step_choice(&ids, current.as_deref(), step) {
            self.choose(&next, cx);
        }
    }

    /// Esc: the band's menu goes (the menu has the keys while open, and
    /// takes its own Esc), else its search; else the window's Esc.
    fn put_away(&mut self, _: &PutAway, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.close_menu(window, cx);
        } else if self.searching {
            self.close_search(window, cx);
        } else {
            cx.propagate();
        }
    }

    /// A registry command as a row of the menu: its label, its key, and
    /// what it does or why it cannot run, in its tip.
    fn command_row(
        &mut self,
        id: &'static str,
        words: &str,
        icon: Icon,
        tip: &str,
        states: &Value,
        cx: &mut Context<Self>,
    ) -> ui::MenuItem {
        let reason = editor_session::palette::unavailable(states, id).map(str::to_owned);
        let mut item = ui::MenuItem::new(words.to_owned())
            .icon(icon)
            .disabled(reason.is_some())
            .tip(reason.unwrap_or_else(|| tip.to_owned()))
            .on_choose(move |window, cx| Self::run(id, window, cx));
        if let Some(key) = self.shown_key(id, cx) {
            item = item.key(key);
        }
        item
    }

    /// A menu, as the kit's rows: what can be done with `media` as it
    /// stands (Insert, Overwrite, Replace Media…, its making and its takes,
    /// Show in Folder), then what the bin offers; the empty bin's (None)
    /// has only the latter. Its title is the media's name. None for a
    /// media that is gone.
    fn menu_items(
        &mut self,
        media: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<(Option<String>, Vec<ui::MenuItem>)> {
        let this = cx.entity().downgrade();
        let session = self.session.read(cx);
        let connected = session.state.connection == editor_state::store::Connection::Connected;
        let states = kerf_bridge::commands_states(&session.command_context());
        let source = match media {
            Some(media) => Some(session.timeline().source(media)?.clone()),
            None => None,
        };
        let mut items: Vec<ui::MenuItem> = Vec::new();
        let mut title = None;
        let not_connected = "Kerf is not connected";
        if let Some(source) = source {
            let session = self.session.read(cx);
            let making = session.state.makings.contains_key(&source.id);
            let steps = source.steps_in_document();
            let carry = source.uncarried();
            let name = session.media_name(&source.id);
            let file = self.file_of(&source, cx).filter(|file| file.exists());
            let media: SharedString = source.id.clone().into();
            title = Some(name);
            items.push(self.command_row(
                "source.insert",
                "Insert",
                Icon::Plus,
                "Put it in at the playhead, pushing what follows along",
                &states,
                cx,
            ));
            items.push(self.command_row(
                "source.overwrite",
                "Overwrite",
                Icon::Scissors,
                "Put it in at the playhead over what is there",
                &states,
                cx,
            ));
            let replace_reason = editor_session::palette::unavailable(&states, "media.replace")
                .map(str::to_owned)
                .or_else(|| making.then(|| format!("{} is being made", source.id)));
            let replace = source.id.clone();
            let band = this.clone();
            items.push(
                ui::MenuItem::new("Replace Media…")
                    .icon(Icon::Import)
                    .separated()
                    .disabled(replace_reason.is_some())
                    .tip(replace_reason.unwrap_or_else(|| {
                        format!(
                            "Swap the file behind {} for everyone; its clips keep their timing",
                            source.id
                        )
                    }))
                    .on_choose(move |window, cx| {
                        let _ =
                            band.update(cx, |band, cx| band.replace(replace.clone(), window, cx));
                    }),
            );
            if carry {
                // Its steps are still in the recipe file it names: what
                // carrying them writes is shown first, as a making's plan is.
                let carried = media.clone();
                let able = connected && !making;
                let recipe = source.recipe.clone().unwrap_or_default();
                items.push(
                    ui::MenuItem::new("Carry Steps into the Document…")
                        .icon(Icon::Import)
                        .disabled(!able)
                        .tip(if connected {
                            format!("Write the steps of {recipe} under {}'s line in the document, as one edit undo takes back; what would be written is shown first, its takes are kept, and {recipe} is left where it is", source.id)
                        } else {
                            not_connected.to_owned()
                        })
                        .on_choose(move |window, cx| {
                            window.dispatch_action(
                                Box::new(MakeMedia {
                                    media: carried.clone(),
                                    rolls: 1,
                                }),
                                cx,
                            );
                        }),
                );
            }
            if steps {
                let graph = media.clone();
                items.push(
                    ui::MenuItem::new("Open Graph")
                        .icon(Icon::Media)
                        .tip(format!("See how {} is made and set it: its steps in the document and what they read, as a graph", source.id))
                        .on_choose(move |window, cx| {
                            window.dispatch_action(
                                Box::new(OpenShotGraph {
                                    media: graph.clone(),
                                }),
                                cx,
                            );
                        }),
                );
            }
            if making {
                let stop = source.id.clone();
                let band = this.clone();
                items.push(
                    ui::MenuItem::new("Stop Making")
                        .icon(Icon::Stop)
                        .disabled(!connected)
                        .tip(if connected {
                            format!("Stop making {0}: its step is asked to end, no take is kept, and {0} stays as it is", source.id)
                        } else {
                            not_connected.to_owned()
                        })
                        .on_choose(move |_, cx| {
                            let _ = band.update(cx, |band, cx| {
                                band.session.update(cx, |session, _| {
                                    session.send(Command::MediaMakeCancel {
                                        media: stop.clone(),
                                    });
                                });
                            });
                        }),
                );
            } else if steps {
                let make = media.clone();
                let shows = if source.shows.is_empty() {
                    String::new()
                } else {
                    format!("; it shows {}", source.shows.join(", "))
                };
                items.push(
                    ui::MenuItem::new("Make…")
                        .icon(Icon::Sparkle)
                        .disabled(!connected)
                        .tip(if connected {
                            format!("Make {}'s file again by running its steps in the document; the command is shown first{shows}", source.id)
                        } else {
                            not_connected.to_owned()
                        })
                        .on_choose(move |window, cx| {
                            window.dispatch_action(
                                Box::new(MakeMedia {
                                    media: make.clone(),
                                    rolls: 1,
                                }),
                                cx,
                            );
                        }),
                );
                items.push(several(&source.id, connected));
            }
            if !source.takes.is_empty() {
                items.push(self.takes(&source, making, &states, cx));
            }
            let reveal = file.clone();
            items.push(
                ui::MenuItem::new("Show in Folder")
                    .icon(Icon::Folder)
                    .disabled(file.is_none())
                    .tip(if file.is_some() {
                        "Show its file in the system's file manager"
                    } else {
                        "Its file is not on this computer"
                    })
                    .on_choose(move |_, cx| {
                        if let Some(file) = &reveal {
                            cx.reveal_path(file);
                        }
                    }),
            );
        }
        let mut import = self.command_row(
            "file.import",
            "Import Media…",
            Icon::Import,
            "Bring files into the project's media",
            &states,
            cx,
        );
        if media.is_some() {
            import = import.separated();
        }
        items.push(import);
        items.push(self.command_row(
            "collab.consolidate",
            "Consolidate Media",
            Icon::Folder,
            "Copy the media outside the project folder into media/ and point the project at the copies",
            &states,
            cx,
        ));
        items.push(
            ui::MenuItem::new("Cast and Styles…")
                .icon(Icon::Sparkle)
                .tip("Who and what this project's shots show, and how they look, said once for every recipe")
                .on_choose(move |window, cx| {
                    let _ = this.update(cx, |band, cx| band.show_cast(window, cx));
                }),
        );
        Some((title, items))
    }

    /// Takes ▸: every take is kept; the one the media is now is ticked, and
    /// choosing another has the media be that take again (one undoable
    /// replace). Not while it is being made.
    fn takes(
        &self,
        source: &Source,
        making: bool,
        states: &Value,
        cx: &mut Context<Self>,
    ) -> ui::MenuItem {
        let this = cx.entity().downgrade();
        let replace_reason =
            editor_session::palette::unavailable(states, "media.replace").map(str::to_owned);
        let can = replace_reason.is_none() && !making;
        let rows = source
            .takes
            .iter()
            .map(|take| {
                let chosen = take.chosen;
                let enabled = can && !chosen;
                let tip = if chosen {
                    format!("{} is this take", source.id)
                } else if can {
                    format!(
                        "Have {} be {} again; its clips keep their timing, and it is undone as any edit",
                        source.id, take.path
                    )
                } else if making {
                    format!("{} is being made", source.id)
                } else {
                    replace_reason.clone().unwrap_or_default()
                };
                let media = source.id.clone();
                let path = take.path.clone();
                let band = this.clone();
                ui::MenuItem::new(format!("Take {}", take.number))
                    .checked(chosen)
                    .disabled(!enabled)
                    .tip(tip)
                    .on_choose(move |_, cx| {
                        let _ = band.update(cx, |band, cx| band.use_take(&media, &path, cx));
                    })
            })
            .collect();
        ui::MenuItem::new("Takes").submenu(rows)
    }

    /// A tile's thumb: what its media looks or sounds like, and how it
    /// stands.
    fn thumb(
        &self,
        source: &Source,
        activity: &MediaActivity,
        badge: Option<Badge>,
        chosen: bool,
        window: &mut Window,
        cx: &App,
    ) -> gpui::Div {
        let session = self.session.read(cx);
        let fps = session.timeline().rate();
        let offline = badge == Some(Badge::Offline);
        let hole = matches!(activity, MediaActivity::NotMade)
            || (offline && source.is_made())
            || (source.is_made() && source.path.is_empty() && *activity == MediaActivity::Ready);
        let sound = !source.is_picture() && source.has_audio;
        let alpha = session
            .state
            .media_info
            .get(&source.id)
            .and_then(|info| info["pixFmt"].as_str())
            .is_some_and(artwork::pixel_format_has_alpha);
        let mut thumb = div()
            .relative()
            .flex_none()
            .w(px(THUMB.0))
            .h(px(THUMB.1))
            .rounded(px(theme::radius::SMALL))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center();
        if hole {
            thumb = thumb
                .bg(theme::well())
                .border_1()
                .border_dashed()
                .border_color(theme::lineh())
                .child(ui::label("Not made", text::META, theme::ink2()).mb(px(theme::space::S12)));
        } else if sound && !offline {
            thumb = thumb.bg(theme::raised());
            match session.artwork.peaks(&source.id) {
                Some(peaks) => {
                    thumb = thumb.child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                let duration = peaks.duration();
                                paint_waveform(
                                    window,
                                    bounds,
                                    &peaks,
                                    0.0,
                                    duration,
                                    theme::waveform::peak(),
                                    theme::waveform::rms(),
                                    1.0,
                                );
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
                }
                None => thumb = thumb.child(Icon::Audio.view(16.0, theme::ink3())),
            }
        } else {
            thumb = thumb.bg(theme::overlay_screen());
            if alpha && !offline {
                thumb = thumb.child(
                    canvas(
                        |_, _, _| {},
                        |bounds, _, window, _| paint_checkerboard(window, bounds),
                    )
                    .absolute()
                    .inset_0(),
                );
            }
            if offline {
                thumb = thumb.child(
                    ui::label("Offline", text::META, theme::neg()).mb(px(theme::space::S12)),
                );
            } else {
                match session.artwork.poster(&source.id) {
                    Some(poster) => {
                        thumb = thumb.child(
                            img(poster)
                                .absolute()
                                .inset_0()
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                        )
                    }
                    None => thumb = thumb.child(Icon::Film.view(16.0, theme::dis())),
                }
            }
        }
        if let MediaActivity::Making { since, .. } = activity {
            // Amber stripes moving across it, drawn again only while they move.
            let phase = motion::phase(*since, scene::STRIPE_CYCLE);
            if !motion::reduced() {
                window.request_animation_frame();
            }
            thumb = thumb.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| paint_stripes(window, bounds, phase),
                )
                .absolute()
                .inset_0(),
            );
        } else if matches!(
            activity,
            MediaActivity::Making { .. } | MediaActivity::Queued { .. }
        ) || !source.path.is_empty()
            || source.duration > 0.0
        {
            // Its length, in frames at the sequence's rate.
            thumb = thumb.child(
                div()
                    .absolute()
                    .right(px(4.0))
                    .bottom(px(4.0))
                    .px(px(theme::space::S4))
                    .py(px(1.0))
                    .rounded(px(theme::radius::SMALL))
                    .bg(theme::activity::chip())
                    .child(ui::label(
                        badge_text(source.duration, fps),
                        text::BADGE,
                        theme::ink(),
                    )),
            );
        }
        if let Some(badge) = badge.filter(|badge| *badge != Badge::Offline) {
            thumb = thumb.child(
                ui::dot(
                    if badge == Badge::NotDownloaded {
                        theme::accent_ink()
                    } else {
                        theme::ink3()
                    },
                    6.0,
                )
                .absolute()
                .left(px(4.0))
                .top(px(4.0)),
            );
        }
        // Its edge: a ring while chosen, dashed amber while it waits its
        // turn, amber while made, else a hairline.
        let (width, colour, style) = if chosen {
            (1.5, theme::ink(), BorderStyle::Solid)
        } else {
            match activity {
                MediaActivity::Queued { .. } => (1.0, theme::activity::gpu(), BorderStyle::Dashed),
                MediaActivity::Making { .. } => (1.0, theme::activity::gpu(), BorderStyle::Solid),
                _ if hole => (0.0, theme::line(), BorderStyle::Solid),
                _ => (1.0, theme::line(), BorderStyle::Solid),
            }
        };
        if width > 0.0 {
            thumb = thumb.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        window.paint_quad(quad(
                            bounds,
                            Corners::all(px(theme::radius::SMALL)),
                            gpui::transparent_black(),
                            px(width),
                            colour,
                            style,
                        ));
                    },
                )
                .absolute()
                .inset_0(),
            );
        }
        thumb
    }

    fn tile(
        &self,
        source: &Source,
        name: &str,
        used_on: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = self.session.read(cx);
        let chosen = session.chosen_media.as_deref() == Some(source.id.as_str());
        let activity = session.state.media_activity(&source.id);
        let badge = session.state.media_status.badge(&source.id);
        let wants = session.state.wants_making.get(&source.id).cloned();
        let fps = session.timeline().rate();
        let status = status_text(
            source,
            &activity,
            badge,
            wants.as_deref(),
            used_on,
            Instant::now(),
        );
        let ink = status_ink(&activity, &status);
        let tip = {
            let mut tip = tile_tip(name, source, fps, &status, badge);
            if let Some(making) = session.state.makings.get(&source.id) {
                tip = format!(
                    "{}\n{tip}",
                    editor_state::store::making_status(name, making)
                );
            }
            tip
        };
        let id = source.id.clone();
        let menu_open = self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.media.as_deref() == Some(source.id.as_str()));
        let mut thumb = self.thumb(source, &activity, badge, chosen, window, cx);
        // Its ⋯, on hover or while its menu is open.
        let more = id.clone();
        thumb =
            thumb.child(
                div()
                    .absolute()
                    .top(px(4.0))
                    .right(px(4.0))
                    .rounded(px(theme::radius::CONTROL))
                    .bg(theme::activity::chip())
                    .invisible()
                    .group_hover(SharedString::from(format!("tile-{id}")), |more| {
                        more.visible()
                    })
                    .when(menu_open, |more| more.visible())
                    .child(
                        // The kit's trigger: another menu open goes on its
                        // press, and pressed again, its own menu goes.
                        ui::trigger(
                            ElementId::Name(format!("media-more-press-{id}").into()),
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                if this.menu.as_ref().is_some_and(|open| {
                                    open.media.as_deref() == Some(more.as_str())
                                }) {
                                    this.close_menu(window, cx);
                                } else {
                                    this.close_menu(window, cx);
                                    window.focus(&this.focus);
                                    this.choose(&more, cx);
                                    this.open_menu(Some(&more), event.position, window, cx);
                                }
                            }),
                        )
                        .child(
                            icon_button(
                                ElementId::Name(format!("media-more-{id}").into()),
                                Icon::More,
                            )
                            .size(20.0)
                            .tooltip("More"),
                        ),
                    ),
            );
        let choose = id.clone();
        let right_click = id.clone();
        let tile = div()
            .id(ElementId::Name(format!("media-{id}").into()))
            .group(SharedString::from(format!("tile-{id}")))
            .flex_none()
            .w(px(THUMB.0))
            .flex()
            .flex_col()
            .cursor_pointer()
            .child(thumb)
            .child(
                ui::line(
                    name.to_owned(),
                    text::CONTROL,
                    if chosen { theme::ink() } else { theme::ink2() },
                )
                .mt(px(NAME_TOP))
                .h(px(LINE)),
            )
            .child(ui::line(status, text::META, ink).h(px(LINE)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.close_menu(window, cx);
                    window.focus(&this.focus);
                    this.choose(&right_click, cx);
                    this.open_menu(Some(&right_click), event.position, window, cx);
                }),
            )
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    window.focus(&this.focus);
                    this.choose(&choose, cx);
                    if event.click_count() >= 2 {
                        // A double-click places it at the playhead.
                        Self::run("source.insert", window, cx);
                    }
                }),
            );
        // Its tip waits while a menu is open.
        let tip = self.menu.is_none().then(|| ui::Tip::new(tip));
        div()
            .flex_none()
            .child(ui::tip_on(tile, tip))
            .with_animation(
                ElementId::Name(format!("media-pop-{id}").into()),
                motion::animation(scene::POP),
                |tile, t| tile.opacity(t),
            )
            .into_any_element()
    }

    /// A row of the list: its name, and its length on the right.
    fn list_row(
        &self,
        source: &Source,
        name: &str,
        used_on: &[String],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = self.session.read(cx);
        let chosen = session.chosen_media.as_deref() == Some(source.id.as_str());
        let fps = session.timeline().rate();
        let activity = session.state.media_activity(&source.id);
        let badge = session.state.media_status.badge(&source.id);
        let wants = session.state.wants_making.get(&source.id).cloned();
        let status = status_text(
            source,
            &activity,
            badge,
            wants.as_deref(),
            used_on,
            Instant::now(),
        );
        let tip = tile_tip(name, source, fps, &status, badge);
        let choose = source.id.clone();
        let right_click = source.id.clone();
        let row = div()
            .id(ElementId::Name(format!("media-row-{}", source.id).into()))
            .h(px(LIST_ROW))
            .flex_none()
            .px(px(theme::space::S8))
            .py(px(theme::space::S2))
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .px(px(theme::space::S8))
                    .rounded(px(theme::radius::CONTROL))
                    .when(chosen, |row| row.bg(theme::on()))
                    .when(!chosen, |row| row.hover(|row| row.bg(theme::raised())))
                    .child(
                        ui::line(
                            name.to_owned(),
                            text::CONTROL,
                            if chosen { theme::ink() } else { theme::ink2() },
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .child(
                        ui::label(
                            badge_text(source.duration, fps),
                            text::MONO_SMALL,
                            theme::ink3(),
                        )
                        .flex_none(),
                    ),
            )
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.close_menu(window, cx);
                    window.focus(&this.focus);
                    this.choose(&right_click, cx);
                    this.open_menu(Some(&right_click), event.position, window, cx);
                }),
            )
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    window.focus(&this.focus);
                    this.choose(&choose, cx);
                    if event.click_count() >= 2 {
                        Self::run("source.insert", window, cx);
                    }
                }),
            );
        // Its tip waits while a menu is open.
        let tip = self.menu.is_none().then(|| ui::Tip::new(tip));
        ui::tip_on(row, tip).into_any_element()
    }

    /// The strip's segmented "Media N | Index": the fill slides from one
    /// to the other over `SEGMENT`.
    fn tabs(&self, count: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let pad = theme::space::S8;
        let media_width = text_width(window, "Media", text::CONTROL_STRONG)
            + text_width(window, &format!(" {count}"), text::CONTROL)
            + pad * 2.0;
        let index_width = text_width(window, "Index", text::CONTROL_STRONG) + pad * 2.0;
        let t = self.tab_fill.sample(window);
        let fill_left = 2.0 + media_width * t;
        let fill_width = media_width + (index_width - media_width) * t;
        let on_media = self.tab == Tab::Media;
        let segment = |id: &'static str, width: f32| {
            div()
                .id(id)
                .w(px(width))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
        };
        div()
            .relative()
            .flex()
            .flex_none()
            .p(px(theme::space::S2))
            .h(px(24.0))
            .rounded(px(theme::radius::CONTROL))
            .bg(theme::raised())
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .left(px(fill_left))
                    .w(px(fill_width))
                    .h(px(20.0))
                    .rounded(px(theme::radius::SMALL))
                    .bg(theme::on())
                    .border_1()
                    .border_color(theme::line2()),
            )
            .child(
                segment("media-tab", media_width)
                    .child(
                        theme::type_style(
                            div(),
                            if on_media {
                                text::CONTROL_STRONG
                            } else {
                                text::CONTROL
                            },
                        )
                        .text_color(if on_media {
                            theme::ink()
                        } else {
                            theme::ink2()
                        })
                        .hover(|words| words.text_color(theme::ink()))
                        .child("Media"),
                    )
                    .child(ui::label(format!(" {count}"), text::CONTROL, theme::ink3()))
                    .on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Media, cx))),
            )
            .child(
                segment("index-tab", index_width)
                    .child(
                        theme::type_style(
                            div(),
                            if on_media {
                                text::CONTROL
                            } else {
                                text::CONTROL_STRONG
                            },
                        )
                        .text_color(if on_media {
                            theme::ink2()
                        } else {
                            theme::ink()
                        })
                        .hover(|words| words.text_color(theme::ink()))
                        .child("Index"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Index, cx))),
            )
            .into_any_element()
    }

    /// The strip: the tabs, Search, the view, Import and the close button.
    fn strip(&mut self, count: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let states = kerf_bridge::commands_states(&self.session.read(cx).command_context());
        let import_reason =
            editor_session::palette::unavailable(&states, "file.import").map(str::to_owned);
        let search_key = self.shown_key("bin.search", cx);
        let view_key = self.shown_key("bin.toggleView", cx);
        let import_key = self.shown_key("file.import", cx);
        let close_key = self.shown_key("window.project", cx);
        let tabs = self.tabs(count, window, cx);
        // Each button's tip says its name and key, or why it cannot be
        // pressed; it says what the button says now (the view it offers).
        let button = |id: &'static str, icon: Icon, words: &'static str, key: Option<String>| {
            let button = icon_button(id, icon).size(24.0).tooltip(words);
            match key {
                Some(key) => button.key(key),
                None => button,
            }
        };
        div()
            .h(px(theme::layout::STRIP))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme::space::S4))
            .pl(px(theme::space::S12))
            .pr(px(theme::space::S8))
            .border_b_1()
            .border_color(theme::line())
            .child(tabs)
            .child(div().flex_1())
            .child(
                button("media-search", Icon::Search, "Search media", search_key)
                    .on(self.searching)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.searching {
                            this.close_search(window, cx);
                        } else {
                            this.focus_search(window, cx);
                        }
                    })),
            )
            .child(
                // The button offers the other view.
                button(
                    "media-view",
                    if self.list { Icon::Grid } else { Icon::List },
                    if self.list { "Grid view" } else { "List view" },
                    view_key,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_view(cx))),
            )
            .child({
                let import = button("import-media", Icon::Import, "Import media", import_key)
                    .disabled(import_reason.is_some())
                    .on_click(|_, window, cx| Self::run("file.import", window, cx));
                match import_reason {
                    Some(reason) => import.reason(reason),
                    None => import,
                }
            })
            .child(
                button("media-close", Icon::Close, "Close Media", close_key)
                    .on_click(|_, window, cx| Self::run("window.project", window, cx)),
            )
            .into_any_element()
    }

    /// The Media page: the tiles in a row on the well, or a list.
    fn media_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.shown(cx);
        let timeline = self.session.read(cx).timeline();
        let none = timeline.sources.is_empty();
        let used = timeline.used_on();
        let items: Vec<AnyElement> = shown
            .iter()
            .map(|(source, name)| {
                let used_on = used.get(&source.id).map(Vec::as_slice).unwrap_or_default();
                if self.list {
                    self.list_row(source, name, used_on, cx)
                } else {
                    self.tile(source, name, used_on, window, cx)
                }
            })
            .collect();
        let empty_words = if none {
            Some("No media yet. Import footage, or ask Kerf to make a shot.")
        } else if shown.is_empty() {
            Some("No media's name holds that.")
        } else {
            None
        };
        let well = div().id("media-tiles").size_full().on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.close_menu(window, cx);
                window.focus(&this.focus);
                this.open_menu(None, event.position, window, cx);
            }),
        );
        let well = if self.list {
            well.overflow_y_scroll()
                .flex()
                .flex_col()
                .pt(px(theme::space::S4))
        } else {
            well.overflow_x_scroll()
                .flex()
                .gap(px(TILE_GAP))
                .pl(px(SIDE_PADDING))
                .pr(px(SIDE_PADDING))
                .pt(px(if self.searching { 2.0 } else { TOP_PADDING }))
        };
        well.children(items)
            .when_some(empty_words, |well, words| {
                well.items_center().justify_center().child(ui::label(
                    words,
                    text::BODY,
                    theme::ink3(),
                ))
            })
            .into_any_element()
    }

    /// The Index page: the chosen media's shots, silences and loudest
    /// moments, or what stands in for them.
    fn index_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let timeline = session.timeline();
        let chosen = session
            .chosen_media
            .as_deref()
            .and_then(|id| timeline.source(id))
            .cloned();
        let empty = |words: String| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .px(px(SIDE_PADDING))
                .child(ui::label(words, text::META, theme::ink3()).text_center())
                .into_any_element()
        };
        let Some(source) = chosen else {
            return empty(if timeline.sources.is_empty() {
                "Import media to index it".to_owned()
            } else {
                "Choose media in the bin to see its shots".to_owned()
            });
        };
        let name = timeline.media_name(&source.id);
        if let Some(error) = session.state.media_index_errors.get(&source.id) {
            return empty(error.clone());
        }
        let Some(index) = session.state.media_indexes.get(&source.id) else {
            return empty(format!("Indexing {name}…"));
        };
        let fps = if source.fps > 0.0 {
            source.fps
        } else {
            timeline.rate()
        };
        let rows = index_rows(index, source.duration, fps);
        if rows.is_empty() {
            return empty(format!("{name} has no cuts, silences or loud moments"));
        }
        let media = source.id.clone();
        div()
            .id("media-index")
            .size_full()
            .overflow_y_scroll()
            .pt(px(theme::space::S4))
            .flex()
            .flex_col()
            .children(rows.into_iter().enumerate().map(|(number, row)| {
                match row {
                    IndexRow::Header(words) => div()
                        .h(px(INDEX_ROW))
                        .flex_none()
                        .flex()
                        .items_center()
                        .px(px(SIDE_PADDING))
                        .child(ui::line(words, text::META, theme::ink3()))
                        .into_any_element(),
                    IndexRow::Row { name, meta, at } => {
                        let media = media.clone();
                        let start = meta.split('–').next().unwrap_or_default().to_owned();
                        let row = div()
                            .id(("media-index-row", number))
                            .h(px(INDEX_ROW))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(theme::space::S8))
                            .px(px(SIDE_PADDING))
                            .cursor_pointer()
                            .hover(|row| row.bg(theme::raised()))
                            .child(
                                ui::line(name, text::CONTROL, theme::ink())
                                    .flex_1()
                                    .min_w_0(),
                            )
                            .child(ui::label(meta, text::MONO_SMALL, theme::ink3()).flex_none())
                            // Where the program shows that moment, the playhead goes.
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session.update(cx, |session, cx| {
                                    if let Some(time) =
                                        program_time_of(session.timeline(), &media, at)
                                    {
                                        session.seek(time, false);
                                        cx.notify();
                                    }
                                });
                            }));
                        ui::tip_on(row, Some(ui::Tip::new(start))).into_any_element()
                    }
                }
            }))
            .into_any_element()
    }

    /// Roles: a row for each role of the program's tracks, whose box turns
    /// them all on or off, and Edit roles….
    fn roles(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let timeline = self.session.read(cx).timeline();
        let groups = role_groups(&timeline.tracks);
        let rows: Vec<AnyElement> = groups
            .into_iter()
            .map(|group| {
                let badges: Vec<String> = group
                    .tracks
                    .iter()
                    .map(|track| timeline.track_badge(track))
                    .collect();
                let runs = badge_runs(&badges);
                let tip = format!(
                    "Turn {} {}",
                    if group.enabled { "off" } else { "on" },
                    group.tracks.join(" ")
                );
                let tracks = group.tracks.clone();
                let on = group.enabled;
                div()
                    .id(ElementId::Name(format!("media-role-{}", group.key).into()))
                    .h(px(ROLE_ROW))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .px(px(SIDE_PADDING))
                    .child(ui::tip_on(
                        div()
                            .id(ElementId::Name(
                                format!("media-role-check-{}", group.key).into(),
                            ))
                            .cursor_pointer()
                            .child(checkbox(on))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let ops = enable_ops(&tracks, !on);
                                this.session.update(cx, |session, cx| {
                                    session.edit(ops);
                                    cx.notify();
                                });
                            })),
                        Some(ui::Tip::new(tip)),
                    ))
                    .child(
                        ui::line(group.name, text::CONTROL, theme::ink())
                            .flex_1()
                            .min_w_0(),
                    )
                    .child(ui::label(runs, text::MONO_SMALL, theme::ink3()).flex_none())
                    .into_any_element()
            })
            .collect();
        let editable = !timeline.tracks.is_empty();
        let column = div()
            .id("media-roles")
            .track_scroll(&self.roles_scroll)
            .size_full()
            .border_l_1()
            .border_color(theme::line())
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(theme::space::S4))
            .child(
                div()
                    .h(px(ROLE_ROW))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(px(SIDE_PADDING))
                    .pr(px(theme::space::S8))
                    .child(ui::label("Roles", text::CONTROL_STRONG, theme::ink()))
                    .child(div().flex_1())
                    .child(ui::tip_on(
                        theme::type_style(div(), text::META)
                            .id("media-edit-roles")
                            .h(px(theme::height::H20))
                            .flex()
                            .items_center()
                            .px(px(theme::space::S8))
                            .rounded(px(theme::radius::CONTROL))
                            .text_color(if editable {
                                theme::ink3()
                            } else {
                                theme::dis()
                            })
                            .child("Edit roles…")
                            .when(editable, |button| {
                                button
                                    .cursor_pointer()
                                    .hover(|button| {
                                        button.bg(theme::raised()).text_color(theme::ink())
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.edit_roles(window, cx)
                                    }))
                            }),
                        Some(ui::Tip::new("Assign a role to each track")),
                    )),
            )
            .children(rows);
        // Scrolling with no bar, it fades where a row is cut and while
        // there is more below, in the chrome it sits on.
        div()
            .relative()
            .w(px(ROLES_WIDTH))
            .flex_none()
            .h_full()
            .child(column)
            .child(ui::scroll_fade(&self.roles_scroll, theme::chrome()))
            .into_any_element()
    }

    /// A dialog the band shows over the window itself.
    fn hosted(&self, window: &Window) -> Option<AnyElement> {
        let (view, key) = self.hosted.as_ref()?;
        let size = window.viewport_size();
        Some(
            deferred(
                anchored().position(point(px(0.0), px(0.0))).child(
                    div()
                        .id("media-dialog-scrim")
                        .w(size.width)
                        .h(size.height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme::with_alpha(theme::scrim(), scene::SCRIM))
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .relative()
                                .rounded(px(theme::radius::PANEL))
                                .overflow_hidden()
                                .border_1()
                                .border_color(theme::line2())
                                .shadow_2xl()
                                .child(view.clone())
                                .with_animation(
                                    ("media-dialog-rise", *key),
                                    motion::animation(scene::PAGE),
                                    |body, t| {
                                        body.top(px(scene::PAGE_RISE * (1.0 - t)))
                                            .opacity(0.2 + 0.8 * t)
                                    },
                                ),
                        ),
                ),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
}

/// How wide `words` set in `style` are.
fn text_width(window: &Window, words: &str, style: theme::TypeStyle) -> f32 {
    let font = if style.mono {
        theme::mono_font(style.weight)
    } else {
        theme::ui_font(style.weight)
    };
    let run = TextRun {
        len: words.len(),
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(words.to_owned().into(), px(style.px), &[run], None);
    f32::from(line.width).ceil()
}

/// The 8 pt checkerboard under a picture with alpha.
fn paint_checkerboard(window: &mut Window, bounds: Bounds<Pixels>) {
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let mut y = 0.0;
    while y < height {
        let mut x = 0.0;
        while x < width {
            let dark = ((x / CHECKER) as i32 + (y / CHECKER) as i32) % 2 == 1;
            let square = Bounds {
                origin: point(bounds.origin.x + px(x), bounds.origin.y + px(y)),
                size: gpui::size(px(CHECKER.min(width - x)), px(CHECKER.min(height - y))),
            };
            window.paint_quad(gpui::fill(
                square,
                if dark { theme::on() } else { theme::raised() },
            ));
            x += CHECKER;
        }
        y += CHECKER;
    }
}

/// A box, ticked when `on`.
fn checkbox(on: bool) -> gpui::Div {
    div()
        .size(px(12.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .when(on, |check| {
            check
                .bg(theme::accent())
                .child(Icon::Check.view(10.0, theme::accent_text()))
        })
        .when(!on, |check| {
            check
                .border_1()
                .border_color(theme::lineh())
                .bg(theme::well())
        })
}

impl Focusable for MediaBand {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MediaBand {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let sources = &session.timeline().sources;
        let count = sources.len();
        // A menu of a media that is gone goes with it.
        let gone = self.menu.as_ref().is_some_and(|menu| {
            menu.media
                .as_ref()
                .is_some_and(|media| !sources.iter().any(|source| &source.id == media))
        });
        if gone {
            self.close_menu(window, cx);
        }
        // The Index follows the chosen media.
        if self.tab == Tab::Index {
            self.ask_index(cx);
        }
        let strip = self.strip(count, window, cx);
        let page = match self.tab {
            Tab::Media => self.media_page(window, cx),
            Tab::Index => self.index_page(cx),
        };
        let roles = self.roles(cx);
        let menu = self.menu.as_ref().map(|open| {
            deferred(
                anchored()
                    .position(open.at)
                    .snap_to_window_with_margin(px(8.0))
                    .child(open.menu.clone()),
            )
            .with_priority(1)
        });
        let search = self.searching.then(|| {
            div()
                .h(px(SEARCH_ROW))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(theme::space::S8))
                .pl(px(theme::space::S12))
                .pr(px(theme::space::S8))
                .border_b_1()
                .border_color(theme::line())
                .child(Icon::Search.view(14.0, theme::ink3()))
                .child(div().flex_1().min_w_0().child(self.search.clone()))
        });
        bind_keys(cx);
        let band = div()
            .size_full()
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &PreviousMedia, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &NextMedia, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &InsertMedia, window, cx| {
                // A dialog over the window has the keys, not the tiles under it.
                if this.hosted.is_some() {
                    return;
                }
                if this.session.read(cx).chosen_media.is_some() {
                    Self::run("source.insert", window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(Self::put_away))
            .flex()
            .flex_col()
            .bg(theme::chrome())
            .border_t_1()
            .border_color(theme::line())
            .font(theme::ui_font(FontWeight::NORMAL))
            .child(strip)
            .children(search)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .bg(theme::well())
                            .child(page),
                    )
                    .child(roles),
            );
        // A dialog the band shows is not under the band's key context, so
        // that its Return and arrow keys are the dialog's, as one the
        // workspace shows over the window is (`deferred` keeps the
        // dispatch parent it is drawn under).
        // Its menu is not under the band's key context either: while the
        // menu has the keys, the band's arrows and Return do nothing.
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(band)
            .children(menu)
            .children(self.hosted(window))
    }
}

// ------------------------------------------------------------- Edit roles

/// A track's row in Edit roles: its badge, and the role typed for it.
struct RoleRow {
    track: String,
    badge: String,
    kind: TrackKind,
    field: Entity<TextInput>,
    /// Its role as it stands, lower case; empty for its kind's default.
    before: String,
}

/// Edit roles…, over the window: a role for each track, chosen from the
/// kind's default, the engine's roles and the timeline's own, or typed.
/// Apply sends only what changed, as one edit. It gives the window back by
/// emitting [`DismissEvent`].
pub struct RolesView {
    session: Entity<Session>,
    focus: FocusHandle,
    rows: Vec<RoleRow>,
    choices: Vec<String>,
    /// The row whose choices are shown.
    choosing: Option<usize>,
    /// Its rows, for their fades.
    scroll: gpui::ScrollHandle,
    /// Esc in a row's field closes the dialog; Return is Apply.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for RolesView {}

impl Focusable for RolesView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

gpui::actions!(roles_dialog, [CloseRoles, ApplyRoles]);
const ROLES_CONTEXT: &str = "RolesDialog";

impl RolesView {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Return is Apply, the dialog's default button.
        if !editor_launcher::settings::escape_bound(cx, &CloseRoles) {
            cx.bind_keys([
                gpui::KeyBinding::new("escape", CloseRoles, Some(ROLES_CONTEXT)),
                gpui::KeyBinding::new("enter", ApplyRoles, Some(ROLES_CONTEXT)),
            ]);
        }
        let timeline = session.read(cx).timeline().clone();
        let choices = role_choices(&timeline.tracks);
        let rows = timeline
            .tracks
            .iter()
            .map(|track| {
                let role = track.role.clone().unwrap_or_default();
                let before = if track.role_assigned {
                    role.to_lowercase()
                } else {
                    String::new()
                };
                let shown = if track.role_assigned {
                    role_name(&role, kind_word(track.kind))
                } else {
                    String::new()
                };
                let placeholder = format!("{} (default)", default_role(track.kind));
                let field = cx.new(|cx| {
                    let mut field = TextInput::new(cx, placeholder).with_style(text::CONTROL);
                    field.set_text(shown, cx);
                    field
                });
                RoleRow {
                    track: track.id.clone(),
                    badge: timeline.track_badge(&track.id),
                    kind: track.kind,
                    field,
                    before,
                }
            })
            .collect::<Vec<RoleRow>>();
        // A field keeps its own Esc and Return: they are said here, where
        // Esc closes the dialog and Return is Apply, so a role typed in a
        // field is applied without reaching for the button.
        let subscriptions = rows
            .iter()
            .map(|row| {
                cx.subscribe(&row.field, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Cancel => cx.emit(DismissEvent),
                    InputEvent::Submit => this.apply(cx),
                    InputEvent::Changed => {}
                })
            })
            .collect();
        Self {
            session,
            focus: cx.focus_handle(),
            rows,
            choices,
            choosing: None,
            scroll: gpui::ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let changes: Vec<(String, String)> = self
            .rows
            .iter()
            .filter_map(|row| {
                let typed = row.field.read(cx).text().to_owned();
                role_change(&typed, &self.choices, &row.before)
                    .map(|role| (row.track.clone(), role))
            })
            .collect();
        if !changes.is_empty() {
            let ops = role_ops(&changes);
            self.session.update(cx, |session, cx| {
                session.edit(ops);
                cx.notify();
            });
        }
        cx.emit(DismissEvent);
    }
}

impl Render for RolesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let open = self.choosing == Some(index);
                let mut offered = vec![format!("{} (default)", default_role(row.kind))];
                offered.extend(self.choices.iter().cloned());
                let field = row.field.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S4))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::space::S12))
                            .child(div().w(px(64.0)).flex_none().child(ui::line(
                                if row.badge == row.track {
                                    row.badge.clone()
                                } else {
                                    format!("{} · {}", row.badge, row.track)
                                },
                                text::CONTROL,
                                theme::ink2(),
                            )))
                            .child(
                                editor_launcher::launcher::field(&row.field, window, cx).flex_1(),
                            )
                            .child(
                                icon_button(("roles-choose", index), Icon::ChevronDown)
                                    .size(24.0)
                                    .on(open)
                                    .tooltip("Choose a role")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.choosing = if this.choosing == Some(index) {
                                            None
                                        } else {
                                            Some(index)
                                        };
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(open, |row| {
                        row.child(
                            div()
                                .pl(px(76.0))
                                .flex()
                                .flex_wrap()
                                .gap(px(theme::space::S4))
                                .children(offered.into_iter().enumerate().map(|(at, choice)| {
                                    let field = field.clone();
                                    let default = at == 0;
                                    ui::chip(choice.clone(), theme::ink())
                                        .id(ElementId::Name(
                                            format!("roles-choice-{index}-{at}").into(),
                                        ))
                                        .cursor_pointer()
                                        .hover(|chip| chip.bg(theme::hover2()))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let set = if default {
                                                String::new()
                                            } else {
                                                choice.clone()
                                            };
                                            field.update(cx, |field, cx| field.set_text(set, cx));
                                            this.choosing = None;
                                            cx.notify();
                                        }))
                                })),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .key_context(ROLES_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &CloseRoles, _, cx| cx.emit(DismissEvent)))
            .on_action(cx.listener(|this, _: &ApplyRoles, _, cx| this.apply(cx)))
            .w(px(440.0))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(theme::space::S12))
            .bg(theme::popup())
            .font(theme::ui_font(FontWeight::NORMAL))
            .child(ui::label("Edit roles", text::TITLE, theme::ink()))
            .child(ui::label(
                "A role names what a track carries. Roles group tracks in the bin, and exports can follow them.",
                text::CONTROL,
                theme::ink2(),
            ))
            .child(
                // Scrolling with no bar, it fades where a row is cut and
                // while there is more below.
                div()
                    .relative()
                    .child(
                        div()
                            .id("roles-rows")
                            .track_scroll(&self.scroll)
                            .max_h(px(360.0))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(theme::space::S8))
                            .children(rows),
                    )
                    .child(ui::scroll_fade(&self.scroll, theme::popup())),
            )
            .child(
                div()
                    .mt(px(theme::space::S4))
                    .flex()
                    .justify_end()
                    .gap(px(theme::space::S8))
                    .child(
                        ui::button("roles-cancel", "Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        ui::button("roles-apply", "Apply")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    ),
            )
    }
}
