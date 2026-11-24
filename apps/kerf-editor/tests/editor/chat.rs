//! The conversation: what is said and asked, the composer, the proposals Kerf stages and how a turn reads.

use crate::support::{apply, clip, event, preview, snapshot, texts, timeline_value};
use kerf_bridge::event::{Check, CheckSummary};
use kerf_bridge::Event;
use kerf_editor::ask::{place, scope_choices, scope_words, GAP, HEIGHT, WIDTH};
use kerf_editor::changes::{clip_changes, signed_frames};
use kerf_editor::chat::{
    access_reason, card_picture_size, card_turns, check_suggestions, clock_time, colour_change,
    cut_frames, describe_script_line, draft_dot, effect_label, finishing, join_queued,
    length_badge, media_stem, model_choices, model_label, next_permissions, permission_look,
    permission_words, pin_order, previewing, recent_conversations, refusal, request_before,
    rim_point, say_permissions, script_seconds, script_summary, script_words, settled_age,
    settled_age_changes, settled_words, shown_frame, signed_seconds, spans, strays_note,
    timecode_position, timecode_seconds, try_asking, work_clock, work_length, worked_headline,
    ProposalBook, Queued, Span, CARD_DONE, CARD_MARKS_AFTER, CARD_PICTURE_TALLEST,
    CARD_PICTURE_WIDEST, DRAFT_BEAT, MODE_HOLD,
};
use kerf_editor::model::{
    timecode, ConversationSummary, Proposal, ProposalFreshness, ProposalStatus, Snapshot,
    SnapshotShow, Speaker, Timeline, TimelineShow, Turn,
};
use kerf_editor::palette::unavailable;
use kerf_editor::prefs::{Permissions, Provider};
use kerf_editor::session::{ask_focus, ask_label, proposal_context, AskScope, FocusBook, Focused};
use kerf_editor::store::{
    short_count, title_line, ContextLevel, ContextUse, EditorState, StatusLine,
};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::time::Duration;

// ------------------------------------------------------------ the composer

#[test]
fn the_composer_says_why_it_cannot_ask() {
    let no_folder = refusal(false, false, false, None).unwrap();
    assert_eq!(no_folder.placeholder, "Open a folder to ask Kerf");
    assert_eq!(no_folder.reason, "Open a folder or start a project first");
    // No folder comes first, whatever else holds.
    assert_eq!(
        refusal(
            false,
            true,
            false,
            Some("Add your API key in Settings → Chat")
        )
        .unwrap()
        .placeholder,
        "Open a folder to ask Kerf"
    );
    let offline = refusal(true, true, false, Some("x")).unwrap();
    assert_eq!(offline.placeholder, "Connect to ask Kerf");
    assert_eq!(offline.reason, "Connect to start editing");
    let no_key = refusal(
        true,
        false,
        false,
        Some("Add your API key in Settings → Chat"),
    )
    .unwrap();
    assert_eq!(no_key.placeholder, "Set up Kerf in Settings to ask");
    assert_eq!(no_key.reason, "Add your API key in Settings → Chat");
    // While Kerf works the request waits its turn instead.
    assert_eq!(refusal(true, false, true, Some("x")), None);
    assert_eq!(refusal(true, false, false, None), None);
    assert_eq!(refusal(true, false, false, Some("  ")), None);
}

#[test]
fn the_reason_ask_cannot_run_is_the_registrys_for_the_connections_access() {
    assert_eq!(
        access_reason("noKey").as_deref(),
        Some("Add your API key in Settings → Chat")
    );
    assert_eq!(
        access_reason("signedOut").as_deref(),
        Some("Sign in to your Kerf account in Settings")
    );
    assert_eq!(
        access_reason("outOfCredits").as_deref(),
        Some("You've used this period's requests to Kerf")
    );
    assert_eq!(access_reason("ready"), None);
}

proptest! {
    /// A request can go exactly when a folder is open, kerfd is there, and
    /// the connection takes requests (or Kerf is working, and it waits).
    #[test]
    fn a_request_is_refused_only_for_a_reason(
        has_project in any::<bool>(),
        disconnected in any::<bool>(),
        busy in any::<bool>(),
        reason in proptest::option::of("[a-zA-Z →]{0,12}"),
    ) {
        let refused = refusal(has_project, disconnected, busy, reason.as_deref());
        let can_take = reason.as_deref().is_none_or(|reason| reason.trim().is_empty()) || busy;
        prop_assert_eq!(refused.is_none(), has_project && !disconnected && can_take);
        if let Some(refused) = refused {
            prop_assert!(!refused.reason.is_empty());
        }
    }
}

#[test]
fn a_second_request_written_while_kerf_works_goes_with_its_own_playhead_time() {
    let mut queued = Queued::default();
    assert_eq!(queued.text(), None);
    queued.push("trim this", 2.0);
    queued.push("  ", 9.0);
    queued.push("and add a title here", 5.0);
    assert_eq!(queued.text(), Some("trim this\n\nand add a title here"));
    // The joined request goes at 5, where the latest was written and the
    // picture last looked at, not at 2.
    assert_eq!(
        queued.take(),
        Some(("trim this\n\nand add a title here".to_owned(), 5.0))
    );
    assert_eq!(queued.take(), None);
    queued.push("cut the end", 7.0);
    queued.clear();
    assert_eq!(queued.text(), None);
}

proptest! {
    /// What waits is every request written, joined, at the playhead time of
    /// the latest that said anything; taken, nothing waits.
    #[test]
    fn what_waits_is_every_request_at_the_latest_ones_time(
        requests in proptest::collection::vec(("[ a-z]{0,8}", 0.0f64..600.0), 0..6),
    ) {
        let mut queued = Queued::default();
        let mut joined: Option<String> = None;
        let mut at = None;
        for (text, time) in &requests {
            queued.push(text, *time);
            if !text.trim().is_empty() {
                joined = Some(join_queued(joined.as_deref(), text));
                at = Some(*time);
            }
        }
        prop_assert_eq!(queued.text(), joined.as_deref());
        prop_assert_eq!(queued.take(), joined.zip(at));
        prop_assert_eq!(queued.text(), None);
    }
}

#[test]
fn a_failed_or_waiting_preview_does_not_read_as_previewing() {
    for status in ["running", "completed", "ready", "live"] {
        assert!(previewing(status), "{status}");
    }
    for status in ["failed", "required", "unavailable", "stopped", "exited", ""] {
        assert!(!previewing(status), "{status}");
    }
}

// ------------------------------------------------------------ proposal cards

fn clip_with_effects(id: &str, effects: Value) -> Value {
    json!({"id": id, "name": id, "timelineStart": 0.0, "duration": 4.0,
           "startFrame": 0, "durationFrames": 96, "enabled": true, "effects": effects})
}

fn timeline(clips: &[Value]) -> Timeline {
    Timeline::from_value(&json!({"name": "Cut", "fps": 24.0, "duration": 8.0,
        "tracks": [{"id": "V1", "type": "video", "name": "V1", "enabled": true, "clips": clips}]}))
}

#[test]
fn a_colour_change_on_one_clip_gets_a_line_of_its_own() {
    let program = timeline(&[clip_with_effects(
        "B003_C011",
        json!([{"fx": "color.hue_vs_hue", "params": {"hue": 0.0, "sat": 0.0, "mix": 1.0}}]),
    )]);
    let proposed = timeline(&[clip_with_effects(
        "B003_C011",
        json!([{"fx": "color.hue_vs_hue",
                "params": {"hue": -9.0, "sat": -6.0, "mix": 1.0, "curve": [[0, 1]]}}]),
    )]);
    let changes = clip_changes(&program, &proposed);
    assert_eq!(
        colour_change(&program, &proposed, &changes),
        Some(("B003_C011 · Hue vs hue".into(), "Hue −9, Sat −6".into()))
    );
    // A colour effect added says all it sets.
    let bare = timeline(&[clip_with_effects("A", json!([]))]);
    let graded = timeline(&[clip_with_effects(
        "A",
        json!([{"fx": "color.white_balance", "params": {"temperature": 0.125}}]),
    )]);
    assert_eq!(
        colour_change(&bare, &graded, &clip_changes(&bare, &graded)),
        Some(("A · White balance".into(), "Temperature 0.125".into()))
    );
}

#[test]
fn only_a_lone_colour_change_is_said_so() {
    let program = timeline(&[
        clip_with_effects("A", json!([])),
        clip_with_effects("B", json!([])),
    ]);
    let blurred = timeline(&[
        clip_with_effects(
            "A",
            json!([{"fx": "blur.gaussian", "params": {"radius": 4}}]),
        ),
        clip_with_effects("B", json!([])),
    ]);
    assert_eq!(
        colour_change(&program, &blurred, &clip_changes(&program, &blurred)),
        None
    );
    let both = timeline(&[
        clip_with_effects("A", json!([{"fx": "color.lift", "params": {"r": 0.1}}])),
        clip_with_effects("B", json!([{"fx": "color.lift", "params": {"r": 0.1}}])),
    ]);
    assert_eq!(
        colour_change(&program, &both, &clip_changes(&program, &both)),
        None
    );
    assert_eq!(effect_label("color.white_balance"), "White balance");
    assert_eq!(effect_label("sat"), "Sat");
}

#[test]
fn a_checks_failings_become_requests_to_fix_them() {
    let event = event(
        json!({"type": "check", "status": "done", "summary": {"rows": [
            {"name": "True peak", "passed": false, "at": 4.5},
            {"name": "Loudness", "passed": true},
            {"name": "", "passed": false},
            {"name": "Black frames", "passed": false},
        ]}}),
    );
    let Event::Check(Check::Done { summary, .. }) = &event else {
        panic!("{event:?} is not a finished check");
    };
    assert_eq!(
        check_suggestions(summary, 24.0),
        vec![
            (
                "Fix the true peak".to_owned(),
                "Fix the true peak at 00:00:04:12".to_owned()
            ),
            (
                "Fix the black frames".to_owned(),
                "Fix the black frames".to_owned()
            ),
        ]
    );
    assert!(check_suggestions(&CheckSummary::default(), 24.0).is_empty());
}

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
    // Never less than a second: no turn reads as taking no time.
    assert_eq!(worked_headline(None, 0), "Worked for 1s");
}

#[test]
fn ask_again_sends_the_request_the_answer_was_to() {
    let turn = |kind: Speaker, text: &str| Turn {
        kind: kind.wire().into(),
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

#[test]
fn a_proposal_says_which_clips_it_adds_removes_moves_and_trims() {
    let program = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 4.0, 4.0),
        clip("c", 8.0, 4.0),
        clip("d", 12.0, 4.0),
    ]));
    let proposed = Timeline::from_value(&timeline_value(&[
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

#[test]
fn proposals_keep_their_versions_and_the_nearest_open_one_is_pinned() {
    let value = snapshot();
    let book = ProposalBook::read(&Snapshot::from_value(&value));
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

proptest! {
    /// A settled card's age says the same until the moment it is said to
    /// change, and something else from then.
    #[test]
    fn a_settled_age_changes_when_it_is_said_to(
        seconds in prop_oneof![0u64..3600, 3600u64..86_400, 86_400u64..10_000_000],
        nanos in 0u32..1_000_000_000,
    ) {
        let ago = Duration::new(seconds, nanos);
        let until = settled_age_changes(ago);
        prop_assert!(until > Duration::ZERO);
        prop_assert_eq!(settled_age(ago + until - Duration::from_nanos(1)), settled_age(ago));
        prop_assert_ne!(settled_age(ago + until), settled_age(ago));
    }
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
        author: serde_json::from_value(author).unwrap_or_default(),
        ..Proposal::default()
    };
    // The author as kerf-mcp stages it (kerf-runtime's `Author::new(AuthorKind::Mcp, …)`).
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
    use kerf_editor::changes::{reel_plan, reel_total};
    let program = Timeline::from_value(&timeline_value(&[
        clip("a", 0.0, 4.0),
        clip("b", 4.0, 4.0),
        clip("c", 8.0, 4.0),
        clip("d", 20.0, 4.0),
    ]));
    let proposed = Timeline::from_value(&timeline_value(&[
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

fn kerf(id: &str, proposal: Option<&str>) -> Turn {
    Turn {
        id: id.into(),
        kind: Speaker::Kerf.wire().into(),
        text: "Done.".into(),
        proposal: proposal.map(str::to_owned),
        ..Turn::default()
    }
}

fn you(id: &str) -> Turn {
    Turn {
        id: id.into(),
        kind: Speaker::You.wire().into(),
        text: "Tighten it".into(),
        ..Turn::default()
    }
}

#[test]
fn a_proposal_has_one_card_in_the_newest_turn_that_names_it() {
    let value = json!({
        "proposals": [
            {"proposalId": "p-2", "status": "pending", "freshness": "current", "version": 2,
             "script": "trim title out -14f", "supersedes": "p-1",
             "versions": [{"version": 1, "proposalId": "p-1", "script": "trim title out -12f"},
                          {"version": 2, "proposalId": "p-2", "script": "trim title out -14f"}]},
            {"proposalId": "q-1", "status": "accepted", "freshness": "current", "version": 1,
             "script": "move b to 2s"}
        ]
    });
    let book = ProposalBook::read(&Snapshot::from_value(&value));
    let proposals = Snapshot::from_value(&value).proposals;
    let turns = vec![
        you("1"),
        kerf("2", Some("p-1")),
        you("3"),
        kerf("4", Some("q-1")),
        you("5"),
        kerf("6", Some("p-2")),
        kerf("7", Some("gone")),
        Turn {
            proposal: Some("q-1".into()),
            ..you("8")
        },
    ];
    let holders = card_turns(&turns, &proposals, &book);
    // v1's turn and v2's are one proposal: the newer turn holds its card.
    assert_eq!(holders.get("p-2"), Some(&5));
    assert_eq!(holders.get("q-1"), Some(&3));
    // A proposal the snapshot no longer has holds nothing; yours never do.
    assert_eq!(holders.len(), 2);
    // Named once, its one turn holds it.
    let one = card_turns(&turns[..2], &proposals, &book);
    assert_eq!(one.get("p-2"), Some(&1));
}

fn sequence(fps: f64, start: f64) -> Timeline {
    Timeline {
        fps,
        start: Some(start),
        ..Timeline::default()
    }
}

const NTSC: f64 = 30000.0 / 1001.0;

/// Kerf's own way of writing a frame (kerf-model's `format_timecode`).
fn kerf_timecode(frames: i64, base: i64, drop: bool) -> String {
    let mut count = frames;
    if drop {
        let dropped = base / 15;
        let per_ten_minutes = base * 600 - dropped * 9;
        let per_minute = base * 60 - dropped;
        let tens = count / per_ten_minutes;
        let rest = count % per_ten_minutes;
        count += dropped * 9 * tens;
        if rest > dropped {
            count += dropped * ((rest - dropped) / per_minute);
        }
    }
    let ff = count % base;
    let seconds = count / base;
    let separator = if drop { ';' } else { ':' };
    format!(
        "{:02}:{:02}:{:02}{separator}{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        ff
    )
}

proptest! {
    /// Every frame Kerf writes, from any start, drop-frame or not, lands on
    /// that frame of the program.
    #[test]
    fn what_kerf_writes_lands_on_its_frame(
        frame in 0i64..200_000,
        // From zero, an hour at 24, an hour drop-frame at 29.97, and non-drop.
        start in prop::sample::select(vec![0i64, 86_400, 107_892, 108_000]),
        (fps, base, drop) in prop::sample::select(vec![
            (24.0, 24, false), (25.0, 25, false), (24000.0 / 1001.0, 24, false),
            (NTSC, 30, false), (NTSC, 30, true), (60000.0 / 1001.0, 60, true), (60.0, 60, false),
        ]),
    ) {
        let timeline = sequence(fps, start as f64 / fps);
        let written = kerf_timecode(start + frame, base, drop);
        let at = timecode_position(&written, &timeline).expect("Kerf's timecode reads");
        prop_assert!((at * fps - frame as f64).abs() < 1e-6, "{written} landed at {at}");
    }
}

#[test]
fn the_composer_says_connect_while_kerfd_is_not_connected() {
    let refused = refusal(true, true, false, None).expect("refused");
    assert_eq!(refused.placeholder, "Connect to ask Kerf");
    assert_eq!(refused.reason, "Connect to start editing");
    // Connected, with Kerf set up: nothing in the way.
    assert!(refusal(true, false, false, None).is_none());
}

// ------------------------------------------------------------ the mode said

#[test]
fn each_mode_chosen_takes_the_place_of_the_mode_said_last() {
    let mut state = EditorState::default();
    state.notify("Exported cut.mov", false);
    for permissions in [
        Permissions::AcceptEdits,
        Permissions::Auto,
        Permissions::Ask,
        Permissions::AcceptEdits,
    ] {
        say_permissions(&mut state, permissions);
    }
    assert_eq!(
        texts(&state),
        [
            "Exported cut.mov",
            "Accept edits · Edits apply as Kerf makes them"
        ]
    );
    let said = state.line_shown(StatusLine::Permissions).unwrap();
    assert_eq!(said.hold, Some(MODE_HOLD));
    assert_eq!(MODE_HOLD, Duration::from_millis(3000));
    // What the preview says is a line of its own: neither takes the other's place.
    state.apply(&preview("required"));
    say_permissions(&mut state, Permissions::Auto);
    assert_eq!(
        texts(&state),
        [
            "Exported cut.mov",
            "Rendered preview required",
            "Auto · Edits apply, and Kerf makes its shots"
        ]
    );
    assert_eq!(
        state.preview_line().map(|line| line.text.as_str()),
        Some("Rendered preview required")
    );
    // It keeps its own 3 s past the window's hold; gone, the next is said anew.
    state.expire_notices(Duration::ZERO, Duration::ZERO);
    assert_eq!(
        texts(&state),
        ["Auto · Edits apply, and Kerf makes its shots"]
    );
    state.notices.clear();
    assert!(state.line_shown(StatusLine::Permissions).is_none());
    say_permissions(&mut state, Permissions::Ask);
    assert_eq!(
        texts(&state),
        ["Ask before edits · Each edit waits for you"]
    );
}

// ------------------------------------------------------------ words on a rest

fn proposal(freshness: &str) -> Proposal {
    Proposal {
        proposal_id: "p1".into(),
        status: ProposalStatus::Pending,
        freshness: if freshness == "stale" {
            ProposalFreshness::Stale
        } else {
            ProposalFreshness::Current
        },
        ..Proposal::default()
    }
}

#[test]
fn a_stale_proposal_can_be_brought_forward_and_accept_says_how() {
    let states = |proposal: &Proposal| {
        kerf_bridge::commands_states(&json!({
            "connected": true,
            "proposal": proposal_context(proposal),
        }))
    };
    let stale = states(&proposal("stale"));
    assert_eq!(unavailable(&stale, "proposal.forward"), None);
    let accept = unavailable(&stale, "proposal.accept").unwrap();
    assert!(accept.contains("bring it forward"), "{accept}");
    assert_eq!(unavailable(&stale, "proposal.reject"), None);

    let fresh = states(&proposal("fresh"));
    assert_eq!(unavailable(&fresh, "proposal.accept"), None);
    assert_eq!(
        unavailable(&fresh, "proposal.forward"),
        Some("This proposal is already on the edit as it is")
    );
    // Bring Proposal Forward resolves to its request for the stale one.
    let forward = kerf_bridge::commands_run(&json!({
        "command": "proposal.forward",
        "context": {"connected": true, "proposal": proposal_context(&proposal("stale"))},
        "params": {},
    }));
    assert!(forward["request"].is_object(), "{forward}");
}
