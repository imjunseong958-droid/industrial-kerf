//! Typed edits the editor sends (keyframes, markers, track roles and sync
//! lock) and the render queue's options and listing.

use super::*;
use serde_json::{json, Value};

/// A project folder with a 24 fps sequence: V1 holds c1 (0–2 s) and c2
/// (2–4 s), V2 a title at 3–4 s, and A1 sound linked to c1 and a later
/// clip at 3–4 s.
fn project(label: &str) -> (Runtime, PathBuf) {
    let directory = std::env::temp_dir().join(format!("kerf-e1-{label}-{}", unique_id()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(PROJECT_FILE),
        "kerf 1\nproject name=\"E1\"\n\n\
         media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000\n\n\
         sequence s1 name=\"Main\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:02:00 link=L1\n\
         \x20   clip c2 media=m1 src=00:00:10:00 at=00:00:02:00 dur=00:00:02:00\n\
         \x20 track V2 video\n\
         \x20   clip t1 gen=title@1 at=00:00:03:00 dur=00:00:01:00\n\
         \x20     set text=\"Day One\"\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:02:00 link=L1\n\
         \x20   clip a2 media=m1 src=00:00:20:00 at=00:00:03:00 dur=00:00:01:00\n",
    )
    .unwrap();
    (Runtime::open(&directory).unwrap(), directory)
}

// ---- renders ---------------------------------------------------------------

fn render(
    runtime: &mut Runtime,
    options: Value,
) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
    runtime.start_render_with(serde_json::from_value(options).unwrap())
}

#[test]
fn render_outputs_outside_the_project_are_refused() {
    let (mut runtime, directory) = project("render-outside");
    let missing = std::env::temp_dir().join(format!("kerf-e1-missing-{}", unique_id()));
    let refused = |runtime: &mut Runtime, output: Value| {
        let error = render(runtime, json!({"quality": "master", "output": output})).unwrap_err();
        assert_eq!(error.code, "INVALID_RENDER_OUTPUT", "{output}: {error:?}");
        error.message
    };
    assert!(refused(&mut runtime, json!("../elsewhere/")).contains("climbs out"));
    assert!(refused(&mut runtime, json!("deliveries/../../x.mov")).contains("climbs out"));
    assert!(refused(&mut runtime, json!(missing.join("out/"))).contains("does not exist"));
    assert!(refused(&mut runtime, json!(".git/")).contains("hidden folder"));
    assert!(refused(&mut runtime, json!("review/.cache/m.mov")).contains("hidden folder"));
    // The container decides the extension.
    assert!(refused(&mut runtime, json!("deliveries/master.mp4")).contains(".mov file"));
    assert!(refused(&mut runtime, json!(" ")).contains("empty"));
    assert!(!missing.exists(), "nothing is made outside the project");
    #[cfg(unix)]
    {
        // A link inside the project that leads out of it.
        let outside = std::env::temp_dir().join(format!("kerf-e1-outside-{}", unique_id()));
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, directory.join("exports")).unwrap();
        assert!(refused(&mut runtime, json!("exports/")).contains("leads outside"));
        // Folders that do not exist yet behind the link are looked at
        // before any is made, so nothing appears outside.
        assert!(refused(&mut runtime, json!("exports/new/sub/")).contains("leads outside"));
        assert!(refused(&mut runtime, json!("exports/new/m.mov")).contains("leads outside"));
        assert!(
            !outside.join("new").exists(),
            "nothing is made through a link"
        );
        // A link that leads nowhere is refused, not followed.
        std::os::unix::fs::symlink(outside.join("gone"), directory.join("dangling")).unwrap();
        assert!(refused(&mut runtime, json!("dangling/new/")).contains("leads outside"));
        assert!(!outside.join("gone").exists());
        let _ = fs::remove_dir_all(outside);
    }
    // Two queued renders never write one file.
    render(&mut runtime, json!({"quality": "master", "name": "Master"})).unwrap();
    let error = render(&mut runtime, json!({"quality": "master", "name": "Master"})).unwrap_err();
    assert_eq!(error.code, "RENDER_OUTPUT_BUSY");
    assert_eq!(runtime.render_jobs().len(), 1);
    // A busy render is refused before any folder is made for it.
    render(
        &mut runtime,
        json!({"quality": "master", "output": "busy/", "name": "B"}),
    )
    .unwrap();
    fs::remove_dir_all(directory.join("busy")).unwrap();
    let error = render(
        &mut runtime,
        json!({"quality": "master", "output": "busy/", "name": "B"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "RENDER_OUTPUT_BUSY");
    assert!(
        !directory.join("busy").exists(),
        "a refused render makes nothing"
    );
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn finished_renders_are_never_written_over() {
    let (mut runtime, directory) = project("render-again");
    let options = json!({"quality": "master", "output": "deliveries/", "name": "Master"});
    let (first, _, output) = render(&mut runtime, options.clone()).unwrap();
    assert_eq!(output, directory.join("deliveries/Master.mov"));
    runtime
        .update_job(&first.job_id, "completed", None, None, None)
        .unwrap();
    // Exporting `Master` again takes the next free name, so the finished
    // job still describes its file.
    let (second, _, output) = render(&mut runtime, options.clone()).unwrap();
    assert_eq!(output, directory.join("deliveries/Master 2.mov"));
    assert_eq!(second.output_path.as_deref(), output.to_str());
    assert_eq!(second.name.as_deref(), Some("Master"));
    assert_eq!(
        runtime
            .render_job(&first.job_id)
            .unwrap()
            .output_path
            .as_deref(),
        directory.join("deliveries/Master.mov").to_str()
    );
    // A file already on disk is a name taken too.
    fs::write(directory.join("deliveries/Taken.mov"), b"not a render").unwrap();
    let (_, _, output) = render(
        &mut runtime,
        json!({"quality": "master", "output": "deliveries/", "name": "Taken"}),
    )
    .unwrap();
    assert_eq!(output, directory.join("deliveries/Taken 2.mov"));
    assert_eq!(
        fs::read(directory.join("deliveries/Taken.mov")).unwrap(),
        b"not a render"
    );
    // A file named outright is refused rather than renamed.
    let error = render(
        &mut runtime,
        json!({"quality": "master", "output": "deliveries/Master.mov"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "RENDER_OUTPUT_EXISTS");
    // A job that failed without leaving a file does not hold its name.
    let (failed, _, _) = render(
        &mut runtime,
        json!({"quality": "master", "output": "deliveries/", "name": "Retry"}),
    )
    .unwrap();
    runtime
        .update_job(&failed.job_id, "failed", Some("boom".into()), None, None)
        .unwrap();
    let (_, _, output) = render(
        &mut runtime,
        json!({"quality": "master", "output": "deliveries/", "name": "Retry"}),
    )
    .unwrap();
    assert_eq!(output, directory.join("deliveries/Retry.mov"));
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}
