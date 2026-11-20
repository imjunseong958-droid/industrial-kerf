//! What a person chose in Settings, kept in `~/.kerf/editor.json`. API keys
//! live in the system's keychain, through `kerf_bridge::secret`, under
//! `agent/<provider>/apiKey`; only where it cannot be reached are they kept
//! here ([`Prefs::kept_keys`]).
//!
//! The file carries its version ([`VERSION`]). An older one is migrated when
//! it is read; one a newer Kerf wrote is never migrated, and what this Kerf
//! does not know in it stays as it is. Each field is read on its own, so a
//! value this Kerf does not know (a provider a newer Kerf added) gives the
//! default for that field alone.
//!
//! Every window holds a copy of its own. A save writes only what that copy
//! changed since it last read or wrote the file, onto the file as it is
//! now ([`merge`]), so one window's save never undoes another's change.
//!
//! The first time the editor starts with no file of its own, it takes up
//! what the Qt editor kept ([`import_qt_once`]): its QSettings, read from
//! the INI file on Linux, the plist on macOS (`defaults export`) and the
//! registry on Windows (`reg query`).

use crate::theme::{Face, ReadingFace};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

/// The version of the file this Kerf writes.
pub const VERSION: u32 = 1;

/// How long after asking GitHub for a newer Kerf it is asked again.
pub const UPDATE_EVERY_MS: u64 = 20 * 60 * 60 * 1000;

/// Who answers in the conversation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    /// Kerf's hosted model, through the person's Kerf account: no key.
    Cloud,
    #[default]
    Anthropic,
    Openai,
    /// The installed Claude Code, signed in on its own: no key.
    ClaudeCode,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::Cloud,
        Provider::Anthropic,
        Provider::Openai,
        Provider::ClaudeCode,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Cloud => "Kerf",
            Provider::Anthropic => "Anthropic",
            Provider::Openai => "OpenAI",
            Provider::ClaudeCode => "Claude Code",
        }
    }

    /// The name the bridge's agent command takes (Kerf's is `cloud: true`).
    pub fn wire(self) -> &'static str {
        match self {
            Provider::Cloud => "cloud",
            Provider::Anthropic => "anthropic",
            Provider::Openai => "openai",
            Provider::ClaudeCode => "claude-code",
        }
    }

    /// Where its key is kept in the keychain; None for one that needs none.
    pub fn secret_name(self) -> Option<&'static str> {
        match self {
            Provider::Anthropic => Some("agent/anthropic/apiKey"),
            Provider::Openai => Some("agent/openai/apiKey"),
            Provider::Cloud | Provider::ClaudeCode => None,
        }
    }

    /// The environment variable a key may also come from.
    pub fn key_env(self) -> Option<&'static str> {
        match self {
            Provider::Anthropic => Some("ANTHROPIC_API_KEY"),
            Provider::Openai => Some("OPENAI_API_KEY"),
            Provider::Cloud | Provider::ClaudeCode => None,
        }
    }

    /// The one `KERF_PROVIDER` names, as the bridge reads it.
    fn named(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::Openai),
            "claude-code" | "claudecode" => Some(Provider::ClaudeCode),
            "cloud" | "kerf" => Some(Provider::Cloud),
            _ => None,
        }
    }

    /// The one `KERF_PROVIDER` names, if it names one.
    fn from_named_environment() -> Option<Self> {
        std::env::var("KERF_PROVIDER")
            .ok()
            .and_then(|name| Self::named(&name))
    }

    /// Who answers when nothing is chosen yet, by the environment alone
    /// ([`default_provider`] with the keys in `ANTHROPIC_API_KEY` and
    /// `OPENAI_API_KEY`).
    fn from_environment() -> Self {
        let set = |name: &str| std::env::var(name).is_ok_and(|value| !value.trim().is_empty());
        default_provider(
            Self::from_named_environment(),
            set("ANTHROPIC_API_KEY"),
            set("OPENAI_API_KEY"),
        )
    }
}

/// Who answers when nothing is chosen yet, as the Qt editor chose: the
/// provider `KERF_PROVIDER` names; else a key of one's own (OpenAI's when it
/// is the only one, else Anthropic's); else, with no key kept anywhere,
/// Kerf's hosted model.
pub fn default_provider(
    named: Option<Provider>,
    anthropic_key: bool,
    openai_key: bool,
) -> Provider {
    if let Some(named) = named {
        return named;
    }
    match (anthropic_key, openai_key) {
        (false, true) => Provider::Openai,
        (true, _) => Provider::Anthropic,
        (false, false) => Provider::Cloud,
    }
}

/// The shape of an OpenAI-compatible service's chat API.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiMode {
    /// `/chat/completions`.
    #[default]
    Chat,
    /// `/responses`.
    Responses,
}

impl ApiMode {
    pub const ALL: [ApiMode; 2] = [ApiMode::Chat, ApiMode::Responses];

    pub fn label(self) -> &'static str {
        match self {
            ApiMode::Chat => "Chat Completions",
            ApiMode::Responses => "Responses",
        }
    }

    /// The name the bridge's agent command takes as `apiMode`.
    pub fn wire(self) -> &'static str {
        match self {
            ApiMode::Chat => "chat",
            ApiMode::Responses => "responses",
        }
    }

    /// `OPENAI_API_MODE`, as the bridge reads it, else Chat Completions.
    fn from_environment() -> Self {
        match std::env::var("OPENAI_API_MODE")
            .ok()
            .as_deref()
            .map(str::trim)
        {
            Some("responses") => ApiMode::Responses,
            _ => ApiMode::Chat,
        }
    }
}

/// The models Claude Code can be asked for by name; "default" leaves the
/// choice to it.
pub const CLAUDE_CODE_MODELS: [&str; 4] = ["default", "opus", "sonnet", "haiku"];

/// What Kerf may do without asking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permissions {
    #[default]
    Ask,
    AcceptEdits,
    Auto,
}

impl Permissions {
    pub const ALL: [Permissions; 3] = [
        Permissions::Ask,
        Permissions::AcceptEdits,
        Permissions::Auto,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Permissions::Ask => "Ask before edits",
            Permissions::AcceptEdits => "Accept edits",
            Permissions::Auto => "Auto",
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            Permissions::Ask => "ask",
            Permissions::AcceptEdits => "accept-edits",
            Permissions::Auto => "auto",
        }
    }
}

/// Whose shortcuts the editor answers to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Keymap {
    #[default]
    Kerf,
    Premiere,
    FinalCut,
}

impl Keymap {
    pub const ALL: [Keymap; 3] = [Keymap::Kerf, Keymap::Premiere, Keymap::FinalCut];

    pub fn label(self) -> &'static str {
        match self {
            Keymap::Kerf => "Kerf",
            Keymap::Premiere => "Premiere Pro",
            Keymap::FinalCut => "Final Cut Pro",
        }
    }

    /// The name kerf-commands' catalog takes.
    pub fn wire(self) -> &'static str {
        match self {
            Keymap::Kerf => "kerf",
            Keymap::Premiere => "premiere",
            Keymap::FinalCut => "fcp",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// The file's version: [`VERSION`], or a newer Kerf's.
    pub version: u32,
    pub provider: Provider,
    /// The model each provider is asked for; empty takes its default.
    pub anthropic_model: String,
    pub openai_model: String,
    /// One of [`CLAUDE_CODE_MODELS`]; empty is "default".
    pub claude_code_model: String,
    /// A service that speaks each API; empty for the provider's own.
    pub anthropic_base_url: String,
    pub openai_base_url: String,
    pub openai_api_mode: ApiMode,
    pub permissions: Permissions,
    pub mcp_enabled: bool,
    /// Counts the saves of the project's `.kerf/mcp.json` from Settings, so
    /// the next request takes the servers as saved.
    pub mcp_config_version: u64,
    /// The interface's face ([`Face::INTERFACE`]; a `Serif` an older Kerf
    /// saved is IBM Plex with [`ReadingFace::Serif`], [`Prefs::faces`]).
    pub face: Face,
    /// What is said in the conversation reads in: the interface's face, or
    /// the serif.
    pub reading_face: ReadingFace,
    /// Points added to what is said in the conversation.
    pub prose_step: i32,
    pub reduce_motion: bool,
    pub keymap: Keymap,
    pub check_updates: bool,
    /// When GitHub was last asked for a newer Kerf, in ms since 1970.
    pub update_checked_ms: u64,
    /// A release the person chose to skip: never offered again.
    pub update_skipped: String,
    /// The newest crash report the person has been told of, in ms since 1970.
    pub crashes_seen_ms: u64,
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    /// Projects opened, newest first.
    pub recent: Vec<PathBuf>,
    /// Projects whose Live Sync was left on.
    pub live_sync: Vec<PathBuf>,
    /// While Kerf works, the playhead goes where it is looking.
    pub follow_kerf: bool,
    /// Action and title safe drawn over the picture (View ▸ Safe Zones).
    pub safe_zones: bool,
    /// A request carries where the playhead was when it was asked.
    pub send_playhead: bool,
    /// API keys kept here because the system's keychain could not be
    /// reached, by their keychain name. Empty wherever the keychain works.
    pub kept_keys: std::collections::BTreeMap<String, String>,
    /// What the launcher was left on: its way ("new", "clone", "preset"),
    /// the template ("standard", "short", …), the frame rate, and whether
    /// Details was open. Empty or false: the launcher's own first choice.
    pub launcher_way: String,
    pub launcher_format: String,
    pub launcher_rate: String,
    pub launcher_details: bool,
    /// The id of this person's link inbox off macOS, where an editor
    /// started for a kerf:// link hands it to the one already open
    /// ([`crate::links::inbox_address`]); made the first time it is needed.
    pub links_inbox: String,
    /// Where Export Master renders each project ([`MasterOutput`]): by the
    /// project's absolute folder ([`project_key`]), or, carried over from
    /// the Qt editor, by its key there ([`qt_master_key`]).
    pub master_outputs: BTreeMap<String, MasterOutput>,
    /// How tall each project's tracks are drawn, as the person left them
    /// (a head's edge dragged, View ▸ Fit Tracks to Panel), by the
    /// project's absolute folder ([`project_key`]).
    pub track_heights: BTreeMap<String, crate::timeline::TrackHeights>,
    /// Where a shot's graph's cards were dragged, each card's place by its
    /// id, under its project and shot (`making::placement_key`, or a
    /// model's workflow under its shot, `making::workflow_key`).
    pub graph_places: BTreeMap<String, BTreeMap<String, (f32, f32)>>,
    /// What this copy last read from or wrote to the file: bookkeeping,
    /// public only so prefs can be built field by field elsewhere
    /// (`..Prefs::default()`).
    #[serde(skip)]
    pub known: Known,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            version: VERSION,
            provider: Provider::from_environment(),
            anthropic_model: String::new(),
            openai_model: String::new(),
            claude_code_model: String::new(),
            anthropic_base_url: String::new(),
            openai_base_url: String::new(),
            openai_api_mode: ApiMode::from_environment(),
            permissions: Permissions::default(),
            mcp_enabled: std::env::var("KERF_ENABLE_MCP").is_ok_and(|value| value.trim() == "1"),
            mcp_config_version: 0,
            face: Face::default(),
            reading_face: ReadingFace::default(),
            prose_step: 0,
            reduce_motion: false,
            keymap: Keymap::default(),
            check_updates: true,
            update_checked_ms: 0,
            update_skipped: String::new(),
            crashes_seen_ms: 0,
            sidebar_open: true,
            sidebar_width: crate::theme::layout::SIDEBAR,
            recent: Vec::new(),
            live_sync: Vec::new(),
            follow_kerf: true,
            safe_zones: false,
            send_playhead: true,
            kept_keys: Default::default(),
            launcher_way: String::new(),
            launcher_format: String::new(),
            launcher_rate: String::new(),
            launcher_details: false,
            links_inbox: String::new(),
            master_outputs: BTreeMap::new(),
            track_heights: BTreeMap::new(),
            graph_places: BTreeMap::new(),
            known: Known::default(),
        }
    }
}

/// The file's fields as a copy of [`Prefs`] last read or wrote them, so a
/// save can tell what it changed. Bookkeeping: two prefs alike in every
/// choice are equal whatever it holds.
#[derive(Default)]
pub struct Known(Mutex<Option<Map<String, Value>>>);

impl Known {
    fn get(&self) -> Option<Map<String, Value>> {
        self.0.lock().ok().and_then(|known| known.clone())
    }

    fn set(&self, fields: Option<Map<String, Value>>) {
        if let Ok(mut known) = self.0.lock() {
            *known = fields;
        }
    }

    /// What the file is taken to hold for `name`, so a save leaves it be
    /// unless it changes.
    fn set_field(&self, name: &str, value: Value) {
        if let Ok(mut known) = self.0.lock() {
            if let Some(known) = known.as_mut() {
                known.insert(name.to_owned(), value);
            }
        }
    }
}

impl Clone for Known {
    fn clone(&self) -> Self {
        Self(Mutex::new(self.get()))
    }
}

impl PartialEq for Known {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Known {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Known")
    }
}

/// How many recent projects are kept.
const RECENT_MOST: usize = 12;

impl Prefs {
    /// `~/.kerf/editor.json`; `KERF_EDITOR_PREFS` moves it (tests and
    /// `--settings-dir` do).
    pub fn path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("KERF_EDITOR_PREFS").filter(|path| !path.is_empty()) {
            return Some(path.into());
        }
        home().map(|home| home.join(".kerf").join("editor.json"))
    }

    /// The person's settings. With nothing chosen of who answers, a key of
    /// their own in the system's keychain keeps them on its provider; with
    /// none anywhere, Kerf's hosted model answers ([`default_provider`]).
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let mut prefs = Self::load_from(&path);
        let chosen = read_fields(&path).is_some_and(|fields| fields.contains_key("provider"));
        if !chosen
            && prefs.provider == Provider::Cloud
            && Provider::from_named_environment().is_none()
        {
            if let Some(own) = keychain_provider() {
                prefs.provider = own;
                prefs
                    .known
                    .set_field("provider", serde_json::to_value(own).unwrap_or_default());
            }
        }
        prefs
    }

    /// Reads the file at `path`, migrating one an older Kerf wrote (and
    /// writing it back so). A missing or unreadable file gives the defaults.
    pub fn load_from(path: &Path) -> Self {
        let Some(mut fields) = read_fields(path) else {
            // No file (or none that reads) holds the defaults: a save writes
            // what this copy changed from them, and leaves another's be.
            let prefs = Self::default();
            prefs.known.set(prefs.fields());
            return prefs;
        };
        if migrate(&mut fields) {
            // Best effort: read again, it migrates again.
            let _ = write_fields(path, &fields);
        }
        Self::from_fields(&fields)
    }

    /// Prefs from a file's fields, each read on its own: one this Kerf
    /// cannot read gives its default and leaves the others be. What this
    /// copy knows of the file is what it read.
    pub fn from_fields(fields: &Map<String, Value>) -> Self {
        let mut taken = match serde_json::to_value(Self::default()) {
            Ok(Value::Object(defaults)) => defaults,
            _ => Map::new(),
        };
        for (name, value) in fields {
            if !taken.contains_key(name) {
                continue;
            }
            let before = taken.insert(name.clone(), value.clone());
            if serde_json::from_value::<Self>(Value::Object(taken.clone())).is_err() {
                if let Some(before) = before {
                    taken.insert(name.clone(), before);
                }
            }
        }
        let mut prefs: Self = serde_json::from_value(Value::Object(taken)).unwrap_or_default();
        // Nothing chosen of who answers: a key kept in the file (where the
        // keychain could not be reached) is one of the person's own.
        if !fields.contains_key("provider") {
            let kept = |provider: Provider| {
                provider.secret_name().is_some_and(|name| {
                    prefs
                        .kept_keys
                        .get(name)
                        .is_some_and(|key| !key.trim().is_empty())
                })
            };
            let set = |name: &str| std::env::var(name).is_ok_and(|value| !value.trim().is_empty());
            prefs.provider = default_provider(
                Provider::from_named_environment(),
                kept(Provider::Anthropic) || set("ANTHROPIC_API_KEY"),
                kept(Provider::Openai) || set("OPENAI_API_KEY"),
            );
        }
        prefs.known.set(prefs.fields());
        prefs
    }

    /// The interface's face and the conversation's, as they are in force:
    /// a `Serif` an older Kerf saved alone is IBM Plex with the serif for
    /// reading ([`Face::split`]).
    pub fn faces(&self) -> (Face, ReadingFace) {
        match self.face {
            Face::Serif => Face::Serif.split(),
            face => (face, self.reading_face),
        }
    }

    /// Chooses the interface's face, keeping the conversation's as it is in force.
    pub fn choose_face(&mut self, face: Face) {
        let (_, reading) = self.faces();
        self.face = face;
        self.reading_face = reading;
    }

    /// Chooses the conversation's face, keeping the interface's as it is in force.
    pub fn choose_reading_face(&mut self, reading: ReadingFace) {
        let (face, _) = self.faces();
        self.face = face;
        self.reading_face = reading;
    }

    /// This copy's fields, as the file writes them.
    fn fields(&self) -> Option<Map<String, Value>> {
        match serde_json::to_value(self) {
            Ok(Value::Object(fields)) => Some(fields),
            _ => None,
        }
    }

    pub fn save(&self) {
        let _ = self.save_checked();
    }

    /// Saves, saying why it could not.
    pub fn save_checked(&self) -> std::io::Result<()> {
        match Self::path() {
            Some(path) => self.save_to(&path),
            None => Err(std::io::Error::other("no home folder")),
        }
    }

    /// Writes what this copy changed onto the file as it is now, through a
    /// temporary file, so a crash never leaves half a file and another
    /// window's change is kept.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let mine = self
            .fields()
            .ok_or_else(|| std::io::Error::other("the settings do not write as JSON"))?;
        // A file that does not read is written whole.
        let (on_disk, known) = match read_fields(path) {
            Some(on_disk) => (on_disk, self.known.get()),
            None => (Map::new(), None),
        };
        let merged = merge(on_disk, known.as_ref(), &mine);
        write_fields(path, &merged)?;
        self.known.set(Some(mine));
        Ok(())
    }

    /// Takes what other windows saved since this copy last read or wrote
    /// the file, keeping what this copy changed and has not saved.
    pub fn refresh(&mut self) {
        if let Some(path) = Self::path() {
            self.refresh_from(&path);
        }
    }

    pub fn refresh_from(&mut self, path: &Path) {
        let (Some(on_disk), Some(mine)) = (read_fields(path), self.fields()) else {
            return;
        };
        let merged = merge(on_disk.clone(), self.known.get().as_ref(), &mine);
        let read = Self::from_fields(&on_disk);
        let mut refreshed = Self::from_fields(&merged);
        // What this copy has not saved stays unsaved.
        refreshed.known = read.known;
        *self = refreshed;
    }

    /// Puts `project` first among the recent ones.
    pub fn opened(&mut self, project: &Path) {
        let project = project.to_path_buf();
        self.recent.retain(|known| known != &project);
        self.recent.insert(0, project);
        self.recent.truncate(RECENT_MOST);
    }

    /// The model asked for; empty takes the service's default.
    pub fn model(&self) -> &str {
        match self.provider {
            Provider::Anthropic => &self.anthropic_model,
            Provider::Openai => &self.openai_model,
            Provider::ClaudeCode if self.claude_code_model != "default" => &self.claude_code_model,
            Provider::ClaudeCode | Provider::Cloud => "",
        }
    }

    /// The base URL asked for; empty for the provider's own.
    pub fn base_url(&self) -> &str {
        match self.provider {
            Provider::Anthropic => &self.anthropic_base_url,
            Provider::Openai => &self.openai_base_url,
            Provider::ClaudeCode | Provider::Cloud => "",
        }
    }

    /// Whether GitHub is to be asked for a newer Kerf at `now_ms`: update
    /// notices are on, and it was last asked [`UPDATE_EVERY_MS`] ago or more.
    pub fn update_due(&self, now_ms: u64) -> bool {
        self.check_updates && now_ms.saturating_sub(self.update_checked_ms) >= UPDATE_EVERY_MS
    }

    /// The release to offer from `kerf_bridge::update_check`'s answer, as
    /// (version, page): a newer one not skipped.
    pub fn update_offer(&self, answer: &Value) -> Option<(String, String)> {
        if answer["newer"].as_bool() != Some(true) {
            return None;
        }
        let latest = answer["latest"].as_str()?.trim();
        let url = answer["url"].as_str()?.trim();
        (!latest.is_empty() && !url.is_empty() && latest != self.update_skipped.trim())
            .then(|| (latest.to_owned(), url.to_owned()))
    }

    /// The crash reports of `kerf_bridge::crash_reports` the person has not
    /// been told of, newest first.
    pub fn unseen_crashes(&self, listed: &Value) -> Vec<Value> {
        listed["reports"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|report| report["atMs"].as_f64().unwrap_or(0.0) > self.crashes_seen_ms as f64)
            .cloned()
            .collect()
    }

    /// The id of this person's link inbox, made (and saved) the first time
    /// it is asked for. `main` asks for it off macOS.
    pub fn inbox_id(&mut self) -> String {
        if self.links_inbox.trim().is_empty() {
            self.links_inbox = random_id();
            self.save();
        }
        self.links_inbox.clone()
    }

    /// Where Export Master renders `project`, if it was set: kept by its
    /// absolute folder, else carried over from the Qt editor under its key
    /// there, by the folder's name and a digest of its path, else under the
    /// folder's name alone (the Qt editor's first key).
    pub fn master_output(&self, project: &Path) -> Option<&MasterOutput> {
        [
            Some(project_key(project)),
            qt_master_key(project),
            qt_legacy_master_key(project),
        ]
        .into_iter()
        .flatten()
        .find_map(|key| self.master_outputs.get(&key))
    }

    /// The track heights kept for `project`, or each kind's own.
    pub fn track_heights_of(&self, project: &Path) -> crate::timeline::TrackHeights {
        self.track_heights
            .get(&project_key(project))
            .cloned()
            .unwrap_or_default()
    }

    /// Keeps `heights` for `project`; each kind's own heights keep nothing.
    pub fn keep_track_heights(&mut self, project: &Path, heights: crate::timeline::TrackHeights) {
        let key = project_key(project);
        if heights == crate::timeline::TrackHeights::default() {
            self.track_heights.remove(&key);
        } else {
            self.track_heights.insert(key, heights);
        }
    }

    /// Where the cards kept under `key` were put, by card.
    pub fn graph_places_of(&self, key: &str) -> std::collections::HashMap<String, (f32, f32)> {
        self.graph_places
            .get(key)
            .map(|places| {
                places
                    .iter()
                    .filter(|(_, (x, y))| x.is_finite() && y.is_finite())
                    .map(|(id, at)| (id.clone(), *at))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Keeps `places` under `key`; none lets the key go.
    pub fn keep_graph_places(
        &mut self,
        key: &str,
        places: &std::collections::HashMap<String, (f32, f32)>,
    ) {
        if places.is_empty() {
            self.graph_places.remove(key);
        } else {
            let places = places
                .iter()
                .filter(|(_, (x, y))| x.is_finite() && y.is_finite())
                .map(|(id, at)| (id.clone(), *at))
                .collect();
            self.graph_places.insert(key.to_owned(), places);
        }
    }

    /// Remembers that the person has been told of `reports`.
    pub fn saw_crashes(&mut self, reports: &[Value]) {
        let newest = reports
            .iter()
            .filter_map(|report| report["atMs"].as_f64())
            .fold(self.crashes_seen_ms as f64, f64::max);
        self.crashes_seen_ms = newest.max(0.0) as u64;
    }
}

/// The file at `path` as JSON fields; None when it is missing or no JSON
/// object.
fn read_fields(path: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text).ok()? {
        Value::Object(fields) => Some(fields),
        _ => None,
    }
}

fn write_fields(path: &Path, fields: &Map<String, Value>) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let text = serde_json::to_string_pretty(fields).map_err(std::io::Error::other)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text)?;
    // It may hold API keys: only its owner reads it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temporary, path)
}

/// Brings fields an older Kerf wrote up to [`VERSION`]; true when it
/// changed them. A newer Kerf's are left as they are.
pub fn migrate(fields: &mut Map<String, Value>) -> bool {
    let version = fields.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version >= u64::from(VERSION) {
        return false;
    }
    // 0 → 1: the first file to carry its version. Nothing was renamed or
    // retired before it; it is stamped.
    fields.insert("version".into(), Value::from(VERSION));
    true
}

/// The file's fields as a save writes them: `on_disk` as it is now, with
/// what `mine` changed since `known` (what this copy last read or wrote;
/// None when it knows nothing of the file, and so changed everything).
/// A list or a map takes this copy's additions and removals, and keeps
/// another's; the version never goes down.
pub fn merge(
    mut on_disk: Map<String, Value>,
    known: Option<&Map<String, Value>>,
    mine: &Map<String, Value>,
) -> Map<String, Value> {
    for (name, value) in mine {
        let before = known.and_then(|known| known.get(name));
        if before == Some(value) {
            continue;
        }
        let merged = match (before, on_disk.get(name), value) {
            (Some(Value::Array(before)), Some(Value::Array(theirs)), Value::Array(mine)) => {
                Value::Array(merge_list(before, theirs, mine))
            }
            (Some(Value::Object(before)), Some(Value::Object(theirs)), Value::Object(mine)) => {
                Value::Object(merge_map(before, theirs, mine))
            }
            _ => value.clone(),
        };
        on_disk.insert(name.clone(), merged);
    }
    let newest = [on_disk.get("version"), mine.get("version")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .max();
    if let Some(newest) = newest {
        on_disk.insert("version".into(), Value::from(newest));
    }
    on_disk
}

/// This copy's list in its order, then what another added that this copy
/// never had.
fn merge_list(before: &[Value], theirs: &[Value], mine: &[Value]) -> Vec<Value> {
    let mut merged = mine.to_vec();
    for item in theirs {
        if !mine.contains(item) && !before.contains(item) {
            merged.push(item.clone());
        }
    }
    merged
}

/// Theirs, with the entries this copy added, changed or took out.
fn merge_map(
    before: &Map<String, Value>,
    theirs: &Map<String, Value>,
    mine: &Map<String, Value>,
) -> Map<String, Value> {
    let mut merged = theirs.clone();
    for (key, value) in mine {
        if before.get(key) != Some(value) {
            merged.insert(key.clone(), value.clone());
        }
    }
    for key in before.keys() {
        if !mine.contains_key(key) {
            merged.remove(key);
        }
    }
    merged
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Whose key the system's keychain keeps, Anthropic's first: asked once a
/// run, and only when nothing else says who answers.
fn keychain_provider() -> Option<Provider> {
    static FOUND: std::sync::OnceLock<Option<Provider>> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        let kept = |provider: Provider| {
            provider.secret_name().is_some_and(|name| {
                kerf_bridge::secret(&serde_json::json!({"op": "get", "name": name}))["value"]
                    .as_str()
                    .is_some_and(|key| !key.trim().is_empty())
            })
        };
        let anthropic = kept(Provider::Anthropic);
        let openai = !anthropic && kept(Provider::Openai);
        match default_provider(None, anthropic, openai) {
            Provider::Cloud => None,
            own => Some(own),
        }
    })
}

/// 32 hex digits no one else can work out: the process's random hash keys,
/// the time and the process.
pub fn random_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    let half = |salt: u64| {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(salt);
        hasher.write_u128(now);
        hasher.write_u32(std::process::id());
        hasher.finish()
    };
    format!("{:016x}{:016x}", half(1), half(2))
}

// ---------------------------------------------------------------- the master

/// Where Export Master renders a project, when it is not deliveries/ as
/// `<project>_master`, ProRes 422 HQ: its folder (relative to the project,
/// or absolute; empty for where kerfd puts it), file name and codec
/// (`prores_hq`, `h264`, `hevc`, `hevc_hdr`). What is left out keeps its
/// default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MasterOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
}

/// `path` made absolute and clean (no `.`, no `..`, no trailing slash), as
/// the Qt editor wrote a project's path.
pub fn clean_absolute(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut clean = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    clean.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                ) {
                    clean.pop();
                }
            }
            other => clean.push(other.as_os_str()),
        }
    }
    clean
}

/// The key a project's own settings are kept under: its absolute folder.
pub fn project_key(project: &Path) -> String {
    clean_absolute(project).to_string_lossy().into_owned()
}

/// The project's folder name as a key's part: a `/` or `\` in it would
/// open a group of its own.
fn key_folder(project: &Path) -> Option<String> {
    let name = clean_absolute(project)
        .file_name()?
        .to_string_lossy()
        .replace(['/', '\\'], "-");
    (!name.is_empty()).then_some(name)
}

/// Where the Qt editor kept the project's master output (under
/// `deliver/<key>/outputs`), as [`Prefs::master_outputs`] keeps what it
/// carried over: `qt:<folder>~<12 hex of the SHA-1 of its absolute path>`.
pub fn qt_master_key(project: &Path) -> Option<String> {
    let folder = key_folder(project)?;
    let path = clean_absolute(project);
    // The path as Qt writes it: forward slashes.
    let written = path.to_string_lossy().replace('\\', "/");
    let digest = sha1_smol::Sha1::from(written.as_bytes())
        .digest()
        .to_string();
    Some(format!("qt:{folder}~{}", &digest[..12]))
}

/// The Qt editor's first key for it: the folder's name alone.
pub fn qt_legacy_master_key(project: &Path) -> Option<String> {
    key_folder(project).map(|folder| format!("qt:{folder}"))
}

// ---------------------------------------------------------------- the Qt editor's settings

/// The Qt editor's QSettings, by key (`agent/permissions`), each value as
/// QSettings read it back: text, a list of texts, a number or a switch.
pub type QtSettings = BTreeMap<String, Value>;

/// Reads the Qt editor's settings where it kept them: with `settings_dir`
/// (`--settings-dir`), the INI file it wrote there; else the INI file in
/// `~/.config/Industrial Kerf` on Linux, the plist on macOS, the registry
/// on Windows. None when there are none.
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

/// The Qt editor's organization and application name, both.
const QT_NAME: &str = "Industrial Kerf";
/// Its preferences domain on macOS (QSettings' made-up domain for the
/// organization, `industrial-kerf.com`, reversed, and the application).
const QT_MAC_DOMAIN: &str = "com.industrial-kerf.Industrial Kerf";
/// Its key in the Windows registry.
const QT_WINDOWS_KEY: &str = r"HKCU\Software\Industrial Kerf\Industrial Kerf";

/// The first time the editor starts with no settings file of its own, it
/// takes up the Qt editor's ([`read_qt_settings`], [`import_qt`]) and
/// writes them as its own; a key the Qt editor kept in its settings goes
/// to the system's keychain, or stays in the file where there is none.
/// Returns what it took (empty when it took nothing). `main` calls it
/// before anything reads the settings; `settings_dir` is `--settings-dir`'s.
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
    // Unsaid there, the Qt editor answered with a key of one's own when
    // the keychain (which both editors share) kept one, on the protocol chosen.
    if !settings.contains_key("agent/connection")
        && prefs.provider == Provider::Cloud
        && Provider::from_named_environment().is_none()
    {
        if let Some(own) = keychain_provider() {
            let protocol = settings
                .get("agent/activeProvider")
                .and_then(Value::as_str)
                .and_then(Provider::named)
                .filter(|named| matches!(named, Provider::Anthropic | Provider::Openai));
            prefs.provider = protocol.unwrap_or(own);
        }
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

/// Takes what the Qt editor's `settings` hold into `prefs`: who answers,
/// the models, base URLs and API shape, keys it kept in its settings
/// (into [`Prefs::kept_keys`]), permissions, the MCP switch, the faces and
/// the conversation's size, update notices and the release skipped, the
/// crashes told of, the recent projects and those with Live Sync on, the
/// sidebar, Follow Kerf, Safe Zones, the keymap, what the launcher was
/// left on, and each project's master output. What it cannot read is left
/// as it is. Returns the keys it took.
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

    // Settings from before the Qt editor's version 1 held a model nobody chose.
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
    // The Qt editor wrote its field as shown, the provider's own URL
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
    if let Some(keymap) = text("commands/keymap") {
        if let Some(chosen) = Keymap::ALL.into_iter().find(|known| known.wire() == keymap) {
            prefs.keymap = chosen;
            took("commands/keymap");
        }
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

/// A value as QSettings writes it in INI: text, quoted where it must be,
/// with C escapes; several, parted by commas, are a list. `@Invalid()` is
/// nothing; another `@Type(…)` (a date, a map) is not read.
fn ini_value(raw: &str) -> Option<Value> {
    if raw == "@Invalid()" {
        return Some(Value::Null);
    }
    if raw.starts_with('@') && !raw.starts_with("@@") {
        if let Some(bytes) = raw
            .strip_prefix("@ByteArray(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            return Some(Value::String(bytes.to_owned()));
        }
        return None;
    }
    let mut items: Vec<String> = Vec::new();
    // Whitespace outside quotes around an item is not its own: `kept` is
    // how much of `current` ends in what is.
    let mut current = String::new();
    let mut kept = 0;
    let mut quoted = false;
    let mut many = false;
    let add = |current: &mut String, kept: &mut usize, ch: char, own: bool| {
        if !own && ch.is_whitespace() {
            if !current.is_empty() {
                current.push(ch);
            }
            return;
        }
        current.push(ch);
        *kept = current.len();
    };
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                current.truncate(kept);
                items.push(std::mem::take(&mut current));
                kept = 0;
                many = true;
            }
            '\\' => {
                let escaped = match chars.next() {
                    Some('n') => '\n',
                    Some('t') => '\t',
                    Some('r') => '\r',
                    Some('a') => '\u{7}',
                    Some('b') => '\u{8}',
                    Some('f') => '\u{c}',
                    Some('v') => '\u{b}',
                    Some('0') => '\0',
                    Some('x') => {
                        let mut digits = String::new();
                        while let Some(digit) =
                            chars.peek().copied().filter(char::is_ascii_hexdigit)
                        {
                            digits.push(digit);
                            chars.next();
                        }
                        match u32::from_str_radix(&digits, 16)
                            .ok()
                            .and_then(char::from_u32)
                        {
                            Some(code) => code,
                            None => continue,
                        }
                    }
                    Some(other) => other,
                    None => continue,
                };
                add(&mut current, &mut kept, escaped, true);
            }
            other => add(&mut current, &mut kept, other, quoted),
        }
    }
    current.truncate(kept);
    let unat = |item: String| match item.strip_prefix("@@") {
        Some(rest) => format!("@{rest}"),
        None => item,
    };
    if many {
        items.push(current);
        Some(Value::Array(
            items
                .into_iter()
                .map(|item| Value::String(unat(item)))
                .collect(),
        ))
    } else {
        Some(Value::String(unat(current)))
    }
}

/// The QSettings a macOS plist holds, as `defaults export` writes it:
/// Qt's keys with `.` where a `/` was, when they hold no `/` of their own.
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
            "string" | "date" | "data" => Value::String(self.text(&tag)),
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
/// values, a subkey a group of the key; a `REG_MULTI_SZ` a list (its
/// items parted by `\0`), a `REG_DWORD` a number.
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
        let parts: Vec<&str> = line.trim().splitn(3, "    ").collect();
        let [name, kind, value] = parts.as_slice() else {
            continue;
        };
        let key = if group.is_empty() {
            (*name).to_owned()
        } else {
            format!("{group}/{name}")
        };
        let value = match *kind {
            "REG_SZ" | "REG_EXPAND_SZ" => Value::String((*value).to_owned()),
            "REG_MULTI_SZ" => Value::Array(
                value
                    .split("\\0")
                    .filter(|item| !item.is_empty())
                    .map(|item| Value::String(item.to_owned()))
                    .collect(),
            ),
            "REG_DWORD" | "REG_QWORD" => {
                let digits = value.trim().trim_start_matches("0x");
                match i64::from_str_radix(digits, 16) {
                    Ok(number) => Value::from(number),
                    Err(_) => continue,
                }
            }
            _ => continue,
        };
        settings.insert(key, value);
    }
    settings
}
