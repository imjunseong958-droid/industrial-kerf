//! The editor's icons, drawn once as 16-point line drawings and served to
//! GPUI as assets (`icons/<name>.svg`). GPUI draws an SVG as a mask in the
//! element's text colour, so each icon takes the colour of what it is in.

use gpui::{svg, IntoElement, Rgba, SharedString, Styled, Svg};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Sidebar,
    Panel,
    Play,
    Pause,
    Plus,
    Gear,
    Send,
    Stop,
    Export,
    Media,
    Close,
    ChevronDown,
    ChevronRight,
    ChevronLeft,
    Undo,
    Redo,
    Scissors,
    Folder,
    Chat,
    Check,
    Search,
    Archive,
    Trash,
    Film,
    Sparkle,
    Dot,
    StepBack,
    StepForward,
    Import,
    More,
    Eye,
    Lock,
    Mute,
    Clock,
    /// Playback goes round: the transport's Loop.
    Loop,
    /// The mask tools: a rectangle, an ellipse, a pen's nib.
    MaskRectangle,
    MaskEllipse,
    Pen,
    /// The Media band's two ways of showing its media: rows, or tiles.
    List,
    Grid,
    /// A sound: a speaker and its waves (a sound tile before its waveform).
    Audio,
    /// Another seed, at random: a die's face.
    Dice,
    /// Closer and further: a shot's graph's zoom.
    ZoomIn,
    ZoomOut,
    /// Kerf's own mark: the Auto mode's chip.
    Kerf,
}

impl Icon {
    pub fn name(self) -> &'static str {
        match self {
            Icon::Sidebar => "sidebar",
            Icon::Panel => "panel",
            Icon::Play => "play",
            Icon::Pause => "pause",
            Icon::Plus => "plus",
            Icon::Gear => "gear",
            Icon::Send => "send",
            Icon::Stop => "stop",
            Icon::Export => "export",
            Icon::Media => "media",
            Icon::Close => "close",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronRight => "chevron-right",
            Icon::ChevronLeft => "chevron-left",
            Icon::Undo => "undo",
            Icon::Redo => "redo",
            Icon::Scissors => "scissors",
            Icon::Folder => "folder",
            Icon::Chat => "chat",
            Icon::Check => "check",
            Icon::Search => "search",
            Icon::Archive => "archive",
            Icon::Trash => "trash",
            Icon::Film => "film",
            Icon::Sparkle => "sparkle",
            Icon::Dot => "dot",
            Icon::StepBack => "step-back",
            Icon::StepForward => "step-forward",
            Icon::Import => "import",
            Icon::More => "more",
            Icon::Eye => "eye",
            Icon::Lock => "lock",
            Icon::Mute => "mute",
            Icon::Clock => "clock",
            Icon::Loop => "loop",
            Icon::MaskRectangle => "mask-rectangle",
            Icon::MaskEllipse => "mask-ellipse",
            Icon::Pen => "pen",
            Icon::List => "list",
            Icon::Grid => "grid",
            Icon::Audio => "audio",
            Icon::Dice => "dice",
            Icon::ZoomIn => "zoom-in",
            Icon::ZoomOut => "zoom-out",
            Icon::Kerf => "kerf",
        }
    }

    pub fn path(self) -> SharedString {
        format!("icons/{}.svg", self.name()).into()
    }

    /// The icon at `size` points in `color`.
    pub fn view(self, size: f32, color: Rgba) -> Svg {
        svg()
            .path(self.path())
            .size(gpui::px(size))
            .flex_none()
            .text_color(color)
    }

    /// The drawing, as SVG.
    fn body(self) -> &'static str {
        match self {
            Icon::Sidebar => {
                r#"<rect x="1.75" y="2.75" width="12.5" height="10.5" rx="2"/><path d="M6 2.75v10.5"/>"#
            }
            Icon::Panel => {
                r#"<rect x="1.75" y="2.75" width="12.5" height="10.5" rx="2"/><path d="M10 2.75v10.5"/>"#
            }
            Icon::Play => {
                r#"<path d="M5 3.2v9.6a.4.4 0 0 0 .6.35l7.7-4.8a.4.4 0 0 0 0-.7L5.6 2.85a.4.4 0 0 0-.6.35z" fill="black"/>"#
            }
            Icon::Pause => {
                r#"<rect x="4" y="3" width="2.5" height="10" rx=".6" fill="black" stroke="none"/><rect x="9.5" y="3" width="2.5" height="10" rx=".6" fill="black" stroke="none"/>"#
            }
            Icon::Plus => r#"<path d="M8 3v10M3 8h10"/>"#,
            Icon::Gear => {
                r#"<circle cx="8" cy="8" r="2.2"/><path d="M8 1.75v1.6M8 12.65v1.6M14.25 8h-1.6M3.35 8h-1.6M12.42 3.58l-1.13 1.13M4.71 11.29l-1.13 1.13M12.42 12.42l-1.13-1.13M4.71 4.71L3.58 3.58"/>"#
            }
            Icon::Send => r#"<path d="M8 13V3.5M3.75 7.5L8 3.25l4.25 4.25"/>"#,
            Icon::Stop => r#"<rect x="4" y="4" width="8" height="8" rx="1.5" fill="black"/>"#,
            Icon::Export => {
                r#"<path d="M8 10V2.5M5 5.25L8 2.25l3 3"/><path d="M3 8.5v4a1.25 1.25 0 0 0 1.25 1.25h7.5A1.25 1.25 0 0 0 13 12.5v-4"/>"#
            }
            Icon::Media => {
                r#"<rect x="2" y="2" width="5" height="5" rx="1"/><rect x="9" y="2" width="5" height="5" rx="1"/><rect x="2" y="9" width="5" height="5" rx="1"/><rect x="9" y="9" width="5" height="5" rx="1"/>"#
            }
            Icon::Close => r#"<path d="M4 4l8 8M12 4l-8 8"/>"#,
            Icon::ChevronDown => r#"<path d="M4 6l4 4 4-4"/>"#,
            Icon::ChevronRight => r#"<path d="M6 4l4 4-4 4"/>"#,
            Icon::ChevronLeft => r#"<path d="M10 4L6 8l4 4"/>"#,
            Icon::Undo => {
                r#"<path d="M5.5 6.5H10a3 3 0 0 1 0 6H6"/><path d="M7.5 4L5 6.5 7.5 9"/>"#
            }
            Icon::Redo => {
                r#"<path d="M10.5 6.5H6a3 3 0 0 0 0 6h4"/><path d="M8.5 4L11 6.5 8.5 9"/>"#
            }
            Icon::Scissors => {
                r#"<circle cx="4.5" cy="11.5" r="2"/><circle cx="11.5" cy="11.5" r="2"/><path d="M5.9 10.1L12 2.5M10.1 10.1L4 2.5"/>"#
            }
            Icon::Folder => {
                r#"<path d="M1.75 4.25a1.5 1.5 0 0 1 1.5-1.5h2.6l1.5 1.5h5.4a1.5 1.5 0 0 1 1.5 1.5v6a1.5 1.5 0 0 1-1.5 1.5h-9.5a1.5 1.5 0 0 1-1.5-1.5z"/>"#
            }
            Icon::Chat => {
                r#"<path d="M2.25 4.5a2 2 0 0 1 2-2h7.5a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2H7l-3 2.5v-2.5h0a2 2 0 0 1-1.75-2z"/>"#
            }
            Icon::Check => r#"<path d="M3.25 8.5l3 3 6.5-7"/>"#,
            Icon::Search => r#"<circle cx="7" cy="7" r="4.25"/><path d="M10.25 10.25l3.5 3.5"/>"#,
            Icon::Archive => {
                r#"<rect x="1.75" y="2.75" width="12.5" height="3" rx="1"/><path d="M2.75 5.75v6.5a1 1 0 0 0 1 1h8.5a1 1 0 0 0 1-1v-6.5M6.5 8.5h3"/>"#
            }
            Icon::Trash => {
                r#"<path d="M2.5 4.25h11M6 4.25V2.75h4v1.5M3.75 4.25l.7 8.6a1 1 0 0 0 1 .9h5.1a1 1 0 0 0 1-.9l.7-8.6"/>"#
            }
            Icon::Film => {
                r#"<rect x="1.75" y="2.75" width="12.5" height="10.5" rx="1.5"/><path d="M4.75 2.75v10.5M11.25 2.75v10.5M1.75 6h3M1.75 10h3M11.25 6h3M11.25 10h3"/>"#
            }
            Icon::Sparkle => {
                r#"<path d="M8 1.75l1.4 3.85 3.85 1.4-3.85 1.4L8 12.25 6.6 8.4 2.75 7l3.85-1.4z"/>"#
            }
            Icon::Dot => r#"<circle cx="8" cy="8" r="3" fill="black" stroke="none"/>"#,
            Icon::StepBack => {
                r#"<path d="M4 3.5v9"/><path d="M12 3.6v8.8a.4.4 0 0 1-.62.33L6 8.33a.4.4 0 0 1 0-.66l5.38-4.4a.4.4 0 0 1 .62.33z" fill="black"/>"#
            }
            Icon::StepForward => {
                r#"<path d="M12 3.5v9"/><path d="M4 3.6v8.8a.4.4 0 0 0 .62.33L10 8.33a.4.4 0 0 0 0-.66L4.62 3.27A.4.4 0 0 0 4 3.6z" fill="black"/>"#
            }
            Icon::Import => {
                r#"<path d="M8 2.5V10M5 7l3 3 3-3"/><path d="M3 8.5v4a1.25 1.25 0 0 0 1.25 1.25h7.5A1.25 1.25 0 0 0 13 12.5v-4"/>"#
            }
            Icon::More => {
                r#"<circle cx="3.5" cy="8" r="1.1" fill="black" stroke="none"/><circle cx="8" cy="8" r="1.1" fill="black" stroke="none"/><circle cx="12.5" cy="8" r="1.1" fill="black" stroke="none"/>"#
            }
            Icon::Eye => {
                r#"<path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8 12.1 12.5 8 12.5 1.5 8 1.5 8z"/><circle cx="8" cy="8" r="1.9"/>"#
            }
            Icon::Lock => {
                r#"<rect x="3.25" y="7" width="9.5" height="6.75" rx="1.25"/><path d="M5.25 7V5.25a2.75 2.75 0 0 1 5.5 0V7"/>"#
            }
            Icon::Mute => {
                r#"<path d="M2.5 6.25h2.25L8 3.5v9L4.75 9.75H2.5z"/><path d="M10.5 6l3 4M13.5 6l-3 4"/>"#
            }
            Icon::Clock => r#"<circle cx="8" cy="8" r="5.75"/><path d="M8 4.75V8l2.25 1.5"/>"#,
            Icon::Loop => {
                r#"<path d="M11 2.5l2 2-2 2"/><path d="M13 4.5H5.5a3 3 0 0 0-3 3"/><path d="M5 13.5l-2-2 2-2"/><path d="M3 11.5h7.5a3 3 0 0 0 3-3"/>"#
            }
            Icon::MaskRectangle => r#"<rect x="2.5" y="3.5" width="11" height="9"/>"#,
            Icon::MaskEllipse => r#"<ellipse cx="8" cy="8" rx="6" ry="5"/>"#,
            Icon::Pen => {
                r#"<path d="M5.6 10.4l7-7"/><path d="M3 13l1.3-3.6 2.3 2.3z" fill="black"/>"#
            }
            Icon::List => {
                r#"<path d="M5.5 4h8M5.5 8h8M5.5 12h8"/><circle cx="2.75" cy="4" r=".85" fill="black" stroke="none"/><circle cx="2.75" cy="8" r=".85" fill="black" stroke="none"/><circle cx="2.75" cy="12" r=".85" fill="black" stroke="none"/>"#
            }
            Icon::Grid => {
                r#"<rect x="2" y="2" width="5" height="5" rx=".75"/><rect x="9" y="2" width="5" height="5" rx=".75"/><rect x="2" y="9" width="5" height="5" rx=".75"/><rect x="9" y="9" width="5" height="5" rx=".75"/>"#
            }
            Icon::Audio => {
                r#"<path d="M2.25 6.25H4.5L7.5 3.5v9L4.5 9.75H2.25z"/><path d="M10 5.75a3 3 0 0 1 0 4.5M11.9 3.9a5.6 5.6 0 0 1 0 8.2"/>"#
            }
            Icon::Dice => {
                r#"<rect x="2.25" y="2.25" width="11.5" height="11.5" rx="2.25"/><circle cx="5.5" cy="5.5" r=".95" fill="black" stroke="none"/><circle cx="8" cy="8" r=".95" fill="black" stroke="none"/><circle cx="10.5" cy="10.5" r=".95" fill="black" stroke="none"/>"#
            }
            Icon::ZoomIn => {
                r#"<circle cx="7" cy="7" r="4.25"/><path d="M10.25 10.25l3.5 3.5M5 7h4M7 5v4"/>"#
            }
            Icon::ZoomOut => {
                r#"<circle cx="7" cy="7" r="4.25"/><path d="M10.25 10.25l3.5 3.5M5 7h4"/>"#
            }
            Icon::Kerf => r#"<path d="M3 3.5h4v5.5h2V3.5h4v9H3z" fill="black" stroke="none"/>"#,
        }
    }

    pub fn svg(self) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            self.body()
        )
    }

    pub const ALL: [Icon; 45] = [
        Icon::Sidebar,
        Icon::Panel,
        Icon::Play,
        Icon::Pause,
        Icon::Plus,
        Icon::Gear,
        Icon::Send,
        Icon::Stop,
        Icon::Export,
        Icon::Media,
        Icon::Close,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::ChevronLeft,
        Icon::Undo,
        Icon::Redo,
        Icon::Scissors,
        Icon::Folder,
        Icon::Chat,
        Icon::Check,
        Icon::Search,
        Icon::Archive,
        Icon::Trash,
        Icon::Film,
        Icon::Sparkle,
        Icon::Dot,
        Icon::StepBack,
        Icon::StepForward,
        Icon::Import,
        Icon::More,
        Icon::Eye,
        Icon::Lock,
        Icon::Mute,
        Icon::Clock,
        Icon::Loop,
        Icon::MaskRectangle,
        Icon::MaskEllipse,
        Icon::Pen,
        Icon::List,
        Icon::Grid,
        Icon::Audio,
        Icon::Dice,
        Icon::ZoomIn,
        Icon::ZoomOut,
        Icon::Kerf,
    ];
}

/// Serves the icons, and Kerf's own app icon, to GPUI.
pub struct Assets;

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == "icons/app.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icon/industrial-kerf.svg"
            ))));
        }
        let Some(name) = path
            .strip_prefix("icons/")
            .and_then(|name| name.strip_suffix(".svg"))
        else {
            return Ok(None);
        };
        Ok(Icon::ALL
            .iter()
            .find(|icon| icon.name() == name)
            .map(|icon| Cow::Owned(icon.svg().into_bytes())))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        if path.starts_with("icons") {
            return Ok(Icon::ALL.iter().map(|icon| icon.path()).collect());
        }
        Ok(Vec::new())
    }
}

/// An element showing nothing, where an icon would be.
pub fn spacer(size: f32) -> impl IntoElement {
    gpui::div().size(gpui::px(size)).flex_none()
}
