use cut_core::preview::{PreviewMode, PreviewPlan, PreviewRange};
use cut_playback::{PlaybackConfig, PlaybackError, PlaybackEvent, PlaybackSession, PlaybackState};
use cut_runtime::{RuntimeError, StateSnapshot};
use egui::{Color32, RichText, Slider, TextureHandle, TextureOptions};
use serde_json::json;
use std::path::Path;
use std::time::Duration;

/// State of the path currently allowed to feed the native viewer.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewState {
    Empty,
    NativeExact,
    WaitingForRender,
    Rendering { job_id: String, progress: f64 },
    RenderedFallback,
    Failed { message: String },
    Unavailable,
}

/// Bounded preview decision shown alongside the viewer.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewStatus {
    pub plan: PreviewPlan,
    pub state: PreviewState,
}

impl PreviewStatus {
    pub fn empty() -> Self {
        let mut status = Self::native_exact();
        status.state = PreviewState::Empty;
        status
    }

    pub fn native_exact() -> Self {
        Self::from_plan(PreviewPlan {
            mode: PreviewMode::NativeExact,
            range: None,
            reasons: Vec::new(),
            reasons_truncated: false,
        })
    }

    pub fn from_plan(plan: PreviewPlan) -> Self {
        let state = match plan.mode {
            PreviewMode::NativeExact => PreviewState::NativeExact,
            PreviewMode::RenderedFallback => PreviewState::WaitingForRender,
            PreviewMode::Unavailable => PreviewState::Unavailable,
        };
        Self { plan, state }
    }

    pub fn begin_render(&mut self, job_id: String) {
        self.state = PreviewState::Rendering {
            job_id,
            progress: 0.0,
        };
    }

    pub fn set_progress(&mut self, progress: f64) {
        if let PreviewState::Rendering {
            progress: current, ..
        } = &mut self.state
        {
            *current = progress.clamp(0.0, 1.0);
        }
    }

    pub fn mark_rendered(&mut self) {
        self.state = PreviewState::RenderedFallback;
    }

    pub fn mark_failed(&mut self, message: String) {
        self.state = PreviewState::Failed { message };
    }

    pub fn job_id(&self) -> Option<&str> {
        match &self.state {
            PreviewState::Rendering { job_id, .. } => Some(job_id),
            PreviewState::Empty
            | PreviewState::NativeExact
            | PreviewState::WaitingForRender
            | PreviewState::RenderedFallback
            | PreviewState::Failed { .. }
            | PreviewState::Unavailable => None,
        }
    }

    pub fn accepts_playback(&self) -> bool {
        matches!(
            &self.state,
            PreviewState::NativeExact | PreviewState::RenderedFallback
        )
    }

    fn is_empty(&self) -> bool {
        matches!(self.state, PreviewState::Empty)
    }

    fn label(&self) -> &'static str {
        match &self.state {
            PreviewState::Empty => "No media",
            PreviewState::NativeExact => "Native exact",
            PreviewState::WaitingForRender => "Rendered fallback pending",
            PreviewState::Rendering { .. } => "Rendered fallback rendering",
            PreviewState::RenderedFallback => "Rendered fallback",
            PreviewState::Failed { .. } | PreviewState::Unavailable => "Unavailable",
        }
    }

    fn reason_summary(&self) -> String {
        let mut reasons = self
            .plan
            .reasons
            .iter()
            .map(|reason| reason.message.as_str())
            .take(3)
            .collect::<Vec<_>>();
        if self.plan.reasons_truncated {
            reasons.push("more reasons omitted");
        }
        reasons.join(" · ")
    }
}

/// View state for the native viewer. Decoded media stays in the app playback
/// adapter; this model remains cheap to rebuild after a daemon snapshot
/// changes.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewerViewModel {
    pub playhead: f64,
    pub duration: f64,
    pub playing: bool,
    pub volume: f32,
    pub last_volume: f32,
    pub muted: bool,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub preview: PreviewStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ViewerAction {
    TogglePlay,
    Seek(f64),
    SetVolume(f32),
    ToggleMute,
    ToggleFullscreen,
    CancelPreview,
    RetryPreview,
}

impl ViewerViewModel {
    pub fn new(duration: f64) -> Self {
        Self {
            playhead: 0.0,
            duration: duration.max(0.0),
            playing: false,
            volume: 1.0,
            last_volume: 1.0,
            muted: false,
            canvas_width: 1920,
            canvas_height: 1080,
            preview: PreviewStatus::native_exact(),
        }
    }

    pub fn sync_duration(&mut self, duration: f64) {
        self.duration = duration.max(0.0);
        self.playhead = self.playhead.min(self.duration);
    }

    pub fn sync_canvas(&mut self, width: u32, height: u32) {
        self.canvas_width = width.max(2);
        self.canvas_height = height.max(2);
    }

    /// Draw the viewer and return a UI command for the app playback adapter.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        texture: Option<&TextureHandle>,
        caption: Option<&str>,
    ) -> Option<ViewerAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Viewer").color(Color32::WHITE));
            ui.label(
                RichText::new(format!("Preview · {}", self.preview.label()))
                    .small()
                    .color(
                        if self.preview.accepts_playback() || self.preview.is_empty() {
                            Color32::from_gray(170)
                        } else {
                            Color32::from_rgb(228, 174, 84)
                        },
                    ),
            );
            if let Some(job_id) = self.preview.job_id() {
                let progress = match &self.preview.state {
                    PreviewState::Rendering { progress, .. } => *progress,
                    _ => 0.0,
                };
                ui.label(format!("{} {:.0}%", job_id, progress * 100.0));
                if ui.button("Cancel render").clicked() {
                    action = Some(ViewerAction::CancelPreview);
                }
            } else if self.preview.plan.needs_rendered_fallback()
                && matches!(
                    &self.preview.state,
                    PreviewState::WaitingForRender | PreviewState::Failed { .. }
                )
                && ui.button("Render preview").clicked()
            {
                action = Some(ViewerAction::RetryPreview);
            }
        });
        let reason_summary = self.preview.reason_summary();
        if !reason_summary.is_empty() {
            ui.label(
                RichText::new(reason_summary)
                    .small()
                    .color(Color32::from_gray(165)),
            );
        }
        ui.add_space(8.0);
        let available = ui.available_size();
        let aspect = self.canvas_width as f32 / self.canvas_height.max(1) as f32;
        let controls_height = 108.0;
        let max_height = (available.y - controls_height).max(180.0);
        let height = (available.x / aspect).min(max_height).max(180.0);
        let width = (height * aspect).min(available.x).max(1.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        ui.painter().rect_filled(rect, 4.0, Color32::BLACK);
        if self.preview.accepts_playback() {
            if let Some(texture) = texture {
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                self.draw_placeholder(ui, rect);
            }
        } else {
            self.draw_placeholder(ui, rect);
        }
        if self.preview.accepts_playback() {
            if let Some(caption) = caption.filter(|caption| !caption.is_empty()) {
                ui.painter().text(
                    egui::pos2(rect.center().x, rect.max.y - 26.0),
                    egui::Align2::CENTER_CENTER,
                    caption,
                    egui::FontId::proportional(18.0),
                    Color32::WHITE,
                );
            }
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.preview.accepts_playback(),
                    egui::Button::new(if self.playing { "Pause" } else { "Play" }),
                )
                .clicked()
            {
                action = Some(ViewerAction::TogglePlay);
            }
            ui.label(format_time(self.playhead));
            let mut playhead = self.playhead;
            if ui
                .add_enabled(
                    self.preview.accepts_playback(),
                    Slider::new(&mut playhead, 0.0..=self.duration.max(0.001))
                        .show_value(false)
                        .text("playhead"),
                )
                .changed()
            {
                action = Some(ViewerAction::Seek(playhead));
            }
            ui.label(format_time(self.duration));
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.preview.accepts_playback(),
                    egui::Button::new(if self.muted { "Unmute" } else { "Mute" }),
                )
                .clicked()
            {
                action = Some(ViewerAction::ToggleMute);
            }
            let mut volume = self.volume;
            if ui
                .add_enabled(
                    self.preview.accepts_playback(),
                    Slider::new(&mut volume, 0.0..=1.0).text("volume"),
                )
                .changed()
            {
                action = Some(ViewerAction::SetVolume(volume));
            }
            if ui.button("Fullscreen").clicked() {
                action = Some(ViewerAction::ToggleFullscreen);
            }
        });
        action
    }

    fn draw_placeholder(&self, ui: &egui::Ui, rect: egui::Rect) {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            if matches!(&self.preview.state, PreviewState::Empty) {
                "DROP MEDIA TO START"
            } else if matches!(
                &self.preview.state,
                PreviewState::WaitingForRender | PreviewState::Rendering { .. }
            ) {
                "RENDERING PREVIEW"
            } else if matches!(
                &self.preview.state,
                PreviewState::Unavailable | PreviewState::Failed { .. }
            ) {
                "PREVIEW UNAVAILABLE"
            } else if self.playing {
                "BUFFERING"
            } else {
                "NO MEDIA"
            },
            egui::FontId::proportional(16.0),
            Color32::from_gray(130),
        );
    }
}

/// App-owned adapter between playback events and egui's wgpu-backed texture
/// registry. The playback worker never touches an egui or winit object.
pub struct ViewerPlaybackAdapter {
    session: PlaybackSession,
    texture: Option<TextureHandle>,
    caption: Option<String>,
    rendered_range: Option<PreviewRange>,
}

impl ViewerPlaybackAdapter {
    pub fn open(snapshot: &StateSnapshot) -> Result<Self, RuntimeError> {
        let session = PlaybackSession::open_json(&snapshot.project, PlaybackConfig::default())
            .map_err(playback_runtime_error)?;
        Ok(Self {
            session,
            texture: None,
            caption: None,
            rendered_range: None,
        })
    }

    pub fn texture(&self) -> Option<&TextureHandle> {
        self.texture.as_ref()
    }

    pub fn caption(&self) -> Option<&str> {
        self.caption.as_deref()
    }

    /// Repoint playback at the latest authoritative daemon snapshot.
    pub fn replace_project(
        &mut self,
        project: &serde_json::Value,
        position: f64,
        playing: bool,
    ) -> Result<(), RuntimeError> {
        let handle = self.session.handle();
        handle.open_json(project).map_err(playback_runtime_error)?;
        handle
            .seek(position.max(0.0))
            .map_err(playback_runtime_error)?;
        if playing {
            handle.play().map_err(playback_runtime_error)?;
        }
        self.texture = None;
        self.caption = None;
        self.rendered_range = None;
        Ok(())
    }

    /// Replace playback with a bounded draft render. The generated project is
    /// an adapter-only projection and never enters canonical runtime state.
    pub fn replace_rendered_preview(
        &mut self,
        output: &Path,
        range: PreviewRange,
        fps: f64,
        size: (u32, u32),
        position: f64,
        playing: bool,
    ) -> Result<(), RuntimeError> {
        let (width, height) = size;
        let project = json!({
            "version": 2,
            "fps": fps,
            "width": width,
            "height": height,
            "sources": [{
                "id": "rendered-preview-source",
                "path": output.to_string_lossy(),
                "duration": range.duration(),
                "width": width,
                "height": height,
                "hasAudio": true
            }],
            "tracks": [{
                "id": "rendered-preview-track",
                "type": "video",
                "clips": [{
                    "id": "rendered-preview-clip",
                    "sourceId": "rendered-preview-source",
                    "in": 0,
                    "out": range.duration(),
                    "timelineStart": 0
                }]
            }]
        });
        let local_position = (position - range.start).clamp(0.0, range.duration());
        self.replace_project(&project, local_position, playing)?;
        self.rendered_range = Some(range);
        Ok(())
    }

    pub fn handle_action(
        &mut self,
        action: ViewerAction,
        model: &mut ViewerViewModel,
    ) -> Result<bool, RuntimeError> {
        if !matches!(
            &action,
            ViewerAction::ToggleFullscreen
                | ViewerAction::CancelPreview
                | ViewerAction::RetryPreview
        ) && !model.preview.accepts_playback()
        {
            return Err(RuntimeError::new(
                "PREVIEW_NOT_READY",
                "rendered preview is not ready",
            ));
        }
        match action {
            ViewerAction::TogglePlay => {
                if model.playing {
                    self.session.pause().map_err(playback_runtime_error)?;
                    model.playing = false;
                } else {
                    self.session.play().map_err(playback_runtime_error)?;
                    model.playing = true;
                }
                Ok(false)
            }
            ViewerAction::Seek(position) => {
                let session_position = if let Some(range) = self.rendered_range {
                    if position < range.start || position > range.end {
                        return Err(RuntimeError::new(
                            "PREVIEW_RANGE_MISS",
                            "requested position is outside the rendered preview range",
                        ));
                    }
                    position - range.start
                } else {
                    position
                };
                self.session
                    .seek(session_position.max(0.0))
                    .map_err(playback_runtime_error)?;
                model.playhead = position.clamp(0.0, model.duration);
                Ok(true)
            }
            ViewerAction::SetVolume(volume) => {
                let was_muted = model.muted;
                self.session
                    .set_volume(volume)
                    .map_err(playback_runtime_error)?;
                if volume == 0.0 {
                    self.session
                        .set_muted(true)
                        .map_err(playback_runtime_error)?;
                } else if was_muted {
                    self.session
                        .set_muted(false)
                        .map_err(playback_runtime_error)?;
                }
                if volume > 0.0 {
                    model.last_volume = volume;
                }
                model.volume = volume;
                model.muted = volume == 0.0;
                Ok(false)
            }
            ViewerAction::ToggleMute => {
                let muted = !model.muted;
                if muted {
                    if model.volume > 0.0 {
                        model.last_volume = model.volume;
                    }
                } else if model.volume == 0.0 {
                    let volume = model.last_volume.max(0.01);
                    self.session
                        .set_volume(volume)
                        .map_err(playback_runtime_error)?;
                    model.volume = volume;
                }
                self.session
                    .set_muted(muted)
                    .map_err(playback_runtime_error)?;
                model.muted = muted;
                Ok(false)
            }
            ViewerAction::ToggleFullscreen
            | ViewerAction::CancelPreview
            | ViewerAction::RetryPreview => Ok(false),
        }
    }

    /// Drain worker events and upload only the latest valid frame for this
    /// repaint. This is intentionally called from the UI thread.
    pub fn poll(&mut self, context: &egui::Context, model: &mut ViewerViewModel) -> Option<String> {
        let rendered_range = self.rendered_range;
        let can_present_frames = model.preview.accepts_playback();
        let mut error = None;
        let mut latest_frame = None;
        for event in self.session.drain_events() {
            if !can_present_frames {
                continue;
            }
            match event {
                PlaybackEvent::Opened {
                    duration,
                    width,
                    height,
                } => {
                    if rendered_range.is_none() {
                        model.sync_duration(duration);
                    }
                    model.sync_canvas(width, height);
                }
                PlaybackEvent::StateChanged(state) => {
                    model.playing = state == PlaybackState::Playing;
                    if state == PlaybackState::Ended {
                        model.playhead = rendered_range
                            .map_or(model.duration, |range| range.end.min(model.duration));
                    }
                }
                PlaybackEvent::Clock(clock) => {
                    if let Some(range) = rendered_range {
                        model.playhead = (range.start + clock.position).clamp(0.0, model.duration);
                    } else {
                        model.sync_duration(clock.duration);
                        model.playhead = clock.position.clamp(0.0, model.duration);
                    }
                }
                PlaybackEvent::Frame(frame) => {
                    if !frame.has_valid_pixels() {
                        error = Some("playback delivered an invalid RGBA frame".into());
                        continue;
                    }
                    latest_frame = Some(frame);
                }
                PlaybackEvent::Ended => {
                    model.playing = false;
                    model.playhead = rendered_range
                        .map_or(model.duration, |range| range.end.min(model.duration));
                }
                PlaybackEvent::Error(error_value) => {
                    error = Some(error_value.to_string());
                }
            }
        }
        if let Some(frame) = latest_frame {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &frame.pixels,
            );
            if let Some(texture) = self.texture.as_mut() {
                texture.set(image, TextureOptions::LINEAR);
            } else {
                self.texture =
                    Some(context.load_texture("cut-playback-frame", image, TextureOptions::LINEAR));
            }
            self.caption = frame
                .captions
                .iter()
                .map(|cue| cue.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
                .into();
            model.playhead = rendered_range.map_or(frame.timeline_time, |range| {
                range.start + frame.timeline_time
            });
            model.playhead = model.playhead.clamp(0.0, model.duration);
        }
        error
    }

    pub fn wait_for_event(&self, timeout: Duration) -> Option<PlaybackEvent> {
        self.session.next_event_timeout(timeout)
    }
}

fn playback_runtime_error(error: PlaybackError) -> RuntimeError {
    RuntimeError::new("PLAYBACK_ERROR", error.to_string())
}

pub fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let minutes = (seconds / 60.0).floor() as u64;
    let remainder = seconds - minutes as f64 * 60.0;
    format!("{minutes:02}:{remainder:04.1}")
}

#[cfg(test)]
mod tests {
    use super::{PreviewState, PreviewStatus};

    #[test]
    fn render_progress_is_clamped_and_keeps_the_active_job() {
        let mut preview = PreviewStatus::native_exact();
        preview.begin_render("preview-1".into());
        preview.set_progress(1.5);
        assert_eq!(preview.job_id(), Some("preview-1"));
        assert!(matches!(
            preview.state,
            PreviewState::Rendering { progress: 1.0, .. }
        ));
    }

    #[test]
    fn empty_preview_is_neutral_and_disables_playback() {
        let preview = PreviewStatus::empty();
        assert_eq!(preview.state, PreviewState::Empty);
        assert_eq!(preview.label(), "No media");
        assert!(!preview.accepts_playback());
        assert!(preview.reason_summary().is_empty());
    }
}
