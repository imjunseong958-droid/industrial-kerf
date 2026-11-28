//! Settings: Kerf's account, what answers in the conversation and its key,
//! what reaches the person's shot models and its key, how the editor looks
//! and moves, its shortcuts, MCP both ways, and about Kerf.
//!
//! A panel over the dimmed window (the workspace, or the launcher with no
//! project, draws the dim and the panel's rise): its pages named down its
//! side, each page rows of a setting's name and what it does, its control
//! at the row's end, a hairline between one and the next. A page chosen
//! fades in over the last. Esc, its close, or a press on the dim gives the
//! window back. With no project (from the launcher) what reaches Kerf
//! through a project, the account and the models listed, says so.

use crate::launcher::{ago, app_icon, field, home_relative};
use editor_session::commands::{self, RunCommand};
use editor_session::session::{BridgeEvent, Session};
use editor_state::prefs::{ApiMode, Keymap, Permissions, Provider, CLAUDE_CODE_MODELS};
use editor_ui::icons::Icon;
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::motion;
use editor_ui::theme::{self, height, radius, text, Face, ReadingFace};
use editor_ui::ui::{self, icon_button};
use gpui::{
    actions, div, img, prelude::*, px, Action, AnimationExt, AnyElement, App, ClipboardItem,
    Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    Keystroke, MouseButton, ScrollHandle, SharedString, Subscription, Task, Window,
};
use kerf_bridge::event::{CloudAccess, CloudStatus, ListedModel};
use kerf_bridge::{About, Command, CrashReports, Event, UpdateCheck};
use kerf_generate::settings::{ProviderSettings, Settings as Generation};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

actions!(settings, [Close]);

/// The key context Settings answers in.
const CONTEXT: &str = "Settings";

/// The panel at its largest: where it goes is the window's to say
/// ([`editor_ui::ui::modal_bounds`]), and it fills that.
pub const MOST: (f32, f32) = (760.0, 540.0);
/// Its title row, and the column of pages down its side.
const HEAD: f32 = 48.0;
const NAV: f32 = 184.0;
/// A field at a row's end: a key or a URL is long.
const FIELD: f32 = 280.0;
/// The sign-in token's field, beside its button.
const TOKEN_FIELD: f32 = 220.0;
/// A page's edges, and the air between a row's words and its control.
const PAGE_SIDE: f32 = 24.0;
const PAGE_TOP: f32 = 20.0;
const ROW_GAP: f32 = 24.0;
/// The MCP servers' editor, and Kerf's own config shown under it.
const SERVERS_HEIGHT: f32 = 160.0;
const CONFIG_HEIGHT: f32 = 132.0;

/// What the project's servers file holds before it is written.
const NO_SERVERS: &str = "{\n  \"mcpServers\": {}\n}\n";

pub enum SettingsEvent {
    Dismiss,
}

/// Settings' pages, down its side in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Account,
    Chat,
    Generation,
    Appearance,
    Shortcuts,
    Mcp,
    About,
}

impl Page {
    pub const ALL: [Page; 7] = [
        Page::Account,
        Page::Chat,
        Page::Generation,
        Page::Appearance,
        Page::Shortcuts,
        Page::Mcp,
        Page::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Account => "Account",
            Page::Chat => "Chat",
            Page::Generation => "Generation",
            Page::Appearance => "Appearance",
            Page::Shortcuts => "Shortcuts",
            Page::Mcp => "MCP",
            Page::About => "About",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Page::Account => Icon::Lock,
            Page::Chat => Icon::Chat,
            Page::Generation => Icon::Sparkle,
            Page::Appearance => Icon::Eye,
            Page::Shortcuts => Icon::Media,
            Page::Mcp => Icon::Gear,
            Page::About => Icon::Film,
        }
    }

    fn index(self) -> usize {
        Page::ALL.iter().position(|page| *page == self).unwrap_or(0)
    }
}

/// The client Kerf's MCP config is shown for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpClient {
    Cursor,
    Claude,
    Codex,
}

impl McpClient {
    pub const ALL: [McpClient; 3] = [McpClient::Cursor, McpClient::Claude, McpClient::Codex];

    pub fn label(self) -> &'static str {
        match self {
            McpClient::Cursor => "Cursor",
            McpClient::Claude => "Claude",
            McpClient::Codex => "Codex",
        }
    }

    /// Where the client keeps it.
    fn place(self) -> &'static str {
        match self {
            McpClient::Cursor => "Goes in Cursor's MCP settings, or the project's .cursor/mcp.json",
            McpClient::Claude => {
                "Goes in Claude Desktop's claude_desktop_config.json, or a project's .mcp.json for Claude Code"
            }
            McpClient::Codex => "Goes in ~/.codex/config.toml",
        }
    }
}

/// Where asking GitHub for a newer Kerf has got to.
enum Update {
    Idle,
    /// The question under way, dropped (and so let go) with the answer.
    Checking {
        _asking: Task<()>,
    },
    Answered(Result<UpdateCheck, String>),
}

pub struct SettingsView {
    session: Entity<Session>,
    focus_handle: FocusHandle,
    page: Page,
    /// The page left, which the chosen one fades in over, and when.
    previous: Option<(Page, Instant)>,
    /// Each page keeps where it was scrolled to.
    scroll: [ScrollHandle; 7],
    model: Entity<TextInput>,
    key: Entity<TextInput>,
    base_url: Entity<TextInput>,
    /// The key as it was loaded or last kept, so only an edited one is saved.
    key_kept: String,
    /// What saving the key came to, and whether it failed.
    key_status: Option<(SharedString, bool)>,
    /// Kerf's sign-in token, while it is typed.
    token: Entity<TextInput>,
    /// A sign-in or out sent, its `cloud` answer awaited.
    signing: bool,
    /// The last `models` event: the person's generation file and the models
    /// its providers reach.
    models: Option<Event>,
    /// Settings ▸ Generation: the generation file as last read, or why it
    /// does not read.
    generation: Result<Generation, String>,
    /// A server's name, address and API key, as typed.
    server_name: Entity<TextInput>,
    server_url: Entity<TextInput>,
    server_key: Entity<TextInput>,
    /// What checking or saving the server came to, and whether it failed.
    server_status: Option<(SharedString, bool)>,
    /// Asking the server which models it reaches, dropped (and so let go)
    /// with the answer.
    checking: Option<Task<()>>,
    /// The project's `.kerf/mcp.json`, as edited here.
    servers: Entity<TextInput>,
    /// What loading or saving the servers came to, and whether it failed.
    servers_status: Option<(SharedString, bool)>,
    client: McpClient,
    /// "Copied" shows on the copy button until this ends.
    copied: Option<Task<()>>,
    /// The registry's commands in the keymap chosen.
    catalog: (Keymap, Vec<commands::Command>),
    update: Update,
    crashes: CrashReports,
    about: About,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_at(session, Page::Chat, window, cx)
    }

    /// Settings open on `page`.
    pub fn new_at(
        session: Entity<Session>,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        bind_close(cx);
        // What another window chose since this one read the settings.
        session.update(cx, |session, _| {
            session.prefs.refresh();
            session.forget_keys();
        });
        let (provider, model_text, base_url_text, keymap, project) = {
            let session = session.read(cx);
            let prefs = &session.prefs;
            (
                prefs.provider,
                prefs.model().to_owned(),
                prefs.base_url().to_owned(),
                prefs.keymap,
                session.project.clone(),
            )
        };
        let key_text = session
            .update(cx, |session, _| session.api_key(provider))
            .unwrap_or_default();
        let model = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Default");
            input.set_text(model_text, cx);
            input
        });
        let key = cx.new(|cx| {
            let mut input = TextInput::new(cx, key_placeholder(provider)).secret();
            input.set_text(key_text.clone(), cx);
            input
        });
        let base_url = cx.new(|cx| {
            let mut input = TextInput::new(cx, base_url_placeholder(provider));
            input.set_text(base_url_text, cx);
            input
        });
        let token = cx.new(|cx| TextInput::new(cx, "Sign-in token").secret());
        let server_name = cx.new(|cx| {
            let mut input = TextInput::new(cx, "pod");
            input.set_text("pod".to_owned(), cx);
            input
        });
        let server_url = cx.new(|cx| TextInput::new(cx, "https://….trycloudflare.com"));
        let server_key = cx.new(|cx| TextInput::new(cx, "kg_…").secret());
        let (servers_text, servers_status) = project_servers(project.as_deref());
        let servers = cx.new(|cx| {
            // JSON, in mono: Return starts a line, ⌘↩ / Ctrl+Enter saves.
            let mut input = TextInput::new(cx, NO_SERVERS.trim())
                .code()
                .with_style(text::TIMECODE)
                .max_height(px(SERVERS_HEIGHT));
            input.set_text(servers_text, cx);
            input
        });
        let mut subscriptions = vec![
            cx.subscribe_in(&model, window, Self::model_event),
            cx.subscribe_in(&key, window, Self::key_event),
            cx.subscribe_in(&base_url, window, Self::base_url_event),
            cx.subscribe_in(&token, window, Self::token_event),
            cx.subscribe_in(&servers, window, Self::servers_event),
            cx.subscribe_in(&server_name, window, Self::server_event),
            cx.subscribe_in(&server_url, window, Self::server_event),
            cx.subscribe_in(&server_key, window, Self::server_event),
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.bridge_event(&event.0, cx)
            }),
        ];
        // A field shows its ring while it is typed in, and what was typed is
        // kept when it is left.
        for input in [&model, &key, &base_url] {
            let handle = input.read(cx).focus_handle(cx);
            subscriptions.push(cx.on_focus(&handle, window, |_, _, cx| cx.notify()));
            subscriptions.push(cx.on_blur(&handle, window, |this, _, cx| {
                this.save_key(cx);
                this.session.read(cx).save_prefs();
                cx.notify();
            }));
        }
        for input in [&token, &servers, &server_name, &server_url, &server_key] {
            let handle = input.read(cx).focus_handle(cx);
            subscriptions.push(cx.on_focus(&handle, window, |_, _, cx| cx.notify()));
            subscriptions.push(cx.on_blur(&handle, window, |_, _, cx| cx.notify()));
        }
        let mut view = Self {
            session,
            focus_handle: cx.focus_handle(),
            page,
            previous: None,
            scroll: Default::default(),
            model,
            key,
            base_url,
            key_kept: key_text.trim().to_owned(),
            key_status: None,
            token,
            signing: false,
            models: None,
            generation: Generation::read(&kerf_generate::Generator::default_file()),
            server_name,
            server_url,
            server_key,
            server_status: None,
            checking: None,
            servers,
            servers_status,
            client: McpClient::Cursor,
            copied: None,
            catalog: (keymap, commands::catalog(keymap.wire())),
            update: Update::Idle,
            crashes: kerf_bridge::crash_reports(),
            about: kerf_bridge::about(),
            _subscriptions: subscriptions,
        };
        view.entered(page, cx);
        view
    }

    /// Keeps what is typed and gives the window back.
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.save_key(cx);
        self.session.read(cx).save_prefs();
        cx.emit(SettingsEvent::Dismiss);
    }

    /// The window's commands wait while Settings is over it; ⌘, closes it
    /// again, and Quit still quits.
    fn command(&mut self, action: &RunCommand, _: &mut Window, cx: &mut Context<Self>) {
        match action.id.as_ref() {
            "app.quit" => cx.propagate(),
            "app.preferences" => self.dismiss(cx),
            "help.about" => self.show_page(Page::About, cx),
            "app.account" => self.show_page(Page::Account, cx),
            _ => {}
        }
    }

    /// Goes to `page`, which fades in over the one left.
    pub fn show_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if page == self.page {
            return;
        }
        self.previous = Some((self.page, Instant::now()));
        self.page = page;
        self.entered(page, cx);
        cx.notify();
    }

    /// What a page asks for as it shows: Kerf's account, the person's
    /// models, the crash reports kept, as they are now.
    fn entered(&mut self, page: Page, cx: &mut Context<Self>) {
        match page {
            Page::Account | Page::Chat => {
                self.session
                    .update(cx, |session, _| session.ask_bridge(Command::CloudRefresh));
            }
            Page::Generation => self.reload_generation(cx),
            Page::About => self.crashes = kerf_bridge::crash_reports(),
            Page::Appearance | Page::Shortcuts | Page::Mcp => {}
        }
    }

    fn bridge_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::Cloud(status) => {
                editor_session::session::hear_cloud(status, cx);
                self.signing = false;
                cx.notify();
            }
            Event::Models { .. } => {
                self.models = Some(event.clone());
                cx.notify();
            }
            _ => {}
        }
    }

    /// A segmented control's or a switch's choice, carried to `this`.
    fn chooser<T, F>(
        cx: &Context<Self>,
        choose: F,
    ) -> impl Fn(T, &mut Window, &mut App) + 'static + use<T, F>
    where
        T: Copy + 'static,
        F: Fn(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    {
        let this = cx.entity().downgrade();
        move |value, window, cx| {
            let _ = this.update(cx, |this, cx| choose(this, value, window, cx));
        }
    }

    // ------------------------------------------------------------ Account

    fn token_event(
        &mut self,
        _: &Entity<TextInput>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {}
            InputEvent::Submit => self.sign_in(cx),
            InputEvent::Cancel => self.dismiss(cx),
        }
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        let token = self.token.read(cx).text().trim().to_owned();
        if token.is_empty() {
            return;
        }
        let sent = self
            .session
            .update(cx, |session, _| session.cloud_sign(Some(&token)));
        if sent {
            self.signing = true;
            self.token.update(cx, |input, cx| input.clear(cx));
        }
        cx.notify();
    }

    fn sign_out(&mut self, cx: &mut Context<Self>) {
        if self
            .session
            .update(cx, |session, _| session.cloud_sign(None))
        {
            self.signing = true;
        }
        cx.notify();
    }

    fn use_kerf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_provider(Provider::Cloud, window, cx);
    }

    fn account_page(&self, window: &Window, cx: &Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        let connected = session.has_project();
        let provider = session.prefs.provider;
        let cloud = editor_session::session::cloud(cx);
        let status = account_status(cloud);
        let words: SharedString = if self.signing {
            "Asking Kerf…".into()
        } else if cloud.is_some() {
            status.words.into()
        } else if connected {
            "Asking Kerf…".into()
        } else {
            "Kerf reaches your account through an open project. Open one to sign in".into()
        };
        let controls = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(theme::space::S8))
            .when(connected && status.sign_in && !self.signing, |controls| {
                controls
                    .child(
                        field(&self.token, window, cx)
                            .w(px(TOKEN_FIELD))
                            .flex_none(),
                    )
                    .child(
                        ui::button("settings-sign-in", "Sign In")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                    )
            })
            .when(connected && status.sign_out && !self.signing, |controls| {
                controls.child(
                    ui::button("settings-sign-out", "Sign Out")
                        .on_click(cx.listener(|this, _, _, cx| this.sign_out(cx))),
                )
            });
        let chat: AnyElement = if provider == Provider::Cloud {
            ui::chip("In use", theme::pos()).into_any_element()
        } else {
            ui::button("settings-use-kerf", "Use Kerf")
                .on_click(cx.listener(|this, _, window, cx| this.use_kerf(window, cx)))
                .into_any_element()
        };
        let chat_hint = if provider == Provider::Cloud {
            "Kerf's hosted model answers in the conversation, on your account".to_owned()
        } else {
            format!(
                "{} answers in the conversation now. Use Kerf to have your account's model answer",
                provider.label()
            )
        };
        vec![group(vec![
            row(
                "Kerf account",
                Some(words),
                Some(controls.into_any_element()),
                Vec::new(),
            ),
            row(
                "Chat on Kerf",
                Some(chat_hint.into()),
                Some(chat),
                Vec::new(),
            ),
        ])
        .into_any_element()]
    }

    // ------------------------------------------------------------ Chat

    fn model_event(
        &mut self,
        input: &Entity<TextInput>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {
                let text = input.read(cx).text().trim().to_owned();
                self.session
                    .update(cx, |session, _| match session.prefs.provider {
                        Provider::Anthropic => session.prefs.anthropic_model = text,
                        Provider::Openai => session.prefs.openai_model = text,
                        Provider::ClaudeCode | Provider::Cloud => {}
                    });
            }
            InputEvent::Submit => self.session.read(cx).save_prefs(),
            InputEvent::Cancel => self.dismiss(cx),
        }
    }

    fn base_url_event(
        &mut self,
        input: &Entity<TextInput>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {
                let text = input.read(cx).text().trim().to_owned();
                self.session
                    .update(cx, |session, _| match session.prefs.provider {
                        Provider::Anthropic => session.prefs.anthropic_base_url = text,
                        Provider::Openai => session.prefs.openai_base_url = text,
                        Provider::ClaudeCode | Provider::Cloud => {}
                    });
            }
            InputEvent::Submit => self.session.read(cx).save_prefs(),
            InputEvent::Cancel => self.dismiss(cx),
        }
    }

    fn key_event(
        &mut self,
        _: &Entity<TextInput>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {}
            InputEvent::Submit => self.save_key(cx),
            InputEvent::Cancel => self.dismiss(cx),
        }
    }

    /// Keeps the key typed in the keychain, when it was edited.
    fn save_key(&mut self, cx: &mut Context<Self>) {
        let provider = self.session.read(cx).prefs.provider;
        if provider.secret_name().is_none() {
            return;
        }
        let typed = self.key.read(cx).text().trim().to_owned();
        if typed == self.key_kept {
            return;
        }
        let saved = self
            .session
            .update(cx, |session, _| session.set_api_key(provider, &typed));
        self.key_status = Some(match saved {
            Ok(kept) => {
                let said = match (typed.is_empty(), kept) {
                    (true, _) => "Removed",
                    (false, editor_session::session::KeyKept::Keychain) => {
                        "Saved to the system keychain"
                    }
                    (false, editor_session::session::KeyKept::SettingsFile) => {
                        "Saved in Kerf's settings file: the system keychain cannot be reached here"
                    }
                };
                self.key_kept = typed;
                (said.into(), false)
            }
            Err(error) => (format!("Not saved: {error}").into(), true),
        });
        cx.notify();
    }

    fn set_provider(&mut self, provider: Provider, _: &mut Window, cx: &mut Context<Self>) {
        self.save_key(cx);
        self.session.update(cx, |session, _| {
            session.prefs.provider = provider;
            session.save_prefs();
        });
        let (model, base_url) = {
            let prefs = &self.session.read(cx).prefs;
            (prefs.model().to_owned(), prefs.base_url().to_owned())
        };
        let key = self
            .session
            .update(cx, |session, _| session.api_key(provider))
            .unwrap_or_default();
        self.key_kept = key.trim().to_owned();
        self.key_status = None;
        self.key.update(cx, |input, cx| {
            input.set_placeholder(key_placeholder(provider), cx);
            input.set_text(key, cx);
        });
        self.model.update(cx, |input, cx| input.set_text(model, cx));
        self.base_url.update(cx, |input, cx| {
            input.set_placeholder(base_url_placeholder(provider), cx);
            input.set_text(base_url, cx);
        });
        if provider == Provider::Cloud {
            self.session
                .update(cx, |session, _| session.ask_bridge(Command::CloudRefresh));
        }
        cx.notify();
    }

    fn set_api_mode(&mut self, mode: ApiMode, _: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.prefs.openai_api_mode = mode;
            session.save_prefs();
        });
        cx.notify();
    }

    fn set_claude_code_model(
        &mut self,
        model: &'static str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session.update(cx, |session, _| {
            session.prefs.claude_code_model = if model == "default" {
                String::new()
            } else {
                model.to_owned()
            };
            session.save_prefs();
        });
        cx.notify();
    }

    fn set_permissions(
        &mut self,
        permissions: Permissions,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session.update(cx, |session, _| {
            session.prefs.permissions = permissions;
            session.save_prefs();
        });
        cx.notify();
    }

    fn chat_page(&self, window: &Window, cx: &Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        let prefs = &session.prefs;
        let provider = prefs.provider;
        let providers = Provider::ALL.map(|provider| (provider, provider.label()));
        let permissions = Permissions::ALL.map(|permissions| (permissions, permissions.label()));
        let mut rows = vec![row(
            "Who answers",
            Some(provider_hint(provider)),
            Some(
                ui::segmented(
                    "settings-provider",
                    &providers,
                    provider,
                    Self::chooser(cx, Self::set_provider),
                )
                .into_any_element(),
            ),
            Vec::new(),
        )];
        let text_row = |label: &'static str, hint: SharedString, input: &Entity<TextInput>| {
            row(
                label,
                Some(hint),
                Some(
                    field(input, window, cx)
                        .w(px(FIELD))
                        .flex_none()
                        .into_any_element(),
                ),
                Vec::new(),
            )
        };
        match provider {
            Provider::Cloud => {
                let words = if session.has_project() || editor_session::session::cloud(cx).is_some()
                {
                    account_status(editor_session::session::cloud(cx)).words
                } else {
                    "Your Kerf account is read with a project open".to_owned()
                };
                rows.push(row(
                    "Kerf account",
                    Some(words.into()),
                    Some(
                        ui::button("settings-account", "Account…")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.show_page(Page::Account, cx)),
                            )
                            .into_any_element(),
                    ),
                    Vec::new(),
                ));
            }
            Provider::Anthropic | Provider::Openai => {
                let env = provider.key_env().unwrap_or_default();
                let status = self.key_status.clone().map(|(said, failed)| {
                    ui::label(
                        said,
                        text::CONTROL,
                        if failed { theme::neg() } else { theme::pos() },
                    )
                    .into_any_element()
                });
                rows.push(row(
                    "API key",
                    Some(
                        format!("Kept in your system keychain; {env} is used when none is kept")
                            .into(),
                    ),
                    Some(
                        field(&self.key, window, cx)
                            .w(px(FIELD))
                            .flex_none()
                            .into_any_element(),
                    ),
                    status.into_iter().collect(),
                ));
                let base_hint = if provider == Provider::Openai {
                    "Any service that speaks the OpenAI API; empty for OpenAI's own"
                } else {
                    "A proxy or gateway that speaks Anthropic's API; empty for Anthropic's own"
                };
                rows.push(text_row("Base URL", base_hint.into(), &self.base_url));
                rows.push(text_row(
                    "Model",
                    "Empty for the service's default".into(),
                    &self.model,
                ));
                if provider == Provider::Openai {
                    let modes = ApiMode::ALL.map(|mode| (mode, mode.label()));
                    rows.push(row(
                        "API shape",
                        Some("What the service serves: /chat/completions or /responses".into()),
                        Some(
                            ui::segmented(
                                "settings-api-mode",
                                &modes,
                                prefs.openai_api_mode,
                                Self::chooser(cx, Self::set_api_mode),
                            )
                            .into_any_element(),
                        ),
                        Vec::new(),
                    ));
                }
            }
            Provider::ClaudeCode => {
                rows.push(row(
                    "API key",
                    Some(
                        "Claude Code keeps its own sign-in, so Kerf needs no key for it. Sign in once by running claude in a terminal"
                            .into(),
                    ),
                    Some(ui::chip("Not needed", theme::ink2()).into_any_element()),
                    Vec::new(),
                ));
                let models = CLAUDE_CODE_MODELS.map(|model| (model, claude_code_label(model)));
                let chosen = CLAUDE_CODE_MODELS
                    .into_iter()
                    .find(|model| *model == prefs.claude_code_model)
                    .unwrap_or("default");
                rows.push(row(
                    "Model",
                    Some("Default leaves the choice to Claude Code".into()),
                    Some(
                        ui::segmented(
                            "settings-claude-code-model",
                            &models,
                            chosen,
                            Self::chooser(cx, Self::set_claude_code_model),
                        )
                        .into_any_element(),
                    ),
                    Vec::new(),
                ));
            }
        }
        rows.push(row(
            "Permissions",
            Some(permission_hint(prefs.permissions).into()),
            Some(
                ui::segmented(
                    "settings-permissions",
                    &permissions,
                    prefs.permissions,
                    Self::chooser(cx, Self::set_permissions),
                )
                // Each mode's tip says what it lets Kerf do, before it is
                // chosen.
                .tips(&Permissions::ALL.map(permission_hint))
                .into_any_element(),
            ),
            Vec::new(),
        ));
        vec![group(rows).into_any_element()]
    }

    // ------------------------------------------------------------ Generation

    /// The generation file read again, and, with a project open, the models
    /// its providers reach.
    fn reload_generation(&mut self, cx: &mut Context<Self>) {
        self.generation = Generation::read(&kerf_generate::Generator::default_file());
        self.session
            .update(cx, |session, _| session.ask_bridge(Command::Models));
        cx.notify();
    }

    fn server_event(
        &mut self,
        _: &Entity<TextInput>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {}
            InputEvent::Submit => self.save_server(cx),
            InputEvent::Cancel => self.dismiss(cx),
        }
    }

    /// The server as typed: its name, its address as a provider holds it,
    /// and its key (none where none is typed); or why it cannot be one.
    fn server_typed(&self, cx: &Context<Self>) -> Result<(String, String, Option<String>), String> {
        let name = self.server_name.read(cx).text().trim().to_owned();
        if !kerf_generate::settings::names_a_provider(&name) {
            return Err("A server's name is letters, digits, ., - and _".to_owned());
        }
        let url = kerf_generate::settings::endpoint_url(self.server_url.read(cx).text())?;
        let key =
            Some(self.server_key.read(cx).text().trim().to_owned()).filter(|key| !key.is_empty());
        Ok((name, url, key))
    }

    /// Asks the server typed which models it reaches, with the key typed, or
    /// the one kept for it where none is: off the window's thread, as it may
    /// take a few seconds.
    fn check_server(&mut self, cx: &mut Context<Self>) {
        let (name, url, key) = match self.server_typed(cx) {
            Ok(typed) => typed,
            Err(why) => {
                self.server_status = Some((why.into(), true));
                cx.notify();
                return;
            }
        };
        let key = key.or_else(|| {
            self.session
                .read(cx)
                .kept_secret(&kerf_generate::secret_name(&name))
        });
        let asked = cx
            .background_executor()
            .spawn(async move { kerf_generate::ask_models(&url, key.as_deref()) });
        self.checking = Some(cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                this.server_status = Some(match answer {
                    Ok(models) => (connected_words(&models).into(), false),
                    Err(why) => (why.into(), true),
                });
                this.checking = None;
                cx.notify();
            });
        }));
        self.server_status = Some(("Checking…".into(), false));
        cx.notify();
    }

    /// Writes the server typed into the generation file, in place of one by
    /// its name, and keeps its key where a chat's is kept.
    fn save_server(&mut self, cx: &mut Context<Self>) {
        let (name, url, key) = match self.server_typed(cx) {
            Ok(typed) => typed,
            Err(why) => {
                self.server_status = Some((why.into(), true));
                cx.notify();
                return;
            }
        };
        let file = kerf_generate::Generator::default_file();
        let written = Generation::read(&file).and_then(|mut settings| {
            settings.set_endpoint(&name, &url)?;
            settings
                .write(&file)
                .map_err(|error| format!("{} could not be written: {error}", home_relative(&file)))
        });
        if let Err(why) = written {
            self.server_status = Some((why.into(), true));
            cx.notify();
            return;
        }
        let kept = match key {
            Some(key) => self
                .session
                .update(cx, |session, _| {
                    session.keep_secret(&kerf_generate::secret_name(&name), &key)
                })
                .map(Some),
            None => Ok(None),
        };
        self.server_status = Some(match kept {
            Ok(Some(editor_session::session::KeyKept::Keychain)) => {
                (format!("Saved `{name}`, its key in the system keychain").into(), false)
            }
            Ok(Some(editor_session::session::KeyKept::SettingsFile)) => (
                format!("Saved `{name}`, its key in Kerf's settings file: the system keychain cannot be reached here").into(),
                false,
            ),
            Ok(None) => (format!("Saved `{name}`").into(), false),
            Err(error) => (format!("Saved `{name}`, and not its key: {error}").into(), true),
        });
        self.server_key
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.reload_generation(cx);
    }

    /// Puts server `name` in the fields, to be changed.
    fn edit_server(&mut self, name: &str, url: &str, cx: &mut Context<Self>) {
        self.server_name
            .update(cx, |input, cx| input.set_text(name.to_owned(), cx));
        self.server_url
            .update(cx, |input, cx| input.set_text(url.to_owned(), cx));
        self.server_key
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.server_status = None;
        cx.notify();
    }

    /// Takes server `name` out of the generation file, and forgets its key.
    fn remove_server(&mut self, name: &str, cx: &mut Context<Self>) {
        let file = kerf_generate::Generator::default_file();
        let written = Generation::read(&file).and_then(|mut settings| {
            settings.remove(name);
            settings
                .write(&file)
                .map_err(|error| format!("{} could not be written: {error}", home_relative(&file)))
        });
        self.server_status = Some(match written {
            Ok(()) => {
                let forgot = self.session.update(cx, |session, _| {
                    session.keep_secret(&kerf_generate::secret_name(name), "")
                });
                match forgot {
                    Ok(_) => (format!("Removed `{name}` and its key").into(), false),
                    Err(error) => (
                        format!("Removed `{name}`, and not its key: {error}").into(),
                        true,
                    ),
                }
            }
            Err(why) => (why.into(), true),
        });
        self.reload_generation(cx);
    }

    fn generation_page(&self, window: &Window, cx: &Context<Self>) -> Vec<AnyElement> {
        let connected = self.session.read(cx).has_project();
        let file = kerf_generate::Generator::default_file();
        let exists = file.is_file();
        // The providers the file sets up, each with what reaches it; a server
        // can be put in the fields to change, or taken out.
        let mut providers: Vec<AnyElement> = Vec::new();
        match &self.generation {
            Err(why) => providers.push(
                ui::label(
                    format!("The generation file does not read: {why}"),
                    text::CONTROL,
                    theme::neg(),
                )
                .into_any_element(),
            ),
            Ok(settings) if settings.providers().next().is_none() => providers.push(
                ui::label(
                    "Nothing is set up yet: add a server below",
                    text::CONTROL,
                    theme::ink3(),
                )
                .into_any_element(),
            ),
            Ok(settings) => {
                for (index, (name, provider)) in settings.providers().enumerate() {
                    let (kind, what) = match provider {
                        ProviderSettings::Endpoint { url } => ("Server", url.clone()),
                        ProviderSettings::Command { run, .. } => ("Program", run.join(" ")),
                    };
                    let mut line = div()
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S8))
                        .py(px(theme::space::S4))
                        .child(ui::label(name.to_owned(), text::BODY_NAME, theme::ink()))
                        .child(ui::chip(kind, theme::ink2()))
                        .child(div().flex_1().min_w_0().child(ui::label(
                            what.clone(),
                            text::MONO_SMALL,
                            theme::ink3(),
                        )));
                    if let ProviderSettings::Endpoint { url } = provider {
                        let (edited, url) = (name.to_owned(), url.clone());
                        let removed = name.to_owned();
                        line = line
                            .child(
                                ui::button(("settings-generation-edit", index), "Edit")
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.edit_server(&edited, &url, cx)
                                    })),
                            )
                            .child(
                                ui::button(("settings-generation-remove", index), "Remove")
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_server(&removed, cx)
                                    })),
                            );
                    }
                    providers.push(line.into_any_element());
                }
            }
        }
        providers.push(
            ui::label(
                format!(
                    "{}{}",
                    home_relative(&file),
                    if exists { "" } else { " (not there yet)" }
                ),
                text::MONO_SMALL,
                theme::ink3(),
            )
            .into_any_element(),
        );
        let (open_words, _) = generation_opener(&file, exists);
        let shown = file.clone();
        let buttons = div()
            .flex()
            .flex_none()
            .gap(px(theme::space::S8))
            .child(
                // The file itself, in the app that opens it, to be edited;
                // not there yet, its folder (made), to put it in.
                ui::button("settings-generation-open", open_words).on_click(move |_, _, cx| {
                    let (_, opened) = generation_opener(&shown, shown.is_file());
                    if !shown.is_file()
                        && let Err(error) = std::fs::create_dir_all(&opened)
                    {
                        log::warn!(
                            "{}: the folder could not be made: {error}",
                            opened.display()
                        );
                    }
                    cx.open_with_system(&opened);
                }),
            )
            .child(
                ui::button("settings-generation-reload", "Reload")
                    .on_click(cx.listener(|this, _, _, cx| this.reload_generation(cx))),
            );
        let text_row = |label: &'static str, hint: &'static str, input: &Entity<TextInput>| {
            row(
                label,
                Some(hint.into()),
                Some(
                    field(input, window, cx)
                        .w(px(FIELD))
                        .flex_none()
                        .into_any_element(),
                ),
                Vec::new(),
            )
        };
        let status = self.server_status.clone().map(|(said, failed)| {
            ui::label(
                said,
                text::CONTROL,
                if failed { theme::neg() } else { theme::pos() },
            )
            .into_any_element()
        });
        let server_buttons = div()
            .flex()
            .flex_none()
            .gap(px(theme::space::S8))
            .child(
                ui::button("settings-generation-check", "Check Connection")
                    .disabled(self.checking.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.check_server(cx))),
            )
            .child(
                ui::button("settings-generation-save", "Save")
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.save_server(cx))),
            );
        vec![group(vec![
            row(
                "Providers",
                Some("What reaches your shot models, from your generation file".into()),
                Some(buttons.into_any_element()),
                providers,
            ),
            text_row("Server name", "What a step names its models by where two servers have one", &self.server_name),
            text_row("Server URL", "A server that speaks the Kerf generation API", &self.server_url),
            text_row(
                "API key",
                "Kept in your system keychain; KERF_GENERATION_KEY is used when none is kept",
                &self.server_key,
            ),
            row(
                "Connection",
                Some("Asks the server which models it reaches, with the key typed or the one kept".into()),
                Some(server_buttons.into_any_element()),
                status.into_iter().collect(),
            ),
            row(
                "Models",
                Some(if connected {
                    "Video and image models your steps can name".into()
                } else {
                    "Your steps' models are listed with a project open".into()
                }),
                None,
                self.models_listed(connected),
            ),
            row(
                "Programs",
                Some("A program on this computer is set up in the file: {\"<name>\": {\"run\": [\"<program>\", \"{request}\"], \"takes\": {…}}} under providers".into()),
                None,
                Vec::new(),
            ),
            row(
                "Built in",
                Some("Scene cuts, speech, tracking and cut-outs run on this computer, with no setup".into()),
                None,
                Vec::new(),
            ),
        ])
        .into_any_element()]
    }

    /// The models of the last `models` event, each in a line, or why there
    /// are none.
    fn models_listed(&self, connected: bool) -> Vec<AnyElement> {
        let heard = match &self.models {
            Some(Event::Models { models, error, .. }) => {
                Some((models.as_slice(), error.as_deref()))
            }
            _ => None,
        };
        let listed: &[ListedModel] = heard.map_or(&[], |(models, _)| models);
        let note = match heard {
            _ if !connected => None,
            None => Some(("Reading…".to_owned(), false)),
            Some((_, Some(error))) => {
                Some((format!("The generation file does not read: {error}"), true))
            }
            Some(([], None)) => Some(("No model is reached yet".to_owned(), false)),
            Some(_) => None,
        };
        let mut below: Vec<AnyElement> = Vec::new();
        if let Some((note, failed)) = note {
            below.push(
                ui::label(
                    note,
                    text::CONTROL,
                    if failed { theme::neg() } else { theme::ink3() },
                )
                .into_any_element(),
            );
        }
        below.extend(listed.iter().map(|model| {
            div()
                .flex()
                .flex_col()
                .gap(px(theme::space::S2))
                .py(px(theme::space::S4))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(theme::space::S8))
                        .child(ui::label(model.name.clone(), text::BODY_NAME, theme::ink()))
                        .child(ui::label(
                            model_summary(model),
                            text::CONTROL,
                            theme::ink2(),
                        )),
                )
                .when_some(
                    model.about.as_deref().filter(|about| !about.is_empty()),
                    |column, about| {
                        column.child(ui::label(about.to_owned(), text::CONTROL, theme::ink3()))
                    },
                )
                .when_some(
                    model.guide.as_deref().filter(|guide| !guide.is_empty()),
                    |column, guide| {
                        column.child(ui::label(
                            format!("Guide: {guide}"),
                            text::MONO_SMALL,
                            theme::ink3(),
                        ))
                    },
                )
                .into_any_element()
        }));
        below
    }

    // ------------------------------------------------------------ MCP

    fn set_mcp(&mut self, on: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.prefs.mcp_enabled = on;
            session.save_prefs();
        });
        let said = if on {
            "On from the next request"
        } else {
            "Off from the next request"
        };
        self.servers_status = Some((said.into(), false));
        cx.notify();
    }

    fn servers_event(
        &mut self,
        _: &Entity<TextInput>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {}
            // Return starts a new line in the field itself; ⌘↩ / Ctrl+Enter
            // is Save and Apply.
            InputEvent::Submit => self.save_servers(cx),
            InputEvent::Cancel => self.dismiss(cx),
        }
    }

    /// Checks the servers typed and writes them to the project's
    /// `.kerf/mcp.json`; the next request takes them.
    fn save_servers(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.session.read(cx).project.clone() else {
            return;
        };
        let text = self.servers.read(cx).text().to_owned();
        let saved =
            validate_mcp_config(&text).and_then(|()| write_project_servers(&project, &text));
        self.servers_status = Some(match saved {
            Ok(()) => {
                self.session.update(cx, |session, _| {
                    session.prefs.mcp_config_version += 1;
                    session.save_prefs();
                });
                let said = if self.session.read(cx).prefs.mcp_enabled {
                    "Saved the project's .kerf/mcp.json. It applies from the next request"
                } else {
                    "Saved the project's .kerf/mcp.json. It applies once External MCP tools is on"
                };
                (said.into(), false)
            }
            Err(error) => (error.into(), true),
        });
        cx.notify();
    }

    /// Puts the file at `path` in the editor; `imported` when it is another
    /// client's, which is saved only by Save and Apply.
    fn load_servers(&mut self, path: &Path, imported: bool, cx: &mut Context<Self>) {
        let shown = home_relative(path);
        self.servers_status = Some(match std::fs::read_to_string(path) {
            Err(error) => (read_failure(&shown, &error).into(), true),
            Ok(text) => {
                let checked = validate_mcp_config(&text);
                self.servers
                    .update(cx, |input, cx| input.set_text(text, cx));
                match (checked, imported) {
                    (Err(error), _) => (format!("Loaded {shown}. {error}").into(), true),
                    (Ok(()), true) => (
                        format!(
                            "Loaded {shown} into the editor only. Review it, then Save and Apply"
                        )
                        .into(),
                        false,
                    ),
                    (Ok(()), false) => (
                        "Loaded the project's servers. Changes apply from the next request".into(),
                        false,
                    ),
                }
            }
        });
        cx.notify();
    }

    fn copy_config(&mut self, config: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(config));
        // It says so for a moment, however motion is set.
        self.copied = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(theme::motion::DONE_HOLD)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn set_client(&mut self, client: McpClient, _: &mut Window, cx: &mut Context<Self>) {
        self.client = client;
        cx.notify();
    }

    fn mcp_page(&self, window: &Window, cx: &Context<Self>) -> Vec<AnyElement> {
        let session = self.session.read(cx);
        let prefs = &session.prefs;
        let project = session.project.clone();
        let switch_row = row(
            "External MCP tools",
            Some("Lets Kerf chat call local MCP servers, over stdio".into()),
            Some(
                ui::switch(
                    "settings-mcp",
                    prefs.mcp_enabled,
                    Self::chooser(cx, Self::set_mcp),
                )
                .into_any_element(),
            ),
            Vec::new(),
        );
        let servers_row = match &project {
            Some(project) => {
                let file = project.join(".kerf").join("mcp.json");
                let home = editor_state::prefs::home();
                let imports = cursor_imports(project, home.as_deref());
                let status = self.servers_status.clone().map(|(said, failed)| {
                    ui::label(
                        said,
                        text::CONTROL,
                        if failed { theme::neg() } else { theme::ink3() },
                    )
                    .into_any_element()
                });
                let reload = file.clone();
                let buttons = div()
                    .flex()
                    .flex_wrap()
                    .gap(px(theme::space::S8))
                    .child(
                        ui::button("settings-mcp-save", "Save and Apply")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.save_servers(cx))),
                    )
                    .child(
                        ui::button("settings-mcp-load", "Load from Project")
                            .disabled(!file.is_file())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.load_servers(&reload, false, cx)
                            })),
                    )
                    // Cursor's, the project's and the person's, always
                    // offered; each one's tip is its file, so it is plain
                    // which file loads.
                    .children(imports.into_iter().enumerate().map(|(at, (words, path))| {
                        ui::button(("settings-mcp-cursor", at), words)
                            .tooltip(home_relative(&path))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.load_servers(&path, true, cx)
                            }))
                    }));
                let mut below = vec![
                    code_well(&self.servers, SERVERS_HEIGHT, window, cx).into_any_element(),
                    buttons.into_any_element(),
                ];
                below.extend(status);
                row(
                    "Servers",
                    Some(
                        "This project's .kerf/mcp.json, then ~/.kerf/mcp.json. They may hold credentials"
                            .into(),
                    ),
                    None,
                    below,
                )
            }
            None => row(
                "Servers",
                Some("A project's .kerf/mcp.json, then ~/.kerf/mcp.json. Open a project to edit its servers here".into()),
                None,
                Vec::new(),
            ),
        };
        let folder = project
            .clone()
            .unwrap_or_else(|| PathBuf::from("/path/to/project"));
        let (command, args) = mcp_server_command(&folder);
        let config = mcp_server_config(self.client, &command, &args);
        let clients = McpClient::ALL.map(|client| (client, client.label()));
        let copied = self.copied.is_some();
        let copy = config.clone();
        let place = if project.is_some() {
            self.client.place().to_owned()
        } else {
            format!(
                "{}. Open a project to fill in its folder",
                self.client.place()
            )
        };
        let server_row = row(
            "Use Kerf from Cursor, Claude or Codex",
            Some("Add this to the client's MCP settings".into()),
            Some(
                ui::button(
                    "settings-mcp-copy",
                    if copied { "Copied" } else { "Copy Config" },
                )
                .on_click(cx.listener(move |this, _, _, cx| this.copy_config(copy.clone(), cx)))
                .into_any_element(),
            ),
            vec![
                div()
                    .flex()
                    .child(ui::segmented(
                        "settings-mcp-client",
                        &clients,
                        self.client,
                        Self::chooser(cx, Self::set_client),
                    ))
                    .into_any_element(),
                config_well(config).into_any_element(),
                ui::label(place, text::CONTROL, theme::ink3()).into_any_element(),
            ],
        );
        vec![
            titled("Tools in Kerf chat", group(vec![switch_row, servers_row])),
            titled("Kerf as an MCP server", group(vec![server_row])),
        ]
    }

    // ------------------------------------------------------------ Appearance

    /// Puts the faces and the conversation's size this window's settings
    /// hold in force.
    fn apply_faces(&self, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = &self.session.read(cx).prefs;
        theme::set_faces(prefs.face, prefs.reading_face, prefs.prose_step);
        window.refresh();
    }

    fn set_face(&mut self, face: Face, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.prefs.choose_face(face);
            session.save_prefs();
        });
        self.apply_faces(window, cx);
    }

    fn set_reading_face(&mut self, face: ReadingFace, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.prefs.choose_reading_face(face);
            session.save_prefs();
        });
        self.apply_faces(window, cx);
    }

    fn step_prose(&mut self, by: i32, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.prefs.prose_step =
                (session.prefs.prose_step + by).clamp(PROSE_LEAST, PROSE_MOST);
            session.save_prefs();
        });
        self.apply_faces(window, cx);
    }

    fn set_reduce_motion(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.prefs.reduce_motion = on;
            session.save_prefs();
        });
        motion::set_reduced(on);
        window.refresh();
    }

    fn appearance_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let prefs = &self.session.read(cx).prefs;
        let (chosen, reading) = prefs.faces();
        let step = prefs.prose_step;
        let readings = ReadingFace::ALL.map(|face| (face, face.label()));
        let faces = div().flex().gap(px(theme::space::S8)).children(
            Face::INTERFACE.iter().enumerate().map(|(index, face)| {
                let face = *face;
                let on = face == chosen;
                let choose = Self::chooser(cx, Self::set_face);
                div()
                    .id(ElementId::named_usize("settings-face", index))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S4))
                    .p(px(theme::space::S12))
                    .rounded(px(radius::CARD))
                    .border_1()
                    .border_color(if on { theme::accent() } else { theme::line2() })
                    .bg(if on {
                        theme::accent_wash()
                    } else {
                        theme::well()
                    })
                    .cursor_pointer()
                    .when(!on, |tile| {
                        tile.hover(|style| style.border_color(theme::lineh()))
                    })
                    .child(
                        in_face(div(), face.ui(), text::DISPLAY)
                            .text_color(theme::ink())
                            .child("Aa 가"),
                    )
                    .child(
                        in_face(div(), face.ui(), text::CONTROL)
                            .text_color(if on { theme::ink() } else { theme::ink2() })
                            .whitespace_nowrap()
                            .child(face.label()),
                    )
                    .on_click(move |_, window, cx| choose(face, window, cx))
            }),
        );
        let size = div()
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .child(
                ui::button("settings-smaller", "−")
                    .small()
                    .disabled(step <= PROSE_LEAST)
                    .on_click(cx.listener(|this, _, window, cx| this.step_prose(-1, window, cx))),
            )
            .child(
                ui::label(prose_step_label(step), text::CONTROL_STRONG, theme::ink())
                    .w(px(56.0))
                    .flex()
                    .justify_center(),
            )
            .child(
                ui::button("settings-larger", "+")
                    .small()
                    .disabled(step >= PROSE_MOST)
                    .on_click(cx.listener(|this, _, window, cx| this.step_prose(1, window, cx))),
            );
        let sample = theme::reading(div())
            .flex()
            .flex_col()
            .gap(px(theme::space::S4))
            .p(px(theme::space::S12))
            .rounded(px(radius::CARD))
            .bg(theme::card())
            .border_1()
            .border_color(theme::line())
            .text_color(theme::ink())
            .child("Make a 30-second cut from the best takes, and open on the wide shot.")
            .child("가장 좋은 테이크로 30초 컷을 만들고, 와이드 숏으로 시작해 줘.");
        vec![group(vec![
            row(
                "Interface font",
                Some("IBM Plex and Inter bring a Korean face of their own".into()),
                None,
                vec![faces.into_any_element()],
            ),
            row(
                "Conversation font",
                Some("For what you ask and what Kerf answers".into()),
                Some(
                    ui::segmented(
                        "settings-reading-face",
                        &readings,
                        reading,
                        Self::chooser(cx, Self::set_reading_face),
                    )
                    .into_any_element(),
                ),
                Vec::new(),
            ),
            row(
                "Conversation text size",
                None,
                Some(size.into_any_element()),
                vec![sample.into_any_element()],
            ),
            row(
                "Reduce Motion",
                Some("Panes, pages and lights are simply there, with no motion".into()),
                Some(
                    ui::switch(
                        "settings-reduce-motion",
                        prefs.reduce_motion,
                        Self::chooser(cx, Self::set_reduce_motion),
                    )
                    .into_any_element(),
                ),
                Vec::new(),
            ),
        ])
        .into_any_element()]
    }

    // ------------------------------------------------------------ Shortcuts

    fn set_keymap(&mut self, keymap: Keymap, _: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.prefs.keymap = keymap;
            session.save_prefs();
        });
        // Binding the keymap clears every binding; it binds the views' own again.
        commands::rebind(keymap.wire(), cx);
        self.catalog = (keymap, commands::catalog(keymap.wire()));
        cx.notify();
    }

    fn shortcuts_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let (keymap, catalog) = &self.catalog;
        let keymaps = Keymap::ALL.map(|keymap| (keymap, keymap.label()));
        let mut elements = vec![group(vec![row(
            "Keymap",
            Some("Whose shortcuts the editor answers to".into()),
            Some(
                ui::segmented(
                    "settings-keymap",
                    &keymaps,
                    *keymap,
                    Self::chooser(cx, Self::set_keymap),
                )
                .into_any_element(),
            ),
            Vec::new(),
        )])
        .into_any_element()];
        // The commands with a shortcut, by menu, in the registry's order.
        let mut menus: Vec<(&str, Vec<&commands::Command>)> = Vec::new();
        for command in catalog.iter().filter(|command| !command.keys.is_empty()) {
            match menus.iter_mut().find(|(menu, _)| *menu == command.menu) {
                Some((_, commands)) => commands.push(command),
                None => menus.push((&command.menu, vec![command])),
            }
        }
        if menus.is_empty() {
            elements.push(
                ui::label("No shortcuts in this keymap", text::BODY, theme::ink3())
                    .pt(px(theme::space::S16))
                    .into_any_element(),
            );
        }
        for (menu, commands) in menus {
            let name = if menu.is_empty() {
                "Other".to_owned()
            } else {
                capitalized(menu)
            };
            elements.push(
                div()
                    .flex()
                    .flex_col()
                    .pt(px(theme::space::S16))
                    .child(
                        ui::label(name, text::META_STRONG, theme::ink3()).pb(px(theme::space::S4)),
                    )
                    .children(commands.into_iter().map(|command| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::space::S12))
                            .min_h(px(height::H28))
                            .border_t_1()
                            .border_color(theme::line())
                            .child(
                                ui::line(command.label.clone(), text::BODY, theme::ink())
                                    .flex_1()
                                    .min_w_0(),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .gap(px(theme::space::S6))
                                    .children(command.keys.iter().map(|keys| keycaps(keys))),
                            )
                    }))
                    .into_any_element(),
            );
        }
        elements
    }

    // ------------------------------------------------------------ About

    fn set_check_updates(&mut self, on: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.prefs.check_updates = on;
            session.save_prefs();
        });
        cx.notify();
    }

    /// Asks GitHub for a newer Kerf, off the window's thread: it may take
    /// up to ten seconds.
    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if matches!(self.update, Update::Checking { .. }) {
            return;
        }
        let asked = cx
            .background_executor()
            .spawn(async { kerf_bridge::update_check() });
        let task = cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                this.update = Update::Answered(answer);
                cx.notify();
            });
        });
        self.update = Update::Checking { _asking: task };
        cx.notify();
    }

    fn about_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let prefs = &self.session.read(cx).prefs;
        let About {
            name,
            version,
            license,
            repository,
        } = self.about;
        let repository = repository.to_owned();
        let link = repository.clone();
        let releases = releases_url(&repository);
        let releases_link = releases.clone();
        let head = div()
            .flex()
            .items_center()
            .gap(px(theme::space::S16))
            .pt(px(theme::space::S8))
            .pb(px(theme::space::S16))
            .child(img(app_icon()).size(px(64.0)).flex_none())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(ui::label(name, text::TITLE, theme::ink()))
                    .child(ui::label(TAGLINE, text::CONTROL, theme::ink2()))
                    .child(ui::label(
                        format!("Version {version}"),
                        text::CONTROL,
                        theme::ink2(),
                    ))
                    .when(!license.is_empty(), |column| {
                        column.child(ui::label(
                            format!("Licensed {license}"),
                            text::CONTROL,
                            theme::ink3(),
                        ))
                    })
                    .child(
                        ui::line(
                            repository.trim_start_matches("https://").to_owned(),
                            text::CONTROL,
                            theme::accent_ink(),
                        )
                        .id("settings-repository")
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(move |_, _, cx| cx.open_url(&link)),
                    )
                    // What changed in each release, the page itself in
                    // its tip.
                    .child(ui::tip_on(
                        ui::line(
                            "Releases and what changed in each",
                            text::CONTROL,
                            theme::accent_ink(),
                        )
                        .id("settings-releases")
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(move |_, _, cx| cx.open_url(&releases_link)),
                        Some(ui::Tip::new(releases)),
                    )),
            );

        let checking = matches!(self.update, Update::Checking { .. });
        let answer: Option<AnyElement> = match &self.update {
            Update::Idle => None,
            Update::Checking { .. } => {
                Some(ui::label("Asking GitHub…", text::CONTROL, theme::ink3()).into_any_element())
            }
            Update::Answered(answer) => Some(update_answer(answer)),
        };
        let reports = &self.crashes.reports;
        let folder = self.crashes.folder.clone();
        let crash_hint: SharedString = match (reports.len(), &folder) {
            (_, None) => "There is nowhere to keep them on this computer".into(),
            (0, Some(folder)) => {
                format!("None so far. They are kept in {}", home_relative(folder)).into()
            }
            (1, Some(folder)) => format!("1 report in {}", home_relative(folder)).into(),
            (count, Some(folder)) => format!("{count} reports in {}", home_relative(folder)).into(),
        };
        let shown_folder = folder.clone();
        let newest: Vec<AnyElement> = reports
            .iter()
            .take(3)
            .map(|report| {
                let component = capitalized(component_name(&report.component));
                let when = ago(std::time::UNIX_EPOCH
                    + Duration::from_millis(u64::try_from(report.at_ms).unwrap_or(u64::MAX)));
                let message = report.message.lines().next().unwrap_or_default().to_owned();
                div()
                    .flex()
                    .flex_col()
                    .child(ui::line(
                        format!("{component} · {when}"),
                        text::META_STRONG,
                        theme::ink2(),
                    ))
                    .child(ui::line(message, text::MONO_SMALL, theme::ink3()))
                    .into_any_element()
            })
            .collect();
        vec![
            head.into_any_element(),
            group(vec![
                row(
                    "Update notices",
                    Some("At launch, at most once a day, asks GitHub for a newer Kerf. Never downloads or installs".into()),
                    Some(
                        ui::switch(
                            "settings-updates",
                            prefs.check_updates,
                            Self::chooser(cx, Self::set_check_updates),
                        )
                        .into_any_element(),
                    ),
                    Vec::new(),
                ),
                row(
                    "Check for updates",
                    Some("Asks GitHub for the newest release now".into()),
                    Some(
                        ui::button(
                            "settings-check-now",
                            if checking { "Checking…" } else { "Check Now" },
                        )
                        .disabled(checking)
                        .on_click(cx.listener(|this, _, _, cx| this.check_for_updates(cx)))
                        .into_any_element(),
                    ),
                    answer.into_iter().collect(),
                ),
                row(
                    "Crash reports",
                    Some(crash_hint),
                    Some(
                        ui::button("settings-crash-folder", "Show Folder")
                            .disabled(shown_folder.is_none())
                            .on_click(move |_, _, cx| {
                                if let Some(folder) = &shown_folder {
                                    if let Err(error) = std::fs::create_dir_all(folder) {
                                        log::warn!(
                                            "{}: the crash folder could not be made: {error}",
                                            folder.display()
                                        );
                                    }
                                    cx.reveal_path(folder);
                                }
                            })
                            .into_any_element(),
                    ),
                    newest,
                ),
            ])
            .into_any_element(),
        ]
    }

    // ------------------------------------------------------------ drawing

    fn head(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        div()
            .h(px(HEAD))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(theme::space::S16))
            .pr(px(theme::space::S12))
            .child(ui::label("Settings", text::SIDE_NAME, theme::ink()))
            .child(div().flex_1())
            .child(
                icon_button("settings-close", Icon::Close)
                    .tooltip("Close")
                    .key("Esc")
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
            )
    }

    fn nav(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        div()
            .w(px(NAV))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(theme::space::S2))
            .px(px(theme::space::S8))
            .py(px(theme::space::S12))
            .border_r_1()
            .border_color(theme::line())
            .children(Page::ALL.iter().map(|page| {
                let page = *page;
                let on = page == self.page;
                let ink = if on { theme::ink() } else { theme::ink2() };
                div()
                    .id(ElementId::named_usize("settings-nav", page.index()))
                    // A Tab stop: its ring while it has the keys, Space or
                    // Enter shows its page; a press of the pointer leaves
                    // the keys where they were.
                    .tab_index(0)
                    .key_context(ui::CONTROL)
                    .on_action(|_: &ui::FocusNext, window, cx| ui::tab_to(true, window, cx))
                    .on_action(|_: &ui::FocusPrevious, window, cx| ui::tab_to(false, window, cx))
                    .capture_any_mouse_down(|_, window, _| window.prevent_default())
                    .border(px(theme::layout::FOCUS_RING))
                    .border_color(theme::with_alpha(theme::accent(), 0.0))
                    .focus(|row| row.border_color(theme::accent()))
                    .h(px(height::H32))
                    .px(px(theme::space::S8))
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .rounded(px(radius::CONTROL))
                    .cursor_pointer()
                    .when(on, |row| row.bg(theme::on()))
                    .when(!on, |row| row.hover(|style| style.bg(theme::raised())))
                    .child(page.icon().view(16.0, ink))
                    .child(ui::label(
                        page.label(),
                        if on { text::BODY_NAME } else { text::BODY },
                        ink,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.show_page(page, cx)))
            }))
    }

    fn page_body(
        &self,
        page: Page,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let content = match page {
            Page::Account => self.account_page(window, cx),
            Page::Chat => self.chat_page(window, cx),
            Page::Generation => self.generation_page(window, cx),
            Page::Appearance => self.appearance_page(cx),
            Page::Shortcuts => self.shortcuts_page(cx),
            Page::Mcp => self.mcp_page(window, cx),
            Page::About => self.about_page(cx),
        };
        div()
            .id(ElementId::named_usize("settings-scroll", page.index()))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll[page.index()])
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px(px(PAGE_SIDE))
                    .pt(px(PAGE_TOP))
                    .pb(px(PAGE_SIDE))
                    .child(
                        ui::label(page.label(), text::SIDE_NAME, theme::ink())
                            .pb(px(theme::space::S4)),
                    )
                    .children(content),
            )
    }

    fn pages(&mut self, window: &Window, cx: &Context<Self>) -> impl IntoElement + use<> {
        // The page left stays under the chosen one while that fades in.
        let fading = self
            .previous
            .filter(|(_, since)| since.elapsed() < motion::duration(theme::motion::APPEAR))
            .map(|(page, _)| page);
        if fading.is_none() {
            self.previous = None;
        }
        let page = self.page;
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .when_some(fading, |area, previous| {
                area.child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(self.page_body(previous, window, cx)),
                )
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(theme::chrome())
                    .child(self.page_body(page, window, cx))
                    .with_animation(
                        ElementId::named_usize("settings-page", page.index()),
                        motion::animation(theme::motion::APPEAR),
                        |page, t| page.opacity(t),
                    ),
            )
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pages = self.pages(window, cx);
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, _, cx| this.dismiss(cx)))
            .on_action(cx.listener(Self::command))
            // A press on the dim around the panel gives the window back, and
            // does nothing to what lies under the dim.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.dismiss(cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::chrome())
            .font(theme::ui_font(FontWeight::NORMAL))
            .text_color(theme::ink())
            .child(self.head(cx))
            .child(ui::hairline(true))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.nav(cx))
                    .child(pages),
            )
    }
}

// ---------------------------------------------------------------- pieces

/// What the conversation's text may grow or shrink by, in points.
const PROSE_LEAST: i32 = -2;
const PROSE_MOST: i32 = 6;

/// One setting: its name and what it does, its control at the row's end,
/// and anything wide under it.
fn row(
    label: impl Into<SharedString>,
    hint: Option<SharedString>,
    control: Option<AnyElement>,
    below: Vec<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::space::S8))
        .py(px(theme::space::S12))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(ROW_GAP))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(theme::space::S2))
                        .child(ui::label(label, text::BODY_NAME, theme::ink()))
                        .when_some(hint, |words, hint| {
                            words.child(ui::label(hint, text::CONTROL, theme::ink3()))
                        }),
                )
                .children(control),
        )
        .children(below)
        .into_any_element()
}

/// Rows with a hairline between one and the next.
fn group(rows: Vec<AnyElement>) -> gpui::Div {
    let mut column = div().flex().flex_col();
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            column = column.child(ui::hairline(true));
        }
        column = column.child(row);
    }
    column
}

/// `element`'s type in `style`, set in `family` as the editor sets it
/// (its Hangul partner, tabular figures).
fn in_face<E: Styled>(element: E, family: &'static str, style: theme::TypeStyle) -> E {
    element
        .font(theme::font(family, style.weight, false))
        .text_size(px(style.px))
        .line_height(px(style.line))
}

/// A shortcut ("Ctrl+Shift+Z", "Ctrl+K, Ctrl+S") as keycaps, one a chord.
fn keycaps(shortcut: &str) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap(px(theme::space::S4))
        .children(shortcut.split(", ").map(|chord| {
            theme::type_style(div(), text::MONO_SMALL)
                .flex()
                .items_center()
                .h(px(height::H20))
                .px(px(theme::space::S6))
                .rounded(px(radius::SMALL))
                .bg(theme::raised())
                .border_1()
                .border_color(theme::line2())
                .text_color(theme::ink2())
                .whitespace_nowrap()
                .child(editor_state::shortcuts::shown_keys(chord))
        }))
}

/// `file` → `File`, as a menu's name.
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn key_placeholder(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "sk-ant-…",
        Provider::Openai => "sk-…",
        Provider::ClaudeCode | Provider::Cloud => "",
    }
}

fn base_url_placeholder(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "https://api.anthropic.com/v1",
        Provider::Openai => "https://api.openai.com/v1",
        Provider::ClaudeCode | Provider::Cloud => "",
    }
}

/// Claude Code's model as the choice names it.
fn claude_code_label(model: &str) -> &'static str {
    match model {
        "opus" => "Opus",
        "sonnet" => "Sonnet",
        "haiku" => "Haiku",
        _ => "Default",
    }
}

fn provider_hint(provider: Provider) -> SharedString {
    match provider {
        Provider::Cloud => "Kerf's hosted model, on your Kerf account: no key of your own".into(),
        Provider::Anthropic => "Claude, through Anthropic's API, with your own key".into(),
        Provider::Openai => "OpenAI, or any service that speaks its API, with your own key".into(),
        Provider::ClaudeCode => {
            "Uses your installed Claude Code, signed in on its own, with this project's Kerf tools"
                .into()
        }
    }
}

fn permission_hint(permissions: Permissions) -> &'static str {
    match permissions {
        Permissions::Ask => "Every edit is a proposal you accept or reject",
        Permissions::AcceptEdits => "Edits apply as Kerf makes them, and Undo takes each back. Shots still ask",
        Permissions::Auto => {
            "Edits apply, and Kerf makes the shots it asks for, five before it asks again. Shots can cost time and money"
        }
    }
}

fn prose_step_label(step: i32) -> String {
    match step {
        0 => "Default".to_owned(),
        step if step > 0 => format!("+{step}"),
        step => format!("−{}", -step),
    }
}

/// What GitHub said of the newest release.
fn update_answer(answer: &Result<UpdateCheck, String>) -> AnyElement {
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            return ui::label(
                format!("Could not check: {error}"),
                text::CONTROL,
                theme::neg(),
            )
            .into_any_element()
        }
    };
    let current = &answer.current;
    match (
        answer.newer,
        answer.latest.as_deref(),
        answer.url.as_deref(),
    ) {
        (true, Some(latest), Some(url)) => {
            let url = url.to_owned();
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .child(ui::label(
                    format!("Industrial Kerf {latest} is out; this is {current}"),
                    text::CONTROL,
                    theme::warn(),
                ))
                .child(
                    ui::button("settings-download", "Download")
                        .small()
                        .primary()
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .into_any_element()
        }
        _ => ui::label(
            format!("This is the newest Kerf ({current})"),
            text::CONTROL,
            theme::pos(),
        )
        .into_any_element(),
    }
}

/// Whether `action` is bound to Esc.
pub fn escape_bound(cx: &App, action: &dyn Action) -> bool {
    let Ok(escape) = Keystroke::parse("escape") else {
        return false;
    };
    cx.all_bindings_for_input(&[escape])
        .iter()
        .any(|binding| binding.action().partial_eq(action))
}

/// Esc closes Settings, before the window's own Esc (Reject Proposal) has
/// it: Settings' context is the deeper one. Binding a keymap clears this,
/// so it is bound again whenever it is missing.
pub fn bind_close(cx: &mut App) {
    if !escape_bound(cx, &Close) {
        cx.bind_keys([KeyBinding::new("escape", Close, Some(CONTEXT))]);
    }
}

/// A group of rows under its name.
fn titled(title: &'static str, rows: gpui::Div) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .pt(px(theme::space::S16))
        .child(ui::label(title, text::META_STRONG, theme::ink3()).pb(px(theme::space::S4)))
        .child(rows)
        .into_any_element()
}

/// A field of several lines in its well, `height` tall at least, ringed in
/// the accent while typed in. A press anywhere in the well types in it.
fn code_well(input: &Entity<TextInput>, height: f32, window: &Window, cx: &App) -> gpui::Div {
    let handle = input.read(cx).focus_handle(cx);
    let focused = handle.is_focused(window);
    div()
        .w_full()
        .min_h(px(height))
        .p(px(theme::space::S8))
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(if focused {
            theme::accent()
        } else {
            theme::line2()
        })
        .cursor_text()
        .on_mouse_down(MouseButton::Left, move |_, window, _| {
            window.focus(&handle);
            // Settings' own focus would take it back.
            window.prevent_default();
        })
        .child(input.clone())
}

/// Words to read and copy, in mono, in a well.
fn config_well(config: String) -> gpui::Div {
    div()
        .w_full()
        .min_h(px(CONFIG_HEIGHT))
        .p(px(theme::space::S8))
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(theme::line())
        .flex()
        .flex_col()
        .children(config.lines().map(|line| {
            theme::type_style(div(), text::MONO_SMALL)
                .text_color(theme::ink2())
                .whitespace_nowrap()
                .child(if line.is_empty() {
                    " ".to_owned()
                } else {
                    line.to_owned()
                })
        }))
}

/// Where Kerf's account stands, in words, and what is offered: signing in
/// (a token field and Sign In) or out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountStatus {
    pub words: String,
    pub sign_in: bool,
    pub sign_out: bool,
}

/// What the last `cloud` event says of the account; None before one is
/// heard.
pub fn account_status(cloud: Option<&CloudStatus>) -> AccountStatus {
    let access = cloud.map(|cloud| cloud.access);
    let account = cloud.and_then(|cloud| cloud.account.as_ref());
    let email = account.map_or("", |account| account.email.as_str());
    let available = access.is_some_and(|access| access != CloudAccess::Unavailable);
    let signed_in = available && !email.is_empty() && access != Some(CloudAccess::SignedOut);
    let mut words = if !available {
        "Not available yet. Chat can run on your own API key or Claude Code".to_owned()
    } else if !signed_in {
        if access == Some(CloudAccess::Unreachable) {
            "Kerf can't be reached right now".to_owned()
        } else {
            "Signed out. Paste your Kerf sign-in token to sign in".to_owned()
        }
    } else {
        let plan = account
            .map(|account| account.plan.as_str())
            .filter(|plan| !plan.is_empty())
            .unwrap_or("Kerf");
        let mut words = format!("Signed in as {email} · {plan} plan");
        if let Some(credits) = account.and_then(|account| account.credits) {
            words.push_str(&format!(
                " · {credits} request{} left",
                if credits == 1 { "" } else { "s" }
            ));
        }
        match access {
            Some(CloudAccess::NotIncluded) => {
                words.push_str("\nThis plan doesn't include requests to Kerf")
            }
            Some(CloudAccess::OutOfCredits) => {
                words.push_str("\nYou've used this period's requests")
            }
            _ => {}
        }
        words
    };
    if let Some(message) = cloud
        .and_then(|cloud| cloud.message.as_deref())
        .filter(|message| !message.trim().is_empty())
    {
        words.push('\n');
        words.push_str(message.trim());
    }
    AccountStatus {
        words,
        sign_in: available && !signed_in,
        sign_out: signed_in,
    }
}

/// A model of the models event in a line: "832x480 · 16 fps · up to 5 s ·
/// reads guide, ref".
pub fn model_summary(model: &ListedModel) -> String {
    let number = |value: f64| {
        let text = format!("{value:.3}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    let mut said = Vec::new();
    if let Some(size) = model.size.as_deref().filter(|size| !size.is_empty()) {
        said.push(size.to_owned());
    }
    if let Some(rate) = model.rate.filter(|rate| *rate > 0.0) {
        said.push(format!("{} fps", number(rate)));
    }
    if let Some(seconds) = model.seconds.filter(|seconds| *seconds > 0.0) {
        said.push(format!("up to {} s", number(seconds)));
    }
    let reads = model.reads.as_deref().unwrap_or_default();
    if !reads.is_empty() {
        said.push(format!("reads {}", reads.join(", ")));
    }
    if said.is_empty() {
        "says nothing of what it takes".to_owned()
    } else {
        said.join(" · ")
    }
}

/// What Check Connection says of a server that answered: how many models
/// it reaches, and their names.
pub fn connected_words(models: &[kerf_generate::api::NamedModel]) -> String {
    match models {
        [] => "Connected: it reaches no model yet".to_owned(),
        [model] => format!("Connected: it reaches {}", model.name),
        models => format!(
            "Connected: it reaches {} models, {}",
            models.len(),
            models
                .iter()
                .map(|model| model.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The project's `.kerf/mcp.json` to edit, and what loading it came to.
fn project_servers(project: Option<&Path>) -> (String, Option<(SharedString, bool)>) {
    let Some(file) = project.map(|project| project.join(".kerf").join("mcp.json")) else {
        return (NO_SERVERS.to_owned(), None);
    };
    match std::fs::read_to_string(&file) {
        Ok(text) => {
            let status = match validate_mcp_config(&text) {
                Ok(()) => (
                    "Loaded the project's servers. Changes apply from the next request".into(),
                    false,
                ),
                Err(error) => (error.into(), true),
            };
            (text, Some(status))
        }
        Err(_) => (
            NO_SERVERS.to_owned(),
            Some(("No project servers yet".into(), false)),
        ),
    }
}

/// Writes `text` as the project's `.kerf/mcp.json`, through a temporary
/// file.
fn write_project_servers(project: &Path, text: &str) -> Result<(), String> {
    let folder = project.join(".kerf");
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("Could not make the project's .kerf folder: {error}"))?;
    let file = folder.join("mcp.json");
    let temporary = folder.join("mcp.json.tmp");
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, &file))
        .map_err(|error| format!("Could not save the servers: {error}"))
}

/// Settings ▸ Generation's button for the generation file at `file`: its
/// words and what it opens in the system's app for it. The file itself while
/// it is there ("Open generation.json"); else its folder, which the press
/// makes ("Open Its Folder"), to put the file in.
pub fn generation_opener(file: &Path, exists: bool) -> (&'static str, PathBuf) {
    match file.parent().filter(|_| !exists) {
        Some(folder) => ("Open Its Folder", folder.to_path_buf()),
        None => ("Open generation.json", file.to_path_buf()),
    }
}

/// Cursor's MCP servers files that Settings ▸ MCP imports from, each
/// with its button's words: the project's `.cursor/mcp.json`, then the
/// person's `~/.cursor/mcp.json`. Both are offered whether there or not;
/// one that is not says so as it is loaded ([`read_failure`]).
pub fn cursor_imports(project: &Path, home: Option<&Path>) -> Vec<(&'static str, PathBuf)> {
    let mut imports = vec![(
        "Load from Project's Cursor",
        project.join(".cursor").join("mcp.json"),
    )];
    if let Some(home) = home {
        imports.push((
            "Load from Home Cursor",
            home.join(".cursor").join("mcp.json"),
        ));
    }
    imports
}

/// "Could not read <path>: <why>", for a servers file that did not open,
/// `why` as the system says it without its number.
pub fn read_failure(shown: &str, error: &std::io::Error) -> String {
    let said = error.to_string();
    let why = match said.rfind(" (os error ") {
        Some(at) if said.ends_with(')') => &said[..at],
        _ => said.as_str(),
    };
    format!("Could not read {shown}: {why}")
}

/// About's line under Kerf's name.
pub const TAGLINE: &str = "A video editor you can talk to.";

/// Where Kerf's releases are listed, and what changed in each: the
/// repository's `/releases` page.
pub fn releases_url(repository: &str) -> String {
    format!("{}/releases", repository.trim().trim_end_matches('/'))
}

/// Which part of Kerf a crash report is of, as a person says it.
pub fn component_name(component: &str) -> &'static str {
    match component {
        "kerfd" => "Kerf's project service (kerfd)",
        "kerf" => "the kerf command",
        _ => "the editor",
    }
}

/// Whether `text` is an MCP servers file Kerf chat can use: an object whose
/// `mcpServers` names servers, each run by a local command, with `args`
/// and `tools` lists of words and `env` words by name. The error says what
/// to mend.
pub fn validate_mcp_config(text: &str) -> Result<(), String> {
    let document: Value =
        serde_json::from_str(text).map_err(|error| format!("Invalid JSON: {error}"))?;
    let Some(document) = document.as_object() else {
        return Err("The JSON root must be an object.".into());
    };
    let Some(servers) = document.get("mcpServers").and_then(Value::as_object) else {
        return Err("mcpServers must be an object.".into());
    };
    for server in servers.values() {
        let Some(server) = server.as_object() else {
            return Err("Each server must be an object.".into());
        };
        if !server
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| !command.trim().is_empty())
        {
            return Err("Each server needs a nonempty local command string. URL-only remote servers are not supported in Kerf chat.".into());
        }
        for field in ["args", "tools"] {
            match server.get(field) {
                None => {}
                Some(Value::Null) if field == "tools" => {}
                Some(Value::Array(items)) => {
                    if !items.iter().all(Value::is_string) {
                        return Err(format!("{field} must contain only strings."));
                    }
                }
                Some(_) => return Err(format!("{field} must be an array of strings.")),
            }
        }
        match server.get("env") {
            None => {}
            Some(Value::Object(env)) => {
                if !env.values().all(Value::is_string) {
                    return Err("env must contain only string values.".into());
                }
            }
            Some(_) => return Err("env must be an object of string values.".into()),
        }
    }
    Ok(())
}

/// What another client runs to reach Kerf's tools on `project`: kerf-mcp
/// (`KERF_MCP_BIN`, beside the editor, else the checkout's build) with
/// `--dir`, else the editor itself serving them (`--serve-mcp`).
pub fn mcp_server_command(project: &Path) -> (String, Vec<String>) {
    let project = project.display().to_string();
    let name = if cfg!(windows) {
        "kerf-mcp.exe"
    } else {
        "kerf-mcp"
    };
    let exe = std::env::current_exe().ok();
    let found = std::env::var_os("KERF_MCP_BIN")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let beside = exe
                .as_ref()
                .and_then(|exe| exe.parent().map(|folder| folder.join(name)));
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
            let built =
                ["debug", "release"].map(|profile| root.join("target").join(profile).join(name));
            beside
                .into_iter()
                .chain(built)
                .find(|path| path.is_file())
                .map(|path| kerf_paths::canonicalize(&path).unwrap_or(path))
        });
    match (found, exe) {
        (Some(mcp), _) => (mcp.display().to_string(), vec!["--dir".into(), project]),
        (None, Some(exe)) => (
            exe.display().to_string(),
            vec!["--serve-mcp".into(), "--dir".into(), project],
        ),
        (None, None) => (name.to_owned(), vec!["--dir".into(), project]),
    }
}

/// Kerf's MCP server as `client` writes it: JSON `mcpServers` for Cursor
/// and Claude, a TOML table for Codex.
pub fn mcp_server_config(client: McpClient, command: &str, args: &[String]) -> String {
    match client {
        McpClient::Cursor | McpClient::Claude => {
            let config = json!({"mcpServers": {"kerf": {"command": command, "args": args}}});
            serde_json::to_string_pretty(&config).unwrap_or_default()
        }
        McpClient::Codex => {
            // A JSON string is a TOML basic string.
            let quoted = |text: &str| serde_json::to_string(text).unwrap_or_default();
            let args: Vec<String> = args.iter().map(|arg| quoted(arg)).collect();
            format!(
                "[mcp_servers.kerf]\ncommand = {}\nargs = [{}]",
                quoted(command),
                args.join(", ")
            )
        }
    }
}
