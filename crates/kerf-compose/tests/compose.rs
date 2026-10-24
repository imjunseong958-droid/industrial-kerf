//! What the compositor draws where the graph cannot be asked: text,
//! blend modes, and raster effects on adjustment layers.

use kerf_compose::{ComposeError, ComposeOptions, Compositor, Device, Frame};
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
            device: Device::Cpu,
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
         \x20   clip t1 gen=title at=0f dur=24f\n\
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
         \x20   clip t1 gen=title at=0f dur=24f\n\
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
         \x20   clip g1 gen=solid at=0f dur=24f\n\
         \x20     set color=#808080\n\
         \x20 track V2 video\n\
         \x20   clip g2 gen=solid at=0f dur=24f\n\
         \x20     set color=#ff8000 blend=multiply\n\
         \x20 track V3 video\n\
         \x20   clip g3 gen=solid at=0f dur=24f\n\
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
         \x20   clip g1 gen=solid at=0f dur=24f\n\
         \x20     set color=#ff0000\n\
         \x20 track V2 video\n\
         \x20   clip a1 gen=adjustment at=0f dur=12f\n\
         \x20     fx channel.invert\n"
    );
    // Red inverted is cyan while the adjustment layer lasts.
    assert_eq!(pixel(&frame(&body, 4, false), 50, 50), [0, 255, 255]);
    assert_eq!(pixel(&frame(&body, 16, false), 50, 50), [255, 0, 0]);
}

#[test]
fn time_effects_read_the_clips_other_frames() {
    // A black solid inverted more and more, to white at the end.
    let fading = |fx: &str| {
        format!(
            "{SEQUENCE}\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid at=0f dur=24f\n\
             \x20     set color=#000000\n\
             \x20     fx channel.invert blend={{0f:100 23f:0}}\n\
             {fx}"
        )
    };
    // Halfway through it is mid grey; held at one frame a second, it is
    // still the black it starts as.
    let plain = pixel(&frame(&fading(""), 12, false), 50, 50)[0];
    assert!(plain.abs_diff(133) <= 8, "{plain}");
    let held = fading("\x20     fx time.posterizetime frameRate=1\n");
    assert!(pixel(&frame(&held, 12, false), 50, 50)[0] <= 8);
}

#[test]
fn effects_read_other_tracks_even_hidden_ones() {
    // A red solid takes its matte from a small white solid on a hidden
    // track: red where the matte is, nothing elsewhere, and no white.
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid at=0f dur=24f\n\
         \x20     set color=#ff0000\n\
         \x20     fx channel.setmatte takeMatteFromLayer=V2\n\
         \x20 track V2 video enabled=false\n\
         \x20   clip m1 gen=solid at=0f dur=24f\n\
         \x20     set color=#ffffff scale=0.5\n"
    );
    let frame = frame(&body, 4, false);
    assert_eq!(pixel(&frame, 160, 90), [255, 0, 0]);
    assert_eq!(pixel(&frame, 10, 10), [0, 0, 0]);
}

#[test]
fn unsupported_things_are_named() {
    let body = format!(
        "{SEQUENCE}\
         \x20 track V1 video\n\
         \x20   clip a1 gen=adjustment at=0f dur=24f\n\
         \x20     fx channel.invert\n\
         \x20   clip g1 gen=solid at=24f dur=24f\n\
         \x20   transition x1 from=a1 to=g1 fx=dissolve dur=6f\n"
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
             \x20   clip g1 gen=solid at=0f dur=24f\n\
             \x20     set color=#3070b0\n\
             \x20   clip g2 gen=solid at=24f dur=24f\n\
             \x20     set color=#b07030\n\
             \x20   transition x1 from=g1 to=g2 fx=dissolve dur=12f\n\
             \x20 track V2 video\n\
             \x20   clip g3 gen=solid at=0f dur=48f\n\
             \x20     set color=#f0f0f0 scale=0.4 rotation=12 x=0.2 opacity=0.8\n\
             \x20     fx blur.gaussian radius=8\n\
             \x20     fx stylize.glow\n\
             \x20 track V3 video\n\
             \x20   clip t1 gen=title at=0f dur=48f\n\
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
