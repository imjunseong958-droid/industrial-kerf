//! The chosen clip's inspector, beside the picture: its name and where it
//! sits, a generator's parameters, its properties, its effects and their
//! parameters, and its masks. And the toolbar's format bank.
//!
//! Nothing here keeps an editable copy of the clip. Every field draws what
//! the newest snapshot says (its timeline, kept from the bridge's `state`
//! event), and a change goes back to kerfd as one typed edit: `set`,
//! `fx-add`, `fx-set`, `fx-remove` or `set-sequence`. While a slider is
//! dragged it shows where the pointer has it; the picture changes only when
//! the edit comes back as a new snapshot.

mod catalog;
mod choice;
mod clip;
mod effects;
mod fields;
mod format_bank;
mod number;
mod sections;
mod values;

pub use choice::{
    choice_is_compact, choice_menu_opens_up, choice_menu_shift, choice_menu_stays, choice_rows,
    choice_sends, scroll_to_show,
};
pub use format_bank::{format_beside, format_index, FormatBank};

use crate::input::{InputEvent, TextInput};
use crate::model::Timeline;
use crate::session::{BridgeEvent, Session};
use crate::theme::{self, radius, space, text};
use crate::ui;
use choice::ChoiceMenu;
use clip::{Field, Inspected};
use effects::Adding;
use fields::TextField;
use gpui::{
    div, prelude::*, px, AnyElement, Bounds, Context, Entity, FocusHandle, Focusable, MouseButton,
    Pixels, ScrollHandle, Subscription, Window,
};
use kerf_bridge::{Command, Event};
use number::{Drag, Pending, Typing};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The chosen clip's inspector, beside the picture.
pub struct InspectorView {
    session: Entity<Session>,
    /// The newest snapshot's timeline, as the bridge sent it.
    timeline: Option<Timeline>,
    revision: u64,
    asked_for_state: bool,
    /// The clip the fields are for.
    clip: Option<String>,
    name: Entity<TextInput>,
    name_sent: Option<String>,
    number: Entity<TextInput>,
    typing: Option<Typing>,
    drag: Option<Drag>,
    pending: Option<Pending>,
    texts: HashMap<Field, TextField>,
    /// Where each slider's rail was drawn, left and width.
    rails: Rc<RefCell<HashMap<Field, (f32, f32)>>>,
    /// Groups opened or closed by hand.
    groups: HashMap<String, bool>,
    /// Effects folded to their heads, by their place in the stack.
    folded: HashSet<usize>,
    /// The colour whose palette is open.
    palette: Option<Field>,
    /// The choice whose menu (the kit's) is open.
    choice_menu: Option<ChoiceMenu>,
    /// Each choice button's (a Tab stop), by its field; given the keys, it
    /// is scrolled into sight.
    choice_focus: HashMap<Field, (FocusHandle, Subscription)>,
    /// Where each choice button was laid out: its menu opens there, as wide.
    choice_bounds: Rc<RefCell<HashMap<Field, Bounds<Pixels>>>>,
    /// Return or Space went down on this choice's button: as it comes up,
    /// its menu opens.
    choice_armed: Option<Field>,
    adding: Option<Adding>,
    addings_opened: u64,
    /// The body's scroll, back at the top for each clip chosen.
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl InspectorView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| TextInput::new(cx, "Clip name").with_style(text::BODY_NAME));
        let number = cx.new(|cx| TextInput::new(cx, "").with_style(text::TIMECODE));
        let name_focus = name.focus_handle(cx);
        let number_focus = number.focus_handle(cx);
        let playhead = session.read(cx).playhead.clone();
        let subscriptions = vec![
            cx.observe(&session, Self::session_notified),
            // What the clip is at the playhead.
            cx.observe(&playhead, |_, _, cx| cx.notify()),
            cx.subscribe(&session, Self::bridge_event),
            cx.subscribe_in(
                &name,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.commit_name(cx),
                    InputEvent::Cancel => {
                        this.revert_name(cx);
                        window.blur();
                    }
                    InputEvent::Changed => {}
                },
            ),
            cx.on_blur(&name_focus, window, |this, _, cx| this.commit_name(cx)),
            cx.subscribe_in(
                &number,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Submit => this.commit_typed(cx),
                    InputEvent::Cancel => {
                        this.typing = None;
                        cx.notify();
                    }
                    InputEvent::Changed => {}
                },
            ),
            cx.on_blur(&number_focus, window, |this, _, cx| this.commit_typed(cx)),
        ];
        Self {
            session,
            timeline: None,
            revision: 0,
            asked_for_state: false,
            clip: None,
            name,
            name_sent: None,
            number,
            typing: None,
            drag: None,
            pending: None,
            texts: HashMap::new(),
            rails: Rc::default(),
            groups: HashMap::new(),
            folded: HashSet::new(),
            palette: None,
            choice_menu: None,
            choice_focus: HashMap::new(),
            choice_bounds: Rc::default(),
            choice_armed: None,
            adding: None,
            addings_opened: 0,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    fn session_notified(&mut self, session: Entity<Session>, cx: &mut Context<Self>) {
        // The snapshot came before this view listened: ask for it again, once.
        let missing = self.timeline.is_none() && session.read(cx).state.has_snapshot;
        if missing && !self.asked_for_state {
            self.asked_for_state = true;
            session.update(cx, |session, _| {
                session.send(Command::Refresh);
            });
        }
        cx.notify();
    }

    fn bridge_event(&mut self, _: Entity<Session>, event: &BridgeEvent, cx: &mut Context<Self>) {
        match &*event.0 {
            Event::State { snapshot } => {
                let revision = snapshot.revision;
                self.timeline = Some(snapshot.timeline.clone());
                if revision != self.revision {
                    self.forget_sent();
                }
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.revision != revision)
                {
                    self.pending = None;
                }
                self.revision = revision;
                cx.notify();
            }
            Event::Error(_) => {
                // A refused edit leaves the clip as it was: show it so.
                self.pending = None;
                self.forget_sent();
                cx.notify();
            }
            _ => {}
        }
    }

    fn forget_sent(&mut self) {
        self.name_sent = None;
        for field in self.texts.values_mut() {
            field.sent = None;
        }
    }

    fn inspected(&self, cx: &Context<Self>) -> Option<Inspected> {
        let session = self.session.read(cx);
        let id = self.clip.as_deref()?;
        Inspected::find(
            self.timeline.as_ref()?,
            id,
            session.state.position,
            session.selection.len(),
        )
    }

    fn edit(&mut self, ops: Vec<Value>, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.edit(ops);
            cx.notify();
        });
    }

    fn set_param(&mut self, clip: &str, field: &Field, value: Value, cx: &mut Context<Self>) {
        self.edit(vec![field.op(clip, value)], cx);
    }

    /// Another clip is chosen: what was typed for the last one goes to it
    /// first, and the fields start again.
    fn choose(&mut self, clip: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.clip.is_some() {
            if self.name.focus_handle(cx).is_focused(window) {
                self.commit_name(cx);
            }
            let focused: Vec<Field> = self
                .texts
                .iter()
                .filter(|(_, field)| field.input.focus_handle(cx).is_focused(window))
                .map(|(key, _)| key.clone())
                .collect();
            for key in focused {
                self.commit_text(&key, cx);
            }
            self.commit_typed(cx);
        }
        self.clip = clip;
        self.texts.clear();
        self.rails.borrow_mut().clear();
        self.typing = None;
        self.drag = None;
        self.pending = None;
        self.palette = None;
        self.close_choice(window, cx);
        self.choice_bounds.borrow_mut().clear();
        self.adding = None;
        self.folded.clear();
        self.scroll.set_offset(gpui::point(px(0.0), px(0.0)));
        self.name_sent = None;
        let name = self.inspected(cx).map(|clip| clip.name).unwrap_or_default();
        self.name.update(cx, |input, cx| input.set_text(name, cx));
    }
}

impl Render for InspectorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Only as it draws: what the inspector holds for its clip (folded
        // cards, its scroll, a list open, what is typed) stays while it is
        // put away, whatever is chosen meanwhile, until it shows another
        // clip; and a field left as it goes keeps what was typed for it.
        let first = self.session.read(cx).selection.first().cloned();
        if first != self.clip {
            self.choose(first, window, cx);
        }
        let root = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme::chrome());
        let Some(clip) = self.inspected(cx) else {
            // Its clip gone, a choice's menu has no button to stay under.
            self.close_choice(window, cx);
            let words = if self.timeline.is_none() {
                "Waiting for the project…"
            } else {
                "Choose a clip on the timeline."
            };
            return root
                .items_center()
                .justify_center()
                .child(ui::label(words, text::CONTROL, theme::ink3()))
                .into_any_element();
        };
        self.sync_fields(&clip, window, cx);
        // Scrolled since its menu opened (the wheel over the body): the
        // menu goes before it is drawn somewhere it did not open.
        self.keep_choice_menu(window, cx);
        let sections: Vec<AnyElement> = [
            self.generator_section(&clip, window, cx),
            self.caption_section(&clip),
            self.clip_section(&clip, window, cx),
            self.effects_section(&clip, window, cx),
            self.masks_section(&clip, cx),
        ]
        .into_iter()
        .flatten()
        .collect();
        let body = div()
            .id("inspector-body")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(space::S16))
            .child(self.head(&clip, window, cx))
            .children(sections.into_iter().map(|section| {
                div()
                    .flex()
                    .flex_col()
                    .child(ui::hairline(true))
                    .child(section)
            }));
        root.child(body)
            .child(ui::scroll_fade(&self.scroll, theme::chrome()))
            .children(self.drag_listener(cx))
            .children(self.adding_list(&clip, window, cx))
            .into_any_element()
    }
}

/// A text field in its frame, ringed in the accent while typed in. A press
/// anywhere in the frame types in it.
fn field_frame(input: &Entity<TextInput>, focused: bool, tall: f32) -> gpui::Div {
    let input = input.clone();
    div()
        .min_w_0()
        .h(px(tall))
        .flex()
        .items_center()
        .px(px(space::S6))
        .overflow_hidden()
        .rounded(px(radius::SMALL))
        .bg(theme::well())
        .border_1()
        .border_color(if focused {
            theme::accent()
        } else {
            theme::line2()
        })
        .on_mouse_down(MouseButton::Left, {
            let input = input.clone();
            move |_, window, cx| {
                input.read(cx).focus(window);
            }
        })
        .child(input)
}

#[cfg(test)]
mod tests {
    use super::clip::{keyed_value, Field, Target};
    use super::number::Drag;
    use super::values::{format_number, parse_number};
    use proptest::prelude::*;
    use serde_json::json;

    proptest! {

        #[test]
        fn a_shown_number_reads_back_as_it_was(value in -1.0e5f64..1.0e5, places in 0usize..4) {
            let shown = format_number(value, places);
            let back = parse_number(&shown).expect("a number");
            // Never further than half its last place, which is at most the fourth.
            prop_assert!((back - value).abs() <= 0.5e-4 + 1e-9, "{} shown as {}", value, shown);
            // Showing what it reads back as changes nothing.
            prop_assert_eq!(format_number(back, places), shown);
        }

        #[test]
        fn between_two_linear_keys_a_value_stays_between_them(a in -100.0f64..100.0, b in -100.0f64..100.0, at in -1.0f64..2.0) {
            let keys = json!([{"at": 0.0, "value": a, "interp": "linear"}, {"at": 1.0, "value": b, "interp": "linear"}]);
            let value = keyed_value(keys.as_array().unwrap(), at).as_f64().unwrap();
            prop_assert!(value >= a.min(b) - 1e-9 && value <= a.max(b) + 1e-9);
        }

        #[test]
        fn a_drag_stays_within_what_the_edit_may_take(x in -1000.0f32..1000.0, from in -10.0f64..10.0) {
            for rail in [Some((10.0, 200.0)), None] {
                let mut drag = Drag {
                    clip: "c".into(),
                    field: Field::new(Target::Clip, "x"),
                    soft: (-1.0, 1.0),
                    hard: (-10.0, 10.0),
                    decimals: 2,
                    from,
                    value: from,
                    start_x: 0.0,
                    scrub: rail.is_none(),
                    moved: false,
                    rail,
                };
                drag.follow(x);
                prop_assert!(drag.value >= -10.0 && drag.value <= 10.0);
                if rail.is_some() {
                    prop_assert!(drag.value >= -1.0 && drag.value <= 1.0);
                }
            }
        }
    }
}
