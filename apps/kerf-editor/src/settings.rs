//! Settings: Kerf's account, what answers in the conversation and its key,
//! the person's shot models, how the editor looks and moves, its shortcuts,
//! MCP both ways, and about Kerf.
//!
//! A panel over the dimmed window (the workspace, or the launcher with no
//! project, draws the dim and the panel's rise): its pages named down its
//! side, each page rows of a setting's name and what it does, its control
//! at the row's end, a hairline between one and the next. A page chosen
//! fades in over the last. Esc, its close, or a press on the dim gives the
//! window back. With no project (from the launcher) what reaches Kerf
//! through a project, the account and the models, says so.

use crate::commands::{self, RunCommand};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::launcher::{ago, app_icon, field, home_relative};
use crate::motion;
use crate::prefs::{ApiMode, Keymap, Permissions, Provider, CLAUDE_CODE_MODELS};
use crate::session::{BridgeEvent, Session};
use crate::theme::{self, height, radius, text, Face};
use crate::ui::{self, icon_button};
use gpui::{
    actions, div, img, prelude::*, px, Action, AnimationExt, AnyElement, App, ClipboardItem,
    Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    Keystroke, MouseButton, ScrollHandle, SharedString, Subscription, Task, Window,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

actions!(settings, [Close]);

/// The key context Settings answers in.
const CONTEXT: &str = "Settings";

/// The panel's size; the workspace rounds its corners.
const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 540.0;
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
    Models,
    Appearance,
    Shortcuts,
    Mcp,
    About,
}

impl Page {
    pub const ALL: [Page; 7] = [
        Page::Account,
        Page::Chat,
        Page::Models,
        Page::Appearance,
        Page::Shortcuts,
        Page::Mcp,
        Page::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Account => "Account",
            Page::Chat => "Chat",
            Page::Models => "Models",
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
            Page::Models => Icon::Sparkle,
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
    Answered(Value),
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
    /// The last `models` event: the person's models file and what it names.
    models: Option<Value>,
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
    crashes: Value,
    about: Value,
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
        for input in [&token, &servers] {
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
                    .update(cx, |session, _| session.ask_bridge("cloud_refresh"));
            }
            Page::Models => {
                self.session
                    .update(cx, |session, _| session.ask_bridge("models"));
            }
            Page::About => self.crashes = kerf_bridge::crash_reports(),
            Page::Appearance | Page::Shortcuts | Page::Mcp => {}
        }
    }

    fn bridge_event(&mut self, event: &Value, cx: &mut Context<Self>) {
        match event["type"].as_str() {
            Some("cloud") => {
                crate::app::hear_cloud(event, cx);
                self.signing = false;
                cx.notify();
            }
            Some("models") => {
                self.models = Some(event.clone());
                cx.notify();
            }
            _ => {}
        }
    }

    /// A segmented control's or a switch's choice, carried to `this`.
    fn chooser<T, F>(cx: &Context<Self>, choose: F) -> impl Fn(T, &mut Window, &mut App) + 'static
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
        let cloud = crate::app::cloud(cx);
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
                    (false, crate::session::KeyKept::Keychain) => "Saved to the system keychain",
                    (false, crate::session::KeyKept::SettingsFile) => {
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
                .update(cx, |session, _| session.ask_bridge("cloud_refresh"));
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
                let words = if session.has_project() || crate::app::cloud(cx).is_some() {
                    account_status(crate::app::cloud(cx)).words
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
                .into_any_element(),
            ),
            Vec::new(),
        ));
        vec![group(rows).into_any_element()]
    }

    // ------------------------------------------------------------ Models

    fn reload_models(&mut self, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, _| session.ask_bridge("models"));
    }

    fn models_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let connected = self.session.read(cx).has_project();
        let event = self.models.as_ref();
        let file = event
            .and_then(|event| event["file"].as_str())
            .filter(|file| !file.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(models_file);
        let exists = event
            .and_then(|event| event["exists"].as_bool())
            .unwrap_or_else(|| file.is_file());
        let listed: Vec<Value> = event
            .and_then(|event| event["models"].as_array())
            .cloned()
            .unwrap_or_default();
        let error = event.and_then(|event| event["error"].as_str());
        let mut below: Vec<AnyElement> = Vec::new();
        let note: Option<(String, bool)> = if let Some(error) = error {
            Some((format!("The models file does not read: {error}"), true))
        } else if !listed.is_empty() {
            None
        } else if !connected {
            Some(("Models are read with a project open".to_owned(), false))
        } else if event.is_none() {
            Some(("Reading…".to_owned(), false))
        } else if exists {
            Some(("Your models file names no model yet".to_owned(), false))
        } else {
            Some((
                "No model is set up: there is no models file yet".to_owned(),
                false,
            ))
        };
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
            let name = model["name"].as_str().unwrap_or("model").to_owned();
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
                        .child(ui::label(name, text::BODY_NAME, theme::ink()))
                        .child(ui::label(
                            model_summary(model),
                            text::CONTROL,
                            theme::ink2(),
                        )),
                )
                .when_some(
                    model["about"].as_str().filter(|about| !about.is_empty()),
                    |column, about| {
                        column.child(ui::label(about.to_owned(), text::CONTROL, theme::ink3()))
                    },
                )
                .when_some(
                    model["guide"].as_str().filter(|guide| !guide.is_empty()),
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
        below.push(
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
        let shown = file.clone();
        let buttons = div()
            .flex()
            .flex_none()
            .gap(px(theme::space::S8))
            .child(
                ui::button("settings-models-show", "Show in Folder").on_click(move |_, _, cx| {
                    if shown.is_file() {
                        cx.reveal_path(&shown);
                    } else if let Some(folder) = shown.parent() {
                        // Not there yet: its folder, to put it in.
                        let _ = std::fs::create_dir_all(folder);
                        cx.reveal_path(folder);
                    }
                }),
            )
            .child(
                ui::button("settings-models-reload", "Reload")
                    .disabled(!connected)
                    .on_click(cx.listener(|this, _, _, cx| this.reload_models(cx))),
            );
        vec![group(vec![
            row(
                "Shot models",
                Some("Video and image models your recipes can name, from your models file".into()),
                Some(buttons.into_any_element()),
                below,
            ),
            row(
                "File format",
                Some("Each entry names a program to run with {request} and, under takes, the size, rate, longest shot and files (guide, first, last, ref) its model accepts".into()),
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
            Err(error) => (format!("Could not read {shown}: {error}").into(), true),
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
                let cursor = cursor_config(Some(project));
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
                    .when_some(cursor, |buttons, cursor| {
                        buttons.child(
                            ui::button("settings-mcp-cursor", "Import from Cursor").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.load_servers(&cursor, true, cx)
                                }),
                            ),
                        )
                    });
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

    fn set_face(&mut self, face: Face, window: &mut Window, cx: &mut Context<Self>) {
        let step = self.session.update(cx, |session, _| {
            session.prefs.face = face;
            session.save_prefs();
            session.prefs.prose_step
        });
        theme::set_face(face, step);
        window.refresh();
    }

    fn step_prose(&mut self, by: i32, window: &mut Window, cx: &mut Context<Self>) {
        let (face, step) = self.session.update(cx, |session, _| {
            session.prefs.prose_step =
                (session.prefs.prose_step + by).clamp(PROSE_LEAST, PROSE_MOST);
            session.save_prefs();
            (session.prefs.face, session.prefs.prose_step)
        });
        theme::set_face(face, step);
        window.refresh();
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
        let chosen = prefs.face;
        let step = prefs.prose_step;
        let faces =
            div()
                .flex()
                .gap(px(theme::space::S8))
                .children(Face::ALL.iter().enumerate().map(|(index, face)| {
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
                            in_face(div(), face, text::DISPLAY)
                                .text_color(theme::ink())
                                .child("Aa 가"),
                        )
                        .child(
                            in_face(div(), face, text::CONTROL)
                                .text_color(if on { theme::ink() } else { theme::ink2() })
                                .whitespace_nowrap()
                                .child(face.label()),
                        )
                        .on_click(move |_, window, cx| choose(face, window, cx))
                }));
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
                "Font",
                Some("IBM Plex and Inter bring a Korean face of their own. Serif sets the conversation in Noto Serif".into()),
                None,
                vec![faces.into_any_element()],
            ),
            row(
                "Conversation text size",
                Some("For what you ask and what Kerf answers".into()),
                Some(size.into_any_element()),
                vec![sample.into_any_element()],
            ),
            row(
                "Reduce Motion",
                Some("Panes, pages and lights are simply there, with no motion".into()),
                Some(ui::switch("settings-reduce-motion", prefs.reduce_motion, Self::chooser(cx, Self::set_reduce_motion)).into_any_element()),
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
        commands::bind(keymap.wire(), cx);
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
        let about = &self.about;
        let name = about["name"]
            .as_str()
            .unwrap_or("Industrial Kerf")
            .to_owned();
        let version = about["version"].as_str().unwrap_or_default().to_owned();
        let license = about["license"].as_str().unwrap_or_default().to_owned();
        let repository = about["repository"]
            .as_str()
            .unwrap_or(kerf_bridge::REPOSITORY)
            .to_owned();
        let link = repository.clone();
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
                    ),
            );

        let checking = matches!(self.update, Update::Checking { .. });
        let answer: Option<AnyElement> = match &self.update {
            Update::Idle => None,
            Update::Checking { .. } => {
                Some(ui::label("Asking GitHub…", text::CONTROL, theme::ink3()).into_any_element())
            }
            Update::Answered(answer) => Some(update_answer(answer)),
        };
        let reports = self.crashes["reports"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let folder = self.crashes["folder"].as_str().map(PathBuf::from);
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
                let component = capitalized(crate::app::component_name(
                    report["component"].as_str().unwrap_or("editor"),
                ));
                let when = report["atMs"]
                    .as_f64()
                    .map(
                        |ms| ago(std::time::UNIX_EPOCH + Duration::from_millis(ms.max(0.0) as u64)),
                    )
                    .unwrap_or_default();
                let message = report["message"]
                    .as_str()
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
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
                                    let _ = std::fs::create_dir_all(folder);
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

    fn head(&self, cx: &Context<Self>) -> impl IntoElement {
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
                    .tooltip("Close · Esc")
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
            )
    }

    fn nav(&self, cx: &Context<Self>) -> impl IntoElement {
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

    fn page_body(&self, page: Page, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let content = match page {
            Page::Account => self.account_page(window, cx),
            Page::Chat => self.chat_page(window, cx),
            Page::Models => self.models_page(cx),
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

    fn pages(&mut self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
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
            .w(px(WIDTH))
            .h(px(HEIGHT))
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
                    .h(px(HEIGHT - HEAD - 1.0))
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

/// `element`'s type in `style`, set in `face`'s family for reading.
fn in_face<E: Styled>(element: E, face: Face, style: theme::TypeStyle) -> E {
    let family = face.prose();
    let hangul = if family == theme::SERIF_FAMILY {
        theme::SERIF_HANGUL_FAMILY
    } else {
        theme::PLEX_HANGUL_FAMILY
    };
    element
        .font(gpui::Font {
            family: family.into(),
            features: gpui::FontFeatures::default(),
            fallbacks: Some(gpui::FontFallbacks::from_fonts(vec![
                hangul.to_owned(),
                theme::PRETENDARD_FAMILY.to_owned(),
            ])),
            weight: style.weight,
            style: gpui::FontStyle::Normal,
        })
        .text_size(px(style.px))
        .line_height(px(style.line))
}

/// A shortcut ("Ctrl+Shift+Z", "Ctrl+K, Ctrl+S") as keycaps, one a chord.
fn keycaps(shortcut: &str) -> impl IntoElement {
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
                .child(commands::shown_keys(chord))
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
fn update_answer(answer: &Value) -> AnyElement {
    if let Some(error) = answer["error"].as_str() {
        return ui::label(
            format!("Could not check: {error}"),
            text::CONTROL,
            theme::neg(),
        )
        .into_any_element();
    }
    let current = answer["current"].as_str().unwrap_or_default();
    match (
        answer["newer"].as_bool(),
        answer["latest"].as_str(),
        answer["url"].as_str(),
    ) {
        (Some(true), Some(latest), Some(url)) => {
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
pub(crate) fn escape_bound(cx: &App, action: &dyn Action) -> bool {
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
pub(crate) fn bind_close(cx: &mut App) {
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

/// What the last `cloud {access, account?, message?}` event says of the
/// account; None before one is heard.
pub fn account_status(cloud: Option<&Value>) -> AccountStatus {
    let access = cloud
        .and_then(|cloud| cloud["access"].as_str())
        .unwrap_or("");
    let account = cloud.map(|cloud| &cloud["account"]);
    let email = account
        .and_then(|account| account["email"].as_str())
        .unwrap_or("");
    let available = !access.is_empty() && access != "unavailable";
    let signed_in = available && !email.is_empty() && access != "signedOut";
    let mut words = if !available {
        "Not available yet. Chat can run on your own API key or Claude Code".to_owned()
    } else if !signed_in {
        if access == "unreachable" {
            "Kerf can't be reached right now".to_owned()
        } else {
            "Signed out. Paste your Kerf sign-in token to sign in".to_owned()
        }
    } else {
        let plan = account
            .and_then(|account| account["plan"].as_str())
            .filter(|plan| !plan.is_empty())
            .unwrap_or("Kerf");
        let mut words = format!("Signed in as {email} · {plan} plan");
        if let Some(credits) = account.and_then(|account| account["credits"].as_u64()) {
            words.push_str(&format!(
                " · {credits} request{} left",
                if credits == 1 { "" } else { "s" }
            ));
        }
        match access {
            "notIncluded" => words.push_str("\nThis plan doesn't include requests to Kerf"),
            "outOfCredits" => words.push_str("\nYou've used this period's requests"),
            _ => {}
        }
        words
    };
    if let Some(message) = cloud
        .and_then(|cloud| cloud["message"].as_str())
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
pub fn model_summary(model: &Value) -> String {
    let number = |value: f64| {
        let text = format!("{value:.3}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    let mut said = Vec::new();
    if let Some(size) = model["size"].as_str().filter(|size| !size.is_empty()) {
        said.push(size.to_owned());
    }
    if let Some(rate) = model["rate"].as_f64().filter(|rate| *rate > 0.0) {
        said.push(format!("{} fps", number(rate)));
    }
    if let Some(seconds) = model["seconds"].as_f64().filter(|seconds| *seconds > 0.0) {
        said.push(format!("up to {} s", number(seconds)));
    }
    let reads: Vec<&str> = model["reads"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !reads.is_empty() {
        said.push(format!("reads {}", reads.join(", ")));
    }
    if said.is_empty() {
        "says nothing of what it takes".to_owned()
    } else {
        said.join(" · ")
    }
}

/// The person's models file, as kerf-engine finds it: `KERF_MODELS`, else
/// `~/.kerf/models.json`.
pub fn models_file() -> PathBuf {
    std::env::var_os("KERF_MODELS")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::prefs::home().map(|home| home.join(".kerf").join("models.json")))
        .unwrap_or_else(|| PathBuf::from("models.json"))
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

/// Cursor's MCP servers to import: the project's `.cursor/mcp.json`, else
/// the person's `~/.cursor/mcp.json`, whichever is there.
pub fn cursor_config(project: Option<&Path>) -> Option<PathBuf> {
    let in_project = project.map(|project| project.join(".cursor").join("mcp.json"));
    let at_home = crate::prefs::home().map(|home| home.join(".cursor").join("mcp.json"));
    in_project
        .into_iter()
        .chain(at_home)
        .find(|path| path.is_file())
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
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let built =
                ["debug", "release"].map(|profile| root.join("target").join(profile).join(name));
            beside
                .into_iter()
                .chain(built)
                .find(|path| path.is_file())
                .map(|path| path.canonicalize().unwrap_or(path))
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
