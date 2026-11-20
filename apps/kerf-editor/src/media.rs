//! The Media band: the project's media as tiles, between the picture and
//! the timeline. It comes down out from under the picture when opened
//! (the workspace moves it). A tile being made has amber stripes moving
//! across it and says "Making take 2 · 0:41"; a media made from steps with
//! no file yet is a dashed hole saying "Not made".
//!
//! Each tile has a menu, by a right-click or its ⋯ on hover: Make… and Make
//! Several, Stop Making while it is made, Open Graph, Carry Steps into the
//! Document for a media whose steps are still in a recipe file, Replace
//! Media…, and Show in Folder. What a making runs is shown first: the
//! workspace opens its plan and its graph over the window.

use crate::icons::Icon;
use crate::making::{MakeMedia, OpenShotGraph};
use crate::model::{self, Source};
use crate::motion;
use crate::session::Session;
use crate::theme::{self, motion as scene, text};
use crate::ui::{self, icon_button};
use gpui::{
    anchored, canvas, deferred, div, point, prelude::*, px, AnimationExt, AnyElement, Bounds,
    Context, ElementId, Entity, MouseButton, MouseDownEvent, PathBuilder, PathPromptOptions,
    Pixels, Point, SharedString, Window,
};
use serde_json::json;
use std::path::PathBuf;

/// Amber stripes across `bounds`, `phase` (0..1) of the way to the next:
/// what is being made, on its tile, its graph's takes and its clips.
pub(crate) fn paint_stripes(window: &mut Window, bounds: Bounds<Pixels>, phase: f32) {
    let space = scene::STRIPE_SPACE;
    let shift = phase * space;
    let height = f32::from(bounds.size.height);
    let width = f32::from(bounds.size.width);
    let mut x = -height + shift;
    while x < width {
        let mut path = PathBuilder::stroke(px(space * 0.45));
        path.move_to(point(
            bounds.origin.x + px(x),
            bounds.origin.y + bounds.size.height,
        ));
        path.line_to(point(bounds.origin.x + px(x + height), bounds.origin.y));
        if let Ok(path) = path.build() {
            window.paint_path(path, theme::activity::gpu_stripe());
        }
        x += space;
    }
}

/// A tile's menu while open: which media, and where it was asked for.
struct Menu {
    media: String,
    at: Point<Pixels>,
}

pub struct MediaBand {
    session: Entity<Session>,
    menu: Option<Menu>,
    /// How many menus have opened, so each fades in as a new one.
    menus: u64,
}

impl MediaBand {
    pub fn new(session: Entity<Session>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            menu: None,
            menus: 0,
        }
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import".into()),
        });
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let _ = session.update(cx, |session, _| {
                for path in &paths {
                    session.import(path, true);
                }
            });
        })
        .detach();
    }

    /// Swaps the file behind `media` for one a person picks: every clip
    /// that uses it keeps its timing.
    fn replace(&mut self, media: String, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Replace".into()),
        });
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = session.update(cx, |session, _| {
                session.send(json!({"type": "media_replace", "media": media, "path": path.to_string_lossy()}));
            });
        })
        .detach();
    }

    fn open_menu(&mut self, media: &str, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.menus += 1;
        self.menu = Some(Menu {
            media: media.to_owned(),
            at,
        });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// The file behind a media, where the project keeps it.
    fn file_of(&self, source: &Source, cx: &Context<Self>) -> Option<PathBuf> {
        if source.path.is_empty() {
            return None;
        }
        let path = PathBuf::from(&source.path);
        if path.is_absolute() {
            return Some(path);
        }
        self.session
            .read(cx)
            .project
            .as_ref()
            .map(|project| project.join(path))
    }

    /// A tile's menu: what can be done with the media as it stands.
    fn menu(&self, menu: &Menu, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        let source = session.timeline().source(&menu.media)?.clone();
        let making = session.state.makings.contains_key(&source.id);
        let connected = session.state.connection == crate::store::Connection::Connected;
        let steps = source.steps.unwrap_or(0) > 0;
        let carry = source.recipe.is_some() && !steps;
        let file = self.file_of(&source, cx).filter(|file| file.exists());
        let media: SharedString = source.id.clone().into();

        let item = |key: &str, words: &str, icon: Icon, enabled: bool| {
            let ink = if enabled { theme::ink() } else { theme::dis() };
            theme::type_style(div(), text::MENU_QUIET)
                .id(ElementId::Name(format!("media-menu-{key}").into()))
                .flex()
                .items_center()
                .gap(px(8.0))
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(theme::radius::CONTROL))
                .text_color(ink)
                .when(enabled, |item| {
                    item.cursor_pointer()
                        .hover(|item| item.bg(theme::popup_hover()))
                })
                .child(icon.view(14.0, if enabled { theme::ink2() } else { theme::dis() }))
                .child(words.to_owned())
        };
        let mut items: Vec<AnyElement> = Vec::new();
        if making {
            let stop = source.id.clone();
            items.push(
                item("stop", "Stop Making", Icon::Stop, connected)
                    .when(connected, |item| {
                        item.on_click(cx.listener(move |this, _, _, cx| {
                            this.close_menu(cx);
                            this.session.update(cx, |session, _| {
                                session.send(json!({"type": "media_make_cancel", "media": stop}));
                            });
                        }))
                    })
                    .into_any_element(),
            );
        } else if steps {
            let make = media.clone();
            items.push(
                item("make", "Make…", Icon::Sparkle, connected)
                    .when(connected, |item| {
                        item.on_click(cx.listener(move |this, _, window, cx| {
                            this.close_menu(cx);
                            window.dispatch_action(
                                Box::new(MakeMedia {
                                    media: make.clone(),
                                    rolls: 1,
                                }),
                                cx,
                            );
                        }))
                    })
                    .into_any_element(),
            );
            // Several takes of the same shot to choose among, each with its seeds one on.
            items.push(
                theme::type_style(div(), text::MENU_QUIET)
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(28.0))
                    .pl(px(10.0))
                    .pr(px(4.0))
                    .text_color(if connected {
                        theme::ink()
                    } else {
                        theme::dis()
                    })
                    .child(Icon::Sparkle.view(
                        14.0,
                        if connected {
                            theme::ink2()
                        } else {
                            theme::dis()
                        },
                    ))
                    .child(div().flex_1().child("Make Several"))
                    .children([2u32, 4, 8].into_iter().map(|rolls| {
                        let media = media.clone();
                        theme::type_style(div(), text::MONO_SMALL)
                            .id(ElementId::Name(format!("media-menu-make-{rolls}").into()))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(26.0))
                            .h(px(22.0))
                            .rounded(px(theme::radius::SMALL))
                            .bg(theme::raised())
                            .text_color(if connected {
                                theme::ink()
                            } else {
                                theme::dis()
                            })
                            .child(format!("{rolls}"))
                            .when(connected, |chip| {
                                chip.cursor_pointer()
                                    .hover(|chip| chip.bg(theme::popup_hover()))
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| {
                                            ui::Tooltip(
                                                format!(
                                                    "Make {rolls} takes; the plan is shown first"
                                                )
                                                .into(),
                                            )
                                        })
                                        .into()
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.close_menu(cx);
                                        window.dispatch_action(
                                            Box::new(MakeMedia {
                                                media: media.clone(),
                                                rolls,
                                            }),
                                            cx,
                                        );
                                    }))
                            })
                    }))
                    .into_any_element(),
            );
        }
        if steps {
            let graph = media.clone();
            items.push(
                item("graph", "Open Graph", Icon::Media, true)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_menu(cx);
                        window.dispatch_action(
                            Box::new(OpenShotGraph {
                                media: graph.clone(),
                            }),
                            cx,
                        );
                    }))
                    .into_any_element(),
            );
        }
        if carry {
            // Its steps are still in the recipe file it names: what carrying
            // them writes is shown first, as a making's plan is.
            let carried = media.clone();
            items.push(
                item(
                    "carry",
                    "Carry Steps into the Document…",
                    Icon::Import,
                    connected && !making,
                )
                .when(connected && !making, |item| {
                    item.on_click(cx.listener(move |this, _, window, cx| {
                        this.close_menu(cx);
                        window.dispatch_action(
                            Box::new(MakeMedia {
                                media: carried.clone(),
                                rolls: 1,
                            }),
                            cx,
                        );
                    }))
                })
                .into_any_element(),
            );
        }
        if !items.is_empty() {
            items.push(ui::hairline(true).my(px(4.0)).into_any_element());
        }
        let replace = source.id.clone();
        items.push(
            item(
                "replace",
                "Replace Media…",
                Icon::Import,
                connected && !making,
            )
            .when(connected && !making, |item| {
                item.on_click(cx.listener(move |this, _, window, cx| {
                    this.close_menu(cx);
                    this.replace(replace.clone(), window, cx);
                }))
            })
            .into_any_element(),
        );
        let reveal = file.clone();
        items.push(
            item("reveal", "Show in Folder", Icon::Folder, file.is_some())
                .when_some(reveal, |item, file| {
                    item.on_click(cx.listener(move |this, _, _, cx| {
                        this.close_menu(cx);
                        cx.reveal_path(&file);
                    }))
                })
                .into_any_element(),
        );
        let key = self.menus;
        Some(
            deferred(
                anchored()
                    .position(menu.at)
                    .snap_to_window_with_margin(px(8.0))
                    .child(
                        div()
                            .w(px(240.0))
                            .p(px(4.0))
                            .rounded(px(theme::radius::CARD))
                            .bg(theme::popup())
                            .border_1()
                            .border_color(theme::line2())
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_menu(cx)))
                            .child(div().px(px(10.0)).py(px(4.0)).child(ui::line(
                                source.id.clone(),
                                text::META,
                                theme::ink3(),
                            )))
                            .children(items)
                            .with_animation(
                                ("media-menu", key),
                                motion::animation(scene::FADE),
                                |menu, t| menu.opacity(t),
                            ),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn tile(
        &self,
        source: &Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let session = self.session.read(cx);
        let chosen = session.chosen_media.as_deref() == Some(source.id.as_str());
        let making = session.state.makings.get(&source.id).cloned();
        let id = source.id.clone();
        let made_but_missing = source.is_made() && source.path.is_empty();
        let video = source.width.is_some();
        let kind_icon = if video { Icon::Film } else { Icon::Mute };
        let length = model::length(source.duration);
        let name = if source.path.is_empty() {
            source.id.clone()
        } else {
            source.file_name().to_owned()
        };
        let menu_open = self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.media == source.id);

        let mut picture = div()
            .relative()
            .h(px(78.0))
            .w_full()
            .rounded(px(theme::radius::SMALL))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::card())
            .child(kind_icon.view(20.0, theme::ink3()));
        if made_but_missing && making.is_none() {
            picture = picture
                .bg(theme::well())
                .border_1()
                .border_dashed()
                .border_color(theme::line2())
                .child(div().absolute().bottom(px(6.0)).child(ui::label(
                    "Not made",
                    text::META,
                    theme::ink3(),
                )));
        }
        if let Some(making) = &making {
            // Amber stripes moving across it, drawn again only while they move.
            let phase = motion::phase(making.since, scene::STRIPE_CYCLE);
            if !motion::reduced() {
                window.request_animation_frame();
            }
            picture = picture.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| paint_stripes(window, bounds, phase),
                )
                .absolute()
                .inset_0(),
            );
            let elapsed = making.since.elapsed().as_secs();
            picture = picture.child(div().absolute().bottom(px(6.0)).child(ui::chip(
                format!(
                    "Making take {} · {}:{:02}",
                    making.take,
                    elapsed / 60,
                    elapsed % 60
                ),
                theme::activity::gpu(),
            )));
        } else {
            picture = picture.child(
                div()
                    .absolute()
                    .right(px(4.0))
                    .bottom(px(4.0))
                    .px(px(4.0))
                    .rounded(px(3.0))
                    .bg(theme::activity::chip())
                    .child(ui::label(length, text::BADGE, theme::ink())),
            );
        }
        // Its ⋯, on hover or while its menu is open.
        let more = id.clone();
        picture = picture.child(
            div()
                .absolute()
                .top(px(4.0))
                .right(px(4.0))
                .rounded(px(theme::radius::CONTROL))
                .bg(theme::activity::chip())
                .invisible()
                .group_hover(SharedString::from(format!("tile-{id}")), |more| {
                    more.visible()
                })
                .when(menu_open, |more| more.visible())
                .child(
                    icon_button(
                        ElementId::Name(format!("media-more-{id}").into()),
                        Icon::More,
                    )
                    .size(22.0)
                    .tooltip("More")
                    .on_click(cx.listener(
                        move |this, event: &gpui::ClickEvent, _, cx| {
                            cx.stop_propagation();
                            if this.menu.as_ref().is_some_and(|menu| menu.media == more) {
                                this.close_menu(cx);
                            } else {
                                this.open_menu(&more, event.position(), cx);
                            }
                        },
                    )),
                ),
        );
        let choose = id.clone();
        let right_click = id.clone();
        div()
            .id(ElementId::Name(format!("media-{id}").into()))
            .group(SharedString::from(format!("tile-{id}")))
            .flex_none()
            .w(px(140.0))
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .rounded(px(theme::radius::CONTROL))
            .cursor_pointer()
            .when(chosen, |tile| {
                tile.bg(theme::on())
                    .border_1()
                    .border_color(theme::accent())
            })
            .when(!chosen, |tile| tile.hover(|tile| tile.bg(theme::raised())))
            .child(picture)
            .child(ui::line(
                name,
                text::META,
                if chosen { theme::ink() } else { theme::ink2() },
            ))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.session.update(cx, |session, _| {
                        session.chosen_media = Some(right_click.clone())
                    });
                    this.open_menu(&right_click, event.position, cx);
                }),
            )
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                let insert_now = event.click_count() >= 2;
                this.session.update(cx, |session, cx| {
                    session.chosen_media = Some(choose.clone());
                    if insert_now {
                        // A double-click places it at the playhead.
                        session.run_command("source.insert", json!({}));
                    }
                    cx.notify();
                });
            }))
            .with_animation(
                ElementId::Name(format!("media-pop-{id}").into()),
                motion::animation(scene::POP),
                |tile, t| tile.opacity(t),
            )
    }
}

impl Render for MediaBand {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sources = self.session.read(cx).timeline().sources.clone();
        // A menu of a media that is gone goes with it.
        if self
            .menu
            .as_ref()
            .is_some_and(|menu| !sources.iter().any(|source| source.id == menu.media))
        {
            self.menu = None;
        }
        let tiles: Vec<_> = sources
            .iter()
            .map(|source| self.tile(source, window, cx).into_any_element())
            .collect();
        let menu = match &self.menu {
            Some(menu) => {
                let menu = Menu {
                    media: menu.media.clone(),
                    at: menu.at,
                };
                self.menu(&menu, cx)
            }
            None => None,
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::chrome())
            .border_t_1()
            .border_color(theme::line())
            .child(
                div()
                    .h(px(theme::layout::STRIP))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .child(ui::label("Media", text::CONTROL_STRONG, theme::ink2()))
                    .child(ui::label(
                        format!("{}", sources.len()),
                        text::MONO_SMALL,
                        theme::ink3(),
                    ))
                    .child(div().flex_1())
                    .child(
                        ui::button("import-media", "Import")
                            .small()
                            .ghost()
                            .icon(Icon::Import)
                            .on_click(cx.listener(|this, _, window, cx| this.import(window, cx))),
                    ),
            )
            .child(
                div()
                    .id("media-tiles")
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .flex()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .pb(px(6.0))
                    .children(tiles)
                    .when(sources.is_empty(), |band| {
                        band.items_center().justify_center().child(ui::label(
                            "No media yet. Import footage, or ask Kerf to make a shot.",
                            text::BODY,
                            theme::ink3(),
                        ))
                    }),
            )
            .children(menu)
    }
}
