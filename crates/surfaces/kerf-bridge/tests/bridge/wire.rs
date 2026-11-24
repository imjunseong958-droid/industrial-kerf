//! The bridge's commands and events as JSON: what the editor was sent reads as events that
//! write the very same text, a command or event written reads back as itself, and an event
//! this build does not know reads as one to ignore.

use kerf_bridge::event::workflow::{
    Blank, By, Described, DescribedInput, DescribedNode, OriginKind, Shown,
};
use kerf_bridge::event::{
    AgentState, AgentUpdate, CaptionsExport, Check, CheckSummary, ClipReading, CollabSync,
    ConnectionStatus, ConversationSummary, ErrorCode, ErrorEvent, Export, ExportStatus,
    LoudnessEvent, MeterReading, Preview, PreviewStatus, QueuedRender, RenderCodec, RenderExport,
    RenderQuality, RenderStatus, Snapshot, TakeRead, TimelineView, ToolState, ViewParamDef,
};
use kerf_bridge::{Command, EditOp, Event, ShotOf, TrimSide};
use proptest::collection::{btree_map, vec};
use proptest::option;
use proptest::prelude::*;
use proptest::sample::select;
use serde_json::json;

/// One event of each kind (and of each status the session gave) the editor was sent in a
/// recorded session, as it got them, with the recording machine's paths and names made
/// neutral. The last three are of a shot that session had none of: its `recipe`, the
/// `workflow` of one of its models with what a server takes, and a take held to its guide
/// (`take_read`), as the bridge wrote them when their payloads were JSON it passed on
/// unread, so that the types they now are write the same text. The `state`'s timeline
/// carries the catalog's definitions as kerfd writes them (`properties`, `maskProperties`,
/// `generators`), which were JSON passed on unread too and are `ViewParamDef`s now. The
/// last is a `state` with a proposal open, its timeline and the proposal's (which the
/// bridge makes itself) as the bridge wrote them while those definitions were JSON.
const SAMPLES: &str = include_str!("wire_samples.jsonl");

#[test]
fn what_the_editor_was_sent_reads_as_events_that_write_the_same_text() {
    for line in SAMPLES.lines() {
        let event: Event =
            serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}"));
        assert_ne!(event, Event::Unknown, "{line}");
        let written = serde_json::to_value(&event).unwrap();
        assert_eq!(serde_json::to_string(&written).unwrap(), line);
    }
}

#[test]
fn an_event_this_build_does_not_know_reads_as_one_to_ignore() {
    for newer in [
        json!({"type": "from_a_newer_kerf"}),
        json!({"type": "from_a_newer_kerf", "status": "ready", "items": [1, 2]}),
    ] {
        assert_eq!(
            serde_json::from_value::<Event>(newer.clone()).unwrap(),
            Event::Unknown,
            "{newer}"
        );
    }
}

#[test]
fn a_definition_this_build_cannot_read_is_left_out_and_the_timeline_still_reads() {
    let number =
        json!({"name": "x", "type": "number", "default": 0.0, "animatable": true, "doc": ""});
    // Choices with labels, as a newer kerfd might write them.
    let newer = json!({"name": "mode", "type": "enum", "default": "a", "animatable": false,
        "doc": "", "options": [{"value": "a", "label": "A"}]});
    let event = json!({"type": "state", "snapshot": {"timeline": {
        "tracks": [{"id": "V1", "type": "video"}],
        "properties": {"video": [number, newer]},
        "maskProperties": [newer, number],
        "generators": {"title": [newer], "solid": [number], "light": {"x": number}},
    }}});
    let Event::State { snapshot } = serde_json::from_value(event).unwrap() else {
        panic!("not a state");
    };
    let timeline = snapshot.timeline;
    let names = |defs: &[ViewParamDef]| defs.iter().map(|def| def.name.clone()).collect::<Vec<_>>();
    assert_eq!(timeline.tracks.len(), 1);
    assert_eq!(names(&timeline.properties["video"]), ["x"]);
    assert_eq!(names(&timeline.mask_properties), ["x"]);
    assert_eq!(names(&timeline.generators["solid"]), ["x"]);
    assert!(timeline.generators["title"].is_empty());
    assert!(timeline.generators["light"].is_empty());
}

proptest! {
    #[test]
    fn an_event_written_reads_back_as_itself_and_writes_the_same_json(event in event()) {
        let written = serde_json::to_string(&event).unwrap();
        let read: Event = serde_json::from_str(&written).unwrap();
        prop_assert_eq!(&read, &event);
        prop_assert_eq!(serde_json::to_string(&read).unwrap(), written);
    }

    #[test]
    fn a_command_written_reads_back_as_itself_and_writes_the_same_json(command in command()) {
        let written = serde_json::to_string(&command).unwrap();
        let read: Command = serde_json::from_str(&written).unwrap();
        let value: serde_json::Value = serde_json::from_str(&written).unwrap();
        prop_assert_eq!(value["type"].as_str(), Some(command.kind()));
        prop_assert_eq!(&read, &command);
        prop_assert_eq!(serde_json::to_string(&read).unwrap(), written);
    }
}

/// Seconds, levels and progress: whole ones, which stay floats, and any other finite one.
fn number() -> impl Strategy<Value = f64> {
    prop_oneof![
        (-100_000_i32..100_000).prop_map(f64::from),
        proptest::num::f64::NORMAL | proptest::num::f64::ZERO,
    ]
}

fn text() -> impl Strategy<Value = String> {
    any::<String>()
}

fn some_text() -> impl Strategy<Value = Option<String>> {
    option::of(text())
}

fn event() -> impl Strategy<Value = Event> {
    prop_oneof![
        (number(), number(), any::<u64>()).prop_map(|(position, duration, dropped)| {
            Event::Clock {
                position,
                duration,
                dropped,
            }
        }),
        any::<bool>().prop_map(|playing| Event::Playback { playing }),
        text().prop_map(|text| Event::Caption { text }),
        Just(Event::ViewerReset),
        meter(),
        error(),
        (
            select(vec![
                ConnectionStatus::Connecting,
                ConnectionStatus::Ready,
                ConnectionStatus::Disconnected,
            ]),
            some_text(),
        )
            .prop_map(|(status, message)| Event::Connection { status, message }),
        preview(),
        export(),
        check(),
        vec(conversation(), 0..4).prop_map(|items| Event::Conversations { items }),
        agent(),
        loudness(),
        vec(queued_render(), 0..4).prop_map(|jobs| Event::RenderQueue { jobs }),
        (text(), any::<u32>(), text()).prop_map(|(media, take, line)| Event::MediaMaking {
            media,
            take,
            line,
        }),
        collab_sync(),
        take_read(),
        workflow(),
        state(),
    ]
}

/// A parameter's definition: a type this build may not know, a default of each kind a
/// parameter value is, and each limit, the unit, the choices and the extensions there or
/// left out (a list of none being there).
fn param_def() -> impl Strategy<Value = ViewParamDef> {
    let default = prop_oneof![
        number().prop_map(serde_json::Value::from),
        any::<bool>().prop_map(serde_json::Value::from),
        text().prop_map(serde_json::Value::from),
    ];
    let names = || option::of(vec(text(), 0..3));
    (
        (text(), text(), default, any::<bool>(), text()),
        (
            option::of(number()),
            option::of(number()),
            some_text(),
            names(),
            names(),
        ),
    )
        .prop_map(
            |((name, kind, default, animatable, doc), (min, max, unit, options, extensions))| {
                ViewParamDef {
                    name,
                    kind,
                    default,
                    animatable,
                    doc,
                    min,
                    max,
                    unit,
                    options,
                    extensions,
                }
            },
        )
}

/// A snapshot whose timeline carries the catalog's definitions: the clip properties and the
/// generators' parameters by name, and the mask properties.
fn state() -> impl Strategy<Value = Event> {
    let defs = || vec(param_def(), 0..3);
    (
        btree_map(text(), defs(), 0..3),
        defs(),
        btree_map(text(), defs(), 0..3),
    )
        .prop_map(|(properties, mask_properties, generators)| Event::State {
            snapshot: Box::new(Snapshot {
                timeline: TimelineView {
                    properties,
                    mask_properties,
                    generators,
                    ..TimelineView::default()
                },
                ..Snapshot::default()
            }),
        })
}

/// One input of a workflow's node: a link, with what it takes where a server has said, or
/// a value (a text, a whole number, a yes or no, nothing) with the blanks it holds, and
/// what a server takes there.
fn workflow_input() -> impl Strategy<Value = DescribedInput> {
    let value = prop_oneof![
        text().prop_map(serde_json::Value::from),
        any::<i64>().prop_map(serde_json::Value::from),
        any::<bool>().prop_map(serde_json::Value::from),
        Just(serde_json::Value::Null),
    ];
    prop_oneof![
        (text(), text(), any::<u64>(), some_text()).prop_map(|(name, from, output, takes)| {
            DescribedInput {
                name,
                link: Some((from, output)),
                takes,
                ..DescribedInput::default()
            }
        }),
        (
            text(),
            value,
            vec(text(), 0..3),
            option::of(vec(text().prop_map(serde_json::Value::from), 0..3)),
            option::of(any::<u32>()),
            option::of(any::<i32>()),
        )
            .prop_map(
                |(name, value, blanks, choices, most, least)| DescribedInput {
                    name,
                    value,
                    blanks,
                    choices,
                    max: most.map(serde_json::Number::from),
                    min: least.map(serde_json::Number::from),
                    ..DescribedInput::default()
                }
            ),
    ]
}

fn workflow() -> impl Strategy<Value = Event> {
    let node = (
        text(),
        text(),
        vec(workflow_input(), 0..3),
        some_text(),
        option::of(vec(text().prop_map(serde_json::Value::from), 0..3)),
    )
        .prop_map(|(id, class, inputs, title, outputs)| DescribedNode {
            id,
            class,
            inputs,
            title,
            outputs,
        });
    let blank = (
        select(vec![
            By::Line,
            By::Kerf,
            By::Moves,
            By::Set,
            By::Signal,
            By::File,
        ]),
        text(),
        option::of(text().prop_map(serde_json::Value::from)),
        some_text(),
    )
        .prop_map(|(by, says, default, given)| Blank {
            by,
            says,
            default,
            given,
        });
    (
        (text(), any::<usize>(), text(), text(), text()),
        select(vec![OriginKind::Bundled, OriginKind::File, OriginKind::Own]),
        some_text(),
        vec(node, 0..3),
        btree_map(text(), blank, 0..3),
        (any::<bool>(), some_text(), any::<bool>()),
    )
        .prop_map(
            |((media, line, model, name, text), origin, file, nodes, blanks, said)| {
                Event::Workflow(Box::new(Shown {
                    described: Described {
                        nodes,
                        blanks,
                        ..Described::default()
                    },
                    media,
                    line,
                    model,
                    name,
                    text,
                    origin,
                    file,
                    taken: said.0,
                    taken_error: said.1,
                    edited: said.2,
                }))
            },
        )
}

/// One subject of a take's reading: followed, with where and on how many frames, or not,
/// with why.
fn clip_reading() -> impl Strategy<Value = ClipReading> {
    prop_oneof![
        (
            text(),
            number(),
            number(),
            number(),
            any::<usize>(),
            any::<usize>()
        )
            .prop_map(|(clip, overlap, off, size, frames, shown)| ClipReading {
                clip,
                overlap,
                off: Some(off),
                size: Some(size),
                frames: Some(frames),
                shown: Some(shown),
                note: None,
            }),
        (text(), text()).prop_map(|(clip, note)| ClipReading {
            clip,
            note: Some(note),
            ..ClipReading::default()
        }),
    ]
}

fn take_read() -> impl Strategy<Value = Event> {
    (
        any::<u32>(),
        text(),
        any::<bool>(),
        number(),
        vec(clip_reading(), 0..3),
        text(),
        any::<u64>(),
    )
        .prop_map(|(take, path, chosen, overlap, clips, media, revision)| {
            Event::TakeRead(TakeRead {
                take,
                path,
                chosen,
                overlap,
                clips,
                media,
                revision,
            })
        })
}

fn meter() -> impl Strategy<Value = Event> {
    (
        option::of([number(), number()]),
        option::of([number(), number()]),
        option::of(number()),
        option::of(number()),
        option::of(number()),
    )
        .prop_map(|(peak, rms, momentary, short_term, at)| {
            Event::Meter(MeterReading {
                peak,
                rms,
                momentary,
                short_term,
                at,
            })
        })
}

/// A refusal's code: one of kerfd's own, or any other name (a newer kerfd's).
fn code() -> impl Strategy<Value = ErrorCode> {
    prop_oneof![
        select(vec![
            "REVISION_CONFLICT",
            "CANCELLED",
            "INVALID_RENDER_OUTPUT"
        ])
        .prop_map(ErrorCode::from_name),
        text().prop_map(|name| ErrorCode::from_name(&name)),
    ]
}

fn error() -> impl Strategy<Value = Event> {
    (
        text(),
        option::of(code()),
        some_text(),
        some_text(),
        some_text(),
        some_text(),
        option::of(select(vec![ShotOf::Moves, ShotOf::Matte])),
        some_text(),
    )
        .prop_map(
            |(message, code, command, job_id, media, clip, of, format)| {
                Event::Error(ErrorEvent {
                    message,
                    code,
                    command,
                    job_id,
                    media,
                    clip,
                    of,
                    format,
                })
            },
        )
}

fn preview() -> impl Strategy<Value = Event> {
    (
        select(vec![
            PreviewStatus::Ready,
            PreviewStatus::Live,
            PreviewStatus::Required,
            PreviewStatus::Unavailable,
            PreviewStatus::Running,
            PreviewStatus::Completed,
            PreviewStatus::Failed,
            PreviewStatus::Stopped,
            PreviewStatus::Exited,
        ]),
        some_text(),
        some_text(),
        option::of(number()),
        some_text(),
    )
        .prop_map(|(status, proposal_id, path, progress, message)| {
            Event::Preview(Preview {
                status,
                proposal_id,
                path,
                progress,
                message,
            })
        })
}

fn export_status() -> impl Strategy<Value = ExportStatus> {
    select(vec![
        ExportStatus::Queued,
        ExportStatus::Running,
        ExportStatus::Completed,
        ExportStatus::Failed,
        ExportStatus::Cancelled,
    ])
}

/// What a render is made for: as kerfd names it, or any other name (a newer kerfd's), or
/// none.
fn quality() -> impl Strategy<Value = Option<RenderQuality>> {
    option::of(prop_oneof![
        select(vec!["preview", "full", "review", "master"]).prop_map(RenderQuality::from_name),
        text().prop_map(|name| RenderQuality::from_name(&name)),
    ])
}

/// A render's codec: as kerfd names it, or any other name, or none.
fn codec() -> impl Strategy<Value = Option<RenderCodec>> {
    option::of(prop_oneof![
        select(vec!["h264", "hevc", "hevc_hdr", "prores_hq"]).prop_map(RenderCodec::from_name),
        text().prop_map(|name| RenderCodec::from_name(&name)),
    ])
}

fn export() -> impl Strategy<Value = Event> {
    let render = (
        export_status(),
        text(),
        text(),
        some_text(),
        number(),
        quality(),
        codec(),
        some_text(),
        some_text(),
    )
        .prop_map(
            |(status, job_id, name, path, progress, quality, codec, sequence, message)| {
                Export::Render(RenderExport {
                    status,
                    job_id,
                    name,
                    path,
                    progress,
                    quality,
                    codec,
                    sequence,
                    message,
                })
            },
        );
    let captions = (
        export_status(),
        text(),
        text(),
        some_text(),
        option::of(number()),
        some_text(),
    )
        .prop_map(|(status, kind, format, path, progress, message)| {
            Export::Captions(CaptionsExport {
                status,
                kind,
                format,
                path,
                progress,
                message,
            })
        });
    prop_oneof![render, captions].prop_map(Event::Export)
}

fn check() -> impl Strategy<Value = Event> {
    prop_oneof![
        (number(), some_text())
            .prop_map(|(progress, sequence)| Check::Running { progress, sequence }),
        (any::<bool>(), some_text()).prop_map(|(valid, sequence)| Check::Done {
            valid,
            findings: Vec::new(),
            summary: CheckSummary::default(),
            sequence,
        }),
        (text(), some_text()).prop_map(|(message, sequence)| Check::Failed { message, sequence }),
    ]
    .prop_map(Event::Check)
}

fn conversation() -> impl Strategy<Value = ConversationSummary> {
    (
        text(),
        text(),
        text(),
        text(),
        any::<bool>(),
        any::<usize>(),
    )
        .prop_map(
            |(id, title, created, updated, archived, count)| ConversationSummary {
                id,
                title,
                created,
                updated,
                archived,
                count,
            },
        )
}

fn agent() -> impl Strategy<Value = Event> {
    let tool_state = select(vec![ToolState::Running, ToolState::Done, ToolState::Failed]);
    let update = prop_oneof![
        text().prop_map(|text| AgentUpdate::Delta { text }),
        text().prop_map(|text| AgentUpdate::Status { text }),
        (text(), tool_state, text(), option::of(any::<bool>())).prop_map(
            |(id, state, text, rendered)| AgentUpdate::Tool {
                id,
                state,
                text,
                rendered,
            }
        ),
        text().prop_map(|text| AgentUpdate::Done { text }),
    ];
    let state = select(vec![
        AgentState::Running,
        AgentState::AwaitingReview,
        AgentState::Aborted,
        AgentState::Completed,
        AgentState::Failed,
    ]);
    prop_oneof![
        update.prop_map(Event::Agent),
        (state, some_text()).prop_map(|(state, message)| Event::AgentState { state, message }),
        any::<bool>().prop_map(|busy| Event::AgentBusy { busy }),
    ]
}

fn loudness() -> impl Strategy<Value = Event> {
    prop_oneof![
        (number(), any::<u64>(), some_text()).prop_map(|(progress, revision, sequence)| {
            LoudnessEvent::Running {
                progress,
                revision,
                sequence,
            }
        }),
        (text(), option::of(any::<u64>()), some_text()).prop_map(
            |(message, revision, sequence)| LoudnessEvent::Failed {
                message,
                revision,
                sequence,
            }
        ),
    ]
    .prop_map(Event::Loudness)
}

/// Where a render job stands: as kerfd names it, or any other name (a newer kerfd's).
fn render_status() -> impl Strategy<Value = RenderStatus> {
    prop_oneof![
        select(vec![
            "queued",
            "running",
            "completed",
            "failed",
            "cancelled"
        ])
        .prop_map(RenderStatus::from_name),
        text().prop_map(|name| RenderStatus::from_name(&name)),
    ]
}

fn queued_render() -> impl Strategy<Value = QueuedRender> {
    (
        text(),
        text(),
        quality(),
        codec(),
        some_text(),
        render_status(),
        number(),
        some_text(),
        some_text(),
    )
        .prop_map(
            |(job_id, name, quality, codec, sequence, status, progress, output_path, error)| {
                QueuedRender {
                    job_id,
                    name,
                    quality,
                    codec,
                    sequence,
                    status,
                    progress,
                    output_path,
                    error,
                }
            },
        )
}

fn collab_sync() -> impl Strategy<Value = Event> {
    prop_oneof![
        select(vec![
            CollabSync::Committing,
            CollabSync::Fetching,
            CollabSync::Merging,
            CollabSync::Pushing,
            CollabSync::Downloading,
        ]),
        text().prop_map(|message| CollabSync::Failed { message }),
    ]
    .prop_map(Event::CollabSync)
}

fn command() -> impl Strategy<Value = Command> {
    prop_oneof![
        (number(), any::<bool>()).prop_map(|(position, scrub)| Command::Seek { position, scrub }),
        Just(Command::Play),
        Just(Command::Pause),
        (proptest::num::f32::NORMAL | proptest::num::f32::ZERO)
            .prop_map(|value| Command::Volume { value }),
        vec(text(), 0..4).prop_map(|tracks| Command::Solo { tracks }),
        (vec(edit_op(), 0..4), any::<u64>())
            .prop_map(|(ops, revision)| Command::Edit { ops, revision }),
        any::<u64>().prop_map(|revision| Command::Undo { revision }),
        any::<u64>().prop_map(|revision| Command::Redo { revision }),
        (text(), any::<u64>()).prop_map(|(proposal_id, revision)| Command::Accept {
            proposal_id,
            revision,
        }),
        (text(), any::<u64>()).prop_map(|(proposal_id, revision)| Command::Reject {
            proposal_id,
            revision,
        }),
    ]
}

fn edit_op() -> impl Strategy<Value = EditOp> {
    prop_oneof![
        (
            text(),
            select(vec![TrimSide::In, TrimSide::Out]),
            any::<i64>(),
            any::<bool>(),
        )
            .prop_map(|(clip, edge, frames, ripple)| EditOp::Trim {
                clip,
                edge,
                frames,
                ripple,
            }),
        (text(), any::<i64>()).prop_map(|(clip, frames)| EditOp::Roll { clip, frames }),
        (text(), any::<i64>()).prop_map(|(clip, frames)| EditOp::Slip { clip, frames }),
        (text(), any::<i64>()).prop_map(|(clip, frames)| EditOp::Slide { clip, frames }),
        (text(), any::<i64>(), some_text()).prop_map(|(clip, frames, track)| EditOp::Move {
            clip,
            frames,
            track
        }),
        (text(), any::<i64>(), some_text()).prop_map(|(clip, at, tail)| EditOp::Split {
            clip,
            at,
            tail
        }),
        vec(text(), 0..4).prop_map(|clips| EditOp::Delete { clips }),
        vec(text(), 0..4).prop_map(|clips| EditOp::RippleDelete { clips }),
    ]
}
