//! What Kerf writes of the API reads back as what it wrote.

use kerf_generate_api::{
    GenerateRequest, GenerationState, ModelSpec, Moves, NamedModel, Region, Sha256, Signal,
};
use proptest::prelude::*;
use serde_json::{Map, Value};

fn sha256() -> impl Strategy<Value = Sha256> {
    "[0-9a-f]{64}".prop_map(|text| Sha256::parse(&text).unwrap())
}

fn name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,8}"
}

fn number() -> impl Strategy<Value = f64> {
    (0u32..10_000).prop_map(|whole| f64::from(whole) / 16.0)
}

fn request() -> impl Strategy<Value = GenerateRequest<Sha256>> {
    (
        (name(), ".{0,40}", proptest::option::of(sha256())),
        proptest::collection::vec((name(), sha256()), 0..3),
        (
            proptest::option::of(sha256()),
            proptest::collection::vec(sha256(), 0..3),
            proptest::option::of(number()),
            proptest::option::of((1u32..4000, 1u32..4000)),
            proptest::option::of(any::<u64>()),
        ),
        (
            proptest::collection::vec(
                (name(), number(), proptest::collection::vec(number(), 0..4)),
                0..2,
            ),
            proptest::collection::vec((".{0,10}", sha256()), 0..2),
            proptest::collection::btree_map(name(), ".{0,10}", 0..3),
            proptest::option::of(number()),
        ),
    )
        .prop_map(
            |(
                (model, prompt, guide),
                guides,
                (first, references, seconds, size, seed),
                (signals, regions, more, from),
            )| {
                // A JSON object holds a guide's name once.
                let mut guides = guides;
                guides.sort_by(|a, b| a.0.cmp(&b.0));
                guides.dedup_by(|a, b| a.0 == b.0);
                GenerateRequest {
                    version: 1,
                    model,
                    prompt,
                    guide,
                    guides,
                    first,
                    last: None,
                    references,
                    seconds,
                    size: size.map(|(width, height)| [width, height]),
                    seed,
                    signals: signals
                        .into_iter()
                        .map(|(name, rate, values)| Signal { name, rate, values })
                        .collect(),
                    moves: vec![Moves {
                        name: "crate".into(),
                        rate: 16.0,
                        frame: (640, 360),
                        places: vec![Some([0.5, 0.25, 0.125, 0.0625]), None],
                    }],
                    regions: regions
                        .into_iter()
                        .map(|(prompt, mask)| Region { prompt, mask })
                        .collect(),
                    more,
                    out: "out.mp4".into(),
                    from,
                }
            },
        )
}

fn spec() -> impl Strategy<Value = ModelSpec> {
    (
        proptest::option::of(".{0,20}"),
        proptest::option::of((1u32..4000, 1u32..4000)),
        proptest::option::of(number().prop_filter("above 0", |n| *n > 0.0)),
        proptest::option::of(number().prop_filter("above 0", |n| *n > 0.0)),
        proptest::option::of(proptest::sample::subsequence(
            vec![
                "guide",
                "first",
                "last",
                "ref",
                "signal",
                "moves",
                "regions",
                "guide_depth",
            ],
            0..4,
        )),
        proptest::collection::btree_map(name(), number(), 0..3),
        proptest::collection::btree_map("x_[a-z]{1,6}", ".{0,6}", 0..2),
    )
        .prop_map(
            |(about, size, rate, seconds, reads, more, other)| ModelSpec {
                about,
                size,
                rate,
                seconds,
                reads: reads.map(|reads| reads.into_iter().map(str::to_owned).collect()),
                more: more
                    .into_iter()
                    .map(|(k, v)| (k, Value::from(v)))
                    .collect::<Map<_, _>>(),
                other: other
                    .into_iter()
                    .map(|(k, v)| (k, Value::from(v)))
                    .collect::<Map<_, _>>(),
            },
        )
}

fn state() -> impl Strategy<Value = GenerationState> {
    prop_oneof![
        Just(GenerationState::Queued),
        (
            proptest::option::of(0u32..=100),
            proptest::option::of(".{0,20}")
        )
            .prop_map(|(progress, line)| {
                GenerationState::Running {
                    progress: progress.map(|percent| f64::from(percent) / 100.0),
                    line,
                }
            }),
        sha256().prop_map(|out| GenerationState::Done { out }),
        ".{0,20}".prop_map(|message| GenerationState::Failed { message }),
        Just(GenerationState::Cancelled),
    ]
}

proptest! {
    #[test]
    fn a_request_reads_back_as_it_was_written(request in request()) {
        let written = request.file().unwrap();
        let read: GenerateRequest<Sha256> = serde_json::from_slice(&written).unwrap();
        prop_assert_eq!(&read, &request);
        // And with each file a path, as a program reads it.
        let paths = request.clone().try_map(|sha| Ok::<_, ()>(format!("/files/{sha}"))).unwrap();
        let read: GenerateRequest = serde_json::from_slice(&paths.file().unwrap()).unwrap();
        prop_assert_eq!(read.files().len(), request.files().len());
    }

    #[test]
    fn what_a_model_takes_reads_back_as_it_was_said(spec in spec(), name in name()) {
        let read = ModelSpec::parse(&serde_json::to_value(&spec).unwrap()).unwrap();
        prop_assert_eq!(&read, &spec);
        let named = NamedModel { name, spec };
        let read: NamedModel = serde_json::from_value(serde_json::to_value(&named).unwrap()).unwrap();
        prop_assert_eq!(read, named);
    }

    #[test]
    fn a_state_reads_back_as_it_was_said(state in state()) {
        let read: GenerationState = serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        prop_assert_eq!(read, state);
    }
}
