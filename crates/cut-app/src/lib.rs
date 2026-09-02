//! Native Cut editor shell.
//!
//! `cut-app` owns the desktop composition root and view state. Project state
//! is read from and mutated through the project-scoped `cutd` client.

pub mod agent_rail;
pub mod config;
pub mod inspector;
pub mod media_bin;
pub mod runtime;
pub mod settings;
pub mod theme;
pub mod timeline;
pub mod ui;
pub mod view_model;
pub mod viewer;

mod icon;

use accesskit_winit::{Event as AccessKitEvent, WindowEvent as AccessKitWindowEvent};
use config::AppConfig;
use cut_agent::{
    AgentEvent as SessionAgentEvent, AgentSession, AgentSessionOptions, OpenAiApiMode,
    ProviderName, ReasoningEffort,
};
use cut_core::preview::{analyze_project, PreviewMode, PreviewRange};
use cut_mcp::{CutMcpRouter, McpProfile};
use cut_runtime::{RenderRange, RuntimeError};
use cut_skills::load_skills;
use inspector::InspectorAction;
use runtime::{run_command_surface_smoke, AppRuntime, ClipProperty};
use serde_json::Value;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, thread};
use timeline::TimelineAction;
use ui::{AppAction, UiState};
use view_model::AppViewModel;
use viewer::{PreviewStatus, ViewerAction, ViewerPlaybackAdapter};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalSize, Size};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::Fullscreen;
use winit::window::{Window, WindowAttributes, WindowId};

#[derive(Debug)]
enum AppEvent {
    AccessKit(AccessKitEvent),
    Agent(Box<AgentTransportEvent>),
    Preview(Box<PreviewTransportEvent>),
}

#[derive(Debug)]
enum AgentTransportEvent {
    AbortHandle(Arc<AtomicBool>),
    Update(Box<SessionAgentEvent>),
    Failed(String),
}

#[derive(Debug)]
enum PreviewTransportEvent {
    Progress {
        job_id: String,
        progress: f64,
    },
    Finished {
        job_id: String,
        output: PathBuf,
        range: RenderRange,
        result: Result<(), RuntimeError>,
    },
}

impl From<AccessKitEvent> for AppEvent {
    fn from(event: AccessKitEvent) -> Self {
        Self::AccessKit(event)
    }
}

struct NativeApp {
    config: AppConfig,
    runtime: AppRuntime,
    model: AppViewModel,
    playback: ViewerPlaybackAdapter,
    ui_state: UiState,
    egui_context: egui::Context,
    window: Option<Arc<Window>>,
    egui_state: Option<egui_winit::State>,
    painter: Option<egui_wgpu::winit::Painter>,
    accesskit_proxy: EventLoopProxy<AppEvent>,
    fatal_error: Option<cut_runtime::RuntimeError>,
    agent_abort: Option<Arc<AtomicBool>>,
    pending_preview: Option<(Instant, PreviewRange)>,
    last_state_poll: Instant,
}

impl NativeApp {
    fn new(
        config: AppConfig,
        accesskit_proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self, cut_runtime::RuntimeError> {
        let (runtime, snapshot) = AppRuntime::open(&config)?;
        let endpoint = runtime.endpoint_label();
        let model = AppViewModel::from_snapshot(&snapshot, &config, endpoint);
        let playback = ViewerPlaybackAdapter::open(&snapshot)?;
        let egui_context = egui::Context::default();
        egui_context.enable_accesskit();
        let mut app = Self {
            config,
            runtime,
            model,
            playback,
            ui_state: UiState::default(),
            egui_context,
            window: None,
            egui_state: None,
            painter: None,
            accesskit_proxy,
            fatal_error: None,
            agent_abort: None,
            pending_preview: None,
            last_state_poll: Instant::now(),
        };
        app.configure_preview(&snapshot, None, false);
        Ok(app)
    }

    fn initialize_window(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), cut_runtime::RuntimeError> {
        if self.window.is_some() {
            return Ok(());
        }

        let attributes = WindowAttributes::default()
            .with_title(self.config.window_title())
            .with_inner_size(Size::Physical(PhysicalSize::new(1440, 900)))
            .with_min_inner_size(Size::Physical(PhysicalSize::new(1024, 680)))
            .with_resizable(true)
            .with_visible(false)
            .with_window_icon(icon::window_icon());
        let window = Arc::new(event_loop.create_window(attributes).map_err(|error| {
            cut_runtime::RuntimeError::new("WINDOW_INIT_ERROR", error.to_string())
        })?);

        let mut egui_state = egui_winit::State::new(
            self.egui_context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            Some(winit::window::Theme::Dark),
            None,
        );
        egui_state.init_accesskit(event_loop, window.as_ref(), self.accesskit_proxy.clone());

        let wgpu_config = egui_wgpu::WgpuConfiguration {
            surface: egui_wgpu::SurfaceConfig::LOW_LATENCY,
            wgpu_setup: egui_wgpu::WgpuSetup::from_display_handle(
                event_loop.owned_display_handle(),
            ),
            ..Default::default()
        };
        let mut painter = pollster::block_on(egui_wgpu::winit::Painter::new(
            self.egui_context.clone(),
            wgpu_config,
            false,
            egui_wgpu::RendererOptions::default(),
        ));
        pollster::block_on(painter.set_window(egui::ViewportId::ROOT, Some(Arc::clone(&window))))
            .map_err(|error| cut_runtime::RuntimeError::new("GPU_INIT_ERROR", error.to_string()))?;

        window.set_visible(true);
        window.request_redraw();
        self.window = Some(window);
        self.egui_state = Some(egui_state);
        self.painter = Some(painter);
        Ok(())
    }

    fn resize(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        if let Some(painter) = self.painter.as_mut() {
            painter.on_window_resized(egui::ViewportId::ROOT, width, height);
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.as_ref().map(Arc::clone) else {
            return;
        };
        let raw_input = {
            let Some(egui_state) = self.egui_state.as_mut() else {
                return;
            };
            egui_state.take_egui_input(window.as_ref())
        };
        if let Some(error) = self
            .playback
            .poll(&self.egui_context, &mut self.model.viewer)
        {
            self.ui_state.status = Some(format!("Playback: {error}"));
        }
        let texture = self.playback.texture();
        let caption = self.playback.caption();
        let mut actions = Vec::new();
        let full_output = self.egui_context.run_ui(raw_input, |root_ui| {
            actions = ui::draw(
                root_ui,
                &mut self.model,
                &mut self.ui_state,
                texture,
                caption,
            );
        });
        self.handle_actions(actions, event_loop);
        if let Some(egui_state) = self.egui_state.as_mut() {
            egui_state.handle_platform_output(window.as_ref(), full_output.platform_output);
        }
        let clipped_primitives = self
            .egui_context
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        if let Some(painter) = self.painter.as_mut() {
            let mut textures_delta = full_output.textures_delta;
            painter.paint_and_update_textures(
                egui::ViewportId::ROOT,
                full_output.pixels_per_point,
                [0.0, 0.0, 0.0, 1.0],
                &clipped_primitives,
                &mut textures_delta,
                Vec::new(),
                &window,
            );
        }
        if self.model.viewer.playing {
            window.request_redraw();
        }
    }

    fn handle_actions(&mut self, actions: Vec<AppAction>, _event_loop: &ActiveEventLoop) {
        for action in actions {
            if let AppAction::AgentPrompt(prompt) = action {
                self.start_agent(prompt);
                continue;
            }
            if let AppAction::Proposal(agent_rail::AgentRailAction::Abort) = action {
                if let Some(abort) = self.agent_abort.as_ref() {
                    abort.store(true, Ordering::SeqCst);
                    self.ui_state.status = Some("Aborting agent…".into());
                }
                continue;
            }
            if let AppAction::Proposal(agent_rail::AgentRailAction::RequestChanges {
                proposal_id,
                feedback,
            }) = action
            {
                self.request_proposal_changes(&proposal_id, feedback);
                continue;
            }
            if let AppAction::Viewer(viewer_action) = action {
                let viewer_action = match viewer_action {
                    ViewerAction::Seek(position) => ViewerAction::Seek(
                        self.model
                            .timeline
                            .frame_grid()
                            .quantize(position.clamp(0.0, self.model.duration.max(0.0))),
                    ),
                    action => action,
                };
                match viewer_action {
                    ViewerAction::ToggleFullscreen => {
                        let fullscreen =
                            self.window.as_ref().and_then(|window| window.fullscreen());
                        if let Some(window) = self.window.as_ref() {
                            if fullscreen.is_some() {
                                window.set_fullscreen(None);
                            } else {
                                window.set_fullscreen(Some(Fullscreen::Borderless(
                                    window.current_monitor(),
                                )));
                            }
                        }
                    }
                    ViewerAction::CancelPreview => self.cancel_preview_render(),
                    ViewerAction::RetryPreview => self.retry_preview_render(),
                    viewer_action => {
                        if let Err(error) = self
                            .playback
                            .handle_action(viewer_action, &mut self.model.viewer)
                        {
                            self.ui_state.status =
                                Some(format!("{}: {}", error.code, error.message));
                        }
                    }
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
                continue;
            }

            let revisioned_edit = matches!(
                &action,
                AppAction::Inspector(_)
                    | AppAction::MediaBin(_)
                    | AppAction::Timeline(
                        TimelineAction::PlaceClip { .. }
                            | TimelineAction::TrimClip { .. }
                            | TimelineAction::SplitClip { .. }
                            | TimelineAction::RemoveClip { .. }
                    )
            );
            let result = match action {
                AppAction::Refresh => self.runtime.refresh(),
                AppAction::AddVideoTrack => self.runtime.add_video_track(self.model.revision),
                AppAction::Undo => self.runtime.undo(self.model.revision),
                AppAction::Redo => self.runtime.redo(self.model.revision),
                AppAction::Inspector(action) => self.apply_inspector_action(action),
                AppAction::MediaBin(media_bin::MediaBinAction::Import(path)) => {
                    let result = self.runtime.import_media(&path, self.model.revision);
                    if result.is_ok() {
                        self.ui_state.media_import_draft.clear();
                    }
                    result
                }
                AppAction::Timeline(TimelineAction::PlaceClip {
                    clip_id,
                    timeline_frame,
                    track_id,
                }) => self.runtime.place_clip(
                    &clip_id,
                    timeline_frame,
                    track_id.as_deref(),
                    self.model.revision,
                ),
                AppAction::Timeline(TimelineAction::TrimClip {
                    clip_id,
                    edge,
                    timeline_frame,
                }) => self
                    .runtime
                    .trim_clip(&clip_id, edge, timeline_frame, self.model.revision),
                AppAction::Timeline(TimelineAction::SplitClip { clip_id, at_frame }) => self
                    .runtime
                    .split_clip(&clip_id, at_frame, self.model.revision),
                AppAction::Timeline(TimelineAction::RemoveClip { clip_id }) => {
                    let result = self.runtime.remove_clip(&clip_id, self.model.revision);
                    if result.is_ok() {
                        self.ui_state.timeline_selection = timeline::TimelineSelection::None;
                    }
                    result
                }
                AppAction::Timeline(TimelineAction::Seek(_)) => continue,
                AppAction::AgentPrompt(_) => continue,
                AppAction::Proposal(agent_rail::AgentRailAction::Accept(proposal_id)) => self
                    .runtime
                    .accept_proposal(&proposal_id, self.model.revision),
                AppAction::Proposal(agent_rail::AgentRailAction::Reject(proposal_id)) => self
                    .runtime
                    .reject_proposal(&proposal_id, self.model.revision),
                AppAction::Proposal(agent_rail::AgentRailAction::Send(_)) => continue,
                AppAction::Proposal(agent_rail::AgentRailAction::Abort) => {
                    unreachable!("abort is handled before revisioned actions")
                }
                AppAction::Proposal(agent_rail::AgentRailAction::RequestChanges { .. }) => {
                    unreachable!("request changes is handled before revisioned actions")
                }
                AppAction::Proposal(agent_rail::AgentRailAction::None) => continue,
                AppAction::Viewer(_) => continue,
            };
            match result {
                Ok(snapshot) => self.sync_snapshot(snapshot, None),
                Err(error) if revisioned_edit && error.code == "REVISION_CONFLICT" => {
                    self.ui_state
                        .timeline_interaction
                        .cancel_for_revision_conflict();
                    match self.runtime.refresh() {
                        Ok(snapshot) => self.sync_snapshot(
                            snapshot,
                            Some("Project changed elsewhere; edit cancelled".into()),
                        ),
                        Err(refresh_error) => {
                            self.ui_state.status = Some(format!(
                                "Timeline conflict; refresh failed: {}: {}",
                                refresh_error.code, refresh_error.message
                            ));
                        }
                    }
                }
                Err(error) => {
                    self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
                }
            }
        }
    }

    fn apply_inspector_action(
        &self,
        action: InspectorAction,
    ) -> Result<cut_runtime::StateSnapshot, RuntimeError> {
        let revision = self.model.revision;
        let grid = self.model.timeline.frame_grid();
        match action {
            InspectorAction::SetTimelineStart {
                clip_id,
                timeline_frame,
            } => self
                .runtime
                .place_clip(&clip_id, timeline_frame, None, revision),
            InspectorAction::SetTimelineDuration {
                clip_id,
                timeline_start_frame,
                duration_frame,
            } => self.runtime.update_clip_timing(
                &clip_id,
                grid.seconds_at(timeline_start_frame),
                grid.seconds_at(duration_frame.max(1)),
                revision,
            ),
            InspectorAction::SetSourceRange {
                clip_id,
                source_in_frame,
                source_out_frame,
            } => self.runtime.update_clip_source_range(
                &clip_id,
                grid.seconds_at(source_in_frame),
                grid.seconds_at(source_out_frame),
                revision,
            ),
            InspectorAction::SetSpeed { clip_id, speed } => {
                self.runtime.update_clip_speed(&clip_id, speed, revision)
            }
            InspectorAction::SetVolume { clip_id, volume } => {
                self.runtime.update_clip_volume(&clip_id, volume, revision)
            }
            InspectorAction::SetCrop { clip_id, crop } => self.runtime.update_clip_property(
                &clip_id,
                ClipProperty::Crop(Some(crop)),
                revision,
            ),
            InspectorAction::ClearCrop { clip_id } => {
                self.runtime
                    .update_clip_property(&clip_id, ClipProperty::Crop(None), revision)
            }
            InspectorAction::SetTransform { clip_id, transform } => {
                self.runtime.update_clip_property(
                    &clip_id,
                    ClipProperty::Transform(Some(*transform)),
                    revision,
                )
            }
            InspectorAction::ClearTransform { clip_id } => {
                self.runtime
                    .update_clip_property(&clip_id, ClipProperty::Transform(None), revision)
            }
            InspectorAction::SetOpacity { clip_id, opacity } => self.runtime.update_clip_property(
                &clip_id,
                ClipProperty::Opacity(opacity),
                revision,
            ),
            InspectorAction::SetEffects { clip_id, effects } => self.runtime.update_clip_property(
                &clip_id,
                ClipProperty::Effects(effects),
                revision,
            ),
            InspectorAction::SetKeyframes { clip_id, keyframes } => self
                .runtime
                .update_clip_property(&clip_id, ClipProperty::Keyframes(keyframes), revision),
            InspectorAction::SetTransitions {
                clip_id,
                transitions,
            } => self.runtime.update_clip_property(
                &clip_id,
                ClipProperty::Transitions(transitions),
                revision,
            ),
        }
    }

    fn sync_snapshot(&mut self, snapshot: cut_runtime::StateSnapshot, status: Option<String>) {
        self.ui_state.timeline_interaction.drag = None;
        let playhead = self.model.viewer.playhead;
        let playing = self.model.viewer.playing;
        let volume = self.model.viewer.volume;
        let last_volume = self.model.viewer.last_volume;
        let muted = self.model.viewer.muted;
        self.cancel_active_preview();
        let endpoint = self.runtime.endpoint_label();
        let agent_run = self.model.agent_rail.run.clone();
        self.model =
            AppViewModel::from_snapshot_with_run(&snapshot, &self.config, endpoint, agent_run);
        self.model.viewer.playhead = playhead.min(self.model.duration.max(0.0));
        self.model.viewer.playing = playing;
        self.model.viewer.volume = volume;
        self.model.viewer.last_volume = last_volume;
        self.model.viewer.muted = muted;
        self.configure_preview(
            &snapshot,
            status.or_else(|| Some(format!("Synced revision {}", snapshot.revision))),
            true,
        );
    }

    fn configure_preview(
        &mut self,
        snapshot: &cut_runtime::StateSnapshot,
        status: Option<String>,
        debounce_render: bool,
    ) {
        let plan = analyze_project(
            &snapshot.project,
            Some(self.runtime.project_directory()),
            None,
        );
        match plan.mode {
            PreviewMode::NativeExact => {
                self.pending_preview = None;
                self.model.viewer.preview = PreviewStatus::from_plan(plan);
                let playhead = self.model.viewer.playhead;
                let playing = self.model.viewer.playing;
                if let Err(error) =
                    self.playback
                        .replace_project(&snapshot.project, playhead, playing)
                {
                    self.model.viewer.playing = false;
                    self.model.viewer.preview.mark_failed(error.message.clone());
                    self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
                } else {
                    self.ui_state.status = status;
                }
            }
            PreviewMode::RenderedFallback => {
                let Some(range) = self.preview_range_for_position(snapshot.duration) else {
                    self.model.viewer.preview = PreviewStatus::from_plan(plan);
                    self.model.viewer.playing = false;
                    self.model.viewer.preview.mark_failed(
                        "project has no positive duration for a bounded rendered preview".into(),
                    );
                    self.ui_state.status = Some(
                        "Preview unavailable: project has no positive renderable duration".into(),
                    );
                    return;
                };
                let mut plan = plan;
                plan.range = Some(range);
                self.model.viewer.preview = PreviewStatus::from_plan(plan);
                self.model.viewer.playing = false;
                if debounce_render {
                    self.pending_preview =
                        Some((Instant::now() + Duration::from_millis(350), range));
                    self.ui_state.status = status.or_else(|| Some("Preview update queued".into()));
                } else {
                    self.start_preview_render(range, status);
                }
            }
            PreviewMode::Unavailable => {
                self.pending_preview = None;
                self.model.viewer.preview = PreviewStatus::from_plan(plan);
                self.model.viewer.playing = false;
                self.ui_state.status = status.or_else(|| {
                    Some("Preview unavailable: fix the project errors before rendering".into())
                });
            }
        }
    }

    fn start_preview_render(&mut self, range: PreviewRange, status: Option<String>) {
        let render_range = match RenderRange::new(range.start, range.duration()) {
            Ok(range) => range,
            Err(error) => {
                self.model.viewer.playing = false;
                self.model.viewer.preview.mark_failed(error.message.clone());
                self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
                return;
            }
        };
        let (job, project, output) = match self.runtime.start_preview_render(render_range.clone()) {
            Ok(result) => result,
            Err(error) => {
                self.model.viewer.playing = false;
                self.model.viewer.preview.mark_failed(error.message.clone());
                self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
                return;
            }
        };
        let job_range = job.range.clone().unwrap_or(render_range);
        let job_id = job.job_id.clone();
        let cancel_file =
            cut_runtime::render_cancel_path(self.runtime.project_directory(), &job_id);
        self.model.viewer.preview.begin_render(job_id.clone());
        self.model.viewer.playing = false;
        self.ui_state.status = status.or_else(|| Some(format!("Rendering preview {job_id}")));

        let runtime = self.runtime.clone();
        let proxy = self.accesskit_proxy.clone();
        let progress_proxy = self.accesskit_proxy.clone();
        let output_for_event = output.clone();
        let job_range_for_event = job_range.clone();
        thread::spawn(move || {
            let result = (|| {
                runtime.update_render_job(&job_id, "running", None, None, Some(0.0))?;
                let progress_runtime = runtime.clone();
                let progress_job_id = job_id.clone();
                let mut last_progress = -1.0_f64;
                let rendered = cut_runtime::run_render_with_cancel_file_progress(
                    job_id.clone(),
                    project,
                    output,
                    true,
                    Some(job_range.clone()),
                    Some(cancel_file.clone()),
                    move |progress| {
                        if progress < 1.0 && progress - last_progress < 0.01 {
                            return;
                        }
                        last_progress = progress;
                        let _ = progress_proxy.send_event(AppEvent::Preview(Box::new(
                            PreviewTransportEvent::Progress {
                                job_id: progress_job_id.clone(),
                                progress,
                            },
                        )));
                        let _ = progress_runtime.update_render_job(
                            &progress_job_id,
                            "running",
                            None,
                            None,
                            Some(progress),
                        );
                    },
                );
                match rendered {
                    Ok(()) => {
                        runtime.update_render_job(
                            &job_id,
                            "completed",
                            None,
                            Some(output_for_event.to_string_lossy().into_owned()),
                            Some(1.0),
                        )?;
                        Ok(())
                    }
                    Err(error) => {
                        let cancelled = runtime
                            .render_job(&job_id)
                            .map(|job| job.status == "cancelled")
                            .unwrap_or(false);
                        if cancelled {
                            Err(RuntimeError::new(
                                "CANCELLED",
                                "preview render was cancelled",
                            ))
                        } else {
                            let message = error.message.clone();
                            runtime.update_render_job(
                                &job_id,
                                "failed",
                                Some(message),
                                None,
                                None,
                            )?;
                            Err(error)
                        }
                    }
                }
            })();
            let _ = std::fs::remove_file(&cancel_file);
            let _ = proxy.send_event(AppEvent::Preview(Box::new(
                PreviewTransportEvent::Finished {
                    job_id,
                    output: output_for_event,
                    range: job_range_for_event,
                    result,
                },
            )));
        });
    }

    fn cancel_active_preview(&mut self) {
        let Some(job_id) = self.model.viewer.preview.job_id().map(str::to_owned) else {
            return;
        };
        if self.runtime.cancel_render(&job_id).is_err() {
            let cancel_file =
                cut_runtime::render_cancel_path(self.runtime.project_directory(), &job_id);
            let _ = std::fs::write(cancel_file, b"cancelled\n");
        }
    }

    fn cancel_preview_render(&mut self) {
        let Some(job_id) = self.model.viewer.preview.job_id().map(str::to_owned) else {
            return;
        };
        match self.runtime.cancel_render(&job_id) {
            Ok(_) => {
                self.model
                    .viewer
                    .preview
                    .mark_failed("preview render cancelled".into());
                self.model.viewer.playing = false;
                self.ui_state.status = Some("Preview render cancelled".into());
            }
            Err(error) => {
                self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
            }
        }
    }

    fn retry_preview_render(&mut self) {
        if self.model.viewer.preview.job_id().is_some() {
            return;
        }
        let range = self
            .model
            .viewer
            .preview
            .plan
            .range
            .or_else(|| self.preview_range_for_position(self.model.duration));
        let Some(range) = range else {
            self.ui_state.status = Some("Preview unavailable: no renderable range".into());
            return;
        };
        self.start_preview_render(range, None);
    }

    fn handle_preview_event(&mut self, event: PreviewTransportEvent) {
        let (job_id, output, job_range, result) = match event {
            PreviewTransportEvent::Progress { job_id, progress } => {
                if self.model.viewer.preview.job_id() == Some(job_id.as_str()) {
                    self.model.viewer.preview.set_progress(progress);
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
                return;
            }
            PreviewTransportEvent::Finished {
                job_id,
                output,
                range,
                result,
            } => (job_id, output, range, result),
        };
        if self.model.viewer.preview.job_id() != Some(job_id.as_str()) {
            return;
        }
        match result {
            Ok(()) => {
                let Some(range) = PreviewRange::new(job_range.start, job_range.end()) else {
                    self.model
                        .viewer
                        .preview
                        .mark_failed("invalid rendered preview range".into());
                    self.ui_state.status = Some("Preview render failed: invalid range".into());
                    return;
                };
                let snapshot = match self.runtime.refresh() {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        self.model.viewer.preview.mark_failed(error.message.clone());
                        self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
                        return;
                    }
                };
                if !output.is_file() {
                    self.model
                        .viewer
                        .preview
                        .mark_failed("render output is missing".into());
                    self.ui_state.status = Some("Preview render failed: output is missing".into());
                    return;
                }
                let fps = snapshot
                    .project
                    .get("fps")
                    .and_then(Value::as_f64)
                    .filter(|fps| fps.is_finite() && *fps > 0.0)
                    .unwrap_or(30.0);
                let width =
                    project_dimension(&snapshot.project, "width", self.model.viewer.canvas_width);
                let height =
                    project_dimension(&snapshot.project, "height", self.model.viewer.canvas_height);
                if let Err(error) = self.playback.replace_rendered_preview(
                    &output,
                    range,
                    fps,
                    (width, height),
                    self.model.viewer.playhead,
                    false,
                ) {
                    self.model.viewer.preview.mark_failed(error.message.clone());
                    self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
                    return;
                }
                self.model.viewer.preview.mark_rendered();
                self.model.viewer.playing = false;
                self.ui_state.status = Some("Rendered preview ready".into());
            }
            Err(error) => {
                self.model.viewer.playing = false;
                let message = if error.code == "CANCELLED" {
                    "preview render cancelled".to_owned()
                } else {
                    error.message
                };
                self.model.viewer.preview.mark_failed(message.clone());
                self.ui_state.status = Some(format!("Preview render failed: {message}"));
            }
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn preview_range_for_position(&self, duration: f64) -> Option<PreviewRange> {
        if !duration.is_finite() || duration <= 0.0 {
            return None;
        }
        let duration = duration.min(1.0e9);
        let length = duration.min(cut_runtime::MAX_RENDER_RANGE_SECONDS);
        let max_start = (duration - length).max(0.0);
        let center = self.model.viewer.playhead.clamp(0.0, duration);
        let start = (center - length / 2.0).clamp(0.0, max_start);
        let end = (start + length).min(duration);
        let grid = self.model.timeline.frame_grid();
        let quantized_start = grid.quantize(start).clamp(0.0, duration);
        let quantized_end = grid.quantize(end).clamp(0.0, duration);
        PreviewRange::new(quantized_start, quantized_end).or_else(|| PreviewRange::new(start, end))
    }

    fn run_idle_work(&mut self) {
        let now = Instant::now();
        if let Some((deadline, range)) = self.pending_preview {
            if now >= deadline {
                self.pending_preview = None;
                self.start_preview_render(range, None);
            }
        }

        if now.duration_since(self.last_state_poll) < Duration::from_millis(750) {
            return;
        }
        self.last_state_poll = now;
        match self.runtime.refresh() {
            Ok(snapshot)
                if snapshot.revision != self.model.revision
                    || pending_proposal_id(snapshot.pending_proposal.as_ref())
                        != self.model.agent_rail.pending_proposal_id() =>
            {
                self.sync_snapshot(snapshot, Some("Project state updated".into()))
            }
            Ok(_) => {}
            Err(error) => {
                self.ui_state.status = Some(format!("Sync unavailable: {}", error.message));
            }
        }
    }

    fn start_agent(&mut self, prompt: String) {
        if self.model.agent_rail.run.is_running() {
            self.ui_state.status = Some("Agent is already running".into());
            return;
        }
        let config = match NativeAgentConfig::from_env() {
            Ok(config) => config,
            Err(error) => {
                self.ui_state.status = Some(error);
                return;
            }
        };
        self.model.agent_rail.begin_run(&prompt);
        self.ui_state.status = Some("Agent is working…".into());
        let directory = self.config.project_directory().to_path_buf();
        let cutd = self.config.cutd_binary().to_path_buf();
        let proxy = self.accesskit_proxy.clone();
        thread::spawn(move || {
            let result = run_native_agent(
                &directory,
                &cutd,
                &config,
                &prompt,
                |abort| {
                    let _ = proxy.send_event(AppEvent::Agent(Box::new(
                        AgentTransportEvent::AbortHandle(abort),
                    )));
                },
                |event| {
                    let _ = proxy.send_event(AppEvent::Agent(Box::new(
                        AgentTransportEvent::Update(Box::new(event)),
                    )));
                },
            );
            if let Err(error) = result {
                let _ = proxy.send_event(AppEvent::Agent(Box::new(AgentTransportEvent::Failed(
                    error,
                ))));
            }
        });
    }

    fn request_proposal_changes(&mut self, proposal_id: &str, feedback: String) {
        if self.model.agent_rail.run.is_running() {
            self.ui_state.status = Some("Agent is already running".into());
            return;
        }
        let summary = rejected_proposal_summary(
            self.runtime
                .refresh()
                .ok()
                .and_then(|snapshot| snapshot.pending_proposal),
            proposal_id,
        );
        match self
            .runtime
            .reject_proposal(proposal_id, self.model.revision)
        {
            Ok(snapshot) => {
                self.sync_snapshot(snapshot, None);
                self.start_agent(format!(
                    "Revise the rejected proposal using this feedback:\n{feedback}\n\nRejected proposal summary:\n{summary}"
                ));
            }
            Err(error) => {
                self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
            }
        }
    }

    fn handle_agent_event(&mut self, event: AgentTransportEvent) {
        match event {
            AgentTransportEvent::AbortHandle(abort) => {
                self.agent_abort = Some(abort);
            }
            AgentTransportEvent::Update(event) => {
                let done = matches!(event.as_ref(), SessionAgentEvent::Done { .. });
                let aborted_event = matches!(event.as_ref(), SessionAgentEvent::Aborted);
                let was_aborted = self.model.agent_rail.run.is_aborted();
                self.model.agent_rail.apply_run_event(*event);
                if done {
                    self.agent_abort = None;
                    self.finish_agent_run(was_aborted);
                } else if aborted_event {
                    self.agent_abort = None;
                    self.ui_state.status = Some("Agent aborted".into());
                }
            }
            AgentTransportEvent::Failed(error) => {
                self.agent_abort = None;
                let error = compact_agent_error(&error);
                self.model.agent_rail.fail_run(&error);
                if let Ok(snapshot) = self.runtime.refresh() {
                    self.sync_snapshot(snapshot, None);
                }
                self.ui_state.status = Some(format!("Agent error: {error}"));
            }
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn finish_agent_run(&mut self, aborted: bool) {
        let snapshot = match self.runtime.refresh() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.ui_state.status = Some(format!(
                    "Agent finished, but refresh failed: {}: {}",
                    error.code, error.message
                ));
                return;
            }
        };
        self.sync_snapshot(snapshot, None);
        if aborted {
            self.ui_state.status = Some("Agent aborted".into());
        } else if let Some(proposal_id) = self.model.agent_rail.run.awaiting_proposal_id() {
            self.ui_state.status = Some(format!("Agent staged proposal {proposal_id}"));
        } else {
            self.ui_state.status = Some("Agent completed".into());
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: cut_runtime::RuntimeError) {
        eprintln!("cut: {}: {}", error.code, error.message);
        self.fatal_error = Some(error);
        event_loop.exit();
    }
}

fn pending_proposal_id(proposal: Option<&cut_runtime::PendingProposal>) -> Option<String> {
    match proposal {
        Some(cut_runtime::PendingProposal::Pending { proposal_id, .. }) => {
            Some(proposal_id.clone())
        }
        Some(cut_runtime::PendingProposal::Accepted { .. })
        | Some(cut_runtime::PendingProposal::Rejected { .. })
        | None => None,
    }
}

impl ApplicationHandler<AppEvent> for NativeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.initialize_window(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(painter) = self.painter.as_mut() {
            let _ = pollster::block_on(painter.set_window(egui::ViewportId::ROOT, None));
        }
        self.window = None;
        self.egui_state = None;
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        let AppEvent::AccessKit(event) = event else {
            match event {
                AppEvent::Agent(event) => self.handle_agent_event(*event),
                AppEvent::Preview(event) => self.handle_preview_event(*event),
                AppEvent::AccessKit(_) => unreachable!("accesskit event was matched above"),
            }
            return;
        };
        let Some(window) = self.window.as_ref().map(Arc::clone) else {
            return;
        };
        if event.window_id != window.id() {
            return;
        }
        match event.window_event {
            AccessKitWindowEvent::InitialTreeRequested => window.request_redraw(),
            AccessKitWindowEvent::ActionRequested(request) => {
                if let Some(egui_state) = self.egui_state.as_mut() {
                    egui_state
                        .egui_input_mut()
                        .events
                        .push(egui::Event::AccessKitActionRequest(request));
                }
                window.request_redraw();
            }
            AccessKitWindowEvent::AccessibilityDeactivated => {}
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref().map(Arc::clone) else {
            return;
        };
        if window.id() != window_id {
            return;
        }
        if matches!(event, WindowEvent::CloseRequested) {
            event_loop.exit();
            return;
        }
        let repaint = self
            .egui_state
            .as_mut()
            .map(|state| state.on_window_event(window.as_ref(), &event).repaint)
            .unwrap_or(false);
        match event {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => self.resize(),
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
        if repaint {
            window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.run_idle_work();
        let next_poll = self.last_state_poll + Duration::from_millis(750);
        let wake_at = self
            .pending_preview
            .map(|(deadline, _)| deadline.min(next_poll))
            .unwrap_or(next_poll);
        event_loop.set_control_flow(ControlFlow::WaitUntil(wake_at));
        if self.model.viewer.playing {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
    }
}

struct NativeAgentConfig {
    provider: ProviderName,
    api_key: String,
    api_mode: OpenAiApiMode,
    reasoning_effort: Option<ReasoningEffort>,
    model: Option<String>,
    base_url: Option<String>,
}

impl NativeAgentConfig {
    fn from_env() -> Result<Self, String> {
        let provider = match env::var("CUT_PROVIDER").ok().as_deref() {
            Some("anthropic") => ProviderName::Anthropic,
            Some("openai") => ProviderName::Openai,
            Some(other) => return Err(format!("Unknown provider: {other}")),
            None if env::var_os("OPENAI_API_KEY").is_some() => ProviderName::Openai,
            _ => ProviderName::Anthropic,
        };
        let key_name = match provider {
            ProviderName::Anthropic => "ANTHROPIC_API_KEY",
            ProviderName::Openai => "OPENAI_API_KEY",
        };
        let api_key = env::var(key_name)
            .map_err(|_| format!("Set {key_name} before sending an agent prompt"))?;
        let api_mode = match env::var("OPENAI_API_MODE").ok().as_deref() {
            Some("responses") => OpenAiApiMode::Responses,
            Some("chat") | None => OpenAiApiMode::Chat,
            Some(other) => return Err(format!("Unknown OpenAI API mode: {other}")),
        };
        let reasoning_effort = env::var("OPENAI_REASONING_EFFORT")
            .ok()
            .map(|value| parse_reasoning_effort(&value))
            .transpose()?;
        Ok(Self {
            provider,
            api_key,
            api_mode,
            reasoning_effort,
            model: env::var("CUT_MODEL").ok(),
            base_url: (provider == ProviderName::Openai)
                .then(|| env::var("OPENAI_BASE_URL").ok())
                .flatten(),
        })
    }
}

fn parse_reasoning_effort(value: &str) -> Result<ReasoningEffort, String> {
    match value {
        "none" => Ok(ReasoningEffort::None),
        "minimal" => Ok(ReasoningEffort::Minimal),
        "low" => Ok(ReasoningEffort::Low),
        "medium" => Ok(ReasoningEffort::Medium),
        "high" => Ok(ReasoningEffort::High),
        "xhigh" => Ok(ReasoningEffort::Xhigh),
        "max" => Ok(ReasoningEffort::Max),
        other => Err(format!("Unknown reasoning effort: {other}")),
    }
}

fn project_dimension(project: &Value, key: &str, fallback: u32) -> u32 {
    project
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value >= 2)
        .unwrap_or(fallback.max(2))
}

fn run_native_agent(
    directory: &Path,
    cutd: &Path,
    config: &NativeAgentConfig,
    prompt: &str,
    on_ready: impl FnOnce(Arc<AtomicBool>),
    mut emit: impl FnMut(SessionAgentEvent),
) -> Result<String, String> {
    let router = CutMcpRouter::new(directory, McpProfile::Full, Some(cutd))?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .ok_or_else(|| "could not resolve bundled skills".to_owned())?;
    let user_skills = env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".cut/skills"))
        .unwrap_or_default();
    let skills = load_skills(&[
        root.join("skills"),
        user_skills,
        directory.join(".cut/skills"),
    ])
    .map_err(|error| error.to_string())?;
    let mut session = AgentSession::new(AgentSessionOptions {
        provider: config.provider,
        api_mode: config.api_mode,
        reasoning_effort: config.reasoning_effort,
        api_key: Some(config.api_key.clone()),
        base_url: config.base_url.clone(),
        model: config.model.clone(),
        skills,
        tools: router.agent_tools(),
        ..AgentSessionOptions::default()
    });
    on_ready(session.abort_handle());
    session
        .run_with_events(prompt, &mut emit)
        .map_err(|error| error.to_string())
}

fn compact_agent_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if [
        "api_key",
        "apikey",
        "access_token",
        "authorization",
        "bearer ",
        "cookie",
        "password",
        "secret",
        "token=",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return "sensitive agent error redacted".into();
    }
    let suffix = "…";
    if error.len() <= 512 {
        return error.to_owned();
    }
    let limit = 512 - suffix.len();
    let mut end = 0;
    for (index, character) in error.char_indices() {
        let next = index + character.len_utf8();
        if next > limit {
            break;
        }
        end = next;
    }
    format!("{}{}", &error[..end], suffix)
}

fn rejected_proposal_summary(
    pending: Option<cut_runtime::PendingProposal>,
    proposal_id: &str,
) -> String {
    const MAX_IMPACTS: usize = 8;
    let Some(cut_runtime::PendingProposal::Pending {
        proposal_id: pending_id,
        impacts,
        diff,
        diff_truncated,
        ..
    }) = pending
    else {
        return format!("proposal {proposal_id}; semantic impacts unavailable");
    };
    if pending_id != proposal_id {
        return format!("proposal {proposal_id}; semantic impacts unavailable");
    }
    let Some(impacts) = impacts else {
        return format!(
            "proposal {proposal_id}; {} raw change(s); semantic impacts unavailable{}",
            diff.len(),
            if diff_truncated {
                "; diff truncated"
            } else {
                ""
            }
        );
    };
    let mut labels = impacts
        .iter()
        .take(MAX_IMPACTS)
        .map(proposal_impact_label)
        .collect::<Vec<_>>();
    if impacts.len() > MAX_IMPACTS {
        labels.push(format!("and {} more", impacts.len() - MAX_IMPACTS));
    }
    format!(
        "proposal {proposal_id}; {} semantic impact(s): {}",
        impacts.len(),
        if labels.is_empty() {
            "no changes".into()
        } else {
            labels.join(", ")
        }
    )
}

fn proposal_impact_label(impact: &cut_runtime::ProposalImpact) -> String {
    use cut_runtime::ProposalImpact;
    match impact {
        ProposalImpact::TrackAdded { track_id, .. } => format!("track {track_id} added"),
        ProposalImpact::TrackRemoved { track_id, .. } => format!("track {track_id} removed"),
        ProposalImpact::TrackReordered { track_id, .. } => format!("track {track_id} reordered"),
        ProposalImpact::ClipAdded { clip_id, .. } => format!("clip {clip_id} added"),
        ProposalImpact::ClipRemoved { clip_id, .. } => format!("clip {clip_id} removed"),
        ProposalImpact::ClipMoved { clip_id, .. } => format!("clip {clip_id} moved"),
        ProposalImpact::ClipTrimmed { clip_id, .. } => format!("clip {clip_id} trimmed"),
        ProposalImpact::ClipPropertyChanged {
            clip_id, property, ..
        } => format!("clip {clip_id} changed {property}"),
        ProposalImpact::OverlayAdded { overlay_id, .. } => format!("overlay {overlay_id} added"),
        ProposalImpact::OverlayRemoved { overlay_id, .. } => {
            format!("overlay {overlay_id} removed")
        }
        ProposalImpact::OverlayChanged { overlay_id, .. } => {
            format!("overlay {overlay_id} changed")
        }
        ProposalImpact::ProjectSettingChanged { property, .. } => {
            format!("project setting {property} changed")
        }
        ProposalImpact::Unclassified { path, .. } => format!("unclassified change at {path}"),
    }
}

/// Open the native window and run the winit event loop.
pub fn launch(config: AppConfig) -> Result<(), cut_runtime::RuntimeError> {
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|error| cut_runtime::RuntimeError::new("EVENT_LOOP_ERROR", error.to_string()))?;
    let mut app = NativeApp::new(config, event_loop.create_proxy())?;
    let result = event_loop
        .run_app(&mut app)
        .map_err(|error| cut_runtime::RuntimeError::new("EVENT_LOOP_ERROR", error.to_string()));
    match app.fatal_error {
        Some(error) => Err(error),
        None => result,
    }
}

/// Expose the real binary-level command smoke surface to integration scripts.
pub fn command_surface_smoke(
    config: &AppConfig,
) -> Result<serde_json::Value, cut_runtime::RuntimeError> {
    run_command_surface_smoke(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_changes_summary_is_semantic_and_bounded() {
        let impacts = (0..10)
            .map(|index| cut_runtime::ProposalImpact::ClipMoved {
                clip_id: format!("clip-{index}"),
                from_track_id: "a".into(),
                from: cut_runtime::ProposalTimeRange {
                    start: 0.0,
                    end: 1.0,
                },
                to_track_id: "b".into(),
                to: cut_runtime::ProposalTimeRange {
                    start: 1.0,
                    end: 2.0,
                },
            })
            .collect();
        let pending = cut_runtime::PendingProposal::Pending {
            proposal_id: "proposal-1".into(),
            base_revision: 2,
            command: serde_json::json!({}),
            diff: Vec::new(),
            diff_truncated: false,
            freshness: cut_runtime::ProposalFreshness::Current,
            impacts: Some(impacts),
        };

        let summary = rejected_proposal_summary(Some(pending), "proposal-1");

        assert!(summary.contains("clip clip-0 moved"));
        assert!(summary.contains("and 2 more"));
        assert!(!summary.contains("clip clip-8 moved"));
    }
}
