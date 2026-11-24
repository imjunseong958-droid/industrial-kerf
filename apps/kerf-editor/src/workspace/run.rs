//! The commands the window carries out: by the registry's id
//! ([`Workspace::run_command`]), what it answers on its root (Export's
//! choice, Esc, the mask tools' keys, the app menu's choices, Open Recent,
//! the palette and what opens over the window, F2, files dropped on it), and
//! how each command stands here for the menus and the palette: greyed with
//! why, or ticked.

use super::Workspace;
use editor_launcher::settings::Page;
use editor_session::commands::{ClearRecent, OpenRecent, RunCommand};
use editor_sidebar::sidebar;
use editor_state::delivery::Quality;
use editor_ui::theme::motion as scene;
use gpui::{
    point, prelude::*, px, size, App, Bounds, Context, Div, KeyDownEvent, MouseDownEvent, Pixels,
    Window,
};
use kerf_bridge::Command;
use std::collections::BTreeSet;
use std::path::PathBuf;

impl Workspace {
    /// What the window answers on its root, `root`: the registry's commands
    /// by id, Export's choice, Esc, the picture's and the mask tools' keys,
    /// the app menu's choices, Open Recent, the palette and what opens over
    /// the window; a press on what takes no keys; F2; files dropped on it.
    pub(super) fn actions(root: Div, cx: &Context<Self>) -> Div {
        let root = Self::command_actions(root, cx);
        let root = Self::menu_actions(root, cx);
        let root = Self::over_window_actions(root, cx);
        Self::pointer_actions(root, cx)
    }

    /// The registry's commands by id, Export's choice, Esc, the picture
    /// brought in, and the mask tools' keys.
    fn command_actions(root: Div, cx: &Context<Self>) -> Div {
        root.on_action(cx.listener(Self::run_command))
            .on_action(
                cx.listener(|this, _: &editor_finish::export::ExportChosen, _, cx| {
                    this.export_chosen(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &editor_session::commands::Escape, window, cx| {
                    this.escape(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &editor_session::commands::RevealPicture, _, cx| {
                    this.reveal_picture(cx)
                }),
            )
            // ⇧R ⇧E ⇧P wherever the window has the keys: a mask tool on the
            // chosen picture clip.
            .on_action(
                cx.listener(|this, _: &editor_stage::stage::RectangleMask, window, cx| {
                    this.toggle_mask_tool(editor_stage::stage::MaskTool::Rectangle, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &editor_stage::stage::EllipseMask, window, cx| {
                    this.toggle_mask_tool(editor_stage::stage::MaskTool::Ellipse, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &editor_stage::stage::PenMask, window, cx| {
                    this.toggle_mask_tool(editor_stage::stage::MaskTool::Pen, window, cx)
                }),
            )
    }

    /// What the app's menu chooses (Sign Out, a keymap, a branch, the
    /// viewer's zoom), and File ▸ Open Recent.
    fn menu_actions(root: Div, cx: &Context<Self>) -> Div {
        root
            // The app's menu: out of the Kerf account, another keymap,
            // another branch, the viewer's zoom.
            .on_action(cx.listener(|this, _: &sidebar::SignOut, _, cx| {
                this.session.update(cx, |session, cx| {
                    session.cloud_sign(None);
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, action: &sidebar::ChooseKeymap, _, cx| {
                this.choose_keymap(action.keymap, cx)
            }))
            .on_action(cx.listener(|this, action: &sidebar::SwitchBranch, _, cx| {
                let name = action.name.to_string();
                this.session.update(cx, |session, cx| {
                    session.run_command("collab.switchBranch", serde_json::json!({"name": name}));
                    cx.notify();
                });
            }))
            .on_action(
                cx.listener(|this, action: &editor_stage::stage::SetZoom, _, cx| {
                    this.reveal_picture(cx);
                    let zoom = action.percent as f32 / 100.0;
                    this.stage.update(cx, |stage, cx| stage.set_zoom(zoom, cx));
                }),
            )
            .on_action(cx.listener(|this, action: &OpenRecent, window, cx| {
                this.open_chosen(PathBuf::from(action.path.as_ref()), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearRecent, _, cx| {
                this.session.update(cx, |session, _| {
                    session.prefs.recent.clear();
                    session.save_prefs();
                });
                let project = this.session.read(cx).project.clone();
                editor_session::commands::refresh_menus(project.as_deref(), cx);
            }))
    }

    /// What opens over the window or fills the composer: the palette, a
    /// failed check's Ask Kerf, Cast and Styles…, Edit roles…, a shot's
    /// graph, Make, Finish.
    fn over_window_actions(root: Div, cx: &Context<Self>) -> Div {
        root.on_action(cx.listener(
            |this, _: &editor_session::palette::OpenPalette, window, cx| {
                if this.modal.is_none() {
                    // What the palette says of a proposal is of the pinned one.
                    this.pin_proposal(cx);
                    let checked = this.checked_commands(cx);
                    let palette = cx.new(|cx| {
                        editor_session::palette::CommandPalette::new(
                            this.session.clone(),
                            window,
                            cx,
                        )
                        .with_checked(checked)
                    });
                    this.open_modal(palette, window, cx);
                }
            },
        ))
        // Ask Kerf on a failed check (Finish's QC, Export's Preflight):
        // its words wait in the composer, to be changed before they
        // are sent.
        .on_action(cx.listener(
            |this, action: &editor_finish::finish::AskKerf, window, cx| {
                let words = action.words.to_string();
                this.chat
                    .update(cx, |chat, cx| chat.fill(words, window, cx));
            },
        ))
        // Cast and Styles…, and Edit roles…, over the window.
        .on_action(
            cx.listener(|this, _: &editor_launcher::cast::OpenCast, window, cx| {
                if this.modal_gives_way() {
                    let view = cx.new(|cx| {
                        editor_launcher::cast::CastView::new(this.session.clone(), window, cx)
                    });
                    this.open_modal(view, window, cx);
                }
            }),
        )
        .on_action(
            cx.listener(|this, _: &editor_making::media::EditRoles, window, cx| {
                if this.modal_gives_way() {
                    let view = cx.new(|cx| {
                        editor_making::media::RolesView::new(this.session.clone(), window, cx)
                    });
                    this.open_modal(view, window, cx);
                }
            }),
        )
        .on_action(cx.listener(
            |this, action: &editor_making::making::OpenShotGraph, window, cx| {
                this.open_shot_graph(action.media.clone(), window, cx)
            },
        ))
        .on_action(cx.listener(
            |this, action: &editor_making::making::MakeMedia, window, cx| {
                let plan = cx.new(|cx| {
                    editor_making::making::MakePlan::new(
                        this.session.clone(),
                        action.media.clone(),
                        action.rolls,
                        window,
                        cx,
                    )
                });
                this.open_modal(plan, window, cx);
            },
        ))
        .on_action(cx.listener(
            |this, action: &editor_finish::finish::OpenFinish, window, cx| {
                let finish = cx.new(|cx| {
                    editor_finish::finish::FinishView::new(
                        this.session.clone(),
                        action.run_check,
                        window,
                        cx,
                    )
                });
                this.open_modal(finish, window, cx);
            },
        ))
    }

    /// A press on what takes no keys, F2, and files dropped on the window.
    fn pointer_actions(root: Div, cx: &Context<Self>) -> Div {
        root
            // A press on what takes no keys (the conversation's empty room, a
            // chip, a label) leaves them with the window's own box: they
            // settle again once the press is dealt with, on the timeline
            // while it shows, so its keys still work after the press.
            .on_any_mouse_down(cx.listener(|_, _: &MouseDownEvent, window, cx| {
                cx.defer_in(window, |this, window, cx| this.settle_focus(window, cx))
            }))
            // F2 renames the open conversation where it is named.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "f2"
                    && !event.keystroke.modifiers.modified()
                    && this.modal.is_none()
                {
                    cx.stop_propagation();
                    this.rename_open(window, cx);
                }
            }))
            // Files dropped on the window come into the project's media.
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.session.update(cx, |session, cx| {
                    for path in paths.paths() {
                        session.import(path, true);
                    }
                    cx.notify();
                });
                if !this.media_open {
                    this.set_media(true, cx);
                }
            }))
    }

    /// The registry's checkable commands that are on in this window: the
    /// panes shown, the switches on, the timeline's tool; and the picture's
    /// mask tool out. Their rows in the menus and the palette are ticked.
    pub fn checked_commands(&self, cx: &App) -> BTreeSet<String> {
        let session = self.session.read(cx);
        let picture = self.panel_open && self.picture_shown.target() > 0.5;
        let timeline = self.panel_open && self.timeline_shown.target() > 0.5;
        let (_, _, looping) = session.marks();
        let live = session
            .state
            .collab
            .as_ref()
            .is_some_and(|collab| collab.live);
        [
            ("view.sidebar", self.sidebar_open),
            ("view.picture", picture),
            ("view.timeline", timeline),
            ("window.project", self.media_open),
            ("view.followKerf", session.prefs.follow_kerf),
            ("view.safeZones", session.prefs.safe_zones),
            ("playback.loop", looping),
            ("collab.live", live),
            ("sequence.snap", self.timeline.read(cx).snapping()),
            (self.timeline.read(cx).tool().command(), true),
        ]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(id, _)| id)
        // The mask tool out, by its own id ([`editor_stage::stage::mask_tool_id`]).
        .chain(
            self.stage
                .read(cx)
                .mask_tool()
                .map(editor_stage::stage::mask_tool_id),
        )
        .map(str::to_owned)
        .collect()
    }

    /// How the registry's commands stand here, for the menu bar (greyed,
    /// ticked). Only macOS draws one.
    pub(super) fn menu_states(&self, cx: &App) -> editor_session::commands::MenuStates {
        let states = self.command_states(cx);
        let unavailable = states
            .as_object()
            .map(|states| {
                states
                    .iter()
                    .filter(|(_, reason)| reason.is_string())
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        editor_session::commands::MenuStates {
            unavailable,
            checked: self.checked_commands(cx),
        }
    }

    /// Why each of the registry's commands cannot run now (null: it can),
    /// as every menu that lists them greys them: Ask Kerf… with Kerf's
    /// access too (no key, signed out), and the picture's mask tools by
    /// their own ids ([`editor_stage::stage::mask_tool_id`]).
    pub(super) fn command_states(&self, cx: &App) -> serde_json::Value {
        let mut states = self
            .session
            .read(cx)
            .command_states(editor_session::session::cloud(cx));
        let reason = self.mask_reason(cx);
        for tool in editor_stage::stage::MaskTool::ALL {
            states[editor_stage::stage::mask_tool_id(tool)] =
                reason.map_or(serde_json::Value::Null, serde_json::Value::from);
        }
        states
    }

    /// Why no mask tool can be taken out now, as the picture says it; None
    /// while one is out, or the chosen clip is a picture clip the picture
    /// places where it can be seen, on the program shown
    /// ([`StageView::can_draw`]).
    ///
    /// [`StageView::can_draw`]: editor_stage::stage::StageView::can_draw
    fn mask_reason(&self, cx: &App) -> Option<&'static str> {
        let stage = self.stage.read(cx);
        if stage.mask_tool().is_some() {
            return None;
        }
        (!stage.can_draw(cx)).then_some(editor_session::commands::NO_MASK_CLIP)
    }

    /// Where the Ask popover stands: over the chosen clip while the
    /// timeline shows it, else over the lanes at the playhead, else (no
    /// timeline in view) over the middle of the window.
    fn ask_anchor(&self, window: &Window, cx: &App) -> Bounds<Pixels> {
        let timeline_shown = self.panel_open && self.timeline_shown.target() > 0.5;
        if timeline_shown {
            let timeline = self.timeline.read(cx);
            let chosen = self.session.read(cx).selection.first().cloned();
            if let Some(bounds) = chosen.and_then(|clip| timeline.clip_bounds(&clip)) {
                return bounds;
            }
            if let Some(bounds) = timeline.playhead_anchor(cx) {
                return bounds;
            }
        }
        let viewport = window.viewport_size();
        Bounds::new(
            point(viewport.width / 2.0, viewport.height / 2.0),
            size(px(1.0), px(1.0)),
        )
    }

    /// The pinned proposal (the card the conversation pins) is what the
    /// registry's proposal commands act on and are said to be available for.
    fn pin_proposal(&mut self, cx: &mut Context<Self>) {
        let pinned = self.chat.read(cx).proposal_target("proposal.reject", cx);
        self.session
            .update(cx, |session, _| session.pinned_proposal = pinned);
    }

    /// Why command `id` cannot run now, as the registry says; None when it
    /// can. Only Ask's reason asks after the connection's key (the keychain).
    fn unavailable(&mut self, id: &str, cx: &mut Context<Self>) -> Option<String> {
        self.pin_proposal(cx);
        let session = self.session.read(cx);
        let states = if id == "ai.ask" {
            let cloud = editor_session::session::cloud(cx).cloned();
            session.command_states(cloud.as_ref())
        } else {
            kerf_bridge::commands_states(&session.command_context())
        };
        editor_session::palette::unavailable(&states, id).map(str::to_owned)
    }

    /// Carries out a command by its id: what the window does itself, else
    /// the request the registry resolves it to. One that cannot run now
    /// says why, and its key goes on to whatever else it means there.
    fn run_command(&mut self, action: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        let id = action.id.as_ref();
        self.refresh_keys(cx);
        if let Some(reason) = self.unavailable(id, cx) {
            // Esc puts things away everywhere: saying on each press that
            // there is no proposal to reject would be noise.
            if !self.bound_to_escape(id) {
                self.session.update(cx, |session, cx| {
                    session.state.notify(&reason, true);
                    cx.notify();
                });
            }
            cx.propagate();
            return;
        }
        // As the pinned card's own buttons: Accept takes the version it
        // shows, Reject and Bring Forward the proposal.
        if matches!(
            id,
            "proposal.accept" | "proposal.reject" | "proposal.forward"
        ) {
            let Some(proposal) = self.chat.read(cx).proposal_target(id, cx) else {
                return;
            };
            self.session.update(cx, |session, _| match id {
                "proposal.accept" => session.accept(&proposal),
                "proposal.reject" => session.reject(&proposal),
                _ => session.forward(&proposal),
            });
            cx.notify();
            return;
        }
        let ran = self.run_in_window(id, window, cx)
            || self.run_on_session(id, cx)
            || self.run_in_views(id, window, cx);
        if !ran {
            self.session.update(cx, |session, _| {
                session.run_command(id, serde_json::json!({}));
            });
        }
        cx.notify();
    }

    /// What the window does itself of command `id`: its regions, what
    /// opens over it, another project, a file to import, a preset. False
    /// when `id` is not one of these.
    fn run_in_window(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let session = self.session.clone();
        match id {
            "app.quit" => cx.quit(),
            "app.preferences" => self.show_settings(None, window, cx),
            "app.account" => self.show_settings(Some(Page::Account), window, cx),
            "view.sidebar" => self.set_sidebar(!self.sidebar_open, cx),
            "view.picture" => self.toggle_pane(true, cx),
            "view.timeline" => self.toggle_pane(false, cx),
            "window.project" => self.set_media(!self.media_open, cx),
            "window.assistant" => self
                .chat
                .update(cx, |chat, cx| chat.focus_composer(window, cx)),
            // Ask Kerf…: its popover over the chosen clip, else over the
            // timeline at the playhead.
            "ai.ask" => {
                let anchor = self.ask_anchor(window, cx);
                self.chat
                    .update(cx, |chat, cx| chat.open_ask(anchor, window, cx));
            }
            "file.import" => self.import(window, cx),
            "file.openProject" => self.open_project(window, cx),
            // The launcher, in a window of its own; what it opens takes this one's place.
            "file.newProject" => self.new_project(window, cx),
            "help.about" => self.show_settings(Some(Page::About), window, cx),
            // Where crash reports are kept, in the system's file manager.
            "help.crashReports" => match kerf_bridge::crash_reports().folder {
                Some(folder) => {
                    if let Err(error) = std::fs::create_dir_all(&folder) {
                        log::warn!(
                            "{}: the crash folder could not be made: {error}",
                            folder.display()
                        );
                    }
                    cx.reveal_path(&folder);
                }
                None => session.update(cx, |session, _| {
                    session.state.notify(
                        "There is nowhere to keep crash reports on this computer",
                        true,
                    )
                }),
            },
            "qc.run" => window.dispatch_action(
                Box::new(editor_finish::finish::OpenFinish { run_check: true }),
                cx,
            ),
            // Edit ▸ Focus Region…: its dialog over the window.
            "clip.focusRegion" => self.focus_region(window, cx),
            // Editing presets: Import asks for the file, then its dialog
            // comes over the window (through this entity: the window is
            // being updated now); Export asks where; Save asks the name.
            "file.applyPreset" => {
                let workspace = cx.entity().downgrade();
                editor_launcher::presets::import_preset(
                    session,
                    window,
                    cx,
                    move |view, window, cx| {
                        let _ = workspace.update(cx, |this, cx| this.open_modal(view, window, cx));
                    },
                );
            }
            "file.exportPreset" => editor_launcher::presets::export_preset(session, window, cx),
            "preset.save" => {
                let view = editor_launcher::presets::save_preset(session, window, cx);
                self.open_modal(view, window, cx);
            }
            _ => return false,
        }
        true
    }

    /// What the session does of command `id`: export, refresh, undo and
    /// redo, playback and its marks, the switches it keeps, Kerf's Stop,
    /// a motion path. False when `id` is not one of these.
    fn run_on_session(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let session = self.session.clone();
        match id {
            // The quality chosen in Export's menu, full size or a preview,
            // where kerfd puts it; the toolbar's Export renders it into
            // deliveries/ (`ExportChosen`).
            "file.export" => session.update(cx, |session, _| session.export_file()),
            "export.master" => session.update(cx, |session, _| {
                session.deliver(Quality::Master.wire());
            }),
            "file.refresh" => session.update(cx, |session, _| {
                session.send(Command::Refresh);
            }),
            "edit.undo" => session.update(cx, |session, _| session.undo()),
            "edit.redo" => session.update(cx, |session, _| session.redo()),
            "playback.toggle" => {
                // A skim ends first, so kerfd is back at the playhead before it plays.
                self.stage.update(cx, |stage, cx| stage.end_skim(cx));
                session.update(cx, |session, _| session.toggle_playback());
            }
            "playback.goToStart" => session.update(cx, |session, _| session.seek(0.0, false)),
            "playback.goToEnd" => session.update(cx, |session, _| {
                let end = session.state.program_length();
                session.seek(end, false);
            }),
            "playback.previousFrame" | "playback.nextFrame" => session.update(cx, |session, _| {
                let frames = if id == "playback.nextFrame" { 1 } else { -1 };
                let at = frame_step(session.state.position, session.timeline().rate(), frames);
                session.seek(at, false);
            }),
            // As the conversation's Stop: its work row says "Stopping…" at once.
            "ai.stop" => self.chat.update(cx, |chat, cx| chat.stop(cx)),
            "clip.startPath" => {
                let started = self.stage.update(cx, |stage, cx| stage.start_path(cx));
                if !started {
                    session.update(cx, |session, _| {
                        session
                            .state
                            .notify("Choose a picture clip with no motion path", true)
                    });
                }
            }
            "mark.in" => session.update(cx, |session, _| {
                let at = session.state.position;
                session.set_mark_in(Some(at));
            }),
            "mark.out" => session.update(cx, |session, _| {
                let at = session.state.position;
                session.set_mark_out(Some(at));
            }),
            "mark.clear" => session.update(cx, |session, _| {
                session.set_mark_in(None);
                session.set_mark_out(None);
            }),
            "view.safeZones" => session.update(cx, |session, _| {
                session.prefs.safe_zones = !session.prefs.safe_zones;
                session.save_prefs();
            }),
            "playback.loop" => session.update(cx, |session, _| session.toggle_loop()),
            "view.followKerf" => session.update(cx, |session, _| {
                session.prefs.follow_kerf = !session.prefs.follow_kerf;
                session.save_prefs();
            }),
            _ => return false,
        }
        true
    }

    /// What the views do of command `id`: the picture's zoom, the
    /// timeline's tools and tracks, the Media band, the format, the sync.
    /// False when `id` is not one of these.
    fn run_in_views(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let session = self.session.clone();
        match id {
            // The zoom's menu over the transport (⇧Z, View ▸ Viewer Zoom…).
            "viewer.zoom" => {
                self.reveal_picture(cx);
                self.stage
                    .update(cx, |stage, cx| stage.toggle_zoom_menu(window, cx));
            }
            // The timeline's own, from the menus and the palette (its keys
            // reach it first while it has them).
            tool if editor_timeline::timeline::Tool::from_command(tool).is_some() => {
                if let Some(tool) = editor_timeline::timeline::Tool::from_command(tool) {
                    self.timeline
                        .update(cx, |timeline, cx| timeline.set_tool(tool, cx));
                }
            }
            "sequence.snap" => self.timeline.update(cx, |timeline, cx| {
                let on = !timeline.snapping();
                timeline.set_snapping(on, cx)
            }),
            "view.fitTracks" => self
                .timeline
                .update(cx, |timeline, cx| timeline.fit_tracks(cx)),
            "view.resetTrackHeights" => self
                .timeline
                .update(cx, |timeline, cx| timeline.reset_track_heights(cx)),
            // Its two choices open under the tracks, the timeline brought
            // in for them.
            "timeline.addTrack" => {
                if !self.panel_open || self.timeline_shown.target() < 0.5 {
                    self.open_panel(false, true, scene::PANEL, cx);
                }
                self.timeline
                    .update(cx, |timeline, cx| timeline.open_add_track(window, cx));
            }
            // The Media band's search, the band brought in for it; its view.
            "bin.search" => {
                if !self.media_open {
                    self.set_media(true, cx);
                }
                self.media
                    .update(cx, |band, cx| band.focus_search(window, cx));
            }
            "bin.toggleView" => self.media.update(cx, |band, cx| band.toggle_view(cx)),
            // The toolbar's format: the sequence becomes that shape.
            "sequence.setFormat.169" | "sequence.setFormat.916" => {
                let index = usize::from(id == "sequence.setFormat.916");
                self.formats
                    .update(cx, |formats, cx| formats.choose(index, cx));
            }
            // What the sync needs words for asks for them first, in the branch
            // chip's menu (Publish's ways, a pull request's description);
            // Switch Branch opens it on the branches. Until the project's Git
            // status is known, a line of words is asked for over the window.
            "collab.commit"
            | "collab.newBranch"
            | "collab.pullRequest"
            | "collab.publish"
            | "collab.switchBranch" => {
                let opened = self
                    .collab
                    .update(cx, |collab, cx| collab.open_command(id, window, cx));
                if !opened {
                    if id == "collab.switchBranch" {
                        session.update(cx, |session, _| {
                            session
                                .state
                                .notify("Reading the project's Git status", false)
                        });
                    } else {
                        self.ask_sync_words(id, window, cx);
                    }
                }
            }
            // As the chip's Sync Now and Live Sync: the sync's end is said,
            // and Live Sync is remembered for the project.
            "collab.sync" => self.collab.update(cx, |collab, cx| collab.sync_now(cx)),
            "collab.live" => self.collab.update(cx, |collab, cx| collab.toggle_live(cx)),
            _ => return false,
        }
        true
    }

    /// The toolbar's Export (its main part): renders the quality chosen in
    /// its menu into deliveries/. While Kerf cannot export it says why, as
    /// Export Master would.
    fn export_chosen(&mut self, cx: &mut Context<Self>) {
        if let Some(reason) = self.unavailable("export.master", cx) {
            self.session.update(cx, |session, cx| {
                session.state.notify(&reason, true);
                cx.notify();
            });
            return;
        }
        self.session.update(cx, |session, cx| {
            let quality = session.export_quality;
            session.export(quality.wire());
            cx.notify();
        });
    }
}

/// One frame on from `position`, or `frames` back, on the frame grid at
/// `rate` frames a second: the playhead rounded to its frame first (after a
/// skim or the clock it can sit between frames), never before the start.
pub fn frame_step(position: f64, rate: f64, frames: i64) -> f64 {
    let rate = if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        30.0
    };
    (((position * rate).round() + frames as f64) / rate).max(0.0)
}
