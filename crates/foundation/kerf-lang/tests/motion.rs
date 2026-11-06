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

#[test]
fn a_kind_that_is_not_one_is_refused_with_a_reason() {
    let wrong = DOCUMENT.replace("kind=path", "kind=blob");
    let errors = match parse_document(&wrong, Catalog::builtin()) {
        Ok(_) => panic!("took `kind=blob`"),
        Err(errors) => errors,
    };
    assert!(format!("{errors:?}").contains("kind"), "{errors:?}");
}
