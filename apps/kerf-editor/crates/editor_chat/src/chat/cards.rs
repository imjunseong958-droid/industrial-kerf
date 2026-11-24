//! Proposal cards ([`ChatView::proposal_card`]): what each proposal does
//! and the clips it changes (a colour change on one clip in a line of its
//! own), its script and its versions, and what to do with it, the open one
//! nearest the playhead pinned; a settled one in a line that says when it
//! was settled here; and the line an earlier turn that named a proposal
//! keeps, a later one holding its card.

use super::menus::Menu;
use super::picture::cut_frames;
use super::script::{colour_change, describe_script_line, script_summary};
use super::turns::link;
use super::ChatView;
use editor_session::commands::RevealPicture;
use editor_state::book::{pin_order, Version};
use editor_state::changes::{clip_changes, reel_plan, reel_total, signed_frames, Segment};
use editor_state::model::{self, Proposal, ProposalStatus, Timeline};
use editor_state::store::Connection;
use editor_ui::icons::Icon;
use editor_ui::motion;
use editor_ui::selectable;
use editor_ui::theme::{self, motion as scene, text};
use editor_ui::ui;
use gpui::{
    anchored, deferred, div, point, prelude::*, px, AnimationExt, AnyElement, App, Context, Corner,
    Edges, ElementId, SharedString, Window,
};
use kerf_bridge::Command;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long ago something was settled here, as a card says it: "now",
/// "4 min", "2 h", then "N d".
pub fn settled_age(ago: Duration) -> String {
    let seconds = ago.as_secs();
    if seconds < 60 {
        "now".to_owned()
    } else if seconds < 3600 {
        format!("{} min", seconds / 60)
    } else if seconds < 86_400 {
        format!("{} h", seconds / 3600)
    } else {
        format!("{} d", seconds / 86_400)
    }
}

/// How long after `ago` what [`settled_age`] says changes: at the next
/// minute, hour or day it counts.
pub fn settled_age_changes(ago: Duration) -> Duration {
    let seconds = ago.as_secs();
    let unit = match seconds {
        0..3600 => 60,
        3600..86_400 => 3600,
        _ => 86_400,
    };
    Duration::from_secs((seconds / unit + 1) * unit).saturating_sub(ago)
}

/// What a settled card says: "Accepted in rev 23 · now", "Accepted",
/// "Rejected · 4 min". The age only for one settled in this window.
pub fn settled_words(accepted: bool, revision: Option<u64>, ago: Option<Duration>) -> String {
    let words = match (accepted, revision) {
        (true, Some(revision)) => format!("Accepted in rev {revision}"),
        (true, None) => "Accepted".to_owned(),
        (false, _) => "Rejected".to_owned(),
    };
    match ago {
        Some(ago) => format!("{words} · {}", settled_age(ago)),
        None => words,
    }
}

/// The line over proposals that no turn of this conversation staged: who
/// proposed the one over MCP when an agent did (the runtime's author kind
/// `mcp`), else how many there are.
pub fn strays_note(strays: &[Proposal]) -> String {
    if let [only] = strays {
        let agent = only.author.kind == kerf_bridge::event::AuthorKind::Mcp;
        let name = only.author.name.trim();
        if agent && !name.is_empty() {
            return format!("{name} proposed this over MCP.");
        }
        return "Proposed outside this conversation.".to_owned();
    }
    format!("{} proposals made outside this conversation.", strays.len())
}

/// The proposal card's Preview, Script and version button say what they
/// do on a rest.
pub const PREVIEW_TIP: &str = "Preview the proposal on the program";
pub const SCRIPT_TIP: &str = "Show the script";
pub const VERSIONS_TIP: &str = "Proposal versions";

/// Whether a proposal's preview in `status` is showing or on its way, so
/// its card says Previewing: not once it failed, stopped or was left, nor
/// while it waits to be rendered (`required`, `unavailable`), when
/// pressing Preview tries again.
pub fn previewing(status: &str) -> bool {
    matches!(status, "running" | "completed" | "ready" | "live")
}

/// The keys a card's Accept and Reject name, and the keymap they were read
/// from.
pub(super) type CardKeys = (String, HashMap<&'static str, String>);

/// What an open card shows, read once for its parts.
struct CardFacts {
    /// The proposal's id, and the id of the version the card shows.
    id: String,
    shown_id: String,
    versions: Vec<Version>,
    version: Version,
    stale: bool,
    connected: bool,
    /// What it changes, a row a clip (its name, where it goes, by how
    /// much); without its timeline, its script's lines in words.
    rows: Vec<(String, String, String)>,
    /// A colour change on one clip, said in a line of its own.
    colour: Option<(String, String)>,
    /// The changed lines of the document, else the script.
    script: Vec<String>,
    preview_shown: bool,
    /// What Watch what changed plays: only what it changes.
    watch: Option<Vec<Segment>>,
    changes_key: String,
    changes_open: bool,
    script_open: bool,
    count: usize,
    /// A first cut's timeline, its picture at the card's top.
    first_cut: Option<Timeline>,
}

impl ChatView {
    /// The pinned proposal's id, and the id of the version its card shows.
    fn pinned(&self, cx: &App) -> Option<(String, String)> {
        let session = self.session.read(cx);
        let snapshot = &session.state.snapshot;
        let pinned = pin_order(
            &snapshot.proposals,
            &self.book,
            session.timeline(),
            snapshot.proposal_timeline.as_ref(),
            session.state.position,
        )
        .into_iter()
        .next()?;
        let shown = snapshot
            .proposals
            .iter()
            .find(|proposal| proposal.proposal_id == pinned)
            .and_then(|proposal| {
                let shown = self.shown.get(&pinned)?;
                self.book
                    .versions_of(proposal)
                    .iter()
                    .any(|version| &version.proposal_id == shown)
                    .then(|| shown.clone())
            })
            .unwrap_or_else(|| pinned.clone());
        Some((pinned, shown))
    }

    /// The version id the pinned proposal's card shows: what its Accept, and
    /// Accept from the keyboard, takes.
    pub fn pinned_proposal(&self, cx: &App) -> Option<String> {
        self.pinned(cx).map(|(_, shown)| shown)
    }

    /// The proposal id the registry's `proposal.accept`, `proposal.reject`
    /// or `proposal.forward` acts on, as the pinned card's own buttons do:
    /// Accept takes the version the card shows, Reject and Bring forward the
    /// proposal. None with no open proposal, or for another command.
    pub fn proposal_target(&self, command: &str, cx: &App) -> Option<String> {
        let (proposal, shown) = self.pinned(cx)?;
        match command {
            "proposal.accept" => Some(shown),
            "proposal.reject" | "proposal.forward" => Some(proposal),
            _ => None,
        }
    }

    /// The key command `id` has in the keymap, as a tip names it: read
    /// again when the keymap changes.
    fn key_of(&self, id: &str, cx: &App) -> Option<String> {
        let keymap = self.session.read(cx).prefs.keymap.wire();
        let mut keys = self.keys.borrow_mut();
        if keys
            .as_ref()
            .is_none_or(|(read, _)| read.as_str() != keymap)
        {
            *keys = Some((keymap.to_owned(), card_keys(keymap)));
        }
        keys.as_ref().and_then(|(_, keys)| keys.get(id).cloned())
    }

    /// A proposal a later turn holds the card of, where an earlier turn
    /// named it: the version it named, in a line ("v1, revised below").
    pub(super) fn proposal_named(&self, proposal: &Proposal, named: &str) -> AnyElement {
        let versions = self.book.versions_of(proposal);
        let newest = versions
            .iter()
            .map(|version| version.version)
            .max()
            .unwrap_or(proposal.version);
        let (script, number) = versions
            .iter()
            .find(|version| version.proposal_id == named)
            .map_or((proposal.script.as_str(), proposal.version), |version| {
                (version.script.as_str(), version.version)
            });
        let words = if number < newest {
            format!("v{}, revised below", number.max(1))
        } else {
            "Shown below".to_owned()
        };
        div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(theme::space::S6))
            .child(Icon::Film.view(12.0, theme::ink3()))
            .child(
                ui::line(script_summary(script, number), text::CONTROL, theme::ink2())
                    .flex_shrink()
                    .min_w_0(),
            )
            .child(ui::label("·", text::CONTROL, theme::ink3()).flex_none())
            .child(ui::label(words, text::CONTROL, theme::ink3()).flex_none())
            .into_any_element()
    }

    /// A proposal's card: settled, one line; open, what it does and the
    /// clips it changes, its script and versions, and what to do with it,
    /// `pinned` marked and its Accept or Bring forward the primary.
    pub(super) fn proposal_card(
        &self,
        proposal: &Proposal,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !proposal.is_pending() {
            return self.settled_card(proposal, cx);
        }
        let mut facts = self.card_facts(proposal, cx);
        let mut card = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(14.0))
            .rounded(px(theme::radius::CARD))
            .bg(theme::raised())
            .border_1()
            .border_color(if facts.stale {
                theme::line()
            } else {
                theme::with_alpha(theme::pos(), 0.45)
            });
        if let Some(cut) = &facts.first_cut {
            let state = self.picture_state(&facts.id, cx);
            card = card.child(self.cut_picture(&facts.id, cut, state, window, cx));
        }
        if pinned {
            card = card
                .child(
                    div()
                        .absolute()
                        .left(px(-1.0))
                        .top(px(12.0))
                        .bottom(px(12.0))
                        .w(px(2.0))
                        .rounded_full()
                        .bg(theme::accent()),
                )
                .child(ui::label("Pinned proposal", text::META, theme::ink3()));
        }
        card = card
            .child(card_title(&facts, cx))
            .child(self.card_links(&facts, window, cx));
        if facts.changes_open && facts.count > 0 {
            let rows = std::mem::take(&mut facts.rows);
            card = card.child(change_rows(rows, facts.colour.take(), &facts.changes_key));
        }
        if facts.script_open && !facts.script.is_empty() {
            card = card.child(diff(&facts.script, &facts.id, &self.selections));
        }
        card.child(self.card_actions(facts, pinned, cx))
            .into_any_element()
    }

    /// A settled proposal, in one line: what it did, and that it was
    /// accepted (in which revision) or rejected; settled here, how long
    /// ago, drawn again as that turns over.
    fn settled_card(&self, proposal: &Proposal, cx: &Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        // Settled, it keeps to one line; settled here, it says when.
        let accepted = proposal.status == ProposalStatus::Accepted;
        let ago = session
            .settled_at(|sent| self.book.is(proposal, sent))
            .map(|at| at.elapsed());
        if let Some(ago) = ago {
            self.ages.at(Instant::now() + settled_age_changes(ago), cx);
        }
        let words = settled_words(
            accepted,
            self.book.applied.get(&proposal.proposal_id).copied(),
            ago,
        );
        let (icon, color) = if accepted {
            (Icon::Check, theme::accent_ink())
        } else {
            (Icon::Close, theme::ink2())
        };
        div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(theme::space::S6))
            .child(icon.view(12.0, color))
            .child(
                ui::line(
                    script_summary(&proposal.script, proposal.version),
                    text::CONTROL,
                    theme::ink2(),
                )
                .flex_shrink()
                .min_w_0(),
            )
            .child(ui::label("·", text::CONTROL, theme::ink3()).flex_none())
            .child(ui::label(words, text::CONTROL, color).flex_none())
            .into_any_element()
    }

    /// What an open card shows: the version shown (the one chosen, else
    /// the newest), what it changes on its timeline or, without one, in
    /// its script's words, its changed lines, whether it previews, what
    /// Watch plays, what is unfolded, and a first cut's picture.
    fn card_facts(&self, proposal: &Proposal, cx: &App) -> CardFacts {
        let id = proposal.proposal_id.clone();
        let session = self.session.read(cx);
        let fps = session.timeline().rate();
        let connected = session.state.connection == Connection::Connected;
        let versions = self.book.versions_of(proposal);
        let shown_id = self
            .shown
            .get(&id)
            .filter(|shown| {
                versions
                    .iter()
                    .any(|version| &version.proposal_id == *shown)
            })
            .cloned()
            .unwrap_or_else(|| id.clone());
        let version = versions
            .iter()
            .find(|version| version.proposal_id == shown_id)
            .cloned()
            .unwrap_or_default();
        let stale = proposal.is_stale();
        // Its own timeline, else the one the snapshot drew for the first open one.
        let timeline = self.book.timelines.get(&shown_id).or_else(|| {
            (session
                .state
                .snapshot
                .pending()
                .map(|first| first.proposal_id.as_str())
                == Some(shown_id.as_str()))
            .then_some(session.state.snapshot.proposal_timeline.as_ref())
            .flatten()
        });
        let changes = timeline
            .map(|timeline| clip_changes(session.timeline(), timeline))
            .unwrap_or_default();
        // A colour change on one clip is said in a line of its own.
        let colour =
            timeline.and_then(|proposed| colour_change(session.timeline(), proposed, &changes));
        let rows: Vec<(String, String, String)> = if changes.is_empty() {
            // Without its timeline, the script's lines say what it does, in words.
            version
                .script
                .lines()
                .map(str::trim)
                .filter(|line| {
                    !line.is_empty() && !line.starts_with('#') && !line.starts_with("use ")
                })
                .map(|line| {
                    let (said, detail) = describe_script_line(line, session.timeline(), timeline);
                    (said, detail, String::new())
                })
                .collect()
        } else {
            changes.iter().map(|change| change.row(fps)).collect()
        };
        // The changed lines of the document, else the script.
        let mut script: Vec<String> = version
            .document_diff
            .lines()
            .filter(|line| !line.starts_with("+++") && !line.starts_with("---"))
            .filter(|line| line.starts_with('+') || line.starts_with('-'))
            .map(str::to_owned)
            .collect();
        if script.is_empty() {
            script = version
                .script
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_owned)
                .collect();
        }
        let preview_shown = session.state.preview.proposal.as_deref() == Some(shown_id.as_str())
            && previewing(&session.state.preview.status);
        // Only what it changes, two seconds either side, one stretch after
        // another.
        let watch = timeline.filter(|_| !stale).and_then(|proposed| {
            let program = session.timeline();
            let segments = reel_plan(
                &changes,
                session.state.program_length().max(program.duration),
                proposed.duration.max(cut_frames(proposed).1),
                2.0,
            );
            (!segments.is_empty()).then_some(segments)
        });
        let changes_key = format!("changes-{id}");
        let changes_open = self.is_open(&changes_key);
        let script_open = self.scripts.contains(&id);
        let count = rows.len();
        // A first cut has its picture at the card's top.
        let first_cut = timeline
            .filter(|cut| !session.timeline().has_clips() && cut.has_clips())
            .cloned();
        CardFacts {
            id,
            shown_id,
            versions,
            version,
            stale,
            connected,
            rows,
            colour,
            script,
            preview_shown,
            watch,
            changes_key,
            changes_open,
            script_open,
            count,
            first_cut,
        }
    }

    /// N changes, Script, and the versions.
    fn card_links(&self, facts: &CardFacts, window: &Window, cx: &Context<Self>) -> gpui::Div {
        let id = &facts.id;
        let count = facts.count;
        let toggle_changes = facts.changes_key.clone();
        let toggle_script = id.clone();
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .when(count > 0, |row| {
                row.child(
                    div()
                        .id(ElementId::Name(format!("changes-toggle-{id}").into()))
                        .flex()
                        .items_center()
                        .gap(px(theme::space::S4))
                        .cursor_pointer()
                        .child(ui::label(
                            if count == 1 {
                                "1 change".to_owned()
                            } else {
                                format!("{count} changes")
                            },
                            text::CONTROL,
                            theme::accent_ink(),
                        ))
                        .child(self.chevron(&facts.changes_key, theme::accent_ink(), window))
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_fold(&toggle_changes, cx)
                            }),
                        ),
                )
            })
            .when(!facts.script.is_empty(), |row| {
                row.child(ui::tip_on(
                    link(
                        ElementId::Name(format!("script-{id}").into()),
                        "Script",
                        facts.script_open,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.scripts.remove(&toggle_script) {
                            this.scripts.insert(toggle_script.clone());
                        }
                        cx.notify();
                    })),
                    Some(ui::Tip::new(SCRIPT_TIP)),
                ))
            })
            .child(div().flex_1())
            .when(facts.versions.len() > 1, |row| {
                row.child(self.versions_button(id, &facts.shown_id, &facts.versions, cx))
            })
    }

    /// What to do with it: Accept (Bring forward once out of date), Watch
    /// what changed, Preview and Reject; the pinned card's first is the
    /// primary.
    fn card_actions(&self, facts: CardFacts, pinned: bool, cx: &App) -> gpui::Div {
        let CardFacts {
            id,
            shown_id,
            stale,
            connected,
            watch,
            preview_shown,
            ..
        } = facts;
        let accept = shown_id.clone();
        let reject = id.clone();
        let preview = shown_id.clone();
        let forward = id.clone();
        div()
            .flex()
            .flex_wrap()
            .gap(px(theme::space::S8))
            .when(!stale, |row| {
                let mut button =
                    ui::button(ElementId::Name(format!("accept-{id}").into()), "Accept")
                        .small()
                        .icon(Icon::Check)
                        .tooltip("Accept")
                        .reason("Connect to start editing")
                        .disabled(!connected);
                if let Some(key) = self.key_of("proposal.accept", cx) {
                    button = button.key(key);
                }
                let button = button.on_click({
                    let session = self.session.clone();
                    move |_, _, cx| session.update(cx, |session, _| session.accept(&accept))
                });
                row.child(if pinned { button.primary() } else { button })
            })
            .when(stale, |row| {
                let button = ui::button(
                    ElementId::Name(format!("forward-{id}").into()),
                    "Bring forward",
                )
                .small()
                .tooltip("Out of date · run it again on the edit as it is…")
                .reason("Connect to start editing")
                .disabled(!connected)
                .on_click({
                    let session = self.session.clone();
                    move |_, _, cx| session.update(cx, |session, _| session.forward(&forward))
                });
                row.child(if pinned { button.primary() } else { button })
            })
            .when_some(watch, |row, segments| {
                let session = self.session.clone();
                let proposal = shown_id.clone();
                let seconds = reel_total(&segments).ceil() as i64;
                row.child(
                    ui::button(
                        ElementId::Name(format!("watch-{id}").into()),
                        format!("Watch what changed · {seconds} s"),
                    )
                    .small()
                    .icon(Icon::Eye)
                    .tooltip("Plays only what this proposal changes")
                    .reason("Connect to start editing")
                    .disabled(!connected)
                    .on_click(move |_, window, cx| {
                        session.update(cx, |session, cx| {
                            session.watch_changes(&proposal, segments.clone());
                            cx.notify();
                        });
                        window.dispatch_action(Box::new(RevealPicture), cx);
                    }),
                )
            })
            .child(
                ui::button(
                    ElementId::Name(format!("preview-{id}").into()),
                    if preview_shown {
                        "Previewing"
                    } else {
                        "Preview"
                    },
                )
                .small()
                .icon(if preview_shown {
                    Icon::Check
                } else {
                    Icon::Play
                })
                .tooltip(PREVIEW_TIP)
                .reason("Connect to start editing")
                .disabled(!connected)
                .on_click({
                    let session = self.session.clone();
                    move |_, window, cx| {
                        // On, it shows what the proposal makes; off, the program again.
                        session.update(cx, |session, _| {
                            if preview_shown {
                                session.send(Command::PreviewExit);
                            } else {
                                session.preview(Some(&preview));
                            }
                        });
                        if !preview_shown {
                            window.dispatch_action(Box::new(RevealPicture), cx);
                        }
                    }
                }),
            )
            .child(
                ui::button(ElementId::Name(format!("reject-{id}").into()), "Reject")
                    .small()
                    .ghost()
                    .tooltip("Reject")
                    .key(
                        self.key_of("proposal.reject", cx)
                            .unwrap_or_else(|| "Esc".to_owned()),
                    )
                    .reason("Connect to start editing")
                    .disabled(!connected)
                    .on_click({
                        let session = self.session.clone();
                        move |_, _, cx| session.update(cx, |session, _| session.reject(&reject))
                    }),
            )
    }

    /// "v2 ⌄": the versions of a proposal, each with where its first move
    /// lands; choosing one shows it and previews it.
    fn versions_button(
        &self,
        id: &str,
        shown_id: &str,
        versions: &[Version],
        cx: &Context<Self>,
    ) -> AnyElement {
        let menu = Menu::Versions(id.to_owned());
        let open = self.menu.as_ref() == Some(&menu);
        let shown = versions
            .iter()
            .find(|version| version.proposal_id == shown_id)
            .map_or(1, |version| version.version);
        let toggle = menu.clone();
        let button =
            theme::type_style(div(), text::MONO_SMALL)
                .id(ElementId::Name(format!("versions-{id}").into()))
                .flex()
                .items_center()
                .gap(px(theme::space::S2))
                .h(px(20.0))
                .pl(px(theme::space::S6))
                .pr(px(theme::space::S2))
                .rounded(px(theme::radius::SMALL))
                .text_color(if open { theme::ink() } else { theme::ink3() })
                .cursor_pointer()
                .hover(|button| button.bg(theme::hover2()).text_color(theme::ink()))
                .child(format!("v{shown}"))
                .child(Icon::ChevronDown.view(14.0, theme::ink3()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_menu(toggle.clone(), window, cx)
                }));
        // Its menu says all the tip would, so the tip waits while it is open.
        let button = ui::tip_on(button, (!open).then(|| ui::Tip::new(VERSIONS_TIP)));
        let mut wrapper = div().relative().flex_none().child(button);
        if let Some(body) = self.kit_menu.clone().filter(|_| open) {
            wrapper = wrapper.child(
                div().absolute().top_full().right_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopRight)
                            .offset(point(px(0.0), px(theme::space::S4)))
                            .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                            .child(body),
                    )
                    .with_priority(1),
                ),
            );
        }
        wrapper.into_any_element()
    }

    /// A card's versions ("v2 00:00:04:12 · +6 fr"), the one shown ticked:
    /// one chosen is shown on the card, previewed, and the picture comes in.
    pub(super) fn version_items(&self, id: &str, cx: &mut Context<Self>) -> Vec<ui::MenuItem> {
        let session = self.session.read(cx);
        let snapshot = &session.state.snapshot;
        let Some(proposal) = self.book.find(&snapshot.proposals, id) else {
            return Vec::new();
        };
        let versions = self.book.versions_of(proposal);
        let shown_id = self.shown.get(id).cloned().unwrap_or_else(|| id.to_owned());
        let program = session.timeline().clone();
        let fps = program.rate();
        let this = cx.entity().downgrade();
        versions
            .iter()
            .map(|version| {
                let label = self
                    .book
                    .timelines
                    .get(&version.proposal_id)
                    .and_then(|timeline| {
                        clip_changes(&program, timeline)
                            .into_iter()
                            .find(|change| change.moved)
                    })
                    .map(|change| {
                        let shift = ((change.new_start - change.old_start) * fps).round() as i64;
                        let shift = signed_frames(shift);
                        format!(
                            "v{} {} · {} fr",
                            version.version,
                            model::timecode(change.new_start, fps),
                            shift.trim_end_matches('f')
                        )
                    })
                    .unwrap_or_else(|| format!("v{}", version.version));
                let proposal = id.to_owned();
                let chosen = version.proposal_id.clone();
                let this = this.clone();
                ui::MenuItem::new(label)
                    .checked(version.proposal_id == shown_id)
                    .on_choose(move |window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            if chosen == proposal {
                                this.shown.remove(&proposal);
                            } else {
                                this.shown.insert(proposal.clone(), chosen.clone());
                            }
                            let chosen = chosen.clone();
                            this.session
                                .update(cx, |session, _| session.preview(Some(&chosen)));
                            cx.notify();
                        });
                        window.dispatch_action(Box::new(RevealPicture), cx);
                    })
            })
            .collect()
    }
}

/// The keys a card's actions and Stop name in `keymap`, as tips name them.
fn card_keys(keymap: &str) -> HashMap<&'static str, String> {
    let catalog = editor_session::commands::catalog(keymap);
    [
        "proposal.accept",
        "proposal.reject",
        "proposal.forward",
        "ai.stop",
    ]
    .into_iter()
    .filter_map(|id| {
        let command = catalog.iter().find(|command| command.id == id)?;
        let shown = editor_session::palette::shortcut(command);
        (!shown.is_empty()).then_some((id, shown))
    })
    .collect()
}

/// An open card's title, what the version shown does; pressed, it opens or
/// closes the card's changes. Out of date, it says so.
fn card_title(facts: &CardFacts, cx: &Context<ChatView>) -> gpui::Div {
    let id = &facts.id;
    let toggle_title = facts.changes_key.clone();
    div()
        .flex()
        .items_start()
        .gap(px(theme::space::S8))
        .child(
            div()
                .pt(px(theme::space::S2))
                .child(Icon::Film.view(16.0, theme::pos())),
        )
        .child(
            // Pressed, the title opens or closes the card's changes.
            ui::label(
                script_summary(&facts.version.script, facts.version.version),
                text::BODY_NAME,
                theme::ink(),
            )
            .id(ElementId::Name(format!("title-{id}").into()))
            .flex_1()
            .min_w_0()
            .when(facts.count > 0, |title| {
                title.cursor_pointer().on_click(
                    cx.listener(move |this, _, _, cx| this.toggle_fold(&toggle_title, cx)),
                )
            }),
        )
        .when(facts.stale, |row| {
            row.child(ui::chip("Out of date", theme::warn()))
        })
}

/// A card's changes, unfolded, fading in over `FADE`: a row a clip (its
/// name, where it goes, by how much), and a colour change on one clip in a
/// line of its own.
fn change_rows(
    rows: Vec<(String, String, String)>,
    colour: Option<(String, String)>,
    changes_key: &str,
) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .children(rows.into_iter().map(|(name, target, shift)| {
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .h(px(24.0))
                .child(
                    ui::line(name, text::CONTROL, theme::ink())
                        .flex_1()
                        .min_w_0(),
                )
                .when(!target.is_empty(), |row| {
                    row.child(ui::line(target, text::TIMECODE, theme::ink2()).flex_none())
                })
                .child(div().w(px(40.0)).flex().justify_end().child(ui::line(
                    shift,
                    text::MONO_SMALL,
                    theme::ink3(),
                )))
        }))
        .when_some(colour, |rows, (title, params)| {
            rows.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::space::S4))
                    .py(px(theme::space::S4))
                    .child(ui::line(title, text::CONTROL, theme::ink()))
                    .when(!params.is_empty(), |line| {
                        line.child(ui::line(params, text::MONO_SMALL, theme::ink2()))
                    }),
            )
        })
        .with_animation(
            ElementId::Name(format!("{changes_key}-open").into()),
            motion::animation(scene::FADE),
            |rows, t| rows.opacity(t),
        )
}

/// A proposal's changed lines, its additions sage and removals brick;
/// they can be chosen and copied as the conversation's words can, one line
/// after another.
fn diff(lines: &[String], id: &str, shared: &selectable::Shared) -> impl IntoElement + use<> {
    let run: SharedString = format!("diff-{id}").into();
    div()
        .id(ElementId::Name(format!("diff-{id}").into()))
        .max_h(px(260.0))
        .overflow_y_scroll()
        .p(px(theme::space::S8))
        .rounded(px(theme::radius::CONTROL))
        .bg(theme::well())
        .children(lines.iter().take(400).enumerate().map(|(index, line)| {
            let color = if line.starts_with('+') {
                theme::pos()
            } else if line.starts_with('-') {
                theme::neg()
            } else if line.starts_with("@@") {
                theme::accent_ink()
            } else {
                theme::ink2()
            };
            theme::type_style(div(), text::MONO_SMALL)
                .text_color(color)
                .child(
                    selectable::selectable(
                        shared,
                        ElementId::Name(format!("{run}-{index}").into()),
                        run.clone(),
                        index,
                        line.clone(),
                    )
                    .before("\n"),
                )
        }))
}
