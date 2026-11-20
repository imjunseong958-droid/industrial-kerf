//! What a person chose in Settings, kept in `~/.kerf/editor.json`. API keys
//! are never written here: they live in the system's keychain, through
//! `kerf_bridge::secret`, under `agent/<provider>/apiKey`.

use crate::theme::Face;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Who answers in the conversation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    #[default]
    Anthropic,
    Openai,
    /// The installed Claude Code, signed in on its own: no key.
    ClaudeCode,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Anthropic, Provider::Openai, Provider::ClaudeCode];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic",
            Provider::Openai => "OpenAI",
            Provider::ClaudeCode => "Claude Code",
        }
    }

    /// The name the bridge's agent command takes.
    pub fn wire(self) -> &'static str {
        match self {
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
            Provider::ClaudeCode => None,
        }
    }

    /// The environment variable a key may also come from.
    pub fn key_env(self) -> Option<&'static str> {
        match self {
            Provider::Anthropic => Some("ANTHROPIC_API_KEY"),
            Provider::Openai => Some("OPENAI_API_KEY"),
            Provider::ClaudeCode => None,
        }
    }
}

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
            Permissions::Ask => "Ask first",
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
    pub provider: Provider,
    /// The model each provider is asked for; empty takes its default.
    pub anthropic_model: String,
    pub openai_model: String,
    pub openai_base_url: String,
    pub permissions: Permissions,
    pub mcp_enabled: bool,
    pub face: Face,
    /// Points added to what is said in the conversation.
    pub prose_step: i32,
    pub reduce_motion: bool,
    pub keymap: Keymap,
    pub check_updates: bool,
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
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            provider: Provider::default(),
            anthropic_model: String::new(),
            openai_model: String::new(),
            openai_base_url: String::new(),
            permissions: Permissions::default(),
            mcp_enabled: false,
            face: Face::default(),
            prose_step: 0,
            reduce_motion: false,
            keymap: Keymap::default(),
            check_updates: true,
            sidebar_open: true,
            sidebar_width: crate::theme::layout::SIDEBAR,
            recent: Vec::new(),
            live_sync: Vec::new(),
            follow_kerf: true,
            safe_zones: false,
            send_playhead: true,
            kept_keys: Default::default(),
        }
    }
}

/// How many recent projects are kept.
const RECENT_MOST: usize = 12;

impl Prefs {
    /// `~/.kerf/editor.json`; `KERF_EDITOR_PREFS` moves it (tests do).
    pub fn path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("KERF_EDITOR_PREFS") {
            return Some(path.into());
        }
        home().map(|home| home.join(".kerf").join("editor.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .map(|path| Self::load_from(&path))
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
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

    /// Writes through a temporary file, so a crash never leaves half a file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
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

    /// Puts `project` first among the recent ones.
    pub fn opened(&mut self, project: &Path) {
        let project = project.to_path_buf();
        self.recent.retain(|known| known != &project);
        self.recent.insert(0, project);
        self.recent.truncate(RECENT_MOST);
    }

    pub fn model(&self) -> &str {
        match self.provider {
            Provider::Anthropic => &self.anthropic_model,
            Provider::Openai => &self.openai_model,
            Provider::ClaudeCode => "",
        }
    }
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
