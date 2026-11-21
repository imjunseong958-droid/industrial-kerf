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
