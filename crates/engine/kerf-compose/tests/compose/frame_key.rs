//! What a kept frame depends on: the same picture under the same number, whichever document or
//! compositor asks, and another number whenever the picture could differ.

use kerf_compose::{ComposeOptions, Compositor};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::path::Path;
use std::process::Command;

fn footage(dir: &Path) -> String {
    let path = dir.join("footage.mp4");
    let ffmpeg = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc2=s=160x90:r=24:d=2"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(output.status.success());
    path.display().to_string()
}

struct Media {
    /// Holds the file while the media is in use.
    _dir: tempfile::TempDir,
    path: String,
}

impl Media {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a folder");
        let path = footage(dir.path());
        Self { _dir: dir, path }
    }

    fn compositor(&self, body: &str, keep: bool) -> Compositor {
        let document = format!(
            "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=48f video=160x90\nsequence s1 rate=24 size=160x90 audio=48000\n{body}",
            self.path
        );
        let project = parse_document(&document, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                keep_frames: keep,
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
    }

    fn key(&self, body: &str, frame: i64) -> Option<u64> {
        self.compositor(body, false)
            .frame_key(Rate::FPS_24.time_of(frame), 1.0)
    }
}

const TWO: &str = "  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n    clip c2 media=m src=12f at=12f dur=12f\n";

#[test]
fn an_edit_leaves_the_frames_it_does_not_touch() {
    let media = Media::new();
    let edited = format!("{TWO}      fx color.tint@2 amount=40\n");
    // Frame 3 shows `c1`, which did not change; frame 15 shows `c2`, which did.
    assert_eq!(media.key(TWO, 3), media.key(&edited, 3));
    assert_ne!(media.key(TWO, 15), media.key(&edited, 15));
    assert!(media.key(TWO, 3).is_some());
}

#[test]
fn what_changes_the_picture_changes_the_number() {
    let media = Media::new();
    let clip = "  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n";
    let base = media.key(clip, 6);
    for changed in [
        format!("{clip}      set opacity=0.5\n"),
        format!("{clip}      set scale=0.5\n"),
        format!("{clip}      set blend=screen\n"),
        format!("{clip}      mask k1 rect=0.1,0.1,0.9,0.9\n"),
        format!("{clip}      fx blur.gaussian@1 radius=4\n"),
        "  track V1 video\n    clip c1 media=m src=2f at=0f dur=24f\n".to_owned(),
        "  track V1 video\n    clip c9 media=m src=0f at=0f dur=24f\n".to_owned(),
    ] {
        assert_ne!(base, media.key(&changed, 6), "{changed}");
    }
    // Animation counts at the time it reaches.
    let animated = format!("{clip}      set opacity={{0f:0 24f:1}}\n");
    assert_ne!(media.key(&animated, 6), media.key(&animated, 7));
    // The layer above counts, and a layer no longer showing does not.
    let over = format!(
        "{clip}  track V2 video\n    clip t1 gen=solid@1 at=0f dur=12f\n      set color=#ff0000\n"
    );
    assert_ne!(base, media.key(&over, 6));
    assert_eq!(media.key(clip, 20), media.key(&over, 20));
    // Another size is another picture.
    let compositor = media.compositor(clip, false);
    let at = Rate::FPS_24.time_of(6);
    assert_ne!(compositor.frame_key(at, 1.0), compositor.frame_key(at, 0.5));
}

#[test]
fn what_the_document_does_not_hold_is_never_kept() {
    let media = Media::new();
    let matte = "  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n      fx key.trackmatte matte=V2\n  track V2 video\n    clip c2 media=m src=0f at=0f dur=24f\n";
    assert_eq!(media.key(matte, 6), None);
    let compositor = media.compositor(matte, true);
    let at = Rate::FPS_24.time_of(6);
    let frame = compositor.frame(at, 1.0);
    assert!(!frame.rgba.is_empty());
    assert!(!compositor.is_kept(at, 1.0));
}

#[test]
fn burn_ins_count_where_they_are_on_the_sequence() {
    let media = Media::new();
    let burned =
        "  track V1 video\n    clip c1 media=m src=0f at=0f dur=12f\n      fx text.timecode\n";
    let later =
        "  track V1 video\n    clip c1 media=m src=0f at=24f dur=12f\n      fx text.timecode\n";
    assert_ne!(media.key(burned, 5), media.key(later, 29));
}

#[test]
fn a_kept_frame_comes_back_to_a_new_compositor() {
    let media = Media::new();
    let clip = "  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n      set scale=0.77\n";
    let at = Rate::FPS_24.time_of(9);
    let first = media.compositor(clip, true);
    assert!(!first.is_kept(at, 1.0));
    let drawn = first.frame(at, 1.0);
    // The compositor an edit would make, with its own decoders.
    let again = media.compositor(clip, true);
    assert!(again.is_kept(at, 1.0));
    assert_eq!(again.frame(at, 1.0), drawn);
    // Without asking to keep frames, nothing is kept.
    let passing = media.compositor(
        "  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f\n      set scale=0.33\n",
        false,
    );
    passing.frame(at, 1.0);
    assert!(!passing.is_kept(at, 1.0));
}
