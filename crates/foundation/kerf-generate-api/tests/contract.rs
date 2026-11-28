//! The API as a server is sent and answers it, written once from these types to a file the
//! generation server's own tests read (`services/kerf-gen/tests/contract.json`): one side
//! changed alone fails one test or the other. After a change made on purpose:
//! `KERF_BLESS=1 cargo test -p kerf-generate-api --test contract`, and review the diff.

use kerf_generate_api::{
    ApiError, Created, GenerateRequest, GenerationState, ModelList, ModelSpec, Moves, NamedModel,
    Region, Sha256, Signal,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn sha(digit: char) -> Sha256 {
    Sha256::parse(&digit.to_string().repeat(64)).unwrap()
}

/// One of everything a server is sent and answers.
fn samples() -> Value {
    let request = GenerateRequest {
        version: kerf_generate_api::REQUEST_VERSION,
        model: "fake".into(),
        prompt: "a red car on a wet street".into(),
        guide: Some(sha('a')),
        guides: vec![("depth".into(), sha('b'))],
        first: Some(sha('c')),
        last: None,
        references: vec![sha('d')],
        seconds: Some(4.0),
        size: Some([832, 480]),
        seed: Some(7),
        signals: vec![Signal {
            name: "swell".into(),
            rate: 16.0,
            values: vec![0.0, 0.5, 1.0],
        }],
        moves: vec![Moves {
            name: "car".into(),
            rate: 16.0,
            frame: (832, 480),
            places: vec![Some([0.5, 0.5, 0.25, 0.125]), None],
        }],
        regions: vec![Region {
            prompt: "a red car".into(),
            mask: sha('e'),
        }],
        more: BTreeMap::from([("steps".into(), "30".into())]),
        out: "out.mp4".into(),
        from: Some(4.0),
    };
    let spec = ModelSpec::parse(&json!({
        "about": "a shot that follows its guide",
        "size": [832, 480],
        "rate": 16,
        "seconds": 5,
        "reads": ["guide", "first", "ref", "regions"],
        "more": {"steps": 30}
    }))
    .unwrap();
    let states = [
        GenerationState::Queued,
        GenerationState::Running {
            progress: Some(0.5),
            line: Some("step 15 of 30".into()),
        },
        GenerationState::Done { out: sha('f') },
        GenerationState::Failed {
            message: "it broke".into(),
        },
        GenerationState::Cancelled,
    ];
    json!({
        "request": request,
        "created": Created { id: "0123abcd".into() },
        "states": states,
        "models": ModelList { models: vec![NamedModel { name: "fake".into(), spec }] },
        "error": ApiError { error: "no such request".into() },
    })
}

#[test]
fn what_a_server_is_sent_and_answers_is_the_file_its_tests_read() {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../services/kerf-gen/tests/contract.json");
    let written = serde_json::to_string_pretty(&samples()).unwrap() + "\n";
    if std::env::var_os("KERF_BLESS").is_some() {
        std::fs::write(&file, &written).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(&file).expect("contract.json (bless with KERF_BLESS=1)");
    assert_eq!(
        kept, written,
        "the API changed: bless it with KERF_BLESS=1 and change the server to it"
    );
}
