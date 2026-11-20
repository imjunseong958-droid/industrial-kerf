//! Settings: what answers in the conversation, its key, how the editor
//! looks and moves, its shortcuts, and about Kerf.
//!
//! A panel over the dimmed window (the workspace draws the dim and the
//! panel's rise): its pages named down its side, each page rows of a
//! setting's name and what it does, its control at the row's end, a
//! hairline between one and the next. A page chosen fades in over the last.
//! Esc, its close, or a press on the dim gives the window back.

use crate::commands::{self, RunCommand};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::launcher::{ago, app_icon, field, home_relative};
use crate::motion;
use crate::prefs::{Keymap, Permissions, Provider};
use crate::session::Session;
use crate::theme::{self, height, radius, text, Face};
use crate::ui::{self, icon_button};
use gpui::{
    actions, div, img, prelude::*, px, Action, AnimationExt, AnyElement, App, Context, ElementId,
    Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, Keystroke, ScrollHandle,
    SharedString, Subscription, Task, Window,
};
use serde_json::Value;
use std::path::PathBuf;
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
/// A page's edges, and the air between a row's words and its control.
const PAGE_SIDE: f32 = 24.0;
const PAGE_TOP: f32 = 20.0;
const ROW_GAP: f32 = 24.0;

pub enum SettingsEvent {
    Dismiss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Kerf,
    Appearance,
    Shortcuts,
    About,
}

impl Page {
    const ALL: [Page; 4] = [Page::Kerf, Page::Appearance, Page::Shortcuts, Page::About];

    fn label(self) -> &'static str {
        match self {
            Page::Kerf => "Kerf",
            Page::Appearance => "Appearance",
            Page::Shortcuts => "Shortcuts",
            Page::About => "About",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Page::Kerf => Icon::Sparkle,
            Page::Appearance => Icon::Eye,
            Page::Shortcuts => Icon::Media,
            Page::About => Icon::Film,
        }
    }

    fn index(self) -> usize {
        Page::ALL.iter().position(|page| *page == self).unwrap_or(0)
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
    scroll: [ScrollHandle; 4],
    model: Entity<TextInput>,
    key: Entity<TextInput>,
    base_url: Entity<TextInput>,
    /// The key as it was loaded or last kept, so only an edited one is saved.
    key_kept: String,
    /// What saving the key came to, and whether it failed.
    key_status: Option<(SharedString, bool)>,
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
        bind_close(cx);
        let (provider, model_text, base_url_text, keymap) = {
            let prefs = &session.read(cx).prefs;
            (
                prefs.provider,
                prefs.model().to_owned(),
                prefs.openai_base_url.clone(),
                prefs.keymap,
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
            let mut input = TextInput::new(cx, "https://api.openai.com/v1");
            input.set_text(base_url_text, cx);
            input
        });
        let mut subscriptions = vec![
            cx.subscribe_in(&model, window, Self::model_event),
            cx.subscribe_in(&key, window, Self::key_event),
            cx.subscribe_in(&base_url, window, Self::base_url_event),
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
        Self {
            session,
            focus_handle: cx.focus_handle(),
            page: Page::Kerf,
            previous: None,
            scroll: Default::default(),
            model,
            key,
            base_url,
            key_kept: key_text.trim().to_owned(),
            key_status: None,
            catalog: (keymap, commands::catalog(keymap.wire())),
            update: Update::Idle,
            crashes: kerf_bridge::crash_reports(),
            about: kerf_bridge::about(),
            _subscriptions: subscriptions,
        }
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
            _ => {}
        }
    }

    fn show(&mut self, page: Page, cx: &mut Context<Self>) {
        if page == self.page {
            return;
        }
        if page == Page::About {
            self.crashes = kerf_bridge::crash_reports();
        }
        self.previous = Some((self.page, Instant::now()));
        self.page = page;
        cx.notify();
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

    // ------------------------------------------------------------ Kerf

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
                        Provider::ClaudeCode => {}
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
                    .update(cx, |session, _| session.prefs.openai_base_url = text);
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
        let model = self.session.read(cx).prefs.model().to_owned();
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

    fn set_mcp(&mut self, on: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.prefs.mcp_enabled = on;
            session.save_prefs();
        });
        cx.notify();
    }

    fn kerf_page(&self, window: &Window, cx: &Context<Self>) -> Vec<AnyElement> {
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
        if let Some(env) = provider.key_env() {
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
                    format!("Kept in your system keychain; {env} is used when none is kept").into(),
                ),
                Some(
                    field(&self.key, window, cx)
                        .w(px(FIELD))
                        .flex_none()
                        .into_any_element(),
                ),
                status.into_iter().collect(),
            ));
            rows.push(row(
                "Model",
                Some("Empty for the service's default".into()),
                Some(
                    field(&self.model, window, cx)
                        .w(px(FIELD))
                        .flex_none()
                        .into_any_element(),
                ),
                Vec::new(),
            ));
            if provider == Provider::Openai {
                rows.push(row(
                    "Base URL",
                    Some("Any service that speaks the OpenAI API; empty for OpenAI's own".into()),
                    Some(
                        field(&self.base_url, window, cx)
                            .w(px(FIELD))
                            .flex_none()
                            .into_any_element(),
                    ),
                    Vec::new(),
                ));
            }
        } else {
            rows.push(row(
                "API key",
                Some("Claude Code keeps its own sign-in, so Kerf needs no key for it".into()),
                Some(ui::chip("Not needed", theme::ink2()).into_any_element()),
                Vec::new(),
            ));
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
        let project_servers = session
            .project
            .as_ref()
            .map(|project| home_relative(&project.join(".kerf").join("mcp.json")))
            .unwrap_or_else(|| "<project>/.kerf/mcp.json".into());
        rows.push(row(
            "External MCP tools",
            Some("Lets Kerf call local MCP servers, over stdio. The servers in these files are attached; they may hold credentials".into()),
            Some(ui::switch("settings-mcp", prefs.mcp_enabled, Self::chooser(cx, Self::set_mcp)).into_any_element()),
            vec![div()
                .flex()
                .flex_col()
                .child(ui::line("~/.kerf/mcp.json", text::MONO_SMALL, theme::ink3()))
                .child(ui::line(project_servers, text::MONO_SMALL, theme::ink3()))
                .into_any_element()],
        ));
        vec![group(rows).into_any_element()]
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
        commands::bind(keymap.wire(), cx);
        // Binding the keymap clears every binding, ours among them.
        bind_close(cx);
        crate::export::bind_close(cx);
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
                let component = report["component"].as_str().unwrap_or("kerf");
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
                    Some("Checks GitHub once a day. Never downloads or installs".into()),
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
                    .on_click(cx.listener(move |this, _, _, cx| this.show(page, cx)))
            }))
    }

    fn page_body(&self, page: Page, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let content = match page {
            Page::Kerf => self.kerf_page(window, cx),
            Page::Appearance => self.appearance_page(cx),
            Page::Shortcuts => self.shortcuts_page(cx),
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
                .child(shown_chord(chord))
        }))
}

/// A chord as this platform writes it: ⌘⇧Z on a Mac, Ctrl+Shift+Z elsewhere.
fn shown_chord(chord: &str) -> String {
    if !cfg!(target_os = "macos") {
        return chord.to_owned();
    }
    let (modifiers, key) = match chord.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None => match chord.rfind('+') {
            Some(at) => (&chord[..at], &chord[at + 1..]),
            None => ("", chord),
        },
    };
    let mut shown = String::new();
    for modifier in modifiers.split('+').filter(|part| !part.is_empty()) {
        shown.push_str(match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => "⌘",
            "meta" => "⌃",
            "alt" | "option" => "⌥",
            "shift" => "⇧",
            other => other,
        });
    }
    shown.push_str(key);
    shown
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
        Provider::ClaudeCode => "",
    }
}

fn provider_hint(provider: Provider) -> SharedString {
    match provider {
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
