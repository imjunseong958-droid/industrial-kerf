//! The preview render's card over the picture: when it shows, and what it
//! says. Pure, so that both can be checked without a window; the stage
//! draws the card, and the window asks whether it shows before Esc stops
//! the render.

use editor_state::changes::ClipChange;
use editor_state::model;
use kerf_bridge::event::PreviewStatus;
use kerf_bridge::Event;

/// Stop's tip and its key: Esc stops the render while the card shows,
/// ahead of what else Esc would do.
pub const STOP_TIP: &str = "Stop preview render";
pub const STOP_KEY: &str = "Esc";

/// Its title: "Rendering preview · v2", with the version when known.
pub fn title(version: u32) -> String {
    if version > 0 {
        format!("Rendering preview · v{version}")
    } else {
        "Rendering preview".to_owned()
    }
}

/// Its second line: the proposal's first change ("Logo → 00:00:26:12 ·
/// 2 changes", "Logo removed · 1 change"), else the first line of what
/// it does, else what the render's event said.
pub fn detail(changes: &[ClipChange], summary: &str, message: &str, fps: f64) -> String {
    if let Some(first) = changes.first() {
        let name = if first.name.is_empty() {
            &first.clip
        } else {
            &first.name
        };
        let count = match changes.len() {
            1 => " · 1 change".to_owned(),
            n => format!(" · {n} changes"),
        };
        if first.removed {
            return format!("{name} removed{count}");
        }
        return format!("{name} → {}{count}", model::timecode(first.new_start, fps));
    }
    if let Some(line) = summary.lines().map(str::trim).find(|line| !line.is_empty()) {
        return line.to_owned();
    }
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How far it has got, as a whole percent.
pub fn percent(progress: f64) -> String {
    format!("{}%", (progress.clamp(0.0, 1.0) * 100.0).round() as i64)
}

/// What the preview render's card over the picture stands on, for
/// [`render_card_shows`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderCard {
    /// A preview renders.
    pub rendering: bool,
    /// That render was stopped here, and runs on until the bridge says it
    /// ended ([`render_stop_holds`]).
    pub stopped: bool,
    /// Kerf builds its cut: the picture is its live preview, and the card
    /// keeps out of the way.
    pub kerf_building: bool,
    /// A change reel plays (it renders what it plays) or Kerf cuts its
    /// first cut: its own strip says so.
    pub reel: bool,
    /// The picture shows: the panel is open with its pane in.
    pub picture_shown: bool,
}

/// Whether the preview render's card shows over the picture
/// (`StageView::preview_card`): Esc stops the render only then, never
/// one no card shows; otherwise Esc does what else it would.
pub fn render_card_shows(card: RenderCard) -> bool {
    card.rendering && !card.stopped && !card.kerf_building && !card.reel && card.picture_shown
}

/// Whether a render stopped here (the render of proposal `stopped`) is
/// still the one running after bridge `event`, its card still done with:
/// until its run ends (a `preview` of any other status), or another
/// proposal's render takes over, as the stage keeps its own Stop.
pub fn render_stop_holds(stopped: Option<&str>, event: &Event) -> bool {
    let Some(stopped) = stopped else {
        return false;
    };
    let Event::Preview(preview) = event else {
        return true;
    };
    preview.status == PreviewStatus::Running
        && preview.proposal_id.as_deref().unwrap_or_default() == stopped
}
