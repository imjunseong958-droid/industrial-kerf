//! [`Kernel::run`]: the passes each kernel takes on the GPU, and [`Kernel::held`] and
//! [`Kernel::in_gamut`], which bring a picture to where an effect written for less works on it and
//! put it back.

use super::stage::{stage, Stage, U};
use super::*;
use crate::video::effects::gaussian_boxes;
use crate::video::gpu::run as run_ops;
use kerf_draw::blend::Blend;
use kerf_geom::Rect;
use kerf_gpu::{wgpu, Frame, Picture};

/// `hash_u`'s constant on its seed.
const HASH_SEED: u64 = 0x27D4_EB2F_1656_67C5;

impl Kernel {
    /// `effect` on a picture brought under 1 and put back, as a picture holding values over 1
    /// needs for an effect written for 0 to 1: `src` is the picture, and `effect` runs on it with
    /// its pixels each under 1.
    pub fn held(
        frame: &mut Frame<'static>,
        picture: Picture,
        effect: impl FnOnce(&mut Frame<'static>, Picture) -> Option<Picture>,
    ) -> Option<Picture> {
        let stage = stage(frame.gpu())?;
        let unit = stage.pass(frame, &stage.to_unit, &picture, None, &U::default());
        let done = effect(frame, unit)?;
        let out = stage.pass(
            frame,
            &stage.from_unit,
            &done,
            Some(&picture),
            &U::default(),
        );
        frame.release(done);
        frame.release(picture);
        Some(out)
    }

    /// `effect` on a picture brought inside BT.709 and put back: `Image::to_gamut` and its way back.
    pub fn in_gamut(
        frame: &mut Frame<'static>,
        picture: Picture,
        effect: impl FnOnce(&mut Frame<'static>, Picture) -> Option<Picture>,
    ) -> Option<Picture> {
        let stage = stage(frame.gpu())?;
        let inside = stage.pass(frame, &stage.to_gamut, &picture, None, &U::default());
        let done = effect(frame, inside)?;
        let out = stage.pass(
            frame,
            &stage.from_gamut,
            &done,
            Some(&picture),
            &U::default(),
        );
        frame.release(done);
        frame.release(picture);
        Some(out)
    }

    /// The kernel on `picture`, or `None` when the kernels did not build.
    pub fn run(&self, frame: &mut Frame<'static>, picture: Picture) -> Option<Picture> {
        let stage = stage(frame.gpu())?;
        Some(match self {
            Kernel::Nothing => picture,
            Kernel::Noise {
                amount,
                grain,
                colored,
                clip,
                hashes,
            } => {
                let numbers = U::noise(*amount, *grain, *colored, *clip, *hashes);
                one_pass(stage, frame, picture, &stage.noise, &numbers)
            }
            Kernel::ColorKey {
                key,
                tolerance,
                softness,
            } => {
                let numbers = U::color_key(*key, *tolerance, *softness);
                one_pass(stage, frame, picture, &stage.color_key, &numbers)
            }
            Kernel::LumaKey { threshold, cutoff } => {
                let numbers = U::floats([*threshold, *cutoff, 0.0, 0.0]);
                one_pass(stage, frame, picture, &stage.luma_key, &numbers)
            }
            Kernel::LogoCutout {
                background,
                threshold,
                softness,
                unmultiply,
                invert,
            } => {
                let numbers =
                    U::logo_cutout(*background, *threshold, *softness, *unmultiply, *invert);
                one_pass(stage, frame, picture, &stage.logo_cutout, &numbers)
            }
            Kernel::UltraKey(key) => ultra_key(stage, frame, picture, key)?,
            Kernel::RadialZoom {
                kind,
                center,
                zoom,
                arc,
                quality,
            } => {
                let numbers = U::radial_zoom(*kind, *center, *zoom, *arc, *quality);
                encoded_pass(stage, frame, picture, &stage.radial_zoom, &numbers)
            }
            Kernel::Lumetri(lumetri) => return lumetri.run(frame, picture),
            Kernel::Grain(grain) => {
                let numbers = U::grain(grain);
                encoded_pass(stage, frame, picture, &stage.add_grain, &numbers)
            }
            Kernel::CornerPin { map, size } => {
                let numbers = U::corner_pin(*map, *size);
                one_pass(stage, frame, picture, &stage.corner_pin, &numbers)
            }
            Kernel::DropShadow {
                offset,
                color,
                opacity,
                blur,
                only,
            } => {
                let shadow = [color[0], color[1], color[2], *opacity];
                drop_shadow(stage, frame, picture, *offset, shadow, blur.as_ref(), *only)?
            }
            Kernel::Mosaic {
                block,
                blocks,
                soft,
            } => mosaic(stage, frame, picture, *block, *blocks, *soft),
            Kernel::Kept(kept) => kept_color(stage, frame, picture, kept),
            Kernel::KeyCleaner {
                around,
                strength,
                contrast,
            } => key_cleaner(stage, frame, picture, around, *strength, *contrast)?,
            Kernel::Choke {
                radius,
                between,
                choked,
                matte,
            } => choke(stage, frame, picture, *radius, *between, *choked, *matte),
            Kernel::Optics {
                center,
                fit,
                focal,
                reach,
                half,
                reverse,
            } => {
                let numbers = U::optics(*center, *fit, *focal, *reach, *half, *reverse);
                encoded_pass(stage, frame, picture, &stage.optics, &numbers)
            }
            Kernel::Sphere {
                turn,
                share,
                layout,
            } => {
                let numbers = U::sphere(*turn, *share, *layout);
                one_pass(stage, frame, picture, &stage.sphere, &numbers)
            }
            Kernel::PlaneToSphere {
                turn,
                half,
                feather,
                layout,
            } => {
                let numbers = U::plane_to_sphere(*turn, *half, *feather, *layout);
                one_pass(stage, frame, picture, &stage.plane_to_sphere, &numbers)
            }
            Kernel::Lens {
                center,
                reach,
                curvature,
            } => {
                let numbers = U::floats([center[0], center[1], *reach, *curvature]);
                one_pass(stage, frame, picture, &stage.lens, &numbers)
            }
            Kernel::FourColors(told) => {
                let numbers = U::four_colors(told);
                one_pass(stage, frame, picture, &stage.four_colors, &numbers)
            }
            Kernel::Grid(told) => {
                let numbers = U::grid(told);
                one_pass(stage, frame, picture, &stage.grid, &numbers)
            }
            Kernel::Ramp {
                start,
                along,
                length2,
                colors,
                radial,
                opacity,
            } => {
                let numbers = U::ramp(*start, *along, *length2, *colors, *radial, *opacity);
                one_pass(stage, frame, picture, &stage.ramp, &numbers)
            }
            Kernel::Gradient(told) => {
                let numbers = U::gradient(told);
                one_pass(stage, frame, picture, &stage.gradient, &numbers)
            }
            Kernel::Checkerboard {
                anchor,
                cell,
                feather,
                color,
                opacity,
                mode,
            } => {
                let numbers = U::checkerboard(*anchor, *cell, *feather, *color, *opacity, *mode);
                one_pass(stage, frame, picture, &stage.checkerboard, &numbers)
            }
            Kernel::Cineon { way, told } => {
                let numbers = U::cineon(*way, told);
                one_pass(stage, frame, picture, &stage.cineon, &numbers)
            }
        })
    }
}

/// One pass of `pipeline` on `picture`, which it releases.
fn one_pass(
    stage: &Stage,
    frame: &mut Frame<'static>,
    picture: Picture,
    pipeline: &wgpu::ComputePipeline,
    numbers: &U,
) -> Picture {
    let out = stage.pass(frame, pipeline, &picture, None, numbers);
    frame.release(picture);
    out
}

/// One pass of `pipeline` on `picture` encoded as a kept effect's picture is, decoded after it.
fn encoded_pass(
    stage: &Stage,
    frame: &mut Frame<'static>,
    picture: Picture,
    pipeline: &wgpu::ComputePipeline,
    numbers: &U,
) -> Picture {
    let encoded = stage.pass(frame, &stage.encode_kept, &picture, None, &U::default());
    frame.release(picture);
    let done = stage.pass(frame, pipeline, &encoded, None, numbers);
    frame.release(encoded);
    let out = stage.pass(frame, &stage.decode_kept, &done, None, &U::default());
    frame.release(done);
    out
}

/// Ultra Key, its matte choked and softened first when it is told to be.
fn ultra_key(
    stage: &Stage,
    frame: &mut Frame<'static>,
    picture: Picture,
    key: &UltraKey,
) -> Option<Picture> {
    let numbers = U {
        a: [key.output, key.dominant, u32::from(key.corrects), 0],
        p0: [key.key[0], key.key[1], key.key[2], key.key_size],
        p1: [key.gain, key.highlight, key.shadow, key.tolerance],
        p2: [key.pedestal, key.contrast, key.mid, key.spill],
        p3: [key.desaturate, key.range, key.spill_luma, 0.0],
        p4: [key.correction[0], key.correction[1], key.correction[2], 0.0],
        ..U::default()
    };
    let out = if key.choke > 0.0 || key.soften > 0.0 {
        let mut matte = stage.pass(frame, &stage.ultra_matte, &picture, None, &numbers);
        if key.choke > 0.0 {
            let copy = frame.copy(&matte);
            let sigma = (key.choke * 4.0 * key.px_scale).max(0.5);
            let blurred = blur_plane(frame, copy, sigma)?;
            let choked = stage.pass(
                frame,
                &stage.ultra_choke,
                &matte,
                Some(&blurred),
                &U::floats([key.choke * 0.5, 0.0, 0.0, 0.0]),
            );
            frame.release(blurred);
            frame.release(matte);
            matte = choked;
        }
        if key.soften > 0.0 {
            matte = blur_plane(frame, matte, key.soften * 10.0 * key.px_scale)?;
        }
        let out = stage.pass(frame, &stage.ultra_keyed, &picture, Some(&matte), &numbers);
        frame.release(matte);
        out
    } else {
        stage.pass(frame, &stage.ultra_key, &picture, None, &numbers)
    };
    frame.release(picture);
    Some(out)
}

/// Drop Shadow: the picture moved by `offset` and made a shadow of `shadow`'s color and opacity,
/// softened by `blur` when it is, with the picture over it unless `only` the shadow is drawn.
fn drop_shadow(
    stage: &Stage,
    frame: &mut Frame<'static>,
    picture: Picture,
    offset: [f64; 2],
    shadow: [f32; 4],
    blur: Option<&FxOp>,
    only: bool,
) -> Option<Picture> {
    let (w, h) = (picture.w, picture.h);
    let whole = Rect::new(0.0, 0.0, f64::from(w), f64::from(h));
    let by = Affine::translate(offset[0], offset[1]);
    let empty = frame.clear(w, h);
    let moved = frame.composite(empty, &picture, &by, whole, 1.0, Blend::Normal);
    let mut cast = stage.pass(frame, &stage.shadow, &moved, None, &U::floats(shadow));
    frame.release(moved);
    if let Some(blur) = blur {
        cast = run_ops(frame, cast, std::slice::from_ref(blur))?;
    }
    if !only {
        let over = &Affine::IDENTITY;
        cast = frame.composite(cast, &picture, over, whole, 1.0, Blend::Normal);
    }
    frame.release(picture);
    Some(cast)
}

/// Mosaic, of `blocks` blocks each `block` in size.
fn mosaic(
    stage: &Stage,
    frame: &mut Frame<'static>,
    picture: Picture,
    block: [f32; 2],
    blocks: [u32; 2],
    soft: f32,
) -> Picture {
    let numbers = U::floats([block[0], block[1], soft, 0.0]);
    // A picture with a pixel to each block, which every pixel of the block then reads.
    let small = frame.take(blocks[0], blocks[1]);
    frame.dispatch(
        &stage.mosaic_blocks,
        &stage.layout,
        &numbers.bytes(),
        (&picture.view, &small.view, None),
        (blocks[0].div_ceil(16), blocks[1].div_ceil(16)),
    );
    let out = stage.pass(frame, &stage.mosaic, &picture, Some(&small), &numbers);
    frame.release(small);
    frame.release(picture);
    out
}

/// A color effect kept from EffectCraft, in its one pass, with its table when it reads one.
fn kept_color(
    stage: &Stage,
    frame: &mut Frame<'static>,
    picture: Picture,
    kept: &KeptColor,
) -> Picture {
    let (pipeline, table, numbers) = kept_numbers(stage, kept);
    let table = table.map(|table| frame.image(&table));
    let out = stage.pass(frame, pipeline, &picture, table.as_ref(), &numbers);
    if let Some(table) = table {
        frame.release(table);
    }
    frame.release(picture);
    out
}

/// The pass a kept color effect takes, the table it reads if it reads one, and its numbers.
fn kept_numbers<'s>(
    stage: &'s Stage,
    kept: &KeptColor,
) -> (&'s wgpu::ComputePipeline, Option<Image>, U) {
    match kept {
        KeptColor::Through => (&stage.kept_through, None, U::default()),
        KeptColor::PhotoFilter(told) => {
            let [r, g, b] = told.color;
            let numbers = U {
                p0: [r, g, b, told.density],
                p1: [told.keep, 0.0, 0.0, 0.0],
                ..U::default()
            };
            (&stage.photo_filter, None, numbers)
        }
        KeptColor::Unmult(told) => {
            let numbers = U {
                p0: [told.white, told.start, told.width, told.decontaminate],
                p1: [told.clip, 0.0, 0.0, 0.0],
                ..U::default()
            };
            (&stage.unmult, None, numbers)
        }
        KeptColor::Curves { curved, table } => {
            let table = Image {
                w: KEPT_CURVE_STEPS + 1,
                h: 4,
                px: table.clone(),
            };
            let numbers = U {
                a: [*curved, 0, 0, 0],
                ..U::default()
            };
            (&stage.curves, Some(table), numbers)
        }
        KeptColor::HueSaturation {
            master,
            colorize,
            ranges,
        } => {
            // A row to a range: where it is, then what it does.
            let mut table = Image::new(2, ranges.len().max(1));
            for (range, row) in ranges.iter().zip(table.px.as_chunks_mut::<8>().0) {
                *row = [
                    range.start,
                    range.end,
                    range.fall_start,
                    range.fall_end,
                    range.hue,
                    range.sat,
                    range.light,
                    0.0,
                ];
            }
            let [h, s, l] = colorize.unwrap_or_default();
            let numbers = U {
                a: [u32::from(colorize.is_some()), ranges.len() as u32, 0, 0],
                p0: [master[0], master[1], master[2], 0.0],
                p1: [h, s, l, 0.0],
                ..U::default()
            };
            (&stage.hue_saturation, Some(table), numbers)
        }
        KeptColor::SelectiveColor { take, relative } => {
            let table = Image {
                w: 9,
                h: 1,
                px: take.iter().flatten().copied().collect(),
            };
            let numbers = U {
                a: [u32::from(*relative), 0, 0, 0],
                ..U::default()
            };
            (&stage.selective_color, Some(table), numbers)
        }
        KeptColor::Spill {
            primary,
            amount,
            range,
            luma,
            key_hue,
            tolerance,
            desaturate,
            neutral,
        } => {
            let numbers = U {
                a: [*primary, 0, 0, 0],
                p0: [*amount, *range, *luma, *key_hue],
                p1: [*tolerance, *desaturate, *neutral, 0.0],
                ..U::default()
            };
            (&stage.spill, None, numbers)
        }
    }
}

/// The Key Cleaner: the colors around each pixel, where it looks for any, and the key's edge
/// cleaned up with them.
fn key_cleaner(
    stage: &Stage,
    frame: &mut Frame<'static>,
    picture: Picture,
    around: &Option<Vec<u32>>,
    strength: f32,
    contrast: f32,
) -> Option<Picture> {
    let encoded = stage.pass(frame, &stage.encode_kept, &picture, None, &U::default());
    let around = match around {
        Some(radii) => {
            let again = stage.pass(frame, &stage.encode_kept, &picture, None, &U::default());
            if radii.iter().all(|radius| *radius == 0) {
                Some(again)
            } else {
                Some(run_ops(
                    frame,
                    again,
                    &[FxOp::Gaussian {
                        rx: radii.clone(),
                        ry: radii.clone(),
                        repeat: true,
                    }],
                )?)
            }
        }
        None => None,
    };
    frame.release(picture);
    let cleaned = stage.pass(
        frame,
        &stage.key_clean,
        &encoded,
        around.as_ref(),
        &U {
            a: [u32::from(around.is_some()), 0, 0, 0],
            p0: [strength, contrast, 0.0, 0.0],
            ..U::default()
        },
    );
    if let Some(around) = around {
        frame.release(around);
    }
    frame.release(encoded);
    let out = stage.pass(frame, &stage.decode_kept, &cleaned, None, &U::default());
    frame.release(cleaned);
    Some(out)
}

/// The Simple Choker: its matte drawn in across, then down, then put on the picture.
fn choke(
    stage: &Stage,
    frame: &mut Frame<'static>,
    picture: Picture,
    radius: u32,
    between: f32,
    choked: bool,
    matte: bool,
) -> Picture {
    let numbers = U {
        a: [radius, u32::from(choked), u32::from(matte), 0],
        p0: [between, 0.0, 0.0, 0.0],
        ..U::default()
    };
    let across = stage.pass(frame, &stage.choke_rows, &picture, None, &numbers);
    let down = stage.pass(frame, &stage.choke_columns, &across, None, &numbers);
    frame.release(across);
    let out = stage.pass(frame, &stage.choke_put, &picture, Some(&down), &numbers);
    frame.release(down);
    frame.release(picture);
    out
}

/// `blur_plane` of `vfx`, with its plane in the red of a picture.
fn blur_plane(frame: &mut Frame<'static>, plane: Picture, sigma: f32) -> Option<Picture> {
    if sigma <= 0.3 {
        return Some(plane);
    }
    let (rx, ry) = gaussian_boxes(plane.w as usize, plane.h as usize, sigma, sigma);
    run_ops(
        frame,
        plane,
        &[FxOp::Gaussian {
            rx,
            ry,
            repeat: false,
        }],
    )
}

/// The numbers of the kernels drawn in one pass, as each one's entry point in `kernels.wgsl` reads
/// them.
impl U {
    fn noise(amount: f32, grain: f32, colored: bool, clip: bool, hashes: [u64; 3]) -> Self {
        let words = |hash: u64| [hash as u32, (hash >> 32) as u32];
        let [z0, z1, z2] = hashes.map(words);
        U {
            a: [u32::from(colored), u32::from(clip), z0[0], z0[1]],
            b: [z1[0], z1[1], z2[0], z2[1]],
            p0: [amount, grain, 0.0, 0.0],
            ..U::default()
        }
    }

    fn color_key(key: [f32; 3], tolerance: f32, softness: f32) -> Self {
        U {
            p0: [key[0], key[1], key[2], tolerance],
            p1: [softness, 0.0, 0.0, 0.0],
            ..U::default()
        }
    }

    fn logo_cutout(
        background: [f32; 3],
        threshold: f32,
        softness: f32,
        unmultiply: bool,
        invert: bool,
    ) -> Self {
        U {
            a: [u32::from(unmultiply), u32::from(invert), 0, 0],
            p0: [background[0], background[1], background[2], 0.0],
            p1: [threshold, softness, 0.0, 0.0],
            ..U::default()
        }
    }

    fn radial_zoom(kind: u32, center: [f32; 2], zoom: f32, arc: f32, quality: f32) -> Self {
        U {
            a: [kind, 0, 0, 0],
            p0: [center[0], center[1], zoom, arc],
            p1: [quality, 0.0, 0.0, 0.0],
            ..U::default()
        }
    }

    fn grain(grain: &Grain) -> Self {
        U {
            a: [grain.mode, u32::from(grain.mono), 0, 0],
            b: [grain.seeds[0], grain.seeds[1], grain.seeds[2], 0],
            p0: [grain.gains[0], grain.gains[1], grain.gains[2], grain.aspect],
            p1: [grain.sizes[0], grain.sizes[1], grain.sizes[2], grain.frame],
            p2: [
                grain.tones[0],
                grain.tones[1],
                grain.tones[2],
                grain.midpoint,
            ],
            p3: [grain.saturation, 0.0, 0.0, 0.0],
            ..U::default()
        }
    }

    fn corner_pin(map: [f32; 9], size: [f32; 2]) -> Self {
        U {
            p0: [map[0], map[1], map[2], map[3]],
            p1: [map[4], map[5], map[6], map[7]],
            p2: [map[8], size[0], size[1], 0.0],
            ..U::default()
        }
    }

    fn optics(
        center: [f32; 2],
        fit: f32,
        focal: f32,
        reach: f32,
        half: f32,
        reverse: bool,
    ) -> Self {
        U {
            a: [u32::from(reverse), 0, 0, 0],
            p0: [center[0], center[1], fit, focal],
            p1: [reach, half, 0.0, 0.0],
            ..U::default()
        }
    }

    fn sphere(turn: [[f32; 3]; 3], share: [f32; 2], layout: u32) -> Self {
        U {
            a: [layout, 0, 0, 0],
            p0: [turn[0][0], turn[0][1], turn[0][2], share[0]],
            p1: [turn[1][0], turn[1][1], turn[1][2], share[1]],
            p2: [turn[2][0], turn[2][1], turn[2][2], 0.0],
            ..U::default()
        }
    }

    fn plane_to_sphere(turn: [[f32; 3]; 3], half: f32, feather: f32, layout: u32) -> Self {
        U {
            a: [layout, 0, 0, 0],
            p0: [turn[0][0], turn[0][1], turn[0][2], half],
            p1: [turn[1][0], turn[1][1], turn[1][2], feather],
            p2: [turn[2][0], turn[2][1], turn[2][2], 0.0],
            ..U::default()
        }
    }

    fn four_colors(told: &kerf_kernel::FourColors<f32>) -> Self {
        let [a, b, c, d] = told.colors;
        U {
            p0: [a[0], a[1], a[2], told.power],
            p1: [b[0], b[1], b[2], told.opacity],
            p2: [c[0], c[1], c[2], told.size[0]],
            p3: [d[0], d[1], d[2], told.size[1]],
            ..U::default()
        }
    }

    fn grid(told: &kerf_kernel::Grid<f32>) -> Self {
        let [r, g, b] = told.color;
        U {
            p0: [told.middle[0], told.middle[1], told.size, told.border],
            p1: [r, g, b, told.opacity],
            ..U::default()
        }
    }

    fn ramp(
        start: [f32; 2],
        along: [f32; 2],
        length2: f32,
        colors: [[f32; 3]; 2],
        radial: bool,
        opacity: f32,
    ) -> Self {
        let [from, to] = colors;
        U {
            a: [u32::from(radial), 0, 0, 0],
            p0: [start[0], start[1], along[0], along[1]],
            p1: [length2, opacity, 0.0, 0.0],
            p2: [from[0], from[1], from[2], 0.0],
            p3: [to[0], to[1], to[2], 0.0],
            ..U::default()
        }
    }

    fn gradient(told: &Gradient) -> Self {
        // What `hash01` scatters by: its seed of 13, by `hash_u`'s constant.
        let seed = 13u64.wrapping_mul(HASH_SEED);
        U {
            a: [told.shape, 0, seed as u32, (seed >> 32) as u32],
            p0: [told.start[0], told.start[1], told.along[0], told.along[1]],
            p1: [told.length2, told.length, told.scatter, told.gamma],
            p2: told.colors[0],
            p3: told.colors[1],
            p4: [told.blend, 0.0, 0.0, 0.0],
            ..U::default()
        }
    }

    fn checkerboard(
        anchor: [f32; 2],
        cell: [f32; 2],
        feather: f32,
        color: [f32; 3],
        opacity: f32,
        mode: u32,
    ) -> Self {
        U {
            a: [mode, 0, 0, 0],
            p0: [anchor[0], anchor[1], cell[0], cell[1]],
            p1: [feather, opacity, 0.0, 0.0],
            p2: [color[0], color[1], color[2], 0.0],
            ..U::default()
        }
    }

    fn cineon(way: u32, told: &kerf_kernel::Cineon<f32>) -> Self {
        U {
            a: [way, 0, 0, 0],
            p0: [told.black, told.white, told.black_inside, told.white_inside],
            p1: [told.gamma, told.rolloff, told.offset, 0.0],
            ..U::default()
        }
    }
}
