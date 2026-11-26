//! The tokens the editor's data takes its defaults from: the faces the
//! settings keep (with the families they stand for), the sidebar's and a
//! track's first sizes, and how long a notice holds. The kit's theme
//! (`editor_ui::theme`) gives them out among the rest; they are here so
//! that the settings and the store, which name no gpui, can keep them.

/// The bundled families (assets/fonts).
pub const UI_FAMILY: &str = "IBM Plex Sans";
pub const MONO_FAMILY: &str = "IBM Plex Mono";
pub const INTER_FAMILY: &str = "Inter";
pub const JETBRAINS_FAMILY: &str = "JetBrains Mono";
pub const PRETENDARD_FAMILY: &str = "Pretendard";
pub const PLEX_HANGUL_FAMILY: &str = "IBM Plex Sans KR";
pub const SERIF_FAMILY: &str = "Noto Serif";
pub const SERIF_HANGUL_FAMILY: &str = "NanumMyeongjo";

/// The faces the interface can be set in (Settings ▸ Appearance ▸
/// Interface font), each with the mono it brings. `Serif` is the one face
/// kept from when a single choice held both: it is IBM Plex for the
/// interface with the serif for reading, and what a saved `"serif"` reads
/// as; [`Face::INTERFACE`] are the faces to choose from now, with the
/// conversation's own in [`ReadingFace`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Face {
    #[default]
    Plex,
    Inter,
    Pretendard,
    Serif,
    /// The computer's own interface face, with the bundled ones for what
    /// it lacks (Hangul from Pretendard).
    System,
}

impl Face {
    pub const ALL: [Face; 5] = [
        Face::Plex,
        Face::Inter,
        Face::Pretendard,
        Face::Serif,
        Face::System,
    ];

    /// The faces Settings' Interface font offers.
    pub const INTERFACE: [Face; 4] = [Face::Plex, Face::Inter, Face::Pretendard, Face::System];

    pub fn label(self) -> &'static str {
        match self {
            Face::Plex => "IBM Plex",
            Face::Inter => "Inter",
            Face::Pretendard => "Pretendard",
            Face::Serif => "Serif for reading",
            Face::System => "System",
        }
    }

    /// The interface's family.
    pub fn ui(self) -> &'static str {
        match self {
            Face::Plex | Face::Serif => UI_FAMILY,
            Face::Inter => INTER_FAMILY,
            Face::Pretendard => PRETENDARD_FAMILY,
            Face::System => system_family(),
        }
    }

    /// The family what is said in the conversation reads in, when it reads
    /// in the interface's face ([`ReadingFace::Same`]); `Serif` keeps its
    /// serif.
    pub fn prose(self) -> &'static str {
        match self {
            Face::Serif => SERIF_FAMILY,
            other => other.ui(),
        }
    }

    /// Only IBM Plex has a mono of its own; the rest are set with the design's.
    pub fn mono(self) -> &'static str {
        match self {
            Face::Plex | Face::Serif => MONO_FAMILY,
            Face::Inter | Face::Pretendard | Face::System => JETBRAINS_FAMILY,
        }
    }

    /// The interface face and the reading face a face saved alone stands
    /// for: `Serif` is IBM Plex with the serif for reading.
    pub fn split(self) -> (Face, ReadingFace) {
        match self {
            Face::Serif => (Face::Plex, ReadingFace::Serif),
            other => (other, ReadingFace::Same),
        }
    }
}

/// What is said in the conversation reads in (Settings ▸ Appearance ▸
/// Conversation font, "For what you ask and what Kerf answers"): the
/// interface's face, or the serif.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadingFace {
    #[default]
    Same,
    Serif,
}

impl ReadingFace {
    pub const ALL: [ReadingFace; 2] = [ReadingFace::Same, ReadingFace::Serif];

    pub fn label(self) -> &'static str {
        match self {
            ReadingFace::Same => "Same as interface",
            ReadingFace::Serif => "Serif",
        }
    }

    /// The family it reads in under the interface face `ui`.
    pub fn family(self, ui: Face) -> &'static str {
        match self {
            ReadingFace::Serif => SERIF_FAMILY,
            ReadingFace::Same => ui.prose(),
        }
    }
}

/// The family GPUI calls the system's interface face, on macOS and Windows.
const SYSTEM_UI: &str = ".SystemUIFont";

/// The computer's own interface face: GPUI's name for it on macOS and
/// Windows; on Linux, what fontconfig gives for `sans-serif` (GPUI's own
/// name for it there is IBM Plex). Looked up once.
#[expect(
    clippy::disallowed_methods,
    reason = "short-lived and waited on: fc-match names the system's face"
)]
pub fn system_family() -> &'static str {
    static FAMILY: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    FAMILY.get_or_init(|| {
        if !cfg!(target_os = "linux") {
            return SYSTEM_UI;
        }
        std::process::Command::new("fc-match")
            .args(["-f", "%{family[0]}", "sans-serif"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|family| family.trim().to_owned())
            .filter(|family| !family.is_empty())
            .map_or(SYSTEM_UI, |family| Box::leak(family.into_boxed_str()))
    })
}

pub mod layout {
    pub const SIDEBAR: f32 = 280.0;
    /// A track's height when none is set: one roomy height for every
    /// track, picture, sound and subtitles alike.
    pub const TRACK_LANE: f32 = 40.0;
}

pub mod motion {
    use std::time::Duration;

    const fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    /// How long a notice stays at the conversation's foot.
    pub const NOTICE_HOLD: Duration = ms(6000);
    /// And one saying what went wrong, which takes longer to read (a
    /// failed push's ssh and git lines, an import's refusal).
    pub const NOTICE_ERROR_HOLD: Duration = ms(12000);
    /// What a sync brought and sent ("Synced: 2 commits in, pushed"), a
    /// pull request's line, and a cast saved.
    pub const NOTICE_LONG_HOLD: Duration = ms(8000);
    /// "Preview ready" and "Live preview: …".
    pub const NOTICE_SHORT_HOLD: Duration = ms(4000);
    /// A mode chosen in the composer, said briefly in its place.
    pub const NOTICE_BRIEF_HOLD: Duration = ms(3000);
}
