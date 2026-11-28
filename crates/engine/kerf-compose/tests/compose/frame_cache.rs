//! Frames kept on disk: what comes back is what was drawn, and only what changed is drawn again.

use kerf_compose::{frame_cache_stats, Accel, ComposeOptions, Compositor};
use kerf_lang::{apply_script, parse_document};
use kerf_model::{Catalog, Project, Rate};
use std::path::Path;

fn document() -> Project {
    parse_document(
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000\n  track V1 video\n    clip g1 gen=solid@1 at=0f dur=12f\n      set color=#336699\n      fx color.tint amount=40\n    clip g2 gen=solid@1 at=12f dur=12f\n      set color=#996633\n",
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project
}

fn compositor(project: &Project, folder: &Path) -> Compositor {
    Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Never,
            frame_folder: Some(folder.to_owned()),
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
}

fn plain(project: &Project) -> Compositor {
    Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            accel: Accel::Never,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
}

#[test]
fn only_what_an_edit_changes_is_drawn_again() {
    let dir = tempfile::tempdir().expect("a folder");
    let project = document();
    let first = compositor(&project, dir.path());
    let frames: Vec<_> = (0..24)
        .map(|at| first.frame(Rate::FPS_24.time_of(at), 1.0))
        .collect();
    first.flush_frames();
    let before = frame_cache_stats(dir.path());
    assert_eq!((before.hits, before.writes), (0, 24), "{before:?}");
    // A new compositor, as a new render or session makes, has nothing of its own.
    let again = compositor(&project, dir.path());
    for (at, frame) in frames.iter().enumerate() {
        assert_eq!(&again.frame(Rate::FPS_24.time_of(at as i64), 1.0), frame);
    }
    let read = frame_cache_stats(dir.path());
    assert_eq!(read.hits - before.hits, 24, "{read:?}");
    assert_eq!(read.writes, before.writes, "{read:?}");
    // An edit to the second clip leaves the first's frames kept.
    let edited = apply_script(&project, Catalog::builtin(), "set g2 color=#113355")
        .expect("edits")
        .project;
    let after_edit = compositor(&edited, dir.path());
    for at in 0..24 {
        let frame = after_edit.frame(Rate::FPS_24.time_of(at), 1.0);
        // What is drawn matches what comes with nothing kept.
        assert_eq!(frame, plain(&edited).frame(Rate::FPS_24.time_of(at), 1.0));
    }
    after_edit.flush_frames();
    let last = frame_cache_stats(dir.path());
    assert_eq!(last.hits - read.hits, 12, "{last:?}");
    assert_eq!(last.writes - read.writes, 12, "{last:?}");
}

#[test]
fn a_scale_and_a_depth_are_different_frames() {
    let dir = tempfile::tempdir().expect("a folder");
    let project = document();
    let kept = compositor(&project, dir.path());
    let at = Rate::FPS_24.time_of(3);
    let (full, half, deep) = (
        kept.frame(at, 1.0),
        kept.frame(at, 0.5),
        kept.frame16(at, 1.0),
    );
    kept.flush_frames();
    assert_eq!(frame_cache_stats(dir.path()).writes, 3);
    let again = compositor(&project, dir.path());
    assert_eq!(again.frame(at, 1.0), full);
    assert_eq!(again.frame(at, 0.5), half);
    assert_eq!(again.frame16(at, 1.0), deep);
    assert_eq!(frame_cache_stats(dir.path()).hits, 3);
    assert_eq!(deep, plain(&project).frame16(at, 1.0));
}

#[test]
fn a_frame_that_reads_what_the_document_does_not_hold_is_not_kept() {
    let dir = tempfile::tempdir().expect("a folder");
    // Another track's picture, through a matte, can come from anywhere.
    let project = parse_document(
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=160x90 audio=48000\n  track V1 video\n    clip g1 gen=solid@1 at=0f dur=12f\n      set color=#336699\n      fx key.trackmatte@1 matte=V2\n  track V2 video\n    clip g2 gen=solid@1 at=0f dur=12f\n      set color=#ffffff\n",
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let kept = compositor(&project, dir.path());
    kept.frame(Rate::FPS_24.time_of(3), 1.0);
    kept.flush_frames();
    assert_eq!(frame_cache_stats(dir.path()).writes, 0);
}

// What the stamp of the code that draws frames is made of, as its build script takes it in.
include!("../../stamped.rs");

/// The files `source` has built into it by name, with `include_str!` or `include_bytes!`.
fn built_in(source: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for taking in ["include_str!(", "include_bytes!("] {
        for (at, _) in source.match_indices(taking) {
            let named = source[at + taking.len()..].trim_start();
            if let Some(name) = named
                .strip_prefix('"')
                .and_then(|name| name.split('"').next())
            {
                found.push(name);
            }
        }
    }
    found
}

/// Every Rust file under `folder` but a crate's tests, which draw nothing a person is shown.
fn rust_files(folder: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_none_or(|name| name != "tests") {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|kind| kind == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_stamp_reads_every_file_built_into_what_draws() {
    // Frames kept on disk are read back only under the stamp they were drawn under, so a file
    // of a kind the stamp passes over can change what is drawn and leave the frames drawn
    // before it to be read: the prelude every kernel is written out with for OpenCL was one.
    // A look is none of the stamp's kinds and need not be: its lines become the effects of
    // each clip that takes it, and those are in the key of every frame the clip is in.
    let in_the_key = "kerflook";
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = Vec::new();
    for folder in STAMPED_FOLDERS {
        rust_files(&here.join(folder), &mut sources);
    }
    assert!(sources.len() > 100, "{} files found", sources.len());
    let mut passed_over = Vec::new();
    let mut built = 0;
    for path in &sources {
        let source = std::fs::read_to_string(path).expect("reads");
        for name in built_in(&source) {
            built += 1;
            let kind = name.rsplit('.').next().unwrap_or_default();
            if !STAMPED_KINDS.contains(&kind) && kind != in_the_key {
                passed_over.push(format!("{}: {name}", path.display()));
            }
        }
    }
    // The kernels, the catalogs and the fonts are built in this way: finding none of them
    // would be finding nothing.
    assert!(built > 40, "{built} files built in");
    assert!(
        passed_over.is_empty(),
        "built into what draws, and of no kind the stamp reads:\n{}",
        passed_over.join("\n")
    );
}
