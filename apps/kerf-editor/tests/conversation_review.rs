//! The conversation as reviewed: a field's own keys win over what the
//! window's keymap unbinds in fields, a proposal has one card however many
//! turns name it, and the timecodes Kerf writes (the sequence's own, from
//! its start, drop-frame where it is) land where they say.

use gpui::{Action, KeyBinding, KeyContext, Keymap, Keystroke, NoAction};
use kerf_editor::chat::{
    card_turns, timecode_position, CyclePermissions, ProposalBook, COMPOSER_CONTEXT,
};
use kerf_editor::commands::{catalog, keystroke, RunCommand, WORKSPACE};
use kerf_editor::input::{self, Backspace, Cancel, End, Home, Left, Right, Submit};
use kerf_editor::model::{Snapshot, Speaker, Timeline, Turn};
use proptest::prelude::*;
use serde_json::json;

/// The window's keymap as `commands::bind` makes it: the field's keys first,
/// then every registry shortcut, each plain one unbound in fields.
fn window_keymap(keymap: &str) -> Keymap {
    let mut bindings = input::key_bindings();
    for command in catalog(keymap) {
        for shortcut in &command.keys {
            let Some(keys) = keystroke(shortcut) else {
                continue;
            };
            bindings.push(KeyBinding::new(
                &keys,
                RunCommand {
                    id: command.id.clone().into(),
                },
                Some(WORKSPACE),
            ));
            let first = keys.split(' ').next().unwrap_or(&keys);
            let plain = !["secondary-", "ctrl-", "alt-", "super-", "cmd-"]
                .iter()
                .any(|modifier| first.contains(modifier));
            if plain {
                bindings.push(KeyBinding::new(&keys, NoAction, Some(input::CONTEXT)));
            }
        }
    }
    Keymap::new(bindings)
}

/// The actions `key` runs with the composer's box focused.
fn in_the_composer(keymap: &Keymap, key: &str) -> Vec<Box<dyn Action>> {
    let stack: Vec<KeyContext> = [WORKSPACE, COMPOSER_CONTEXT, input::CONTEXT, input::KEYS]
        .into_iter()
        .map(|context| KeyContext::parse(context).unwrap())
        .collect();
    let (bindings, _) = keymap.bindings_for_input(&[Keystroke::parse(key).unwrap()], &stack);
    bindings
        .iter()
        .map(|binding| binding.action().boxed_clone())
        .collect()
}

#[test]
fn a_fields_own_keys_come_before_what_the_keymap_unbinds_in_fields() {
    for name in ["kerf", "premiere", "fcp"] {
        let keymap = window_keymap(name);
        let first = |key: &str| {
            in_the_composer(&keymap, key)
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("{name}: {key} does nothing in a field"))
        };
        // Esc is Reject Proposal and Backspace Ripple Delete outside a field;
        // in one they do what they do there.
        assert!(first("escape").partial_eq(&Cancel), "{name}: escape");
        assert!(first("backspace").partial_eq(&Backspace), "{name}");
        assert!(first("left").partial_eq(&Left), "{name}");
        assert!(first("right").partial_eq(&Right), "{name}");
        assert!(first("home").partial_eq(&Home), "{name}");
        assert!(first("end").partial_eq(&End), "{name}");
        assert!(first("enter").partial_eq(&Submit), "{name}");
        assert!(first("shift-tab").partial_eq(&CyclePermissions), "{name}");
        // Typed, a command's plain key is a letter, never the command.
        for key in ["space", "m", "i", "o", "s"] {
            assert!(
                in_the_composer(&keymap, key)
                    .iter()
                    .all(|action| action.as_any().downcast_ref::<RunCommand>().is_none()),
                "{name}: {key} runs a command from a field"
            );
        }
    }
}

fn kerf(id: &str, proposal: Option<&str>) -> Turn {
    Turn {
        id: id.into(),
        kind: Speaker::Kerf,
        text: "Done.".into(),
        proposal: proposal.map(str::to_owned),
        ..Turn::default()
    }
}

fn you(id: &str) -> Turn {
    Turn {
        id: id.into(),
        kind: Speaker::You,
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
    let book = ProposalBook::read(&value);
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
        start,
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
