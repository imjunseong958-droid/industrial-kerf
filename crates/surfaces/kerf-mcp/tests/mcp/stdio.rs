//! kerf-mcp as an MCP host runs it: JSON-RPC on stdin and stdout.

use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// What kerf-mcp warns of goes to stderr, which the host keeps, at the level it logs at when
/// RUST_LOG says nothing; stdout carries MCP and nothing else. The warning here is of a clip's
/// media that cannot be indexed for the cuts asked about.
#[test]
fn a_warning_goes_to_stderr_and_stdout_carries_only_mcp() {
    let directory = tempfile::TempDir::new().unwrap();
    std::fs::write(directory.path().join("shot.mp4"), "not a video").unwrap();
    // The clip plays the first half of its media: a cut into its sound.
    std::fs::write(
        directory.path().join("project.kerf"),
        "kerf 1
project name=\"Stdio\"
media m1 path=\"shot.mp4\" rate=30 dur=00:00:04:00 video=320x180 audio=1ch@48000
sequence s1 rate=30 size=320x180
  track A1 audio
    clip a1 media=m1 at=00:00:00:00 dur=00:00:02:00
",
    )
    .unwrap();
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    #[expect(
        clippy::disallowed_methods,
        reason = "kerf-mcp under test, talked to over its pipes and waited on"
    )]
    let mut mcp = Command::new(env!("CARGO_BIN_EXE_kerf-mcp"))
        .arg("--dir")
        .arg(directory.path())
        .env("KERFD_BIN", &kerfd)
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = mcp.stdin.take().unwrap();
    for message in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"stdio-test","version":"0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"look","arguments":{"cuts":true}}}"#,
    ] {
        writeln!(input, "{message}").unwrap();
    }
    // The host is done: kerf-mcp answers what it was asked, and ends.
    drop(input);
    let output = mcp.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("WARN"), "{stderr}");
    assert!(stderr.contains("was not indexed for its cuts"), "{stderr}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let answers: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| panic!("not MCP: {line}")))
        .collect();
    let ids: Vec<&Value> = answers.iter().map(|answer| &answer["id"]).collect();
    assert_eq!(ids, [1, 2], "{stdout}");
    assert!(answers[1]["result"].is_object(), "{stdout}");
}
