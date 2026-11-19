//! What `look` with `sync` says of each recording it lines up: the one offset that places it,
//! how that offset goes over the recording, and, where no one offset places it, what does not
//! fit.

use kerf_engine::sync::{Alignment, Holds};
use serde_json::{json, Value};

/// A recording as `look` with `sync` answers of it.
pub(crate) struct Lined {
    /// How far into the first it begins, in seconds: the offset nearest every stretch of it
    /// that was placed, or the whole recording's where its stretches say nothing or step.
    pub offset: f64,
    /// Why the script leaves it out, and none where one offset places it.
    pub left: Option<String>,
    /// Its row, but for which media it is and where the script puts it.
    pub row: Value,
}

/// Seconds to the millisecond, as a row has them and as a media's `sync=` is written.
pub(crate) fn milliseconds(seconds: f64) -> f64 {
    let rounded = (seconds * 1000.0).round() / 1000.0;
    // No `-0`.
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

/// Each recording of `found`, the first being the one the others are held to. `frame` is how
/// long a frame of the sequence they go in lasts: a recording is placed where one offset is
/// within half of it wherever the recording was read, which is as near as a whole frame gets.
pub(crate) fn lined_up(found: &[Alignment], frame: f64) -> Vec<Lined> {
    found
        .iter()
        .enumerate()
        .map(|(index, alignment)| {
            if index == 0 {
                return Lined {
                    offset: 0.0,
                    left: None,
                    row: json!({"offset": 0.0, "confidence": 1.0}),
                };
            }
            lined(alignment, frame)
        })
        .collect()
}

fn lined(alignment: &Alignment, frame: f64) -> Lined {
    let course = &alignment.course;
    let holds = course.holds();
    let fit = course.fit();
    let offset = fit.map_or(alignment.offset, |(offset, _)| offset);
    let mut row = json!({
        "offset": milliseconds(offset),
        "confidence": (alignment.confidence * 100.0).round() / 100.0,
        "course": match holds {
            Holds::Unknown => "unknown",
            Holds::Steady => "steady",
            Holds::Drift => "drift",
            Holds::Jump => "jump",
        },
        "read": course.placed().count(),
        "of": course.stretches.len(),
    });
    if let Some((_, spread)) = fit {
        row["within"] = json!(milliseconds(spread));
    }
    // What a drift comes to, from the first stretch placed to the last.
    let drift = course.ends().filter(|_| course.drifts()).map(|(first, last)| {
        let a_minute = course.drift * 60_000.0;
        let minutes = (last.middle() - first.middle()) / 60.0;
        (a_minute, minutes, first.offset, last.offset)
    });
    if let Some((a_minute, minutes, from, to)) = drift {
        row["drift"] = json!({
            "msPerMinute": (a_minute * 10.0).round() / 10.0,
            "minutes": (minutes * 10.0).round() / 10.0,
        });
        // With a step between them the two ends differ by the step as well.
        if let (Holds::Drift, Some(from), Some(to)) = (holds, from, to) {
            row["drift"]["from"] = json!(milliseconds(from));
            row["drift"]["to"] = json!(milliseconds(to));
        }
    }
    if !course.jumps.is_empty() {
        row["jumps"] = course
            .jumps
            .iter()
            .map(|jump| {
                json!({
                    "after": tenths(jump.after),
                    "before": tenths(jump.before),
                    "from": milliseconds(jump.from),
                    "to": milliseconds(jump.to),
                })
            })
            .collect();
    }
    // The stretches nothing was said of, those next to each other as one.
    let mut unknown: Vec<[f64; 2]> = Vec::new();
    for stretch in course.stretches.iter().filter(|s| s.offset.is_none()) {
        match unknown.last_mut() {
            Some(last) if last[1] == stretch.from => last[1] = stretch.to,
            _ => unknown.push([stretch.from, stretch.to]),
        }
    }
    if !unknown.is_empty() {
        row["unknown"] = unknown
            .iter()
            .map(|[from, to]| json!([tenths(*from), tenths(*to)]))
            .collect();
    }
    let left = match (holds, fit, drift) {
        (Holds::Jump, ..) => {
            let steps: Vec<String> = course
                .jumps
                .iter()
                .map(|jump| {
                    format!(
                        "by {} s between {} s and {} s of it",
                        milliseconds(jump.to - jump.from),
                        tenths(jump.after),
                        tenths(jump.before)
                    )
                })
                .collect();
            Some(format!(
                "its offset steps {}: one of the two stopped and went on, or lost sound, there, \
                 and no one offset is right on both sides. Split it there and place each part by \
                 its own offset, the `from` and `to` of `jumps`",
                steps.join(", and ")
            ))
        }
        (Holds::Drift, Some((_, spread)), Some((a_minute, minutes, ..))) if spread > frame / 2.0 => {
            Some(format!(
                "its offset drifts {:.1} ms a minute, one clock running faster than the other: \
                 over the {:.1} minutes read the nearest one offset is {:.0} ms out at each end, \
                 more than the half frame ({:.0} ms) a clip is placed to. Conform one recording \
                 to the other's speed first, or place it in parts short enough to hold",
                a_minute.abs(),
                minutes,
                spread * 1000.0,
                frame * 500.0,
            ))
        }
        _ => None,
    };
    if let Some(why) = &left {
        row["notPlaced"] = json!(why);
    }
    Lined { offset, left, row }
}

/// Seconds to a tenth, as where a step or a quiet stretch is.
fn tenths(seconds: f64) -> f64 {
    (seconds * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_engine::sync::{Course, Jump, Stretch};

    /// A recording read a minute at a time, each stretch with the offset given for it.
    fn read(offsets: &[Option<f64>], drift: f64, jumps: Vec<Jump>) -> Alignment {
        let stretches = offsets
            .iter()
            .enumerate()
            .map(|(index, offset)| Stretch {
                from: index as f64 * 60.0,
                to: (index + 1) as f64 * 60.0,
                offset: *offset,
            })
            .collect();
        Alignment {
            offset: offsets.iter().flatten().next().copied().unwrap_or(0.0),
            confidence: 0.9,
            course: Course {
                stretches,
                drift,
                jumps,
            },
        }
    }

    /// Half a frame at 24 a second is 20.8 ms.
    const FRAME: f64 = 1.0 / 24.0;

    fn row(alignment: &Alignment) -> Lined {
        let first = read(&[], 0.0, Vec::new());
        lined_up(&[first, alignment.clone()], FRAME).remove(1)
    }

    #[test]
    fn a_recording_one_offset_fits_is_placed_by_it() {
        let found = row(&read(
            &[Some(1.2341), None, None, Some(1.2339), Some(1.2344)],
            0.0,
            Vec::new(),
        ));
        assert_eq!(found.left, None);
        assert!((found.offset - 1.23415).abs() < 1e-9);
        assert_eq!(
            found.row,
            json!({"offset": 1.234, "confidence": 0.9, "course": "steady", "read": 3, "of": 5,
                "within": 0.0, "unknown": [[60.0, 180.0]]})
        );
        // The first is what the others are held to, and nothing is read of it.
        let first = &lined_up(&[read(&[], 0.0, Vec::new())], FRAME)[0];
        assert_eq!(first.row, json!({"offset": 0.0, "confidence": 1.0}));
    }

    #[test]
    fn a_drift_is_placed_while_one_offset_stays_within_half_a_frame_of_it() {
        // 6 ms a minute over four minutes: 24 ms from the first minute to the fifth, and the
        // offset halfway is 12 ms from each.
        let drifting = |a_minute: f64, minutes: usize| {
            let offsets: Vec<Option<f64>> = (0..minutes)
                .map(|minute| Some(2.0 + a_minute * minute as f64))
                .collect();
            read(&offsets, a_minute / 60.0, Vec::new())
        };
        let found = row(&drifting(0.006, 5));
        assert_eq!(found.left, None);
        assert!((found.offset - 2.012).abs() < 1e-9);
        assert_eq!(found.row["course"], "drift");
        assert_eq!(found.row["within"], 0.012);
        assert_eq!(
            found.row["drift"],
            json!({"msPerMinute": 6.0, "minutes": 4.0, "from": 2.0, "to": 2.024})
        );
        // Over half an hour the same drift is 87 ms from end to middle, and no offset places it.
        let found = row(&drifting(0.006, 30));
        assert_eq!(found.row["course"], "drift");
        assert_eq!(
            found.left.as_deref(),
            Some(
                "its offset drifts 6.0 ms a minute, one clock running faster than the other: \
                 over the 29.0 minutes read the nearest one offset is 87 ms out at each end, more \
                 than the half frame (21 ms) a clip is placed to. Conform one recording to the \
                 other's speed first, or place it in parts short enough to hold"
            )
        );
        assert_eq!(found.row["notPlaced"].as_str(), found.left.as_deref());
    }

    #[test]
    fn a_recording_whose_offset_steps_is_not_placed_and_says_where() {
        let jump = Jump {
            after: 135.04,
            before: 139.0,
            from: 5.0,
            to: 5.5,
        };
        let found = row(&read(
            &[Some(5.0), Some(5.0), Some(5.5), Some(5.5)],
            0.0,
            vec![jump],
        ));
        assert_eq!(found.row["course"], "jump");
        assert_eq!(
            found.row["jumps"],
            json!([{"after": 135.0, "before": 139.0, "from": 5.0, "to": 5.5}])
        );
        assert!(found.row.get("within").is_none() && found.row.get("drift").is_none());
        assert_eq!(
            found.left.as_deref(),
            Some(
                "its offset steps by 0.5 s between 135 s and 139 s of it: one of the two stopped \
                 and went on, or lost sound, there, and no one offset is right on both sides. \
                 Split it there and place each part by its own offset, the `from` and `to` of \
                 `jumps`"
            )
        );
        // The whole recording's offset is what is left to say of it.
        assert_eq!(found.offset, 5.0);
    }

    #[test]
    fn a_recording_nothing_could_be_read_of_is_placed_by_the_whole_of_it_as_before() {
        let found = row(&Alignment {
            offset: 3.3,
            confidence: 0.2,
            course: Course {
                stretches: vec![Stretch {
                    from: 0.0,
                    to: 20.0,
                    offset: None,
                }],
                drift: 0.0,
                jumps: Vec::new(),
            },
        });
        assert_eq!(found.left, None);
        assert_eq!(
            found.row,
            json!({"offset": 3.3, "confidence": 0.2, "course": "unknown", "read": 0, "of": 1,
                "unknown": [[0.0, 20.0]]})
        );
    }
}
