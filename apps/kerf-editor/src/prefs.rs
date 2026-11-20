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

use crate::theme::Face;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
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

    /// Who answers when nothing is chosen yet: `KERF_PROVIDER`, else OpenAI
    /// when only its key is in the environment, else Anthropic.
    fn from_environment() -> Self {
        if let Some(provider) = std::env::var("KERF_PROVIDER")
            .ok()
            .and_then(|name| Self::named(&name))
        {
            return provider;
        }
        let set = |name: &str| std::env::var(name).is_ok_and(|value| !value.trim().is_empty());
        if set("OPENAI_API_KEY") && !set("ANTHROPIC_API_KEY") {
            Provider::Openai
        } else {
            Provider::Anthropic
        }
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
    pub face: Face,
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

    pub fn load() -> Self {
        Self::path()
            .map(|path| Self::load_from(&path))
            .unwrap_or_default()
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
        let prefs: Self = serde_json::from_value(Value::Object(taken)).unwrap_or_default();
        prefs.known.set(prefs.fields());
        prefs
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
