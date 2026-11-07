//! Real-process proof for the project-scoped kerfd boundary: a Unix socket,
//! or a named pipe on Windows.

#![cfg(any(unix, windows))]

use kerf_runtime::{
    Author, CommentPatch, EndpointAddress, NewComment, RuntimeClient, RuntimeLease,
    StartRenderOptions, StateSnapshot, MAX_REQUEST_BYTES,
};
use std::fs;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn project_directory(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("kerfd-ipc-{label}-{stamp}"))
}

/// A project folder, removed when the test ends however it ends, which
/// also ends any daemon still serving it.
struct Project(PathBuf);

impl Project {
    fn new(label: &str) -> Self {
        let directory = project_directory(label);
        fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn start_daemon(directory: &Path) -> Child {
    daemon_command(directory).spawn().expect("spawn kerfd")
}

/// A daemon that gives up `seconds` after its last request when nothing
/// holds it.
fn start_daemon_idle_for(directory: &Path, seconds: u64) -> Child {
    daemon_command(directory)
        .env("KERFD_IDLE_TIMEOUT", seconds.to_string())
        .spawn()
        .expect("spawn kerfd")
}

fn daemon_command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kerfd"));
    command
        .arg("--dir")
        .arg(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("KERFD_TEST_DROP_RESPONSE_KEY", "lost-response");
    command
}

/// Wait for `daemon` to exit on its own; the deadline is far past any
/// exit that should happen.
fn wait_for_exit(daemon: &mut Child) -> std::process::ExitStatus {
    for _ in 0..1000 {
        if let Some(status) = daemon.try_wait().unwrap() {
            return status;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = daemon.kill();
    panic!("kerfd did not exit once nothing held it");
}

fn wait_until_held(lease: &RuntimeLease) {
    for _ in 0..1000 {
        if lease.is_held() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("the lease never attached");
}

fn wait_until_ready(client: &RuntimeClient) {
    for _ in 0..250 {
        if client.get_state().is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("kerfd did not become ready at {:?}", client.endpoint());
}

fn has_marker(state: &StateSnapshot, id: &str) -> bool {
    state.document.contains(&format!("marker {id} "))
}

/// A raw connection to the daemon, past `RuntimeClient`.
#[cfg(unix)]
fn raw_connection(endpoint: &EndpointAddress) -> std::io::Result<UnixStream> {
    match endpoint {
        EndpointAddress::Unix { path } => UnixStream::connect(path),
        EndpointAddress::WindowsNamedPipe { .. } => panic!("a named pipe on Unix"),
    }
}

/// A raw connection to the daemon, past `RuntimeClient`.
#[cfg(windows)]
fn raw_connection(endpoint: &EndpointAddress) -> std::io::Result<fs::File> {
    match endpoint {
        EndpointAddress::WindowsNamedPipe { name } => {
            fs::OpenOptions::new().read(true).write(true).open(name)
        }
        EndpointAddress::Unix { .. } => panic!("a Unix socket on Windows"),
    }
}

fn stop_daemon(mut daemon: Child) {
    let _ = daemon.kill();
    let _ = daemon.wait();
}

#[test]
fn two_clients_share_one_daemon_and_recover_after_lost_response_and_restart() {
    let directory = project_directory("shared");
    fs::create_dir_all(&directory).unwrap();
    let daemon = start_daemon(&directory);
    let left = RuntimeClient::new(&directory).unwrap();
    let right = RuntimeClient::new(&directory).unwrap();
    wait_until_ready(&left);
    assert_eq!(left.endpoint(), right.endpoint());

    // Both clients submit against the same revision at the same time.  The
    // mailbox admits exactly one and returns a structured revision conflict
    // to the other.
    let barrier = Arc::new(Barrier::new(3));
    let left_barrier = barrier.clone();
    let left_client = left.clone();
    let left_thread = thread::spawn(move || {
        left_barrier.wait();
        left_client.apply_script_request("ui", "simultaneous-left", "marker add at=0f as=left", 0)
    });
    let right_barrier = barrier.clone();
    let right_client = right.clone();
    let right_thread = thread::spawn(move || {
        right_barrier.wait();
        right_client.apply_script_request(
            "mcp",
            "simultaneous-right",
            "marker add at=0f as=right",
            0,
        )
    });
    barrier.wait();
    let left_result = left_thread.join().unwrap();
    let right_result = right_thread.join().unwrap();
    assert_eq!(left_result.is_ok() as u8 + right_result.is_ok() as u8, 1);
    let conflict = left_result
        .err()
        .or_else(|| right_result.err())
        .expect("one simultaneous request conflicts");
    assert_eq!(conflict.code, "REVISION_CONFLICT");

    let revision_one = left.get_state().unwrap();
    assert_eq!(revision_one.revision, 1);

    // The real daemon commits this edit script and deliberately drops its
    // first response. RuntimeClient must reconnect, read authoritative
    // state, and replay the same stable key inside this single method call.
    let lost_key = "lost-response";
    let replay = right
        .apply_script_request(
            "mcp",
            lost_key,
            "marker add at=12f name=\"lost\" as=lost",
            1,
        )
        .unwrap();
    assert_eq!(replay.revision, 2);
    assert!(has_marker(&replay, "lost"));

    // A client has no daemon ownership. Dropping one leaves the other fully
    // usable on the same process.
    drop(left);
    assert_eq!(right.get_state().unwrap().revision, 2);

    // Terminate and explicitly restart the real kerfd process. The second
    // client reconnects to the fresh endpoint and reads checkpoint state.
    stop_daemon(daemon);
    let restarted = start_daemon(&directory);
    wait_until_ready(&right);
    let recovered = right.get_state().unwrap();
    assert_eq!(recovered.revision, 2);
    assert!(has_marker(&recovered, "lost"));
    let (revision, document) = right.get_document(false).unwrap();
    assert_eq!(revision, 2);
    assert!(
        document.contains("marker lost at=00:00:00:12 name=\"lost\""),
        "{document}"
    );
    stop_daemon(restarted);
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn real_daemon_rejects_an_oversized_request_before_reducer_execution() {
    let directory = project_directory("size");
    fs::create_dir_all(&directory).unwrap();
    let daemon = start_daemon(&directory);
    let client = RuntimeClient::new(&directory).unwrap();
    wait_until_ready(&client);
    let mut socket = raw_connection(client.endpoint()).unwrap();
    let oversized = vec![b'x'; MAX_REQUEST_BYTES + 1];
    socket.write_all(&oversized).unwrap();
    #[cfg(unix)]
    let _ = socket.shutdown(std::net::Shutdown::Write);
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    let response: serde_json::Value = serde_json::from_str(response.trim()).unwrap();
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "REQUEST_TOO_LARGE");
    assert_eq!(client.get_state().unwrap().revision, 0);
    stop_daemon(daemon);
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn a_project_round_trips_through_kerfd_on_the_kernel() {
    let directory = project_directory("kernel");
    fs::create_dir_all(&directory).unwrap();
    let document = "\
kerf 1
project name=\"Round Trip\"

media src-1 path=\"/media/a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name=\"Round Trip\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00
  track v1 video
    clip clip-a media=src-1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00 link=L1
    clip clip-b media=src-1 src=00:00:10:00 at=00:00:04:00 dur=00:00:02:00 link=L2
  track A1 audio
    clip clip-a_a media=src-1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:00 link=L1
    clip clip-b_a media=src-1 src=00:00:10:00 at=00:00:04:00 dur=00:00:02:00 link=L2
";
    fs::write(directory.join("project.kerf"), document).unwrap();

    let daemon = start_daemon(&directory);
    let client = RuntimeClient::new(&directory).unwrap();
    wait_until_ready(&client);
    let opened = client.get_state().unwrap();
    assert_eq!(opened.revision, 0);
    assert_eq!(opened.source_version, 3);

    // A ripple trim keeps picture and sound in sync; the clip after moves.
    let trimmed = client
        .apply_script_request("ui", "trim-a", "trim clip-a out -12f ripple", 0)
        .unwrap();
    assert_eq!(trimmed.revision, 1);
    assert!(trimmed.can_undo);
    let (_, after_trim) = client.get_document(false).unwrap();
    assert!(after_trim.contains("clip clip-b media=src-1 src=00:00:10:00 at=00:00:03:12"));
    assert_eq!(
        fs::read_to_string(directory.join("project.kerf")).unwrap(),
        after_trim
    );

    // Deleting a clip retires its id and its sound's.
    let deleted = client
        .apply_script_request("ui", "delete-b", "delete clip-b", 1)
        .unwrap();
    assert_eq!(deleted.revision, 2);
    let undone = client.undo_request("ui", "undo-delete", 2).unwrap();
    assert_eq!(undone.revision, 3);
    assert!(undone.can_redo);
    let (_, after_undo) = client.get_document(false).unwrap();
    assert_eq!(after_undo, after_trim);

    stop_daemon(daemon);
    let restarted = start_daemon(&directory);
    wait_until_ready(&client);
    let recovered = client.get_state().unwrap();
    assert_eq!(recovered.revision, 3);
    assert!(recovered.can_undo && recovered.can_redo);
    let (_, recovered_document) = client.get_document(false).unwrap();
    assert_eq!(recovered_document, after_trim);
    assert_eq!(
        fs::read_to_string(directory.join("project.kerf")).unwrap(),
        after_trim
    );
    // History survived the restart: redo the delete, then undo the trim.
    let redone = client.redo_request("ui", "redo-delete", 3).unwrap();
    assert_eq!(redone.revision, 4);
    let (_, after_redo) = client.get_document(false).unwrap();
    assert!(!after_redo.contains("clip clip-b "), "{after_redo}");
    assert!(after_redo.contains("retired="), "{after_redo}");
    assert!(after_redo.contains("clip-b"), "{after_redo}");
    // A retired id cannot name a new clip.
    let reused = client
        .apply_script_request(
            "ui",
            "reuse-b",
            "insert media=src-1 src=0f dur=24f at=10s on=v1 as=clip-b",
            4,
        )
        .unwrap_err();
    assert_eq!(reused.code, "EDIT_REJECTED");
    assert!(
        reused.message.contains("never reused"),
        "{}",
        reused.message
    );
    stop_daemon(restarted);
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn comments_are_added_listed_and_set_over_the_socket_without_touching_the_project() {
    let project = Project::new("comments");
    let directory = project.0.clone();
    let daemon = start_daemon(&directory);
    let client = RuntimeClient::new(&directory).unwrap();
    wait_until_ready(&client);
    let opened = client.get_state().unwrap();
    assert_eq!(client.comments().unwrap().items, Vec::new());

    let mut note = NewComment::new(
        Author {
            id: Some("mina@example.com".into()),
            ..Author::new("person", "Mina Park")
        },
        "The flare hides the logo",
    );
    note.id = Some("c-socket-1".into());
    note.at = Some(2.0);
    note.tags = vec!["picture".into()];
    let (added, log) = client.add_comment("ui", note.clone()).unwrap();
    assert_eq!(added.id, "c-socket-1");
    assert_eq!(added.frame, Some(60));
    assert_eq!(log.items, vec![added.clone()]);
    // A retry with the same id adds nothing.
    let (again, log) = client.add_comment("ui", note).unwrap();
    assert_eq!(again, added);
    assert_eq!(log.items.len(), 1);

    // An agent's reply arrives from another client, as an MCP note does.
    let agent = RuntimeClient::new(&directory).unwrap();
    let mut reply = NewComment::new(
        Author {
            role: Some("MCP".into()),
            ..Author::new("mcp", "Claude Code")
        },
        "Masked the flare",
    );
    reply.parent = Some(added.id.clone());
    let (reply, _) = agent.add_comment("mcp", reply).unwrap();

    let (resolved, log) = client
        .set_comment(
            "ui",
            &added.id,
            CommentPatch {
                resolved: Some(true),
                ..CommentPatch::default()
            },
        )
        .unwrap();
    let resolved = resolved.unwrap();
    assert!(resolved.resolved);
    assert_eq!(log.items.len(), 2);
    let state = client.get_state().unwrap();
    assert_eq!(state.revision, opened.revision);
    assert_eq!(state.document, opened.document);
    assert!(state.review_revision > opened.review_revision);
    assert_eq!(state.review_revision, log.review_revision);

    let (deleted, log) = client
        .set_comment(
            "ui",
            &reply.id,
            CommentPatch {
                deleted: Some(true),
                ..CommentPatch::default()
            },
        )
        .unwrap();
    assert!(deleted.is_none());
    assert_eq!(log.items, vec![resolved.clone()]);
    let missing = client
        .set_comment("ui", "c-missing", CommentPatch::default())
        .unwrap_err();
    assert_eq!(missing.code, "COMMENT_NOT_FOUND");

    // The log outlives the daemon.
    stop_daemon(daemon);
    let restarted = start_daemon(&directory);
    wait_until_ready(&client);
    assert_eq!(client.comments().unwrap().items, vec![resolved]);
    stop_daemon(restarted);
}

/// `seconds` of 64x36 picture at 24 fps with a tone.
fn media_file(path: &Path, seconds: u32) {
    let output = Command::new(kerf_engine::ffmpeg::resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!("color=c=gray:s=64x36:r=24:d={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=f=440:sample_rate=48000:d={seconds}"))
        .args([
            "-c:a",
            "aac",
            "-shortest",
            "-c:v",
            "mpeg4",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn media_is_replaced_through_the_client_relative_to_the_project() {
    let directory = project_directory("media");
    fs::create_dir_all(directory.join("media")).unwrap();
    media_file(&directory.join("media/a.mp4"), 2);
    media_file(&directory.join("media/b.mp4"), 3);
    media_file(&directory.join("media/short.mp4"), 1);
    let daemon = start_daemon(&directory);
    let client = RuntimeClient::new(&directory)
        .unwrap()
        .with_request_timeout(Duration::from_secs(60));
    assert_eq!(client.request_timeout(), Duration::from_secs(60));
    wait_until_ready(&client);

    // kerfd runs elsewhere; relative paths name files in the project.
    let (imported, media, _) = client
        .import_media_request("ui", "import", "media/a.mp4", 0)
        .unwrap();
    assert!(
        imported.document.contains("path=\"media/a.mp4\""),
        "{}",
        imported.document
    );
    // The first answer is lost; the client retries through the receipt.
    let replaced = client
        .replace_media_request("ui", "lost-response", &media, "media/b.mp4", 1)
        .unwrap();
    assert_eq!(replaced.revision, 2);
    assert!(
        replaced.document.contains(&format!(
            "media {media} path=\"media/b.mp4\" rate=24 dur=00:00:03:00"
        )),
        "{}",
        replaced.document
    );
    let refused = client
        .replace_media_request("ui", "short", &media, "media/short.mp4", 2)
        .unwrap_err();
    assert_eq!(refused.code, "MEDIA_DOES_NOT_FIT", "{}", refused.message);
    assert_eq!(
        fs::read_to_string(directory.join("project.kerf")).unwrap(),
        replaced.document
    );
    let undone = client.undo_request("ui", "undo-replace", 2).unwrap();
    assert_eq!(undone.document, imported.document);
    stop_daemon(daemon);
    let _ = fs::remove_dir_all(&directory);
}

/// What ffprobe finds in `path`: each stream's codec, profile, and sample
/// format.
fn streams(path: &Path) -> Vec<serde_json::Value> {
    let output = Command::new(kerf_engine::ffmpeg::resolve_ffprobe_bin())
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,profile,sample_fmt,bits_per_raw_sample",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let probe: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    probe["streams"].as_array().unwrap().clone()
}

#[test]
fn a_prores_master_renders_through_kerfd_into_the_folder_chosen() {
    if !["prores_ks", "prores_videotoolbox"]
        .into_iter()
        .any(kerf_engine::render::encoder_available)
    {
        eprintln!("skipped: this FFmpeg has no ProRes encoder");
        return;
    }
    let project = Project::new("master");
    let directory = project.0.clone();
    fs::create_dir_all(directory.join("media")).unwrap();
    media_file(&directory.join("media/a.mp4"), 1);
    let daemon = start_daemon(&directory);
    let client = RuntimeClient::new(&directory)
        .unwrap()
        .with_request_timeout(Duration::from_secs(60));
    wait_until_ready(&client);
    client
        .import_media_request("ui", "import", "media/a.mp4", 0)
        .unwrap();

    let options = StartRenderOptions {
        codec: Some("prores_hq".into()),
        output: Some("deliveries/v1/".into()),
        name: Some("Master".into()),
        ..StartRenderOptions::new("master")
    };
    let (job, document, output) = client.start_render_with(&options).unwrap();
    assert!(
        output.ends_with("deliveries/v1/Master.mov"),
        "{}",
        output.display()
    );
    assert_eq!(
        (
            job.quality.as_deref(),
            job.codec.as_deref(),
            job.name.as_deref()
        ),
        (Some("master"), Some("prores_hq"), Some("Master"))
    );
    let queued = client.list_renders().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].status, "queued");

    kerf_runtime::run_render_job(&client, &job, &document, &output, false, |_| {}).unwrap();
    let streams = streams(&output);
    let stream = |kind: &str| {
        streams
            .iter()
            .find(|stream| stream["codec_type"] == kind)
            .unwrap_or_else(|| panic!("no {kind} stream in {streams:?}"))
    };
    assert_eq!(stream("video")["codec_name"], "prores");
    assert_eq!(stream("video")["profile"], "HQ");
    assert_eq!(stream("audio")["codec_name"], "pcm_s24le");
    let done = client.list_renders().unwrap();
    assert_eq!(done[0].status, "completed");
    assert_eq!(done[0].progress, 1.0);

    // An output that climbs out of the project is refused over the socket,
    // and nothing is queued.
    let refused = client
        .start_render_with(&StartRenderOptions {
            output: Some("../elsewhere/".into()),
            ..StartRenderOptions::new("master")
        })
        .unwrap_err();
    assert_eq!(refused.code, "INVALID_RENDER_OUTPUT", "{}", refused.message);
    assert_eq!(client.list_renders().unwrap().len(), 1);
    stop_daemon(daemon);
}

#[test]
fn daemon_exits_once_its_project_directory_is_deleted() {
    let directory = project_directory("orphan");
    fs::create_dir_all(&directory).unwrap();
    let mut daemon = start_daemon(&directory);
    wait_until_ready(&RuntimeClient::new(&directory).unwrap());

    fs::remove_dir_all(&directory).unwrap();
    for _ in 0..250 {
        if let Some(status) = daemon.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    stop_daemon(daemon);
    panic!("kerfd kept running after its project directory was deleted");
}

#[cfg(unix)]
#[test]
fn endpoint_flag_prints_the_socket_even_when_a_long_path_moves_it() {
    let base = project_directory("endpoint");
    let long = base.join("a".repeat(120));
    for directory in [&base, &long] {
        let output = Command::new(env!("CARGO_BIN_EXE_kerfd"))
            .arg("--dir")
            .arg(directory)
            .arg("--endpoint")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let printed = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        let client = RuntimeClient::new(directory).unwrap();
        assert_eq!(Some(printed.as_path()), client.endpoint().as_path());
        if directory == &long {
            assert!(!printed.starts_with(client.directory()), "{printed:?}");
        }
        let daemon = start_daemon(directory);
        wait_until_ready(&client);
        assert!(UnixStream::connect(&printed).is_ok());
        stop_daemon(daemon);
    }
    fs::remove_dir_all(&base).unwrap();
}

#[cfg(windows)]
#[test]
fn endpoint_flag_prints_the_pipe_the_daemon_listens_on() {
    let base = project_directory("endpoint");
    let long = base.join("a".repeat(120));
    for directory in [&base, &long] {
        let output = Command::new(env!("CARGO_BIN_EXE_kerfd"))
            .arg("--dir")
            .arg(directory)
            .arg("--endpoint")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let printed = String::from_utf8(output.stdout).unwrap().trim().to_owned();
        let client = RuntimeClient::new(directory).unwrap();
        assert_eq!(
            client.endpoint(),
            &EndpointAddress::WindowsNamedPipe {
                name: printed.clone()
            }
        );
        assert!(printed.starts_with(r"\\.\pipe\kerfd-"), "{printed}");
        let daemon = start_daemon(directory);
        wait_until_ready(&client);
        assert!(raw_connection(client.endpoint()).is_ok());
        stop_daemon(daemon);
    }
    fs::remove_dir_all(&base).unwrap();
}

#[cfg(windows)]
#[test]
fn a_second_daemon_for_a_project_refuses_to_start() {
    let directory = project_directory("second");
    fs::create_dir_all(&directory).unwrap();
    let daemon = start_daemon(&directory);
    wait_until_ready(&RuntimeClient::new(&directory).unwrap());
    let second = Command::new(env!("CARGO_BIN_EXE_kerfd"))
        .arg("--dir")
        .arg(&directory)
        .output()
        .unwrap();
    assert!(!second.status.success());
    let error = String::from_utf8_lossy(&second.stderr);
    assert!(error.contains("RUNTIME_ALREADY_RUNNING"), "{error}");
    stop_daemon(daemon);
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn a_document_kerfd_cannot_read_is_what_the_client_is_told() {
    let directory = project_directory("unreadable");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("project.kerf"),
        "kerf 1\nproject name=\"Bad\"\nsequence s1 rate=24 size=1920x1080 audio=48000\n  track V1 video\n    clip c1 gen=solid@1 at=0f dur=24f\n      fx color.lumetri@2 no_such_thing=1\n",
    )
    .unwrap();
    let started = std::time::Instant::now();
    let client =
        RuntimeClient::connect_or_spawn(&directory, Some(Path::new(env!("CARGO_BIN_EXE_kerfd"))))
            .unwrap();
    let error = client.get_state().unwrap_err();
    // Says where and what, rather than that nothing was listening, and at once
    // rather than at the end of the wait for a daemon that has already gone.
    assert_eq!(error.code, "INVALID_PROJECT", "{}", error.message);
    assert!(error.message.contains("6:"), "{}", error.message);
    assert!(error.message.contains("no_such_thing"), "{}", error.message);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn a_client_starts_the_daemon_it_cannot_reach() {
    let directory = project_directory("spawn");
    fs::create_dir_all(&directory).unwrap();
    let client =
        RuntimeClient::connect_or_spawn(&directory, Some(Path::new(env!("CARGO_BIN_EXE_kerfd"))))
            .unwrap();
    let state = client.get_state().unwrap();
    assert_eq!(state.revision, 0);
    // Removing the project ends the daemon the client started. Ask through
    // a client made beforehand that starts nothing: making a client, or
    // starting kerfd, makes the folder again.
    let observer = RuntimeClient::new(&directory).unwrap();
    fs::remove_dir_all(&directory).unwrap();
    for _ in 0..250 {
        if observer.get_state().is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("the started kerfd kept running after its project was deleted");
}

#[test]
fn an_idle_daemon_exits_and_removes_its_socket() {
    let _project = Project::new("idle");
    let directory = _project.0.clone();
    let mut daemon = start_daemon_idle_for(&directory, 1);
    let client = RuntimeClient::new(&directory).unwrap();
    wait_until_ready(&client);
    assert!(wait_for_exit(&mut daemon).success());
    if let Some(path) = client.endpoint().as_path() {
        assert!(!path.exists(), "kerfd left its socket behind");
    }
    assert!(client.get_state().is_err());
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn an_idle_timeout_of_zero_keeps_a_daemon_nothing_holds() {
    let _project = Project::new("no-idle");
    let directory = _project.0.clone();
    let mut daemon = start_daemon_idle_for(&directory, 0);
    wait_until_ready(&RuntimeClient::new(&directory).unwrap());
    thread::sleep(Duration::from_secs(3));
    assert!(daemon.try_wait().unwrap().is_none(), "kerfd exited anyway");
    stop_daemon(daemon);
    let _ = fs::remove_dir_all(&directory);
}

/// Not a test of its own: `a_killed_client_releases_its_lease` runs this
/// binary again to hold a lease from a process it can kill.
#[test]
fn lease_holder_process() {
    let Some(directory) = std::env::var_os("KERFD_TEST_LEASE_HOLDER") else {
        return;
    };
    let lease = RuntimeClient::new(Path::new(&directory)).unwrap().hold();
    wait_until_held(&lease);
    println!("lease-held");
    // Until killed, or until the test that started it goes and closes its
    // stdin, so a failed test leaves nothing behind.
    let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
    std::process::exit(0);
}

#[test]
fn a_killed_client_releases_its_lease() {
    let _project = Project::new("killed");
    let directory = _project.0.clone();
    // No idle timeout, so only a released lease can end it.
    let mut daemon = start_daemon_idle_for(&directory, 0);
    wait_until_ready(&RuntimeClient::new(&directory).unwrap());
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args(["lease_holder_process", "--exact", "--nocapture"])
        .env("KERFD_TEST_LEASE_HOLDER", &directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = holder.stdout.take().unwrap();
    let (held, wait) = std::sync::mpsc::channel();
    thread::spawn(move || {
        use std::io::BufRead;
        let held_line = std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .any(|line| line.contains("lease-held"));
        let _ = held.send(held_line);
    });
    let attached = wait.recv_timeout(Duration::from_secs(30)).unwrap_or(false);
    if !attached {
        let _ = holder.kill();
        stop_daemon(daemon);
        panic!("the holder never leased kerfd");
    }
    thread::sleep(Duration::from_secs(2));
    let survived = daemon.try_wait().unwrap().is_none();
    holder.kill().unwrap();
    let _ = holder.wait();
    if !survived {
        let _ = fs::remove_dir_all(&directory);
        panic!("kerfd exited under a lease");
    }
    assert!(wait_for_exit(&mut daemon).success());
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn a_client_restarts_a_daemon_that_exited_and_finds_its_project_as_it_was() {
    let _project = Project::new("restart");
    let directory = _project.0.clone();
    let mut daemon = start_daemon(&directory);
    // Ready first, so the client does not start a daemon of its own.
    wait_until_ready(&RuntimeClient::new(&directory).unwrap());
    let client =
        RuntimeClient::connect_or_spawn(&directory, Some(Path::new(env!("CARGO_BIN_EXE_kerfd"))))
            .unwrap();
    let lease = client.hold();
    wait_until_held(&lease);
    let edited = client
        .apply_script_request("ui", "before-exit", "marker add at=0f as=kept", 0)
        .unwrap();
    assert_eq!(edited.revision, 1);
    drop(lease);
    assert!(wait_for_exit(&mut daemon).success());

    // The next request starts another kerfd, which reloads the project
    // and its history.
    let restarted = client.get_state().unwrap();
    assert_eq!(restarted.revision, 1);
    assert!(has_marker(&restarted, "kept"));
    let undone = client.undo_request("ui", "after-exit", 1).unwrap();
    assert_eq!(undone.revision, 2);
    assert!(!has_marker(&undone, "kept"));
    // Removing the project ends the daemon the client started.
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_client_outwaits_a_daemon_still_letting_go_of_the_project() {
    let _project = Project::new("handover");
    let directory = _project.0.clone();
    // A kerfd in stdio mode holds the project, as one on its way out does,
    // for longer than a new kerfd waits for it.
    let mut holder = Command::new(env!("CARGO_BIN_EXE_kerfd"))
        .arg("--dir")
        .arg(&directory)
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = holder.stdin.take().unwrap();
    let mut output = std::io::BufReader::new(holder.stdout.take().unwrap());
    input
        .write_all(b"{\"protocolVersion\":1,\"id\":\"held\",\"operation\":\"get_state\"}\n")
        .unwrap();
    let mut answer = String::new();
    std::io::BufRead::read_line(&mut output, &mut answer).unwrap();
    assert!(answer.contains("\"ok\":true"), "{answer}");
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(3300));
        drop(input);
        let _ = holder.wait();
    });
    let client =
        RuntimeClient::connect_or_spawn(&directory, Some(Path::new(env!("CARGO_BIN_EXE_kerfd"))))
            .unwrap();
    let state = client.get_state();
    release.join().unwrap();
    assert_eq!(state.unwrap().revision, 0);
}

#[cfg(unix)]
#[test]
fn a_daemon_that_cannot_start_is_not_started_again_for_a_read() {
    use std::os::unix::fs::PermissionsExt;

    let _project = Project::new("broken");
    let directory = _project.0.clone();
    let starts = directory.join("starts");
    let binary = directory.join("broken-kerfd");
    fs::write(
        &binary,
        format!("#!/bin/sh\necho start >> '{}'\nexit 1\n", starts.display()),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let client = RuntimeClient::connect_or_spawn(&directory, Some(&binary)).unwrap();
    let error = client.get_state().unwrap_err();
    assert_eq!(error.code, "RUNTIME_UNAVAILABLE", "{error:?}");
    // One start, and one more once the first has exited; the read is not
    // tried again on top.
    let count = fs::read_to_string(&starts).unwrap().lines().count();
    assert_eq!(count, 2);
}
