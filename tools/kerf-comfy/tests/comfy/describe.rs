//! `kerf-comfy --describe`, which reads the workflows that come with the program and asks
//! no server.

use std::process::{Command, Stdio};

/// `--describe` with no workflow named is a models file whole: an entry for each workflow
/// that comes with the program, under the name `--list` gives it, and no server is asked.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
fn describe_alone_is_an_entry_for_every_workflow_that_comes_with_it() {
    let run = |arguments: &[&str]| {
        let folder = tempfile::tempdir().unwrap();
        let ran = Command::new(env!("CARGO_BIN_EXE_kerf-comfy"))
            .args(arguments)
            .env_remove("COMFY_URL")
            .env_remove("COMFY_API_KEY")
            .env("KERF_COMFY", folder.path().join("no-such.json"))
            .stdin(Stdio::null())
            .output()
            .expect("kerf-comfy starts");
        assert!(
            ran.status.success(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&ran.stderr)
        );
        String::from_utf8(ran.stdout).unwrap()
    };
    let listed: Vec<String> = run(&["--list"])
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect();
    assert!(
        listed.iter().any(|name| name == "shot") && listed.len() > 5,
        "{listed:?}"
    );
    let models: serde_json::Value = serde_json::from_str(&run(&["--describe"])).expect("JSON");
    let models = models.as_object().expect("an entry a name");
    let mut named: Vec<&String> = models.keys().collect();
    let mut wanted: Vec<&String> = listed.iter().collect();
    named.sort();
    wanted.sort();
    assert_eq!(named, wanted);
    // Each is the entry the workflow's own `--describe` prints.
    for name in ["shot", "depth"] {
        let one: serde_json::Value =
            serde_json::from_str(&run(&["--workflow", name, "--describe"])).expect("JSON");
        assert_eq!(models[name], one, "{name}");
        assert_eq!(one["run"][2], name);
    }
}
