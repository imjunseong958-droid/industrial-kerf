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
//! film or sound symbol while the artwork comes ([`crate::artwork`]); its
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
//! tooltip what it does, or why it cannot.

use crate::artwork::{self, paint_waveform};
use crate::cast::{CastView, OpenCast};
use crate::commands::RunCommand;
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::making::{MakeMedia, OpenShotGraph};
use crate::model::{self, Source, Track, TrackKind};
use crate::motion::{self, Tween};
use crate::session::Session;
use crate::store::{Badge, MediaActivity};
use crate::theme::{self, motion as scene, text};
use crate::ui::{self, icon_button};
use gpui::{
    anchored, canvas, deferred, div, img, point, prelude::*, px, quad, AnimationExt, AnyElement,
    AnyView, App, BorderStyle, Bounds, Context, Corners, DismissEvent, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, MouseButton, MouseDownEvent, ObjectFit,
    PathBuilder, PathPromptOptions, Pixels, Point, SharedString, Subscription, TextRun, Window,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// A tile's thumb, as Qt drew it, and the gap between tiles.
pub const THUMB: (f32, f32) = (112.0, 63.0);
const TILE_GAP: f32 = 12.0;
/// From the thumb to its name, and the name's and the status's lines.
const NAME_TOP: f32 = 4.0;
const LINE: f32 = 16.0;
/// The well's padding at its left and above the tiles (none while the
/// search row takes the room).
const SIDE_PADDING: f32 = 16.0;
const TOP_PADDING: f32 = 12.0;
/// The search row under the strip.
const SEARCH_ROW: f32 = 24.0;
/// Roles: its rows, in a column this wide.
const ROLE_ROW: f32 = 20.0;
const ROLES_WIDTH: f32 = 216.0;
/// A row of the list, and of the Index.
const LIST_ROW: f32 = 24.0;
const INDEX_ROW: f32 = 20.0;
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
                crate::stage::chip::elapsed(now.saturating_duration_since(*since).as_secs());
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

/// A menu while open: of a media, or of the empty bin, and where it was
/// asked for.
struct Menu {
    media: Option<String>,
    at: Point<Pixels>,
}

/// What a control says when the pointer rests on it, with its keys after.
struct KeyTip {
    words: SharedString,
    keys: Option<SharedString>,
}

impl Render for KeyTip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        theme::type_style(div(), text::CONTROL)
            .flex()
            .items_center()
            .gap(px(8.0))
            .max_w(px(420.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(theme::radius::CONTROL))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::line2())
            .text_color(theme::ink())
            .shadow_md()
            .child(self.words.clone())
            .when_some(self.keys.clone(), |tip, keys| {
                tip.child(div().flex_none().text_color(theme::ink3()).child(keys))
            })
    }
}

fn key_tip(
    words: impl Into<SharedString>,
    keys: Option<String>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let words = words.into();
    let keys: Option<SharedString> = keys.map(Into::into);
    move |_, cx| {
        cx.new(|_| KeyTip {
            words: words.clone(),
            keys: keys.clone(),
        })
        .into()
    }
}

/// The key context the band's tiles answer in.
const CONTEXT: &str = "MediaBand";

// The band's own keys, while it has them: the arrow keys move the chosen
// media, Return or Enter puts it in at the playhead, and Esc puts away its
// menu or its search before the window's Esc (which puts the band away).
gpui::actions!(media_band, [PreviousMedia, NextMedia, InsertMedia, PutAway]);

/// Binds the band's keys, again whenever a keymap bound since cleared them.
fn bind_keys(cx: &mut App) {
    if crate::settings::escape_bound(cx, &PutAway) {
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
    menu: Option<Menu>,
    /// How many menus have opened, so each fades in as a new one.
    menus: u64,
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
            menus: 0,
            tab: Tab::Media,
            tab_fill: Tween::at(0.0),
            list: false,
            searching: false,
            search,
            query: String::new(),
            hosted: None,
            hosts: 0,
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

    /// Whether the band shows a list rather than tiles.
    pub fn is_list(&self) -> bool {
        self.list
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
            let keys = crate::commands::catalog(keymap)
                .into_iter()
                .filter_map(|command| {
                    let first = command.keys.first()?.clone();
                    Some((command.id, crate::commands::shown_keys(&first)))
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
        let chosen = crate::presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some(format!("Replace {media}").into()),
            },
        );
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let answer = chosen.await;
            let _ = session.update(cx, |session, cx| {
                let path = match answer {
                    crate::presets::Picked::Paths(paths) => paths.into_iter().next(),
                    crate::presets::Picked::Failed(error) => {
                        session
                            .state
                            .notify(&format!("No file chooser could open: {error}"), true);
                        None
                    }
                    crate::presets::Picked::Unusable(words) => {
                        session.state.notify(&words, true);
                        None
                    }
                    crate::presets::Picked::Nothing => None,
                };
                if let Some(path) = path {
                    session.send(json!({"type": "media_replace", "media": media, "path": path.to_string_lossy()}));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Has media `media` be its take `path` again: one undoable replace.
    fn use_take(&mut self, media: &str, path: &str, cx: &mut Context<Self>) {
        let command = json!({"type": "media_replace", "media": media, "path": path});
        self.session.update(cx, |session, cx| {
            session.send(command);
            cx.notify();
        });
    }

    fn open_menu(&mut self, media: Option<&str>, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.menus += 1;
        self.menu = Some(Menu {
            media: media.map(str::to_owned),
            at,
        });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
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
        window.focus(&view.focus_handle(cx));
        self.hosts += 1;
        self.hosted = Some((view.into(), self.hosts));
        self.menu = None;
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

    /// Esc: the band's menu goes, else its search; else the window's Esc.
    fn put_away(&mut self, _: &PutAway, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.close_menu(cx);
        } else if self.searching {
            self.close_search(window, cx);
        } else {
            cx.propagate();
        }
    }

    fn menu_item(
        key: &str,
        words: &str,
        icon: Option<Icon>,
        enabled: bool,
        keys: Option<String>,
        tip: Option<String>,
    ) -> gpui::Stateful<gpui::Div> {
        let ink = if enabled { theme::ink() } else { theme::dis() };
        theme::type_style(div(), text::MENU_QUIET)
            .id(ElementId::Name(format!("media-menu-{key}").into()))
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(28.0))
            .px(px(10.0))
            .rounded(px(theme::radius::CONTROL))
            .text_color(ink)
            .when(enabled, |item| {
                item.cursor_pointer()
                    .hover(|item| item.bg(theme::popup_hover()))
            })
            .child(div().w(px(14.0)).flex_none().children(
                icon.map(|icon| {
                    icon.view(14.0, if enabled { theme::ink2() } else { theme::dis() })
                }),
            ))
            .child(div().flex_1().min_w_0().child(words.to_owned()))
            .children(keys.map(|keys| div().flex_none().text_color(theme::ink3()).child(keys)))
            .when_some(tip, |item, tip| item.tooltip(key_tip(tip, None)))
    }

    /// A registry command as a menu item: its label, its key, and why it
    /// cannot run as its tooltip.
    fn command_item(
        &mut self,
        id: &'static str,
        words: &str,
        icon: Option<Icon>,
        tip: &str,
        states: &Value,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reason = crate::palette::unavailable(states, id).map(str::to_owned);
        let keys = self.shown_key(id, cx);
        let enabled = reason.is_none();
        Self::menu_item(
            id,
            words,
            icon,
            enabled,
            keys,
            Some(reason.unwrap_or_else(|| tip.to_owned())),
        )
        .when(enabled, |item| {
            item.on_click(cx.listener(move |this, _, window, cx| {
                this.close_menu(cx);
                Self::run(id, window, cx);
            }))
        })
        .into_any_element()
    }

    /// A menu: what can be done with its media as it stands, then what the
    /// bin offers; the empty bin's has only the latter.
    fn menu(&mut self, menu: &Menu, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        let connected = session.state.connection == crate::store::Connection::Connected;
        let states = kerf_bridge::commands_states(&session.command_context());
        let source = match &menu.media {
            Some(media) => Some(session.timeline().source(media)?.clone()),
            None => None,
        };
        let mut items: Vec<AnyElement> = Vec::new();
        let mut title = None;
        if let Some(source) = source {
            let session = self.session.read(cx);
            let making = session.state.makings.contains_key(&source.id);
            let steps = source.steps_in_document();
            let carry = source.uncarried();
            let name = session.media_name(&source.id);
            let file = self.file_of(&source, cx).filter(|file| file.exists());
            let media: SharedString = source.id.clone().into();
            title = Some(name.clone());
            items.push(self.command_item(
                "source.insert",
                "Insert",
                Some(Icon::Plus),
                "Put it in at the playhead, pushing what follows along",
                &states,
                cx,
            ));
            items.push(self.command_item(
                "source.overwrite",
                "Overwrite",
                Some(Icon::Scissors),
                "Put it in at the playhead over what is there",
                &states,
                cx,
            ));
            items.push(ui::hairline(true).my(px(4.0)).into_any_element());
            let replace_reason = crate::palette::unavailable(&states, "media.replace")
                .map(str::to_owned)
                .or_else(|| making.then(|| format!("{} is being made", source.id)));
            let replace = source.id.clone();
            items.push(
                Self::menu_item(
                    "replace",
                    "Replace Media…",
                    Some(Icon::Import),
                    replace_reason.is_none(),
                    None,
                    Some(replace_reason.clone().unwrap_or_else(|| {
                        format!(
                            "Swap the file behind {} for everyone; its clips keep their timing",
                            source.id
                        )
                    })),
                )
                .when(replace_reason.is_none(), |item| {
                    item.on_click(cx.listener(move |this, _, window, cx| {
                        this.close_menu(cx);
                        this.replace(replace.clone(), window, cx);
                    }))
                })
                .into_any_element(),
            );
            if carry {
                // Its steps are still in the recipe file it names: what
                // carrying them writes is shown first, as a making's plan is.
                let carried = media.clone();
                let able = connected && !making;
                let recipe = source.recipe.clone().unwrap_or_default();
                items.push(
                    Self::menu_item(
                        "carry",
                        "Carry Steps into the Document…",
                        Some(Icon::Import),
                        able,
                        None,
                        Some(if connected {
                            format!("Write the steps of {recipe} under {}'s line in the document, as one edit undo takes back; what would be written is shown first, its takes are kept, and {recipe} is left where it is", source.id)
                        } else {
                            "Kerf is not connected".to_owned()
                        }),
                    )
                    .when(able, |item| {
                        item.on_click(cx.listener(move |this, _, window, cx| {
                            this.close_menu(cx);
                            window.dispatch_action(
                                Box::new(MakeMedia {
                                    media: carried.clone(),
                                    rolls: 1,
                                }),
                                cx,
                            );
                        }))
                    })
                    .into_any_element(),
                );
            }
            if steps {
                let graph = media.clone();
                items.push(
                    Self::menu_item(
                        "graph",
                        "Open Graph",
                        Some(Icon::Media),
                        true,
                        None,
                        Some(format!("See how {} is made and set it: its steps in the document and what they read, as a graph", source.id)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_menu(cx);
                        window.dispatch_action(
                            Box::new(OpenShotGraph {
                                media: graph.clone(),
                            }),
                            cx,
                        );
                    }))
                    .into_any_element(),
                );
            }
            if making {
                let stop = source.id.clone();
                items.push(
                    Self::menu_item(
                        "stop",
                        "Stop Making",
                        Some(Icon::Stop),
                        connected,
                        None,
                        Some(if connected {
                            format!("Stop making {0}: its step is asked to end, no take is kept, and {0} stays as it is", source.id)
                        } else {
                            "Kerf is not connected".to_owned()
                        }),
                    )
                    .when(connected, |item| {
                        item.on_click(cx.listener(move |this, _, _, cx| {
                            this.close_menu(cx);
                            this.session.update(cx, |session, _| {
                                session.send(json!({"type": "media_make_cancel", "media": stop}));
                            });
                        }))
                    })
                    .into_any_element(),
                );
            } else if steps {
                let make = media.clone();
                let shows = if source.shows.is_empty() {
                    String::new()
                } else {
                    format!("; it shows {}", source.shows.join(", "))
                };
                items.push(
                    Self::menu_item(
                        "make",
                        "Make…",
                        Some(Icon::Sparkle),
                        connected,
                        None,
                        Some(if connected {
                            format!("Make {}'s file again by running its steps in the document; the command is shown first{shows}", source.id)
                        } else {
                            "Kerf is not connected".to_owned()
                        }),
                    )
                    .when(connected, |item| {
                        item.on_click(cx.listener(move |this, _, window, cx| {
                            this.close_menu(cx);
                            window.dispatch_action(
                                Box::new(MakeMedia {
                                    media: make.clone(),
                                    rolls: 1,
                                }),
                                cx,
                            );
                        }))
                    })
                    .into_any_element(),
                );
                items.push(self.several(&source.id, connected, cx));
            }
            if !source.takes.is_empty() {
                items.push(self.takes(&source, making, &states, cx));
            }
            let reveal = file.clone();
            items.push(
                Self::menu_item(
                    "reveal",
                    "Show in Folder",
                    Some(Icon::Folder),
                    file.is_some(),
                    None,
                    Some(if file.is_some() {
                        "Show its file in the system's file manager".to_owned()
                    } else {
                        "Its file is not on this computer".to_owned()
                    }),
                )
                .when_some(reveal, |item, file| {
                    item.on_click(cx.listener(move |this, _, _, cx| {
                        this.close_menu(cx);
                        cx.reveal_path(&file);
                    }))
                })
                .into_any_element(),
            );
            items.push(ui::hairline(true).my(px(4.0)).into_any_element());
        }
        items.push(self.command_item(
            "file.import",
            "Import Media…",
            Some(Icon::Import),
            "Bring files into the project's media",
            &states,
            cx,
        ));
        items.push(self.command_item(
            "collab.consolidate",
            "Consolidate Media",
            Some(Icon::Folder),
            "Copy the media outside the project folder into media/ and point the project at the copies",
            &states,
            cx,
        ));
        items.push(
            Self::menu_item(
                "cast",
                "Cast and Styles…",
                Some(Icon::Sparkle),
                true,
                None,
                Some("Who and what this project's shots show, and how they look, said once for every recipe".to_owned()),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_menu(cx);
                this.show_cast(window, cx);
            }))
            .into_any_element(),
        );
        let key = self.menus;
        Some(
            deferred(
                anchored()
                    .position(menu.at)
                    .snap_to_window_with_margin(px(8.0))
                    .child(
                        div()
                            .id("media-menu")
                            .w(px(272.0))
                            .p(px(4.0))
                            .rounded(px(theme::radius::CARD))
                            .bg(theme::popup())
                            .border_1()
                            .border_color(theme::line2())
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_menu(cx)))
                            .children(title.map(|title| {
                                div().px(px(10.0)).py(px(4.0)).child(ui::line(
                                    title,
                                    text::META,
                                    theme::ink3(),
                                ))
                            }))
                            .children(items)
                            .with_animation(
                                ("media-menu", key),
                                motion::animation(scene::FADE),
                                |menu, t| menu.opacity(t),
                            ),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// Make Several: several takes of the same shot to choose among, each
    /// with its seeds one on.
    fn several(&self, media: &str, connected: bool, cx: &mut Context<Self>) -> AnyElement {
        let ink = if connected {
            theme::ink()
        } else {
            theme::dis()
        };
        theme::type_style(div(), text::MENU_QUIET)
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(28.0))
            .pl(px(10.0))
            .pr(px(4.0))
            .text_color(ink)
            .child(div().w(px(14.0)).flex_none().child(Icon::Sparkle.view(
                14.0,
                if connected { theme::ink2() } else { theme::dis() },
            )))
            .child(div().flex_1().child("Make Several"))
            .children([2u32, 4, 8].into_iter().map(|rolls| {
                let media: SharedString = media.to_owned().into();
                let tip = format!(
                    "Make {rolls} takes of {media}, each with every seed of its steps one on from the last; the command is shown first"
                );
                theme::type_style(div(), text::MONO_SMALL)
                    .id(ElementId::Name(format!("media-menu-make-{rolls}").into()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(26.0))
                    .h(px(22.0))
                    .rounded(px(theme::radius::SMALL))
                    .bg(theme::raised())
                    .text_color(ink)
                    .child(format!("{rolls}"))
                    .when(connected, |chip| {
                        chip.cursor_pointer()
                            .hover(|chip| chip.bg(theme::popup_hover()))
                            .tooltip(key_tip(tip, None))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_menu(cx);
                                window.dispatch_action(
                                    Box::new(MakeMedia {
                                        media: media.clone(),
                                        rolls,
                                    }),
                                    cx,
                                );
                            }))
                    })
            }))
            .into_any_element()
    }

    /// Takes: every take is kept; the one the media is now is ticked, and
    /// choosing another has the media be that take again (one undoable
    /// replace). Not while it is being made.
    fn takes(
        &self,
        source: &Source,
        making: bool,
        states: &Value,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let replace_reason =
            crate::palette::unavailable(states, "media.replace").map(str::to_owned);
        let can = replace_reason.is_none() && !making;
        let rows = source.takes.iter().map(|take| {
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
            theme::type_style(div(), text::MENU_QUIET)
                .id(ElementId::Name(format!("media-take-{}", take.number).into()))
                .flex()
                .items_center()
                .gap(px(8.0))
                .h(px(24.0))
                .pl(px(10.0))
                .pr(px(10.0))
                .rounded(px(theme::radius::CONTROL))
                .text_color(if enabled || chosen {
                    theme::ink()
                } else {
                    theme::dis()
                })
                .child(
                    div()
                        .w(px(14.0))
                        .flex_none()
                        .when(chosen, |tick| tick.child(Icon::Check.view(14.0, theme::ink()))),
                )
                .child(format!("Take {}", take.number))
                .tooltip(key_tip(tip, None))
                .when(enabled, |row| {
                    row.cursor_pointer()
                        .hover(|row| row.bg(theme::popup_hover()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close_menu(cx);
                            this.use_take(&media, &path, cx);
                        }))
                })
        });
        div()
            .flex()
            .flex_col()
            .child(div().px(px(10.0)).pt(px(4.0)).child(ui::line(
                "Takes",
                text::META,
                theme::ink3(),
            )))
            .children(rows)
            .into_any_element()
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
                .child(ui::label("Not made", text::META, theme::ink2()).mb(px(12.0)));
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
                                    theme::with_alpha(theme::ink3(), 0.45),
                                    theme::with_alpha(theme::ink2(), 0.8),
                                    1.0,
                                );
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
                }
                None => thumb = thumb.child(Icon::Mute.view(16.0, theme::ink3())),
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
                thumb = thumb.child(ui::label("Offline", text::META, theme::neg()).mb(px(12.0)));
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
                    .px(px(4.0))
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
                tip = format!("{}\n{tip}", crate::store::making_status(name, making));
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
                        icon_button(
                            ElementId::Name(format!("media-more-{id}").into()),
                            Icon::More,
                        )
                        .size(20.0)
                        .tooltip("More")
                        .on_click(cx.listener(
                            move |this, event: &gpui::ClickEvent, window, cx| {
                                cx.stop_propagation();
                                window.focus(&this.focus);
                                if this.menu.as_ref().is_some_and(|menu| {
                                    menu.media.as_deref() == Some(more.as_str())
                                }) {
                                    this.close_menu(cx);
                                } else {
                                    this.choose(&more, cx);
                                    this.open_menu(Some(&more), event.position(), cx);
                                }
                            },
                        )),
                    ),
            );
        let choose = id.clone();
        let right_click = id.clone();
        div()
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
            .when(self.menu.is_none(), |tile| tile.tooltip(key_tip(tip, None)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus);
                    this.choose(&right_click, cx);
                    this.open_menu(Some(&right_click), event.position, cx);
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
            )
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
        div()
            .id(ElementId::Name(format!("media-row-{}", source.id).into()))
            .h(px(LIST_ROW))
            .flex_none()
            .px(px(8.0))
            .py(px(2.0))
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
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
            .when(self.menu.is_none(), |row| row.tooltip(key_tip(tip, None)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus);
                    this.choose(&right_click, cx);
                    this.open_menu(Some(&right_click), event.position, cx);
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
            )
            .into_any_element()
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
            .p(px(2.0))
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
        let import_reason = crate::palette::unavailable(&states, "file.import").map(str::to_owned);
        let search_key = self.shown_key("bin.search", cx);
        let view_key = self.shown_key("bin.toggleView", cx);
        let import_key = self.shown_key("file.import", cx);
        let close_key = self.shown_key("window.project", cx);
        let tabs = self.tabs(count, window, cx);
        let button = |id: &'static str, icon: Icon| icon_button(id, icon).size(24.0);
        let tipped = |id: &'static str, words: String, keys: Option<String>| {
            div().id(id).flex_none().tooltip(key_tip(words, keys))
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
                tipped("media-search-tip", "Search media".into(), search_key).child(
                    button("media-search", Icon::Search)
                        .on(self.searching)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.searching {
                                this.close_search(window, cx);
                            } else {
                                this.focus_search(window, cx);
                            }
                        })),
                ),
            )
            .child(
                tipped(
                    "media-view-tip",
                    if self.list { "Grid view" } else { "List view" }.into(),
                    view_key,
                )
                .child(
                    // The button offers the other view.
                    button(
                        "media-view",
                        if self.list { Icon::Media } else { Icon::Panel },
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_view(cx))),
                ),
            )
            .child(
                tipped(
                    "media-import-tip",
                    import_reason
                        .clone()
                        .unwrap_or_else(|| "Import media".into()),
                    import_key,
                )
                .child(
                    button("import-media", Icon::Import)
                        .disabled(import_reason.is_some())
                        .on_click(|_, window, cx| Self::run("file.import", window, cx)),
                ),
            )
            .child(
                tipped("media-close-tip", "Close Media".into(), close_key).child(
                    button("media-close", Icon::Close)
                        .on_click(|_, window, cx| Self::run("window.project", window, cx)),
                ),
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
                window.focus(&this.focus);
                this.open_menu(None, event.position, cx);
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
                        div()
                            .id(("media-index-row", number))
                            .h(px(INDEX_ROW))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(SIDE_PADDING))
                            .cursor_pointer()
                            .hover(|row| row.bg(theme::raised()))
                            .tooltip(key_tip(start, None))
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
                            }))
                            .into_any_element()
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
                    .child(
                        div()
                            .id(ElementId::Name(
                                format!("media-role-check-{}", group.key).into(),
                            ))
                            .tooltip(key_tip(tip, None))
                            .cursor_pointer()
                            .child(checkbox(on))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let ops = enable_ops(&tracks, !on);
                                this.session.update(cx, |session, cx| {
                                    session.edit(ops);
                                    cx.notify();
                                });
                            })),
                    )
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
        div()
            .id("media-roles")
            .w(px(ROLES_WIDTH))
            .flex_none()
            .h_full()
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
                    .child(
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
                            .tooltip(key_tip("Assign a role to each track", None))
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
                    ),
            )
            .children(rows)
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
                        .bg(theme::with_alpha(gpui::rgb(0x000000), scene::SCRIM))
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
        if self.menu.as_ref().is_some_and(|menu| {
            menu.media
                .as_ref()
                .is_some_and(|media| !sources.iter().any(|source| &source.id == media))
        }) {
            self.menu = None;
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
        let menu = match &self.menu {
            Some(menu) => {
                let menu = Menu {
                    media: menu.media.clone(),
                    at: menu.at,
                };
                self.menu(&menu, cx)
            }
            None => None,
        };
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
            )
            .children(menu);
        // A dialog the band shows is not under the band's key context, so
        // that its Return and arrow keys are the dialog's, as one the
        // workspace shows over the window is (`deferred` keeps the
        // dispatch parent it is drawn under).
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(band)
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
    /// Esc in a row's field closes the dialog.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for RolesView {}

impl Focusable for RolesView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

gpui::actions!(roles_dialog, [CloseRoles]);
const ROLES_CONTEXT: &str = "RolesDialog";

impl RolesView {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        if !crate::settings::escape_bound(cx, &CloseRoles) {
            cx.bind_keys([gpui::KeyBinding::new(
                "escape",
                CloseRoles,
                Some(ROLES_CONTEXT),
            )]);
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
        // A field keeps its own Esc: it is said here, where it closes the dialog.
        let subscriptions = rows
            .iter()
            .map(|row| {
                cx.subscribe(&row.field, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Cancel) {
                        cx.emit(DismissEvent);
                    }
                })
            })
            .collect();
        Self {
            session,
            focus: cx.focus_handle(),
            rows,
            choices,
            choosing: None,
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
                            .child(crate::launcher::field(&row.field, window, cx).flex_1())
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
                div()
                    .id("roles-rows")
                    .max_h(px(360.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S8))
                    .children(rows),
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
