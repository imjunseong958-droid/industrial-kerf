//! A recipe's `render` step draws a guide at a model's size and rate: the
//! file made is inspected.

use kerf_engine::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::recipe::model::Models;
use kerf_engine::recipe::{make, plan};
use kerf_engine::CancellationToken;
use kerf_lang::parse_document;
use kerf_model::Catalog;
use std::path::Path;
use std::process::Command;

fn run(command: &mut Command) -> String {
    let output = command.output().expect("runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// What ffprobe says of `entries` of `file`'s streams of kind `select`.
fn probe(file: &Path, select: &str, entries: &str) -> String {
    run(Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-select_streams", select])
        .args(["-show_entries", entries, "-of", "csv=p=0"])
        .arg(file))
}

/// The RGB of the pixel at `x`, `y` of the first frame of `file`.
fn pixel(file: &Path, x: u32, y: u32) -> [u8; 3] {
    let output = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("format=rgb24,crop=1:1:{x}:{y}"))
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    [output.stdout[0], output.stdout[1], output.stdout[2]]
}

#[cfg(unix)]
#[test]
fn a_guide_is_the_size_and_rate_its_step_names_and_has_no_sound() {
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    std::fs::write(
        directory.join("shots/open.kerfrecipe"),
        "kerf-recipe 1\nrender s2 {guide.mp4} size=64x48 rate=12\nrender s2 {whole.mp4}\nrun cp {guide.mp4} {out}\n",
    )
    .unwrap();
    // A 160x90 guide: green across the middle, with red at each side for a tenth of its width
    // and more, which a 64x48 picture has no room for.
    let project = parse_document(
        "kerf 1\nproject name=\"guide\"\n\
         media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x90 recipe=\"shots/open.kerfrecipe\"\n\
         sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         sequence s2 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip side gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ff0000\n\
         \x20 track V2 video\n\
         \x20   clip middle gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#00ff00 scale_x=0.78\n",
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let none = Models::default();
    let planned = plan(&project, directory, "m1", &none).expect("a plan");
    let mut said = Vec::new();
    let made = make(
        &project,
        directory,
        &planned,
        &none,
        &CancellationToken::new(),
        |line| said.push(line.to_owned()),
    )
    .expect("made");
    assert!(
        said.iter().any(|line| line == "rendering s2: 100%"),
        "{said:?}"
    );

    let guide = directory.join("shots/open.takes/001.work/guide.mp4");
    assert_eq!(probe(&guide, "v:0", "stream=width,height"), "64,48");
    assert_eq!(probe(&guide, "v:0", "stream=r_frame_rate"), "12/1");
    // A second of the sequence, at twelve frames a second.
    assert_eq!(probe(&guide, "v:0", "stream=nb_read_frames"), "12");
    assert_eq!(probe(&guide, "a", "stream=codec_name"), "", "no sound");
    // The picture fills the size: its sides are cut off, and nothing is squeezed or barred.
    for (x, y) in [(0, 0), (63, 0), (32, 24), (0, 47), (63, 47)] {
        let [red, green, _] = pixel(&guide, x, y);
        assert!(red < 60 && green > 190, "({x}, {y}) is {red}, {green}");
    }
    // The take is the guide, as the last step copied it.
    assert_eq!(
        probe(&directory.join(&made.stored), "v:0", "stream=width,height"),
        "64,48"
    );

    // With nothing named, a step draws the sequence as it is, still with no sound.
    let whole = directory.join("shots/open.takes/001.work/whole.mp4");
    assert_eq!(probe(&whole, "v:0", "stream=width,height"), "160,90");
    assert_eq!(probe(&whole, "v:0", "stream=nb_read_frames"), "24");
    assert_eq!(probe(&whole, "a", "stream=codec_name"), "");
    let [red, green, _] = pixel(&whole, 2, 45);
    assert!(red > 190 && green < 60, "the side is {red}, {green}");
}

#[cfg(unix)]
#[test]
fn a_pass_of_a_guide_is_a_file_of_its_own() {
    use kerf_compose::{ComposeOptions, Compositor, Pass};
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    std::fs::write(
        directory.join("shots/open.kerfrecipe"),
        "kerf-recipe 1\nrender s2 {ids.mp4} pass=ids\nrender s2 {depth.mp4} pass=depth\nrun cp {ids.mp4} {out}\n",
    )
    .unwrap();
    // A backdrop out of depth, and a square in depth before it, lit from one side.
    let project = parse_document(
        "kerf 1\nproject name=\"guide\"\n\
         media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x90 recipe=\"shots/open.kerfrecipe\"\n\
         sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         sequence s2 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip sky gen=solid@1 at=0f dur=12f\n\
         \x20     set color=#ff0000\n\
         \x20 track V2 video\n\
         \x20   clip square gen=solid@1 at=0f dur=12f\n\
         \x20     set color=#00ff00 scale=0.5 three_d=true\n\
         \x20 track V3 video\n\
         \x20   clip sun gen=light@1 at=0f dur=12f\n\
         \x20     set kind=directional x=1 z=0.2\n",
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let none = Models::default();
    let planned = plan(&project, directory, "m1", &none).expect("a plan");
    assert!(
        planned.steps[0].starts_with("Kerf renders the ids of sequence s2 to "),
        "{}",
        planned.steps[0]
    );
    assert!(
        planned.steps[1].starts_with("Kerf renders the depth of sequence s2 to "),
        "{}",
        planned.steps[1]
    );
    make(
        &project,
        directory,
        &planned,
        &none,
        &CancellationToken::new(),
        |_| {},
    )
    .expect("made");

    // The colors an ids pass gives the sequence's clips are what its file holds.
    let ids = Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some("s2".to_owned()),
            pass: Pass::Ids,
            ..ComposeOptions::default()
        },
    )
    .expect("composes")
    .ids();
    assert_eq!(
        ids.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
        ["sky", "square"]
    );
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 8);
    let file = directory.join("shots/open.takes/001.work/ids.mp4");
    assert_eq!(probe(&file, "v:0", "stream=nb_read_frames"), "12");
    let (corner, middle) = (pixel(&file, 4, 4), pixel(&file, 80, 45));
    assert!(near(corner, ids[0].1), "{corner:?} for {:?}", ids[0].1);
    assert!(near(middle, ids[1].1), "{middle:?} for {:?}", ids[1].1);

    // Depth: the square, at z=0 before a camera left where it is, is mid grey, whatever lights
    // it, and what is out of depth is black.
    let file = directory.join("shots/open.takes/001.work/depth.mp4");
    let (corner, middle) = (pixel(&file, 4, 4), pixel(&file, 80, 45));
    assert!(near(corner, [0, 0, 0]), "{corner:?}");
    assert!(near(middle, [128, 128, 128]), "{middle:?}");
}
