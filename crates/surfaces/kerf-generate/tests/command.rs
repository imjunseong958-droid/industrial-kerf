//! A model that is a program on the person's computer, run as a making runs it.
#![cfg(unix)]

use kerf_engine::error::EngineError;
use kerf_engine::process::sidecar::CancellationToken;
use kerf_engine::recipe::{make, plan};
use kerf_generate::Generator;

#[test]
fn a_cancelled_making_lets_its_program_take_back_what_it_asked_for() {
    let folder = tempfile::tempdir().unwrap();
    let directory = folder.path();
    std::fs::create_dir_all(directory.join("shots")).unwrap();
    // A program that waits on a server, and when asked to end says so and leaves a mark, as a
    // program that takes its job back does.
    let mark = directory.join("taken-back");
    let script = format!(
        "trap 'echo took the job back >&2; : > \"{}\"; exit 1' TERM; echo waiting >&2; while :; do sleep 0.05; done",
        mark.display()
    );
    let file = directory.join("generation.json");
    std::fs::write(
        &file,
        serde_json::json!({"providers": {"slow": {"run": ["sh", "-c", script, "slow", "{request}"]}}})
            .to_string(),
    )
    .unwrap();
    let generator = Generator::read(&file).unwrap();
    let project = kerf_lang::parse_document(
        "kerf 1\nproject name=\"shots\"\nmedia m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x120\n  generate model=slow prompt=\"a boat leaves\"\n",
        kerf_model::Catalog::builtin(),
    )
    .unwrap()
    .project;
    let planned = plan(&project, directory, "m1", &generator).unwrap();
    let token = CancellationToken::new();
    let (cancel, said) = (token.clone(), std::sync::Mutex::new(Vec::new()));
    let started = std::time::Instant::now();
    let cancelled = make(&project, directory, &planned, &generator, &token, |line| {
        // Cancelled once the program is known to be waiting.
        if line == "waiting" {
            cancel.cancel();
        }
        said.lock().unwrap().push(line.to_owned());
    })
    .unwrap_err();
    assert!(matches!(cancelled, EngineError::Cancelled), "{cancelled}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    // It was asked, not killed: it did what it does on the way out, and was heard.
    assert!(mark.exists());
    assert!(
        said.lock()
            .unwrap()
            .iter()
            .any(|line| line == "took the job back"),
        "{said:?}"
    );
    // No take, and nothing of one, is left.
    assert!(!directory.join("shots/open.takes/001.work").exists());
    assert!(!directory.join("shots/open.takes/.making-001.mp4").exists());
}
