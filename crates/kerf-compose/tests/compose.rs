//! What the compositor draws where the graph cannot be asked: text,
//! blend modes, and raster effects on adjustment layers.

use kerf_compose::{ComposeError, ComposeOptions, Compositor, Device, Frame, Renderer};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Rate};

/// Both renderers, for what both draw.
const RENDERERS: [Renderer; 2] = [Renderer::EffectCraft, Renderer::Kerf];

fn compositor_by(
    renderer: Renderer,
    body: &str,
    captions: bool,
) -> Result<Compositor, ComposeError> {
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
            device: Device::Cpu,
            renderer,
        },
    )
}

fn frame(body: &str, at: i64, captions: bool) -> Frame {
    frame_by(Renderer::EffectCraft, body, at, captions)
}

fn frame_by(renderer: Renderer, body: &str, at: i64, captions: bool) -> Frame {
    let compositor = compositor_by(renderer, body, captions).expect("composes");
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
    for renderer in RENDERERS {
        let frame = frame_by(renderer, &body, 6, false);
        let (left, top, right, bottom) = bright(&frame, 200).expect("the title shows");
        let (x, y) = ((left + right) / 2, (top + bottom) / 2);
        assert!(
            x.abs_diff(160) <= 6 && y.abs_diff(90) <= 8,
            "{renderer:?}: centered at {x}, {y}"
        );
        // A quarter of the frame tall: capitals about 0.7 of that.
        assert!(
            (25..=45).contains(&(bottom - top)),
            "{renderer:?}: {} tall",
            bottom - top
        );
    }
}

#[test]
fn titles_aligned_left_begin_at_the_center() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip t1 gen=title@1 at=0f dur=24f\n\
         \x20     set text=\"KERF\" size=0.2 align=left x=0.1\n"
    );
    for renderer in RENDERERS {
        let frame = frame_by(renderer, &body, 0, false);
        let (left, ..) = bright(&frame, 200).expect("the title shows");
        // The center, moved a tenth of the frame right.
        assert!(left.abs_diff(192) <= 6, "{renderer:?}: begins at {left}");
    }
}

#[test]
fn captions_are_burned_in_only_when_asked() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track S1 subtitle\n\
         \x20   clip c1 caption=\"Hello there\" at=0f dur=24f\n"
    );
    for renderer in RENDERERS {
        let shown = frame_by(renderer, &body, 3, true);
        let (_, top, _, bottom) = bright(&shown, 200).expect("the caption shows");
        assert!(
            top > 140 && bottom < 180,
            "{renderer:?}: at rows {top}..{bottom}"
        );
        assert!(bright(&frame_by(renderer, &body, 3, false), 200).is_none());
        // Not after it ends.
        assert!(bright(&frame_by(renderer, &body, 30, true), 200).is_none());
    }
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
    for renderer in RENDERERS {
        let frame = frame_by(renderer, &body, 0, false);
        let multiplied = pixel(&frame, 10, 10);
        assert!(
            multiplied[0].abs_diff(128) <= 2
                && multiplied[1].abs_diff(64) <= 2
                && multiplied[2] <= 2,
            "{renderer:?}: {multiplied:?}"
        );
        // Screen lightens: 1 - (1 - a)(1 - b).
        let screened = pixel(&frame, 160, 90);
        assert!(screened[0].abs_diff(144) <= 3, "{renderer:?}: {screened:?}");
    }
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
         \x20     fx stylize.threshold\n"
    );
    // Red is darker than the threshold, so black while the adjustment
    // layer lasts.
    assert_eq!(pixel(&frame(&body, 4, false), 50, 50), [0, 0, 0]);
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
    let at = |body: &str| pixel(&frame_by(Renderer::Kerf, body, 12, false), 50, 50)[0];
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
         \x20     fx channel.setmatte takeMatteFromLayer=V2\n\
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
    for renderer in RENDERERS {
        // A rectangle keeps the middle; outside it the black below shows.
        let kept = frame_by(
            renderer,
            &solid("\x20     mask k1 rect=0.25,0.25,0.75,0.75\n"),
            4,
            false,
        );
        assert_eq!(pixel(&kept, 160, 90), [255, 0, 0], "{renderer:?}");
        assert_eq!(pixel(&kept, 10, 10), [0, 0, 0], "{renderer:?}");
        // Inverted, it cuts the middle out instead.
        let cut = frame_by(
            renderer,
            &solid("\x20     mask k1 rect=0.25,0.25,0.75,0.75 inverted=true\n"),
            4,
            false,
        );
        assert_eq!(pixel(&cut, 160, 90), [0, 0, 0], "{renderer:?}");
        assert_eq!(pixel(&cut, 10, 10), [255, 0, 0], "{renderer:?}");
        // A crop intersects the masks: cropping the left half away leaves
        // the right half (centered), which a mask keeping only the left
        // half empties.
        let cropped = frame_by(renderer, &solid("\x20     set crop_l=0.5\n"), 4, false);
        assert_eq!(pixel(&cropped, 160, 90), [255, 0, 0], "{renderer:?}");
        let both = frame_by(
            renderer,
            &solid("\x20     set crop_l=0.5\n\x20     mask k1 rect=0,0,0.5,1\n"),
            4,
            false,
        );
        assert_eq!(pixel(&both, 160, 90), [0, 0, 0], "{renderer:?}");
    }
    // A mask that cuts nothing is a path: Stroke paints white along it and
    // the red stays everywhere else.
    let stroked = frame(
        &solid(
            "\x20     mask ring rect=0.25,0.25,0.75,0.75 mode=none\n\
             \x20     fx generate.stroke path=ring color=#ffffff brushSize=6\n",
        ),
        4,
        false,
    );
    assert_eq!(pixel(&stroked, 160, 45), [255, 255, 255]);
    assert_eq!(pixel(&stroked, 160, 90), [255, 0, 0]);
    assert_eq!(pixel(&stroked, 10, 10), [255, 0, 0]);
}

#[test]
fn reshape_moves_what_one_mask_holds_into_another() {
    // A white box filled into the red, then morphed from its own outline
    // to one further right, inside a boundary around both.
    let boxed = |fx: &str| {
        format!(
            "{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
             \x20     set color=#ff0000\n\
             \x20     mask box rect=0.25,0.3,0.45,0.7 mode=none\n\
             \x20     mask moved rect=0.55,0.3,0.75,0.7 mode=none\n\
             \x20     mask around rect=0.05,0.1,0.95,0.9 mode=none\n\
             \x20     fx generate.fill fillMask=box color=#ffffff\n\
             {fx}"
        )
    };
    let plain = frame(&boxed(""), 4, false);
    assert_eq!(pixel(&plain, 112, 90), [255, 255, 255]);
    assert_eq!(pixel(&plain, 208, 90), [255, 0, 0]);
    let reshaped = boxed(
        "\x20     fx distort.reshape sourceMask=box destinationMask=moved boundaryMask=around\n",
    );
    let reshaped = frame(&reshaped, 4, false);
    // The box fills the destination, edge to edge, and leaves its old place.
    for x in [180, 208, 236] {
        assert_eq!(pixel(&reshaped, x, 90), [255, 255, 255], "x={x}");
    }
    assert_eq!(pixel(&reshaped, 112, 90), [255, 0, 0]);
    let (_, _, right, _) = bright(&reshaped, 200).expect("the box, moved");
    assert!(right <= 242, "{right}");
    // Outside the boundary nothing moves.
    assert_eq!(pixel(&reshaped, 5, 5), [255, 0, 0]);
}

#[test]
fn color_stabilizer_holds_the_reference_frame() {
    // A dark grey solid exposed more and more, so it brightens over time.
    let brightening = |fx: &str| {
        format!(
            "{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
             \x20     set color=#404040\n\
             \x20     fx color.exposure master/exposure={{0f:0 23f:2}}\n\
             {fx}"
        )
    };
    let plain = i32::from(pixel(&frame(&brightening(""), 12, false), 50, 50)[0]);
    let held = brightening("\x20     fx color.colorstabilizer referenceFrame=0\n");
    let held = i32::from(pixel(&frame(&held, 12, false), 50, 50)[0]);
    // Back toward the first frame's grey (64).
    assert!(
        (held - 64).abs() * 2 < (plain - 64).abs(),
        "{held} vs {plain}"
    );
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
    for renderer in RENDERERS {
        match compositor_by(renderer, &body, false) {
            Err(error @ ComposeError::Unsupported { .. }) => {
                assert_eq!(error.entity(), Some("a1"), "{error}");
            }
            Err(other) => panic!("{other}"),
            Ok(_) => panic!("{renderer:?}: a dissolve out of an adjustment layer draws"),
        }
    }
}

#[test]
fn kerf_draws_transitions_dips_and_adjustments() {
    let at = |body: &str, frame: i64| pixel(&frame_by(Renderer::Kerf, body, frame, false), 160, 90);
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
    let middle = frame_by(Renderer::Kerf, &wipe, 24, false);
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
    let frame = frame_by(Renderer::Kerf, &adjusted, 4, false);
    assert_eq!(pixel(&frame, 160, 90), [128, 128, 128]);
    assert_eq!(pixel(&frame, 10, 10), [64, 64, 64]);
}

#[test]
fn kerf_leaves_raster_effects_to_effectcraft() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
         \x20     fx stylize.glow\n"
    );
    match compositor_by(Renderer::Kerf, &body, false) {
        Err(error @ ComposeError::Unsupported { .. }) => {
            assert_eq!(error.entity(), Some("g1"), "{error}");
            assert!(error.to_string().contains("raster effect"), "{error}");
        }
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("Kerf draws a raster effect"),
    }
    assert!(compositor_by(Renderer::EffectCraft, &body, false).is_ok());
}

#[test]
#[ignore = "opens the GPU, which macOS takes most of a minute to ready for a new build; run with --ignored"]
fn the_gpu_draws_what_the_cpu_draws() {
    // Opening a compositor on the GPU waits for it.
    let probe = Compositor::new(
        &parse_document(
            &format!("kerf 1\nproject name=\"p\"\n{SEQUENCE}"),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project,
        Catalog::builtin(),
        &ComposeOptions {
            device: Device::Gpu,
            ..ComposeOptions::default()
        },
    );
    let Some(adapter) = probe.ok().and_then(|compositor| compositor.gpu()) else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let project = parse_document(
        &format!(
            "kerf 1\nproject name=\"t\"\n{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=24f\n\
             \x20     set color=#3070b0\n\
             \x20   clip g2 gen=solid@1 at=24f dur=24f\n\
             \x20     set color=#b07030\n\
             \x20   transition x1 from=g1 to=g2 fx=dissolve@1 dur=12f\n\
             \x20 track V2 video\n\
             \x20   clip g3 gen=solid@1 at=0f dur=48f\n\
             \x20     set color=#f0f0f0 scale=0.4 rotation=12 x=0.2 opacity=0.8\n\
             \x20     fx blur.gaussian radius=8\n\
             \x20     fx stylize.glow\n\
             \x20 track V3 video\n\
             \x20   clip t1 gen=title@1 at=0f dur=48f\n\
             \x20     set text=\"KERF\" size=0.2 y=0.25\n"
        ),
        Catalog::builtin(),
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let draw = |device| {
        let compositor = Compositor::new(
            &project,
            Catalog::builtin(),
            &ComposeOptions {
                device,
                ..ComposeOptions::default()
            },
        )
        .expect("composes");
        compositor.frame(Rate::FPS_24.time_of(24), 1.0)
    };
    let (cpu, gpu) = (draw(Device::Cpu), draw(Device::Gpu));
    assert_eq!((cpu.width, cpu.height), (gpu.width, gpu.height));
    let total: u64 = cpu
        .rgba
        .iter()
        .zip(&gpu.rgba)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let mean = total as f64 / cpu.rgba.len() as f64;
    let off = cpu
        .rgba
        .chunks_exact(4)
        .zip(gpu.rgba.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 16))
        .count();
    let off = off as f64 / (cpu.rgba.len() / 4) as f64;
    assert!(
        mean < 1.0 && off < 0.002,
        "{adapter}: mean {mean}, off {off}"
    );
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
    let middle = frame_by(Renderer::Kerf, &push, 24, false);
    assert_eq!(pixel(&middle, 40, 90), [0, 0, 255]);
    assert_eq!(pixel(&middle, 280, 90), [255, 0, 0]);
    // Before and after, each clip alone.
    assert_eq!(
        pixel(&frame_by(Renderer::Kerf, &push, 16, false), 160, 90),
        [255, 0, 0]
    );
    assert_eq!(
        pixel(&frame_by(Renderer::Kerf, &push, 31, false), 160, 90),
        [0, 0, 255]
    );
    // From the east, the other way round.
    let east = cut(
        "\x20   transition x1 from=g1 to=g2 fx=push@1 dur=12f motion_blur=0 direction=from-east\n",
    );
    let middle = frame_by(Renderer::Kerf, &east, 24, false);
    assert_eq!(pixel(&middle, 280, 90), [0, 0, 255]);
    assert_eq!(pixel(&middle, 40, 90), [255, 0, 0]);
    // An iris opens the incoming clip from the middle.
    let iris = cut("\x20   transition x1 from=g1 to=g2 fx=iris-round@1 dur=12f\n");
    let opening = frame_by(Renderer::Kerf, &iris, 22, false);
    assert_eq!(pixel(&opening, 160, 90), [0, 0, 255]);
    assert_eq!(pixel(&opening, 2, 2), [255, 0, 0]);
    // The renderer chosen by default draws them; EffectCraft's cannot.
    assert!(
        compositor_by(Renderer::Auto, &push, false).is_ok_and(|c| c.renderer() == Renderer::Kerf)
    );
    match compositor_by(Renderer::EffectCraft, &push, false) {
        Err(error @ ComposeError::Unsupported { .. }) => assert_eq!(error.entity(), Some("x1")),
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("EffectCraft draws a FilmCraft transition"),
    }
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
    let at = |body: &str, x: u32, y: u32| pixel(&frame_by(Renderer::Kerf, body, 4, false), x, y);
    // Red inverted is cyan; the renderer chosen by default draws it, and
    // EffectCraft's cannot.
    let inverted = solid("\x20     fx channel.invert\n");
    assert_eq!(at(&inverted, 160, 90), [0, 255, 255]);
    assert!(compositor_by(Renderer::Auto, &inverted, false)
        .is_ok_and(|c| c.renderer() == Renderer::Kerf));
    match compositor_by(Renderer::EffectCraft, &inverted, false) {
        Err(error @ ComposeError::Unsupported { .. }) => assert_eq!(error.entity(), Some("g1")),
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("EffectCraft draws a FilmCraft effect"),
    }
    // A point is a place across the picture.
    let circled = solid("\x20     fx generate.circle center_x=0.25 radius=20 color=#00ff00\n");
    assert_eq!(at(&circled, 80, 90), [0, 255, 0]);
    assert_eq!(at(&circled, 240, 90), [255, 0, 0]);
    // A track matte reads another track, hidden or not: the red shows only
    // where the small white solid is.
    let matted = solid(
        "\x20     fx key.trackmatte matte=V2\n\
         \x20 track V2 video enabled=false\n\
         \x20   clip m1 gen=solid@1 at=0f dur=24f\n\
         \x20     set color=#ffffff scale=0.5\n",
    );
    assert_eq!(at(&matted, 160, 90), [255, 0, 0]);
    assert_eq!(at(&matted, 10, 10), [0, 0, 0]);
    // On an adjustment layer, the effect changes what is beneath it.
    let adjusted = solid(
        "\x20 track V2 video\n\
         \x20   clip a1 gen=adjustment@1 at=0f dur=24f\n\
         \x20     fx channel.invert\n",
    );
    assert_eq!(at(&adjusted, 160, 90), [0, 255, 255]);
}
