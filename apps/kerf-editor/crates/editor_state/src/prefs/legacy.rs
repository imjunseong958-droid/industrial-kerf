//! What the editor before 0.1.5 (Qt) kept in its settings, taken up the
//! first time this editor starts with no file of its own
//! ([`import_qt_once`]), so a person who used it finds their choices as
//! they left them. It kept them in QSettings: the INI file on Linux, the
//! plist on macOS (`defaults export`) and the registry on Windows (`reg
//! query`). What QSettings kept as QDataStream bytes (`@Variant(…)`,
//! `@DateTime(…)` in INI and in the registry's `REG_BINARY`, `<data>` in a
//! plist: a map, a point, a date and time) is read as Qt reads it back
//! ([`qt_variant`]), so each value is the one that editor saw.

use super::{
    home, keychain_provider, project_key, ApiMode, Keymap, MasterOutput, Permissions, Prefs,
    Provider, TrackHeights, RECENT_MOST,
};
use crate::tokens::{Face, ReadingFace};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The settings the editor before 0.1.5 kept in QSettings, by key
/// (`agent/permissions`), each value as QSettings read it back: text, a
/// list of texts, a number or a switch.
pub type QtSettings = BTreeMap<String, Value>;

/// Reads the settings of the editor before 0.1.5 where it kept them: with
/// `settings_dir` (`--settings-dir`), the INI file it wrote there; else the
/// INI file in `~/.config/Industrial Kerf` on Linux, the plist on macOS,
/// the registry on Windows. None when there are none.
pub fn read_qt_settings(settings_dir: Option<&Path>) -> Option<QtSettings> {
    if let Some(folder) = settings_dir {
        let text = std::fs::read_to_string(folder.join(QT_NAME).join(format!("{QT_NAME}.ini")));
        return text.ok().map(|text| qt_settings_ini(&text));
    }
    let read = if cfg!(target_os = "macos") {
        std::process::Command::new("defaults")
            .args(["export", QT_MAC_DOMAIN, "-"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|xml| qt_settings_plist(&xml))
    } else if cfg!(windows) {
        std::process::Command::new("reg")
            .args(["query", QT_WINDOWS_KEY, "/s"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| qt_settings_reg(&String::from_utf8_lossy(&output.stdout)))
    } else {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|folder| !folder.is_empty())
            .map(PathBuf::from)
            .or_else(|| home().map(|home| home.join(".config")))?;
        std::fs::read_to_string(config.join(QT_NAME).join(format!("{QT_NAME}.conf")))
            .ok()
            .map(|text| qt_settings_ini(&text))
    };
    read.filter(|settings| !settings.is_empty())
}

/// The organization and application name the editor before 0.1.5 gave
/// QSettings, both: they name where its settings are.
const QT_NAME: &str = "Industrial Kerf";
/// Its preferences domain on macOS (QSettings' made-up domain for the
/// organization, `industrial-kerf.com`, reversed, and the application).
const QT_MAC_DOMAIN: &str = "com.industrial-kerf.Industrial Kerf";
/// Its key in the Windows registry.
const QT_WINDOWS_KEY: &str = r"HKCU\Software\Industrial Kerf\Industrial Kerf";

/// The first time the editor starts with no settings file of its own, it
/// takes up those of the editor before 0.1.5 ([`read_qt_settings`],
/// [`import_qt`]) and writes them as its own; an API key that editor kept
/// in its settings goes to the system's keychain, or stays in the file
/// where there is none. Returns what it took (empty when it took nothing).
/// `main` calls it before anything reads the settings; `settings_dir` is
/// `--settings-dir`'s.
pub fn import_qt_once(settings_dir: Option<&Path>) -> Vec<String> {
    let Some(path) = Prefs::path() else {
        return Vec::new();
    };
    if path.exists() {
        return Vec::new();
    }
    let Some(settings) = read_qt_settings(settings_dir) else {
        return Vec::new();
    };
    let mut prefs = Prefs::load_from(&path);
    let taken = import_qt(&settings, &mut prefs);
    if taken.is_empty() {
        return taken;
    }
    // Unsaid there, that editor answered with a key of one's own when the
    // keychain (which both editors share) kept one, on the protocol chosen;
    // so the same provider answers here.
    if !settings.contains_key("agent/connection")
        && prefs.provider == Provider::Cloud
        && Provider::from_named_environment().is_none()
        && let Some(own) = keychain_provider()
    {
        let protocol = settings
            .get("agent/activeProvider")
            .and_then(Value::as_str)
            .and_then(Provider::named)
            .filter(|named| matches!(named, Provider::Anthropic | Provider::Openai));
        prefs.provider = protocol.unwrap_or(own);
    }
    let keys: Vec<(String, String)> = std::mem::take(&mut prefs.kept_keys).into_iter().collect();
    for (name, key) in keys {
        let answer =
            kerf_bridge::secret(&serde_json::json!({"op": "set", "name": name, "value": key}));
        if answer.get("error").is_some() {
            prefs.kept_keys.insert(name, key);
        }
    }
    match prefs.save_to(&path) {
        Ok(()) => taken,
        Err(error) => {
            log::warn!("the Qt editor's settings could not be written: {error}");
            Vec::new()
        }
    }
}

/// Takes what the `settings` of the editor before 0.1.5 hold into `prefs`:
/// who answers, the models, base URLs and API shape, keys it kept in its
/// settings (into [`Prefs::kept_keys`]), permissions, the MCP switch, the
/// faces and the conversation's size, update notices, when GitHub was last
/// asked and the release skipped, the crashes told of, the recent projects
/// and those with Live Sync on, the sidebar and its folders folded, Follow
/// Kerf, Safe Zones, the keymap, what the launcher was left on, each
/// project's master output and track heights, and where graphs' cards were
/// dragged. What it cannot read is left as it is. Returns the keys it took
/// (`timelineTrackHeights` once for every project's).
pub fn import_qt(settings: &QtSettings, prefs: &mut Prefs) -> Vec<String> {
    let mut taken = Vec::new();
    let text = |key: &str| -> Option<String> {
        match settings.get(key)? {
            Value::String(text) => Some(text.trim().to_owned()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(on) => Some(on.to_string()),
            _ => None,
        }
    };
    let switch = |key: &str| -> Option<bool> {
        match settings.get(key)? {
            Value::Bool(on) => Some(*on),
            Value::String(text) => match text.trim() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                _ => None,
            },
            Value::Number(number) => number.as_i64().map(|number| number != 0),
            _ => None,
        }
    };
    let number = |key: &str| -> Option<f64> {
        match settings.get(key)? {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        }
    };
    let list = |key: &str| -> Option<Vec<String>> {
        match settings.get(key)? {
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .filter(|item| !item.trim().is_empty())
                    .collect(),
            ),
            Value::String(one) if !one.trim().is_empty() => Some(vec![one.clone()]),
            Value::Null | Value::String(_) => Some(Vec::new()),
            _ => None,
        }
    };
    let mut took = |key: &str| taken.push(key.to_owned());

    // Settings from before that editor's version 1 held a model nobody chose.
    let version = number("settings/version").unwrap_or(0.0);
    if let Some(model) = text("agent/anthropic/model") {
        let retired = version < 1.0
            && matches!(
                model.as_str(),
                "claude-sonnet-4-20250514" | "claude-3-5-sonnet-latest"
            );
        prefs.anthropic_model = if retired { String::new() } else { model };
        took("agent/anthropic/model");
    }
    if let Some(model) = text("agent/openai/model") {
        prefs.openai_model = model;
        took("agent/openai/model");
    }
    // That editor wrote its field as shown, the provider's own URL
    // included; here that is no URL at all.
    for (key, own, field) in [
        (
            "agent/anthropic/baseUrl",
            "https://api.anthropic.com/v1",
            &mut prefs.anthropic_base_url,
        ),
        (
            "agent/openai/baseUrl",
            "https://api.openai.com/v1",
            &mut prefs.openai_base_url,
        ),
    ] {
        if let Some(url) = text(key) {
            *field = if url.trim_end_matches('/') == own {
                String::new()
            } else {
                url
            };
            took(key);
        }
    }
    if let Some(mode) = text("agent/openai/apiMode") {
        prefs.openai_api_mode = if mode.eq_ignore_ascii_case("responses") {
            ApiMode::Responses
        } else {
            ApiMode::Chat
        };
        took("agent/openai/apiMode");
    }
    let own_key = |provider: Provider, prefs: &mut Prefs| {
        let name = provider.secret_name()?;
        let key = text(name).filter(|key| !key.is_empty())?;
        prefs.kept_keys.insert(name.to_owned(), key);
        Some(name)
    };
    let anthropic_key = own_key(Provider::Anthropic, prefs);
    let openai_key = own_key(Provider::Openai, prefs);
    for name in [anthropic_key, openai_key].into_iter().flatten() {
        took(name);
    }
    // Who answers: Kerf, Claude Code, or a key of one's own with the
    // protocol chosen. Left unsaid, the default decides as it did there.
    let protocol = text("agent/activeProvider").map(|name| name.to_ascii_lowercase());
    match text("agent/connection").as_deref() {
        Some("cloud") => prefs.provider = Provider::Cloud,
        Some("claude-code") => prefs.provider = Provider::ClaudeCode,
        Some("own") => {
            prefs.provider = if protocol.as_deref() == Some("openai") {
                Provider::Openai
            } else {
                Provider::Anthropic
            }
        }
        // Unsaid there: a key of one's own kept its provider.
        _ if anthropic_key.is_some() || openai_key.is_some() => {
            prefs.provider = match protocol.as_deref() {
                Some("openai") => Provider::Openai,
                Some("anthropic") => Provider::Anthropic,
                _ if anthropic_key.is_some() => Provider::Anthropic,
                _ => Provider::Openai,
            }
        }
        _ => {}
    }
    if settings.contains_key("agent/connection") {
        took("agent/connection");
    }
    if let Some(mode) = text("agent/permissions") {
        prefs.permissions = match mode.as_str() {
            "accept-edits" | "acceptEdits" => Permissions::AcceptEdits,
            "auto" => Permissions::Auto,
            _ => Permissions::Ask,
        };
        took("agent/permissions");
    }
    if let Some(on) = switch("agent/mcpEnabled") {
        prefs.mcp_enabled = on;
        took("agent/mcpEnabled");
    }
    if let Some(face) = text("appearance/interfaceFace") {
        prefs.face = match face.as_str() {
            "inter" => Face::Inter,
            "pretendard" => Face::Pretendard,
            "system" => Face::System,
            _ => Face::Plex,
        };
        took("appearance/interfaceFace");
    }
    if let Some(face) = text("appearance/readingFace") {
        prefs.reading_face = if face == "serif" {
            ReadingFace::Serif
        } else {
            ReadingFace::Same
        };
        took("appearance/readingFace");
    }
    if let Some(size) = text("appearance/readingSize") {
        prefs.prose_step = match size.as_str() {
            "small" => -1,
            "large" => 2,
            _ => 0,
        };
        took("appearance/readingSize");
    }
    if let Some(on) = switch("updates/check") {
        prefs.check_updates = on;
        took("updates/check");
    }
    if let Some(version) = text("updates/skipped") {
        prefs.update_skipped = version;
        took("updates/skipped");
    }
    if let Some(seen) = number("crashes/seenMs").filter(|seen| *seen >= 0.0) {
        prefs.crashes_seen_ms = seen as u64;
        took("crashes/seenMs");
    }
    if let Some(recent) = list("projects/recent") {
        let mut kept: Vec<PathBuf> = Vec::new();
        for path in recent.into_iter().map(PathBuf::from) {
            if !kept.contains(&path) {
                kept.push(path);
            }
        }
        kept.truncate(RECENT_MOST);
        prefs.recent = kept;
        took("projects/recent");
    }
    if let Some(live) = list("collab/liveProjects") {
        prefs.live_sync = live.into_iter().map(PathBuf::from).collect();
        took("collab/liveProjects");
    }
    if let Some(open) = switch("layout/sidebar") {
        prefs.sidebar_open = open;
        took("layout/sidebar");
    }
    if let Some(width) = number("layout/sidebarWidth").filter(|width| *width > 0.0) {
        prefs.sidebar_width = width as f32;
        took("layout/sidebarWidth");
    }
    if let Some(follow) = switch("layout/followKerf") {
        prefs.follow_kerf = follow;
        took("layout/followKerf");
    }
    if let Some(shown) = switch("viewer/safeZones") {
        prefs.safe_zones = shown;
        took("viewer/safeZones");
    }
    if let Some(keymap) = text("commands/keymap")
        && let Some(chosen) = Keymap::ALL.into_iter().find(|known| known.wire() == keymap)
    {
        prefs.keymap = chosen;
        took("commands/keymap");
    }
    // The launcher's pages: New Project, Recent, From Preset, Clone.
    if let Some(page) = number("launcher/page") {
        prefs.launcher_way = match page as i64 {
            2 => "preset",
            3 => "clone",
            _ => "new",
        }
        .to_owned();
        took("launcher/page");
    }
    if let Some(details) = switch("launcher/details") {
        prefs.launcher_details = details;
        took("launcher/details");
    }
    if let Some(template) = text("projects/template") {
        prefs.launcher_format = template;
        took("projects/template");
    }
    if let Some(rate) = text("projects/rate") {
        prefs.launcher_rate = rate;
        took("projects/rate");
    }
    // When GitHub was last asked for a newer Kerf (a QDateTime there).
    if let Some(checked) = number("updates/checkedAt").filter(|checked| *checked > 0.0) {
        prefs.update_checked_ms = checked as u64;
        took("updates/checkedAt");
    }
    // The sidebar's folders folded or opened by hand, by their paths.
    if let Some(Value::Object(folded)) = settings.get("sidebar/folded") {
        prefs.sidebar_folded = folded
            .iter()
            .filter(|(folder, _)| Path::new(folder).is_absolute())
            .filter_map(|(folder, value)| {
                let folded = match value {
                    Value::Bool(folded) => *folded,
                    Value::String(text) => text == "true",
                    _ => return None,
                };
                Some((project_key(Path::new(folder)), folded))
            })
            .collect();
        took("sidebar/folded");
    }
    // Where a graph's cards were dragged: graph/placed/<16 hex>, each card's
    // place by its id, kept under the same hex (qt_graph_key) until the
    // graph is opened and moved here.
    for (key, value) in settings {
        let Some(hex) = key.strip_prefix("graph/placed/") else {
            continue;
        };
        let Value::Object(cards) = value else {
            continue;
        };
        let places: BTreeMap<String, (f32, f32)> = cards
            .iter()
            .filter_map(|(card, at)| {
                let x = at.get(0)?.as_f64()? as f32;
                let y = at.get(1)?.as_f64()? as f32;
                (x.is_finite() && y.is_finite()).then(|| (card.clone(), (x, y)))
            })
            .collect();
        if !places.is_empty() && !hex.is_empty() && !hex.contains('/') {
            prefs.graph_places.insert(format!("qt:{hex}"), places);
            took(key);
        }
    }
    // Each project's track heights: timelineTrackHeights/<SHA-1 of its
    // path>/path, its all-tracks height /all, and each track's own under
    // /tracks/<its id, percent-encoded>.
    #[derive(Default)]
    struct QtHeights {
        path: Option<String>,
        all: Option<f32>,
        tracks: BTreeMap<String, f32>,
    }
    let mut heights: BTreeMap<&str, QtHeights> = BTreeMap::new();
    for (key, value) in settings {
        let Some((group, field)) = key
            .strip_prefix("timelineTrackHeights/")
            .and_then(|rest| rest.split_once('/'))
        else {
            continue;
        };
        let height = match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        }
        .filter(|height| *height > 0.0)
        .map(|height| height as f32);
        let kept = heights.entry(group).or_default();
        match (field, field.strip_prefix("tracks/")) {
            ("path", _) => kept.path = value.as_str().map(str::to_owned),
            ("all", _) => kept.all = height,
            (_, Some(track)) if !track.is_empty() => {
                if let Some(height) = height {
                    kept.tracks.insert(percent_decoded(track), height);
                }
            }
            _ => {}
        }
    }
    let mut took_heights = false;
    for kept in heights.into_values() {
        let Some(path) = kept.path.filter(|path| Path::new(path).is_absolute()) else {
            continue;
        };
        let mut project = TrackHeights::default();
        project.set_all(kept.all);
        for (track, height) in &kept.tracks {
            project.set(track, Some(*height));
        }
        if project != TrackHeights::default() {
            prefs.keep_track_heights(Path::new(&path), project);
            took_heights = true;
        }
    }
    if took_heights {
        took("timelineTrackHeights");
    }
    // Each project's master output: deliver/<key>/outputs, as JSON with
    // {master: {folder?, name?, codec?}}.
    for (key, value) in settings {
        let Some(project) = key
            .strip_prefix("deliver/")
            .and_then(|rest| rest.strip_suffix("/outputs"))
        else {
            continue;
        };
        let Some(outputs) = value
            .as_str()
            .and_then(|json| serde_json::from_str::<Value>(json).ok())
        else {
            continue;
        };
        let Ok(master) = serde_json::from_value::<MasterOutput>(outputs["master"].clone()) else {
            continue;
        };
        if master != MasterOutput::default() {
            prefs.master_outputs.insert(format!("qt:{project}"), master);
            took(key);
        }
    }
    taken
}

/// `text` with its `%XX` taken back to what they stand for (QUrl's
/// percent-encoding, of UTF-8).
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The QSettings an INI file holds (Linux's `.conf`, and what
/// `--settings-dir` wrote): `[group]` sections, keys with `\` between
/// their parts and `%XX` for what a key cannot hold, values unquoted and
/// unescaped, a list where commas part them.
pub fn qt_settings_ini(text: &str) -> QtSettings {
    let mut settings = QtSettings::new();
    let mut group = String::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let mut line = line.trim().to_owned();
        // A value may go on over lines ending in a backslash.
        while line.ends_with('\\') && !line.ends_with("\\\\") {
            line.pop();
            match lines.next() {
                Some(next) => line.push_str(next.trim()),
                None => break,
            }
        }
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            let name = ini_unescape_key(name);
            group = if name.eq_ignore_ascii_case("General") {
                String::new()
            } else {
                name
            };
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = ini_unescape_key(key.trim());
        let key = if group.is_empty() {
            key
        } else {
            format!("{group}/{key}")
        };
        if let Some(value) = ini_value(value.trim()) {
            settings.insert(key, value);
        }
    }
    settings
}

/// A key as QSettings writes it in INI: `\` between its parts, `%XX` and
/// `%UXXXX` for the rest.
fn ini_unescape_key(key: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = key.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        let hex = |from: usize, count: usize| -> Option<u32> {
            let digits: String = chars.get(from..from + count)?.iter().collect();
            u32::from_str_radix(&digits, 16).ok()
        };
        if ch == '\\' {
            out.push('/');
        } else if ch == '%' {
            if chars.get(index + 1) == Some(&'U') {
                if let Some(code) = hex(index + 2, 4).and_then(char::from_u32) {
                    out.push(code);
                    index += 6;
                    continue;
                }
            } else if let Some(code) = hex(index + 1, 2).and_then(char::from_u32) {
                out.push(code);
                index += 3;
                continue;
            }
            out.push(ch);
        } else {
            out.push(ch);
        }
        index += 1;
    }
    out
}

/// A value as QSettings writes it in INI (its `iniEscapedString` over its
/// `variantToString`): text, quoted where it must be, with C escapes;
/// several, parted by commas, are a list. Read back as QSettings reads it
/// ([`ini_unescape`], then [`qt_string_value`] on the text or each item):
/// `@Invalid()` is nothing, `@ByteArray(…)` and `@String(…)` their text,
/// `@Variant(…)` and `@DateTime(…)` what their QDataStream bytes hold
/// ([`qt_variant`]); one that cannot be read is not read.
fn ini_value(raw: &str) -> Option<Value> {
    let (items, many) = ini_unescape(raw);
    if many {
        Some(Value::Array(
            items
                .into_iter()
                .map(|item| qt_string_value(&item).unwrap_or(Value::String(item)))
                .collect(),
        ))
    } else {
        qt_string_value(items.first().map(String::as_str).unwrap_or_default())
    }
}

/// QSettings' `iniUnescapedStringList`: a raw INI value's text, or its
/// items and true when commas outside quotes part it into a list. Spaces
/// and tabs before an item, or after one not quoted, are not its own; `\`
/// escapes as C's (`\x` hex and `\0`… octal of any length).
fn ini_unescape(raw: &str) -> (Vec<String>, bool) {
    let chars: Vec<char> = raw.chars().collect();
    let skip_spaces = |at: &mut usize| {
        while chars.get(*at).is_some_and(|ch| *ch == ' ' || *ch == '\t') {
            *at += 1;
        }
    };
    let chop = |text: &mut String, limit: usize| {
        while text.len() > limit && text.ends_with([' ', '\t']) {
            text.pop();
        }
    };
    let mut items = Vec::new();
    let mut current = String::new();
    let mut many = false;
    let mut in_quotes = false;
    let mut quoted = false;
    let mut at = 0;
    skip_spaces(&mut at);
    // What an escape or a quote put in is never chopped as a space.
    let mut chop_limit = 0;
    while at < chars.len() {
        match chars[at] {
            '\\' => {
                at += 1;
                let Some(&ch) = chars.get(at) else {
                    break;
                };
                at += 1;
                let escaped = match ch {
                    'a' => Some('\u{7}'),
                    'b' => Some('\u{8}'),
                    'f' => Some('\u{c}'),
                    'n' => Some('\n'),
                    'r' => Some('\r'),
                    't' => Some('\t'),
                    'v' => Some('\u{b}'),
                    '"' | '?' | '\'' | '\\' => Some(ch),
                    'x' if chars.get(at).is_some_and(char::is_ascii_hexdigit) => {
                        char::from_u32(escaped_number(&chars, &mut at, 16, 0))
                    }
                    '0'..='7' => {
                        char::from_u32(escaped_number(&chars, &mut at, 8, ch as u32 - '0' as u32))
                    }
                    // A line's end escaped (\r\n or \n\r as one).
                    '\n' | '\r' => {
                        if chars
                            .get(at)
                            .is_some_and(|next| (*next == '\n' || *next == '\r') && *next != ch)
                        {
                            at += 1;
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(escaped) = escaped {
                    current.push(escaped);
                }
                chop_limit = current.len();
            }
            '"' => {
                at += 1;
                quoted = true;
                in_quotes = !in_quotes;
                if !in_quotes {
                    skip_spaces(&mut at);
                    chop_limit = current.len();
                }
            }
            ',' if !in_quotes => {
                if !quoted {
                    chop(&mut current, chop_limit);
                }
                many = true;
                items.push(std::mem::take(&mut current));
                quoted = false;
                at += 1;
                skip_spaces(&mut at);
                chop_limit = 0;
            }
            first => {
                current.push(first);
                at += 1;
                while let Some(&ch) = chars.get(at).filter(|ch| !matches!(ch, '\\' | '"' | ',')) {
                    current.push(ch);
                    at += 1;
                }
            }
        }
    }
    if !quoted {
        chop(&mut current, chop_limit);
    }
    items.push(current);
    (items, many)
}

/// The digits in `radix` from `at` on, after `first`: an escape's number
/// (`\x` hex, `\0` octal), as long as it goes.
fn escaped_number(chars: &[char], at: &mut usize, radix: u32, first: u32) -> u32 {
    let mut value = first;
    while let Some(digit) = chars.get(*at).and_then(|ch| ch.to_digit(radix)) {
        value = value.wrapping_mul(radix).wrapping_add(digit);
        *at += 1;
    }
    value
}

/// QSettings' `stringToVariant`: what a value's text (unescaped) stands
/// for. `@Invalid()` is nothing; `@ByteArray(…)` and `@String(…)` their
/// text; `@Variant(…)` and `@DateTime(…)` what their bytes hold
/// ([`qt_variant`], None when they cannot be read); `@Rect(x y w h)`,
/// `@Size(w h)` and `@Point(x y)` their numbers; `@@…` text that began
/// with `@`; anything else itself.
pub fn qt_string_value(text: &str) -> Option<Value> {
    if text.starts_with('@') && text.ends_with(')') {
        let inner = |prefix: &str| {
            text.strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(')'))
        };
        if text == "@Invalid()" {
            return Some(Value::Null);
        }
        if let Some(bytes) = inner("@ByteArray(").or_else(|| inner("@String(")) {
            return Some(Value::String(bytes.to_owned()));
        }
        for (prefix, version) in [
            ("@Variant(", QDATASTREAM_4_0),
            ("@DateTime(", QDATASTREAM_5_6),
        ] {
            if let Some(rest) = text.strip_prefix(prefix) {
                // Its bytes, one a character, as QSettings takes them back
                // (toLatin1, `?` for what is not Latin-1); the stream reads
                // what it needs.
                let bytes: Vec<u8> = rest
                    .chars()
                    .map(|ch| u8::try_from(u32::from(ch)).unwrap_or(b'?'))
                    .collect();
                return qt_variant(&bytes, version);
            }
        }
        let numbers = |prefix: &str, count: usize| -> Option<Value> {
            let numbers: Vec<i64> = inner(prefix)?
                .split_whitespace()
                .map(|number| number.parse().ok())
                .collect::<Option<_>>()?;
            (numbers.len() == count).then(|| Value::from(numbers))
        };
        if let Some(value) = numbers("@Rect(", 4)
            .or_else(|| numbers("@Size(", 2))
            .or_else(|| numbers("@Point(", 2))
        {
            return Some(value);
        }
    }
    match text.strip_prefix("@@") {
        Some(rest) => Some(Value::String(format!("@{rest}"))),
        None => Some(Value::String(text.to_owned())),
    }
}

/// QDataStream's version 4.0, which QSettings writes `@Variant(…)` in.
pub const QDATASTREAM_4_0: u32 = 7;
/// And 5.6, which it writes `@DateTime(…)` in.
pub const QDATASTREAM_5_6: u32 = 17;
const QDATASTREAM_4_2: u32 = 8;
const QDATASTREAM_4_6: u32 = 12;
const QDATASTREAM_5_0: u32 = 13;
const QDATASTREAM_5_2: u32 = 15;

/// The Julian day of 1970-01-01.
const UNIX_EPOCH_JULIAN_DAY: i64 = 2_440_588;
const DAY_MS: i64 = 86_400_000;

/// A QVariant as QDataStream wrote it at stream `version`
/// ([`QDATASTREAM_4_0`] for QSettings' `@Variant`, [`QDATASTREAM_5_6`] for
/// its `@DateTime`), big-endian: its type, then (from 4.2) whether it is
/// null, then its value. As JSON: a map an object, a list an array, text a
/// string, a number a number, a point or a size `[x, y]`, a rectangle
/// `[x, y, width, height]`, a date and time its milliseconds since 1970 in
/// UTC (a date its midnight's, a time its milliseconds into the day; one
/// not set null). None for a type it does not know (a user type, a colour)
/// or bytes too few for what they say.
pub fn qt_variant(bytes: &[u8], version: u32) -> Option<Value> {
    QtStream {
        bytes,
        at: 0,
        version,
    }
    .variant()
}

/// QDataStream's reader.
struct QtStream<'a> {
    bytes: &'a [u8],
    at: usize,
    version: u32,
}

impl<'a> QtStream<'a> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let end = self.at.checked_add(N)?;
        let taken = self.bytes.get(self.at..end)?.try_into().ok()?;
        self.at = end;
        Some(taken)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|[byte]| byte)
    }

    fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_be_bytes)
    }

    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_be_bytes)
    }

    fn i32(&mut self) -> Option<i32> {
        self.take().map(i32::from_be_bytes)
    }

    fn u64(&mut self) -> Option<u64> {
        self.take().map(u64::from_be_bytes)
    }

    fn i64(&mut self) -> Option<i64> {
        self.take().map(i64::from_be_bytes)
    }

    fn f64(&mut self) -> Option<f64> {
        self.take().map(f64::from_be_bytes)
    }

    /// A float: four bytes before 4.6, as a double after.
    fn float(&mut self) -> Option<f64> {
        if self.version >= QDATASTREAM_4_6 {
            self.f64()
        } else {
            self.take().map(f32::from_be_bytes).map(f64::from)
        }
    }

    /// How many of something follow; none for the "null" marker.
    fn count(&mut self) -> Option<usize> {
        match self.u32()? {
            u32::MAX => Some(0),
            count => usize::try_from(count).ok(),
        }
    }

    /// A QString: its length in bytes, then UTF-16; a null one is empty.
    fn string(&mut self) -> Option<String> {
        let length = self.count()?;
        let end = self.at.checked_add(length)?;
        let units: Vec<u16> = self
            .bytes
            .get(self.at..end)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect();
        self.at = end;
        Some(String::from_utf16_lossy(&units))
    }

    /// A QByteArray's bytes: its length, then them; a null one is empty.
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let length = self.count()?;
        let end = self.at.checked_add(length)?;
        let bytes = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(bytes)
    }

    /// A QByteArray, its bytes one a character.
    fn byte_array(&mut self) -> Option<String> {
        Some(self.bytes()?.iter().map(|byte| char::from(*byte)).collect())
    }

    fn number(value: f64) -> Value {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }

    /// A QDate as milliseconds since 1970 at its midnight (None: not set,
    /// or a day further than milliseconds since 1970 can count).
    fn date(&mut self) -> Option<Option<i64>> {
        let day = if self.version < QDATASTREAM_5_0 {
            match self.u32()? {
                0 => None,
                day => Some(i64::from(day)),
            }
        } else {
            match self.i64()? {
                i64::MIN => None,
                day => Some(day),
            }
        };
        Some(day.and_then(|day| day.checked_sub(UNIX_EPOCH_JULIAN_DAY)?.checked_mul(DAY_MS)))
    }

    /// A QTime as milliseconds into the day (None: not set, or past the
    /// day's end, which QTime does not take either).
    fn time(&mut self) -> Option<Option<i64>> {
        let ms = self.u32()?;
        Some((i64::from(ms) < DAY_MS).then_some(i64::from(ms)))
    }

    /// A QDateTime as milliseconds since 1970 in UTC. Its date and time
    /// are as its clock read them: one at an offset from UTC is taken back
    /// by it; a local time, or one in a named zone, is taken as UTC (what
    /// that zone was then is not known here).
    fn date_time(&mut self) -> Option<Value> {
        let date = self.date()?;
        let time = self.time()?;
        let spec = self.u8()?;
        let mut offset_ms = 0;
        if self.version >= QDATASTREAM_5_2 {
            match spec {
                // Qt::OffsetFromUTC, then its seconds ahead of UTC.
                2 => offset_ms = i64::from(self.i32()?) * 1000,
                // Qt::TimeZone, then the zone.
                3 => offset_ms = self.zone_offset()? * 1000,
                _ => {}
            }
        }
        // A date with no time is at its midnight, as QDateTime takes it; one
        // further than milliseconds since 1970 can count is not set.
        let ms = date.and_then(|date| date.checked_add(time.unwrap_or(0))?.checked_sub(offset_ms));
        Some(ms.map(Value::from).unwrap_or(Value::Null))
    }

    /// A QTimeZone, as its seconds ahead of UTC where it says them (Qt's
    /// own UTC and offset zones), else 0 (a named zone, which is its id).
    fn zone_offset(&mut self) -> Option<i64> {
        Some(match self.string()?.as_str() {
            // QUtcTimeZonePrivate: id, territory, name, abbreviation,
            // offset, comment.
            "OffsetFromUtc" => {
                self.string()?;
                self.i32()?;
                self.string()?;
                self.string()?;
                let offset = self.i32()?;
                self.string()?;
                i64::from(offset)
            }
            "AheadOfUtcBy" => i64::from(self.i32()?),
            _ => 0,
        })
    }

    fn variant(&mut self) -> Option<Value> {
        let mut kind = self.u32()?;
        if self.version >= QDATASTREAM_4_2 {
            self.u8()?;
        }
        if self.version < QDATASTREAM_5_0 {
            match kind {
                // Qt 4's user type, named: not known here.
                127 => return None,
                // Qt 4 numbered these 97 higher (Long … SChar).
                129..=137 => kind -= 97,
                // An invalid one: a null QString follows.
                0 => {
                    self.string()?;
                    return Some(Value::Null);
                }
                _ => {}
            }
        }
        self.value(kind)
    }

    fn value(&mut self, kind: u32) -> Option<Value> {
        Some(match kind {
            0 => Value::Null,
            1 => Value::Bool(self.u8()? != 0),
            2 => Value::from(self.i32()?),
            3 => Value::from(self.u32()?),
            4 | 32 => Value::from(self.i64()?),
            5 | 35 => Value::from(self.u64()?),
            6 => Self::number(self.f64()?),
            7 => Value::String(char::from_u32(u32::from(self.u16()?))?.to_string()),
            // QVariantMap and QVariantHash: how many, then each key and value.
            8 | 28 => {
                let count = self.count()?;
                let mut map = Map::new();
                for _ in 0..count {
                    let key = self.string()?;
                    let value = self.variant()?;
                    map.insert(key, value);
                }
                Value::Object(map)
            }
            9 => {
                let count = self.count()?;
                let mut items = Vec::new();
                for _ in 0..count {
                    items.push(self.variant()?);
                }
                Value::Array(items)
            }
            10 => Value::String(self.string()?),
            // A QUrl: its encoded form, a QByteArray.
            17 => Value::String(String::from_utf8_lossy(self.bytes()?).into_owned()),
            11 => {
                let count = self.count()?;
                let mut items = Vec::new();
                for _ in 0..count {
                    items.push(Value::String(self.string()?));
                }
                Value::Array(items)
            }
            12 => Value::String(self.byte_array()?),
            14 => self.date()?.map(Value::from).unwrap_or(Value::Null),
            15 => self.time()?.map(Value::from).unwrap_or(Value::Null),
            16 => self.date_time()?,
            // QRect: its corners, as x, y, width and height.
            19 => {
                let (left, top, right, bottom) =
                    (self.i32()?, self.i32()?, self.i32()?, self.i32()?);
                Value::from(vec![
                    i64::from(left),
                    i64::from(top),
                    i64::from(right) - i64::from(left) + 1,
                    i64::from(bottom) - i64::from(top) + 1,
                ])
            }
            // QRectF, QLineF: four doubles; QSizeF, QPointF: two.
            20 | 24 => Value::Array(vec![
                Self::number(self.f64()?),
                Self::number(self.f64()?),
                Self::number(self.f64()?),
                Self::number(self.f64()?),
            ]),
            22 | 26 => Value::Array(vec![Self::number(self.f64()?), Self::number(self.f64()?)]),
            // QLine: four ints; QSize, QPoint: two.
            23 => Value::from(vec![self.i32()?, self.i32()?, self.i32()?, self.i32()?]),
            21 | 25 => Value::from(vec![self.i32()?, self.i32()?]),
            33 => Value::from(self.u16()? as i16),
            34 | 40 => Value::from(self.u8()? as i8),
            36 => Value::from(self.u16()?),
            37 => Value::from(self.u8()?),
            38 => Self::number(self.float()?),
            _ => return None,
        })
    }
}

/// The QSettings a macOS plist holds, as `defaults export` writes it:
/// its keys with `.` where a `/` was, when they hold no `/` of their own.
pub fn qt_settings_plist(xml: &str) -> QtSettings {
    let mut settings = QtSettings::new();
    let mut reader = Plist::new(xml);
    // The outer dict.
    if reader.open("dict").is_none() {
        return settings;
    }
    while let Some(key) = reader.key() {
        let Some(value) = reader.value() else {
            break;
        };
        let key = if key.contains('/') {
            key
        } else {
            key.replace('.', "/")
        };
        settings.insert(key, value);
    }
    settings
}

/// A reader of the XML plists `defaults export` writes, enough for
/// QSettings: dicts, arrays, strings, numbers, switches.
struct Plist<'a> {
    rest: &'a str,
}

impl<'a> Plist<'a> {
    fn new(xml: &'a str) -> Self {
        Self { rest: xml }
    }

    /// The next tag, past text and declarations: (name, closing, empty).
    fn tag(&mut self) -> Option<(String, bool, bool)> {
        loop {
            let start = self.rest.find('<')?;
            let end = self.rest[start..].find('>')? + start;
            let inside = &self.rest[start + 1..end];
            self.rest = &self.rest[end + 1..];
            if inside.starts_with('?') || inside.starts_with('!') {
                continue;
            }
            let closing = inside.starts_with('/');
            let empty = inside.ends_with('/');
            let name = inside
                .trim_start_matches('/')
                .trim_end_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            return Some((name, closing, empty));
        }
    }

    /// Past the tag `name` opens.
    fn open(&mut self, name: &str) -> Option<()> {
        loop {
            let (tag, closing, _) = self.tag()?;
            if tag == name && !closing {
                return Some(());
            }
        }
    }

    /// The text up to the closing tag, unescaped.
    fn text(&mut self, name: &str) -> String {
        let close = format!("</{name}>");
        let end = self.rest.find(&close).unwrap_or(self.rest.len());
        let text = xml_unescape(&self.rest[..end]);
        self.rest = &self.rest[(end + close.len()).min(self.rest.len())..];
        text
    }

    /// A dict's next key; None at its end.
    fn key(&mut self) -> Option<String> {
        let (tag, closing, _) = self.tag()?;
        (tag == "key" && !closing).then(|| self.text("key"))
    }

    /// The next value.
    fn value(&mut self) -> Option<Value> {
        let (tag, closing, empty) = self.tag()?;
        if closing {
            return None;
        }
        Some(match tag.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ if empty => Value::Null,
            "string" => Value::String(self.text(&tag)),
            // A CFDate (a QDateTime in local time): its milliseconds since 1970.
            "date" => {
                let text = self.text("date");
                plist_date_ms(&text)
                    .map(Value::from)
                    .unwrap_or(Value::String(text))
            }
            // CFData: what QSettings could not keep as text (a QVariant's
            // bytes, a QDateTime in UTC), its UTF-8 read as QSettings reads it.
            "data" => {
                let text = self.text("data");
                match base64_decoded(&text) {
                    Some(bytes) => {
                        qt_string_value(&String::from_utf8_lossy(&bytes)).unwrap_or(Value::Null)
                    }
                    None => Value::String(text),
                }
            }
            "integer" => {
                let text = self.text("integer");
                text.trim()
                    .parse::<i64>()
                    .map(Value::from)
                    .unwrap_or(Value::String(text))
            }
            "real" => {
                let text = self.text("real");
                text.trim()
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(Value::Number)
                    .unwrap_or(Value::String(text))
            }
            "array" => {
                let mut items = Vec::new();
                while let Some(item) = self.value() {
                    items.push(item);
                }
                Value::Array(items)
            }
            "dict" => {
                let mut fields = Map::new();
                while let Some(key) = self.key() {
                    let Some(value) = self.value() else {
                        break;
                    };
                    fields.insert(key, value);
                }
                Value::Object(fields)
            }
            _ => Value::Null,
        })
    }
}

/// A plist's `<date>` (`2026-03-04T05:06:07Z`, UTC) as milliseconds
/// since 1970.
fn plist_date_ms(text: &str) -> Option<i64> {
    let text = text.trim().strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.splitn(3, '-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let mut time = time.splitn(3, ':').map(|part| part.parse::<f64>().ok());
    let (hour, minute, second) = (time.next()??, time.next()??, time.next()??);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0.0..=24.0).contains(&hour)
        || !(0.0..60.0).contains(&minute)
        || !(0.0..61.0).contains(&second)
    {
        return None;
    }
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil), wide enough
    // for any year written; one further than milliseconds since 1970 can
    // count is no date.
    let (year, month, day) = (i128::from(year), i128::from(month), i128::from(day));
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_era_days = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_era_days - 719_468;
    let time = ((hour * 3600.0 + minute * 60.0 + second) * 1000.0).round() as i128;
    i64::try_from(days * i128::from(DAY_MS) + time).ok()
}

/// Base64 (a plist's `<data>`, its lines and indents let be), decoded.
fn base64_decoded(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    for byte in text.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        bits = bits << 6 | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    Some(out)
}

fn xml_unescape(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else {
            break;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The QSettings `reg query <key> /s` lists on Windows: each subkey's
/// values, a subkey a group of the key, each read back as QSettings'
/// registry reads it: a `REG_SZ` its text ([`qt_string_value`]); a
/// `REG_BINARY` (what held a `\0`: each `@Variant(…)` and `@DateTime(…)`,
/// a list with an empty item) the UTF-16 of its text, `reg` showing its
/// bytes in hex; a `REG_MULTI_SZ` a list (its items parted by `\0`), each
/// item read as text; a `REG_DWORD` or `REG_QWORD` a number.
pub fn qt_settings_reg(text: &str) -> QtSettings {
    let mut settings = QtSettings::new();
    let root = QT_WINDOWS_KEY.trim_start_matches(r"HKCU\");
    let mut group: Option<String> = None;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            // A key: HKEY_CURRENT_USER\Software\Industrial Kerf\Industrial Kerf[\a\b].
            group = line
                .trim()
                .split_once(root)
                .map(|(_, rest)| rest.trim_start_matches('\\').replace('\\', "/"));
            continue;
        }
        let Some(group) = group.as_ref() else {
            continue;
        };
        // name    REG_SZ    value; an empty value ends with its kind.
        let mut parts = line.trim_start().splitn(3, "    ");
        let (Some(name), Some(kind)) = (parts.next(), parts.next()) else {
            continue;
        };
        let value = parts.next().unwrap_or_default();
        let key = if group.is_empty() {
            name.to_owned()
        } else {
            format!("{group}/{name}")
        };
        let value = match kind.trim() {
            "REG_SZ" | "REG_EXPAND_SZ" => qt_string_value(value),
            "REG_NONE" | "REG_BINARY" => hex_bytes(value).and_then(|bytes| {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes(*pair))
                    .collect();
                qt_string_value(&String::from_utf16_lossy(&units))
            }),
            // QSettings' stringListToVariantList: an item read as text,
            // `@@…` its `@…`, as one in INI.
            "REG_MULTI_SZ" => Some(Value::Array(
                value
                    .split("\\0")
                    .filter(|item| !item.is_empty())
                    .map(|item| qt_string_value(item).unwrap_or(Value::String(item.to_owned())))
                    .collect(),
            )),
            "REG_DWORD" | "REG_QWORD" => {
                let digits = value.trim().trim_start_matches("0x");
                i64::from_str_radix(digits, 16).ok().map(Value::from)
            }
            _ => None,
        };
        if let Some(value) = value {
            settings.insert(key, value);
        }
    }
    settings
}

/// The bytes `reg query` shows in hex for a `REG_BINARY`; None when they
/// are not hex.
fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .map(|byte| char::from(byte).to_digit(16).map(|digit| digit as u8))
        .collect::<Option<_>>()?;
    Some(
        digits
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| pair[0] << 4 | pair[1])
            .collect(),
    )
}
