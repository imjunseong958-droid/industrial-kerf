//! What a workflow's model program says its inputs take ([`kerf_shots::workflow::taken`]),
//! asked of a stand-in for `kerf-comfy`: a shell script of that name. One leaves a process
//! outside its group with Perl, which macOS has where it has no `setsid`.
#![expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
#![cfg(unix)]

use kerf_engine::recipe::model::Models;
use kerf_shots::workflow;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// A shot whose one step asks the model `video`.
const PROJECT: &str = "kerf 1
project name=\"Court\"
media shot path=\"shots/bounce.mp4\" rate=16 dur=4s video=832x480
  generate model=video prompt=\"a basketball bounces on a court\" seed=7
";

/// Ask the stand-in `script`, a shell script that is `kerf-comfy` in `dir`, what the
/// inputs of the shot's workflow take.
fn taken(dir: &Path, script: &str) -> Result<Value, String> {
    use std::os::unix::fs::PermissionsExt;
    let program = dir.join("kerf-comfy");
    std::fs::write(&program, script).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let models = dir.join("models.json");
    std::fs::write(
        &models,
        json!({"video": {"run": [program, "--workflow", "shot", "{request}"]}}).to_string(),
    )
    .unwrap();
    let models = Models::read(&models).unwrap();
    let project = kerf_runtime::parse_document(PROJECT).unwrap();
    let recipe = kerf_shots::graph::written(&project, "shot").unwrap();
    let line = kerf_engine::recipe::text::step_lines(&recipe)[0];
    let read = workflow::read(&project, dir, "shot", line, &models).unwrap();
    workflow::taken(&models, &read).map_err(|error| error.to_string())
}

/// Whether process `pid` ends within `limit`; a zombie awaiting its new parent counts as
/// ended.
fn ends_within(pid: &str, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .expect("ps runs");
        let state = String::from_utf8_lossy(&output.stdout);
        if state.trim().is_empty() || state.trim_start().starts_with('Z') {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn an_answer_longer_than_a_pipe_holds_is_read_whole() {
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    // What a server with many weights says: far more than a pipe holds, on stdout and on
    // stderr alike, said before the program ends.
    let names: Vec<String> = (0..20_000)
        .map(|n| format!("weights-{n:05}.safetensors"))
        .collect();
    let answer = json!({"UNETLoader": {"unet_name": {"choices": names}}});
    std::fs::write(dir.join("answer.json"), answer.to_string()).unwrap();
    std::fs::write(dir.join("noise.txt"), "asking the server\n".repeat(10_000)).unwrap();
    let said = taken(
        dir,
        "#!/bin/sh\n\
         here=$(dirname \"$0\")\n\
         [ \"$1 $2 $3\" = '--workflow shot --inputs' ] || exit 2\n\
         cat \"$here/noise.txt\" >&2\n\
         cat \"$here/answer.json\"\n",
    );
    // Compared, not shown: the answer is too long to read in a failure.
    assert_eq!(
        said.map(|said| said == answer),
        Ok(true),
        "the answer read is not the one said"
    );
}

#[test]
fn an_answer_is_heard_though_what_the_program_left_outside_its_group_holds_its_pipes() {
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    // It leaves a process in a group of its own that holds its stdout and stderr open for
    // half a minute, says which once it has left, then answers and ends.
    let started = Instant::now();
    let said = taken(
        dir,
        r#"#!/bin/sh
here=$(dirname "$0")
perl -e 'setpgrp(0, 0); open(my $f, ">", "$ARGV[0].new") or die; print $f $$; close($f); rename("$ARGV[0].new", $ARGV[0]); exec("sleep", "30")' "$here/holder" &
until [ -s "$here/holder" ]; do sleep 0.05; done
echo '{"KSampler": {"seed": {"min": 0}}}'
"#,
    );
    let took = started.elapsed();
    let holder = std::fs::read_to_string(dir.join("holder")).unwrap_or_default();
    // Not left running past the test.
    let _ = Command::new("kill").args(["-KILL", holder.trim()]).status();
    assert_eq!(said, Ok(json!({"KSampler": {"seed": {"min": 0}}})));
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn a_program_that_does_not_answer_in_time_is_stopped_with_what_it_started() {
    let folder = tempfile::tempdir().unwrap();
    let dir = folder.path();
    // It starts a child, says which, and waits on it; asked to end, it says it was asked.
    let said = taken(
        dir,
        "#!/bin/sh\n\
         here=$(dirname \"$0\")\n\
         trap 'echo asked > \"$here/asked\"; exit 1' TERM\n\
         sleep 600 &\n\
         echo \"$$ $!\" > \"$here/started\"\n\
         wait\n",
    );
    let started = std::fs::read_to_string(dir.join("started")).unwrap();
    let (program, child) = started.trim().split_once(' ').unwrap();
    let ended = [program, child].map(|pid| ends_within(pid, Duration::from_secs(3)));
    if ended != [true, true] {
        // Not left running past a failed test.
        let _ = Command::new("kill")
            .args(["-KILL", program, child])
            .status();
    }
    assert_eq!(
        said,
        Err("kerf-comfy did not say what the server takes within 20 s".to_owned())
    );
    assert_eq!(ended, [true, true], "the program, then its child");
    assert!(
        dir.join("asked").is_file(),
        "the program was killed without being asked to end"
    );
}
