//! Shapes, particles, and titles that come in, as a document says them.

use kerf_lang::{parse_document, print_document, PrintOptions};
use kerf_model::Catalog;

const DOCUMENT: &str = r##"kerf 1
project name="t"
sequence s1 rate=24 size=1280x720 audio=48000
  track V1 video
    clip c1 gen=shape@1 at=0f dur=48f
      set closed=false fill=#ffffff00 kind=path points="-0.5,0 0.1,0.2 0.5,0" smooth=true stroke=#ff0000 stroke_width=0.01 trim_end={0f:0 24f:1}
    clip c2 gen=particles@1 at=48f dur=48f
      set color_end=#ff000000 gravity=0.5 life=2 rate=200 seed=7 spread=30
    clip c3 gen=title@1 at=96f dur=48f
      set from_scale=2 from_y=0.1 reveal={0f:0 24f:1} reveal_by=word text="Two words"
"##;

#[test]
fn they_round_trip() {
    let once =
        parse_document(DOCUMENT, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let printed = print_document(&once.project, Catalog::builtin(), PrintOptions::default());
    let twice =
        parse_document(&printed, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(once.project, twice.project);
    for word in [
        "kind=path",
        "trim_end={",
        "gen=particles@1",
        "reveal_by=word",
        "seed=7",
    ] {
        assert!(printed.contains(word), "{word} in {printed}");
    }
}

const OUTLINES: &str = r##"kerf 1
project name="t"
sequence s1 rate=24 size=1280x720 audio=48000
  track V1 video
    clip c1 gen=shape@1 at=0f dur=48f
      set kind=rect offset={0f:0 24f:0.02} stroke_width=0.02 taper_end=0.25 taper_start={0f:0 24f:0.5}
      fx shape.outline@1 height=0.1 kind=ellipse mode=subtract rotation={0f:0 24f:90} width=0.1 x={0f:-0.1 24f:0.1} y=0.05
      fx shape.outline@1 kind=path points="0,0 0.2,0 0.1,0.2" smooth=true
"##;

fn refused(document: &str, word: &str) {
    let errors = match parse_document(document, Catalog::builtin()) {
        Ok(_) => panic!("took {document}"),
        Err(errors) => errors,
    };
    assert!(format!("{errors:?}").contains(word), "{word}: {errors:?}");
}

const AXES: &str = r##"kerf 1
project name="t"
sequence s1 rate=24 size=1280x720 audio=48000
  track V1 video
    clip c1 gen=title@1 at=0f dur=48f
      set font="Some Variable" ital=1 opsz=14 slnt=-10 text="Words" wdth=75 wght={0f:100 24f:900}
"##;

#[test]
fn a_titles_axes_round_trip_and_a_bad_one_is_refused() {
    let once =
        parse_document(AXES, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let printed = print_document(&once.project, Catalog::builtin(), PrintOptions::default());
    let twice =
        parse_document(&printed, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(once.project, twice.project);
    for word in ["wght={", "wdth=75", "slnt=-10", "opsz=14", "ital=1"] {
        assert!(printed.contains(word), "{word} in {printed}");
    }
    refused(&AXES.replace("wdth=75", "wdth=5000"), "wdth");
    refused(&AXES.replace("slnt=-10", "slnt=-100"), "slnt");
    refused(&AXES.replace("ital=1", "ital=2"), "ital");
    refused(&AXES.replace("wght={0f:100 24f:900}", "wght=-1"), "wght");
    // A shape has no font to set.
    refused(&OUTLINES.replace("kind=rect", "kind=rect wght=700"), "wght");
}

#[test]
fn outlines_round_trip_with_what_animates() {
    let once =
        parse_document(OUTLINES, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let printed = print_document(&once.project, Catalog::builtin(), PrintOptions::default());
    let twice =
        parse_document(&printed, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(once.project, twice.project);
    for word in [
        "fx shape.outline@1",
        "mode=subtract",
        "offset={",
        "taper_start={",
        "taper_end=0.25",
        "x={",
        "rotation={",
        "points=\"0,0 0.2,0 0.1,0.2\"",
    ] {
        assert!(printed.contains(word), "{word} in {printed}");
    }
}

#[test]
fn a_bad_outline_or_stroke_is_refused_when_the_document_reads() {
    refused(&OUTLINES.replace("mode=subtract", "mode=blend"), "mode");
    refused(&OUTLINES.replace("kind=ellipse", "kind=blob"), "kind");
    refused(
        &OUTLINES.replace("offset={0f:0 24f:0.02}", "offset=99"),
        "offset",
    );
    refused(
        &OUTLINES.replace("taper_end=0.25", "taper_end=2"),
        "taper_end",
    );
    refused(
        &OUTLINES.replace("taper_end=0.25", "taper_end=-0.5"),
        "taper_end",
    );
    refused(&OUTLINES.replace("y=0.05", "y=101"), "`y`");
    refused(&OUTLINES.replace("0,0 0.2,0 0.1,0.2", "0,0 x,1"), "points");
    // Two points hold no area.
    refused(&OUTLINES.replace("0,0 0.2,0 0.1,0.2", "0,0 0.2,0"), "three");
    // Only a shape has outlines to combine.
    for generator in ["solid", "title"] {
        refused(
            &format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=1280x720 audio=48000\n  track V1 video\n    clip c1 gen={generator}@1 at=0f dur=48f\n      fx shape.outline@1 kind=ellipse\n"
            ),
            "gen=shape",
        );
    }
}

#[test]
fn a_kind_that_is_not_one_is_refused_with_a_reason() {
    let wrong = DOCUMENT.replace("kind=path", "kind=blob");
    let errors = match parse_document(&wrong, Catalog::builtin()) {
        Ok(_) => panic!("took `kind=blob`"),
        Err(errors) => errors,
    };
    assert!(format!("{errors:?}").contains("kind"), "{errors:?}");
}

const SOFT: &str = r##"kerf 1
project name="t"
sequence s1 rate=24 size=1280x720 audio=48000
  track V1 video
    clip c1 gen=solid@1 at=0f dur=48f
      mask k1 points="0.1,0.1,4 0.9,0.1,12.5 0.9,0.9,0 0.1,0.9,3" feather=2
      mask k2 points={0f:0,0,1;0.5,0,2;0.5,0.5,3 24f:0.1,0,4;0.6,0,5;0.6,0.5,6:ease}
      mask k3 points="0.1,0.1 0.9,0.1,12 0.9,0.9"
      mask k4 points="0.1,0.1 0.9,0.1 0.9,0.9"
"##;

#[test]
fn a_masks_soft_corners_round_trip_and_a_bad_one_is_refused() {
    let once =
        parse_document(SOFT, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let printed = print_document(&once.project, Catalog::builtin(), PrintOptions::default());
    let twice =
        parse_document(&printed, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(once.project, twice.project);
    for word in [
        "points=\"0.1,0.1,4 0.9,0.1,12.5 0.9,0.9,0 0.1,0.9,3\"",
        "0f:0,0,1;0.5,0,2;0.5,0.5,3",
        // What one corner says, all of them say.
        "points=\"0.1,0.1,0 0.9,0.1,12 0.9,0.9,0\"",
        // And nothing changes for an outline that says none.
        "points=\"0.1,0.1 0.9,0.1 0.9,0.9\"",
    ] {
        assert!(printed.contains(word), "{word} in {printed}");
    }
    let k1 = "0.1,0.1,4 0.9,0.1,12.5 0.9,0.9,0 0.1,0.9,3";
    refused(&SOFT.replace(k1, "0.1,0.1,-4 0.9,0.1 0.9,0.9"), "soft");
    refused(&SOFT.replace(k1, "0.1,0.1,20000 0.9,0.1 0.9,0.9"), "soft");
    refused(&SOFT.replace(k1, "0.1,0.1,1,2 0.9,0.1 0.9,0.9"), "x,y,soft");
    refused(
        &SOFT.replace(k1, "0.1,0.1,x 0.9,0.1 0.9,0.9"),
        "not a number",
    );
    // An open outline has no edge.
    refused(&SOFT.replace("feather=2", "closed=false"), "open outline");
    // A key's softness is held to the same.
    refused(&SOFT.replace("0.5,0.5,3", "0.5,0.5,-3"), "soft");
}

const DEPTH: &str = r##"kerf 1
project name="t"
sequence s1 rate=24 size=1280x720 audio=48000
  track V1 video
    clip c1 gen=solid@1 at=0f dur=48f
      set three_d=true z=0.5
  track V2 video
    clip l1 gen=light@1 at=0f dur=48f
      set kind=directional shadow={0f:0 24f:0.8} size={0f:0 24f:5} x=0.2 z=1
    clip l2 gen=light@1 at=48f dur=48f
      set shadow=1 size=0.1
  track V3 video
    clip k1 gen=camera@1 at=0f dur=48f
      set aperture=0.05 focus={0f:0 24f:0.5}
    clip k2 gen=camera@1 at=48f dur=48f
      set aperture={0f:0 24f:0.1} focus=2
"##;

#[test]
fn a_lights_shadow_round_trips_and_a_bad_one_is_refused() {
    let once =
        parse_document(DEPTH, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let printed = print_document(&once.project, Catalog::builtin(), PrintOptions::default());
    let twice =
        parse_document(&printed, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(once.project, twice.project);
    for word in ["shadow={0f:0 24f:0.8}", "shadow=1"] {
        assert!(printed.contains(word), "{word} in {printed}");
    }
    for bad in ["shadow=2", "shadow=-0.1", "shadow={0f:0 24f:2}"] {
        refused(&DEPTH.replace("shadow=1", bad), "shadow");
    }
}

#[test]
fn a_lights_size_round_trips_and_a_bad_one_is_refused() {
    let once =
        parse_document(DEPTH, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let printed = print_document(&once.project, Catalog::builtin(), PrintOptions::default());
    let twice =
        parse_document(&printed, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(once.project, twice.project);
    for word in ["size={0f:0 24f:5}", "size=0.1"] {
        assert!(printed.contains(word), "{word} in {printed}");
    }
    for bad in ["size=-1", "size=91", "size={0f:0 24f:91}", "size=wide"] {
        refused(&DEPTH.replace("size=0.1", bad), "size");
    }
    // Only a light is that wide.
    refused(
        &DEPTH.replace("set aperture=0.05", "set size=1 aperture=0.05"),
        "size",
    );
}

#[test]
fn a_cameras_focus_round_trips_and_a_bad_one_is_refused() {
    let once =
        parse_document(DEPTH, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    let printed = print_document(&once.project, Catalog::builtin(), PrintOptions::default());
    let twice =
        parse_document(&printed, Catalog::builtin()).unwrap_or_else(|errors| panic!("{errors:?}"));
    assert_eq!(once.project, twice.project);
    for word in [
        "aperture=0.05",
        "focus={0f:0 24f:0.5}",
        "aperture={0f:0 24f:0.1}",
        "focus=2",
    ] {
        assert!(printed.contains(word), "{word} in {printed}");
    }
    refused(&DEPTH.replace("aperture=0.05", "aperture=2"), "aperture");
    refused(&DEPTH.replace("aperture=0.05", "aperture=-0.1"), "aperture");
    refused(&DEPTH.replace("focus=2", "focus=2000"), "focus");
    refused(&DEPTH.replace("focus=2", "focus=near"), "focus");
}
