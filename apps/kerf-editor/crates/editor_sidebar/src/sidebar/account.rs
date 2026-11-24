//! The sidebar's foot: the account, which says who chat runs on (the Kerf
//! account's email and plan, Claude Code, or your own key and its model)
//! and, pressed, opens the app's menu above it; and Settings beside it.

use super::keys::{OpenMenu, ITEM};
use super::{SidebarEvent, SidebarView};
use editor_session::session::Session;
use editor_state::prefs::Provider;
use editor_ui::icons::Icon;
use editor_ui::theme::{self, layout, text};
use editor_ui::ui::{self, icon_button};
use gpui::{div, prelude::*, px, App, ClickEvent, Context, MouseDownEvent};
use kerf_bridge::event::CloudStatus;

/// Who chat runs on, as the account at the sidebar's foot says it: the Kerf
/// account's email and plan while signed in; else Claude Code on "Your
/// Claude plan", Kerf "Not signed in", or "My own API key" with its model,
/// "Set up in Settings" while there is no key.
pub fn account_line(
    provider: Provider,
    cloud: Option<&CloudStatus>,
    has_key: bool,
    model: &str,
) -> (String, String) {
    if editor_launcher::settings::account_status(cloud).sign_out {
        let account = cloud.and_then(|cloud| cloud.account.as_ref());
        let email = account.map_or("", |account| account.email.as_str());
        let plan = account
            .map(|account| account.plan.as_str())
            .filter(|plan| !plan.trim().is_empty())
            .map_or_else(|| "Kerf account".to_owned(), |plan| format!("{plan} plan"));
        return (email.to_owned(), plan);
    }
    match provider {
        Provider::ClaudeCode => ("Claude Code".into(), "Your Claude plan".into()),
        Provider::Cloud => ("Kerf".into(), "Not signed in".into()),
        Provider::Anthropic | Provider::Openai => {
            let model = model.trim();
            let below = if !has_key {
                "Set up in Settings".to_owned()
            } else if model.is_empty() {
                provider.label().to_owned()
            } else {
                editor_state::prefs::model_label(model)
            };
            ("My own API key".into(), below)
        }
    }
}

/// Who chat runs on in `session`'s window, as the account at the foot says
/// it ([`account_line`]), and whether it is signed in to Kerf.
pub fn account_of(session: &Session, cx: &App) -> (String, String, bool) {
    let cloud = editor_session::session::cloud(cx);
    let has_key = session.ai_access(cloud).as_deref() != Some("noKey");
    let (title, below) = account_line(
        session.prefs.provider,
        cloud,
        has_key,
        session.prefs.model(),
    );
    let signed_in = editor_launcher::settings::account_status(cloud).sign_out;
    (title, below, signed_in)
}

impl SidebarView {
    /// Whether the app's menu is open above the account (the window holds
    /// it): the account stays held meanwhile.
    pub fn set_app_menu_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.app_menu_open != open {
            self.app_menu_open = open;
            cx.notify();
        }
    }

    /// The foot: the account, and Settings beside it.
    pub(super) fn foot(
        &self,
        title: String,
        below: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .h(px(layout::SIDEBAR_FOOT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme::space::S4))
            .px(px(theme::space::S6))
            .border_t_1()
            .border_color(theme::line())
            .child(self.account(title, below, cx))
            // A Tab stop of the kit's itself, with its ring.
            .child(
                icon_button("settings", Icon::Gear)
                    .tooltip("Settings")
                    .key(editor_state::shortcuts::shown_keys("Ctrl+,"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::Settings))),
            )
    }

    /// The account at the foot: a disc with its first letter, who chat
    /// runs on, and under it the plan or the model. Pressed, the app's menu
    /// opens above it.
    fn account(
        &self,
        title: String,
        below: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let open = self.app_menu_open;
        let letter = title
            .chars()
            .find(|ch| ch.is_alphanumeric())
            .map(|ch| ch.to_uppercase().collect::<String>())
            .unwrap_or_default();
        Self::keyed(
            ui::capture_trigger(
                "account",
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.close_menus(cx);
                    cx.emit(SidebarEvent::AppMenu { from_keys: false });
                }),
            ),
            ITEM,
        )
        .flex()
        .items_center()
        .gap(px(theme::space::S8))
        .flex_1()
        .min_w_0()
        .h(px(36.0))
        .px(px(theme::space::S6))
        .rounded(px(theme::radius::CARD))
        .cursor_pointer()
        .when(open, |account| account.bg(theme::raised()))
        .hover(|account| account.bg(theme::raised()))
        .child(
            theme::type_style(div(), text::CONTROL_STRONG)
                .flex_none()
                .size(px(24.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::on())
                .text_color(theme::ink2())
                .child(letter),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(ui::line(title, text::CONTROL_STRONG, theme::ink2()))
                .child(ui::line(below, text::META, theme::ink3())),
        )
        .on_action(cx.listener(|this, _: &OpenMenu, _, cx| {
            this.close_menus(cx);
            cx.emit(SidebarEvent::AppMenu { from_keys: true });
        }))
        .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
            if event.is_keyboard() {
                this.close_menus(cx);
                cx.emit(SidebarEvent::AppMenu { from_keys: true });
            }
        }))
    }
}
