use cut_playback::{PlaybackConfig, PlaybackError, PlaybackEvent, PlaybackSession, PlaybackState};
use cut_runtime::{RuntimeError, StateSnapshot};
use egui::{Color32, RichText, Slider, TextureHandle, TextureOptions};
use std::time::Duration;

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
}

#[derive(Clone, Debug, PartialEq)]
pub enum ViewerAction {
    TogglePlay,
    Seek(f64),
    SetVolume(f32),
    ToggleMute,
    ToggleFullscreen,
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
        ui.heading(RichText::new("Viewer").color(Color32::WHITE));
        ui.add_space(8.0);
        let available = ui.available_size();
        let aspect = self.canvas_width as f32 / self.canvas_height.max(1) as f32;
        let controls_height = 74.0;
        let max_height = (available.y - controls_height).max(180.0);
        let height = (available.x / aspect).min(max_height).max(180.0);
        let width = (height * aspect).min(available.x).max(1.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        ui.painter().rect_filled(rect, 4.0, Color32::BLACK);
        if let Some(texture) = texture {
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if self.playing {
                    "BUFFERING"
                } else {
                    "NO MEDIA"
                },
                egui::FontId::proportional(16.0),
                Color32::from_gray(130),
            );
        }
        if let Some(caption) = caption.filter(|caption| !caption.is_empty()) {
            ui.painter().text(
                egui::pos2(rect.center().x, rect.max.y - 26.0),
                egui::Align2::CENTER_CENTER,
                caption,
                egui::FontId::proportional(18.0),
                Color32::WHITE,
            );
        }

        ui.add_space(8.0);
        let mut action = None;
        ui.horizontal(|ui| {
            if ui
                .button(if self.playing { "Pause" } else { "Play" })
                .clicked()
            {
                action = Some(ViewerAction::TogglePlay);
            }
            ui.label(format_time(self.playhead));
            let mut playhead = self.playhead;
            if ui
                .add(
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
                .button(if self.muted { "Unmute" } else { "Mute" })
                .clicked()
            {
                action = Some(ViewerAction::ToggleMute);
            }
            let mut volume = self.volume;
            if ui
                .add(Slider::new(&mut volume, 0.0..=1.0).text("volume"))
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
}

/// App-owned adapter between playback events and egui's wgpu-backed texture
/// registry. The playback worker never touches an egui or winit object.
pub struct ViewerPlaybackAdapter {
    session: PlaybackSession,
    texture: Option<TextureHandle>,
    caption: Option<String>,
}

impl ViewerPlaybackAdapter {
    pub fn open(snapshot: &StateSnapshot) -> Result<Self, RuntimeError> {
        let session = PlaybackSession::open_json(&snapshot.project, PlaybackConfig::default())
            .map_err(playback_runtime_error)?;
        Ok(Self {
            session,
            texture: None,
            caption: None,
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
        self.caption = None;
        Ok(())
    }

    pub fn handle_action(
        &mut self,
        action: ViewerAction,
        model: &mut ViewerViewModel,
    ) -> Result<bool, RuntimeError> {
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
                self.session
                    .seek(position.max(0.0))
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
            ViewerAction::ToggleFullscreen => Ok(false),
        }
    }

    /// Drain worker events and upload only the latest valid frame for this
    /// repaint. This is intentionally called from the UI thread.
    pub fn poll(&mut self, context: &egui::Context, model: &mut ViewerViewModel) -> Option<String> {
        let mut error = None;
        let mut latest_frame = None;
        for event in self.session.drain_events() {
            match event {
                PlaybackEvent::Opened {
                    duration,
                    width,
                    height,
                } => {
                    model.sync_duration(duration);
                    model.sync_canvas(width, height);
                }
                PlaybackEvent::StateChanged(state) => {
                    model.playing = state == PlaybackState::Playing;
                    if state == PlaybackState::Ended {
                        model.playhead = model.duration;
                    }
                }
                PlaybackEvent::Clock(clock) => {
                    model.sync_duration(clock.duration);
                    model.playhead = clock.position.clamp(0.0, model.duration);
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
                    model.playhead = model.duration;
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
            model.playhead = frame.timeline_time.clamp(0.0, model.duration);
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
