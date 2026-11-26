//! A `render` step draws a guide at a model's size and rate: the
//! file made is inspected. The model is `cp`, so these run on Unix only.
#![cfg(unix)]

use kerf_engine::process::ffmpeg::resolve_ffmpeg_bin;
use kerf_engine::recipe::model::Models;
use kerf_engine::recipe::{make, plan};
use kerf_engine::CancellationToken;
use kerf_lang::parse_document;
use kerf_model::Catalog;
use std::path::Path;
use std::process::Command;

#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
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

/// The models file of a project in `directory` that names one program: `copy`, which
/// copies the guide it is handed to where its file goes.
fn copy(directory: &Path) -> Models {
    let file = directory.join("models.json");
    std::fs::write(&file, r#"{"copy": {"run": ["cp", "{guide}", "{out}"]}}"#)
        .expect("a models file");
    Models::read(&file).expect("models")
}

#[test]
fn a_guide_is_the_size_and_rate_its_step_names_and_has_no_sound() {
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    // A 160x90 guide: green across the middle, with red at each side for a tenth of its width
    // and more, which a 64x48 picture has no room for.
    let project = parse_document(
        "kerf 1\nproject name=\"guide\"\n\
         media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x90\n\
         \x20 render s2 {guide.mp4} size=64x48 rate=12\n\
         \x20 render s2 {whole.mp4}\n\
         \x20 generate model=copy guide={guide.mp4}\n\
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
    let copy = copy(directory);
    let planned = plan(&project, directory, "m1", &copy).expect("a plan");
    let mut said = Vec::new();
    let made = make(
        &project,
        directory,
        &planned,
        &copy,
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
    // The take is the guide, as the program the last step names copied it.
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

#[test]
fn a_pass_of_a_guide_is_a_file_of_its_own() {
    use kerf_compose::{ComposeOptions, Compositor, Pass};
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    // A backdrop out of depth, and a square in depth before it, lit from one side.
    let project = parse_document(
        "kerf 1\nproject name=\"guide\"\n\
         media m1 path=\"shots/open.mp4\" rate=24 dur=1s video=160x90\n\
         \x20 render s2 {ids.mp4} pass=ids\n\
         \x20 render s2 {depth.mp4} pass=depth\n\
         \x20 generate model=copy guide={ids.mp4}\n\
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
    let copy = copy(directory);
    let planned = plan(&project, directory, "m1", &copy).expect("a plan");
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
        &copy,
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

/// Every frame of `file`, decoded to `pix_fmt`, one after another.
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
fn decoded(file: &Path, pix_fmt: &str) -> Vec<u8> {
    let output = Command::new(resolve_ffmpeg_bin())
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-f", "rawvideo", "-pix_fmt", pix_fmt, "-"])
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// A phone stood in depth over a floor, its screen cut by a mask feathered over 30 pixels, a mesh
/// beside it, and a glow at less than full opacity, feathered too, that dissolves into a spark: a
/// scene whose clips share many pixels along soft edges.
const PHONE: &str = "\
sequence s1 rate=24 size=640x360 audio=48000
  track V1 video
sequence s2 rate=24 size=640x360 audio=48000
  track V1 video
    clip sky gen=solid@1 at=0f dur=12f
      set color=#88aaff
  track V2 video
    clip floor gen=solid@1 at=0f dur=12f
      set color=#404040 three_d=true rotate_x=-80 y=0.4 z=1 scale=3
  track V3 video
    clip phone gen=solid@1 at=0f dur=12f
      set color=#111111 three_d=true scale={0f:0.5 11f:0.6} rotate_y=25 rotation=8
      mask k1 rect=0.3,0.1,0.7,0.9 feather=6
  track V4 video
    clip screen gen=solid@1 at=0f dur=12f
      set color=#3080ff three_d=true scale={0f:0.5 11f:0.6} rotate_y=25 rotation=8 z=-0.01
      mask k1 rect=0.33,0.14,0.67,0.86 feather=30
  track V5 video
    clip ball gen=mesh@1 at=0f dur=12f
      set kind=ball width=0.3 height=0.3 length=0.3 x=0.35 y=0.1
  track V6 video
    clip glow gen=solid@1 at=0f dur=6f
      set color=#ffffff opacity=0.7 scale=0.3 x=-0.3 y=-0.2
      mask k1 ellipse=0.5,0.5,0.4,0.4 feather=40
    clip spark gen=solid@1 at=6f dur=6f
      set color=#ffcc00 scale=0.25 x=-0.25 y=-0.2 rotation=20
    transition t1 from=glow to=spark fx=dissolve@1 dur=4f
  track V7 video
    clip sun gen=light@1 at=0f dur=12f
      set kind=directional x=0.3 y=0.6 z=1 shadow=0.8
";

#[test]
fn a_pass_is_written_with_no_loss_and_each_pixel_of_an_ids_pass_is_one_clips() {
    use kerf_compose::{ComposeOptions, Compositor, Pass};
    use kerf_model::Rate;
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    std::fs::create_dir(directory.join("shots")).unwrap();
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"guide\"\n\
             media m1 path=\"shots/open.mp4\" rate=24 dur=12f video=640x360\n\
             \x20 render s2 {{ids.mp4}} pass=ids\n\
             \x20 render s2 {{ids.mkv}} pass=ids\n\
             \x20 render s2 {{depth.mp4}} pass=depth\n\
             \x20 render s2 {{depth.mkv}} pass=depth\n\
             \x20 render s2 {{relative.mkv}} pass=relative\n\
             \x20 render s2 {{screen.mp4}} matte=screen\n\
             \x20 generate model=copy guide={{ids.mp4}}\n\
             {PHONE}"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let copy = copy(directory);
    let planned = plan(&project, directory, "m1", &copy).expect("a plan");
    make(
        &project,
        directory,
        &planned,
        &copy,
        &CancellationToken::new(),
        |_| {},
    )
    .expect("made");
    let work = directory.join("shots/open.takes/001.work");
    let drawn = |pass: Pass, hard_edges: bool| {
        Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                sequence: Some("s2".to_owned()),
                pass,
                hard_edges,
                ..ComposeOptions::default()
            },
        )
        .expect("composes")
    };
    let rgb = |rgba: &[u8]| -> Vec<u8> {
        rgba.as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect()
    };
    let times: Vec<_> = (0..12).map(|frame| Rate::FPS_24.time_of(frame)).collect();

    // A reader takes a pixel for a clip's when each channel is within 6 of the clip's color.
    let soft = drawn(Pass::Ids, false);
    let colors: Vec<[u8; 3]> = soft.ids().into_iter().map(|(_, color)| color).collect();
    assert_eq!(colors.len(), 7, "{:?}", soft.ids());
    let unread = |pixels: &[u8]| {
        pixels
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|pixel| {
                **pixel != [0, 0, 0]
                    && !colors
                        .iter()
                        .any(|color| color.iter().zip(*pixel).all(|(a, b)| a.abs_diff(*b) <= 6))
            })
            .count()
    };
    // Drawn soft, as `look` reads how much of each clip shows, many pixels are of no clip.
    let between: usize = times
        .iter()
        .map(|at| unread(&rgb(&soft.frame(*at, 1.0).rgba)))
        .sum();
    assert!(between > 1000, "only {between} pixels between clips");
    // Written, every pixel of every frame is one clip's or black, and is what was drawn.
    let hard = drawn(Pass::Ids, true);
    let wanted: Vec<u8> = times
        .iter()
        .flat_map(|at| rgb(&hard.frame(*at, 1.0).rgba))
        .collect();
    assert_eq!(unread(&wanted), 0);
    for file in ["ids.mp4", "ids.mkv"] {
        let pixels = decoded(&work.join(file), "rgb24");
        assert_eq!(pixels.len(), wanted.len(), "{file}");
        assert_eq!(unread(&pixels), 0, "{file}");
        assert!(pixels == wanted, "{file} is not the pass as it was drawn");
    }
    // The screen's mask cuts where it is half through, so the screen is the screen's color well
    // in from where its feather starts, and the matte of it is white there and nowhere else.
    let matte = decoded(&work.join("screen.mp4"), "rgb24");
    let screen = colors[3];
    for (pixel, its) in matte
        .as_chunks::<3>()
        .0
        .iter()
        .zip(wanted.as_chunks::<3>().0)
    {
        let white = *its == screen;
        assert_eq!(*pixel, if white { [255; 3] } else { [0; 3] });
    }

    // Depth keeps 16 bits in Matroska and 10 in MP4, where 8 would give a floor tipped back
    // through the frame no more than 256 greys.
    let greys = |file: &str| -> Vec<u16> {
        decoded(&work.join(file), "gray16le")
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect()
    };
    let depth = drawn(Pass::Depth, false);
    let deep: Vec<u16> = times
        .iter()
        .flat_map(|at| {
            depth
                .frame16(*at, 1.0)
                .rgba
                .as_chunks::<8>()
                .0
                .iter()
                .map(|pixel| u16::from_le_bytes([pixel[0], pixel[1]]))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        greys("depth.mkv") == deep,
        "depth.mkv is not the pass as it was drawn"
    );
    let ten = greys("depth.mp4");
    assert_eq!(ten.len(), deep.len());
    // Rounded to 10 bits, as the file is read back at 16.
    let ten_of = |deep: u16| {
        let ten = (u32::from(deep) * 1023 + 32_767) / 65_535;
        (ten << 6 | ten >> 4) as u16
    };
    assert!(ten
        .iter()
        .zip(&deep)
        .all(|(ten, deep)| *ten == ten_of(*deep)));
    let distinct = |greys: &[u16]| {
        greys
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    assert!(distinct(&ten) > 256, "{} greys", distinct(&ten));
    assert!(distinct(&deep) > distinct(&ten));
    // Relative depth too: its farthest a 255th of white, above black at any depth.
    let relative = greys("relative.mkv");
    let first = &relative[..640 * 360];
    let farthest = first.iter().filter(|grey| **grey > 0).min().copied();
    assert_eq!(farthest, Some(257));
    assert_eq!(first.iter().max().copied(), Some(u16::MAX));
    assert!(distinct(first) > 256, "{} greys", distinct(first));
}

/// A scene of a backdrop a model films and a sign over its middle that Kerf draws, with the
/// colors given, and a media made from it as `steps` say after `render s2 {out}`.
fn scene_with_a_sign(back: &str, sign: &str, steps: &str, path: &str) -> kerf_model::Project {
    parse_document(
        &format!(
            "kerf 1\nproject name=\"shots\"\n\
             media m1 path=\"{path}\" rate=24 dur=12f video=160x90\n\
             \x20 render s2 {{out}}{steps}\n\
             sequence s1 rate=24 size=160x90 audio=48000\n\
             \x20 track V1 video\n\
             sequence s2 rate=24 size=160x90 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip back gen=solid@1 at=0f dur=12f\n\
             \x20     set color={back} filmed=true\n\
             \x20 track V2 video\n\
             \x20   clip sign gen=solid@1 at=0f dur=12f\n\
             \x20     set color={sign} scale=0.5\n"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project
}

#[test]
fn a_guide_of_only_what_a_model_films_is_held_to_those_clips_alone() {
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    let only = " only=filmed";
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 8);
    let project = scene_with_a_sign("#ff0000", "#0000ff", only, "shots/guide.mp4");
    let models = Models::default();
    let planned = plan(&project, directory, "m1", &models).expect("a plan");
    assert!(
        planned.steps[0].contains("what a model films of sequence s2"),
        "{:?}",
        planned.steps
    );
    let made = make(
        &project,
        directory,
        &planned,
        &models,
        &CancellationToken::new(),
        |_| {},
    )
    .expect("a take");
    // The sign no model films is not in it: the backdrop is where the sign would be.
    let middle = pixel(&directory.join(&made.stored), 80, 45);
    assert!(near(middle, [255, 0, 0]), "{middle:?}");
    // With the media that take, what it wants: nothing as things stand, and nothing when
    // what Kerf draws changes, the guide being as it was. A change to what a model films
    // wants a new take.
    let wants = |back: &str, sign: &str| {
        let project = scene_with_a_sign(back, sign, only, &made.stored);
        let media = project.media("m1").expect("the media").clone();
        kerf_engine::recipe::unmade(&project, directory, &media)
    };
    assert_eq!(wants("#ff0000", "#0000ff"), None);
    assert_eq!(wants("#ff0000", "#00ffff"), None);
    assert!(wants("#00ff00", "#0000ff").is_some());
    // The guide is as long as its sequence, so a clip Kerf draws that makes the sequence
    // longer wants one too.
    let mut longer = scene_with_a_sign("#ff0000", "#0000ff", only, &made.stored);
    for clip in longer.sequences[1]
        .tracks
        .iter_mut()
        .flat_map(|track| &mut track.clips)
        .filter(|clip| clip.id.as_str() == "sign")
    {
        clip.duration = clip.duration + clip.duration;
    }
    let media = longer.media("m1").expect("the media").clone();
    assert!(kerf_engine::recipe::unmade(&longer, directory, &media).is_some());
    // And a guide of all of it is held to all of it.
    let whole = scene_with_a_sign("#ff0000", "#0000ff", "", "shots/whole.mp4");
    let planned = plan(&whole, directory, "m1", &models).expect("a plan");
    let made = make(
        &whole,
        directory,
        &planned,
        &models,
        &CancellationToken::new(),
        |_| {},
    )
    .expect("a take");
    let middle = pixel(&directory.join(&made.stored), 80, 45);
    assert!(near(middle, [0, 0, 255]), "{middle:?}");
    let changed = scene_with_a_sign("#ff0000", "#00ffff", "", &made.stored);
    let media = changed.media("m1").expect("the media").clone();
    assert!(kerf_engine::recipe::unmade(&changed, directory, &media).is_some());
    // A sequence none of whose clips a model films has no such guide, which is said before
    // anything is made.
    let mut none = scene_with_a_sign("#ff0000", "#0000ff", only, "shots/none.mp4");
    for clip in none.sequences[1]
        .tracks
        .iter_mut()
        .flat_map(|track| &mut track.clips)
    {
        clip.props.remove("filmed");
    }
    let refused = plan(&none, directory, "m1", &models)
        .expect_err("refused")
        .to_string();
    assert!(refused.contains("`filmed=true`"), "{refused}");
    // Nor has one whose only such clip is off, which draws nothing.
    let mut off = scene_with_a_sign("#ff0000", "#0000ff", only, "shots/off.mp4");
    for clip in off.sequences[1]
        .tracks
        .iter_mut()
        .flat_map(|track| &mut track.clips)
        .filter(|clip| clip.id.as_str() == "back")
    {
        clip.enabled = false;
    }
    assert!(plan(&off, directory, "m1", &models).is_err());
}

#[test]
fn where_a_clip_goes_is_read_as_a_guide_of_only_what_a_model_films_shows_it() {
    // A car a model films passes behind a sign Kerf draws.
    let project = parse_document(
        "kerf 1\nproject name=\"shots\"\n\
         sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip car gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ff0000 filmed=true three_d=true z=1 scale=0.2 x={0f:-0.3 23f:0.3}\n\
         \x20 track V2 video\n\
         \x20   clip sign gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ffffff three_d=true scale=0.5\n",
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let middle = |only_filmed: bool| {
        kerf_engine::recipe::moves::Moves::read(&project, "car", only_filmed)
            .expect("its way")
            .places[12]
    };
    // Behind the sign it does not show, and the guide without the sign shows it there.
    assert_eq!(middle(false), None);
    assert!(middle(true).is_some());
}

#[test]
fn a_motion_guide_says_how_far_things_go_in_a_frame_of_its_file() {
    let folder = tempfile::tempdir().expect("a folder");
    let directory = folder.path();
    // A square that slides 2 pixels a frame of its sequence, drawn at half the sequence's
    // rate: 4 pixels a frame of the file.
    let project = parse_document(
        "kerf 1\nproject name=\"shots\"\n\
         media m1 path=\"shots/moves.mp4\" rate=12 dur=1s video=160x90\n\
         \x20 render s2 {out} pass=motion rate=12\n\
         sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         sequence s2 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip slides gen=solid@1 at=0f dur=24f\n\
         \x20     set scale=0.4 x={0f:-0.15 24f:0.15}\n",
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let models = Models::default();
    let planned = plan(&project, directory, "m1", &models).expect("a plan");
    assert!(
        planned.steps[0].contains("the motion of sequence s2"),
        "{:?}",
        planned.steps
    );
    let made = make(
        &project,
        directory,
        &planned,
        &models,
        &CancellationToken::new(),
        |_| {},
    )
    .expect("a take");
    let file = directory.join(&made.stored);
    // Red is how far right, of an eighth of the frame's height: 4 of 11.25 pixels past mid
    // grey. Green is how far down, none, and blue says the square is there; beside it, nothing.
    let [red, green, blue] = pixel(&file, 56, 45);
    assert!(red.abs_diff(173) <= 2, "{red}");
    assert!(green.abs_diff(128) <= 1 && blue == 255, "{green} {blue}");
    assert_eq!(pixel(&file, 150, 80), [0, 0, 0]);
}
