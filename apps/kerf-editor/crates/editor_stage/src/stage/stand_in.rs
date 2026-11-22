//! Never a blank while a picture is coming: as Kerf builds a cut on an
//! empty program the newest clip it placed stands in for its render; with
//! no frame yet, the poster of the clip at the playhead ([`poster_source`]),
//! the newest crossfading in over the last; with no poster, words
//! ("Connecting to kerfd…", "No picture yet", the project's name).

use super::activity::kerf_cutting;
use super::StageView;
use editor_session::session::Session;
use editor_state::model;
use editor_ui::motion;
use editor_ui::theme::{self, motion as scene, text};
use editor_ui::ui;
use gpui::{div, prelude::*, px, App, Bounds, Context, Div, Pixels, RenderImage, SharedString};
use kerf_bridge::event::PreviewStatus;
use kerf_bridge::Event;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

/// The media whose poster stands in for the picture at `at` of
/// `timeline` until a frame comes: the topmost picture clip there with
/// media of its own (a title or a solid has none), else the first clip of
/// footage on the timeline. None with no footage.
pub fn poster_source(timeline: &model::Timeline, at: f64) -> Option<String> {
    let pictures = || {
        timeline
            .tracks
            .iter()
            .rev()
            .filter(|track| track.kind == model::TrackKind::Video)
    };
    pictures()
        .flat_map(|track| track.clips.iter())
        .find(|clip| clip.timeline_start <= at && at < clip.end() && clip.source_id.is_some())
        .or_else(|| {
            pictures()
                .flat_map(|track| track.clips.iter())
                .filter(|clip| clip.source_id.is_some())
                .min_by(|a, b| a.timeline_start.total_cmp(&b.timeline_start))
        })
        .and_then(|clip| clip.source_id.clone())
}

/// Of `timeline`'s picture clips with media, those not in `seen`, in the
/// order they lie (as a cut builds, each newly placed clip): their ids
/// and media.
pub fn placed_clips(seen: &HashSet<String>, timeline: &model::Timeline) -> Vec<(String, String)> {
    let mut placed: Vec<(f64, String, String)> = timeline
        .tracks
        .iter()
        .filter(|track| track.kind == model::TrackKind::Video)
        .flat_map(|track| track.clips.iter())
        .filter(|clip| !seen.contains(&clip.id))
        .filter_map(|clip| {
            Some((
                clip.timeline_start,
                clip.id.clone(),
                clip.source_id.clone()?,
            ))
        })
        .collect();
    placed.sort_by(|a, b| a.0.total_cmp(&b.0));
    placed
        .into_iter()
        .map(|(_, clip, source)| (clip, source))
        .collect()
}

/// Whether a bridge event releases the poster standing in for a cut Kerf
/// builds: a `preview` of `first_cut` (the cut shown on the empty program)
/// that has stopped running, so its render is what the picture shows.
pub fn releases_cover(event: &Event, first_cut: Option<&str>) -> bool {
    let Event::Preview(preview) = event else {
        return false;
    };
    !preview.status.as_str().is_empty()
        && preview.status != PreviewStatus::Running
        && first_cut.is_some_and(|cut| preview.proposal_id.as_deref() == Some(cut))
}

impl StageView {
    /// As Kerf builds a cut on an empty program, the newest clip it placed:
    /// the picture stands in as it until the render comes. The clips of a
    /// proposal already there when it is first seen are not new.
    pub(super) fn follow_cut(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let state = &session.state;
        let proposed = state
            .snapshot
            .proposal_timeline
            .as_ref()
            .filter(|_| !session.timeline().has_clips());
        let Some(proposed) = proposed else {
            self.cut_seen.clear();
            self.cut_newest = None;
            return;
        };
        let placed = placed_clips(&self.cut_seen, proposed);
        let first_sight = self.cut_seen.is_empty();
        for (clip, _) in &placed {
            self.cut_seen.insert(clip.clone());
        }
        if state.work.busy()
            && !first_sight
            && let Some((_, source)) = placed.last()
        {
            self.cut_newest = Some(source.clone());
            cx.notify();
        }
    }

    /// The media whose poster stands in for the picture now: as Kerf
    /// builds a cut the newest clip it placed; with no frame yet, the
    /// clip's at the playhead; none whose poster has not come.
    fn poster_media(&self, session: &Session) -> Option<String> {
        let timeline = session.timeline();
        let cutting = kerf_cutting(
            session.state.work.busy(),
            timeline.has_clips(),
            session.state.snapshot.proposal_timeline.is_some(),
        );
        let poster_media = if cutting {
            self.cut_newest.clone()
        } else if session.picture.is_none() {
            let shown = if timeline.has_clips() {
                Some(timeline)
            } else {
                session.state.snapshot.proposal_timeline.as_ref()
            };
            shown.and_then(|shown| poster_source(shown, session.state.position))
        } else {
            None
        };
        poster_media.filter(|media| session.artwork.poster(media).is_some())
    }

    /// Follows the poster standing in: a new one crossfades in over the
    /// one before, from now.
    pub(super) fn follow_posters(&mut self, cx: &mut Context<Self>) {
        let poster_media = self.poster_media(self.session.read(cx));
        if poster_media.as_deref()
            != self
                .stand_in_poster
                .as_ref()
                .map(|(media, _)| media.as_str())
        {
            self.stand_in_before = self.stand_in_poster.take().map(|(media, _)| media);
            self.stand_in_poster = poster_media.map(|media| (media, Instant::now()));
        }
    }

    /// The posters standing in now, each with how clear it is: the one
    /// before whole while the newest crossfades in over it over APPEAR
    /// (frames asked for meanwhile), at once under Reduce Motion.
    pub(super) fn posters(&self, window: &gpui::Window, cx: &App) -> Vec<(Arc<RenderImage>, f32)> {
        let session = self.session.read(cx);
        // The newest crossfades in over the one before, over APPEAR.
        let poster_in = self.stand_in_poster.as_ref().map_or(1.0, |(_, since)| {
            if motion::reduced() {
                return 1.0;
            }
            let t = (since.elapsed().as_secs_f32() / scene::APPEAR.as_secs_f32()).min(1.0);
            if t < 1.0 {
                window.request_animation_frame();
            }
            motion::ease(t)
        });
        [
            (poster_in < 1.0)
                .then_some(self.stand_in_before.as_deref())
                .flatten()
                .and_then(|media| session.artwork.poster(media))
                .map(|image| (image, 1.0)),
            self.stand_in_poster
                .as_ref()
                .and_then(|(media, _)| session.artwork.poster(media))
                .map(|image| (image, poster_in)),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// The words standing in for a picture with no frame (`framed`) and no
    /// poster (`postered`): until kerfd has said anything, with nothing on
    /// the program, else the project's name.
    pub(super) fn standing_in(
        &self,
        framed: bool,
        postered: bool,
        cx: &App,
    ) -> Option<SharedString> {
        let session = self.session.read(cx);
        if framed || postered {
            None
        } else if !session.state.has_snapshot {
            Some("Connecting to kerfd…".into())
        } else if !session.timeline().has_clips() {
            Some("No picture yet".into())
        } else {
            Some(session.project_name().into())
        }
    }
}

/// The posters standing in over `picture` (in the stage's own points), the
/// newest fading in over the last.
pub(super) fn poster_layers(
    posters: &[(Arc<RenderImage>, f32)],
    picture: Bounds<Pixels>,
) -> Vec<Div> {
    let (left, top) = (picture.origin.x, picture.origin.y);
    posters
        .iter()
        .enumerate()
        .map(|(index, (poster, level))| {
            div()
                .absolute()
                .left(left)
                .top(top)
                .w(picture.size.width)
                .h(picture.size.height)
                .rounded(px(theme::radius::CARD))
                .overflow_hidden()
                .opacity(*level)
                .child(
                    gpui::img(poster.clone())
                        .id(("stand-in", index))
                        .size_full()
                        .object_fit(gpui::ObjectFit::Cover),
                )
        })
        .collect()
}

/// The words standing in for a picture, in the middle of `picture`.
pub(super) fn words_over(words: SharedString, picture: Bounds<Pixels>) -> Div {
    div()
        .absolute()
        .left(picture.origin.x)
        .top(picture.origin.y)
        .w(picture.size.width)
        .h(picture.size.height)
        .flex()
        .items_center()
        .justify_center()
        .child(ui::label(words, text::BODY, theme::ink3()))
}
