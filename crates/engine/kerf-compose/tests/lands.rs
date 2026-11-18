//! Where a clip of footage has its picture on the frame, said from its numbers: it is where
//! the frame is drawn.

use kerf_compose::{ComposeOptions, Compositor};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};
use std::path::Path;
use std::process::Command;

fn ffmpeg() -> String {
    std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned())
}

/// The white box of the media, in shares of its picture: left, top, right, bottom.
const BOX: [f64; 4] = [0.625, 0.25, 0.875, 0.5];

/// A second of black, 160 by 120 at 24 a second, with a white box 40 by 30 at 100, 30.
fn boxed(dir: &Path) -> String {
    let path = dir.join("boxed.mp4");
    let output = Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg("color=c=black:s=160x120:r=24:d=1,drawbox=x=100:y=30:w=40:h=30:color=white:t=fill")
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    path.display().to_string()
}

fn compositor(dir: &Path, tracks: &str) -> Compositor {
    let text = format!(
        "kerf 1\nproject name=\"t\"\n\
         media m path=\"{}\" rate=24 dur=24f video=160x120\n\
         sequence s1 rate=24 size=320x180 audio=48000\n{tracks}",
        boxed(dir)
    );
    let project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
    Compositor::new(&project, Catalog::builtin(), &ComposeOptions::default()).expect("composes")
}

/// The box around what is light on the frame at `frame`, in its pixels.
fn drawn(compositor: &Compositor, frame: i64) -> [f64; 4] {
    let picture = compositor.frame(Rate::FPS_24.time_of(frame), 1.0);
    let width = picture.width as usize;
    let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0, 0);
    for (index, pixel) in picture.rgba.as_chunks::<4>().0.iter().enumerate() {
        if pixel[0] > 128 {
            let (x, y) = (index % width, index / width);
            (left, top) = (left.min(x), top.min(y));
            (right, bottom) = (right.max(x + 1), bottom.max(y + 1));
        }
    }
    [left as f64, top as f64, right as f64, bottom as f64]
}

/// The box around where the media's white box lands on the frame at `frame`, in its pixels,
/// as the clip `c1` is said to put it.
fn said(compositor: &Compositor, frame: i64) -> [f64; 4] {
    let landing = compositor
        .lands("c1", Rate::FPS_24.time_of(frame))
        .expect("it lands");
    let mut around = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for corner in [[BOX[0], BOX[1]], [BOX[2], BOX[1]], [BOX[2], BOX[3]], [BOX[0], BOX[3]]] {
        let [x, y] = landing.on_frame(corner);
        around = [
            around[0].min(x * 320.0),
            around[1].min(y * 180.0),
            around[2].max(x * 320.0),
            around[3].max(y * 180.0),
        ];
    }
    around
}

#[test]
fn a_clip_lands_where_its_frame_is_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let clip = "  track V1 video\n    clip c1 media=m src=0f at=0f dur=24f";
    for (set, frame) in [
        // Fitted to the frame by its height, with nothing set.
        ("", 3),
        ("\n      set scale=0.5 x=0.2 y=-0.1", 3),
        ("\n      set scale=1.4 scale_y=0.8 anchor_x=0.2", 3),
        ("\n      set rotation=90", 3),
        ("\n      set rotation=30 x=-0.1", 3),
        ("\n      set scale_x=-1", 3),
        // A crop is fitted again, and moves what it leaves.
        ("\n      set crop_l=0.2", 3),
        ("\n      set crop_t=0.1 crop_b=0.3 scale=0.7", 3),
        // Where its keys have it on that frame, and along a path.
        ("\n      set x={0f:-0.2 20f:0.2} scale={0f:0.5 20f:1}", 15),
        ("\n      set path=\"-0.2,0.1 0,-0.1 0.2,0.1\" along={0f:0 20f:1}", 10),
        // Moved with a parent that is itself scaled and turned.
        (
            " parent=n1\n  track V2 video\n    clip n1 gen=null@1 at=0f dur=24f\n      set x=0.15 y=0.1 scale=0.8 rotation=20",
            3,
        ),
    ] {
        let compositor = compositor(dir.path(), &format!("{clip}{set}\n"));
        let (drawn, said) = (drawn(&compositor, frame), said(&compositor, frame));
        assert!(
            drawn.iter().zip(said).all(|(drawn, said)| (drawn - said).abs() <= 1.5),
            "`{set}`: drawn at {drawn:?}, said at {said:?}"
        );
    }
}

#[test]
fn a_landing_says_which_frame_shows_and_what_is_done_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let compositor = compositor(
        dir.path(),
        concat!(
            "  track V1 video\n",
            "    clip c1 media=m src=6f at=0f dur=6f speed=2\n",
            "      set crop_l=0.1 crop_b=0.25 opacity={0f:1 5f:0.5}\n",
            "      mask k1 rect=0.1,0.1,0.9,0.9\n",
            "      fx color.basic@1 exposure=0.3\n",
            "      fx look.shake@1 hit=0f\n",
            "    clip c2 media=m at=6f dur=6f\n",
            "      set three_d=true\n",
            "    clip c3 gen=solid@1 at=12f dur=6f\n",
        ),
    );
    let at = |frame: i64| Rate::FPS_24.time_of(frame);
    // Six frames into its media, at twice the speed: the frame that shows, with no file read.
    assert_eq!(compositor.lands("c1", at(0)).unwrap().frame, 6);
    let later = compositor.lands("c1", at(5)).unwrap();
    assert_eq!(later.frame, 16);
    assert_eq!(later.kept, [0.1, 0.0, 1.0, 0.75]);
    assert!((later.opacity - 0.5).abs() < 1e-9 && later.masked, "{later:?}");
    // A look is the effects it stands for.
    assert_eq!(later.effects, ["color.basic", "blur.directional"]);
    // What does not show then, what a camera places, and what is no footage are refused.
    assert!(compositor.lands("c1", at(6)).unwrap_err().contains("does not show then"));
    assert!(compositor.lands("c2", at(6)).unwrap_err().contains("in depth"));
    assert!(compositor.lands("c3", at(12)).unwrap_err().contains("plays no footage"));
    assert!(compositor.lands("c9", at(0)).unwrap_err().contains("no clip of this sequence"));
}
