//! The conversation's own logic, through the public `chat` module: what
//! Kerf wrote read into bold, code and timecode links; a request queued
//! behind a turn; what a finished turn folds to; and proposals read from a
//! snapshot, the clips each changes, and which one is pinned.

use kerf_editor::chat::{
    clip_changes, join_queued, model_choices, model_label, permission_words, pin_order,
    request_before, script_summary, signed_frames, spans, timecode_seconds, worked_headline,
    ProposalBook, Span,
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
    assert_eq!(spans("2 ** 3 and a ` tick"), vec![plain("2 ** 3 and a ` tick")]);
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
    assert_eq!(worked_headline(Some(72.4), 6), "Worked for 1m 12s · 6 steps");
    assert_eq!(worked_headline(Some(41.0), 1), "Worked for 41s · 1 step");
    assert_eq!(worked_headline(Some(3.0), 0), "Worked for 3s");
    assert_eq!(worked_headline(None, 0), "Worked for 0s");
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
