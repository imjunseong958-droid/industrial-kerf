use cut_core::{FrameGrid, FrameIndex};
use cut_domain::{CropRegion, Effect, Keyframe, Transform, Transition, TrimEdge};
use cut_runtime::{
    PendingProposal, ProposalFreshness, ProposalImpact, ProposalTimeRange, StateSnapshot,
};
use egui::{
    Color32, FontId, Rect, RichText, ScrollArea, Sense, Slider, Stroke, StrokeKind, WidgetInfo,
    WidgetType,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

pub const TIMELINE_LABEL_WIDTH: f32 = 156.0;
pub const TIMELINE_RULER_HEIGHT: f32 = 30.0;
pub const TIMELINE_ROW_HEIGHT: f32 = 58.0;
pub const DEFAULT_PIXELS_PER_SECOND: f32 = 80.0;
pub const MIN_TIMELINE_ZOOM: f32 = 0.5;
pub const MAX_TIMELINE_ZOOM: f32 = 4.0;
const TRIM_HANDLE_WIDTH: f32 = 8.0;
const SNAP_DISTANCE_PIXELS: f32 = 8.0;

/// Media kind used by timeline decoration and accessibility labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaType {
    Video,
    Audio,
    Image,
    Text,
    #[default]
    Unknown,
}

impl MediaType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Image => "image",
            Self::Text => "text",
            Self::Unknown => "media",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Video => Color32::from_rgb(57, 126, 206),
            Self::Audio => Color32::from_rgb(190, 125, 48),
            Self::Image => Color32::from_rgb(70, 161, 125),
            Self::Text => Color32::from_rgb(147, 93, 184),
            Self::Unknown => Color32::from_gray(105),
        }
    }
}

/// A timeline interval in project seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineRange {
    pub start: f64,
    pub end: f64,
}

impl TimelineRange {
    pub fn new(start: f64, end: f64) -> Option<Self> {
        (start.is_finite() && end.is_finite() && start >= 0.0 && end > start)
            .then_some(Self { start, end })
    }

    pub fn duration(self) -> f64 {
        self.end - self.start
    }

    pub fn intersects(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    pub fn frame_range(self, grid: FrameGrid) -> Range<FrameIndex> {
        grid.frame_at(self.start)..grid.frame_at(self.end)
    }
}

/// A media source projected into the timeline without retaining source JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceViewModel {
    pub id: String,
    pub label: String,
    pub path: String,
    pub media_type: MediaType,
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_audio: Option<bool>,
    pub thumbnail_path: Option<String>,
    pub waveform_path: Option<String>,
    pub waveform_available: Option<bool>,
    pub thumbnail_cache_key: String,
    pub waveform_cache_key: String,
}

/// A typed clip presentation used by the read-only timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipViewModel {
    pub id: String,
    pub source_id: Option<String>,
    pub source_label: String,
    pub media_type: MediaType,
    pub timeline: TimelineRange,
    pub source: Option<TimelineRange>,
    pub source_duration: Option<f64>,
    pub speed: f64,
    pub volume: f64,
    pub opacity: Option<f64>,
    pub crop: Option<CropRegion>,
    pub transform: Option<Transform>,
    pub effects: Vec<Effect>,
    pub keyframes: Vec<Keyframe>,
    pub transitions: Vec<Transition>,
    pub thumbnail_cache_key: Option<String>,
    pub waveform_cache_key: Option<String>,
}

impl ClipViewModel {
    pub fn timeline_start(&self) -> f64 {
        self.timeline.start
    }

    pub fn timeline_end(&self) -> f64 {
        self.timeline.end
    }
}

/// A track row and its stable clip presentations.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackViewModel {
    pub id: String,
    pub index: usize,
    pub name: String,
    pub kind: String,
    pub clip_count: usize,
    pub first_clip_id: Option<String>,
    pub first_clip_volume: f64,
    pub enabled: bool,
    pub muted: bool,
    pub locked: bool,
    pub clips: Vec<ClipViewModel>,
}

/// A transient selection in the timeline. It is never sent to cutd.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TimelineSelection {
    #[default]
    None,
    Track(String),
    Clip(String),
}

impl TimelineSelection {
    pub fn selected_track_id(&self) -> Option<&str> {
        match self {
            Self::Track(id) => Some(id),
            Self::Clip(_) | Self::None => None,
        }
    }

    pub fn selected_clip_id(&self) -> Option<&str> {
        match self {
            Self::Clip(id) => Some(id),
            Self::Track(_) | Self::None => None,
        }
    }
}

/// Scroll and zoom state for the custom timeline surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineViewport {
    pub zoom: f32,
    pub horizontal_scroll: f32,
    pub vertical_scroll: f32,
}

impl Default for TimelineViewport {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            horizontal_scroll: 0.0,
            vertical_scroll: 0.0,
        }
    }
}

impl TimelineViewport {
    pub fn pixels_per_second(self) -> f32 {
        DEFAULT_PIXELS_PER_SECOND * self.zoom.clamp(MIN_TIMELINE_ZOOM, MAX_TIMELINE_ZOOM)
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        if zoom.is_finite() {
            self.zoom = zoom.clamp(MIN_TIMELINE_ZOOM, MAX_TIMELINE_ZOOM);
        }
    }
}

/// Coordinates used by the painter and its hit-testing pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineGeometry {
    pub label_width: f32,
    pub pixels_per_second: f32,
}

impl TimelineGeometry {
    pub fn new(viewport: TimelineViewport) -> Self {
        Self {
            label_width: TIMELINE_LABEL_WIDTH,
            pixels_per_second: viewport.pixels_per_second(),
        }
    }

    /// Convert project time to a content-local pixel, including the label lane.
    pub fn time_to_pixel(self, seconds: f64) -> f32 {
        self.label_width + seconds.max(0.0) as f32 * self.pixels_per_second
    }

    /// Convert a content-local pixel back to project time.
    pub fn pixel_to_time(self, pixel: f32) -> f64 {
        ((pixel - self.label_width) / self.pixels_per_second.max(f32::EPSILON)).max(0.0) as f64
    }

    pub fn time_to_x(self, seconds: f64) -> f32 {
        self.time_to_pixel(seconds)
    }

    pub fn x_to_time(self, pixel: f32) -> f64 {
        self.pixel_to_time(pixel)
    }

    pub fn clip_rect(self, content_rect: Rect, row: usize, range: TimelineRange) -> Rect {
        let top =
            content_rect.min.y + TIMELINE_RULER_HEIGHT + row as f32 * TIMELINE_ROW_HEIGHT + 8.0;
        let bottom =
            content_rect.min.y + TIMELINE_RULER_HEIGHT + (row + 1) as f32 * TIMELINE_ROW_HEIGHT
                - 8.0;
        let left = content_rect.min.x + self.time_to_pixel(range.start);
        let right = (content_rect.min.x + self.time_to_pixel(range.end)).max(left + 3.0);
        Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom))
    }

    pub fn visible_time_range(self, visible_rect: Rect, content_rect: Rect) -> TimelineRange {
        let start = self.pixel_to_time(visible_rect.min.x - content_rect.min.x);
        let end = self
            .pixel_to_time(visible_rect.max.x - content_rect.min.x)
            .max(start + 0.001);
        TimelineRange { start, end }
    }

    pub fn visible_rows(visible_rect: Rect, content_rect: Rect, row_count: usize) -> Range<usize> {
        if row_count == 0 {
            return 0..0;
        }
        let rows_top = content_rect.min.y + TIMELINE_RULER_HEIGHT;
        let first = ((visible_rect.min.y - rows_top) / TIMELINE_ROW_HEIGHT)
            .floor()
            .max(0.0) as usize;
        let last = ((visible_rect.max.y - rows_top) / TIMELINE_ROW_HEIGHT)
            .ceil()
            .max(0.0) as usize
            + 1;
        first.min(row_count)..last.min(row_count)
    }
}

/// The playhead as both a project time and a deterministic frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelinePlayhead {
    pub frame: FrameIndex,
    pub seconds: f64,
}

/// The direct manipulation currently being previewed by the timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineDragKind {
    /// Move the entire clip.
    Move,
    /// Move the clip's leading edge.
    TrimLeading,
    /// Move the clip's trailing edge.
    TrimTrailing,
}

/// Transient timeline interaction state. It is discarded after a command is
/// committed or a revision conflict cancels the gesture.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineDragState {
    pub clip_id: String,
    pub original_track_id: String,
    pub kind: TimelineDragKind,
    pub pointer_start_frame: FrameIndex,
    pub original_start_frame: FrameIndex,
    pub original_end_frame: FrameIndex,
    pub preview_start_frame: FrameIndex,
    pub preview_end_frame: FrameIndex,
    pub preview_track_id: String,
}

/// Mutable interaction state owned by the desktop shell, never by `cutd`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineInteractionState {
    pub drag: Option<TimelineDragState>,
}

impl TimelineInteractionState {
    pub fn cancel_for_revision_conflict(&mut self) {
        self.drag = None;
    }
}

/// A visual-only proposal state derived from the daemon checkpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct ProposalViewModel {
    pub proposal_id: String,
    pub base_revision: u64,
    pub stale: bool,
    pub diff_truncated: bool,
    pub impacts_available: bool,
    pub impacts: Vec<ProposalImpact>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalGhostKind {
    Added,
    Removed,
    MovedOriginal,
    MovedDestination,
    TrimmedOriginal,
    TrimmedProposed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProposalGhost {
    pub clip_id: String,
    pub track_id: String,
    pub range: TimelineRange,
    pub kind: ProposalGhostKind,
}

impl ProposalViewModel {
    pub fn ghosts(&self) -> Vec<ProposalGhost> {
        if self.stale {
            return Vec::new();
        }
        let mut ghosts = Vec::new();
        for impact in &self.impacts {
            match impact {
                ProposalImpact::ClipAdded {
                    clip_id,
                    track_id,
                    timeline,
                    ..
                } => ghosts.push(ProposalGhost {
                    clip_id: clip_id.clone(),
                    track_id: track_id.clone(),
                    range: proposal_range(timeline),
                    kind: ProposalGhostKind::Added,
                }),
                ProposalImpact::ClipRemoved {
                    clip_id,
                    track_id,
                    timeline,
                    ..
                } => ghosts.push(ProposalGhost {
                    clip_id: clip_id.clone(),
                    track_id: track_id.clone(),
                    range: proposal_range(timeline),
                    kind: ProposalGhostKind::Removed,
                }),
                ProposalImpact::ClipMoved {
                    clip_id,
                    from_track_id,
                    from,
                    to_track_id,
                    to,
                } => {
                    ghosts.push(ProposalGhost {
                        clip_id: clip_id.clone(),
                        track_id: from_track_id.clone(),
                        range: proposal_range(from),
                        kind: ProposalGhostKind::MovedOriginal,
                    });
                    ghosts.push(ProposalGhost {
                        clip_id: clip_id.clone(),
                        track_id: to_track_id.clone(),
                        range: proposal_range(to),
                        kind: ProposalGhostKind::MovedDestination,
                    });
                }
                ProposalImpact::ClipTrimmed {
                    clip_id,
                    track_id,
                    before,
                    after,
                    ..
                } => {
                    ghosts.push(ProposalGhost {
                        clip_id: clip_id.clone(),
                        track_id: track_id.clone(),
                        range: proposal_range(before),
                        kind: ProposalGhostKind::TrimmedOriginal,
                    });
                    ghosts.push(ProposalGhost {
                        clip_id: clip_id.clone(),
                        track_id: track_id.clone(),
                        range: proposal_range(after),
                        kind: ProposalGhostKind::TrimmedProposed,
                    });
                }
                ProposalImpact::TrackAdded { .. }
                | ProposalImpact::TrackRemoved { .. }
                | ProposalImpact::TrackReordered { .. }
                | ProposalImpact::ClipPropertyChanged { .. }
                | ProposalImpact::OverlayAdded { .. }
                | ProposalImpact::OverlayRemoved { .. }
                | ProposalImpact::OverlayChanged { .. }
                | ProposalImpact::ProjectSettingChanged { .. }
                | ProposalImpact::Unclassified { .. } => {}
            }
        }
        ghosts
    }
}

/// Read-only projection of the canonical project and its pending proposal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineViewModel {
    pub fps: f64,
    pub duration: f64,
    pub sources: Vec<SourceViewModel>,
    pub tracks: Vec<TrackViewModel>,
    pub proposal: Option<ProposalViewModel>,
}

/// A command-producing interaction emitted by the timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineAction {
    /// Seek the playhead in project seconds.
    Seek(f64),
    /// Place an existing clip on a project frame, optionally on another track.
    PlaceClip {
        clip_id: String,
        timeline_frame: FrameIndex,
        track_id: Option<String>,
    },
    /// Trim one edge of an existing clip at a project frame.
    TrimClip {
        clip_id: String,
        edge: TrimEdge,
        timeline_frame: FrameIndex,
    },
    /// Split an existing clip at an interior project frame.
    SplitClip {
        clip_id: String,
        at_frame: FrameIndex,
    },
    /// Remove the selected clip as one undoable canonical edit.
    RemoveClip { clip_id: String },
}

impl TimelineViewModel {
    pub fn from_snapshot(snapshot: &StateSnapshot) -> Self {
        let mut model = Self::from_project_with_duration(&snapshot.project, snapshot.duration);
        model.proposal = proposal_from_snapshot(snapshot);
        model
    }

    pub fn from_project(project: &Value) -> Self {
        Self::from_project_with_duration(project, project_duration(project))
    }

    fn from_project_with_duration(project: &Value, duration: f64) -> Self {
        let fps = project
            .get("fps")
            .and_then(Value::as_f64)
            .filter(|fps| fps.is_finite() && *fps > 0.0)
            .unwrap_or(30.0);
        let sources = project
            .get("sources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(source_from_value)
            .collect::<Vec<_>>();
        let source_by_id: BTreeMap<&str, &SourceViewModel> = sources
            .iter()
            .map(|source| (source.id.as_str(), source))
            .collect();
        let tracks = project
            .get("tracks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, track)| track_from_value(index, track, &source_by_id, fps))
            .collect();
        Self {
            fps,
            duration: duration.max(0.0),
            sources,
            tracks,
            proposal: None,
        }
    }

    pub fn frame_grid(&self) -> FrameGrid {
        FrameGrid::new(self.fps).expect("timeline projection always has a positive fps")
    }

    pub fn playhead(&self, seconds: f64) -> TimelinePlayhead {
        let grid = self.frame_grid();
        let frame = grid.frame_at(seconds);
        TimelinePlayhead {
            frame,
            seconds: grid.seconds_at(frame),
        }
    }

    pub fn selected(&self, selected_track: Option<&str>) -> Option<&TrackViewModel> {
        selected_track.and_then(|id| self.tracks.iter().find(|track| track.id == id))
    }

    pub fn selected_for_selection(&self, selection: &TimelineSelection) -> Option<&TrackViewModel> {
        match selection {
            TimelineSelection::Track(track_id) => self.selected(Some(track_id)),
            TimelineSelection::Clip(clip_id) => self
                .tracks
                .iter()
                .find(|track| track.clips.iter().any(|clip| clip.id == *clip_id)),
            TimelineSelection::None => None,
        }
    }

    /// Resolve the selected clip together with its track for clip inspectors.
    pub fn selected_clip_for_selection(
        &self,
        selection: &TimelineSelection,
    ) -> Option<(&TrackViewModel, &ClipViewModel)> {
        match selection {
            TimelineSelection::Clip(clip_id) => self.tracks.iter().find_map(|track| {
                track
                    .clips
                    .iter()
                    .find(|clip| clip.id == *clip_id)
                    .map(|clip| (track, clip))
            }),
            TimelineSelection::Track(track_id) => self
                .tracks
                .iter()
                .find(|track| track.id == *track_id)
                .and_then(|track| track.clips.first().map(|clip| (track, clip))),
            TimelineSelection::None => None,
        }
    }

    /// Return only clip indexes that can intersect a visible time window.
    pub fn visible_clip_indices(
        &self,
        track_index: usize,
        visible_time: TimelineRange,
    ) -> Vec<usize> {
        self.tracks
            .get(track_index)
            .map(|track| {
                track
                    .clips
                    .iter()
                    .enumerate()
                    .filter_map(|(index, clip)| {
                        clip.timeline.intersects(visible_time).then_some(index)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl TimelineViewModel {
    /// Draw the timeline and return at most one seek or semantic edit action.
    pub fn show(
        &self,
        ui: &mut egui::Ui,
        selection: &mut TimelineSelection,
        viewport: &mut TimelineViewport,
        interaction: &mut TimelineInteractionState,
        playhead_seconds: f64,
    ) -> Option<TimelineAction> {
        let playhead = self.playhead(playhead_seconds.min(self.duration.max(0.0)));
        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Timeline").color(Color32::WHITE));
            ui.separator();
            if ui.small_button("−").clicked() {
                viewport.set_zoom(viewport.zoom - 0.25);
            }
            let mut zoom = viewport.zoom;
            if ui
                .add(
                    Slider::new(&mut zoom, MIN_TIMELINE_ZOOM..=MAX_TIMELINE_ZOOM)
                        .show_value(false)
                        .text("zoom"),
                )
                .changed()
            {
                viewport.set_zoom(zoom);
            }
            if ui.small_button("+").clicked() {
                viewport.set_zoom(viewport.zoom + 0.25);
            }
            ui.label(format!("{:.0}%", viewport.zoom * 100.0));
            ui.separator();
            ui.label(format!(
                "{} @ {} fps",
                format_time(playhead.seconds),
                trim_fps(self.fps)
            ));
            let selected_clip = selection.selected_clip_id().and_then(|clip_id| {
                self.tracks.iter().find_map(|track| {
                    track
                        .clips
                        .iter()
                        .find(|clip| clip.id == clip_id)
                        .map(|clip| (track, clip))
                })
            });
            let can_split = selected_clip.is_some_and(|(track, clip)| {
                !track.locked
                    && playhead.seconds > clip.timeline.start
                    && playhead.seconds < clip.timeline.end
            });
            if ui
                .add_enabled(can_split, egui::Button::new("Split at playhead"))
                .clicked()
            {
                if let Some((_, clip)) = selected_clip {
                    action = Some(TimelineAction::SplitClip {
                        clip_id: clip.id.clone(),
                        at_frame: playhead.frame,
                    });
                }
            }
            let can_remove = selected_clip.is_some_and(|(track, _)| !track.locked);
            if ui
                .add_enabled(can_remove, egui::Button::new("Delete"))
                .clicked()
            {
                if let Some((_, clip)) = selected_clip {
                    action = Some(TimelineAction::RemoveClip {
                        clip_id: clip.id.clone(),
                    });
                }
            }
            if let Some(proposal) = &self.proposal {
                let status = if proposal.stale {
                    format!("proposal {} · stale", proposal.proposal_id)
                } else if !proposal.impacts_available {
                    format!(
                        "proposal {} · semantic impacts unavailable",
                        proposal.proposal_id
                    )
                } else {
                    format!(
                        "proposal {} · {} impact(s)",
                        proposal.proposal_id,
                        proposal.impacts.len()
                    )
                };
                ui.label(RichText::new(status).color(if proposal.stale {
                    Color32::from_rgb(255, 165, 95)
                } else {
                    Color32::from_rgb(130, 210, 160)
                }));
            }
        });
        ui.add_space(4.0);

        let geometry = TimelineGeometry::new(*viewport);
        let proposal_end = self
            .proposal
            .as_ref()
            .map(|proposal| {
                proposal
                    .ghosts()
                    .into_iter()
                    .map(|ghost| ghost.range.end)
                    .fold(0.0, f64::max)
            })
            .unwrap_or(0.0);
        let timeline_end = self
            .duration
            .max(
                self.tracks
                    .iter()
                    .flat_map(|track| track.clips.iter().map(|clip| clip.timeline.end))
                    .fold(0.0, f64::max),
            )
            .max(proposal_end)
            .max(10.0);
        let content_width = (geometry.time_to_pixel(timeline_end) + 24.0)
            .max(ui.available_width())
            .max(TIMELINE_LABEL_WIDTH + 24.0);
        let content_height = TIMELINE_RULER_HEIGHT + self.tracks.len() as f32 * TIMELINE_ROW_HEIGHT;
        let mut clicked_clip = false;
        let mut timeline_has_focus = false;
        let output = ScrollArea::both()
            .id_salt("cut-timeline-canvas")
            .animated(false)
            .scroll_offset(egui::vec2(
                viewport.horizontal_scroll.max(0.0),
                viewport.vertical_scroll.max(0.0),
            ))
            .show(ui, |canvas_ui| {
                let (content_rect, canvas_response) = canvas_ui.allocate_exact_size(
                    egui::vec2(content_width, content_height.max(TIMELINE_RULER_HEIGHT)),
                    Sense::click(),
                );
                let visible_rect = canvas_ui.clip_rect().intersect(content_rect);
                let visible_time = geometry.visible_time_range(visible_rect, content_rect);
                let visible_rows =
                    TimelineGeometry::visible_rows(visible_rect, content_rect, self.tracks.len());
                let painter = canvas_ui.painter().with_clip_rect(visible_rect);

                paint_background_and_grid(&painter, content_rect, visible_time, geometry);
                paint_ruler(&painter, content_rect, visible_time, geometry, self.fps);
                paint_track_rows(&painter, content_rect, &self.tracks, &visible_rows);
                paint_clip_bodies(
                    &painter,
                    content_rect,
                    self,
                    visible_rows.clone(),
                    visible_time,
                    geometry,
                );
                paint_media_decorations(
                    &painter,
                    content_rect,
                    self,
                    visible_rows.clone(),
                    visible_time,
                    geometry,
                );
                paint_drag_preview(
                    &painter,
                    content_rect,
                    self,
                    interaction.drag.as_ref(),
                    visible_rows.clone(),
                    visible_time,
                    geometry,
                );

                for row in visible_rows.clone() {
                    let row_top = content_rect.min.y
                        + TIMELINE_RULER_HEIGHT
                        + row as f32 * TIMELINE_ROW_HEIGHT;
                    let row_rect = Rect::from_min_max(
                        egui::pos2(content_rect.min.x, row_top),
                        egui::pos2(content_rect.max.x, row_top + TIMELINE_ROW_HEIGHT),
                    );
                    let track = &self.tracks[row];
                    let response = canvas_ui.interact(
                        row_rect,
                        canvas_ui.make_persistent_id(("timeline-track", &track.id)),
                        Sense::click(),
                    );
                    response.widget_info(|| {
                        WidgetInfo::labeled(
                            WidgetType::SelectableLabel,
                            true,
                            format!(
                                "Track {}. {}. {} clip(s). {}",
                                track.name,
                                track.kind,
                                track.clip_count,
                                track_state_label(track)
                            ),
                        )
                    });
                    if response.has_focus() {
                        timeline_has_focus = true;
                    }
                    if response.clicked() {
                        response.request_focus();
                        *selection = TimelineSelection::Track(track.id.clone());
                    }

                    for clip_index in self.visible_clip_indices(row, visible_time) {
                        let clip = &track.clips[clip_index];
                        let clip_rect = geometry.clip_rect(content_rect, row, clip.timeline);
                        let body_response = canvas_ui.interact(
                            clip_rect,
                            canvas_ui.make_persistent_id(("timeline-clip", &clip.id)),
                            Sense::click_and_drag(),
                        );
                        body_response.widget_info(|| {
                            WidgetInfo::labeled(
                                WidgetType::SelectableLabel,
                                true,
                                clip_accessibility_label(track, clip, self.frame_grid()),
                            )
                        });
                        let leading_response = canvas_ui.interact(
                            trim_handle_rect(clip_rect, TrimEdge::Leading),
                            canvas_ui.make_persistent_id(("timeline-trim-leading", &clip.id)),
                            Sense::click_and_drag(),
                        );
                        leading_response.widget_info(|| {
                            WidgetInfo::labeled(
                                WidgetType::Other,
                                true,
                                format!("Leading trim handle for clip {}", clip.id),
                            )
                        });
                        let trailing_response = canvas_ui.interact(
                            trim_handle_rect(clip_rect, TrimEdge::Trailing),
                            canvas_ui.make_persistent_id(("timeline-trim-trailing", &clip.id)),
                            Sense::click_and_drag(),
                        );
                        trailing_response.widget_info(|| {
                            WidgetInfo::labeled(
                                WidgetType::Other,
                                true,
                                format!("Trailing trim handle for clip {}", clip.id),
                            )
                        });
                        if body_response.has_focus()
                            || leading_response.has_focus()
                            || trailing_response.has_focus()
                        {
                            timeline_has_focus = true;
                        }
                        if body_response.clicked()
                            || leading_response.clicked()
                            || trailing_response.clicked()
                        {
                            body_response.request_focus();
                            *selection = TimelineSelection::Clip(clip.id.clone());
                            clicked_clip = true;
                        }

                        let grid = self.frame_grid();
                        let pointer_frame_for = |response: &egui::Response| {
                            response
                                .interact_pointer_pos()
                                .or_else(|| canvas_ui.input(|input| input.pointer.hover_pos()))
                                .map(|pointer| pointer_frame(pointer, content_rect, geometry, grid))
                                .unwrap_or_else(|| grid.frame_at(clip.timeline.start))
                        };
                        if body_response.drag_started() && !track.locked {
                            start_drag(
                                interaction,
                                clip,
                                track,
                                TimelineDragKind::Move,
                                pointer_frame_for(&body_response),
                                grid,
                            );
                        }
                        if leading_response.drag_started() && !track.locked {
                            start_drag(
                                interaction,
                                clip,
                                track,
                                TimelineDragKind::TrimLeading,
                                pointer_frame_for(&leading_response),
                                grid,
                            );
                        }
                        if trailing_response.drag_started() && !track.locked {
                            start_drag(
                                interaction,
                                clip,
                                track,
                                TimelineDragKind::TrimTrailing,
                                pointer_frame_for(&trailing_response),
                                grid,
                            );
                        }

                        let active_kind = interaction
                            .drag
                            .as_ref()
                            .and_then(|drag| (drag.clip_id == clip.id).then_some(drag.kind));
                        if let Some(kind) = active_kind {
                            let active_response = match kind {
                                TimelineDragKind::Move => &body_response,
                                TimelineDragKind::TrimLeading => &leading_response,
                                TimelineDragKind::TrimTrailing => &trailing_response,
                            };
                            if active_response.dragged() || active_response.drag_stopped() {
                                let pointer = canvas_ui
                                    .input(|input| input.pointer.hover_pos())
                                    .or_else(|| active_response.interact_pointer_pos());
                                update_drag_preview(
                                    interaction,
                                    pointer,
                                    content_rect,
                                    geometry,
                                    self,
                                    playhead.frame,
                                    grid,
                                );
                            }
                            if active_response.drag_stopped() {
                                if let Some(drag) = interaction.drag.take() {
                                    action = finish_drag(drag);
                                }
                            }
                        }
                    }
                }

                if canvas_response.has_focus() {
                    timeline_has_focus = true;
                }
                let proposal_label = self
                    .proposal
                    .as_ref()
                    .map(|proposal| {
                        if proposal.stale {
                            "stale proposal"
                        } else if proposal.impacts_available {
                            "proposal ghosts"
                        } else {
                            "proposal impacts unavailable"
                        }
                    })
                    .unwrap_or("no proposal");
                canvas_response.widget_info(|| {
                    WidgetInfo::labeled(
                        WidgetType::Other,
                        true,
                        format!(
                            "Timeline. Playhead at {}. {} track(s). {}.",
                            format_time(playhead.seconds),
                            self.tracks.len(),
                            proposal_label
                        ),
                    )
                });
                if canvas_response.clicked() {
                    canvas_response.request_focus();
                }

                paint_selection(
                    &painter,
                    content_rect,
                    self,
                    selection,
                    visible_rows.clone(),
                    visible_time,
                    geometry,
                );
                paint_proposal_overlay(
                    &painter,
                    content_rect,
                    self,
                    visible_rows,
                    visible_time,
                    geometry,
                );
                paint_playhead(&painter, content_rect, playhead, geometry);

                let playhead_x = content_rect.min.x + geometry.time_to_pixel(playhead.seconds);
                let playhead_response = canvas_ui.interact(
                    Rect::from_min_max(
                        egui::pos2(playhead_x - 3.0, content_rect.min.y),
                        egui::pos2(playhead_x + 3.0, content_rect.max.y),
                    ),
                    canvas_ui.make_persistent_id("timeline-playhead"),
                    Sense::focusable_noninteractive(),
                );
                playhead_response.widget_info(|| {
                    WidgetInfo::labeled(
                        WidgetType::Other,
                        true,
                        format!(
                            "Playhead at {} (frame {})",
                            format_time(playhead.seconds),
                            playhead.frame
                        ),
                    )
                });
                if playhead_response.has_focus() {
                    timeline_has_focus = true;
                }

                if timeline_has_focus {
                    if let Some(keyboard_action) =
                        handle_keyboard_selection(canvas_ui, self, selection, playhead.frame)
                    {
                        action = Some(keyboard_action);
                    }
                }
                if canvas_ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    interaction.drag = None;
                }

                if canvas_response.clicked() && !clicked_clip {
                    if let Some(pointer) = canvas_response.interact_pointer_pos() {
                        let local_x = pointer.x - content_rect.min.x;
                        let local_y = pointer.y - content_rect.min.y;
                        if local_x >= geometry.label_width || local_y < TIMELINE_RULER_HEIGHT {
                            let requested = geometry.pixel_to_time(local_x);
                            action = Some(TimelineAction::Seek(
                                self.frame_grid()
                                    .quantize(requested.min(self.duration.max(0.0))),
                            ));
                        }
                    }
                }
            });
        viewport.horizontal_scroll = output.state.offset.x.max(0.0);
        viewport.vertical_scroll = output.state.offset.y.max(0.0);
        if interaction.drag.is_some() {
            ui.ctx().request_repaint();
        }
        action
    }
}

fn paint_selection(
    painter: &egui::Painter,
    content_rect: Rect,
    model: &TimelineViewModel,
    selection: &TimelineSelection,
    visible_rows: Range<usize>,
    visible_time: TimelineRange,
    geometry: TimelineGeometry,
) {
    let Some(selected_clip_id) = selection.selected_clip_id() else {
        if let TimelineSelection::Track(track_id) = selection {
            if let Some(track) = model.tracks.iter().find(|track| track.id == *track_id) {
                let row = track.index;
                if visible_rows.contains(&row) {
                    let top = content_rect.min.y
                        + TIMELINE_RULER_HEIGHT
                        + row as f32 * TIMELINE_ROW_HEIGHT;
                    painter.rect_stroke(
                        Rect::from_min_max(
                            egui::pos2(content_rect.min.x, top),
                            egui::pos2(content_rect.max.x, top + TIMELINE_ROW_HEIGHT),
                        ),
                        0.0,
                        Stroke::new(1.5, Color32::WHITE),
                        StrokeKind::Inside,
                    );
                }
            }
        }
        return;
    };
    for row in visible_rows {
        let Some(track) = model.tracks.get(row) else {
            continue;
        };
        for clip_index in model.visible_clip_indices(row, visible_time) {
            let clip = &track.clips[clip_index];
            if clip.id == selected_clip_id {
                painter.rect_stroke(
                    geometry.clip_rect(content_rect, row, clip.timeline),
                    3.0,
                    Stroke::new(2.0, Color32::WHITE),
                    StrokeKind::Outside,
                );
            }
        }
    }
}

fn paint_proposal_overlay(
    painter: &egui::Painter,
    content_rect: Rect,
    model: &TimelineViewModel,
    visible_rows: Range<usize>,
    visible_time: TimelineRange,
    geometry: TimelineGeometry,
) {
    let Some(proposal) = &model.proposal else {
        return;
    };
    if proposal.stale || !proposal.impacts_available {
        return;
    }
    for ghost in proposal.ghosts() {
        let Some(row) = model
            .tracks
            .iter()
            .position(|track| track.id == ghost.track_id)
        else {
            continue;
        };
        if !visible_rows.contains(&row) || !ghost.range.intersects(visible_time) {
            continue;
        }
        let rect = geometry.clip_rect(content_rect, row, ghost.range);
        match ghost.kind {
            ProposalGhostKind::Added | ProposalGhostKind::MovedDestination => {
                painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(87, 205, 135, 70));
                painter.rect_stroke(
                    rect,
                    3.0,
                    Stroke::new(1.5, Color32::from_rgb(105, 230, 150)),
                    StrokeKind::Inside,
                );
            }
            ProposalGhostKind::Removed => {
                painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(230, 85, 85, 82));
                painter.line_segment(
                    [rect.left_bottom(), rect.right_top()],
                    Stroke::new(2.0, Color32::from_rgb(255, 130, 130)),
                );
            }
            ProposalGhostKind::MovedOriginal | ProposalGhostKind::TrimmedOriginal => {
                painter.rect_stroke(
                    rect,
                    3.0,
                    Stroke::new(1.5, Color32::from_rgb(255, 172, 84)),
                    StrokeKind::Inside,
                );
            }
            ProposalGhostKind::TrimmedProposed => {
                painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(255, 194, 95, 48));
                painter.rect_stroke(
                    rect,
                    3.0,
                    Stroke::new(1.5, Color32::from_rgb(255, 210, 125)),
                    StrokeKind::Inside,
                );
            }
        }
        if rect.width() > 64.0 {
            painter.text(
                rect.right_top() + egui::vec2(-6.0, 5.0),
                egui::Align2::RIGHT_TOP,
                match ghost.kind {
                    ProposalGhostKind::Added => "ADDED",
                    ProposalGhostKind::Removed => "REMOVED",
                    ProposalGhostKind::MovedOriginal => "FROM",
                    ProposalGhostKind::MovedDestination => "TO",
                    ProposalGhostKind::TrimmedOriginal => "ORIGINAL",
                    ProposalGhostKind::TrimmedProposed => "TRIMMED",
                },
                FontId::proportional(8.0),
                Color32::WHITE,
            );
        }
    }
    for impact in &proposal.impacts {
        let ProposalImpact::ClipPropertyChanged {
            clip_id, property, ..
        } = impact
        else {
            continue;
        };
        let Some((row, clip)) = model.tracks.iter().enumerate().find_map(|(row, track)| {
            track
                .clips
                .iter()
                .find(|clip| clip.id == *clip_id)
                .map(|clip| (row, clip))
        }) else {
            continue;
        };
        if !visible_rows.contains(&row) || !clip.timeline.intersects(visible_time) {
            continue;
        }
        let rect = geometry.clip_rect(content_rect, row, clip.timeline);
        painter.text(
            rect.right_top() + egui::vec2(-6.0, rect.height() * 0.5),
            egui::Align2::RIGHT_CENTER,
            format!("Δ {property}"),
            FontId::proportional(9.0),
            Color32::from_rgb(255, 240, 170),
        );
    }
}

fn paint_playhead(
    painter: &egui::Painter,
    content_rect: Rect,
    playhead: TimelinePlayhead,
    geometry: TimelineGeometry,
) {
    let x = content_rect.min.x + geometry.time_to_pixel(playhead.seconds);
    painter.line_segment(
        [
            egui::pos2(x, content_rect.min.y),
            egui::pos2(x, content_rect.max.y),
        ],
        Stroke::new(1.5, Color32::from_rgb(255, 82, 82)),
    );
    painter.circle_filled(
        egui::pos2(x, content_rect.min.y + 4.0),
        4.0,
        Color32::from_rgb(255, 82, 82),
    );
}

fn trim_handle_rect(rect: Rect, edge: TrimEdge) -> Rect {
    let width = TRIM_HANDLE_WIDTH.min(rect.width() / 3.0);
    match edge {
        TrimEdge::Leading => Rect::from_min_max(
            rect.min,
            egui::pos2((rect.left() + width).min(rect.right()), rect.bottom()),
        ),
        TrimEdge::Trailing => Rect::from_min_max(
            egui::pos2((rect.right() - width).max(rect.left()), rect.top()),
            rect.max,
        ),
    }
}

fn pointer_frame(
    pointer: egui::Pos2,
    content_rect: Rect,
    geometry: TimelineGeometry,
    grid: FrameGrid,
) -> FrameIndex {
    grid.frame_at(geometry.pixel_to_time(pointer.x - content_rect.min.x))
}

fn start_drag(
    interaction: &mut TimelineInteractionState,
    clip: &ClipViewModel,
    track: &TrackViewModel,
    kind: TimelineDragKind,
    pointer_start_frame: FrameIndex,
    grid: FrameGrid,
) {
    if track.locked {
        return;
    }
    let original_start_frame = grid.frame_at(clip.timeline.start);
    let original_end_frame = grid.frame_at(clip.timeline.end);
    interaction.drag = Some(TimelineDragState {
        clip_id: clip.id.clone(),
        original_track_id: track.id.clone(),
        kind,
        pointer_start_frame,
        original_start_frame,
        original_end_frame,
        preview_start_frame: original_start_frame,
        preview_end_frame: original_end_frame,
        preview_track_id: track.id.clone(),
    });
}

fn update_drag_preview(
    interaction: &mut TimelineInteractionState,
    pointer: Option<egui::Pos2>,
    content_rect: Rect,
    geometry: TimelineGeometry,
    model: &TimelineViewModel,
    playhead_frame: FrameIndex,
    grid: FrameGrid,
) {
    let Some(pointer) = pointer else {
        return;
    };
    let Some(drag) = interaction.drag.as_mut() else {
        return;
    };
    let pointer_frame = pointer_frame(pointer, content_rect, geometry, grid);
    let candidates = snap_candidates(model, &drag.clip_id, playhead_frame, grid);
    match drag.kind {
        TimelineDragKind::Move => {
            let delta = pointer_frame.saturating_sub(drag.pointer_start_frame);
            let raw_start = drag.original_start_frame.saturating_add(delta).max(0);
            let duration = drag
                .original_end_frame
                .saturating_sub(drag.original_start_frame)
                .max(1);
            drag.preview_start_frame = snap_move_start(
                raw_start,
                duration,
                &candidates,
                grid,
                geometry.pixels_per_second,
            );
            drag.preview_end_frame = drag.preview_start_frame.saturating_add(duration);
            if let Some(track_id) = track_id_at_pointer(model, content_rect, pointer) {
                drag.preview_track_id = track_id;
            } else {
                drag.preview_track_id = drag.original_track_id.clone();
            }
        }
        TimelineDragKind::TrimLeading => {
            let snapped =
                snap_edge_frame(pointer_frame, &candidates, grid, geometry.pixels_per_second);
            let minimum = drag.original_start_frame.saturating_add(1);
            let maximum = drag.original_end_frame.saturating_sub(1);
            drag.preview_start_frame = if minimum <= maximum {
                snapped.clamp(minimum, maximum)
            } else {
                drag.original_start_frame
            };
            drag.preview_end_frame = drag.original_end_frame;
        }
        TimelineDragKind::TrimTrailing => {
            let snapped =
                snap_edge_frame(pointer_frame, &candidates, grid, geometry.pixels_per_second);
            let minimum = drag.original_start_frame.saturating_add(1);
            let maximum = drag.original_end_frame;
            drag.preview_start_frame = drag.original_start_frame;
            drag.preview_end_frame = if minimum <= maximum {
                snapped.clamp(minimum, maximum)
            } else {
                drag.original_end_frame
            };
        }
    }
}

fn track_id_at_pointer(
    model: &TimelineViewModel,
    content_rect: Rect,
    pointer: egui::Pos2,
) -> Option<String> {
    let y = pointer.y - content_rect.min.y - TIMELINE_RULER_HEIGHT;
    if y < 0.0 {
        return None;
    }
    let row = (y / TIMELINE_ROW_HEIGHT).floor() as usize;
    model
        .tracks
        .get(row)
        .filter(|track| !track.locked)
        .map(|track| track.id.clone())
}

fn snap_candidates(
    model: &TimelineViewModel,
    active_clip_id: &str,
    playhead_frame: FrameIndex,
    grid: FrameGrid,
) -> Vec<FrameIndex> {
    let mut candidates = vec![0, playhead_frame.max(0)];
    for track in &model.tracks {
        for clip in &track.clips {
            if clip.id == active_clip_id {
                continue;
            }
            candidates.push(grid.frame_at(clip.timeline.start));
            candidates.push(grid.frame_at(clip.timeline.end));
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

fn snap_edge_frame(
    frame: FrameIndex,
    candidates: &[FrameIndex],
    grid: FrameGrid,
    pixels_per_second: f32,
) -> FrameIndex {
    let Some((candidate, distance)) = candidates
        .iter()
        .copied()
        .map(|candidate| {
            (
                candidate,
                (grid.seconds_at(candidate) - grid.seconds_at(frame)).abs()
                    * pixels_per_second.max(0.0) as f64,
            )
        })
        .min_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    else {
        return frame.max(0);
    };
    if distance <= SNAP_DISTANCE_PIXELS as f64 {
        candidate
    } else {
        frame.max(0)
    }
}

fn snap_move_start(
    raw_start: FrameIndex,
    duration: FrameIndex,
    candidates: &[FrameIndex],
    grid: FrameGrid,
    pixels_per_second: f32,
) -> FrameIndex {
    let raw_end = raw_start.saturating_add(duration);
    let mut best: Option<(FrameIndex, f64)> = None;
    for candidate in candidates {
        for edge in [raw_start, raw_end] {
            let distance = (grid.seconds_at(*candidate) - grid.seconds_at(edge)).abs()
                * pixels_per_second.max(0.0) as f64;
            if distance > SNAP_DISTANCE_PIXELS as f64 {
                continue;
            }
            let delta = candidate.saturating_sub(edge);
            let adjusted = raw_start.saturating_add(delta).max(0);
            if best.is_none_or(|(_, best_distance)| distance < best_distance) {
                best = Some((adjusted, distance));
            }
        }
    }
    best.map_or(raw_start.max(0), |(frame, _)| frame)
}

fn finish_drag(drag: TimelineDragState) -> Option<TimelineAction> {
    match drag.kind {
        TimelineDragKind::Move
            if drag.preview_start_frame == drag.original_start_frame
                && drag.preview_track_id == drag.original_track_id =>
        {
            None
        }
        TimelineDragKind::Move => Some(TimelineAction::PlaceClip {
            clip_id: drag.clip_id,
            timeline_frame: drag.preview_start_frame,
            track_id: (drag.preview_track_id != drag.original_track_id)
                .then_some(drag.preview_track_id),
        }),
        TimelineDragKind::TrimLeading if drag.preview_start_frame == drag.original_start_frame => {
            None
        }
        TimelineDragKind::TrimLeading => Some(TimelineAction::TrimClip {
            clip_id: drag.clip_id,
            edge: TrimEdge::Leading,
            timeline_frame: drag.preview_start_frame,
        }),
        TimelineDragKind::TrimTrailing if drag.preview_end_frame == drag.original_end_frame => None,
        TimelineDragKind::TrimTrailing => Some(TimelineAction::TrimClip {
            clip_id: drag.clip_id,
            edge: TrimEdge::Trailing,
            timeline_frame: drag.preview_end_frame,
        }),
    }
}

fn paint_drag_preview(
    painter: &egui::Painter,
    content_rect: Rect,
    model: &TimelineViewModel,
    drag: Option<&TimelineDragState>,
    visible_rows: Range<usize>,
    visible_time: TimelineRange,
    geometry: TimelineGeometry,
) {
    let Some(drag) = drag else {
        return;
    };
    let Some((row, clip)) = model
        .tracks
        .iter()
        .find_map(|track| {
            track
                .clips
                .iter()
                .find(|clip| clip.id == drag.clip_id)
                .map(|clip| (track, clip))
        })
        .map(|(source_track, clip)| {
            let target_track = model
                .tracks
                .iter()
                .find(|track| track.id == drag.preview_track_id)
                .unwrap_or(source_track);
            (target_track.index, clip)
        })
    else {
        return;
    };
    let grid = model.frame_grid();
    let Some(range) = TimelineRange::new(
        grid.seconds_at(drag.preview_start_frame),
        grid.seconds_at(drag.preview_end_frame),
    ) else {
        return;
    };
    if !visible_rows.contains(&row) || !range.intersects(visible_time) {
        return;
    }
    let rect = geometry.clip_rect(content_rect, row, range);
    painter.rect_filled(
        rect,
        3.0,
        clip.media_type
            .color()
            .gamma_multiply(0.7)
            .linear_multiply(0.72),
    );
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(2.0, Color32::WHITE),
        StrokeKind::Inside,
    );
    if rect.width() > 42.0 {
        let label = match drag.kind {
            TimelineDragKind::Move => "MOVE",
            TimelineDragKind::TrimLeading => "TRIM IN",
            TimelineDragKind::TrimTrailing => "TRIM OUT",
        };
        painter.text(
            rect.left_top() + egui::vec2(6.0, 6.0),
            egui::Align2::LEFT_TOP,
            label,
            FontId::proportional(9.0),
            Color32::WHITE,
        );
    }
}

fn handle_keyboard_selection(
    ui: &egui::Ui,
    model: &TimelineViewModel,
    selection: &mut TimelineSelection,
    playhead_frame: FrameIndex,
) -> Option<TimelineAction> {
    let (left, right, up, down, home, end, split, delete) = ui.input(|input| {
        (
            input.key_pressed(egui::Key::ArrowLeft),
            input.key_pressed(egui::Key::ArrowRight),
            input.key_pressed(egui::Key::ArrowUp),
            input.key_pressed(egui::Key::ArrowDown),
            input.key_pressed(egui::Key::Home),
            input.key_pressed(egui::Key::End),
            input.key_pressed(egui::Key::S),
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace),
        )
    });
    let clips = model
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .collect::<Vec<_>>();
    if clips.is_empty() {
        return None;
    }
    if home {
        *selection = TimelineSelection::Clip(clips[0].id.clone());
        return None;
    }
    if end {
        *selection = TimelineSelection::Clip(clips[clips.len() - 1].id.clone());
        return None;
    }

    if split {
        if let Some((track, clip)) = selected_clip(model, selection) {
            let grid = model.frame_grid();
            let start = grid.frame_at(clip.timeline.start);
            let end = grid.frame_at(clip.timeline.end);
            if !track.locked && playhead_frame > start && playhead_frame < end {
                return Some(TimelineAction::SplitClip {
                    clip_id: clip.id.clone(),
                    at_frame: playhead_frame,
                });
            }
        }
    }

    if delete {
        if let Some((track, clip)) = selected_clip(model, selection) {
            if !track.locked {
                return Some(TimelineAction::RemoveClip {
                    clip_id: clip.id.clone(),
                });
            }
        }
    }

    if left || right {
        if let Some((track, clip)) = selected_clip(model, selection) {
            if !track.locked {
                let grid = model.frame_grid();
                let current = grid.frame_at(clip.timeline.start);
                let next = if left {
                    current.saturating_sub(1)
                } else {
                    current.saturating_add(1)
                };
                if next != current {
                    return Some(TimelineAction::PlaceClip {
                        clip_id: clip.id.clone(),
                        timeline_frame: next,
                        track_id: None,
                    });
                }
            }
            return None;
        }
    }

    if !(up || down) {
        return None;
    }
    let current = selection
        .selected_clip_id()
        .and_then(|id| clips.iter().position(|clip| clip.id == id));
    let next = match (current, down) {
        (Some(index), true) => (index + 1).min(clips.len() - 1),
        (Some(index), false) => index.saturating_sub(1),
        (None, true) => 0,
        (None, false) => clips.len() - 1,
    };
    *selection = TimelineSelection::Clip(clips[next].id.clone());
    None
}

fn selected_clip<'a>(
    model: &'a TimelineViewModel,
    selection: &TimelineSelection,
) -> Option<(&'a TrackViewModel, &'a ClipViewModel)> {
    let clip_id = selection.selected_clip_id()?;
    model.tracks.iter().find_map(|track| {
        track
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .map(|clip| (track, clip))
    })
}

fn clip_accessibility_label(
    track: &TrackViewModel,
    clip: &ClipViewModel,
    grid: FrameGrid,
) -> String {
    let start_frame = grid.frame_at(clip.timeline.start);
    let end_frame = grid.frame_at(clip.timeline.end);
    format!(
        "Clip {} on track {}. {} to {}. Frames {} to {}. Source {}. {}.",
        clip.id,
        track.name,
        format_time(clip.timeline.start),
        format_time(clip.timeline.end),
        start_frame,
        end_frame,
        clip.source_label,
        clip.media_type.as_str()
    )
}

fn track_state_label(track: &TrackViewModel) -> &'static str {
    if !track.enabled {
        "disabled"
    } else if track.muted {
        "muted"
    } else if track.locked {
        "locked"
    } else {
        "active"
    }
}

fn proposal_range(range: &ProposalTimeRange) -> TimelineRange {
    let start = if range.start.is_finite() {
        range.start.max(0.0)
    } else {
        0.0
    };
    let end = if range.end.is_finite() {
        range.end
    } else {
        start + 0.001
    };
    TimelineRange::new(start, end).unwrap_or(TimelineRange {
        start,
        end: start + 0.001,
    })
}

fn ruler_step(pixels_per_second: f32) -> f64 {
    [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0]
        .into_iter()
        .find(|step| *step as f32 * pixels_per_second >= 72.0)
        .unwrap_or(300.0)
}

fn trim_fps(fps: f64) -> String {
    if (fps - fps.round()).abs() < 0.000_001 {
        format!("{}", fps.round() as i64)
    } else {
        format!("{fps:.2}")
    }
}

fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let minutes = (seconds / 60.0).floor() as u64;
    let whole_seconds = seconds.floor() as u64 % 60;
    let tenths = ((seconds.fract() * 10.0).round() as u64).min(9);
    format!("{minutes:02}:{whole_seconds:02}.{tenths}")
}

fn proposal_from_snapshot(snapshot: &StateSnapshot) -> Option<ProposalViewModel> {
    let PendingProposal::Pending {
        proposal_id,
        base_revision,
        diff_truncated,
        freshness,
        impacts,
        ..
    } = snapshot.pending_proposal.as_ref()?
    else {
        return None;
    };
    Some(ProposalViewModel {
        proposal_id: proposal_id.clone(),
        base_revision: *base_revision,
        stale: *freshness == ProposalFreshness::Stale,
        diff_truncated: *diff_truncated,
        impacts_available: impacts.is_some(),
        impacts: impacts.clone().unwrap_or_default(),
    })
}

fn source_from_value(value: &Value) -> Option<SourceViewModel> {
    let id = value.get("id")?.as_str()?.to_owned();
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("unknown source")
        .to_owned();
    let label = path
        .rsplit(['/', '\\'])
        .next()
        .filter(|label| !label.is_empty())
        .unwrap_or(&path)
        .to_owned();
    let media_type = source_media_type(value, &path);
    let thumbnail_path = source_path_value(value, &["thumbnailPath", "thumbnail"]);
    let waveform_path = source_path_value(value, &["waveformPath", "waveform"]);
    let waveform_available = value
        .get("waveformAvailable")
        .and_then(Value::as_bool)
        .or_else(|| waveform_path.as_ref().map(|_| true));
    let identity = value
        .get("fingerprint")
        .or_else(|| value.get("sha256"))
        .or_else(|| value.get("checksum"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{}:{}:{}:{}",
                path,
                value
                    .get("duration")
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
                value
                    .get("width")
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
                value
                    .get("height")
                    .and_then(Value::as_f64)
                    .unwrap_or_default()
            )
        });
    Some(SourceViewModel {
        id: id.clone(),
        label,
        path,
        media_type,
        duration: value
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|duration| duration.is_finite() && *duration >= 0.0),
        width: dimension_value(value.get("width")),
        height: dimension_value(value.get("height")),
        has_audio: value.get("hasAudio").and_then(Value::as_bool),
        thumbnail_path,
        waveform_path,
        waveform_available,
        thumbnail_cache_key: format!("derived/thumbnail/{id}/{identity}"),
        waveform_cache_key: format!("derived/waveform/{id}/{identity}"),
    })
}

fn source_path_value(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .filter(|path| !path.trim().is_empty())
        .map(str::to_owned)
}

fn dimension_value(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .and_then(|value| u32::try_from(value.round() as u64).ok())
        .filter(|value| *value > 0)
}

fn track_from_value(
    index: usize,
    value: &Value,
    source_by_id: &BTreeMap<&str, &SourceViewModel>,
    fps: f64,
) -> Option<TrackViewModel> {
    let id = value.get("id")?.as_str()?.to_owned();
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("track")
        .to_owned();
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&kind)
        .to_owned();
    let clips = value
        .get("clips")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|clip| clip_from_value(clip, &id, &kind, source_by_id, fps))
        .collect::<Vec<_>>();
    let first_clip = clips.first();
    Some(TrackViewModel {
        id,
        index,
        name,
        kind,
        clip_count: clips.len(),
        first_clip_id: first_clip.map(|clip| clip.id.clone()),
        first_clip_volume: first_clip.map_or(1.0, |clip| clip.volume),
        enabled: value
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        muted: value.get("muted").and_then(Value::as_bool).unwrap_or(false),
        locked: value
            .get("locked")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        clips,
    })
}

fn clip_from_value(
    value: &Value,
    _track_id: &str,
    track_kind: &str,
    source_by_id: &BTreeMap<&str, &SourceViewModel>,
    fps: f64,
) -> Option<ClipViewModel> {
    let id = value.get("id")?.as_str()?.to_owned();
    let timeline_start = value
        .get("timelineStart")
        .or_else(|| value.get("start"))
        .and_then(Value::as_f64)?;
    let speed = value
        .get("speed")
        .and_then(Value::as_f64)
        .filter(|speed| speed.is_finite() && *speed > 0.0)
        .unwrap_or(1.0);
    let source_in = value
        .get("in")
        .or_else(|| value.get("sourceIn"))
        .and_then(Value::as_f64);
    let source_out = value
        .get("out")
        .or_else(|| value.get("sourceOut"))
        .and_then(Value::as_f64);
    let source_id = value
        .get("sourceId")
        .or_else(|| value.get("src"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let source_model = source_id
        .as_deref()
        .and_then(|source_id| source_by_id.get(source_id).copied());
    let source_duration = source_model.and_then(|source| source.duration);
    let source = match (source_in, source_out) {
        (Some(start), Some(end)) => TimelineRange::new(start, end),
        (None, None) => source_duration.and_then(|duration| TimelineRange::new(0.0, duration)),
        _ => None,
    };
    let duration = value
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .or_else(|| source.map(|source| source.duration() / speed))?;
    let timeline = TimelineRange::new(timeline_start, timeline_start + duration)?;
    let media_type = source_model
        .map(|source| source.media_type)
        .filter(|media_type| *media_type != MediaType::Unknown)
        .unwrap_or_else(|| media_type_for_track(track_kind, value));
    let source_label = source_model
        .map(|source| source.label.clone())
        .or_else(|| value.get("text").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| "Generated clip".into());
    let frame_grid = FrameGrid::new(fps).expect("timeline projection has a positive fps");
    let frame = frame_grid.frame_at(timeline.start);
    let thumbnail_cache_key =
        source_model.map(|source| format!("{}:frame:{frame}", source.thumbnail_cache_key));
    let cache_start = source_in.unwrap_or(0.0);
    let cache_end = source_out
        .or_else(|| source_model.and_then(|source| source.duration))
        .unwrap_or(0.0);
    let waveform_cache_key = source_model.map(|source| {
        format!(
            "{}:range:{cache_start:.6}-{cache_end:.6}:speed:{speed:.6}",
            source.waveform_cache_key
        )
    });
    Some(ClipViewModel {
        id,
        source_id,
        source_label,
        media_type,
        timeline,
        source,
        source_duration: source_model.and_then(|source| source.duration),
        speed,
        volume: value
            .get("volume")
            .and_then(Value::as_f64)
            .filter(|volume| volume.is_finite())
            .unwrap_or(1.0)
            .clamp(0.0, 2.0),
        opacity: value
            .get("opacity")
            .and_then(Value::as_f64)
            .filter(|opacity| opacity.is_finite()),
        crop: value.get("crop").and_then(deserialize_value),
        transform: value.get("transform").and_then(deserialize_value),
        effects: value
            .get("effects")
            .and_then(Value::as_array)
            .map(|effects| effects.iter().filter_map(deserialize_value).collect())
            .unwrap_or_default(),
        keyframes: value
            .get("keyframes")
            .and_then(Value::as_array)
            .map(|keyframes| keyframes.iter().filter_map(deserialize_value).collect())
            .unwrap_or_default(),
        transitions: transitions_from_value(value),
        thumbnail_cache_key,
        waveform_cache_key,
    })
}

fn transitions_from_value(value: &Value) -> Vec<Transition> {
    if let Some(transitions) = value.get("transitions").and_then(Value::as_array) {
        return transitions.iter().filter_map(deserialize_value).collect();
    }
    value
        .get("transition")
        .and_then(deserialize_value)
        .into_iter()
        .collect()
}

fn deserialize_value<T: DeserializeOwned>(value: &Value) -> Option<T> {
    serde_json::from_value(value.clone()).ok()
}

fn source_media_type(value: &Value, path: &str) -> MediaType {
    let explicit = value
        .get("mediaType")
        .or_else(|| value.get("media_type"))
        .or_else(|| value.get("mimeType"))
        .or_else(|| value.get("mime_type"))
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("metadata")
                .and_then(Value::as_object)
                .and_then(|metadata| {
                    metadata
                        .get("mediaType")
                        .or_else(|| metadata.get("mimeType"))
                        .and_then(Value::as_str)
                })
        });
    if let Some(explicit) = explicit {
        let explicit = explicit.to_ascii_lowercase();
        if explicit.starts_with("video") {
            return MediaType::Video;
        }
        if explicit.starts_with("audio") {
            return MediaType::Audio;
        }
        if explicit.starts_with("image") {
            return MediaType::Image;
        }
        if explicit.starts_with("text") {
            return MediaType::Text;
        }
    }
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" => MediaType::Video,
        "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" => MediaType::Audio,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => MediaType::Image,
        _ => MediaType::Unknown,
    }
}

fn media_type_for_track(track_kind: &str, clip: &Value) -> MediaType {
    match track_kind.to_ascii_lowercase().as_str() {
        "video" => MediaType::Video,
        "audio" => MediaType::Audio,
        "title" => MediaType::Text,
        _ if clip.get("text").and_then(Value::as_str).is_some() => MediaType::Text,
        _ => MediaType::Unknown,
    }
}

fn project_duration(project: &Value) -> f64 {
    let declared = project
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|duration| duration.is_finite() && *duration >= 0.0)
        .unwrap_or(0.0);
    project
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|track| track.get("clips").and_then(Value::as_array))
        .flatten()
        .filter_map(|clip| {
            let start = clip
                .get("timelineStart")
                .or_else(|| clip.get("start"))
                .and_then(Value::as_f64)?;
            let duration = clip
                .get("duration")
                .and_then(Value::as_f64)
                .filter(|duration| duration.is_finite() && *duration > 0.0)
                .or_else(|| {
                    let speed = clip
                        .get("speed")
                        .and_then(Value::as_f64)
                        .filter(|speed| speed.is_finite() && *speed > 0.0)
                        .unwrap_or(1.0);
                    Some((clip.get("out")?.as_f64()? - clip.get("in")?.as_f64()?) / speed)
                })
                .filter(|duration| duration.is_finite() && *duration > 0.0)?;
            Some(start + duration)
        })
        .fold(declared, f64::max)
}

fn paint_background_and_grid(
    painter: &egui::Painter,
    content_rect: Rect,
    visible_time: TimelineRange,
    geometry: TimelineGeometry,
) {
    painter.rect_filled(content_rect, 0.0, Color32::from_rgb(8, 8, 8));
    let step = ruler_step(geometry.pixels_per_second);
    let first = (visible_time.start / step).floor().max(0.0) as i64;
    let last = (visible_time.end / step).ceil().max(0.0) as i64;
    for tick in first..=last {
        let seconds = tick as f64 * step;
        let x = content_rect.min.x + geometry.time_to_pixel(seconds);
        painter.line_segment(
            [
                egui::pos2(x, content_rect.min.y + TIMELINE_RULER_HEIGHT),
                egui::pos2(x, content_rect.max.y),
            ],
            Stroke::new(1.0, Color32::from_gray(30)),
        );
    }
    painter.line_segment(
        [
            egui::pos2(
                content_rect.min.x,
                content_rect.min.y + TIMELINE_RULER_HEIGHT,
            ),
            egui::pos2(
                content_rect.max.x,
                content_rect.min.y + TIMELINE_RULER_HEIGHT,
            ),
        ],
        Stroke::new(1.0, Color32::from_gray(52)),
    );
}

fn paint_ruler(
    painter: &egui::Painter,
    content_rect: Rect,
    visible_time: TimelineRange,
    geometry: TimelineGeometry,
    fps: f64,
) {
    painter.rect_filled(
        Rect::from_min_max(
            content_rect.min,
            egui::pos2(
                content_rect.max.x,
                content_rect.min.y + TIMELINE_RULER_HEIGHT,
            ),
        ),
        0.0,
        Color32::from_rgb(16, 16, 16),
    );
    painter.text(
        egui::pos2(content_rect.min.x + 10.0, content_rect.min.y + 8.0),
        egui::Align2::LEFT_TOP,
        format!("TIME · {} fps", trim_fps(fps)),
        FontId::proportional(11.0),
        Color32::from_gray(145),
    );
    let step = ruler_step(geometry.pixels_per_second);
    let first = (visible_time.start / step).floor().max(0.0) as i64;
    let last = (visible_time.end / step).ceil().max(0.0) as i64;
    for tick in first..=last {
        let seconds = tick as f64 * step;
        let x = content_rect.min.x + geometry.time_to_pixel(seconds);
        painter.line_segment(
            [
                egui::pos2(x, content_rect.min.y + TIMELINE_RULER_HEIGHT - 7.0),
                egui::pos2(x, content_rect.min.y + TIMELINE_RULER_HEIGHT),
            ],
            Stroke::new(1.0, Color32::from_gray(105)),
        );
        painter.text(
            egui::pos2(x + 4.0, content_rect.min.y + 8.0),
            egui::Align2::LEFT_TOP,
            format_time(seconds),
            FontId::proportional(11.0),
            Color32::from_gray(170),
        );
    }
}

fn paint_track_rows(
    painter: &egui::Painter,
    content_rect: Rect,
    tracks: &[TrackViewModel],
    visible_rows: &Range<usize>,
) {
    for row in visible_rows.clone() {
        let top = content_rect.min.y + TIMELINE_RULER_HEIGHT + row as f32 * TIMELINE_ROW_HEIGHT;
        let row_rect = Rect::from_min_max(
            egui::pos2(content_rect.min.x, top),
            egui::pos2(content_rect.max.x, top + TIMELINE_ROW_HEIGHT),
        );
        painter.rect_filled(
            row_rect,
            0.0,
            if row % 2 == 0 {
                Color32::from_rgb(12, 12, 12)
            } else {
                Color32::from_rgb(16, 16, 16)
            },
        );
        painter.rect_filled(
            Rect::from_min_max(
                row_rect.min,
                egui::pos2(row_rect.min.x + TIMELINE_LABEL_WIDTH, row_rect.max.y),
            ),
            0.0,
            Color32::from_rgb(18, 18, 18),
        );
        if let Some(track) = tracks.get(row) {
            let state = track_state_label(track);
            painter.text(
                row_rect.min + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                &track.name,
                FontId::proportional(13.0),
                if track.enabled {
                    Color32::WHITE
                } else {
                    Color32::from_gray(105)
                },
            );
            painter.text(
                row_rect.min + egui::vec2(10.0, 30.0),
                egui::Align2::LEFT_TOP,
                format!(
                    "{} · {} · {}",
                    track.kind.to_uppercase(),
                    track.clip_count,
                    state
                ),
                FontId::proportional(10.0),
                Color32::from_gray(115),
            );
        }
        painter.line_segment(
            [row_rect.left_bottom(), row_rect.right_bottom()],
            Stroke::new(1.0, Color32::from_gray(31)),
        );
    }
}

fn paint_clip_bodies(
    painter: &egui::Painter,
    content_rect: Rect,
    model: &TimelineViewModel,
    visible_rows: Range<usize>,
    visible_time: TimelineRange,
    geometry: TimelineGeometry,
) {
    for row in visible_rows {
        let Some(track) = model.tracks.get(row) else {
            continue;
        };
        for clip_index in model.visible_clip_indices(row, visible_time) {
            let clip = &track.clips[clip_index];
            let rect = geometry.clip_rect(content_rect, row, clip.timeline);
            let base = clip.media_type.color();
            painter.rect_filled(
                rect,
                3.0,
                base.gamma_multiply(if track.enabled { 1.0 } else { 0.35 }),
            );
            painter.rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, base.gamma_multiply(0.85)),
                StrokeKind::Inside,
            );
            if rect.width() > 48.0 {
                painter.text(
                    rect.left_top() + egui::vec2(8.0, 7.0),
                    egui::Align2::LEFT_TOP,
                    &clip.source_label,
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
                painter.text(
                    rect.left_bottom() - egui::vec2(-8.0, 7.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!(
                        "{} · {}",
                        format_time(clip.timeline.start),
                        format_time(clip.timeline.end)
                    ),
                    FontId::proportional(9.0),
                    Color32::from_gray(220),
                );
            }
        }
    }
}

fn paint_media_decorations(
    painter: &egui::Painter,
    content_rect: Rect,
    model: &TimelineViewModel,
    visible_rows: Range<usize>,
    visible_time: TimelineRange,
    geometry: TimelineGeometry,
) {
    for row in visible_rows {
        let Some(track) = model.tracks.get(row) else {
            continue;
        };
        for clip_index in model.visible_clip_indices(row, visible_time) {
            let clip = &track.clips[clip_index];
            let rect = geometry.clip_rect(content_rect, row, clip.timeline);
            painter.rect_filled(
                Rect::from_min_max(
                    rect.left_top(),
                    egui::pos2((rect.left() + 5.0).min(rect.right()), rect.bottom()),
                ),
                2.0,
                Color32::from_rgba_unmultiplied(255, 255, 255, 42),
            );
            if rect.width() > 70.0 {
                painter.text(
                    rect.left_top() + egui::vec2(5.0, 5.0),
                    egui::Align2::LEFT_TOP,
                    clip.media_type.as_str().to_uppercase(),
                    FontId::proportional(8.0),
                    Color32::from_rgba_unmultiplied(255, 255, 255, 180),
                );
            }
            if clip.media_type == MediaType::Audio && rect.width() > 60.0 {
                let center = rect.center().y + 5.0;
                for index in 0..8 {
                    let x = rect.left() + 12.0 + index as f32 * 5.0;
                    let height = 3.0 + ((index * 7 + clip.id.len()) % 8) as f32;
                    painter.line_segment(
                        [
                            egui::pos2(x, center - height),
                            egui::pos2(x, center + height),
                        ],
                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 235, 180, 150)),
                    );
                }
            }
            if rect.width() >= TRIM_HANDLE_WIDTH * 2.0 {
                for edge in [TrimEdge::Leading, TrimEdge::Trailing] {
                    let handle = trim_handle_rect(rect, edge);
                    painter.line_segment(
                        [handle.center_top(), handle.center_bottom()],
                        Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 255, 255, 145)),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        finish_drag, snap_edge_frame, snap_move_start, MediaType, ProposalGhostKind,
        TimelineAction, TimelineDragKind, TimelineDragState, TimelineGeometry,
        TimelineInteractionState, TimelineRange, TimelineSelection, TimelineViewModel,
        TimelineViewport, TIMELINE_ROW_HEIGHT, TIMELINE_RULER_HEIGHT,
    };
    use cut_core::FrameGrid;
    use cut_domain::TrimEdge;
    use cut_runtime::{
        PendingProposal, ProposalFreshness, ProposalImpact, ProposalTimeRange, StateSnapshot,
    };
    use serde_json::json;
    use std::time::Instant;

    fn snapshot(
        project: serde_json::Value,
        pending_proposal: Option<PendingProposal>,
    ) -> StateSnapshot {
        StateSnapshot {
            duration: project
                .get("duration")
                .and_then(|value| value.as_f64())
                .unwrap_or(10.0),
            project,
            revision: 3,
            can_undo: false,
            can_redo: false,
            sources: Vec::new(),
            pending_proposal,
            source_version: 2,
            projection_status: "current".into(),
        }
    }

    #[test]
    fn projection_contains_typed_sources_ranges_and_cache_keys() {
        let model = TimelineViewModel::from_project(&json!({
            "version": 2,
            "fps": 29.97,
            "duration": 8.0,
            "sources": [{"id":"source-1","path":"/media/shot.mp4","duration":8.0}],
            "tracks": [{
                "id":"video-1", "type":"video", "clips":[{
                    "id":"clip-1", "sourceId":"source-1", "timelineStart":1.0,
                    "in":2.0, "out":6.0
                }]
            }]
        }));
        let clip = &model.tracks[0].clips[0];
        assert_eq!(clip.timeline.start, 1.0);
        assert_eq!(clip.timeline.end, 5.0);
        assert_eq!(clip.media_type, MediaType::Video);
        assert!(clip
            .thumbnail_cache_key
            .as_deref()
            .is_some_and(|key| key.contains("source-1")));
        assert!(clip
            .waveform_cache_key
            .as_deref()
            .is_some_and(|key| key.contains("waveform")));
        assert_eq!(model.frame_grid().frame_at(1.0), 30);
    }

    #[test]
    fn geometry_round_trips_time_to_pixel_and_culls_rows() {
        let geometry = TimelineGeometry::new(TimelineViewport::default());
        for seconds in [0.0, 0.25, 1.0, 12.5] {
            let pixel = geometry.time_to_pixel(seconds);
            assert!((geometry.pixel_to_time(pixel) - seconds).abs() < 0.000_001);
        }
        let content = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 400.0));
        let visible = egui::Rect::from_min_max(
            egui::pos2(0.0, TIMELINE_RULER_HEIGHT + TIMELINE_ROW_HEIGHT * 3.0),
            egui::pos2(640.0, TIMELINE_RULER_HEIGHT + TIMELINE_ROW_HEIGHT * 5.0),
        );
        assert_eq!(
            TimelineGeometry::visible_rows(visible, content, 1_000),
            3..6
        );
    }

    #[test]
    fn visible_clip_culling_returns_only_intersecting_clips() {
        let clips = (0..1_000)
            .map(|index| {
                json!({
                    "id": format!("clip-{index}"),
                    "timelineStart": index as f64,
                    "duration": 0.5
                })
            })
            .collect::<Vec<_>>();
        let model = TimelineViewModel::from_project(&json!({
            "version": 2,
            "fps": 30,
            "tracks": [{"id":"video-1","type":"video","clips":clips}]
        }));
        let visible = TimelineRange::new(500.0, 501.0).unwrap();
        assert_eq!(model.visible_clip_indices(0, visible), vec![500]);
    }

    #[test]
    fn proposal_ghosts_keep_move_trim_add_remove_semantics() {
        let pending = PendingProposal::Pending {
            proposal_id: "proposal-1".into(),
            base_revision: 3,
            command: json!({"type":"batch","commands":[]}),
            diff: Vec::new(),
            diff_truncated: false,
            freshness: ProposalFreshness::Current,
            impacts: Some(vec![
                ProposalImpact::ClipAdded {
                    clip_id: "added".into(),
                    track_id: "video-1".into(),
                    timeline: ProposalTimeRange {
                        start: 2.0,
                        end: 4.0,
                    },
                    source: None,
                },
                ProposalImpact::ClipMoved {
                    clip_id: "moved".into(),
                    from_track_id: "video-1".into(),
                    from: ProposalTimeRange {
                        start: 0.0,
                        end: 1.0,
                    },
                    to_track_id: "video-2".into(),
                    to: ProposalTimeRange {
                        start: 5.0,
                        end: 6.0,
                    },
                },
                ProposalImpact::ClipTrimmed {
                    clip_id: "trimmed".into(),
                    track_id: "video-1".into(),
                    before: ProposalTimeRange {
                        start: 0.0,
                        end: 3.0,
                    },
                    after: ProposalTimeRange {
                        start: 0.0,
                        end: 2.0,
                    },
                    source_before: None,
                    source_after: None,
                },
                ProposalImpact::ClipRemoved {
                    clip_id: "removed".into(),
                    track_id: "video-1".into(),
                    timeline: ProposalTimeRange {
                        start: 7.0,
                        end: 8.0,
                    },
                    source: None,
                },
            ]),
        };
        let model = TimelineViewModel::from_snapshot(&snapshot(
            json!({"version":2,"fps":30,"duration":10,"tracks":[]}),
            Some(pending),
        ));
        let ghosts = model.proposal.unwrap().ghosts();
        assert_eq!(ghosts.len(), 6);
        assert!(ghosts
            .iter()
            .any(|ghost| ghost.kind == ProposalGhostKind::MovedOriginal));
        assert!(ghosts
            .iter()
            .any(|ghost| ghost.kind == ProposalGhostKind::TrimmedProposed));
    }

    #[test]
    fn legacy_pending_proposal_is_explicitly_unavailable() {
        let pending = PendingProposal::Pending {
            proposal_id: "legacy".into(),
            base_revision: 3,
            command: json!({"type":"batch","commands":[]}),
            diff: Vec::new(),
            diff_truncated: false,
            freshness: ProposalFreshness::Current,
            impacts: None,
        };
        let model = TimelineViewModel::from_snapshot(&snapshot(
            json!({"version":2,"fps":30,"tracks":[]}),
            Some(pending),
        ));
        let proposal = model.proposal.unwrap();
        assert!(!proposal.impacts_available);
        assert!(proposal.ghosts().is_empty());
    }

    #[test]
    fn selection_resolves_the_inspector_track_for_clip_selection() {
        let model = TimelineViewModel::from_project(&json!({
            "version": 2,
            "fps": 30,
            "tracks": [{"id":"video-1","type":"video","clips":[{
                "id":"clip-1","timelineStart":0.0,"duration":1.0
            }]}]
        }));
        assert_eq!(
            model
                .selected_for_selection(&TimelineSelection::Clip("clip-1".into()))
                .map(|track| track.id.as_str()),
            Some("video-1")
        );
    }

    #[test]
    fn projection_infers_an_implicit_source_range_from_probe_duration() {
        let model = TimelineViewModel::from_project(&json!({
            "version": 2,
            "fps": 30,
            "sources": [{"id": "source-1", "path": "clip.mp4", "duration": 10.0}],
            "tracks": [{"id": "video-1", "type": "video", "clips": [{
                "id": "clip-1",
                "sourceId": "source-1",
                "timelineStart": 2.0,
                "speed": 2.0
            }]}]
        }));
        let clip = &model.tracks[0].clips[0];
        assert_eq!(clip.source, TimelineRange::new(0.0, 10.0));
        assert_eq!(clip.timeline, TimelineRange::new(2.0, 7.0).unwrap());
    }

    #[test]
    fn snapping_uses_project_frame_edges_with_a_pixel_threshold() {
        let grid = FrameGrid::new(30.0).unwrap();
        assert_eq!(snap_edge_frame(28, &[30], grid, 80.0), 30);
        assert_eq!(snap_edge_frame(40, &[30], grid, 80.0), 40);
        assert_eq!(snap_move_start(28, 60, &[90], grid, 80.0), 30);
    }

    #[test]
    fn completed_drag_becomes_one_semantic_action() {
        let moved = finish_drag(TimelineDragState {
            clip_id: "clip-1".into(),
            original_track_id: "video-1".into(),
            kind: TimelineDragKind::Move,
            pointer_start_frame: 30,
            original_start_frame: 30,
            original_end_frame: 90,
            preview_start_frame: 31,
            preview_end_frame: 91,
            preview_track_id: "video-1".into(),
        });
        assert_eq!(
            moved,
            Some(TimelineAction::PlaceClip {
                clip_id: "clip-1".into(),
                timeline_frame: 31,
                track_id: None,
            })
        );

        let trimmed = finish_drag(TimelineDragState {
            clip_id: "clip-1".into(),
            original_track_id: "video-1".into(),
            kind: TimelineDragKind::TrimTrailing,
            pointer_start_frame: 90,
            original_start_frame: 30,
            original_end_frame: 90,
            preview_start_frame: 30,
            preview_end_frame: 75,
            preview_track_id: "video-1".into(),
        });
        assert_eq!(
            trimmed,
            Some(TimelineAction::TrimClip {
                clip_id: "clip-1".into(),
                edge: TrimEdge::Trailing,
                timeline_frame: 75,
            })
        );
    }

    #[test]
    fn revision_conflict_cancels_transient_drag_preview() {
        let mut interaction = TimelineInteractionState {
            drag: Some(TimelineDragState {
                clip_id: "clip-1".into(),
                original_track_id: "video-1".into(),
                kind: TimelineDragKind::Move,
                pointer_start_frame: 30,
                original_start_frame: 30,
                original_end_frame: 90,
                preview_start_frame: 45,
                preview_end_frame: 105,
                preview_track_id: "video-2".into(),
            }),
        };

        interaction.cancel_for_revision_conflict();

        assert!(interaction.drag.is_none());
    }

    #[test]
    fn trim_hit_regions_include_clip_boundaries_without_overlap() {
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 60.0));
        let leading = super::trim_handle_rect(rect, TrimEdge::Leading);
        let trailing = super::trim_handle_rect(rect, TrimEdge::Trailing);
        assert!(leading.contains(rect.left_top()));
        assert!(trailing.contains(rect.right_bottom()));
        assert!(leading.right() < trailing.left());
        assert!(!leading.contains(rect.center()));
        assert!(!trailing.contains(rect.center()));
    }

    #[test]
    fn thousand_clip_fixture_measures_culled_paint_and_hit_work() {
        let clips = (0..1_000)
            .map(|index| {
                json!({
                    "id": format!("clip-{index}"),
                    "timelineStart": index as f64,
                    "duration": 0.75
                })
            })
            .collect::<Vec<_>>();
        let model = TimelineViewModel::from_project(&json!({
            "version": 2,
            "fps": 30,
            "tracks": [{"id": "video-1", "type": "video", "clips": clips}]
        }));
        let context = egui::Context::default();
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1_440.0, 320.0),
            )),
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("root viewport")
            .inner_rect = input.screen_rect;
        let mut selection = TimelineSelection::default();
        let mut viewport = TimelineViewport::default();
        let mut interaction = super::TimelineInteractionState::default();
        let started = Instant::now();
        let mut output = context.run_ui(input, |ui| {
            model.show(ui, &mut selection, &mut viewport, &mut interaction, 0.0);
        });
        let paint_elapsed = started.elapsed();

        let hit_started = Instant::now();
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 40.0));
        for x in 0..1_000 {
            let point = egui::pos2(x as f32 / 10.0, 20.0);
            let _ = super::trim_handle_rect(rect, TrimEdge::Leading).contains(point)
                || super::trim_handle_rect(rect, TrimEdge::Trailing).contains(point);
        }
        let hit_elapsed = hit_started.elapsed();

        assert!(!output.shapes.is_empty());
        assert!(output.shapes.len() < 500, "off-screen clips were painted");
        eprintln!(
            "1,000 clip timeline: {} paint shapes in {:?}; 1,000 boundary hits in {:?}",
            output.shapes.len(),
            paint_elapsed,
            hit_elapsed
        );
        output.textures_delta.clear();
    }
}
