//! What sits on the frame as it is drawn, kept inside the picture so it
//! stays in sight zoomed in: the caption at the playhead, its foot on title
//! safe (one line on a wide picture, balanced lines on a vertical one), and
//! the last finished check's caption-overlap box with its measure, fading
//! in and out as the playhead comes onto the finding and leaves it.

use super::overlay;
use super::picture::{wide, zoomed, TITLE_SAFE};
use super::StageView;
use editor_ui::theme::{self, motion as scene, over_picture, text};
use editor_ui::ui;
use gpui::{
    div, point, prelude::*, px, size, App, Bounds, Context, Div, Pixels, SharedString, TextRun,
    Window,
};

/// Points either side of the caption's words, and over and under them.
const CAPTION_PAD: (f32, f32) = (8.0, 2.0);

impl StageView {
    /// Where the caption at the playhead sits over `picture` (in the
    /// stage's own points): centred, its foot on title safe. On a wide
    /// picture it is one line cut at title safe; on a vertical, square or
    /// near-square one ([`wide`]) it wraps within title safe, balanced: the
    /// narrowest width that keeps the lines the widest needs.
    fn caption_box(
        &self,
        window: &Window,
        picture: Bounds<Pixels>,
        vertical: bool,
        words: SharedString,
    ) -> Bounds<Pixels> {
        let style = text::CAPTION;
        let (w, h) = (
            f32::from(picture.size.width),
            f32::from(picture.size.height),
        );
        let (tx, ty) = ((w * TITLE_SAFE).round(), (h * TITLE_SAFE).round());
        let runs = [TextRun {
            len: words.len(),
            font: theme::ui_font(style.weight),
            color: over_picture::caption_ink().into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let font_size = px(style.px);
        let (across, lines) = if vertical {
            let most = (w - 2.0 * tx - 16.0 - 2.0 * CAPTION_PAD.0).max(1.0);
            // Worked out again only for other words or another room.
            let key = (words.clone(), most.round() as i32);
            let kept = self
                .caption_fit
                .borrow()
                .as_ref()
                .filter(|(cached, at, _, _)| (cached, at) == (&key.0, &key.1))
                .map(|(_, _, across, lines)| (*across, *lines));
            match kept {
                Some(fit) => fit,
                None => {
                    let lines_at = |width: f32| {
                        window
                            .text_system()
                            .shape_text(words.clone(), font_size, &runs, Some(px(width)), None)
                            .map(|lines| {
                                lines
                                    .iter()
                                    .map(|line| line.wrap_boundaries().len() + 1)
                                    .sum::<usize>()
                            })
                            .unwrap_or(1)
                            .max(1)
                    };
                    let lines = lines_at(most);
                    let (mut low, mut high) = (1.0_f32, most);
                    for _ in 0..24 {
                        if high - low <= 0.5 {
                            break;
                        }
                        let middle = (low + high) / 2.0;
                        if lines_at(middle) <= lines {
                            high = middle;
                        } else {
                            low = middle;
                        }
                    }
                    let fit = (high.ceil().min(most), lines);
                    *self.caption_fit.borrow_mut() = Some((key.0, key.1, fit.0, fit.1));
                    fit
                }
            }
        } else {
            let most = (w - 2.0 * tx - 2.0 * CAPTION_PAD.0).max(1.0);
            let shaped = window
                .text_system()
                .shape_line(words, font_size, &runs, None);
            (f32::from(shaped.width).ceil().min(most), 1)
        };
        let shown = size(
            px(across + 2.0 * CAPTION_PAD.0),
            px(lines as f32 * style.line + 2.0 * CAPTION_PAD.1),
        );
        Bounds::new(
            point(
                picture.origin.x + px(((w - f32::from(shown.width)) / 2.0).round()),
                picture.origin.y + px(h - ty) - shown.height,
            ),
            shown,
        )
    }

    /// Follows the last finished check's caption-overlap box at the
    /// playhead: it fades in as the playhead comes onto a finding with a
    /// frame shown, and out as it leaves, the last one kept while it goes.
    pub(super) fn follow_overlap(&mut self, cx: &mut Context<Self>) {
        let (playhead, fps, framed) = {
            let session = self.session.read(cx);
            (
                session.state.position,
                session.timeline().rate(),
                session.picture.is_some(),
            )
        };
        let found = self
            .check
            .as_ref()
            .and_then(|check| overlay::overlap_at(check, playhead, fps))
            .filter(|_| framed);
        if found.is_some() != (self.overlap_level.target() > 0.5) {
            self.overlap_level
                .go(if found.is_some() { 1.0 } else { 0.0 }, scene::FADE);
        }
        if found.is_some() {
            self.overlap = found;
        }
    }

    /// The caption and the check's box over the frame as drawn over `card`
    /// (the picture, in the stage's own points), kept inside the picture:
    /// zoomed in, the frame is bigger than the picture, and they go with
    /// its edges.
    pub(super) fn on_frame(
        &self,
        card: Bounds<Pixels>,
        aspect: f32,
        frame_size: (u32, u32),
        window: &mut Window,
        cx: &App,
    ) -> Div {
        let frame_at = zoomed(card, frame_size, self.zoom, window.scale_factor());
        // In the picture's own points, as the layer kept to it places them.
        let picture = Bounds::new(
            point(
                frame_at.origin.x - card.origin.x,
                frame_at.origin.y - card.origin.y,
            ),
            frame_at.size,
        );
        let (left, top) = (picture.origin.x, picture.origin.y);
        let mut on_frame = div()
            .absolute()
            .left(card.origin.x)
            .top(card.origin.y)
            .w(card.size.width)
            .h(card.size.height)
            .overflow_hidden();
        let words: SharedString = self
            .session
            .read(cx)
            .state
            .caption
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .into();
        let caption_box = (!words.is_empty())
            .then(|| self.caption_box(window, picture, !wide(aspect), words.clone()));
        if let Some(shown) = caption_box {
            on_frame = on_frame.child(
                div()
                    .absolute()
                    .left(shown.origin.x)
                    .top(shown.origin.y)
                    .w(shown.size.width)
                    .h(shown.size.height)
                    .px(px(CAPTION_PAD.0))
                    .py(px(CAPTION_PAD.1))
                    .bg(over_picture::caption_ground())
                    .child(
                        theme::type_style(div(), text::CAPTION)
                            .w_full()
                            .text_center()
                            .text_color(over_picture::caption_ink())
                            .map(|line| {
                                if wide(aspect) {
                                    line.whitespace_nowrap()
                                        .overflow_hidden()
                                        .child(ui::cut_line(words))
                                } else {
                                    line.child(words)
                                }
                            }),
                    ),
            );
        }
        // The last finished check's caption-overlap box at the playhead,
        // fading in and out.
        let level = self.overlap_level.sample(window);
        if let Some(overlap) = self.overlap.clone().filter(|_| level > 0.001) {
            let [x, y, w, h] = overlap.rect;
            let (across, down) = (
                f64::from(f32::from(picture.size.width)),
                f64::from(f32::from(picture.size.height)),
            );
            let found_box = Bounds::new(
                point(left + px((x * across) as f32), top + px((y * down) as f32)),
                size(px((w * across) as f32), px((h * down) as f32)),
            );
            on_frame = on_frame.child(
                div()
                    .absolute()
                    .left(found_box.origin.x)
                    .top(found_box.origin.y)
                    .w(found_box.size.width)
                    .h(found_box.size.height)
                    .bg(over_picture::overlap())
                    .border_1()
                    .border_color(theme::neg())
                    .opacity(level),
            );
            if let Some(shown) = caption_box {
                // The caption the note is about, outlined 2 points out.
                on_frame = on_frame.child(
                    div()
                        .absolute()
                        .left(shown.origin.x - px(3.0))
                        .top(shown.origin.y - px(3.0))
                        .w(shown.size.width + px(6.0))
                        .h(shown.size.height + px(6.0))
                        .border_1()
                        .border_color(theme::neg())
                        .opacity(level),
                );
            }
            if !overlap.tag.is_empty() {
                // Over the caption's right end, 4 points up; without a
                // caption, over the box's.
                let anchor = caption_box.unwrap_or(found_box);
                on_frame = on_frame.child(
                    div()
                        .absolute()
                        .right(card.size.width - (anchor.origin.x + anchor.size.width))
                        .top(anchor.origin.y - px(4.0 + 16.0))
                        .child(
                            theme::type_style(div(), text::MONO_SMALL)
                                .h(px(16.0))
                                .px(px(theme::space::S4))
                                .flex()
                                .items_center()
                                .rounded(px(theme::radius::SMALL))
                                .bg(theme::chrome())
                                .text_color(theme::neg())
                                .child(overlap.tag),
                        )
                        .opacity(level),
                );
            }
        }
        on_frame
    }
}
