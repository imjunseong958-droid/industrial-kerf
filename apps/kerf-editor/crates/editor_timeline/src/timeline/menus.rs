//! The timeline's menus, open over it where they were asked for: a clip's
//! (right-click), a head's (Sync lock, the heights) and the add-track
//! row's; and each command's keys in the keymap in use, which their rows
//! and the strip's tips say.

use super::heads::ADD_ROW;
use super::lanes::ClipKind;
use super::TimelineView;
use editor_session::commands::RunCommand;
use editor_state::model::{ClipShow, TrackKind};
use editor_state::track_heights::default_height;
use editor_ui::icons::Icon;
use editor_ui::theme::layout;
use editor_ui::ui;
use gpui::{
    point, prelude::*, px, App, Context, DismissEvent, Entity, Pixels, Point, SharedString,
    Subscription, Window,
};
use serde_json::Value;

/// A menu open over the timeline, where it opened.
pub(super) struct OpenMenu {
    pub(super) menu: Entity<ui::Menu>,
    pub(super) at: Point<Pixels>,
    _dismiss: Subscription,
}

impl TimelineView {
    /// Sequence ▸ Add Track… (`timeline.addTrack`): its two choices open
    /// under the tracks, from the keys.
    pub fn open_add_track(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = self.bounds.get();
        let y = (layout::RULER + self.lanes.height - self.scroll_y + ADD_ROW)
            .min(f32::from(bounds.size.height));
        let at = point(
            bounds.origin.x - px(layout::TRACK_HEADER) + px(8.0),
            bounds.origin.y + px(y),
        );
        self.add_track_menu(at, true, window, cx);
    }

    /// Each command's keys in the keymap in use, read again as the session
    /// says the keymap has changed.
    pub(super) fn follow_keymap(&mut self, cx: &App) {
        let keymap = self.session.read(cx).prefs.keymap.wire();
        if self.keys.0 != keymap {
            let keys = editor_session::commands::catalog(keymap)
                .into_iter()
                .map(|command| (command.id, command.keys))
                .collect();
            self.keys = (keymap.to_owned(), keys);
        }
    }

    /// Command `id`'s keys in the keymap in use: as last read
    /// ([`Self::follow_keymap`]), or read from the registry here when a
    /// keymap was bound since without the session saying.
    pub(super) fn keys_of(&self, id: &str, cx: &App) -> Option<String> {
        let keymap = self.session.read(cx).prefs.keymap.wire();
        if self.keys.0 == keymap {
            return self
                .keys
                .1
                .get(id)
                .and_then(|keys| keys.first())
                .map(|key| editor_state::shortcuts::shown_keys(key));
        }
        // The last of its rows, as the keys read in follow_keymap keep it.
        editor_session::commands::catalog(keymap)
            .into_iter()
            .rev()
            .find(|command| command.id == id)
            .and_then(|command| {
                command
                    .keys
                    .first()
                    .map(|key| editor_state::shortcuts::shown_keys(key))
            })
    }

    fn open_menu(
        &mut self,
        items: Vec<ui::MenuItem>,
        at: Point<Pixels>,
        width: f32,
        from_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx).width(width);
            if from_keys {
                menu.from_keys()
            } else {
                menu
            }
        });
        let dismiss = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            menu,
            at,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    /// A row of a menu that runs command `id`, greyed with why when it
    /// cannot run now.
    fn command_item(
        &self,
        id: &'static str,
        words: &'static str,
        states: &Value,
        cx: &App,
    ) -> ui::MenuItem {
        let mut item = ui::MenuItem::new(words)
            .on_choose(move |window, cx| window.dispatch_action(Box::new(RunCommand::new(id)), cx));
        if let Some(key) = self.keys_of(id, cx) {
            item = item.key(key);
        }
        if let Some(reason) = editor_session::palette::unavailable(states, id) {
            item = item.disabled(true).tip(reason.to_owned());
        }
        item
    }

    /// A clip's menu: Ask Kerf…, the edits on it, Attach to Its Shot or
    /// Detach, Focus Region…,
    /// Start Motion Path where it can take one, and for a shot made from
    /// steps Open Graph, or Carry Steps into the Document… while its
    /// steps are still in a recipe file.
    pub(super) fn clip_menu(
        &mut self,
        row: usize,
        index: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lanes = self.lanes.clone();
        let lane = &lanes.rows[row];
        let clip = &lane.clips[index];
        // Ask's state with Kerf's access too (no key, signed out), as the
        // window's menus have it: greyed with why, its key shown.
        let states = self
            .session
            .read(cx)
            .command_states(editor_session::session::cloud(cx));
        let mut items = vec![self
            .command_item("ai.ask", "Ask Kerf…", &states, cx)
            .icon(Icon::Sparkle)];
        items.push(
            self.command_item("clip.split", "Split at Playhead", &states, cx)
                .separated(),
        );
        items.push(self.command_item("clip.delete", "Delete", &states, cx));
        items.push(self.command_item("clip.rippleDelete", "Ripple Delete", &states, cx));
        items.push(self.command_item("clip.extract", "Extract", &states, cx));
        // One that follows a clip can let go, and one off the shots' own track can follow
        // its shot.
        let on_shots_track = self
            .session
            .read(cx)
            .timeline()
            .tracks
            .iter()
            .find(|track| track.kind == TrackKind::Video)
            .is_some_and(|track| track.id == lane.id);
        if clip.clip.follows.is_some() {
            items.push(
                self.command_item("clip.detach", "Detach", &states, cx)
                    .separated(),
            );
        } else if !on_shots_track {
            items.push(
                self.command_item("clip.attach", "Attach to Its Shot", &states, cx)
                    .separated(),
            );
        }
        items.push(
            self.command_item("clip.focusRegion", "Focus Region…", &states, cx)
                .separated(),
        );
        let path = clip
            .clip
            .prop("path")
            .as_str()
            .is_some_and(|path| !path.trim().is_empty());
        if lane.kind == TrackKind::Video && clip.kind != ClipKind::Caption && !path {
            items.push(self.command_item("clip.startPath", "Start Motion Path", &states, cx));
        }
        let source = clip
            .source
            .as_deref()
            .and_then(|id| self.session.read(cx).timeline().source(id).cloned());
        if let Some(source) = source {
            if source.uncarried() {
                let media: SharedString = source.id.clone().into();
                let recipe = source.recipe.clone().unwrap_or_default();
                items.push(
                    ui::MenuItem::new("Carry Steps into the Document…")
                        .separated()
                        .tip(format!(
                            "Write the steps of {recipe} under {}'s line in the document; what would be written is shown first",
                            source.id
                        ))
                        .on_choose(move |window, cx| {
                            window.dispatch_action(
                                Box::new(editor_making::making::MakeMedia {
                                    media: media.clone(),
                                    rolls: 1,
                                }),
                                cx,
                            )
                        }),
                );
            } else if source.is_made() {
                let media: SharedString = source.id.clone().into();
                items.push(ui::MenuItem::new("Open Graph").separated().on_choose(
                    move |window, cx| {
                        window.dispatch_action(
                            Box::new(editor_making::making::OpenShotGraph {
                                media: media.clone(),
                            }),
                            cx,
                        )
                    },
                ));
            }
        }
        self.open_menu(items, at, 232.0, false, window, cx);
    }

    /// A head's menu: Sync lock, its height, every track's, Fit tracks to
    /// panel and Reset track heights.
    pub(super) fn head_menu(
        &mut self,
        row: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.offline(cx) {
            return;
        }
        let lanes = self.lanes.clone();
        let Some(lane) = lanes.rows.get(row) else {
            return;
        };
        let this = cx.entity().downgrade();
        let (track, synced) = (lane.id.clone(), lane.sync_locked);
        let mut items = Vec::new();
        {
            let this = this.clone();
            let track = track.clone();
            items.push(
                ui::MenuItem::new("Sync lock")
                    .checked(synced)
                    .disabled(lanes.proposed)
                    .tip("Ripple edits move this track")
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |view, cx| {
                            view.set_track(&track, "syncLocked", !synced, cx)
                        });
                    }),
            );
        }
        let presets = [
            ("Small", Some(layout::TRACK_SMALL)),
            ("Medium", None),
            ("Large", Some(layout::TRACK_LARGE)),
        ];
        let own: Vec<ui::MenuItem> = presets
            .iter()
            .map(|(name, height)| {
                let wanted = height.unwrap_or_else(|| {
                    self.heights
                        .all
                        .unwrap_or_else(|| default_height(lane.kind))
                });
                let this = this.clone();
                let track = track.clone();
                let height = *height;
                ui::MenuItem::new(*name)
                    .checked((lane.height - wanted).abs() < 0.5)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |view, cx| {
                            view.heights.set(&track, height);
                            view.heights_changed(cx);
                        });
                    })
            })
            .collect();
        items.push(ui::MenuItem::new("Track height").separated().submenu(own));
        let all: Vec<ui::MenuItem> = presets
            .iter()
            .map(|(name, height)| {
                let every = lanes.rows.iter().all(|row| {
                    let wanted = height.unwrap_or_else(|| default_height(row.kind));
                    (row.height - wanted).abs() < 0.5
                });
                let this = this.clone();
                let height = *height;
                ui::MenuItem::new(*name)
                    .checked(every)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |view, cx| match height {
                            Some(height) => {
                                view.heights.set_all(Some(height));
                                view.heights_changed(cx);
                            }
                            None => view.reset_track_heights(cx),
                        });
                    })
            })
            .collect();
        items.push(ui::MenuItem::new("All tracks").submenu(all));
        {
            let this = this.clone();
            items.push(
                ui::MenuItem::new("Fit tracks to panel")
                    .separated()
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |view, cx| view.fit_tracks(cx));
                    }),
            );
        }
        items.push(
            ui::MenuItem::new("Reset track heights").on_choose(move |_, cx| {
                let _ = this.update(cx, |view, cx| view.reset_track_heights(cx));
            }),
        );
        self.open_menu(items, at, 220.0, false, window, cx);
    }

    /// The add-track row's choices: a picture track or a sound track.
    pub(super) fn add_track_menu(
        &mut self,
        at: Point<Pixels>,
        from_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let states = kerf_bridge::commands_states(&self.session.read(cx).command_context());
        let items = vec![
            self.command_item("sequence.addVideoTrack", "Add video track", &states, cx),
            self.command_item("sequence.addAudioTrack", "Add audio track", &states, cx),
        ];
        self.open_menu(items, at, 200.0, from_keys, window, cx);
    }
}
