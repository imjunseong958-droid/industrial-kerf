//! Where the eye is either side of each cut of a sequence's picture, as `look` with `eye`
//! says it.
//!
//! [`kerf_engine::eye`] measures it; this is how it is told: counts of every cut, and each
//! cut with what its two guides say and, where the shots are made, what their takes do, or
//! with why it was not read. Nothing is called wrong: a hard cut jumps on purpose.

use kerf_engine::eye::{Cut, Eye, Seen};
use kerf_lang::Clock;
use serde_json::{json, Value};

/// A share of the frame or a distance in frame heights, to a thousandth.
fn place(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0 + 0.0
}

/// How large one thing is beside another, to a hundredth.
fn times(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// One reading of a cut.
fn seen(seen: &Seen) -> Value {
    json!({
        "out": seen.out.map(place),
        "in": seen.into.map(place),
        "jump": place(seen.jump),
        "across": place(seen.across),
        "down": place(seen.down),
        "size": times(seen.size),
    })
}

/// A cut, as its entry.
fn entry(cut: &Cut, clock: &Clock) -> Value {
    let mut entry = json!({
        "at": clock.print_position(cut.at),
        "from": cut.from,
        "to": cut.to,
    });
    if let Some(transition) = &cut.transition {
        entry["transition"] = json!(transition);
    }
    if !cut.under.is_empty() {
        entry["under"] = json!(cut.under);
    }
    match &cut.read {
        Err(why) => entry["notMeasured"] = json!(why),
        Ok(read) => {
            entry["on"] = json!(read.on);
            let mut guide = seen(&read.guide);
            if let Some(going) = read.going {
                guide["going"] = json!(going.map(|going| going.name()));
                if read.reversed() {
                    guide["reversed"] = json!(true);
                }
            }
            entry["guide"] = guide;
            match &read.take {
                None => {}
                Some(Ok(take)) => entry["take"] = seen(take),
                Some(Err(why)) => entry["take"] = json!({"notMeasured": why}),
            }
        }
    }
    entry
}

/// What `look` answers under `eye`, positions printed by `clock`, the sequence's.
pub(crate) fn said(eye: &Eye, clock: &Clock) -> Value {
    let read = || eye.cuts.iter().filter_map(|cut| cut.read.as_ref().ok());
    let held = read().filter(|read| read.guide.held()).count();
    let mut summary = json!({
        "cuts": eye.cuts.len(),
        "read": read().count(),
        "held": held,
        "jumps": read().count() - held,
        "reversed": read().filter(|read| read.reversed()).count(),
        "takes": read().filter(|read| matches!(read.take, Some(Ok(_)))).count(),
        "notMeasured": eye.cuts.len() - read().count(),
    });
    if eye.unread > 0 {
        summary["unread"] = json!(eye.unread);
    }
    json!({
        "summary": summary,
        "cuts": eye.cuts.iter().map(|cut| entry(cut, clock)).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_engine::eye::{Going, Read};
    use kerf_model::Rate;

    fn seen_at(out: [f64; 4], into: [f64; 4], jump: f64, across: f64, size: f64) -> Seen {
        Seen {
            out,
            into,
            jump,
            across,
            down: 0.0104,
            size,
        }
    }

    #[test]
    fn a_cut_is_said_with_both_readings_or_with_why_it_has_none() {
        let sequence = kerf_lang::parse_document(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=320x180 audio=48000\n",
            kerf_model::Catalog::builtin(),
        )
        .unwrap()
        .project
        .sequences
        .remove(0);
        let clock = Clock::of_sequence(&sequence);
        let cut = |frame: i64, from: &str, to: &str, read| Cut {
            at: Rate::FPS_24.time_of(frame),
            from: from.to_owned(),
            to: to.to_owned(),
            transition: None,
            under: Vec::new(),
            read,
        };
        // A subject that crosses the frame at the cut and turns back, in a take that has
        // it nearer where it was; one that holds, under a title and a dissolve, with a take
        // that cannot be read; one whose shots are not made; and footage with no guide.
        let jumps = Read {
            on: ["ball".to_owned(), "car".to_owned()],
            guide: seen_at([0.55, 0.40, 0.75, 0.70], [0.20, 0.38, 0.42, 0.72], 0.6044, -0.6043, 1.1149),
            going: Some([Going::Right, Going::Left]),
            take: Some(Ok(seen_at([0.5, 0.4, 0.7, 0.7], [0.6, 0.4, 0.8, 0.7], 0.1781, 0.1778, 1.0))),
        };
        let mut holds = cut(
            192,
            "c2",
            "c3",
            Ok(Read {
                on: ["car".to_owned(), "car2".to_owned()],
                guide: seen_at([0.4, 0.4, 0.6, 0.6], [0.41, 0.4, 0.61, 0.6], 0.0178, 0.0178, 1.0),
                going: Some([Going::Still, Going::Right]),
                take: Some(Err("`m3`: the picture could not be read".to_owned())),
            }),
        );
        holds.transition = Some("dissolve".to_owned());
        holds.under = vec!["t1".to_owned()];
        let unmade = Read {
            on: ["car2".to_owned(), "boat".to_owned()],
            guide: seen_at([0.4, 0.4, 0.6, 0.6], [0.4, 0.1, 0.6, 0.3], 0.3, 0.0, 1.0),
            going: None,
            take: None,
        };
        let said = said(
            &Eye {
                cuts: vec![
                    cut(96, "c1", "c2", Ok(jumps)),
                    holds,
                    cut(288, "c3", "c4", Ok(unmade)),
                    cut(384, "c4", "c5", Err("`c5` plays footage with no guide".to_owned())),
                ],
                unread: 2,
            },
            &clock,
        );
        assert_eq!(
            said,
            json!({
                "summary": {"cuts": 4, "read": 3, "held": 1, "jumps": 2, "reversed": 1, "takes": 1, "notMeasured": 1, "unread": 2},
                "cuts": [
                    {"at": "00:00:04:00", "from": "c1", "to": "c2", "on": ["ball", "car"],
                     "guide": {"out": [0.55, 0.4, 0.75, 0.7], "in": [0.2, 0.38, 0.42, 0.72], "jump": 0.604, "across": -0.604, "down": 0.01, "size": 1.11, "going": ["right", "left"], "reversed": true},
                     "take": {"out": [0.5, 0.4, 0.7, 0.7], "in": [0.6, 0.4, 0.8, 0.7], "jump": 0.178, "across": 0.178, "down": 0.01, "size": 1.0}},
                    {"at": "00:00:08:00", "from": "c2", "to": "c3", "transition": "dissolve", "under": ["t1"], "on": ["car", "car2"],
                     "guide": {"out": [0.4, 0.4, 0.6, 0.6], "in": [0.41, 0.4, 0.61, 0.6], "jump": 0.018, "across": 0.018, "down": 0.01, "size": 1.0, "going": ["still", "right"]},
                     "take": {"notMeasured": "`m3`: the picture could not be read"}},
                    {"at": "00:00:12:00", "from": "c3", "to": "c4", "on": ["car2", "boat"],
                     "guide": {"out": [0.4, 0.4, 0.6, 0.6], "in": [0.4, 0.1, 0.6, 0.3], "jump": 0.3, "across": 0.0, "down": 0.01, "size": 1.0}},
                    {"at": "00:00:16:00", "from": "c4", "to": "c5", "notMeasured": "`c5` plays footage with no guide"},
                ],
            })
        );
    }
}
