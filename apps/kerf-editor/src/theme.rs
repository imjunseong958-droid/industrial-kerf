//! The editor's design tokens: every colour, type style, radius, spacing,
//! control height, motion and layout size the editor uses, and nothing else.
//! The editor is dark only, so each token has one value.
//!
//! Colours are `0xRRGGBB` / `0xRRGGBBAA` literals turned into [`gpui::Rgba`].

use gpui::{rgb, rgba, FontWeight, Rgba};

// ---------------------------------------------------------------- colour

/// Around a frame; the window's room.
pub fn surround() -> Rgba {
    rgb(0x0e0e0e)
}
/// The window's background.
pub fn surround_dim() -> Rgba {
    rgb(0x050505)
}
/// Bin grid, timeline lanes, scope plots.
pub fn well() -> Rgba {
    rgb(0x121212)
}
/// Toolbar, strips, inspector, bands.
pub fn chrome() -> Rgba {
    rgb(0x171717)
}
/// Menus, tooltips, clips.
pub fn card() -> Rgba {
    rgb(0x1c1c1c)
}
/// The folders and conversations, a step up from the room.
pub fn sidebar() -> Rgba {
    rgb(0x1e1e1e)
}
/// Hover fill, segmented track, proposal card, composer.
pub fn raised() -> Rgba {
    rgb(0x262626)
}
/// Selected/on fill, avatars.
pub fn on() -> Rgba {
    rgb(0x303030)
}
/// Hover over a raised surface.
pub fn hover2() -> Rgba {
    rgb(0x363636)
}
/// What opens from a press is a step up from what it opens over.
pub fn popup() -> Rgba {
    raised()
}
pub fn popup_hover() -> Rgba {
    hover2()
}

// The inks are paper, not white: warmed a little, as type on an old page is.
/// Primary text.
pub fn ink() -> Rgba {
    rgb(0xece6da)
}
/// Secondary text, icon default.
pub fn ink2() -> Rgba {
    rgb(0xada79b)
}
/// Tertiary text, meta.
pub fn ink3() -> Rgba {
    rgb(0x878278)
}
/// Disabled.
pub fn dis() -> Rgba {
    rgb(0x5c5c5c)
}
/// Slider/progress track, minor ruler ticks.
pub fn track() -> Rgba {
    rgb(0x3a3a3a)
}
/// Slider fill, major ticks, played bar.
pub fn track2() -> Rgba {
    rgb(0x6a6a6a)
}

/// Hairlines.
pub fn line() -> Rgba {
    rgba(0xffffff14)
}
/// Control outlines.
pub fn line2() -> Rgba {
    rgba(0xffffff24)
}
/// Hover outline.
pub fn lineh() -> Rgba {
    rgba(0xffffff38)
}

// The colours that say something are faded: printed rather than lit.
/// Primary button, focus ring, selected range.
pub fn accent() -> Rgba {
    rgb(0x4a6ab0)
}
pub fn accent_hover() -> Rgba {
    rgb(0x405e9e)
}
/// Timecode links, accepted text, Kerf at work.
pub fn accent_ink() -> Rgba {
    rgb(0xa0b4d8)
}
pub fn accent_wash() -> Rgba {
    rgba(0x4a6ab024)
}
/// Text on an accent fill.
pub fn accent_text() -> Rgba {
    rgb(0xffffff)
}
/// Sage: pass, additions, connected, a clip a proposal changes.
pub fn pos() -> Rgba {
    rgb(0x86a87c)
}
/// Faded brick: fail, blocking, removals.
pub fn neg() -> Rgba {
    rgb(0xd8806e)
}
/// The playhead: a dusty violet, found at once.
pub fn playhead() -> Rgba {
    rgb(0xa084d4)
}
/// Ochre: what wants a look.
pub fn warn() -> Rgba {
    rgb(0xc8a264)
}

/// The colours a marker on the ruler can be: the nine names a document
/// gives its markers, each its own colour, every one reading at 3:1 or
/// more on the ruler and the lanes ([`contrast`]). A marker with no colour,
/// or one of no name here, is green, as Qt drew it.
pub mod marker {
    use gpui::{rgb, Rgba};

    /// The names, in the order a menu would offer them.
    pub const NAMES: [&str; 9] = [
        "red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "white",
    ];

    pub fn red() -> Rgba {
        super::neg()
    }
    pub fn orange() -> Rgba {
        rgb(0xe0904a)
    }
    pub fn yellow() -> Rgba {
        super::warn()
    }
    pub fn green() -> Rgba {
        super::pos()
    }
    pub fn cyan() -> Rgba {
        rgb(0x5cb8c4)
    }
    pub fn blue() -> Rgba {
        super::accent_ink()
    }
    pub fn purple() -> Rgba {
        rgb(0xb48ce6)
    }
    pub fn pink() -> Rgba {
        rgb(0xe07ab0)
    }
    pub fn white() -> Rgba {
        super::ink()
    }

    /// The colour of a marker named `name` (any case, spaces round it
    /// ignored); none, or a name not of the nine, is green.
    pub fn named(name: Option<&str>) -> Rgba {
        match name.map(|name| name.trim().to_ascii_lowercase()).as_deref() {
            Some("red") => red(),
            Some("orange") => orange(),
            Some("yellow") => yellow(),
            Some("cyan") => cyan(),
            Some("blue") => blue(),
            Some("purple") => purple(),
            Some("pink") => pink(),
            Some("white") => white(),
            _ => green(),
        }
    }
}

/// The WCAG contrast ratio of two opaque colours, 1..21.
pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    fn channel(value: f32) -> f32 {
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    let luminance = |c: Rgba| 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b);
    let (light, dark) = {
        let (x, y) = (luminance(a), luminance(b));
        if x > y {
            (x, y)
        } else {
            (y, x)
        }
    };
    (light + 0.05) / (dark + 0.05)
}

/// What the editor is doing, in two hues only.
pub mod activity {
    use gpui::{rgb, rgba, Rgba};
    /// Kerf looking or cutting.
    pub fn kerf() -> Rgba {
        super::accent_ink()
    }
    /// The computer rendering or making: the one bright colour.
    pub fn gpu() -> Rgba {
        rgb(0xe0a448)
    }
    pub fn gpu_stripe() -> Rgba {
        rgba(0xe0a4484d)
    }
    /// A chip's ground over the picture.
    pub fn chip() -> Rgba {
        rgba(0x0000008c)
    }
    /// A conversation where something waits for you to look at it.
    pub fn review() -> Rgba {
        super::accent_ink()
    }
    /// A conversation that stopped on an error.
    pub fn failed() -> Rgba {
        super::neg()
    }
}

/// A colour with its alpha replaced by `alpha` (0..1).
pub fn with_alpha(color: Rgba, alpha: f32) -> Rgba {
    Rgba {
        a: alpha.clamp(0.0, 1.0),
        ..color
    }
}

/// The colour `t` of the way from `a` to `b`.
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

// ------------------------------------------------------------------ type

/// A size/line pair, weight and face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStyle {
    pub px: f32,
    pub line: f32,
    pub weight: FontWeight,
    pub mono: bool,
}

const fn style(px: f32, line: f32, medium: bool, mono: bool) -> TypeStyle {
    TypeStyle {
        px,
        line,
        weight: if medium {
            FontWeight::MEDIUM
        } else {
            FontWeight::NORMAL
        },
        mono,
    }
}

pub mod text {
    use super::{style, TypeStyle};
    pub const BODY: TypeStyle = style(13.0, 20.0, false, false);
    pub const BODY_NAME: TypeStyle = style(13.0, 20.0, true, false);
    pub const SIDE: TypeStyle = style(15.0, 20.0, false, false);
    pub const SIDE_NAME: TypeStyle = style(15.0, 20.0, true, false);
    pub const SIDE_NOTE: TypeStyle = style(14.0, 20.0, false, false);
    pub const DISPLAY: TypeStyle = style(34.0, 44.0, true, false);
    pub const READING: TypeStyle = style(14.0, 22.0, false, false);
    pub const TITLE: TypeStyle = style(14.0, 20.0, true, false);
    pub const CONTROL: TypeStyle = style(12.0, 16.0, false, false);
    pub const CONTROL_STRONG: TypeStyle = style(12.0, 16.0, true, false);
    pub const META: TypeStyle = style(11.0, 16.0, false, false);
    pub const META_STRONG: TypeStyle = style(11.0, 16.0, true, false);
    pub const TIMECODE: TypeStyle = style(12.0, 16.0, false, true);
    pub const TIMECODE_STRONG: TypeStyle = style(12.0, 16.0, true, true);
    pub const MONO_SMALL: TypeStyle = style(11.0, 16.0, false, true);
    pub const MONO_LARGE: TypeStyle = style(13.0, 20.0, true, true);
    pub const CAPTION: TypeStyle = style(15.0, 20.0, false, false);
    pub const BADGE: TypeStyle = style(11.0, 12.0, false, true);
    pub const MENU_ITEM: TypeStyle = style(14.0, 20.0, false, false);
    pub const MENU_QUIET: TypeStyle = style(13.0, 18.0, false, false);
}

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

    /// The interface's faces, as Qt's Interface font offered them.
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

// ---------------------------------------------------------- radii and spacing

pub mod radius {
    pub const SMALL: f32 = 4.0;
    pub const MARKER: f32 = 5.0;
    pub const CONTROL: f32 = 6.0;
    pub const CARD: f32 = 8.0;
    pub const PANEL: f32 = 12.0;
}

pub mod space {
    pub const S2: f32 = 2.0;
    pub const S4: f32 = 4.0;
    pub const S6: f32 = 6.0;
    pub const S8: f32 = 8.0;
    pub const S12: f32 = 12.0;
    pub const S16: f32 = 16.0;
    pub const PANEL_LEFT: f32 = 16.0;
}

pub mod height {
    pub const H20: f32 = 20.0;
    pub const H24: f32 = 24.0;
    pub const H28: f32 = 28.0;
    pub const H32: f32 = 32.0;
    pub const H40: f32 = 40.0;
}

// ---------------------------------------------------------------- motion

/// What each scene takes; MOTION_UI.md says which is which. Durations in
/// milliseconds, distances in points.
pub mod motion {
    use std::time::Duration;

    const fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    /// Hover and colour changes.
    pub const HOVER: Duration = ms(120);
    /// Popovers and fades.
    pub const FADE: Duration = ms(160);
    /// The segmented control's fill, a switch's knob.
    pub const SEGMENT: Duration = ms(200);
    /// The window's regions moving to a new arrangement (the stage growing).
    pub const STAGE: Duration = ms(850);
    /// A pane opened or closed by hand: the sidebar, the panel, the Media band.
    pub const PANEL: Duration = ms(420);
    /// Falling back or coming forward: dims.
    pub const SETTLE: Duration = ms(420);
    /// A thing fading in where it is.
    pub const APPEAR: Duration = ms(320);
    /// A thing moving up into place.
    pub const RISE: Duration = ms(620);
    /// Settings' pages and a shot's graph coming up into the room.
    pub const PAGE: Duration = ms(520);
    /// A tile appearing.
    pub const POP: Duration = ms(460);
    pub const POP_STAGGER: Duration = ms(70);
    /// Between the sidebar's archived conversations as they unfold.
    pub const UNFOLD_STAGGER: Duration = ms(30);
    /// The light once over a working conversation's title, and the rest after it.
    pub const TITLE_SHIMMER: Duration = ms(2200);
    /// The share of a title's pass the light takes to cross it.
    pub const TITLE_CROSS: f32 = 0.7;
    /// The light once over "Working" while Kerf works.
    pub const WORK_SHIMMER: Duration = ms(1600);
    /// The light once round the picture while something works.
    pub const EDGE_TURN: Duration = ms(3200);
    /// The light and its chip coming and going.
    pub const EDGE_FADE: Duration = ms(420);
    /// The stripes over a clip or a tile being made, moving one stripe on.
    pub const STRIPE_CYCLE: Duration = ms(1600);
    /// A making's clock, and all that moves under Reduce Motion.
    pub const CLOCK_FRAME: Duration = ms(1000);
    /// The working chip's dot, once.
    pub const WORK_PULSE: Duration = ms(1200);
    /// Done: how long a finished render or cut says so.
    pub const DONE_HOLD: Duration = ms(2400);
    /// A picture or tile flying to where it lands (an export going home).
    pub const FLIGHT: Duration = ms(720);
    /// Its fade once landed, over what it became.
    pub const FLIGHT_LAND: Duration = ms(240);
    /// Between tiles flying one after another.
    pub const FLIGHT_STAGGER: Duration = ms(160);
    /// How long a press waits to learn it was the first of a double-click
    /// (the sidebar's edge: one press puts it away, two reset its width).
    pub const DOUBLE_CLICK: Duration = ms(400);
    /// How often the bridge is asked for events and frames.
    pub const POLL: Duration = ms(16);
    /// How long a notice stays at the conversation's foot (Qt's 6 s).
    pub const NOTICE_HOLD: Duration = ms(6000);
    /// And one saying what went wrong, which takes longer to read (a
    /// failed push's ssh and git lines, an import's refusal).
    pub const NOTICE_ERROR_HOLD: Duration = ms(12000);

    /// Where a flight lands in a row, as Qt landed it: in from the row's
    /// left and top (and as far up from its bottom), and how wide.
    pub const LANDING_INSET: (f32, f32) = (8.0, 4.0);
    pub const LANDING_WIDTH: f32 = 44.0;
    /// Points Settings' pages come up.
    pub const PAGE_RISE: f32 = 16.0;
    /// Points a message rises as it fades in.
    pub const APPEAR_RISE: f32 = 6.0;
    /// The light's tail, a share of the way round.
    pub const EDGE_TAIL: f32 = 0.22;
    /// Points the light is wide, outside the picture's edge.
    pub const EDGE_WIDTH: f32 = 2.0;
    /// And how far out from it.
    pub const EDGE_GAP: f32 = 1.0;
    /// The rest of the ring, under the light, as a share of it.
    pub const EDGE_REST: f32 = 0.28;
    /// The ring under Reduce Motion, whole and steady, with no light running.
    pub const EDGE_STILL: f32 = 0.85;
    /// Points from one stripe to the next.
    pub const STRIPE_SPACE: f32 = 10.0;
    /// The conversation while a modal covers it.
    pub const SCRIM: f32 = 0.55;
    /// Points a menu or a popover rises as it fades in over `FADE`.
    pub const MENU_RISE: f32 = 4.0;
    /// How long the pointer rests on a control before its tip shows.
    pub const TIP_DELAY: Duration = ms(400);
    /// A tip hidden this recently (or one still showing) makes the next one
    /// show at once, with no fade.
    pub const TIP_QUICK: Duration = ms(300);
    /// The system's Reduce motion setting is read again when a motion
    /// starts and the last read is older than this, as Qt read it.
    pub const SYSTEM_POLL: Duration = ms(1000);
    /// The most the first motion waits for the first read of it, made as
    /// the editor starts.
    pub const SYSTEM_FIRST_WAIT: Duration = ms(300);

    /// The one easing curve: cubic-bezier(.22, 1, .36, 1).
    pub const EASE: (f32, f32, f32, f32) = (0.22, 1.0, 0.36, 1.0);
}

// ---------------------------------------------------------------- layout

pub mod layout {
    pub const TOOLBAR: f32 = 40.0;
    pub const STRIP: f32 = 28.0;
    pub const MEDIA_BAND: f32 = 152.0;
    pub const SIDEBAR: f32 = 280.0;
    pub const SIDEBAR_LEAST: f32 = 232.0;
    pub const SIDEBAR_LEAVES: f32 = 640.0;
    pub const SIDEBAR_GRIP: f32 = 8.0;
    pub const SIDEBAR_WINDOW_LEAST: f32 = 1100.0;
    pub const SIDEBAR_FOOT: f32 = 44.0;
    pub const RAIL: f32 = 448.0;
    pub const RAIL_LEAST: f32 = 320.0;
    pub const CONVERSATION_COLUMN: f32 = 720.0;
    pub const TIMELINE: f32 = 260.0;
    pub const TRACK_HEADER: f32 = 120.0;
    pub const RULER: f32 = 24.0;
    pub const VIDEO_LANE: f32 = 44.0;
    pub const AUDIO_LANE: f32 = 36.0;
    pub const SUBTITLE_LANE: f32 = 26.0;
    pub const TRANSPORT: f32 = 40.0;
    /// The chosen clip's inspector, beside the picture.
    pub const INSPECTOR: f32 = 300.0;
    /// The tallest the composer grows with its lines before it scrolls.
    pub const COMPOSER: f32 = 110.0;
    /// A bar under the toolbar (kerfd gone, the file changed, an error).
    pub const BAR: f32 = 40.0;
    /// Points a press on an edge or a handle moves before it is a drag.
    pub const DRAG_START: f32 = 4.0;
    /// The widest a notice is, at the conversation's foot; longer words wrap.
    pub const NOTICE_MOST: f32 = 560.0;
    /// Width over height from which a picture is wide (Qt's isWide).
    pub const WIDE: f32 = 1.2;
    /// The timeline's share of the panel's height under a wide picture,
    /// and its least there (Qt's timelineWideShare and timelineWideLeast;
    /// the transport, which Qt's timeline carried, counted in).
    pub const TIMELINE_WIDE_SHARE: f32 = 0.5625;
    pub const TIMELINE_WIDE_LEAST: f32 = 260.0;
    /// And under a vertical or square one.
    pub const TIMELINE_UPRIGHT_SHARE: f32 = 0.36;
    pub const TIMELINE_UPRIGHT_LEAST: f32 = 220.0;
    /// What the Media band leaves the timeline under it before the
    /// picture gives way.
    pub const MEDIA_BAND_TIMELINE_LEAST: f32 = 200.0;

    /// Points wide the ring round what has the keys, just inside its edge.
    pub const FOCUS_RING: f32 = 2.0;
    /// On what is filled with the accent (a primary button), where a ring
    /// inside would not show, it goes round it outside, this far out, as
    /// the design drew every ring.
    pub const FOCUS_RING_OUT: f32 = 1.0;
    /// What a panel one goes into (Settings, Finish) leaves of the window
    /// round it at least, and the share of the height to spare above it.
    pub const MODAL_MARGIN: f32 = 48.0;
    pub const MODAL_ABOVE: f32 = 0.25;
    /// A list that scrolls with no bar: the fade at its top while a row is
    /// cut there, and at its foot while there is more below.
    pub const TOP_FADE: f32 = 12.0;
    pub const BOTTOM_FADE: f32 = 32.0;
    /// A tip: its height, its gap from the control, how far it keeps from
    /// the window's sides, and where a control is low enough to have its
    /// tip above (its bottom past `TIP_FLIP_BELOW`, its top past
    /// `TIP_FLIP_ABOVE`).
    pub const TIP_HEIGHT: f32 = 24.0;
    pub const TIP_GAP: f32 = 6.0;
    pub const TIP_EDGE: f32 = 8.0;
    pub const TIP_FLIP_BELOW: f32 = 540.0;
    pub const TIP_FLIP_ABOVE: f32 = 100.0;
    /// A menu's row, and how wide a menu is at least.
    pub const MENU_ROW: f32 = 30.0;
    pub const MENU_LEAST: f32 = 180.0;

    /// The widest a drag leaves the sidebar in a window `window` wide.
    pub fn sidebar_most(window: f32) -> f32 {
        (window - SIDEBAR_LEAVES).max(SIDEBAR_LEAST)
    }
}

// ---------------------------------------------------------------- faces in use

static FACE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static READING: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static PROSE_STEP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// The interface's face, as chosen in Settings.
pub fn face() -> Face {
    Face::ALL[FACE.load(std::sync::atomic::Ordering::Relaxed) as usize % Face::ALL.len()]
}

/// What the conversation reads in, as chosen in Settings.
pub fn reading_face() -> ReadingFace {
    ReadingFace::ALL
        [READING.load(std::sync::atomic::Ordering::Relaxed) as usize % ReadingFace::ALL.len()]
}

/// Puts a face saved alone in force: `Serif` is IBM Plex with the serif
/// for reading ([`Face::split`]), any other reads in itself.
pub fn set_face(face: Face, prose_step: i32) {
    let (ui, reading) = face.split();
    set_faces(ui, reading, prose_step);
}

/// Puts the interface's face, the conversation's, and the points added to
/// the conversation's size in force. Views draw in them from their next
/// frame (`Window::refresh`).
pub fn set_faces(ui: Face, reading: ReadingFace, prose_step: i32) {
    let (ui, reading) = match (ui, reading) {
        // Serif named as the interface's is the old single choice.
        (Face::Serif, _) => (Face::Plex, ReadingFace::Serif),
        other => other,
    };
    let index = |all: &[Face], face: Face| all.iter().position(|known| *known == face);
    FACE.store(
        index(&Face::ALL, ui).unwrap_or(0) as u8,
        std::sync::atomic::Ordering::Relaxed,
    );
    READING.store(
        ReadingFace::ALL
            .iter()
            .position(|known| *known == reading)
            .unwrap_or(0) as u8,
        std::sync::atomic::Ordering::Relaxed,
    );
    PROSE_STEP.store(
        prose_step.clamp(-2, 6),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Points added to what is said in the conversation.
pub fn prose_step() -> f32 {
    PROSE_STEP.load(std::sync::atomic::Ordering::Relaxed) as f32
}

/// The interface's font: the chosen face, with Hangul from its partner.
pub fn ui_font(weight: FontWeight) -> gpui::Font {
    font(face().ui(), weight, false)
}

pub fn prose_font(weight: FontWeight) -> gpui::Font {
    font(reading_face().family(face()), weight, false)
}

pub fn mono_font(weight: FontWeight) -> gpui::Font {
    font(face().mono(), weight, true)
}

/// A font in `family` as the editor sets it: Hangul and what else the
/// family lacks from the bundled faces, and tabular figures (and, in mono,
/// the plain zero) so counts and times do not shift as they tick. Settings'
/// samples of each face are set with it.
pub fn font(family: &'static str, weight: FontWeight, mono: bool) -> gpui::Font {
    let hangul = if family == SERIF_FAMILY {
        SERIF_HANGUL_FAMILY
    } else if family == system_family() || family == INTER_FAMILY {
        PRETENDARD_FAMILY
    } else {
        PLEX_HANGUL_FAMILY
    };
    gpui::Font {
        family: family.into(),
        features: font_features(mono),
        fallbacks: Some(gpui::FontFallbacks::from_fonts(vec![
            hangul.to_owned(),
            PRETENDARD_FAMILY.to_owned(),
            JETBRAINS_FAMILY.to_owned(),
        ])),
        weight,
        style: gpui::FontStyle::Normal,
    }
}

/// The OpenType features every face is set with: `tnum` (tabular
/// figures), and in mono `zero` off (the plain zero), as Qt set them.
pub fn font_features(mono: bool) -> gpui::FontFeatures {
    static TEXT: std::sync::OnceLock<gpui::FontFeatures> = std::sync::OnceLock::new();
    static MONO: std::sync::OnceLock<gpui::FontFeatures> = std::sync::OnceLock::new();
    let make = |mono: bool| {
        let mut tags = vec![("tnum".to_owned(), 1)];
        if mono {
            tags.push(("zero".to_owned(), 0));
        }
        gpui::FontFeatures(std::sync::Arc::new(tags))
    };
    if mono {
        MONO.get_or_init(|| make(true)).clone()
    } else {
        TEXT.get_or_init(|| make(false)).clone()
    }
}

/// Sets `element`'s type in `style`.
pub fn type_style<E: gpui::Styled>(element: E, style: TypeStyle) -> E {
    let font = if style.mono {
        mono_font(style.weight)
    } else {
        ui_font(style.weight)
    };
    element
        .font(font)
        .text_size(gpui::px(style.px))
        .line_height(gpui::px(style.line))
}

/// Sets `element`'s type for reading: what is said in the conversation.
pub fn reading<E: gpui::Styled>(element: E) -> E {
    let style = text::READING;
    element
        .font(prose_font(style.weight))
        .text_size(gpui::px(style.px + prose_step()))
        .line_height(gpui::px(style.line + prose_step() * 1.5))
}

/// The bundled faces, to register at start.
pub fn bundled_fonts() -> Vec<std::borrow::Cow<'static, [u8]>> {
    macro_rules! fonts {
        ($($name:literal),* $(,)?) => {
            vec![$(std::borrow::Cow::Borrowed(&include_bytes!(concat!("../assets/fonts/", $name))[..])),*]
        };
    }
    fonts![
        "IBMPlexSans-Regular.ttf",
        "IBMPlexSans-Medium.ttf",
        "IBMPlexSansKR-Regular.ttf",
        "IBMPlexSansKR-Medium.ttf",
        "IBMPlexMono-Regular.ttf",
        "IBMPlexMono-Medium.ttf",
        "Inter-Regular.ttf",
        "Inter-Medium.ttf",
        "JetBrainsMono-Regular.ttf",
        "JetBrainsMono-Medium.ttf",
        "Pretendard-Regular.ttf",
        "Pretendard-Medium.ttf",
        "NotoSerif-Regular.ttf",
        "NanumMyeongjo-Regular.ttf",
    ]
}

/// The black of a screen around and under a picture.
pub fn overlay_screen() -> Rgba {
    rgb(0x000000)
}

/// What is drawn over the picture, whatever it shows: white over a dark
/// rule reads on any picture, in the accent's own hue too, so none of it
/// is the accent but the chosen mask's tag and its handles.
pub mod over_picture {
    use gpui::{rgb, rgba, Rgba};
    /// A chosen mask's outline, a motion path's curve, a handle's edge.
    pub fn line() -> Rgba {
        rgb(0xffffff)
    }
    /// A mask's outline while it is not the chosen one (white, 70%).
    pub fn line_idle() -> Rgba {
        rgba(0xffffffb3)
    }
    /// The dark rule under a line, so that it reads on a light picture (black, 55%).
    pub fn rule() -> Rgba {
        rgba(0x0000008c)
    }
    /// A mask's name tag; the chosen one's is the accent (black, 60%).
    pub fn tag() -> Rgba {
        rgba(0x00000099)
    }
    /// The words on a tag.
    pub fn tag_ink() -> Rgba {
        rgb(0xffffff)
    }
    /// Title-safe corner ticks and the centre cross (white, 45%).
    pub fn tick() -> Rgba {
        rgba(0xffffff73)
    }
    /// The device round the picture: its bezel, and a phone's island.
    pub fn bezel() -> Rgba {
        rgb(0x0a0a0a)
    }
    pub fn island() -> Rgba {
        rgb(0x000000)
    }
    /// The ground under the caption at the playhead, and its words.
    pub fn caption_ground() -> Rgba {
        rgba(0x000000b8)
    }
    pub fn caption_ink() -> Rgba {
        rgb(0xffffff)
    }
    /// The box a check found text overlapping in (the brick, 32%).
    pub fn overlap() -> Rgba {
        rgba(0xec5d6152)
    }
}
