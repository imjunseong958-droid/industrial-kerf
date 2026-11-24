//! Shots another program makes: what Kerf asks to make in the conversation,
//! a media's Make… plan and its yes, and a shot's graph.
//!
//! Nothing here runs a step on its own. A making is `media_make`: asked
//! without `confirmed`, the bridge answers with the plan, what would run; only
//! a person's Make sends it again with `confirmed: true`. The graph lays out a
//! media's steps as the bridge reads them (`recipe`), and a change to one is a
//! `recipe_edit` the bridge applies as one edit, which undo takes back.
//!
//! Nothing in the graph moves by itself: a card stays where it was laid out
//! or dragged, and only a making's amber stripes run over its takes' card.

use gpui::{actions, App, KeyBinding, SharedString};
use serde_json::Value;

mod graph;
mod makings;
mod plan;
mod recipe;

pub use graph::{
    another_seed, arrange, card_height, card_rect, card_size, draw_signal, draws_curve, extent,
    fitted_in, graph_start, in_port, nearest_toward, one_line, open_when_ready, out_port,
    output_within, part_number, placed_over, placement_key, point_on, reading_lines, settle,
    signal_keys, signal_line, signal_under, take_tip, workflow_key, zoom_about, Camera, GraphStart,
    PictureQueue, ShotGraph, Toward, ZOOM_LEAST, ZOOM_MOST, ZOOM_READ, ZOOM_STEP,
};
pub use makings::{card_state, card_title, CardState, Makings, Progress};
pub use plan::{ask_words, takes_words, wants_carrying, AskWords, MakePlan, Plan};
pub use recipe::{
    gives_word, input_kind, link_output, part_id, port_says, ports_for, reads_port, shown_number,
    shown_value, value_of, wire_nth, GraphEdge, GraphNode, Holds, InputKind, Recipe, Workflow,
};

/// Open the graph of the steps that make `media`.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct OpenShotGraph {
    pub media: SharedString,
}

/// Make `rolls` takes of `media` (1 for Make…, more for Make Several), after
/// a person has read the plan and said yes.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct MakeMedia {
    pub media: SharedString,
    pub rolls: u32,
}

actions!(making, [Close]);

/// The key context the plan and the graph answer in.
const CONTEXT: &str = "Making";

/// Esc gives the window back, before the window's own Esc has it: this
/// context is the deeper one. Binding a keymap clears it, so it is bound
/// again whenever a panel opens and finds it missing.
fn bind_close(cx: &mut App) {
    if !editor_launcher::settings::escape_bound(cx, &Close) {
        cx.bind_keys([KeyBinding::new("escape", Close, Some(CONTEXT))]);
    }
}

/// The plan's and the graph's own keys: Esc, which gives the window back,
/// and the canvas's fit, zoom and arrows. Binding a keymap clears them, so
/// each is bound again where it is missing: as a plan or a graph opens, as
/// what was over the graph closes, and by `keymap::bind` after a keymap.
pub fn bind_keys(cx: &mut App) {
    bind_close(cx);
    graph::bind_graph_keys(cx);
}

/// The head of the plan's panel and of the graph's.
const HEAD: f32 = 48.0;

// ------------------------------------------------------------------ values as they are read

/// A value as a field shows it: words as they are, a whole number without a
/// point, nothing for null.
fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(words) => words.clone(),
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(whole), _) => whole.to_string(),
            (None, Some(real)) => plain(real),
            _ => number.to_string(),
        },
        Value::Bool(on) => on.to_string(),
        other => other.to_string(),
    }
}

fn plain(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        let written = format!("{value:.3}");
        written
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

/// `m:ss` since `since`.
fn clock(since: std::time::Instant) -> String {
    let elapsed = since.elapsed().as_secs();
    format!("{}:{:02}", elapsed / 60, elapsed % 60)
}
