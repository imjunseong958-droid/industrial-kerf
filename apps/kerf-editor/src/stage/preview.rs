//! A proposal's preview on the picture: its render card in title safe at
//! the picture's top right (what renders, how far it has got, and Stop,
//! which Esc is too while the card shows), and at the transport's end what
//! the preview needs or plays (Render preview and why, or "Previewing
//! proposal v2" with Exit preview).

use super::hud::{self, render_card_shows, RenderCard};
use super::picture::TITLE_SAFE;
use super::StageView;
use crate::icons::Icon;
use crate::model::{self, ProposalShow};
use crate::theme::{self, layout, text};
use crate::ui;
use gpui::{div, prelude::*, px, AnyElement, App, Bounds, ClickEvent, Context, Pixels, Window};
use kerf_bridge::Command;

/// The preview render's card: its widest and its height.
const HUD_SIZE: (f32, f32) = (288.0, 72.0);

/// The version of the proposal the preview plays (`id`, the preview's
/// proposal) among the snapshot's `proposals`: the version's own number
/// when `id` is one of a proposal's versions, else that proposal's. None
/// when no proposal is `id`, or it has no number.
pub fn previewed_version(proposals: &[model::Proposal], id: &str) -> Option<u32> {
    let mut version = 0;
    for proposal in proposals {
        let mut matches = proposal.proposal_id == id;
        for listed in &proposal.versions {
            if listed.proposal_id == id {
                matches = true;
                version = listed.version;
            }
        }
        if matches && version == 0 {
            version = proposal.version;
        }
    }
    (version > 0).then_some(version)
}

/// What the chip over the picture says while a proposal's preview plays:
/// "Previewing proposal v2", or "Previewing proposal" with no version known.
pub fn preview_chip_words(version: Option<u32>) -> String {
    match version {
        Some(version) => format!("Previewing proposal v{version}"),
        None => "Previewing proposal".to_owned(),
    }
}

impl StageView {
    /// Whether the preview render's card shows ([`render_card_shows`]): a
    /// render runs, Stop was not pressed on it, Kerf is not building a cut
    /// (the picture just keeps up) and no change reel plays. The window
    /// asks before Esc stops the render, and adds whether the picture is in.
    pub fn render_card_shown(&self, cx: &App) -> bool {
        let session = self.session.read(cx);
        let state = &session.state;
        let preview = &state.preview;
        render_card_shows(RenderCard {
            rendering: preview.rendering(),
            stopped: self.stopped.as_deref()
                == Some(preview.proposal.as_deref().unwrap_or_default()),
            kerf_building: state.work.busy() && state.snapshot.proposal_timeline.is_some(),
            reel: session.reel().is_some() || session.cutting(),
            picture_shown: true,
        })
    }

    /// Stops the preview rendering (the card's Stop, or Esc while the card
    /// shows): its card goes at once and stays away until that run ends
    /// ([`super::render_stop_holds`]).
    pub fn stop_render(&mut self, cx: &mut Context<Self>) {
        let proposal = self
            .session
            .read(cx)
            .state
            .preview
            .proposal
            .clone()
            .unwrap_or_default();
        self.stopped = Some(proposal);
        self.session.update(cx, |session, cx| {
            session.send(Command::PreviewStop);
            cx.notify();
        });
        cx.notify();
    }

    /// What the preview render's card says while it shows
    /// ([`StageView::render_card_shown`]): its title, its line, and how far
    /// it has got.
    pub(super) fn preview_card(&self, cx: &App) -> Option<(String, String, f64)> {
        if !self.render_card_shown(cx) {
            return None;
        }
        let session = self.session.read(cx);
        let state = &session.state;
        let preview = &state.preview;
        let proposal_id = preview.proposal.clone().unwrap_or_default();
        let proposal = self.book.find(&state.snapshot.proposals, &proposal_id);
        let version = proposal.map_or(0, |proposal| {
            self.book
                .versions_of(proposal)
                .iter()
                .find(|version| version.proposal_id == proposal_id)
                .map_or(proposal.version, |version| version.version)
        });
        let timeline = session.timeline();
        let changes = proposal
            .and_then(|proposal| self.book.timeline_of(proposal, &proposal_id))
            .map(|proposed| crate::changes::clip_changes(timeline, proposed))
            .unwrap_or_default();
        let summary = proposal.map(ProposalShow::summary).unwrap_or_default();
        let detail = hud::detail(
            &changes,
            &summary,
            preview.message.as_deref().unwrap_or_default(),
            timeline.rate(),
        );
        Some((hud::title(version), detail, preview.progress.unwrap_or(0.0)))
    }

    /// At the transport's end, what the preview needs or plays: why the
    /// picture must be rendered first and Render preview; the version
    /// previewing and Exit preview; or why no preview can be made.
    pub(super) fn preview_control(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        let preview = session.state.preview.clone();
        // The chip keeps quiet while a change reel plays (its strip says
        // what plays) or Kerf is cutting the first cut (its card says so).
        let chip_quiet = session.reel().is_some() || session.cutting();
        // Which version of its proposal plays, as the chip says it.
        let previewing = preview
            .proposal
            .as_deref()
            .map(|id| preview_chip_words(previewed_version(&session.state.snapshot.proposals, id)));
        match preview.status.as_str() {
            // Why the picture needs it rendered (what the live picture cannot
            // show yet), cut short before the button and whole in the tips.
            "required" => {
                let reason = preview
                    .reason()
                    .unwrap_or_else(|| crate::store::PREVIEW_REQUIRED.to_owned());
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S8))
                        .min_w_0()
                        .flex_shrink()
                        .child(ui::tip_on(
                            ui::line(reason.clone(), text::CONTROL, theme::ink3())
                                .id("render-preview-reason")
                                .min_w_0()
                                .flex_shrink()
                                .max_w(px(layout::PREVIEW_REASON_MOST)),
                            Some(ui::Tip::new(reason.clone())),
                        ))
                        .child(
                            ui::button("render-preview", "Render preview")
                                .small()
                                .icon(Icon::Sparkle)
                                .tooltip(reason)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session.update(cx, |session, _| {
                                        session.preview(None);
                                    })
                                })),
                        )
                        .into_any_element(),
                )
            }
            // While it renders, its card over the picture says so, with Stop.
            "ready" | "live" if preview.proposal.is_some() && !chip_quiet => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S6))
                    .child(ui::chip(previewing.unwrap_or_default(), theme::pos()))
                    .child(
                        ui::button("exit-preview", "Exit preview")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.update(cx, |session, _| {
                                    session.send(Command::PreviewExit);
                                })
                            })),
                    )
                    .into_any_element(),
            ),
            "unavailable" => preview
                .message
                .clone()
                .map(|message| ui::chip(message, theme::ink3()).into_any_element()),
            _ => None,
        }
    }
}

/// What Stop on the preview render's card does.
pub(super) type OnStop = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The preview render's card, in title safe at the top right of `card` (the
/// picture, in the stage's own points): what renders and its first change,
/// a bar that fills as kerfd says with the percent, and Stop (`on_stop`).
#[derive(IntoElement)]
pub(super) struct PreviewCard {
    pub(super) card: Bounds<Pixels>,
    pub(super) title: String,
    pub(super) detail: String,
    pub(super) progress: f64,
    pub(super) on_stop: OnStop,
}

impl RenderOnce for PreviewCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let card = self.card;
        let (left, top) = (card.origin.x, card.origin.y);
        let (w, h) = (f32::from(card.size.width), f32::from(card.size.height));
        let (tx, ty) = ((w * TITLE_SAFE).round(), (h * TITLE_SAFE).round());
        let across = HUD_SIZE.0.min(w - 2.0 * tx - 16.0).max(0.0);
        div()
            .absolute()
            .left(left + px(w - tx - 8.0 - across))
            .top(top + px(ty + 8.0))
            .w(px(across))
            .h(px(HUD_SIZE.1))
            .flex()
            .flex_col()
            .justify_between()
            .px(px(theme::space::S12))
            .py(px(theme::space::S8))
            .rounded(px(theme::radius::CARD))
            .bg(theme::with_alpha(theme::card(), 0.94))
            .border_1()
            .border_color(theme::line2())
            .shadow_lg()
            // Its presses are its own, not the picture's (which plays).
            .occlude()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(16.0))
                            .rounded(px(theme::radius::SMALL))
                            .bg(theme::on())
                            .child(Icon::Sparkle.view(12.0, theme::ink())),
                    )
                    .child(
                        ui::line(self.title, text::CONTROL_STRONG, theme::ink())
                            .flex_1()
                            .min_w_0(),
                    ),
            )
            .child(ui::line(self.detail, text::META, theme::ink3()).pl(px(24.0)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .pl(px(24.0))
                    .child(
                        div()
                            .flex_1()
                            .child(ui::progress(self.progress, theme::activity::gpu())),
                    )
                    .child(ui::label(
                        hud::percent(self.progress),
                        text::MONO_SMALL,
                        theme::ink2(),
                    ))
                    .child(
                        // Its tip says what it stops, and its key: Esc
                        // stops the render too while the card shows
                        // (agent_hud.cpp's setTip).
                        ui::button("stop-preview", "Stop")
                            .small()
                            .ghost()
                            .tooltip(hud::STOP_TIP)
                            .key(hud::STOP_KEY)
                            .on_click(self.on_stop),
                    ),
            )
    }
}
