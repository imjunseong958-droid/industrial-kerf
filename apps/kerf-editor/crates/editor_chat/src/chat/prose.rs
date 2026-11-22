//! What Kerf said, set for reading ([`ChatView::prose`]): paragraphs (their
//! line breaks kept), lists, headings and code, with bold, code and each
//! timecode it names a link that moves the playhead there. Its words can
//! be chosen and copied as one run.

use super::spans::{spans, timecode_position, Span};
use super::ChatView;
use editor_session::commands::RevealPicture;
use editor_ui::selectable;
use editor_ui::theme::{self, text};
use gpui::{div, prelude::*, px, AnyElement, App, ElementId, FontWeight, Rgba, TextRun};

impl ChatView {
    /// What Kerf said, set for reading: paragraphs (their line breaks kept),
    /// lists, headings and code, with bold, code and timecode links inline.
    /// Its words can be chosen with the pointer as one run, `key`'s.
    pub(super) fn prose(&self, words: &str, key: &str, ink: Rgba, cx: &App) -> gpui::Div {
        let mut column = div().flex().flex_col().gap(px(theme::space::S8));
        let mut code: Option<Vec<String>> = None;
        let mut paragraph: Vec<String> = Vec::new();
        // The pieces so far, and whether the last was a list's item.
        let mut piece = 0usize;
        let mut listing = false;
        for line in words.lines() {
            if line.trim_start().starts_with("```") {
                match code.take() {
                    Some(lines) => {
                        column = column.child(self.code_block(key, piece, lines));
                        piece += 1;
                    }
                    None => {
                        column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                        code = Some(Vec::new());
                    }
                }
                listing = false;
                continue;
            }
            if let Some(lines) = code.as_mut() {
                lines.push(line.to_owned());
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                listing = false;
            } else if let Some(item) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
            {
                column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                let before = if listing { "\n" } else { "\n\n" };
                column = column.child(
                    div()
                        .flex()
                        .gap(px(theme::space::S8))
                        .child(theme::reading(div()).text_color(theme::ink3()).child("•"))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.rich(key, piece, before, item, ink, cx)),
                        ),
                );
                piece += 1;
                listing = true;
            } else if let Some(heading) = trimmed.strip_prefix('#') {
                column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                let words: String = spans(heading.trim_start_matches('#').trim())
                    .into_iter()
                    .map(|span| match span {
                        Span::Plain(words)
                        | Span::Bold(words)
                        | Span::Code(words)
                        | Span::Timecode(words) => words,
                    })
                    .collect();
                column = column.child(
                    theme::type_style(div(), text::TITLE)
                        .text_color(theme::ink())
                        .child(selectable::selectable(
                            &self.selections,
                            ElementId::Name(format!("{key}-{piece}").into()),
                            key.to_owned(),
                            piece,
                            words,
                        )),
                );
                piece += 1;
                listing = false;
            } else {
                if listing {
                    column = self.flush(column, key, &mut piece, &mut paragraph, ink, cx);
                }
                paragraph.push(trimmed.to_owned());
                listing = false;
            }
        }
        if let Some(lines) = code {
            column = column.child(self.code_block(key, piece, lines));
            piece += 1;
        }
        self.flush(column, key, &mut piece, &mut paragraph, ink, cx)
    }

    /// The paragraph gathered so far, as a piece of `key`'s run.
    fn flush(
        &self,
        column: gpui::Div,
        key: &str,
        piece: &mut usize,
        paragraph: &mut Vec<String>,
        ink: Rgba,
        cx: &App,
    ) -> gpui::Div {
        if paragraph.is_empty() {
            return column;
        }
        let text = paragraph.join("\n");
        paragraph.clear();
        let element = self.rich(key, *piece, "\n\n", &text, ink, cx);
        *piece += 1;
        column.child(element)
    }

    /// A paragraph of what Kerf wrote, piece `piece` of `key`'s run (after
    /// `before` when copied with the one before): bold in the medium face,
    /// code in mono, and each timecode a mono link in accent ink that moves
    /// the playhead there and brings the picture in. Its words can be chosen.
    fn rich(
        &self,
        key: &str,
        piece: usize,
        before: &'static str,
        words: &str,
        ink: Rgba,
        cx: &App,
    ) -> AnyElement {
        let timeline = self.session.read(cx).timeline();
        let plain = theme::prose_font(FontWeight::NORMAL);
        let strong = theme::prose_font(FontWeight::MEDIUM);
        let mono = theme::mono_font(FontWeight::NORMAL);
        let mut shown = String::new();
        let mut runs = Vec::new();
        let mut links = Vec::new();
        let mut moments = Vec::new();
        for span in spans(words) {
            let (words, font, color, ground) = match &span {
                Span::Plain(words) => (words, plain.clone(), ink, None),
                Span::Bold(words) => (words, strong.clone(), ink, None),
                Span::Code(words) => (words, mono.clone(), theme::ink2(), Some(theme::well())),
                Span::Timecode(words) => (words, mono.clone(), theme::accent_ink(), None),
            };
            if words.is_empty() {
                continue;
            }
            let start = shown.len();
            shown.push_str(words);
            if let Span::Timecode(timecode) = &span {
                if let Some(seconds) = timecode_position(timecode, timeline) {
                    links.push(start..shown.len());
                    moments.push(seconds);
                }
            }
            runs.push(TextRun {
                len: words.len(),
                font,
                color: color.into(),
                background_color: ground.map(Into::into),
                underline: None,
                strikethrough: None,
            });
        }
        let mut words = selectable::selectable(
            &self.selections,
            ElementId::Name(format!("{key}-{piece}").into()),
            key.to_owned(),
            piece,
            shown,
        )
        .runs(runs)
        .before(before);
        if !links.is_empty() {
            // A press on a link that does not drag goes there.
            let session = self.session.clone();
            words = words.links(links, move |link, window, cx| {
                let Some(at) = moments.get(link).copied() else {
                    return;
                };
                session.update(cx, |session, cx| {
                    session.seek(at, false);
                    cx.notify();
                });
                window.dispatch_action(Box::new(RevealPicture), cx);
            });
        }
        theme::reading(div())
            .text_color(ink)
            .child(words)
            .into_any_element()
    }

    /// Code Kerf wrote, in mono on the well: piece `piece` of `key`'s run.
    fn code_block(&self, key: &str, piece: usize, lines: Vec<String>) -> gpui::Div {
        theme::type_style(div(), text::TIMECODE)
            .p(px(10.0))
            .rounded(px(theme::radius::CONTROL))
            .bg(theme::well())
            .text_color(theme::ink2())
            .child(selectable::selectable(
                &self.selections,
                ElementId::Name(format!("{key}-{piece}").into()),
                key.to_owned(),
                piece,
                lines.join("\n"),
            ))
    }
}
