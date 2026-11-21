//! The conversation's own logic, through the public `chat` module: what
//! Kerf wrote read into bold, code and timecode links; a request queued
//! behind a turn; what a finished turn folds to; and proposals read from a
//! snapshot, the clips each changes, and which one is pinned.

use kerf_editor::chat::{
    clip_changes, join_queued, model_choices, model_label, next_permissions, permission_words,
    pin_order, request_before, script_summary, signed_frames, spans, timecode_seconds,
    worked_headline, ProposalBook, Span,
};
use kerf_editor::input::scroll_to_show;
use kerf_editor::model::{timecode, Snapshot, Speaker, Timeline, Turn};
use kerf_editor::prefs::{Permissions, Provider};
use proptest::prelude::*;
use serde_json::{json, Value};

fn plain(words: &str) -> Span {
    Span::Plain(words.to_owned())
}

#[test]
fn timecodes_kerf_writes_are_links_and_marks_read_as_bold_and_code() {
    assert_eq!(
        spans("Cut at 00:00:12:05 and **tighten** the `title` at 01:02:03;04."),
        vec![
            plain("Cut at "),
            Span::Timecode("00:00:12:05".into()),
            plain(" and "),
            Span::Bold("tighten".into()),
            plain(" the "),
            Span::Code("title".into()),
            plain(" at "),
            Span::Timecode("01:02:03;04".into()),
            plain("."),
        ]
    );
    // Nothing inside code is a link; a longer run of digits is no timecode.
    assert_eq!(
        spans("`00:00:01:00` and 100:00:01:00 and 00:00:01:000"),
        vec![
            Span::Code("00:00:01:00".into()),
            plain(" and 100:00:01:00 and 00:00:01:000"),
        ]
    );
    // A mark with no partner stays as written.
    assert_eq!(
        spans("2 ** 3 and a ` tick"),
        vec![plain("2 ** 3 and a ` tick")]
    );
    assert_eq!(spans(""), Vec::<Span>::new());
}

#[test]
fn a_timecode_names_the_seconds_the_editor_writes_it_at() {
    assert_eq!(timecode_seconds("00:00:01:00", 25.0), Some(1.0));
    assert_eq!(timecode_seconds("00:01:00:12", 24.0), Some(60.5));
    // Drop-frame text reads as whole frames of the rounded rate, as written.
    let at = timecode_seconds("00:00:10;00", 29.97).unwrap();
    assert!((at - 300.0 / 29.97).abs() < 1e-9);
    assert_eq!(timecode_seconds("0:00:01:00", 25.0), None);
    assert_eq!(timecode_seconds("aa:bb:cc:dd", 25.0), None);
}

proptest! {
    /// What the editor writes reads back as the same frame, and as the same text.
    #[test]
    fn a_timecode_written_reads_back(frames in 0i64..400_000, fps in prop::sample::select(vec![23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0])) {
        let seconds = frames as f64 / fps;
        let written = timecode(seconds, fps);
        let read = timecode_seconds(&written, fps).expect("the editor's own timecode reads");
        prop_assert!((read * fps - frames as f64).abs() < 1e-6, "{written} read as {read}");
        prop_assert_eq!(timecode(read, fps), written);
    }

    /// Cutting into spans loses nothing but the marks: their words, joined,
    /// are the text with its paired marks taken out.
    #[test]
    fn spans_keep_every_word(words in "[a-z :;0-9*`]{0,60}") {
        let joined: String = spans(&words).into_iter().map(|span| match span {
            Span::Plain(words) | Span::Bold(words) | Span::Code(words) | Span::Timecode(words) => words,
        }).collect();
        let kept: String = words.chars().filter(|c| *c != '*' && *c != '`').collect();
        let left: String = joined.chars().filter(|c| *c != '*' && *c != '`').collect();
        prop_assert_eq!(left, kept);
    }

    /// The cursor's line is always in view after scrolling to it, and a line
    /// already in view does not move the text.
    #[test]
    fn the_composer_scrolls_only_as_far_as_the_cursor_needs(scroll in 0.0f32..400.0, top in 0.0f32..800.0, visible in 22.0f32..110.0) {
        use gpui::px;
        let line = px(22.0);
        let next = scroll_to_show(px(scroll), px(top), line, px(visible));
        prop_assert!(px(top) >= next - px(0.001));
        prop_assert!(px(top) + line <= next + px(visible).max(line) + px(0.001));
        if px(top) >= px(scroll) && px(top) + line <= px(scroll) + px(visible) {
            prop_assert_eq!(next, px(scroll));
        }
    }
}

#[test]
fn a_second_request_joins_the_one_waiting() {
    assert_eq!(join_queued(None, "  Trim the title  "), "Trim the title");
    assert_eq!(
        join_queued(Some("Trim the title"), "and add captions"),
        "Trim the title\n\nand add captions"
    );
    assert_eq!(join_queued(Some("Trim the title"), "   "), "Trim the title");
}

#[test]
fn a_finished_turn_says_how_long_it_worked_and_its_steps() {
    assert_eq!(
        worked_headline(Some(72.4), 6),
        "Worked for 1m 12s · 6 steps"
    );
    assert_eq!(worked_headline(Some(41.0), 1), "Worked for 41s · 1 step");
    assert_eq!(worked_headline(Some(3.0), 0), "Worked for 3s");
    // Never less than a second, as Qt's workLength said it.
    assert_eq!(worked_headline(None, 0), "Worked for 1s");
}

#[test]
fn ask_again_sends_the_request_the_answer_was_to() {
    let turn = |kind, text: &str| Turn {
        kind,
        text: text.into(),
        ..Turn::default()
    };
    let turns = vec![
        turn(Speaker::You, "Trim the title"),
        turn(Speaker::Kerf, "Done"),
        turn(Speaker::You, "Add captions"),
        turn(Speaker::Kerf, "Add an API key in Settings"),
    ];
    assert_eq!(request_before(&turns, 3), Some("Add captions"));
    assert_eq!(request_before(&turns, 1), Some("Trim the title"));
    assert_eq!(request_before(&turns, 0), None);
}

#[test]
fn the_composer_chips_say_the_mode_and_offer_the_models() {
    assert_eq!(permission_words(Permissions::Ask).0, "Ask before edits");
    assert_eq!(permission_words(Permissions::AcceptEdits).0, "Accept edits");
    assert_eq!(permission_words(Permissions::Auto).0, "Auto");
    // Shift-Tab goes round the three and back.
    let mut mode = Permissions::Ask;
    for expected in [
        Permissions::AcceptEdits,
        Permissions::Auto,
        Permissions::Ask,
    ] {
        mode = next_permissions(mode);
        assert_eq!(mode, expected);
    }
    assert_eq!(
        model_choices(Provider::ClaudeCode, ""),
        ["default", "opus", "sonnet", "haiku"]
    );
    assert_eq!(model_choices(Provider::Anthropic, ""), ["default"]);
    assert_eq!(model_choices(Provider::Openai, " gpt-x "), ["gpt-x"]);
    assert_eq!(model_label("opus"), "Opus");
    assert_eq!(model_label("claude-something"), "claude-something");
}

fn clip(id: &str, start: f64, duration: f64) -> Value {
    json!({"id": id, "name": id.to_uppercase(), "timelineStart": start, "duration": duration,
           "startFrame": (start * 25.0) as i64, "durationFrames": (duration * 25.0) as i64, "enabled": true})
}

fn timeline(clips: &[Value]) -> Value {
    json!({"name": "Cut", "fps": 25.0, "duration": 30.0,
           "tracks": [{"id": "V1", "type": "video", "name": "V1", "enabled": true, "clips": clips}]})
}

#[test]
fn a_proposal_says_which_clips_it_adds_removes_moves_and_trims() {
    let program = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 4.0, 4.0),
        clip("c", 8.0, 4.0),
        clip("d", 12.0, 4.0),
    ]));
    let proposed = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 4.68, 4.0),
        clip("c", 8.0, 3.0),
        clip("e", 16.0, 2.0),
    ]));
    let changes = clip_changes(&program, &proposed);
    let rows: Vec<(String, String, String)> =
        changes.iter().map(|change| change.row(25.0)).collect();
    assert_eq!(
        rows,
        vec![
            ("B · V1".into(), "→ 00:00:04:17".into(), "+17f".into()),
            ("C · V1".into(), "→ 00:00:11:00".into(), "−25f".into()),
            ("E · V1".into(), "→ 00:00:16:00".into(), "new".into()),
            ("D · V1".into(), "removed".into(), "".into()),
        ]
    );
    assert!(changes[0].moved && !changes[0].trimmed);
    assert!(changes[1].trimmed && !changes[1].moved);
    assert!(changes[2].added);
    assert!(changes[3].removed);
    // Nothing changed, nothing said.
    assert!(clip_changes(&program, &program).is_empty());
    assert_eq!(signed_frames(0), "0f");
}

fn snapshot() -> Value {
    json!({
        "revision": 7,
        "timeline": timeline(&[clip("a", 0.0, 4.0), clip("b", 20.0, 4.0)]),
        "proposals": [
            {"proposalId": "p-far", "status": "pending", "freshness": "current", "version": 1,
             "script": "# a comment\nuse main\ntrim a out -12f", "documentDiff": "", "author": {"kind": "kerf", "name": "Kerf"},
             "supersedes": null,
             "versions": [{"version": 1, "proposalId": "p-far", "script": "trim a out -12f", "documentDiff": ""}]},
            {"proposalId": "p-near-2", "status": "pending", "freshness": "current", "version": 2,
             "script": "move b to 21s", "documentDiff": "", "author": {"kind": "mcp", "name": "Claude Code"},
             "supersedes": "p-near-1",
             "versions": [{"version": 1, "proposalId": "p-near-1", "script": "move b to 20.5s", "documentDiff": ""},
                          {"version": 2, "proposalId": "p-near-2", "script": "move b to 21s", "documentDiff": ""}]},
            {"proposalId": "p-done", "status": "accepted", "freshness": "current", "version": 1, "appliedRevision": 6,
             "script": "trim b in +2f", "documentDiff": ""}
        ],
        "proposalTimelines": {
            "p-far": timeline(&[clip("a", 0.0, 3.52), clip("b", 20.0, 4.0)]),
            "p-near-1": timeline(&[clip("a", 0.0, 4.0), clip("b", 20.5, 4.0)]),
            "p-near-2": timeline(&[clip("a", 0.0, 4.0), clip("b", 21.0, 4.0)]),
        }
    })
}

#[test]
fn proposals_keep_their_versions_and_the_nearest_open_one_is_pinned() {
    let value = snapshot();
    let book = ProposalBook::read(&value);
    let snapshot = Snapshot::from_value(&value);
    let near = &snapshot.proposals[1];
    let versions = book.versions_of(near);
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].proposal_id, "p-near-1");
    // A turn that staged the first version still finds its card.
    assert_eq!(
        book.find(&snapshot.proposals, "p-near-1")
            .map(|p| p.proposal_id.as_str()),
        Some("p-near-2")
    );
    assert_eq!(book.applied.get("p-done"), Some(&6));
    // A proposal the snapshot lists no versions of has its own as its one.
    assert_eq!(book.versions_of(&snapshot.proposals[2]).len(), 1);
    assert_eq!(
        script_summary(&snapshot.proposals[0].script, 1),
        "trim a out -12f"
    );
    assert_eq!(script_summary("# only a comment", 3), "Proposal v3");

    let program = &snapshot.timeline;
    // At 0 s the trim of a is nearer; at 22 s the move of b. Settled ones are not pinned.
    assert_eq!(
        pin_order(&snapshot.proposals, &book, program, None, 0.0),
        ["p-far", "p-near-2"]
    );
    assert_eq!(
        pin_order(&snapshot.proposals, &book, program, None, 22.0),
        ["p-near-2", "p-far"]
    );
    // Without timelines, the snapshot's order.
    assert_eq!(
        pin_order(
            &snapshot.proposals,
            &ProposalBook::default(),
            program,
            None,
            22.0
        ),
        ["p-far", "p-near-2"]
    );
}

// ---------------------------------------------------------------- wave three

use kerf_editor::ask::{place, scope_words, GAP, HEIGHT, WIDTH};
use kerf_editor::chat::{
    card_picture_size, clock_time, cut_frames, describe_script_line, draft_dot, finishing,
    length_badge, media_stem, recent_conversations, rim_point, script_seconds, script_words,
    settled_age, settled_words, shown_frame, signed_seconds, strays_note, try_asking, work_clock,
    work_length, CARD_DONE, CARD_MARKS_AFTER, CARD_PICTURE_TALLEST, CARD_PICTURE_WIDEST,
    DRAFT_BEAT,
};
use kerf_editor::model::{ConversationSummary, Proposal};
use kerf_editor::session::{
    activity_of, ask_focus, ask_label, cutting_status, newest_change, AskScope,
};
use kerf_editor::store::{short_count, title_line, ContextLevel, ContextUse, EditorState};
use std::collections::HashSet;
use std::time::Duration;

fn apply(state: &mut EditorState, events: &[Value]) {
    for event in events {
        state.apply(event);
    }
}

#[test]
fn a_long_turn_says_its_hours_and_never_less_than_a_second() {
    assert_eq!(work_length(0.2), "1s");
    assert_eq!(work_length(41.0), "41s");
    assert_eq!(work_length(72.4), "1m 12s");
    assert_eq!(work_length(3852.0), "1h 4m");
    assert_eq!(
        worked_headline(Some(3852.0), 2),
        "Worked for 1h 4m · 2 steps"
    );
    assert_eq!(work_clock(7.9), "0:07");
    assert_eq!(work_clock(760.0), "12:40");
}

#[test]
fn the_drafting_dots_light_in_turn_and_hold_still_under_reduce_motion() {
    assert_eq!(draft_dot(Duration::ZERO, false), Some(0));
    assert_eq!(draft_dot(DRAFT_BEAT, false), Some(1));
    assert_eq!(draft_dot(DRAFT_BEAT * 2, false), Some(2));
    assert_eq!(draft_dot(DRAFT_BEAT * 3, false), Some(0));
    assert_eq!(draft_dot(DRAFT_BEAT * 5, true), None);
}

#[test]
fn the_work_row_says_the_step_kerf_is_on_beside_working() {
    let mut state = EditorState::default();
    apply(
        &mut state,
        &[
            json!({"type": "agent_busy", "busy": true}),
            json!({"type": "agent", "kind": "status", "text": "Planning  next\nstep…"}),
        ],
    );
    assert_eq!(state.work.status, "Planning next step…");
    apply(
        &mut state,
        &[
            json!({"type": "agent", "kind": "tool", "id": "t1", "state": "running", "text": "Reading the cut"}),
        ],
    );
    assert_eq!(state.work.status, "Reading the cut");
    apply(
        &mut state,
        &[
            json!({"type": "agent", "kind": "tool", "id": "t1", "state": "done", "text": "Read the cut"}),
        ],
    );
    assert_eq!(state.work.status, "Read the cut");
    // A turn starts with nothing said yet beside "Working".
    apply(
        &mut state,
        &[
            json!({"type": "agent_busy", "busy": false}),
            json!({"type": "agent_busy", "busy": true}),
        ],
    );
    assert_eq!(state.work.status, "");
}

#[test]
fn activity_keeps_what_kerf_did() {
    let line = |event: Value| activity_of(&event);
    assert_eq!(
        line(json!({"type": "agent", "kind": "tool", "state": "done", "text": "Trimmed two"})),
        Some(("tool", "Trimmed two".to_owned()))
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "tool", "state": "failed", "text": "Render failed"})),
        Some(("tool", "Render failed".to_owned()))
    );
    // A call still running is said once it ends.
    assert_eq!(
        line(json!({"type": "agent", "kind": "tool", "state": "running", "text": "Trimming"})),
        None
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "status", "text": "Planning next step…"})),
        Some(("status", "Planning next step…".to_owned()))
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "note", "text": "Asked for a key"})),
        Some(("agent", "Asked for a key".to_owned()))
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "delta", "text": "I"})),
        None
    );
    assert_eq!(
        line(json!({"type": "agent", "kind": "done", "text": "Done."})),
        None
    );
    let long = "x".repeat(300);
    assert_eq!(
        line(json!({"type": "agent", "kind": "status", "text": long}))
            .map(|(_, text)| text.chars().count()),
        Some(120)
    );
}

#[test]
fn the_context_meter_reads_its_share_its_colour_and_what_it_is_of() {
    let use_of = |used: f64, tokens: bool| ContextUse {
        conversation: None,
        used,
        limit: 200_000.0,
        tokens,
    };
    let meter = use_of(68_000.0, true);
    assert_eq!(meter.percent(), 34);
    assert_eq!(meter.level(), ContextLevel::Room);
    assert_eq!(meter.note(), "68k of 200k tokens");
    assert_eq!(meter.describe(), "Context 34% full · 68k of 200k tokens");
    assert_eq!(use_of(150_000.0, true).level(), ContextLevel::Filling);
    assert_eq!(use_of(180_000.0, true).level(), ContextLevel::Full);
    assert_eq!(
        use_of(10.0, false).note(),
        "Kerf puts older steps away as it fills"
    );
    assert_eq!(short_count(950.0), "950");
    assert_eq!(short_count(1_200_000.0), "1.2M");
    assert_eq!(short_count(12_000_000.0), "12M");
}

proptest! {
    #[test]
    fn the_context_share_stays_between_none_and_all(used in -1e9f64..1e9, limit in -1e9f64..1e9) {
        let meter = ContextUse { conversation: None, used, limit, tokens: true };
        let share = meter.fraction();
        prop_assert!((0.0..=1.0).contains(&share));
        prop_assert!(meter.percent() <= 100);
    }
}

#[test]
fn the_context_meter_shows_only_in_the_conversation_it_is_of() {
    let mut state = EditorState::default();
    // A turn in a new conversation says how full it is before it has an id.
    apply(
        &mut state,
        &[
            json!({"type": "agent_busy", "busy": true}),
            json!({"type": "agent_context", "used": 1000, "limit": 4000, "unit": "tokens", "conversation": null}),
        ],
    );
    assert_eq!(state.context_shown().map(|c| c.percent()), Some(25));
    // The bridge names it: the reading is of it.
    apply(
        &mut state,
        &[json!({"type": "conversation", "id": "c1", "title": "Trim", "items": []})],
    );
    assert_eq!(state.chosen.as_deref(), Some("c1"));
    assert_eq!(
        state
            .context
            .as_ref()
            .and_then(|c| c.conversation.as_deref()),
        Some("c1")
    );
    apply(&mut state, &[json!({"type": "agent_busy", "busy": false})]);
    assert!(state.context_shown().is_some());
    // Another conversation, or a new one, has no reading of its own.
    state.choose(Some("c2".into()));
    assert!(state.context_shown().is_none());
    state.choose(None);
    assert!(state.context_shown().is_none());
    state.choose(Some("c1".into()));
    assert!(state.context_shown().is_some());
    // A reading of another conversation is not this one's.
    apply(
        &mut state,
        &[
            json!({"type": "agent_context", "used": 1, "limit": 4000, "unit": "kerf", "conversation": "c9"}),
        ],
    );
    assert!(state.context_shown().is_none());
}

#[test]
fn a_new_conversation_is_titled_by_its_running_request() {
    let mut state = EditorState::default();
    assert_eq!(state.conversation_title(), "New conversation");
    state.work.request = Some("- **Tighten** the `intro`\nand the rest".into());
    assert_eq!(state.conversation_title(), "Tighten the intro");
    assert_eq!(title_line("* one\ntwo"), "one");
    apply(
        &mut state,
        &[json!({"type": "conversations", "items": [{"id": "c1", "title": "Harbour cut"}]})],
    );
    state.choose(Some("c1".into()));
    assert_eq!(state.conversation_title(), "Harbour cut");
}

#[test]
fn a_time_kerf_names_as_a_timecode_is_followed() {
    let mut state = EditorState::default();
    apply(
        &mut state,
        &[
            json!({"type": "state", "snapshot": {"revision": 1, "timeline":
            {"name": "Cut", "fps": 25.0, "start": 3600.0, "duration": 30.0, "tracks": []}}}),
        ],
    );
    apply(
        &mut state,
        &[
            json!({"type": "agent_focus", "times": ["01:00:04:12", 2.5, "50f", "an anchor"], "clips": []}),
        ],
    );
    assert_eq!(state.agent_focus.len(), 3);
    assert!((state.agent_focus[0] - (4.0 + 12.0 / 25.0)).abs() < 1e-9);
    assert_eq!(state.agent_focus[1], 2.5);
    assert_eq!(state.agent_focus[2], 2.0);
}

fn named_timeline() -> Timeline {
    Timeline::from_value(&json!({"name": "Cut", "fps": 25.0, "duration": 30.0,
        "sources": [{"id": "m1", "path": "media/sea.mov", "duration": 10.0, "fps": 25.0}],
        "tracks": [{"id": "V1", "type": "video", "name": "V1", "enabled": true, "clips": [
            {"id": "c1", "name": "fisher-close.mov", "timelineStart": 4.0, "duration": 2.0,
             "startFrame": 100, "durationFrames": 50, "enabled": true}]}]}))
}

#[test]
fn the_ask_popover_asks_about_the_clip_or_the_whole_sequence() {
    let timeline = named_timeline();
    let clip = ask_focus(&timeline, Some("c1"), AskScope::Clip, 0.0);
    assert_eq!(
        clip,
        json!({"scope": "clip", "clip": "c1", "fromFrame": 100, "toFrame": 150})
    );
    let around = ask_focus(&timeline, Some("c1"), AskScope::Sequence, 0.0);
    assert_eq!(around["scope"], "sequence");
    assert_eq!(around["clip"], "c1");
    // With no clip chosen, the sequence at the playhead, whatever the scope.
    let at = ask_focus(&timeline, None, AskScope::Clip, 2.0);
    assert_eq!(
        at,
        json!({"scope": "sequence", "fromFrame": 50, "toFrame": 50})
    );
    assert_eq!(
        ask_label(&timeline, Some("c1"), AskScope::Clip, 0.0),
        "fisher-close.mov"
    );
    assert_eq!(
        ask_label(&timeline, Some("c1"), AskScope::Sequence, 0.0),
        "Sequence around fisher-close.mov"
    );
    assert_eq!(
        ask_label(&timeline, None, AskScope::Sequence, 2.0),
        "Sequence at 00:00:02:00"
    );
    assert_eq!(
        scope_words(AskScope::Clip, Some("A019")),
        "This clip · A019"
    );
    assert_eq!(scope_words(AskScope::Clip, None), "Whole sequence");
    assert_eq!(
        scope_words(AskScope::Sequence, Some("A019")),
        "Whole sequence"
    );
}

proptest! {
    #[test]
    fn the_ask_popover_stays_inside_the_window(
        x in -200f32..2000.0, y in -200f32..1200.0, w in 0f32..400.0, h in 0f32..200.0,
        width in 500f32..2400.0, height in 400f32..1400.0,
    ) {
        let anchor = gpui::Bounds {
            origin: gpui::point(gpui::px(x), gpui::px(y)),
            size: gpui::size(gpui::px(w), gpui::px(h)),
        };
        let at = place(anchor, gpui::size(gpui::px(width), gpui::px(height)));
        let left = f32::from(at.x);
        prop_assert!(left >= GAP - 1e-3);
        prop_assert!(left + WIDTH <= width.max(WIDTH + 2.0 * GAP) - GAP + 1e-3);
        // Above the anchor when there is room for it there.
        if y - GAP - HEIGHT >= GAP {
            prop_assert!((f32::from(at.y) + HEIGHT + GAP - y).abs() < 1e-3);
        } else {
            prop_assert!(f32::from(at.y) > y);
        }
    }
}

#[test]
fn kerf_at_work_goes_to_its_newest_change() {
    let program = Timeline::from_value(&timeline(&[clip("a", 0.0, 4.0), clip("b", 4.0, 4.0)]));
    let trimmed = Timeline::from_value(&timeline(&[clip("a", 0.0, 3.0), clip("b", 4.0, 4.0)]));
    let changes = clip_changes(&program, &trimmed);
    let (keys, at) = newest_change(&changes, &HashSet::new());
    // A trim: where it cuts.
    assert_eq!(at, Some(3.0));
    // Seen already: nothing new to go to.
    assert_eq!(newest_change(&changes, &keys).1, None);
    let moved = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 6.0, 4.0),
        clip("c", 12.0, 2.0),
    ]));
    // A move and a new clip: the latest start of them.
    assert_eq!(
        newest_change(&clip_changes(&program, &moved), &HashSet::new()).1,
        Some(12.0)
    );
    // Taking a clip away is nothing to look at.
    let removed = Timeline::from_value(&timeline(&[clip("a", 0.0, 4.0)]));
    assert_eq!(
        newest_change(&clip_changes(&program, &removed), &HashSet::new()).1,
        None
    );
    assert_eq!(cutting_status(1), "Cutting · 1 clip");
    assert_eq!(cutting_status(3), "Cutting · 3 clips");
}

#[test]
fn a_settled_card_says_how_long_ago_it_was_settled_here() {
    assert_eq!(settled_age(Duration::from_secs(5)), "now");
    assert_eq!(settled_age(Duration::from_secs(4 * 60 + 5)), "4 min");
    assert_eq!(settled_age(Duration::from_secs(2 * 3600)), "2 h");
    assert_eq!(
        settled_words(true, Some(23), Some(Duration::from_secs(3))),
        "Accepted in rev 23 · now"
    );
    assert_eq!(settled_words(true, None, None), "Accepted");
    assert_eq!(
        settled_words(false, Some(4), Some(Duration::from_secs(300))),
        "Rejected · 5 min"
    );
}

#[test]
fn a_proposal_from_outside_says_who_made_it() {
    let proposal = |author: Value| Proposal {
        proposal_id: "p".into(),
        author,
        ..Proposal::default()
    };
    // The author as kerf-mcp stages it (kerf-runtime's `Author::new("mcp", …)`).
    assert_eq!(
        strays_note(&[proposal(
            json!({"kind": "mcp", "name": "stage", "id": "stage-client"})
        )]),
        "stage proposed this over MCP."
    );
    // Only the runtime's kinds: kerf, mcp, person and qc.
    for kind in ["kerf", "person", "qc", "agent"] {
        assert_eq!(
            strays_note(&[proposal(json!({"kind": kind, "name": "Kerf"}))]),
            "Proposed outside this conversation.",
            "{kind}"
        );
    }
    assert_eq!(
        strays_note(&[proposal(json!({"kind": "mcp", "name": "  "}))]),
        "Proposed outside this conversation."
    );
    assert_eq!(
        strays_note(&[proposal(json!("Kerf")), proposal(json!("Kerf"))]),
        "2 proposals made outside this conversation."
    );
}

#[test]
fn a_new_conversation_offers_the_latest_others_or_things_to_ask() {
    let summary = |id: &str, updated: &str, archived: bool| ConversationSummary {
        id: id.into(),
        title: id.to_uppercase(),
        updated: updated.into(),
        archived,
        ..ConversationSummary::default()
    };
    let all = [
        summary("a", "2026-01-01", false),
        summary("b", "2026-03-01", false),
        summary("c", "2026-02-01", true),
        summary("d", "2026-04-01", false),
        summary("e", "2026-05-01", false),
    ];
    let recent: Vec<&str> = recent_conversations(&all, Some("e"), 3)
        .into_iter()
        .map(|summary| summary.id.as_str())
        .collect();
    assert_eq!(recent, ["d", "b", "a"]);
    assert_eq!(
        try_asking(&["Tighten".into(), " ".into()]),
        [kerf_editor::store::NextStep::from("Tighten")]
    );
    assert_eq!(try_asking(&[]).len(), 3);
}

#[test]
fn a_script_line_reads_in_words_where_no_timeline_says_more() {
    let program = named_timeline();
    let say = |line: &str| describe_script_line(line, &program, None);
    assert_eq!(
        say("trim c1 out -1.5s"),
        ("Trim fisher-close".into(), "end −1.5 s".into())
    );
    assert_eq!(
        say("trim c1 in to=00:00:05:00"),
        ("Trim fisher-close".into(), "start to 0:05".into())
    );
    assert_eq!(
        say("remove c1"),
        (
            "Remove fisher-close".into(),
            "0:04\u{2009}–\u{2009}0:06".into()
        )
    );
    assert_eq!(
        say("add title \"Harbour\" at 00:00:12:00"),
        ("Add a title".into(), "at 0:12".into())
    );
    assert_eq!(
        say("add title t1 text=\"Harbour\" at=12s"),
        ("Add title “Harbour”".into(), "at 0:12".into())
    );
    assert_eq!(
        say("insert media=m1 at=00:00:12:00"),
        ("Insert sea".into(), "at 0:12".into())
    );
    assert_eq!(
        say("move c1 +48f"),
        ("Move fisher-close".into(), "+1.9 s".into())
    );
    assert_eq!(
        say("split c1 at=5s"),
        ("Split fisher-close".into(), "at 0:05".into())
    );
    assert_eq!(
        say("set c1 gain=-6"),
        ("Change fisher-close’s level".into(), "−6 dB".into())
    );
    assert_eq!(
        say("set c1 opacity=0.5"),
        ("Change fisher-close’s opacity".into(), "".into())
    );
    assert_eq!(
        say("fx c1 add color.basic"),
        ("Add basic to fisher-close".into(), "".into())
    );
    assert_eq!(
        say("transition c1 fx=dissolve dur=12f"),
        ("Add a dissolve".into(), "0.5 s".into())
    );
    assert_eq!(
        say("ripple everything"),
        ("Ripple everything".into(), "".into())
    );
    assert_eq!(script_words("add title \"A lower third\" at 4s").len(), 5);
    assert_eq!(script_seconds("-36f", 24.0), Some(-1.5));
    assert_eq!(script_seconds("00:01:00:12", 24.0), Some(60.5));
    assert_eq!(script_seconds("anchor", 24.0), None);
    assert_eq!(clock_time(3870.0), "1:04:30");
    assert_eq!(signed_seconds(12.0), "+12 s");
    assert_eq!(media_stem("sea.MOV"), "sea");
    assert_eq!(media_stem("clip.v2"), "clip.v2");
}

#[test]
fn a_proposal_says_the_transitions_and_properties_it_changes() {
    let fade = |id: &str, start: f64, fx: &str| json!({"id": id, "from": "a", "to": "b", "fx": fx, "start": start, "end": start + 1.0});
    let with = |clips: Vec<Value>, fades: Vec<Value>| {
        Timeline::from_value(&json!({"name": "Cut", "fps": 25.0, "duration": 30.0,
            "tracks": [{"id": "V1", "type": "video", "name": "V1", "enabled": true,
                        "clips": clips, "transitions": fades}]}))
    };
    let mut lifted = clip("a", 0.0, 4.0);
    lifted["props"] = json!({"opacity": 0.5});
    let program = with(vec![clip("a", 0.0, 4.0)], vec![fade("x", 3.5, "dissolve")]);
    let proposed = with(
        vec![lifted],
        vec![fade("x", 3.5, "wipe"), fade("y", 8.0, "dissolve")],
    );
    let changes = clip_changes(&program, &proposed);
    let rows: Vec<(String, String, String)> =
        changes.iter().map(|change| change.row(25.0)).collect();
    assert_eq!(
        rows,
        vec![
            ("A · V1".into(), "properties".into(), "".into()),
            ("Transition · V1".into(), "properties".into(), "".into()),
            (
                "Transition · V1".into(),
                "→ 00:00:08:00".into(),
                "new".into()
            ),
        ]
    );
    let gone = clip_changes(&proposed, &program);
    assert!(gone
        .iter()
        .any(|change| change.transition && change.removed));
}

#[test]
fn a_first_cuts_picture_runs_through_its_clips() {
    let cut = Timeline::from_value(&json!({"name": "Cut", "fps": 25.0, "duration": 9.0,
        "tracks": [
            {"id": "A1", "type": "audio", "name": "A1", "clips": [
                {"id": "s", "sourceId": "m9", "timelineStart": 0.0, "duration": 9.0}]},
            {"id": "V1", "type": "video", "name": "V1", "clips": [
                {"id": "b", "sourceId": "m2", "timelineStart": 3.0, "duration": 3.0},
                {"id": "a", "sourceId": "m1", "timelineStart": 0.0, "duration": 3.0}]},
            {"id": "V2", "type": "video", "name": "V2", "clips": [
                {"id": "t", "sourceId": "m3", "timelineStart": 1.0, "duration": 1.0}]}]}));
    let (frames, duration) = cut_frames(&cut);
    assert_eq!(duration, 9.0);
    assert_eq!(frames, vec![(0.0, "m1".to_owned()), (3.0, "m2".to_owned())]);
    let posters = [(0.0, true), (3.0, true), (6.0, false)];
    // Over its first half, the first clip; past three seconds in, the second.
    assert_eq!(shown_frame(&posters, 9.0, Some(0.1), false), Some(0));
    assert_eq!(shown_frame(&posters, 9.0, Some(0.5), false), Some(1));
    assert_eq!(shown_frame(&posters, 9.0, None, false), Some(0));
    // At work, the newest with a poster.
    assert_eq!(shown_frame(&posters, 9.0, None, true), Some(1));
    assert_eq!(shown_frame(&[(0.0, false)], 9.0, None, false), None);
    assert_eq!(length_badge(42.4), "0:42");
    assert_eq!(length_badge(725.0), "12:05");
    assert_eq!(
        card_picture_size(16.0 / 9.0),
        (CARD_PICTURE_WIDEST, CARD_PICTURE_WIDEST * 9.0 / 16.0)
    );
    let upright = card_picture_size(9.0 / 16.0);
    assert_eq!(upright.1, CARD_PICTURE_TALLEST);
}

#[test]
fn a_finished_cut_lights_once_says_done_then_shows_its_marks() {
    let start = finishing(Duration::ZERO);
    assert_eq!(
        (start.work, start.ring, start.chip, start.marks),
        (1.0, 1.0, 1.0, 0.0)
    );
    let later = finishing(CARD_MARKS_AFTER + Duration::from_millis(600));
    assert_eq!(later.ring, 0.0);
    assert_eq!(later.marks, 1.0);
    assert!(later.chip > 0.0);
    let done = finishing(CARD_DONE);
    assert_eq!(
        (done.work, done.ring, done.chip, done.marks),
        (0.0, 0.0, 0.0, 1.0)
    );
}

proptest! {
    #[test]
    fn the_light_runs_round_the_pictures_edge(
        x in -100f32..100.0, y in -100f32..100.0, w in 1f32..600.0, h in 1f32..400.0,
        radius in 0f32..20.0, t in -2f32..2.0,
    ) {
        let bounds = gpui::Bounds {
            origin: gpui::point(gpui::px(x), gpui::px(y)),
            size: gpui::size(gpui::px(w), gpui::px(h)),
        };
        let at = rim_point(bounds, radius, t);
        let (px_, py_) = (f32::from(at.x), f32::from(at.y));
        prop_assert!(px_ >= x - 1e-2 && px_ <= x + w + 1e-2);
        prop_assert!(py_ >= y - 1e-2 && py_ <= y + h + 1e-2);
        // On the edge: on a side, or on a corner's arc.
        let r = radius.min(w / 2.0).min(h / 2.0);
        let on_side = (px_ - x).abs() < 1e-2 || (px_ - x - w).abs() < 1e-2
            || (py_ - y).abs() < 1e-2 || (py_ - y - h).abs() < 1e-2;
        let corners = [(x + r, y + r), (x + w - r, y + r), (x + r, y + h - r), (x + w - r, y + h - r)];
        let on_arc = corners.iter().any(|(cx, cy)| ((px_ - cx).hypot(py_ - cy) - r).abs() < 1e-2);
        prop_assert!(on_side || on_arc);
    }
}

#[test]
fn a_reel_plays_only_what_changed_two_seconds_either_side() {
    use kerf_editor::chat::{reel_plan, reel_total};
    let program = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 4.0, 4.0),
        clip("c", 8.0, 4.0),
        clip("d", 20.0, 4.0),
    ]));
    let proposed = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 3.0),
        clip("b", 4.0, 3.0),
        clip("c", 8.0, 4.0),
        clip("d", 20.0, 2.0),
    ]));
    let changes = clip_changes(&program, &proposed);
    let segments = reel_plan(&changes, 24.0, 22.0, 2.0);
    // The trims of a and b touch: one stretch; then the trim of d.
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].title, "Trim A and 1 more");
    assert_eq!((segments[0].after, segments[0].after_end), (0.0, 9.0));
    assert_eq!((segments[0].before, segments[0].before_end), (0.0, 10.0));
    assert_eq!(segments[1].title, "Trim D");
    // Each side kept to its own length.
    assert_eq!((segments[1].after, segments[1].after_end), (18.0, 22.0));
    assert_eq!((segments[1].before, segments[1].before_end), (18.0, 24.0));
    assert_eq!(reel_total(&segments), 13.0);
    // Nothing changed, nothing to watch.
    assert!(reel_plan(&clip_changes(&program, &program), 24.0, 24.0, 2.0).is_empty());
}

// ---------------------------------------------------------------- review fixes

#[test]
fn under_reduce_motion_a_finished_cut_stands_as_it_ends_at_once() {
    use kerf_editor::chat::finish_drawn;
    let end = finishing(CARD_DONE);
    for since in [
        Duration::ZERO,
        Duration::from_millis(1),
        CARD_MARKS_AFTER,
        CARD_DONE,
    ] {
        // No shade, no ring, no Done: the length and the play mark, on.
        assert_eq!(finish_drawn(since, true), end, "{since:?}");
        assert_eq!(finish_drawn(since, false), finishing(since), "{since:?}");
    }
    assert_eq!(
        (end.work, end.ring, end.chip, end.marks),
        (0.0, 0.0, 0.0, 1.0)
    );
}

#[test]
fn a_first_cut_renders_again_once_a_reel_over_it_ends() {
    use kerf_editor::session::{CutRender, CUT_SETTLE};
    use std::time::Instant;
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let settle = CUT_SETTLE.as_millis() as u64;
    let mut cut = CutRender::default();
    // Kerf stages a first cut; once its edits are quiet it renders.
    cut.follow(Some("p1"), true, false, at(0));
    assert_eq!(cut.take_due(Some("p1"), false, at(settle - 1)), None);
    assert_eq!(
        cut.take_due(Some("p1"), false, at(settle)),
        Some("p1".to_owned())
    );
    cut.set_rendering(true);
    cut.rendered(Some("p1"), false, true, at(settle + 100));
    assert!(!cut.rendering());
    assert_eq!(cut.due(), None);
    // The same cut again: it was rendered, nothing is due.
    cut.follow(Some("p1"), true, false, at(settle + 200));
    assert_eq!(cut.due(), None);
    // Watch what changed, then Exit: the program came back under the
    // reel's picture, so the cut renders again once settled.
    let exit = settle + 5_000;
    cut.reel_ended(Some("p1"), at(exit));
    assert_eq!(cut.previewed(), None);
    assert_eq!(cut.take_due(Some("p1"), false, at(exit + settle - 1)), None);
    assert_eq!(
        cut.take_due(Some("p1"), false, at(exit + settle)),
        Some("p1".to_owned())
    );
    // A reel that ends with no first cut shown leaves nothing to render.
    let mut none = CutRender::default();
    none.reel_ended(None, at(0));
    assert_eq!(none.due(), None);
}

#[derive(Clone, Debug)]
enum CutStep {
    Follow(Option<u8>, bool, bool),
    TakeDue(Option<u8>, bool),
    Asked(bool),
    Rendered(Option<u8>, bool, bool),
    ReelEnded(Option<u8>),
}

fn cut_step() -> impl Strategy<Value = CutStep> {
    let id = proptest::option::of(0u8..3);
    prop_oneof![
        (id.clone(), any::<bool>(), any::<bool>())
            .prop_map(|(id, follow, reel)| CutStep::Follow(id, follow, reel)),
        (id.clone(), any::<bool>()).prop_map(|(id, reel)| CutStep::TakeDue(id, reel)),
        any::<bool>().prop_map(CutStep::Asked),
        (id.clone(), any::<bool>(), any::<bool>())
            .prop_map(|(id, still, follow)| CutStep::Rendered(id, still, follow)),
        id.prop_map(CutStep::ReelEnded),
    ]
}

proptest! {
    /// Whatever the bridge says in whatever order: a render is given only
    /// for the cut shown, never while one runs or a reel plays, and never
    /// before it is due; what is due is never further than `CUT_SETTLE`
    /// from the last step; a cut no longer shown forgets it all; and a
    /// reel ending over a cut always leaves it to render.
    #[test]
    fn a_first_cut_renders_only_when_settled_and_free(
        steps in proptest::collection::vec((cut_step(), 0u64..800), 1..60)
    ) {
        use kerf_editor::session::{CutRender, CUT_SETTLE};
        use std::time::Instant;
        let name = |id: Option<u8>| id.map(|id| format!("p{id}"));
        let mut now = Instant::now();
        let mut cut = CutRender::default();
        for (step, wait) in steps {
            now += Duration::from_millis(wait);
            let due_before = cut.due();
            let rendering_before = cut.rendering();
            match step {
                CutStep::Follow(id, follow, reel) => {
                    let id = name(id);
                    cut.follow(id.as_deref(), follow, reel, now);
                    if id.is_none() {
                        prop_assert_eq!(cut.due(), None);
                        prop_assert_eq!(cut.previewed(), None);
                    }
                }
                CutStep::TakeDue(id, reel) => {
                    let id = name(id);
                    let given = cut.take_due(id.as_deref(), reel, now);
                    if let Some(given) = given {
                        prop_assert_eq!(Some(&given), id.as_ref());
                        prop_assert!(!reel && !rendering_before);
                        prop_assert!(due_before.is_some_and(|due| due <= now));
                        prop_assert_eq!(cut.previewed(), Some(given.as_str()));
                        prop_assert_eq!(cut.due(), None);
                    }
                }
                CutStep::Asked(taken) => cut.set_rendering(taken),
                CutStep::Rendered(id, still, follow) => {
                    let id = name(id);
                    cut.rendered(id.as_deref(), still, follow, now);
                    if still {
                        prop_assert_eq!(cut.rendering(), rendering_before);
                    }
                }
                CutStep::ReelEnded(id) => {
                    let id = name(id);
                    cut.reel_ended(id.as_deref(), now);
                    if id.is_some() {
                        prop_assert_eq!(cut.previewed(), None);
                        prop_assert_eq!(cut.due(), Some(now + CUT_SETTLE));
                    } else {
                        prop_assert_eq!(cut.due(), due_before);
                    }
                }
            }
            if let Some(due) = cut.due() {
                prop_assert!(due <= now + CUT_SETTLE);
            }
        }
    }
}

// ---------------------------------------------------------------- wave five

use kerf_editor::ask::scope_choices;
use kerf_editor::chat::permission_look;
use kerf_editor::session::{preview_request, preview_span, FocusBook, Focused, REEL_MOST_RENDERED};
use kerf_editor::store::{
    next_steps, preview_notice, NextStep, Preview, PREVIEW_NOTICE_HOLD, PREVIEW_REQUIRED,
};

/// A step as the bridge sends it, or as plain words.
#[derive(Clone, Debug)]
enum Shape {
    Object(String, String),
    Words(String),
}

fn shape() -> impl Strategy<Value = Shape> {
    let words = "[ a-zA-Z]{0,12}";
    prop_oneof![
        (words, words).prop_map(|(label, request)| Shape::Object(label, request)),
        words.prop_map(Shape::Words),
    ]
}

fn as_value(shape: &Shape) -> Value {
    match shape {
        Shape::Object(label, request) => json!({"label": label, "request": request}),
        Shape::Words(words) => json!(words),
    }
}

/// What Qt read of a step: the words as label and request, or the object's
/// label and request (its label when it has none); none without a label.
fn as_qt_read(shape: &Shape) -> Option<NextStep> {
    let (label, request) = match shape {
        Shape::Object(label, request) => (label.trim(), request.trim()),
        Shape::Words(words) => (words.trim(), words.trim()),
    };
    (!label.is_empty())
        .then(|| NextStep::new(label, if request.is_empty() { label } else { request }))
}

proptest! {
    /// Kerf's next steps read in either shape: `{label, request}` as the
    /// bridge sends them, or plain words as both; a step with no label is
    /// left out, the rest kept in their order.
    #[test]
    fn next_steps_read_in_either_shape(shapes in proptest::collection::vec(shape(), 0..6)) {
        let event = json!({"type": "agent_next", "steps": shapes.iter().map(as_value).collect::<Vec<_>>()});
        let expected: Vec<NextStep> = shapes.iter().filter_map(as_qt_read).collect();
        prop_assert_eq!(&next_steps(&event), &expected);
        for step in &expected {
            prop_assert!(!step.label.is_empty() && !step.request.is_empty());
            prop_assert_eq!(step.label.trim(), step.label.as_str());
        }
        let mut state = EditorState::default();
        state.apply(&event);
        prop_assert_eq!(&state.next_steps, &expected);
        // Try asking shows at most four of them, else the first requests.
        let offered = try_asking(&state.next_steps);
        if expected.is_empty() {
            prop_assert_eq!(offered.len(), 3);
        } else {
            prop_assert_eq!(&offered[..], &expected[..expected.len().min(4)]);
        }
    }
}

#[test]
fn kerf_s_next_steps_show_their_label_and_ask_their_request() {
    let mut state = EditorState::default();
    state.apply(&json!({"type": "agent_next", "steps": [
        {"label": "Stronger glow", "request": "Make the glow on the title stronger"},
        "Tighten the pauses",
        {"label": "  ", "request": "nothing to press"},
        {"label": "Captions"},
        42
    ]}));
    assert_eq!(
        state.next_steps,
        [
            NextStep::new("Stronger glow", "Make the glow on the title stronger"),
            NextStep::new("Tighten the pauses", "Tighten the pauses"),
            NextStep::new("Captions", "Captions"),
        ]
    );
    // An empty conversation's Try asking offers them, label and request.
    assert_eq!(try_asking(&state.next_steps), state.next_steps);
    // A new turn takes them away until Kerf offers more.
    state.apply(&json!({"type": "agent_busy", "busy": true}));
    assert!(state.next_steps.is_empty());
    assert_eq!(
        try_asking(&state.next_steps)[0].label,
        "Make a 30-second cut from the best takes"
    );
}

#[test]
fn what_the_bridge_says_of_the_preview_is_said_as_qt_said_it() {
    let said = |event: Value| preview_notice(&event);
    let reason = "A title needs a render";
    assert_eq!(
        said(json!({"type": "preview", "status": "required", "message": reason})),
        Some((reason.to_owned(), None))
    );
    assert_eq!(
        said(json!({"type": "preview", "status": "required", "message": ""})),
        Some((PREVIEW_REQUIRED.to_owned(), None))
    );
    assert_eq!(
        said(json!({"type": "preview", "status": "unavailable", "message": "No GPU"})),
        Some(("No GPU".to_owned(), None))
    );
    assert_eq!(
        said(json!({"type": "preview", "status": "ready", "proposalId": "p1"})),
        Some(("Preview ready".to_owned(), Some(PREVIEW_NOTICE_HOLD)))
    );
    assert_eq!(
        said(json!({"type": "preview", "status": "live", "message": "masks are drawn soft"})),
        Some((
            "Live preview: masks are drawn soft".to_owned(),
            Some(PREVIEW_NOTICE_HOLD)
        ))
    );
    assert_eq!(
        said(json!({"type": "preview", "status": "live"})),
        Some(("Live preview".to_owned(), Some(PREVIEW_NOTICE_HOLD)))
    );
    for quiet in ["running", "stopped", "exited", "failed", ""] {
        assert_eq!(
            said(json!({"type": "preview", "status": quiet})),
            None,
            "{quiet}"
        );
    }
    assert_eq!(PREVIEW_NOTICE_HOLD, Duration::from_millis(4000));
    // Activity keeps why a render is needed, in Qt's words without one.
    assert_eq!(
        activity_of(&json!({"type": "preview", "status": "required", "message": reason})),
        Some(("preview", reason.to_owned()))
    );
    assert_eq!(
        activity_of(&json!({"type": "preview", "status": "required"})),
        Some(("preview", "Native preview unavailable".to_owned()))
    );
    assert_eq!(
        activity_of(&json!({"type": "preview", "status": "ready"})),
        None
    );
    // A render that failed is kept in the words of the `required` after it.
    assert_eq!(
        activity_of(&json!({"type": "preview", "status": "failed", "proposalId": "p1"})),
        None
    );
}

#[test]
fn the_preview_s_words_take_the_last_ones_place() {
    let mut state = EditorState::default();
    state.notify("Exported cut.mov", false);
    state.apply(
        &json!({"type": "preview", "status": "required", "message": "A title needs a render"}),
    );
    let texts = |state: &EditorState| -> Vec<String> {
        state
            .notices
            .iter()
            .map(|notice| notice.text.clone())
            .collect()
    };
    assert_eq!(
        texts(&state),
        ["Exported cut.mov", "A title needs a render"]
    );
    assert_eq!(state.notices[1].hold, None, "the window's 6 s");
    assert!(!state.notices[1].error);
    // Rendered: "Preview ready" in its place, for 4 s.
    state.apply(&json!({"type": "preview", "status": "running", "progress": 0.5}));
    state.apply(&json!({"type": "preview", "status": "ready", "proposalId": "p1"}));
    assert_eq!(texts(&state), ["Exported cut.mov", "Preview ready"]);
    assert_eq!(state.notices[1].hold, Some(PREVIEW_NOTICE_HOLD));
    // The same words again stay where they are, held afresh.
    let id = state.notices[1].id;
    state.apply(&json!({"type": "preview", "status": "ready"}));
    assert_eq!(texts(&state), ["Exported cut.mov", "Preview ready"]);
    assert_eq!(state.notices[1].id, id);
    state.apply(&json!({"type": "preview", "status": "live", "message": "masks are soft"}));
    assert_eq!(
        texts(&state),
        ["Exported cut.mov", "Live preview: masks are soft"]
    );
    // It keeps its own 4 s, past the window's hold.
    state.expire_notices(Duration::ZERO, Duration::ZERO);
    assert_eq!(texts(&state), ["Live preview: masks are soft"]);
    // Gone, what the preview says next is said anew.
    state.notices.clear();
    state.apply(&json!({"type": "preview", "status": "ready"}));
    assert_eq!(texts(&state), ["Preview ready"]);
}

#[test]
fn a_failed_render_is_said_once_in_the_bridge_s_words() {
    let mut state = EditorState::default();
    // As the bridge says a render that failed: failed, then why a render
    // is still needed.
    state.apply(&json!({"type": "preview", "status": "failed", "proposalId": "p1"}));
    state.apply(
        &json!({"type": "preview", "status": "required", "proposalId": "p1",
        "message": "Preview render failed. Retry after fixing the reported error."}),
    );
    let lines: Vec<(String, bool, Option<Duration>)> = state
        .notices
        .iter()
        .map(|notice| (notice.text.clone(), notice.error, notice.hold))
        .collect();
    assert_eq!(
        lines,
        [(
            "Preview render failed. Retry after fixing the reported error.".to_owned(),
            true,
            Some(kerf_editor::theme::motion::NOTICE_HOLD)
        )]
    );
    assert_eq!(
        state.preview.reason().as_deref(),
        Some("Preview render failed. Retry after fixing the reported error.")
    );
    // Rendered again: ready in its place.
    state.apply(&json!({"type": "preview", "status": "running", "proposalId": "p1"}));
    state.apply(&json!({"type": "preview", "status": "ready", "proposalId": "p1"}));
    assert_eq!(state.notices.len(), 1);
    assert_eq!(state.notices[0].text, "Preview ready");
    assert!(!state.notices[0].error);
    assert!(state.preview_line().is_some());
}

#[test]
fn the_render_preview_button_says_why_a_render_is_needed() {
    let preview = |status: &str, message: Option<&str>| Preview {
        status: status.to_owned(),
        message: message.map(str::to_owned),
        ..Preview::default()
    };
    assert_eq!(
        preview("required", Some("Effects need a render"))
            .reason()
            .as_deref(),
        Some("Effects need a render")
    );
    assert_eq!(
        preview("required", Some("  ")).reason().as_deref(),
        Some("Rendered preview required")
    );
    assert_eq!(
        preview("unavailable", None).reason().as_deref(),
        Some("Rendered preview required")
    );
    for status in ["ready", "live", "running", "stopped", ""] {
        assert_eq!(preview(status, Some("x")).reason(), None, "{status}");
    }
    // As the store keeps it from the bridge's event.
    let mut state = EditorState::default();
    state.apply(
        &json!({"type": "preview", "status": "required", "message": "Effects need a render"}),
    );
    assert_eq!(
        state.preview.reason().as_deref(),
        Some("Effects need a render")
    );
}

#[test]
fn a_card_s_preview_renders_only_what_the_proposal_changes() {
    let snapshot = Snapshot::from_value(&snapshot());
    // p-far trims a at 3.52 s: from a's start to its old end.
    assert_eq!(
        preview_request(&snapshot, Some("p-far")),
        json!({"type": "preview", "proposalId": "p-far", "start": 0.0, "duration": 4.0})
    );
    // An earlier version renders what it changes itself.
    assert_eq!(
        preview_request(&snapshot, Some("p-near-1")),
        json!({"type": "preview", "proposalId": "p-near-1", "start": 20.0, "duration": 4.5})
    );
    assert_eq!(
        preview_request(&snapshot, Some("p-near-2")),
        json!({"type": "preview", "proposalId": "p-near-2", "start": 20.0, "duration": 5.0})
    );
    // One with no timeline known renders whole; so does the program.
    assert_eq!(
        preview_request(&snapshot, Some("p-done")),
        json!({"type": "preview", "proposalId": "p-done"})
    );
    assert_eq!(preview_request(&snapshot, None), json!({"type": "preview"}));
    assert_eq!(
        preview_request(&snapshot, Some("unknown")),
        json!({"type": "preview", "proposalId": "unknown"})
    );
}

#[test]
fn a_preview_s_stretch_is_held_to_a_frame_and_to_two_minutes() {
    let program = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 300.0, 4.0),
        clip("c", 400.0, 4.0),
    ]));
    // Moved far: from where it was to where it goes, at most 120 s.
    let moved = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 10.0, 4.0),
        clip("c", 400.0, 4.0),
    ]));
    assert_eq!(
        preview_span(&program, &moved),
        Some((10.0, REEL_MOST_RENDERED))
    );
    let program = Timeline::from_value(&timeline(&[clip("a", 0.0, 4.0), clip("b", 300.0, 4.0)]));
    // A clip added with no length still renders a frame (25 fps).
    let added = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 300.0, 4.0),
        clip("c", 7.0, 0.0),
    ]));
    assert_eq!(preview_span(&program, &added), Some((7.0, 1.0 / 25.0)));
    assert_eq!(preview_span(&program, &program), None);
}

proptest! {
    /// Whatever a proposal moves, its preview starts where the earliest
    /// change does, never before the program, lasts a frame to 120 s, and
    /// ends where the change does or, sooner, where the proposal does.
    #[test]
    fn a_preview_s_stretch_covers_the_earliest_change(
        from in 0.0f64..400.0,
        to in 0.0f64..400.0,
        length in 0.05f64..60.0,
    ) {
        let program = Timeline::from_value(&timeline(&[clip("a", from, length)]));
        let proposed = Timeline::from_value(&timeline(&[clip("a", to, length)]));
        let span = preview_span(&program, &proposed);
        if (from - to).abs() <= 1e-4 {
            prop_assert_eq!(span, None);
        } else {
            let (start, duration) = span.expect("a stretch");
            prop_assert!((start - from.min(to)).abs() < 1e-9);
            prop_assert!(start >= 0.0);
            prop_assert!((1.0 / 25.0 - 1e-12..=REEL_MOST_RENDERED).contains(&duration));
            // Moved later, it renders from where it was to its new end;
            // moved earlier, the proposal ends with it.
            let end = to + length;
            prop_assert!((duration - (end - start).min(REEL_MOST_RENDERED)).abs() < 1e-9);
            prop_assert!(start + duration <= end + 1e-9);
        }
    }
}

fn focused(clip: &str, label: &str, stamp: f64) -> Focused {
    Focused {
        focus: json!({"scope": "clip", "clip": clip, "fromFrame": 0, "toFrame": 24}),
        label: label.to_owned(),
        stamp: Some(stamp),
    }
}

#[test]
fn ask_again_goes_about_what_the_request_was_about() {
    let mut book = FocusBook::default();
    assert!(book.is_empty() && book.last().is_none());
    book.remember("Tighten it", focused("c1", "A019_C001", 2.0));
    book.remember(" Brighter ", focused("c2", "Sequence around B", 5.0));
    // By its words, as the conversation keeps them.
    assert_eq!(
        book.of("Tighten it"),
        Some(&focused("c1", "A019_C001", 2.0))
    );
    assert_eq!(
        book.of("Brighter"),
        Some(&focused("c2", "Sequence around B", 5.0))
    );
    assert_eq!(book.last(), Some(&focused("c2", "Sequence around B", 5.0)));
    assert_eq!(book.of("Something else"), None);
    // Asked again about something else, the newest is what it went about.
    book.remember("Tighten it", focused("c3", "Sequence at 00:00:01:00", 1.0));
    assert_eq!(
        book.of("Tighten it"),
        Some(&focused("c3", "Sequence at 00:00:01:00", 1.0))
    );
    assert_eq!(book.len(), 2);
    book.remember("   ", focused("c4", "x", 0.0));
    assert_eq!(book.len(), 2);
    // Only the newest are kept.
    for index in 0..FocusBook::KEPT + 5 {
        book.remember(&format!("request {index}"), focused("c", "x", 0.0));
    }
    assert_eq!(book.len(), FocusBook::KEPT);
    assert_eq!(book.of("Tighten it"), None);
    assert!(book
        .of(&format!("request {}", FocusBook::KEPT + 4))
        .is_some());
}

#[test]
fn the_permission_chip_carries_its_mode_s_icon_and_tone() {
    use kerf_editor::icons::Icon;
    use kerf_editor::theme;
    assert_eq!(permission_look(Permissions::Ask), (Icon::Lock, None));
    assert_eq!(
        permission_look(Permissions::AcceptEdits),
        (Icon::Check, Some(theme::accent_ink()))
    );
    let (_, auto) = permission_look(Permissions::Auto);
    assert_eq!(auto, Some(theme::activity::gpu()));
}

#[test]
fn the_scope_menu_offers_the_clip_then_the_sequence_and_jumps_by_letter() {
    assert_eq!(
        scope_choices(Some("A019_C001")),
        [
            (AskScope::Clip, "This clip · A019_C001".to_owned()),
            (AskScope::Sequence, "Whole sequence".to_owned()),
        ]
    );
    assert_eq!(
        scope_choices(None),
        [(AskScope::Sequence, "Whole sequence".to_owned())]
    );
    let choices = scope_choices(Some("A019_C001"));
    let words: Vec<&str> = choices.iter().map(|(_, words)| words.as_str()).collect();
    let enabled = [true, true];
    assert_eq!(
        kerf_editor::ui::menu_letter(&words, &enabled, Some(0), 'w'),
        Some(1)
    );
    assert_eq!(
        kerf_editor::ui::menu_letter(&words, &enabled, Some(1), 'T'),
        Some(0)
    );
    assert_eq!(
        kerf_editor::ui::menu_letter(&words, &enabled, None, 'x'),
        None
    );
}

#[test]
fn an_archived_conversation_stays_open() {
    let mut state = EditorState::default();
    state.choose(Some("c1".into()));
    state.apply(&json!({"type": "conversation", "id": "c1", "title": "Trim", "archived": true, "items": []}));
    state.apply(&json!({"type": "conversations", "items": [
        {"id": "c1", "title": "Trim", "archived": true, "count": 2},
        {"id": "c2", "title": "Other", "archived": false, "count": 1}
    ]}));
    assert_eq!(state.chosen.as_deref(), Some("c1"));
    assert!(state
        .open_conversation
        .as_ref()
        .is_some_and(|open| open.archived));
    assert!(state.conversations[0].archived);
    // Deleted, it goes.
    state.apply(&json!({"type": "conversations", "items": [
        {"id": "c2", "title": "Other", "archived": false, "count": 1}
    ]}));
    assert_eq!(state.chosen, None);
}

// ------------------------------------------------- wave five review fixes

/// Where a timeline's last clip ends: how long kerfd lets a render of it run.
fn last_end(timeline: &Timeline) -> f64 {
    timeline
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| clip.end())
        .fold(0.0, f64::max)
}

#[test]
fn a_preview_of_a_shorter_programme_ends_where_the_proposal_does() {
    // Tighten the pauses: b and c come earlier, the programme ends at 12.
    let program = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 5.0, 4.0),
        clip("c", 10.0, 4.0),
    ]));
    let tightened = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 4.0, 4.0),
        clip("c", 8.0, 4.0),
    ]));
    assert_eq!(preview_span(&program, &tightened), Some((4.0, 8.0)));
    // The last clip trimmed: from where it starts to its new end.
    let trimmed = Timeline::from_value(&timeline(&[
        clip("a", 0.0, 4.0),
        clip("b", 5.0, 4.0),
        clip("c", 10.0, 1.5),
    ]));
    assert_eq!(preview_span(&program, &trimmed), Some((10.0, 1.5)));
    // The last clip taken out: nothing it changes lies within the
    // proposal, which renders whole.
    let cut = Timeline::from_value(&timeline(&[clip("a", 0.0, 4.0), clip("b", 5.0, 4.0)]));
    assert_eq!(preview_span(&program, &cut), None);
    // The card's request says the same.
    let mut value = snapshot();
    value["timeline"] = timeline(&[clip("a", 0.0, 4.0), clip("b", 4.0, 3.0)]);
    value["proposalTimelines"]["p-far"] = timeline(&[clip("a", 0.0, 4.0)]);
    value["proposalTimelines"]["p-near-2"] = timeline(&[clip("a", 0.0, 3.0), clip("b", 3.0, 3.0)]);
    let snapshot = Snapshot::from_value(&value);
    assert_eq!(
        preview_request(&snapshot, Some("p-far")),
        json!({"type": "preview", "proposalId": "p-far"})
    );
    assert_eq!(
        preview_request(&snapshot, Some("p-near-2")),
        json!({"type": "preview", "proposalId": "p-near-2", "start": 0.0, "duration": 6.0})
    );
}

/// What a proposal does to one clip of a cut.
#[derive(Clone, Debug)]
enum Edit {
    Keep,
    Remove,
    Trim(f64),
    Shift(f64),
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        Just(Edit::Keep),
        Just(Edit::Remove),
        (0.04f64..1.0).prop_map(Edit::Trim),
        (-30.0f64..30.0).prop_map(Edit::Shift),
    ]
}

proptest! {
    /// Whatever a proposal takes out, trims or moves, kerfd takes the
    /// stretch its preview asks for: it starts at the program's start or
    /// later, lasts a frame to 120 s, and ends within the proposal; and it
    /// starts where the earliest change does whenever it is asked for.
    #[test]
    fn kerfd_takes_every_preview_stretch_a_proposal_asks_for(
        cut in proptest::collection::vec((0.0f64..20.0, 0.5f64..40.0, edit()), 1..6),
    ) {
        let mut at = 0.0;
        let mut before = Vec::new();
        let mut after = Vec::new();
        for (index, (gap, length, edit)) in cut.iter().enumerate() {
            let id = format!("c{index}");
            let start = at + gap;
            at = start + length;
            before.push(clip(&id, start, *length));
            match edit {
                Edit::Keep => after.push(clip(&id, start, *length)),
                Edit::Remove => {}
                Edit::Trim(part) => after.push(clip(&id, start, length * part)),
                Edit::Shift(by) => after.push(clip(&id, (start + by).max(0.0), *length)),
            }
        }
        let program = Timeline::from_value(&timeline(&before));
        let proposed = Timeline::from_value(&timeline(&after));
        let frame = 1.0 / 25.0;
        let changes = clip_changes(&program, &proposed);
        if let Some((start, duration)) = preview_span(&program, &proposed) {
            prop_assert!(start >= 0.0);
            prop_assert!((frame - 1e-12..=REEL_MOST_RENDERED).contains(&duration));
            prop_assert!(start + duration <= last_end(&proposed) + 1e-9);
            let earliest = changes
                .iter()
                .map(|change| change.span().0)
                .fold(f64::INFINITY, f64::min);
            prop_assert!((start - earliest.max(0.0)).abs() < 1e-9);
        } else {
            // Nothing changed, or nothing of it a frame inside the proposal.
            let earliest = changes
                .iter()
                .map(|change| change.span().0.max(0.0))
                .fold(f64::INFINITY, f64::min);
            prop_assert!(changes.is_empty() || last_end(&proposed) - earliest < frame);
        }
    }
}

use kerf_editor::chat::Queued;

#[test]
fn ask_again_pressed_while_kerf_works_keeps_what_it_was_about() {
    let about = focused("c1", "A019_C001", 2.0);
    let mut queued = Queued::default();
    queued.push_again(" Tighten it ", 9.0, Some(about.clone()));
    assert_eq!(queued.text(), Some("Tighten it"));
    assert_eq!(queued.about(), Some(&about));
    // Pressed again, nothing doubles and it is still about the clip.
    queued.push_again("Tighten it", 11.0, Some(focused("c9", "x", 0.0)));
    assert_eq!(queued.text(), Some("Tighten it"));
    assert_eq!(queued.about(), Some(&about));
    queued.push_again("   ", 12.0, None);
    assert_eq!(queued.about(), Some(&about));
    assert_eq!(queued.take(), Some(("Tighten it".to_owned(), 9.0)));
    assert_eq!(queued.about(), None);
    // Words this window never sent go as the composer's do.
    queued.push_again("Brighter", 3.0, None);
    assert_eq!(queued.about(), None);
    assert_eq!(queued.take(), Some(("Brighter".to_owned(), 3.0)));
    // Joined with a request written meanwhile, it goes as that does: about
    // the sequence at the latest one's time.
    queued.push_again("Tighten it", 9.0, Some(about.clone()));
    queued.push("and add a title", 14.0);
    assert_eq!(queued.about(), None);
    assert_eq!(
        queued.take(),
        Some(("Tighten it\n\nand add a title".to_owned(), 14.0))
    );
    queued.push("trim this", 2.0);
    queued.push_again("Tighten it", 5.0, Some(about));
    assert_eq!(queued.about(), None);
    assert_eq!(queued.text(), Some("trim this\n\nTighten it"));
}

/// Something pressed or written while Kerf works.
#[derive(Clone, Debug)]
enum Waits {
    Written(String, f64),
    Again(String, f64, bool),
}

fn waits() -> impl Strategy<Value = Waits> {
    let words = prop_oneof![Just("Tighten it".to_owned()), "[ a-c]{0,4}"];
    prop_oneof![
        (words.clone(), 0.0f64..60.0).prop_map(|(text, at)| Waits::Written(text, at)),
        (words, 0.0f64..60.0, any::<bool>())
            .prop_map(|(text, at, sent)| Waits::Again(text, at, sent)),
    ]
}

proptest! {
    /// What waits keeps an Ask again's focus only while that request is
    /// all that waits; its words never double from being pressed twice.
    #[test]
    fn an_ask_again_s_focus_waits_only_while_it_waits_alone(
        steps in proptest::collection::vec(waits(), 0..8),
    ) {
        let mut queued = Queued::default();
        // The words that wait, their time, and the focus they go about.
        let mut model: Option<(String, f64, Option<Focused>)> = None;
        for step in &steps {
            match step {
                Waits::Written(text, at) => {
                    queued.push(text, *at);
                    if !text.trim().is_empty() {
                        let waiting = model.as_ref().map(|(text, _, _)| text.as_str());
                        model = Some((join_queued(waiting, text), *at, None));
                    }
                }
                Waits::Again(text, at, sent) => {
                    let about = sent.then(|| focused("c1", text.trim(), *at));
                    queued.push_again(text, *at, about.clone());
                    let text = text.trim();
                    model = match model.take() {
                        waiting if text.is_empty() => waiting,
                        None => Some((text.to_owned(), *at, about)),
                        Some(waiting) if waiting.0 == text => Some(waiting),
                        Some((waiting, _, _)) => {
                            Some((join_queued(Some(&waiting), text), *at, None))
                        }
                    };
                }
            }
            prop_assert_eq!(queued.text(), model.as_ref().map(|(text, _, _)| text.as_str()));
            prop_assert_eq!(queued.about(), model.as_ref().and_then(|(_, _, about)| about.as_ref()));
        }
        prop_assert_eq!(queued.take(), model.map(|(text, at, _)| (text, at)));
        prop_assert_eq!(queued.about(), None);
    }
}
