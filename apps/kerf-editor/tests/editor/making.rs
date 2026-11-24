//! Shot graphs, recipes and takes: how a shot is made.

use crate::support::{command, event};
use kerf_bridge::event::graph::Gives;
use kerf_editor::cast;
use kerf_editor::cast::Entry;
use kerf_editor::making::{
    another_seed, arrange, card_height, card_size, card_state, card_title, draw_signal,
    draws_curve, extent, fitted_in, graph_start, in_port, input_kind, link_output, nearest_toward,
    one_line, out_port, output_within, part_id, part_number, placed_over, point_on, port_says,
    ports_for, reading_lines, reads_port, settle, shown_value, signal_keys, signal_line,
    signal_under, take_tip, value_of, wire_nth, zoom_about, Camera, CardState, GraphEdge,
    GraphNode, GraphStart, Holds, InputKind, PictureQueue, Progress, Recipe, Toward, Workflow,
    ZOOM_LEAST, ZOOM_MOST, ZOOM_READ,
};
use kerf_editor::model::Ask;
use kerf_editor::store::Making;
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------- cast

#[test]
fn the_cast_is_read_and_written_as_the_file_reads() {
    let event = event(json!({"type": "cast", "entries": [
        {"kind": "cast", "name": "Maya", "words": "a woman", "refs": ["refs/maya.png"]},
        {"kind": "style", "name": "Dusk", "words": "", "refs": []}
    ], "toMake": [{"media": "m3", "why": "cast Maya changed"}]}));
    let entries = cast::entries_of(&event);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].refs, vec!["refs/maya.png".to_owned()]);
    assert_eq!(
        cast::wants_line(&cast::to_make_of(&event)),
        "Wants making now: m3 (cast Maya changed)"
    );
    assert_eq!(
        cast::wants_line(&[]),
        "No shot wants making as the cast stands."
    );
    let typed = vec![Entry {
        kind: "cast".into(),
        name: "  Maya ".into(),
        words: " a  woman\nin   red ".into(),
        refs: cast::split_pictures(" refs/a.png, ,refs/b.png ,"),
    }];
    assert_eq!(
        cast::cast_set(&typed),
        command(json!({"type": "cast_set", "entries": [
            {"kind": "cast", "name": "Maya", "words": "a woman in red", "refs": ["refs/a.png", "refs/b.png"]}
        ]}))
    );
}

#[test]
fn a_cast_that_does_not_read_cannot_be_saved_over() {
    use cast::{heard, Heard, Stage};
    let unreadable = event(json!({"type": "error", "command": "cast",
        "message": "cast.kerfcast:4: `csat` is not `cast` or `style`"}));
    // Asked for and not read: nothing to show, and nothing to save over the file.
    assert_eq!(
        heard(Stage::Reading, &unreadable),
        Heard::Failed {
            message: "cast.kerfcast:4: `csat` is not `cast` or `style`".into(),
            stage: Stage::Unreadable,
        }
    );
    assert!(!Stage::Unreadable.can_save());
    assert!(!Stage::Reading.can_save() && !Stage::Saving.can_save());
    // A save refused leaves the rows as typed, to be mended and saved again.
    let refused = event(
        json!({"type": "error", "command": "cast", "message": "cannot save the cast: two named Ada"}),
    );
    assert_eq!(
        heard(Stage::Saving, &refused),
        Heard::Failed {
            message: "cannot save the cast: two named Ada".into(),
            stage: Stage::Editing,
        }
    );
    assert!(Stage::Editing.can_save());
    // Read, the rows come in; saved, the window goes back.
    let read = event(
        json!({"type": "cast", "entries": [{"kind": "cast", "name": "mara", "words": "", "refs": []}],
        "toMake": [], "saved": false}),
    );
    assert!(matches!(heard(Stage::Reading, &read), Heard::Read(entries) if entries.len() == 1));
    let saved = event(json!({"type": "cast", "entries": [], "toMake": [], "saved": true}));
    assert_eq!(heard(Stage::Saving, &saved), Heard::Saved);
    assert_eq!(heard(Stage::Editing, &saved), Heard::ToMake);
    // Another command's error is not the dialog's.
    let other = event(json!({"type": "error", "command": "media_make", "message": "no"}));
    assert_eq!(heard(Stage::Reading, &other), Heard::Nothing);
}

/// The dialog's stage after each event, as `CastView::heard` moves it.
fn cast_stage_after(stage: cast::Stage, event: &kerf_bridge::Event) -> cast::Stage {
    match cast::heard(stage, event) {
        cast::Heard::Read(_) => cast::Stage::Editing,
        cast::Heard::Failed { stage, .. } => stage,
        _ => stage,
    }
}

proptest! {
    /// Save is pressed only on a cast that was read: whatever kerfd says,
    /// in whatever order, the dialog can save only once a `cast` it asked
    /// for came in while it was reading.
    #[test]
    fn save_needs_the_cast_read_first(events in proptest::collection::vec(0usize..5, 0..12)) {
        let said = [
            json!({"type": "error", "command": "cast", "message": "does not read"}),
            json!({"type": "error", "command": "media_make", "message": "no"}),
            json!({"type": "cast", "entries": [], "toMake": [], "saved": true}),
            json!({"type": "snapshot"}),
            json!({"type": "cast", "entries": [], "toMake": [], "saved": false}),
        ]
        .map(crate::support::event);
        let mut stage = cast::Stage::Reading;
        let mut read = false;
        for event in events {
            // A `cast` holds the file's entries, saved by another or not.
            if stage == cast::Stage::Reading && (event == 2 || event == 4) {
                read = true;
            }
            stage = cast_stage_after(stage, &said[event]);
            if stage.can_save() {
                prop_assert!(read);
            }
        }
    }
}

#[test]
fn a_picture_from_outside_is_copied_into_refs_under_a_free_name() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let face = outside.path().join("face.png");
    std::fs::write(&face, b"one").unwrap();
    assert_eq!(
        cast::cast_picture_path(project.path(), &face).unwrap(),
        "refs/face.png"
    );
    std::fs::write(&face, b"two").unwrap();
    assert_eq!(
        cast::cast_picture_path(project.path(), &face).unwrap(),
        "refs/face-2.png"
    );
    assert_eq!(
        std::fs::read(project.path().join("refs/face-2.png")).unwrap(),
        b"two"
    );
    // One of the project's own is named as it is.
    let inside = project.path().join("shots").join("still.png");
    std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
    std::fs::write(&inside, b"x").unwrap();
    assert_eq!(
        cast::cast_picture_path(project.path(), &inside).unwrap(),
        "shots/still.png"
    );
    // One that is not there cannot be copied.
    assert!(cast::cast_picture_path(project.path(), Path::new("/nowhere/x.png")).is_err());
}

/// A node as the bridge says one: one of a shot's graph, as its `recipe` lists it (there,
/// and set up, unless `holds` says otherwise), or with the kind `part` one of a workflow's
/// own, as its `workflow` lists it.
fn node(id: &str, kind: &str, gives: &str, line: Option<usize>, holds: Value) -> GraphNode {
    if kind == "part" {
        return GraphNode {
            id: id.into(),
            title: id.into(),
            detail: String::new(),
            gives: Gives::File,
            line,
            holds: Holds::Part(serde_json::from_value(holds).unwrap()),
        };
    }
    let mut said = json!({"id": id, "kind": kind, "title": id, "gives": gives,
        "there": true, "setUp": true});
    if let Some(line) = line {
        said["line"] = json!(line);
    }
    for (key, value) in holds.as_object().into_iter().flatten() {
        said[key] = value.clone();
    }
    let node: kerf_bridge::event::graph::Node = serde_json::from_value(said).unwrap();
    GraphNode {
        id: node.id,
        title: node.title,
        detail: node.detail,
        gives: node.gives,
        line: node.line,
        holds: Holds::Shot(node.what),
    }
}

fn edge(from: &str, to: &str, port: &str, nth: usize) -> GraphEdge {
    GraphEdge {
        from: from.into(),
        to: to.into(),
        port: port.into(),
        nth,
        gives: Gives::Video,
    }
}

/// A shot drawn into a guide, a still and a clip's way and a signal and
/// one of the cast read by the video model, which makes the takes.
fn court() -> Recipe {
    Recipe {
        media: "shot".into(),
        nodes: vec![
            node(
                "sequence:court",
                "sequence",
                "video",
                None,
                json!({"sequence": "court"}),
            ),
            node(
                "made:depth.mp4",
                "render",
                "video",
                Some(1),
                json!({"makes": "depth.mp4"}),
            ),
            node(
                "made:ball.png",
                "model",
                "image",
                Some(2),
                json!({"makes": "ball.png"}),
            ),
            node(
                "made:{out}",
                "model",
                "video",
                Some(3),
                json!({"reads": ["guide", "guide_ball", "ref", "signal"]}),
            ),
            node(
                "signal:bounce",
                "signal",
                "signal",
                None,
                json!({"clip": "bounce", "values": [0.0, 1.0, 0.0]}),
            ),
            node(
                "moves:ball",
                "moves",
                "path",
                None,
                json!({"clip": "ball", "path": "-0.4,-0.3 0,0.3 0.4,-0.2"}),
            ),
            node("cast:Ada", "cast", "words", None, json!({"name": "Ada"})),
            node(
                "file:refs/court.png",
                "file",
                "image",
                None,
                json!({"path": "refs/court.png"}),
            ),
            node("takes", "takes", "video", None, json!({})),
        ],
        edges: vec![
            edge("sequence:court", "made:depth.mp4", "sequence", 0),
            edge("made:depth.mp4", "made:{out}", "guide", 0),
            edge("made:ball.png", "made:{out}", "ref", 0),
            edge("file:refs/court.png", "made:{out}", "ref", 1),
            edge("signal:bounce", "made:{out}", "signal", 0),
            edge("moves:ball", "made:{out}", "moves", 0),
            edge("cast:Ada", "made:{out}", "cast", 0),
            edge("made:{out}", "takes", "take", 0),
        ],
        ..Recipe::default()
    }
}

fn of<'a>(recipe: &'a Recipe, id: &str) -> &'a GraphNode {
    recipe.node(id).unwrap()
}

#[test]
fn a_node_is_read_under_the_names_its_reader_takes_it_by() {
    let recipe = court();
    let ports = |from: &str, to: &str| ports_for(of(&recipe, from), of(&recipe, to));
    // A guide reads a sequence, and nothing else.
    assert_eq!(ports("sequence:court", "made:depth.mp4"), ["sequence"]);
    assert!(ports("made:ball.png", "made:depth.mp4").is_empty());
    // A model reads a picture in motion as a guide (or another it reads), a reference or its first frame.
    assert_eq!(
        ports("made:depth.mp4", "made:{out}"),
        ["guide", "guide_ball", "ref", "first"]
    );
    // A still as a first or last frame, or a reference.
    assert_eq!(
        ports("made:ball.png", "made:{out}"),
        ["first", "last", "ref"]
    );
    assert_eq!(
        ports("file:refs/court.png", "made:{out}"),
        ["first", "last", "ref"]
    );
    assert_eq!(ports("signal:bounce", "made:{out}"), ["signal"]);
    assert_eq!(ports("moves:ball", "made:{out}"), ["moves"]);
    assert_eq!(ports("cast:Ada", "made:{out}"), ["cast"]);
    // A sequence goes to a guide, not to a model; the takes give nothing on.
    assert!(ports("sequence:court", "made:{out}").is_empty());
    assert!(ports("takes", "made:{out}").is_empty());
    // A step reads what a step before it made, never one after; nothing reads into
    // what is not a step, nor into itself.
    assert!(ports("made:{out}", "made:ball.png").is_empty());
    assert!(ports("made:{out}", "takes").is_empty());
    assert!(ports("made:{out}", "made:{out}").is_empty());
    assert!(ports("made:ball.png", "signal:bounce").is_empty());
}

#[test]
fn a_reading_is_written_on_its_readers_line_as_the_line_names_it() {
    let recipe = court();
    assert_eq!(value_of(of(&recipe, "signal:bounce")), "bounce");
    assert_eq!(value_of(of(&recipe, "moves:ball")), "ball");
    assert_eq!(value_of(of(&recipe, "cast:Ada")), "Ada");
    assert_eq!(
        value_of(of(&recipe, "file:refs/court.png")),
        "refs/court.png"
    );
    assert_eq!(value_of(of(&recipe, "sequence:court")), "court");
    assert_eq!(value_of(of(&recipe, "made:depth.mp4")), "{depth.mp4}");
    assert_eq!(value_of(of(&recipe, "made:ball.png")), "{ball.png}");
    assert_eq!(value_of(of(&recipe, "takes")), "");
    // A name a line gives once is replaced; one it may give again is one more.
    assert_eq!(wire_nth(&recipe, "made:{out}", "ref"), 2);
    assert_eq!(wire_nth(&recipe, "made:{out}", "cast"), 1);
    assert_eq!(wire_nth(&recipe, "made:{out}", "style"), 0);
    assert_eq!(wire_nth(&recipe, "made:{out}", "guide"), 0);
    assert_eq!(wire_nth(&recipe, "made:{out}", "first"), 0);
    // What the model has said it has no use for is dashed, and said so.
    let model = of(&recipe, "made:{out}");
    assert!(reads_port(model, "guide") && reads_port(model, "signal"));
    assert!(!reads_port(model, "moves") && !reads_port(model, "first"));
    assert!(reads_port(model, "cast") && reads_port(model, "style"));
    // One that has said nothing reads everything.
    assert!(reads_port(of(&recipe, "made:ball.png"), "moves"));
    assert_eq!(port_says("guide"), "a picture in motion to follow");
    assert_eq!(
        port_says("guide_ball"),
        "another picture in motion to follow, of its ball"
    );
    assert_eq!(port_says("ref"), "who or what is in it, or how it looks");
}

#[test]
fn edges_leave_a_card_at_its_right_and_come_in_spread_down_its_left() {
    let recipe = court();
    let places = arrange(&recipe);
    let model = "made:{out}";
    let (x, y) = places[model];
    let (width, _) = card_size(of(&recipe, model));
    let height = card_height(of(&recipe, model), recipe.reads_into(model).len());
    let out = out_port(&recipe, &places, model).unwrap();
    assert_eq!(out, (x + width, y + height / 2.0));
    let ins: Vec<(f32, f32)> = recipe
        .reads_into(model)
        .into_iter()
        .map(|edge| in_port(&recipe, &places, edge).unwrap())
        .collect();
    assert_eq!(ins.len(), 6);
    for pair in ins.windows(2) {
        assert_eq!(pair[0].0, x);
        assert!(pair[1].1 > pair[0].1, "{ins:?}");
    }
    assert!(ins[0].1 > y && ins[5].1 < y + height);
    // A card the person put elsewhere is where they put it, and its edges with it.
    let mut put = HashMap::new();
    put.insert(model.to_owned(), (900.0, -40.0));
    put.insert("gone".to_owned(), (1.0, 1.0));
    let moved = placed_over(&recipe, &put);
    assert_eq!(moved[model], (900.0, -40.0));
    assert!(!moved.contains_key("gone"));
    assert_eq!(out_port(&recipe, &moved, model).unwrap().0, 900.0 + width);
}

#[test]
fn a_card_is_as_tall_as_what_it_shows_and_reads() {
    let recipe = court();
    // A model says a third line; a signal or a clip's way draws under its lines.
    assert!(card_size(of(&recipe, "made:{out}")).1 > card_size(of(&recipe, "made:depth.mp4")).1);
    assert!(draws_curve(of(&recipe, "signal:bounce")));
    assert!(draws_curve(of(&recipe, "moves:ball")));
    assert!(!draws_curve(&node(
        "signal:none",
        "signal",
        "signal",
        None,
        json!({"values": []})
    )));
    assert!(card_size(of(&recipe, "signal:bounce")).1 > card_size(of(&recipe, "cast:Ada")).1);
    // Six things come into the model apart.
    assert!(card_height(of(&recipe, "made:{out}"), 6) >= 16.0 + 24.0 * 6.0);
    // A workflow's node is its name and a row an input, however many come in.
    let part = node(
        "part:3",
        "part",
        "file",
        None,
        json!({"inputs": [{"name": "a"}, {"name": "b"}, {"name": "c"}]}),
    );
    assert_eq!(card_height(&part, 9), card_size(&part).1);
    assert!(card_size(&part).1 > card_size(&node("part:4", "part", "file", None, json!({}))).1);
}

proptest! {
    /// What is under the point zoomed about stays under it, and the zoom
    /// stays between the least and the most.
    #[test]
    fn zooming_keeps_what_is_under_the_pointer_under_it(
        zoom in 0.35f32..2.0,
        offset in (-2000.0f32..2000.0, -2000.0f32..2000.0),
        factor in 0.1f32..10.0,
        about in (0.0f32..1400.0, 0.0f32..900.0),
    ) {
        let camera = Camera { zoom, offset };
        let zoomed = zoom_about(camera, factor, about);
        prop_assert!(zoomed.zoom >= ZOOM_LEAST && zoomed.zoom <= ZOOM_MOST);
        let before = camera.to_scene(about);
        let after = zoomed.to_scene(about);
        prop_assert!((before.0 - after.0).abs() < 0.05 && (before.1 - after.1).abs() < 0.05,
            "{before:?} {after:?}");
        let back = zoomed.to_view(zoomed.to_scene(about));
        prop_assert!((back.0 - about.0).abs() < 0.05 && (back.1 - about.1).abs() < 0.05);
    }

    /// Fit takes all of it in, in the room the legend leaves, where it can
    /// at no less than the least; short of Fit, a graph too large to read
    /// whole is drawn at a size that is read, its left or top at the room's.
    #[test]
    fn a_graph_settles_in_its_canvas(
        left in -3000.0f32..3000.0,
        top in -3000.0f32..3000.0,
        width in 100.0f32..6000.0,
        height in 50.0f32..4000.0,
        room in (400.0f32..1600.0, 300.0f32..1000.0),
    ) {
        let all = Some([left, top, left + width, top + height]);
        let whole = settle(all, room, true);
        prop_assert!(whole.zoom >= ZOOM_LEAST && whole.zoom <= 1.0);
        let fits = width * ZOOM_LEAST <= room.0 - 64.0 && height * ZOOM_LEAST <= room.1 - 24.0 - 60.0;
        if fits {
            let (l, t) = whole.to_view((left, top));
            let (r, b) = whole.to_view((left + width, top + height));
            prop_assert!(l >= 32.0 - 1.0 && r <= room.0 - 32.0 + 1.0, "{l} {r}");
            prop_assert!(t >= 24.0 - 1.0 && b <= room.1 - 60.0 + 1.0, "{t} {b}");
        }
        let read = settle(all, room, false);
        if whole.zoom < ZOOM_READ {
            prop_assert_eq!(read.zoom, ZOOM_READ);
            let (l, _) = read.to_view((left, top));
            if width * ZOOM_READ > room.0 - 64.0 {
                prop_assert!((l - 32.0).abs() <= 1.0, "{l}");
            }
        } else {
            prop_assert_eq!(read, whole);
        }
    }

    /// An arrow goes to a card that way, the nearest; none chosen, the first.
    #[test]
    fn an_arrow_goes_to_the_nearest_card_that_way(
        centres in prop::collection::vec((-1000.0f32..1000.0, -1000.0f32..1000.0), 1..12),
        from in 0usize..12,
        which in 0usize..4,
    ) {
        let cards: Vec<(String, (f32, f32))> = centres
            .iter()
            .enumerate()
            .map(|(at, centre)| (format!("n{at}"), *centre))
            .collect();
        let toward = [Toward::Left, Toward::Right, Toward::Up, Toward::Down][which];
        prop_assert_eq!(nearest_toward(&cards, None, toward), Some("n0".to_owned()));
        let from = from % cards.len();
        let start = cards[from].1;
        if let Some(next) = nearest_toward(&cards, Some(&cards[from].0), toward) {
            let to = cards.iter().find(|(id, _)| *id == next).unwrap().1;
            let along = match toward {
                Toward::Left => start.0 - to.0,
                Toward::Right => to.0 - start.0,
                Toward::Up => start.1 - to.1,
                Toward::Down => to.1 - start.1,
            };
            prop_assert!(along > 1.0, "{toward:?} {start:?} {to:?}");
            prop_assert!(next != cards[from].0);
        }
    }

    /// A drawing kept as keys misses none of its frames by more than the
    /// tolerance (and the thousandth they are written to), starts and ends
    /// where it does, and its keys go forward.
    #[test]
    fn a_signal_drawn_is_kept_as_keys_that_hold_to_it(
        values in prop::collection::vec(0.0f64..1.0, 2..120),
        start in 0usize..40,
    ) {
        let drawn: BTreeMap<usize, f64> =
            values.iter().enumerate().map(|(at, value)| (start + at, *value)).collect();
        let keys = signal_keys(&drawn, 0.02);
        let at: Vec<usize> = keys.iter().map(|key| key["at"].as_u64().unwrap() as usize).collect();
        prop_assert_eq!(at.first().copied(), Some(start));
        prop_assert_eq!(at.last().copied(), Some(start + values.len() - 1));
        prop_assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{at:?}");
        prop_assert!(keys.iter().all(|key| key["interp"] == "linear"));
        for (frame, value) in &drawn {
            let after = keys.iter().position(|key| key["at"].as_u64().unwrap() as usize >= *frame).unwrap();
            let line = if at[after] == *frame {
                keys[after]["value"].as_f64().unwrap()
            } else {
                let (a, b) = (&keys[after - 1], &keys[after]);
                let (fa, fb) = (a["at"].as_f64().unwrap(), b["at"].as_f64().unwrap());
                let (va, vb) = (a["value"].as_f64().unwrap(), b["value"].as_f64().unwrap());
                va + (vb - va) * (*frame as f64 - fa) / (fb - fa)
            };
            prop_assert!((line - value).abs() <= 0.02 + 0.001, "{frame}: {line} vs {value}");
        }
    }

    /// Drawing from one frame to another fills every frame between, on the
    /// straight line from the one value to the other.
    #[test]
    fn a_drag_across_a_signal_fills_the_frames_it_passed(
        from in 0usize..200,
        to in 0usize..200,
        a in 0.0f64..1.0,
        b in 0.0f64..1.0,
    ) {
        let mut drawn = BTreeMap::new();
        draw_signal(&mut drawn, None, from, a);
        draw_signal(&mut drawn, Some(from), to, b);
        let (low, high) = (from.min(to), from.max(to));
        prop_assert_eq!(drawn.len(), high - low + 1);
        prop_assert_eq!(drawn[&to], b);
        for (frame, value) in &drawn {
            prop_assert!(*value >= a.min(b) - 1e-9 && *value <= a.max(b) + 1e-9);
            if from != to {
                let expected = a + (b - a) * (*frame as f64 - from as f64) / (to as f64 - from as f64);
                prop_assert!((value - expected).abs() < 1e-9);
            }
        }
    }

    /// The curve drawn and the pointer read back agree: a frame's point is
    /// that frame, at its value.
    #[test]
    fn a_signal_is_drawn_where_the_pointer_reads_it(
        values in prop::collection::vec(0.0f64..1.0, 2..100),
        rect in (0.0f32..500.0, 0.0f32..500.0, 50.0f32..400.0, 30.0f32..200.0),
    ) {
        let rect = [rect.0, rect.1, rect.2, rect.3];
        let line = signal_line(&values, rect);
        prop_assert_eq!(line.len(), values.len());
        for (frame, (point, value)) in line.iter().zip(&values).enumerate() {
            let (read, said) = signal_under(rect, values.len(), *point);
            prop_assert_eq!(read, frame);
            prop_assert!((said - value).abs() < 1e-3, "{said} {value}");
        }
    }

    /// A press on a picture is where on it as shares of it; off it, nowhere.
    #[test]
    fn a_point_on_a_picture_is_a_share_of_it(
        room in (0.0f32..300.0, 0.0f32..300.0, 50.0f32..400.0, 50.0f32..400.0),
        size in (1u32..4000, 1u32..4000),
        at in (0.0f32..1.0, 0.0f32..1.0),
    ) {
        let room = [room.0, room.1, room.2, room.3];
        let frame = fitted_in(room, size);
        // Inside the room, at the picture's own shape.
        prop_assert!(frame[0] >= room[0] - 0.01 && frame[0] + frame[2] <= room[0] + room[2] + 0.01);
        prop_assert!(frame[1] >= room[1] - 0.01 && frame[1] + frame[3] <= room[1] + room[3] + 0.01);
        let shape = size.0 as f32 / size.1 as f32;
        prop_assert!((frame[2] / frame[3] - shape).abs() <= shape * 0.01);
        let pressed = (frame[0] + at.0 * frame[2], frame[1] + at.1 * frame[3]);
        let (x, y) = point_on(frame, pressed).unwrap();
        prop_assert!((x - f64::from(at.0)).abs() < 1e-3 && (y - f64::from(at.1)).abs() < 1e-3);
        prop_assert!(point_on(frame, (frame[0] - 1.0, frame[1])).is_none());
        prop_assert!(point_on(frame, (frame[0], frame[1] + frame[3] + 1.0)).is_none());
    }
}

#[test]
fn a_models_workflow_reads_as_a_graph_of_its_nodes() {
    let event = json!({
        "type": "workflow", "media": "shot", "line": 3, "model": "video", "name": "shot",
        "origin": "bundled", "file": null, "about": "A shot that follows a guide",
        "text": "{…}",
        "blanks": {
            "prompt": {"says": "the line's prompt", "given": "it bounces"},
            "guide": {"says": "the file the line gives as guide=", "by": "file"},
            "steps": {"says": "20 unless the line says steps="}
        },
        "nodes": [
            {"id": "1", "class": "UNETLoader", "title": "UNETLoader", "outputs": ["MODEL"],
             "inputs": [{"name": "unet_name", "value": "{unet}", "blanks": ["unet"]},
                        {"name": "weight_dtype", "value": "default", "choices": ["default", "fp8"]}]},
            {"id": "2", "class": "KSampler", "title": "Sampler", "outputs": ["LATENT"],
             "inputs": [{"name": "model", "link": ["1", 0], "type": "MODEL"},
                        {"name": "steps", "value": 20, "min": 1, "max": 100},
                        {"name": "cfg", "value": 7.5},
                        {"name": "add_noise", "value": true}]},
            {"id": "3", "class": "Other", "inputs": [{"name": "latent", "link": ["9", 0]}]},
            {"class": "Nameless"}
        ]
    });
    let workflow = Workflow::read(&serde_json::from_value(event).unwrap());
    assert_eq!((workflow.line, workflow.model.as_str()), (3, "video"));
    assert!(!workflow.is_own());
    let graph = &workflow.graph;
    assert!(graph.parts);
    assert_eq!(graph.nodes.len(), 3, "a node without an id is left out");
    let sampler = graph.node(&part_id("2")).unwrap();
    // By what ComfyUI's editor calls it, and its kind under that.
    assert_eq!(
        (sampler.title.as_str(), sampler.detail.as_str()),
        ("Sampler", "KSampler")
    );
    assert_eq!(graph.node(&part_id("1")).unwrap().detail, "");
    // A link is an edge; one from a node that is not there is not.
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(
        (
            graph.edges[0].from.as_str(),
            graph.edges[0].to.as_str(),
            graph.edges[0].port.as_str()
        ),
        ("part:1", "part:2", "model")
    );
    let inputs = |id: &str| {
        graph
            .node(&part_id(id))
            .unwrap()
            .part()
            .unwrap()
            .inputs
            .clone()
    };
    assert_eq!(input_kind(&inputs("1")[0]), InputKind::Blank);
    assert_eq!(input_kind(&inputs("1")[1]), InputKind::Value);
    assert_eq!(input_kind(&inputs("2")[0]), InputKind::Link);
    assert_eq!(shown_value(&json!(7.5)), "7.5");
    assert_eq!(shown_value(&json!(20)), "20");
    assert_eq!(shown_value(&json!(20.0)), "20");
    assert_eq!(shown_value(&json!([1, 2])), "[1,2]");
    // What fills each blank, and what the line gives for it now.
    assert_eq!(
        workflow.blank_says("prompt"),
        "The line's prompt. Now: it bounces"
    );
    assert_eq!(
        workflow.blank_says("guide"),
        "The file the line gives as guide=. The line gives none, so the node is left out"
    );
    assert_eq!(
        workflow.blank_says("steps"),
        "20 unless the line says steps="
    );
    assert_eq!(workflow.blank_says("missing"), "");
    // An output is read by an input that is a link already and takes what it gives.
    let (loader, other) = (
        graph.node(&part_id("1")).unwrap(),
        graph.node(&part_id("3")).unwrap(),
    );
    assert_eq!(ports_for(loader, sampler), ["model"]);
    assert!(ports_for(sampler, loader).is_empty());
    assert_eq!(
        ports_for(sampler, other),
        ["latent"],
        "an input that says no type takes any"
    );
    assert_eq!(link_output(loader, sampler, "model"), 0);
    // A workflow's node reads nothing of the shot's, nor the shot's of it.
    let step = node("made:{out}", "model", "video", Some(2), json!({}));
    assert!(ports_for(loader, &step).is_empty() && ports_for(&step, loader).is_empty());
}

#[test]
fn a_number_set_in_a_workflow_is_held_to_what_the_server_takes() {
    assert_eq!(part_number(" 30 ", Some(1.0), Some(100.0)), Some(30.0));
    assert_eq!(part_number("0.5", None, None), Some(0.5));
    assert_eq!(part_number("101", Some(1.0), Some(100.0)), None);
    assert_eq!(part_number("0", Some(1.0), None), None);
    assert_eq!(part_number("thirty", None, None), None);
    assert_eq!(part_number("inf", None, None), None);
    // A prompt is one line, as its step's line is; a seed is under 2^31.
    assert_eq!(
        one_line("a ball\n  bounces\ton\r\nthe floor "),
        "a ball bounces on the floor"
    );
    for _ in 0..64 {
        assert!(another_seed() < 1 << 31);
    }
}

#[test]
fn a_take_says_how_it_holds_beside_its_guide_on_hovering() {
    let reading: kerf_bridge::event::TakeRead = serde_json::from_value(
        json!({"type": "take_read", "take": 2, "overlap": 0.7, "clips": [
            {"clip": "ball", "overlap": 0.82, "frames": 40, "shown": 48},
            {"clip": "lamp", "note": "the take has nothing of it"}
        ]}),
    )
    .unwrap();
    assert_eq!(
        reading_lines(&reading),
        [
            "ball: 0.82 of where it was asked for, followed on 40 of the 48 frames the guide shows it on",
            "lamp: the take has nothing of it"
        ]
    );
    let tip = take_tip(2, false, Some(&reading));
    assert!(tip.starts_with("Have the shot be take 2 again;"));
    assert_eq!(tip.lines().count(), 3);
    assert_eq!(take_tip(1, true, None), "The shot is this take");
}

#[test]
fn a_graph_to_start_on_is_taken_out_of_the_arguments() {
    let words = |words: &[&str]| {
        words
            .iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>()
    };
    let mut args = words(&["/p/court", "--graph", "shot", "--graph-node", "workflow:3"]);
    assert_eq!(
        graph_start(&mut args),
        Ok(Some(GraphStart {
            media: "shot".into(),
            node: Some("workflow:3".into())
        }))
    );
    assert_eq!(args, ["/p/court"]);
    let mut args = words(&["--graph=shot", "/p/court"]);
    assert_eq!(
        graph_start(&mut args),
        Ok(Some(GraphStart {
            media: "shot".into(),
            node: None
        }))
    );
    assert_eq!(args, ["/p/court"]);
    let mut none = words(&["/p/court", "--settings-dir", "/s"]);
    assert_eq!(graph_start(&mut none), Ok(None));
    assert_eq!(none, ["/p/court", "--settings-dir", "/s"]);
    assert!(graph_start(&mut words(&["--graph-node", "takes"])).is_err());
    assert!(graph_start(&mut words(&["--graph"])).is_err());
    assert!(graph_start(&mut words(&["--graph", "--graph-node", "x"])).is_err());
}

#[test]
fn a_make_card_made_already_stays_made() {
    let ask = Ask {
        media: "m4".into(),
        rolls: None,
        take: Some(2),
        text: String::new(),
    };
    let progress = |state, take| Progress {
        state,
        detail: "the step said".into(),
        take,
    };
    let making = |take| Making {
        take,
        line: "rendering".into(),
        since: std::time::Instant::now(),
    };
    // Its take is made: a later making of the same media, and its failure, are another card's.
    let (state, said) = card_state(
        &ask,
        true,
        Some(&progress(CardState::Failed, 3)),
        Some(&making(3)),
        false,
    );
    assert_eq!((state, said.take), (CardState::Made, 2));
    // Not made yet: its own making shows, a later one does not.
    assert_eq!(
        card_state(&ask, false, None, Some(&making(2)), false).0,
        CardState::Making
    );
    assert_eq!(
        card_state(&ask, false, None, Some(&making(3)), false).0,
        CardState::Asked
    );
    assert_eq!(
        card_state(
            &ask,
            false,
            Some(&progress(CardState::Failed, 2)),
            None,
            false
        )
        .0,
        CardState::Failed
    );
    assert_eq!(
        card_state(
            &ask,
            false,
            Some(&progress(CardState::Failed, 3)),
            None,
            true
        )
        .0,
        CardState::Declined
    );
    // A making whose take is not known yet is the card's that pressed Make.
    assert_eq!(
        card_state(
            &ask,
            false,
            Some(&progress(CardState::Making, 0)),
            None,
            false
        )
        .0,
        CardState::Making
    );
    // A take made before the ask's is not what it asked for.
    assert_eq!(
        card_state(
            &ask,
            false,
            Some(&progress(CardState::Made, 1)),
            None,
            false
        )
        .0,
        CardState::Asked
    );
    // Several takes asked for: any of them is the card's.
    let several = Ask {
        rolls: Some(3),
        ..ask.clone()
    };
    assert_eq!(
        card_state(&several, false, None, Some(&making(4)), false).0,
        CardState::Making
    );
    assert_eq!(card_state(&several, true, None, None, false).1.take, 4);
    // Named as the bin names the media.
    assert_eq!(card_title(CardState::Asked, "bounce", 1, 0), "Make bounce?");
    assert_eq!(
        card_title(CardState::Asked, "bounce", 3, 0),
        "Make 3 takes of bounce?"
    );
    assert_eq!(
        card_title(CardState::Made, "bounce", 1, 2),
        "Take 2 of bounce is made"
    );
}

#[test]
fn a_recipe_leaves_out_an_edge_to_what_is_not_there() {
    let recipe = Recipe::read(&serde_json::from_value(json!({
        "type": "recipe", "media": "shot",
        "nodes": [{"id": "a", "kind": "render"}, {"id": "b", "kind": "model"}],
        "edges": [{"from": "a", "to": "b", "port": "guide"}, {"from": "a", "to": "gone", "port": "x"}]
    })).unwrap());
    assert_eq!(recipe.edges.len(), 1);
    assert!(!recipe.parts && recipe.reads());
    assert!(extent(&recipe, &arrange(&recipe)).is_some());
    assert!(!Recipe::default().reads());
}

/// One thing done to a graph's queue of pictures.
#[derive(Clone, Debug)]
enum QueueStep {
    Want(u8),
    Start,
    End,
}

fn queue_step() -> impl Strategy<Value = QueueStep> {
    prop_oneof![
        (0u8..6).prop_map(QueueStep::Want),
        Just(QueueStep::Start),
        Just(QueueStep::End),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// However pictures are asked for and end, no more than the most are
    /// made at once, a file waits once however often it is asked for, they
    /// start in the order asked, and one starts whenever there is room.
    #[test]
    fn a_graphs_pictures_are_made_a_few_at_a_time_in_order(
        most in 1usize..4,
        steps in proptest::collection::vec(queue_step(), 0..64),
    ) {
        let mut queue = PictureQueue::new(most);
        let (mut waiting, mut running) = (VecDeque::<PathBuf>::new(), 0usize);
        for step in steps {
            match step {
                QueueStep::Want(file) => {
                    let path = PathBuf::from(format!("take{file}.mp4"));
                    if !waiting.contains(&path) {
                        waiting.push_back(path.clone());
                    }
                    queue.push(path);
                }
                QueueStep::Start => {
                    let started = queue.start();
                    let expected = if running < most { waiting.pop_front() } else { None };
                    if expected.is_some() {
                        running += 1;
                    }
                    prop_assert_eq!(started, expected);
                }
                QueueStep::End => {
                    queue.ended();
                    running = running.saturating_sub(1);
                }
            }
            prop_assert!(queue.running() <= most);
            prop_assert_eq!(queue.running(), running);
            prop_assert_eq!(queue.waiting(), waiting.len());
        }
    }
}

#[test]
fn a_picture_queue_that_may_run_none_runs_one() {
    let mut queue = PictureQueue::new(0);
    queue.push(PathBuf::from("a.png"));
    queue.push(PathBuf::from("b.png"));
    assert_eq!(queue.start(), Some(PathBuf::from("a.png")));
    assert_eq!(queue.start(), None);
    queue.ended();
    assert_eq!(queue.start(), Some(PathBuf::from("b.png")));
}

#[cfg(unix)]
#[test]
fn what_a_program_writes_is_kept_unless_it_runs_too_long() {
    let mut says = std::process::Command::new("sh");
    says.args(["-c", "printf 'P7 picture'; echo trouble >&2"]);
    assert_eq!(
        output_within(says, Duration::from_secs(20)).as_deref(),
        Some(&b"P7 picture"[..])
    );
    // A program that ends badly gives what it wrote, for its reader to
    // make what it can of.
    let mut fails = std::process::Command::new("sh");
    fails.args(["-c", "printf half; exit 3"]);
    assert_eq!(
        output_within(fails, Duration::from_secs(20)).as_deref(),
        Some(&b"half"[..])
    );
    // One that hangs is stopped at its limit, and gives nothing.
    let started = Instant::now();
    let mut hangs = std::process::Command::new("sleep");
    hangs.arg("30");
    assert_eq!(output_within(hangs, Duration::from_millis(300)), None);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    // One that is not there gives nothing.
    let missing = std::process::Command::new("/nonexistent/kerf-ffmpeg");
    assert_eq!(output_within(missing, Duration::from_secs(1)), None);
}
