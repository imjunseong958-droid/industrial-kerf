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
pub mod timeline;
pub mod ui;
pub mod view_model;
pub mod viewer;

mod icon;

use accesskit_winit::{Event as AccessKitEvent, WindowEvent as AccessKitWindowEvent};
use config::AppConfig;
use cut_agent::{AgentSession, AgentSessionOptions, OpenAiApiMode, ProviderName, ReasoningEffort};
use cut_mcp::{CutMcpRouter, McpProfile};
use cut_skills::load_skills;
use runtime::{run_command_surface_smoke, AppRuntime};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{env, thread};
use ui::{AppAction, UiState};
use view_model::AppViewModel;
use viewer::{ViewerAction, ViewerPlaybackAdapter};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalSize, Size};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::Fullscreen;
use winit::window::{Window, WindowAttributes, WindowId};

#[derive(Debug)]
enum AppEvent {
    AccessKit(AccessKitEvent),
    AgentFinished(Result<String, String>),
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
    agent_in_flight: bool,
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
        Ok(Self {
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
            agent_in_flight: false,
        })
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
            if let AppAction::Viewer(viewer_action) = action {
                if viewer_action == ViewerAction::ToggleFullscreen {
                    let fullscreen = self.window.as_ref().and_then(|window| window.fullscreen());
                    if let Some(window) = self.window.as_ref() {
                        if fullscreen.is_some() {
                            window.set_fullscreen(None);
                        } else {
                            window.set_fullscreen(Some(Fullscreen::Borderless(
                                window.current_monitor(),
                            )));
                        }
                    }
                } else if let Err(error) = self
                    .playback
                    .handle_action(viewer_action, &mut self.model.viewer)
                {
                    self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
                continue;
            }

            let result = match action {
                AppAction::Refresh => self.runtime.refresh(),
                AppAction::AddVideoTrack => self.runtime.add_video_track(self.model.revision),
                AppAction::Undo => self.runtime.undo(self.model.revision),
                AppAction::Redo => self.runtime.redo(self.model.revision),
                AppAction::Inspector(inspector::InspectorAction::SetClipVolume {
                    clip_id,
                    volume,
                }) => self
                    .runtime
                    .update_clip_volume(&clip_id, volume, self.model.revision),
                AppAction::AgentPrompt(_) => continue,
                AppAction::Proposal(agent_rail::AgentRailAction::Accept(proposal_id)) => self
                    .runtime
                    .accept_proposal(&proposal_id, self.model.revision),
                AppAction::Proposal(agent_rail::AgentRailAction::Reject(proposal_id)) => self
                    .runtime
                    .reject_proposal(&proposal_id, self.model.revision),
                AppAction::Proposal(agent_rail::AgentRailAction::Send(_)) => continue,
                AppAction::Proposal(agent_rail::AgentRailAction::None) => continue,
                AppAction::Viewer(_) => continue,
            };
            match result {
                Ok(snapshot) => self.sync_snapshot(snapshot, None),
                Err(error) => {
                    self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
                }
            }
        }
    }

    fn sync_snapshot(&mut self, snapshot: cut_runtime::StateSnapshot, status: Option<String>) {
        let playhead = self.model.viewer.playhead;
        let playing = self.model.viewer.playing;
        let volume = self.model.viewer.volume;
        let last_volume = self.model.viewer.last_volume;
        let muted = self.model.viewer.muted;
        let endpoint = self.runtime.endpoint_label();
        self.model = AppViewModel::from_snapshot(&snapshot, &self.config, endpoint);
        self.model.viewer.playhead = playhead.min(self.model.duration.max(0.0));
        self.model.viewer.playing = playing;
        self.model.viewer.volume = volume;
        self.model.viewer.last_volume = last_volume;
        self.model.viewer.muted = muted;
        if let Err(error) = self
            .playback
            .replace_project(&snapshot.project, playhead, playing)
        {
            self.ui_state.status = Some(format!("{}: {}", error.code, error.message));
        } else {
            self.ui_state.status =
                status.or_else(|| Some(format!("Synced revision {}", snapshot.revision)));
        }
    }

    fn start_agent(&mut self, prompt: String) {
        if self.agent_in_flight {
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
        self.agent_in_flight = true;
        self.ui_state.status = Some("Agent is working…".into());
        let directory = self.config.project_directory().to_path_buf();
        let cutd = self.config.cutd_binary().to_path_buf();
        let proxy = self.accesskit_proxy.clone();
        thread::spawn(move || {
            let result = run_native_agent(&directory, &cutd, &config, &prompt);
            let _ = proxy.send_event(AppEvent::AgentFinished(result));
        });
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: cut_runtime::RuntimeError) {
        eprintln!("cut: {}: {}", error.code, error.message);
        self.fatal_error = Some(error);
        event_loop.exit();
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
            if let AppEvent::AgentFinished(result) = event {
                self.agent_in_flight = false;
                match result {
                    Ok(text) => {
                        if let Ok(snapshot) = self.runtime.refresh() {
                            self.sync_snapshot(
                                snapshot,
                                Some(if text.trim().is_empty() {
                                    "Agent completed".into()
                                } else {
                                    format!("Agent: {text}")
                                }),
                            );
                        } else {
                            self.ui_state.status = Some(if text.trim().is_empty() {
                                "Agent completed".into()
                            } else {
                                format!("Agent: {text}")
                            });
                        }
                    }
                    Err(error) => self.ui_state.status = Some(format!("Agent error: {error}")),
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
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

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
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

fn run_native_agent(
    directory: &Path,
    cutd: &Path,
    config: &NativeAgentConfig,
    prompt: &str,
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
    session.run(prompt).map_err(|error| error.to_string())
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
