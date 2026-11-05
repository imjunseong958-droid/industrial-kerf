//! What the compositor draws where the graph cannot be asked: text,
//! blend modes, masks, transitions, and effects.

use kerf_compose::{ComposeError, ComposeOptions, Compositor, Frame};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

fn compositor(body: &str, captions: bool) -> Result<Compositor, ComposeError> {
    let project = parse_document(
        &format!("kerf 1\nproject name=\"t\"\n{body}"),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    Compositor::new(
        &project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: None,
            captions,
        },
    )
}

fn frame(body: &str, at: i64, captions: bool) -> Frame {
    let compositor = compositor(body, captions).expect("composes");
    compositor.frame(Rate::FPS_24.time_of(at), 1.0)
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * frame.width + x) * 4) as usize;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

/// The bounds of the pixels brighter than `level`, as (left, top, right,
/// bottom), if any are.
fn bright(frame: &Frame, level: u8) -> Option<(u32, u32, u32, u32)> {
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for y in 0..frame.height {
        for x in 0..frame.width {
            if pixel(frame, x, y).iter().all(|c| *c > level) {
                let (l, t, r, b) = bounds.unwrap_or((x, y, x, y));
                bounds = Some((l.min(x), t.min(y), r.max(x), b.max(y)));
            }
        }
    }
    bounds
}

const SEQUENCE: &str = "sequence s1 rate=24 size=320x180 audio=48000\n";

#[test]
fn titles_are_centered_on_the_frame() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip t1 gen=title@1 at=0f dur=24f\n\
         \x20     set text=\"KERF\" size=0.25\n"
    );
    let frame = frame(&body, 6, false);
    let (left, top, right, bottom) = bright(&frame, 200).expect("the title shows");
    let (x, y) = ((left + right) / 2, (top + bottom) / 2);
    assert!(
        x.abs_diff(160) <= 6 && y.abs_diff(90) <= 8,
        "centered at {x}, {y}"
    );
    // A quarter of the frame tall: capitals about 0.7 of that.
    assert!((25..=45).contains(&(bottom - top)), "{} tall", bottom - top);
}

#[test]
fn titles_aligned_left_begin_at_the_center() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip t1 gen=title@1 at=0f dur=24f\n\
         \x20     set text=\"KERF\" size=0.2 align=left x=0.1\n"
    );
    let frame = frame(&body, 0, false);
    let (left, ..) = bright(&frame, 200).expect("the title shows");
    // The center, moved a tenth of the frame right.
    assert!(left.abs_diff(192) <= 6, "begins at {left}");
}

#[test]
fn captions_are_burned_in_only_when_asked() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track S1 subtitle\n\
         \x20   clip c1 caption=\"Hello there\" at=0f dur=24f\n"
    );
    let shown = frame(&body, 3, true);
    let (_, top, _, bottom) = bright(&shown, 200).expect("the caption shows");
    assert!(top > 140 && bottom < 180, "at rows {top}..{bottom}");
    assert!(bright(&frame(&body, 3, false), 200).is_none());
    // Not after it ends.
    assert!(bright(&frame(&body, 30, true), 200).is_none());
}

#[test]
fn blend_modes_combine_with_the_layers_below() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#808080\n\
         \x20 track V2 video\n\
         \x20   clip g2 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ff8000 blend=multiply\n\
         \x20 track V3 video\n\
         \x20   clip g3 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#202020 blend=screen scale=0.5\n"
    );
    let frame = frame(&body, 0, false);
    let multiplied = pixel(&frame, 10, 10);
    assert!(
        multiplied[0].abs_diff(128) <= 2 && multiplied[1].abs_diff(64) <= 2 && multiplied[2] <= 2,
        "{multiplied:?}"
    );
    // Screen lightens: 1 - (1 - a)(1 - b).
    let screened = pixel(&frame, 160, 90);
    assert!(screened[0].abs_diff(144) <= 3, "{screened:?}");
}

#[test]
fn adjustment_layers_take_raster_effects() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ff0000\n\
         \x20 track V2 video\n\
         \x20   clip a1 gen=adjustment@1 at=0f dur=12f\n\
         \x20     fx color.huesaturation saturation=-100\n"
    );
    // Red is grey while the adjustment layer lasts.
    let [r, g, b] = pixel(&frame(&body, 4, false), 50, 50);
    assert!(
        r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1 && r > 20,
        "{r} {g} {b}"
    );
    assert_eq!(pixel(&frame(&body, 16, false), 50, 50), [255, 0, 0]);
}

#[test]
fn time_effects_read_the_clips_other_frames() {
    // A black solid inverted more and more, to white at the end.
    let fading = |fx: &str| {
        format!(
            "{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
             \x20     set color=#000000\n\
             \x20     fx channel.invert blend={{0f:100 23f:0}}\n\
             {fx}"
        )
    };
    // Halfway through it is grey; held at one frame a second, it is still
    // the black it starts as.
    let at = |body: &str| pixel(&frame(body, 12, false), 50, 50)[0];
    let plain = at(&fading(""));
    assert!((100..=200).contains(&plain), "{plain}");
    assert!(at(&fading("\x20     fx time.posterizetime rate=1\n")) <= 8);
}

#[test]
fn effects_read_other_tracks_even_hidden_ones() {
    // A red solid takes its matte from a small white solid on a hidden
    // track: red where the matte is, nothing elsewhere, and no white.
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ff0000\n\
         \x20     fx key.trackmatte matte=V2\n\
         \x20 track V2 video enabled=false\n\
         \x20   clip m1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ffffff scale=0.5\n"
    );
    let frame = frame(&body, 4, false);
    assert_eq!(pixel(&frame, 160, 90), [255, 0, 0]);
    assert_eq!(pixel(&frame, 10, 10), [0, 0, 0]);
}

#[test]
fn masks_cut_the_picture_and_effects_draw_along_them() {
    let solid = |lines: &str| {
        format!(
            "{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
             \x20     set color=#ff0000\n\
             {lines}"
        )
    };
    // A rectangle keeps the middle; outside it the black below shows.
    let kept = frame(
        &solid("\x20     mask k1 rect=0.25,0.25,0.75,0.75\n"),
        4,
        false,
    );
    assert_eq!(pixel(&kept, 160, 90), [255, 0, 0]);
    assert_eq!(pixel(&kept, 10, 10), [0, 0, 0]);
    // Inverted, it cuts the middle out instead.
    let cut = frame(
        &solid("\x20     mask k1 rect=0.25,0.25,0.75,0.75 inverted=true\n"),
        4,
        false,
    );
    assert_eq!(pixel(&cut, 160, 90), [0, 0, 0]);
    assert_eq!(pixel(&cut, 10, 10), [255, 0, 0]);
    // A crop intersects the masks: cropping the left half away leaves
    // the right half (centered), which a mask keeping only the left
    // half empties.
    let cropped = frame(&solid("\x20     set crop_l=0.5\n"), 4, false);
    assert_eq!(pixel(&cropped, 160, 90), [255, 0, 0]);
    let both = frame(
        &solid("\x20     set crop_l=0.5\n\x20     mask k1 rect=0,0,0.5,1\n"),
        4,
        false,
    );
    assert_eq!(pixel(&both, 160, 90), [0, 0, 0]);
    // A mask that cuts nothing is only a path, for effects to read.
    let path = frame(
        &solid("\x20     mask ring rect=0.25,0.25,0.75,0.75 mode=none\n"),
        4,
        false,
    );
    assert_eq!(pixel(&path, 10, 10), [255, 0, 0]);
}

#[test]
fn unsupported_things_are_named() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip a1 gen=adjustment@1 at=0f dur=24f\n\
         \x20     fx color.basic exposure=1\n\
         \x20   clip g1 gen=solid@1 at=24f dur=24f\n\
         \x20   transition x1 from=a1 to=g1 fx=dissolve@1 dur=6f\n"
    );
    match compositor(&body, false) {
        Err(error @ ComposeError::Unsupported { .. }) => {
            assert_eq!(error.entity(), Some("a1"), "{error}");
        }
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("a dissolve out of an adjustment layer draws"),
    }
}

#[test]
fn transitions_dips_and_adjustments_mix_in_linear_light() {
    let at = |body: &str, index: i64| pixel(&frame(body, index, false), 160, 90);
    let cut = |transition: &str| {
        format!(
            "{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
             \x20     set color=#ff0000\n\
             \x20   clip g2 gen=solid@1 at=24f dur=24f\n\
             \x20     set color=#0000ff\n\
             {transition}"
        )
    };
    // A dissolve mixes the light of the two halfway through: in linear
    // light, so each half is brighter than the encoded mean.
    let dissolve = cut("\x20   transition x1 from=g1 to=g2 fx=dissolve@1 dur=12f\n");
    assert_eq!(at(&dissolve, 17), [255, 0, 0]);
    let [r, g, b] = at(&dissolve, 24);
    assert!(r > 170 && b > 170 && g == 0, "{r} {g} {b}");
    assert_eq!(at(&dissolve, 31), [0, 0, 255]);
    // A dip goes through black at the cut.
    let dip = cut("\x20   transition x1 from=g1 to=g2 fx=dip@1 dur=12f\n");
    assert_eq!(at(&dip, 24), [0, 0, 0]);
    // A wipe from the left shows the incoming clip on the left first.
    let wipe = cut("\x20   transition x1 from=g1 to=g2 fx=wipe@1 dur=12f\n");
    let middle = frame(&wipe, 24, false);
    assert_eq!(pixel(&middle, 40, 90), [0, 0, 255]);
    assert_eq!(pixel(&middle, 280, 90), [255, 0, 0]);
    // An adjustment layer's effects apply to what is beneath it, inside its
    // frame.
    let adjusted = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#404040\n\
         \x20 track V2 video\n\
         \x20   clip a1 gen=adjustment@1 at=0f dur=24f\n\
         \x20     set scale=0.5\n\
         \x20     fx color.basic@1 exposure=1\n"
    );
    let frame = frame(&adjusted, 4, false);
    assert_eq!(pixel(&frame, 160, 90), [128, 128, 128]);
    assert_eq!(pixel(&frame, 10, 10), [64, 64, 64]);
}

#[test]
fn filmcraft_transitions_mix_both_clips_across_the_cut() {
    let cut = |transition: &str| {
        format!(
            "{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
             \x20     set color=#ff0000\n\
             \x20   clip g2 gen=solid@1 at=24f dur=24f\n\
             \x20     set color=#0000ff\n\
             {transition}"
        )
    };
    // A push from the west halfway through: the incoming clip fills the
    // left half, the outgoing one is pushed into the right.
    let push = cut("\x20   transition x1 from=g1 to=g2 fx=push@1 dur=12f motion_blur=0\n");
    let middle = frame(&push, 24, false);
    assert_eq!(pixel(&middle, 40, 90), [0, 0, 255]);
    assert_eq!(pixel(&middle, 280, 90), [255, 0, 0]);
    // Before and after, each clip alone.
    assert_eq!(pixel(&frame(&push, 16, false), 160, 90), [255, 0, 0]);
    assert_eq!(pixel(&frame(&push, 31, false), 160, 90), [0, 0, 255]);
    // From the east, the other way round.
    let east = cut(
        "\x20   transition x1 from=g1 to=g2 fx=push@1 dur=12f motion_blur=0 direction=from-east\n",
    );
    let middle = frame(&east, 24, false);
    assert_eq!(pixel(&middle, 280, 90), [0, 0, 255]);
    assert_eq!(pixel(&middle, 40, 90), [255, 0, 0]);
    // An iris opens the incoming clip from the middle.
    let iris = cut("\x20   transition x1 from=g1 to=g2 fx=iris-round@1 dur=12f\n");
    let opening = frame(&iris, 22, false);
    assert_eq!(pixel(&opening, 160, 90), [0, 0, 255]);
    assert_eq!(pixel(&opening, 2, 2), [255, 0, 0]);
}

#[test]
fn filmcraft_effects_run_on_kerfs_renderer() {
    let solid = |lines: &str| {
        format!(
            "{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
             \x20     set color=#ff0000\n\
             {lines}"
        )
    };
    let at = |body: &str, x: u32, y: u32| pixel(&frame(body, 4, false), x, y);
    // Red inverted is cyan.
    let inverted = solid("\x20     fx channel.invert\n");
    assert_eq!(at(&inverted, 160, 90), [0, 255, 255]);
    // A point is a place across the picture.
    let circled = solid("\x20     fx generate.circle center_x=0.25 radius=20 color=#00ff00\n");
    assert_eq!(at(&circled, 80, 90), [0, 255, 0]);
    assert_eq!(at(&circled, 240, 90), [255, 0, 0]);
    // On an adjustment layer, the effect changes what is beneath it.
    let adjusted = solid(
        "\x20 track V2 video\n\
         \x20   clip a1 gen=adjustment@1 at=0f dur=24f\n\
         \x20     fx channel.invert\n",
    );
    assert_eq!(at(&adjusted, 160, 90), [0, 255, 255]);
}

#[test]
fn kept_effectcraft_effects_run_on_kerfs_renderer_beside_filmcrafts() {
    let body = |lines: &str| {
        format!(
            "{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
             \x20     set color=#ff0000\n\
             {lines}"
        )
    };
    let at = |body: &str| pixel(&frame(body, 4, false), 160, 90);
    // Hue/Saturation, kept from EffectCraft, desaturates red to grey.
    let grey = body("\x20     fx color.huesaturation saturation=-100\n");
    let [r, g, b] = at(&grey);
    assert!(
        r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1 && r > 20,
        "{r} {g} {b}"
    );
    // With FilmCraft's Invert after it.
    let both = body(
        "\x20     fx color.huesaturation saturation=-100\n\
         \x20     fx channel.invert\n",
    );
    let inverted = at(&both);
    assert!(
        inverted.iter().all(|c| c.abs_diff(255 - r) <= 2),
        "{inverted:?} after {r}"
    );
}

#[test]
fn looks_draw_as_the_effects_they_stand_for() {
    // A flash at frame 6: the grey solid is brighter just after the hit
    // than before it, and back by the end of the fade.
    let flashed = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#404040\n\
         \x20     fx look.flash@1 hit=6f fade=6f\n"
    );
    let at = |body: &str, index: i64| pixel(&frame(body, index, false), 160, 90)[0];
    assert_eq!(at(&flashed, 3), 64);
    assert!(at(&flashed, 7) > 150, "{}", at(&flashed, 7));
    assert_eq!(at(&flashed, 20), 64);
    // A whip across a cut: halfway, the outgoing clip has moved off to the
    // left and the incoming one comes in from the right, over it.
    let whipped = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ff0000\n\
         \x20   clip g2 gen=solid@1 at=24f dur=24f\n\
         \x20     set color=#0000ff\n\
         \x20   transition x1 from=g1 to=g2 fx=look.whip@1 dur=12f\n"
    );
    let before = pixel(&frame(&whipped, 16, false), 160, 90);
    assert_eq!(before, [255, 0, 0]);
    let after = pixel(&frame(&whipped, 31, false), 160, 90);
    assert_eq!(after, [0, 0, 255]);
    let middle = frame(&whipped, 24, false);
    let (left, right) = (pixel(&middle, 20, 90), pixel(&middle, 300, 90));
    assert!(
        right[2] > right[0],
        "the incoming clip enters from the right: {right:?}"
    );
    assert!(left[0] >= left[2], "{left:?}");
}

#[test]
fn a_filled_area_takes_the_picture_around_it() {
    // A white square on a gray solid: the fill's mask, which cuts nothing, covers
    // all of the square.
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip b1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#404040\n\
         \x20 track V2 video\n\
         \x20   clip t1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ffffff\n\
         \x20     mask k9 rect=0.4,0.4,0.6,0.6\n\
         \x20 track V3 video\n\
         \x20   clip a1 gen=adjustment@1 at=0f dur=24f\n\
         \x20     mask k1 rect=0.3,0.3,0.7,0.7 mode=none\n\
         \x20     fx image.fill@1 path=k1 method=smooth\n"
    );
    let center = |body: &str| pixel(&frame(body, 6, false), 160, 90);
    assert!(
        center(&body).iter().all(|channel| *channel < 96),
        "{:?}",
        center(&body)
    );
    // Naming no mask, it leaves the picture alone.
    let unfilled = body.replace("path=k1 ", "");
    assert!(center(&unfilled).iter().all(|channel| *channel > 200));
}
