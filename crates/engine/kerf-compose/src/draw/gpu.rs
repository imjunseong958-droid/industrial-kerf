//! A frame drawn on the GPU: the same walk as the CPU's, through the sequence's tracks and each
//! clip's picture, masks, effects, and placing, with the numbers worked out by the same functions.
//! [`Drawing::gpu_can_draw`] decides for a whole frame before anything is decoded, and whatever it
//! refuses (transitions, adjustment layers, effects without a kernel) the CPU draws.

use super::*;
use kerf_fx::filmcraft::catalog::Headroom;
use kerf_fx::filmcraft::gpufx::FxOp;
use kerf_fx::filmcraft::instance::GpuEffect;
use kerf_gpu::{Frame as GpuFrame, Gpu, Picture as GpuPicture};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

/// Frames with fewer pixels than this gain nothing: the CPU draws them before a GPU round trip is
/// over.
const SMALLEST_PIXELS: usize = 640 * 360;

/// The device, once it and every kernel are built.
fn ready_gpu() -> Option<&'static Gpu> {
    kerf_gpu::shared().filter(|gpu| filmcraft::gpu::available(gpu))
}

/// One effect of a clip as a kernel.
enum Op {
    Effect(GpuEffect),
    /// An effect written for 0 to 1 on a picture holding values over 1.
    Held(GpuEffect),
    /// An effect that keeps what is over 1 and not what is below 0.
    Gamut(GpuEffect),
    Correct(Correction),
    /// [`Op::Correct`], written for 0 to 1, on a picture holding values over 1.
    HeldCorrect(Correction),
}

/// What a clip's picture is before its effects, in numbers.
struct Shape {
    /// The size it decodes at or is drawn at.
    source: (usize, usize),
    /// How many of those pixels a source pixel is.
    source_per: f64,
    /// The size its effects see: the canvas when letterboxed.
    size: (usize, usize),
    /// How many of the pixels its effects see a source pixel is.
    per_source: f64,
    to_output: Affine,
}

impl Drawing {
    /// The frame at `at`, when the GPU draws it.
    pub(super) fn gpu_frame(&self, at: Time, scale: f64) -> Option<Frame> {
        let hdr = self.hdr.clone();
        self.gpu_frame_as(at, scale, move |mut frame, canvas| match &hdr {
            Some(output) => {
                let sdr = frame.hdr_output(canvas, output, false);
                frame.finish(&sdr)
            }
            None => frame.finish(canvas),
        })
    }

    /// The frame at `at` at 16 bits a component, when the GPU draws it.
    pub(super) fn gpu_frame16(&self, at: Time, scale: f64) -> Option<Frame> {
        let hdr = self.hdr.clone();
        self.gpu_frame_as(at, scale, move |mut frame, canvas| match &hdr {
            Some(output) => {
                let signal = frame.hdr_output(canvas, output, true);
                frame.finish_signal(&signal)
            }
            None => frame.finish16(canvas),
        })
    }

    fn gpu_frame_as(
        &self,
        at: Time,
        scale: f64,
        finish: impl FnOnce(GpuFrame<'static>, &GpuPicture) -> Option<Vec<u8>>,
    ) -> Option<Frame> {
        let plan = &self.sequences[0];
        let (w, h) = output_size(plan.width, plan.height, scale);
        if self.accel == Accel::Auto && w * h < SMALLEST_PIXELS {
            return None;
        }
        let gpu = self.gpu()?;
        if !self.gpu_can_draw(gpu, 0, at, scale) {
            return None;
        }
        let mut frame = gpu.frame();
        let canvas = self.gpu_draw(&mut frame, 0, at, scale, None)?;
        let rgba = finish(frame, &canvas)?;
        Some(Frame {
            width: w as u32,
            height: h as u32,
            rgba,
        })
    }

    fn gpu(&self) -> Option<&'static Gpu> {
        match self.accel {
            Accel::Never => None,
            Accel::Always => ready_gpu(),
            // Building the kernels takes a second or two, which a viewer should not wait for: the
            // CPU draws until they are ready.
            Accel::Auto => {
                static WARMING: Once = Once::new();
                static READY: AtomicBool = AtomicBool::new(false);
                WARMING.call_once(|| {
                    std::thread::spawn(|| {
                        READY.store(ready_gpu().is_some(), Ordering::Release);
                    });
                });
                if READY.load(Ordering::Acquire) {
                    ready_gpu()
                } else {
                    None
                }
            }
        }
    }

    /// Whether the GPU would draw the frame at `at`.
    pub(crate) fn draws_on_gpu(&self, at: Time, scale: f64) -> bool {
        self.gpu()
            .is_some_and(|gpu| self.gpu_can_draw(gpu, 0, at, scale.clamp(1.0 / 16.0, 1.0)))
    }

    /// Whether the kernels can draw sequence `index` at its time `at`.
    fn gpu_can_draw(&self, gpu: &Gpu, index: usize, at: Time, scale: f64) -> bool {
        self.gpu_can_draw_of(gpu, index, at, scale, None)
    }

    fn gpu_can_draw_of(
        &self,
        gpu: &Gpu,
        index: usize,
        at: Time,
        scale: f64,
        angle: Option<&kerf_model::Id>,
    ) -> bool {
        let plan = &self.sequences[index];
        let (w, h) = output_size(plan.width, plan.height, scale);
        if !gpu.fits(w, h) {
            return false;
        }
        plan.tracks
            .iter()
            .filter(|track| track.drawn(angle))
            .all(|track| self.gpu_can_draw_track(gpu, plan, track, at, scale))
    }

    fn gpu_can_draw_track(
        &self,
        gpu: &Gpu,
        sequence: &SequencePlan,
        track: &TrackPlan,
        at: Time,
        scale: f64,
    ) -> bool {
        track.shown.iter().enumerate().all(|(index, shown)| {
            if !shown.shows(at) {
                return true;
            }
            // A transition mixes two clips with its own arithmetic, on the CPU.
            let mixed = track.pairs.iter().any(|pair| {
                pair.start <= at && at < pair.end && (pair.from == index || pair.to == index)
            });
            if mixed {
                return false;
            }
            let plan = &shown.plan;
            // Frames between frames are made on the CPU, and so are clips in depth.
            if plan.clip.retime != kerf_model::Retime::Frames || plan.three_d {
                return false;
            }
            let time = at - plan.clip.start;
            if let Source::Nested {
                sequence: nested,
                source_in,
                speed,
                duration,
                angle,
            } = &plan.source
            {
                let inner = &self.sequences[*nested];
                let shown = nested_time(time, *source_in, speed, *duration, inner.rate);
                if !self.gpu_can_draw_of(gpu, *nested, shown, scale, angle.as_ref()) {
                    return false;
                }
            }
            let shape = self.gpu_shape(sequence, plan, time, scale);
            gpu.fits(shape.source.0, shape.source.1)
                && self
                    .gpu_ops(sequence, plan, time, at, scale, &shape)
                    .is_some()
        })
    }

    /// The numbers [`Drawing::input`] works out for a clip's picture.
    fn gpu_shape(&self, sequence: &SequencePlan, plan: &ClipPlan, time: Time, scale: f64) -> Shape {
        let canvas = output_size(sequence.width, sequence.height, scale);
        let (source, per_source) = match &plan.source {
            Source::Media { .. } => {
                let factor = plan.shown * scale;
                let w = ((f64::from(plan.size.0) * factor).round() as usize).max(1);
                let h = ((f64::from(plan.size.1) * factor).round() as usize).max(1);
                ((w, h), w as f64 / f64::from(plan.size.0))
            }
            Source::Nested {
                sequence: nested, ..
            } => {
                let inner = &self.sequences[*nested];
                let size = output_size(inner.width, inner.height, scale);
                (size, size.0 as f64 / f64::from(inner.width))
            }
            Source::Solid(_) | Source::Title(_) | Source::Motion(_) => (canvas, scale),
            // It works on the canvas beneath it, unplaced.
            Source::Adjustment | Source::Nothing => {
                return Shape {
                    source: canvas,
                    source_per: scale,
                    size: canvas,
                    per_source: scale,
                    to_output: Affine::IDENTITY,
                }
            }
        };
        let place = Affine::scale(scale, scale).then_apply(&self.geometry(sequence, plan, time));
        if plan.letterboxed {
            Shape {
                source,
                source_per: per_source,
                size: canvas,
                per_source: place.determinant().abs().sqrt(),
                to_output: Affine::IDENTITY,
            }
        } else {
            Shape {
                source,
                source_per: per_source,
                size: source,
                per_source,
                to_output: place.then_apply(&Affine::scale(1.0 / per_source, 1.0 / per_source)),
            }
        }
    }

    /// A clip's effects as kernels, or `None` when any of them has none.
    fn gpu_ops(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        at: Time,
        scale: f64,
        shape: &Shape,
    ) -> Option<Vec<Op>> {
        let mut ops = Vec::with_capacity(plan.effects.len());
        let density = scale / shape.to_output.determinant().abs().sqrt().max(1e-6);
        for effect in &plan.effects {
            if filmcraft::instance::leaves_alone(effect, time, self.working()) {
                continue;
            }
            // Bringing a whole picture under 1 goes by its peak, which only the CPU knows before it
            // draws; each pixel under 1 needs no such thing.
            let way = if self.hdr.is_some() {
                filmcraft::catalog::headroom(effect.fx.as_str())
            } else {
                Headroom::Keeps
            };
            if way == Headroom::Whole {
                return None;
            }
            let before = ops.len();
            match effect.fx.as_str() {
                "color.basic" => {
                    let correction = Correction::new(&|name| effect_number(effect, name, time));
                    if !correction.is_identity() {
                        ops.push(Op::Correct(correction));
                    }
                }
                "blur.gaussian" => {
                    let sigma = (effect_number(effect, "radius", time) / 2.0 * density) as f32;
                    let (across, down) =
                        blur::gaussian_radii(shape.size.0, shape.size.1, sigma, sigma);
                    let radii = |radii: Vec<usize>| radii.into_iter().map(|r| r as u32).collect();
                    ops.push(Op::Effect(GpuEffect::Op(FxOp::Gaussian {
                        rx: radii(across),
                        ry: radii(down),
                        repeat: true,
                    })));
                }
                "blur.radialzoom" => {
                    let kernel = kerf_fx::kept::radial_zoom_kernel(
                        effect,
                        time,
                        shape.size,
                        shape.per_source,
                    )?;
                    ops.push(Op::Effect(GpuEffect::Kernel(kernel)));
                }
                id if kerf_fx::kept::kept(id) || domains::effect(id).is_some() => return None,
                _ => {
                    let track = |id: &str| {
                        sequence
                            .tracks
                            .iter()
                            .position(|track| track.id == id)
                            .map(|place| place + 1)
                    };
                    let on = Picture {
                        size: shape.size,
                        per_source: shape.per_source as f32,
                    };
                    let Some(instance) = filmcraft::instance::instance(effect, time, on, &track)
                    else {
                        continue;
                    };
                    if filmcraft::vfx::is_temporal(&instance.effect) {
                        return None;
                    }
                    let timecode =
                        format_timecode(sequence.start + at, sequence.rate, sequence.drop_frame);
                    let run = filmcraft::instance::Run {
                        working: self.working(),
                        at: time,
                        per_source: shape.per_source as f32,
                        timecode: &timecode,
                        clip_name: &plan.name,
                        library: &self.library,
                        env: None,
                        frame_rate: sequence.rate.fps(),
                    };
                    let op = filmcraft::instance::gpu_effect(&instance, &run, shape.size)?;
                    ops.push(Op::Effect(op));
                }
            }
            if ops.len() > before {
                match (way, ops.pop()) {
                    (Headroom::PerPixel, Some(Op::Effect(op))) => ops.push(Op::Held(op)),
                    (Headroom::PerPixel, Some(Op::Correct(correction))) => {
                        ops.push(Op::HeldCorrect(correction));
                    }
                    (Headroom::Gamut, Some(Op::Effect(op))) => ops.push(Op::Gamut(op)),
                    (_, other) => ops.extend(other),
                }
            }
        }
        Some(ops)
    }

    /// Sequence `index` at its time `at`, transparent where nothing is.
    fn gpu_draw(
        &self,
        frame: &mut GpuFrame<'static>,
        index: usize,
        at: Time,
        scale: f64,
        angle: Option<&kerf_model::Id>,
    ) -> Option<GpuPicture> {
        let plan = &self.sequences[index];
        let (w, h) = output_size(plan.width, plan.height, scale);
        let mut canvas = frame.clear(w as u32, h as u32);
        for track in plan.tracks.iter().filter(|track| track.drawn(angle)) {
            for shown in track.shown.iter().filter(|shown| shown.shows(at)) {
                canvas = self.gpu_layer(frame, canvas, plan, shown, at, scale)?;
            }
            for dip in track.dips.iter().filter(|dip| dip.shows(at)) {
                let opacity = f64::from(dip.color.a) / 255.0
                    * (1.0 - (2.0 * progress(at, dip.from, dip.to) - 1.0).abs());
                let solid = frame.filled(
                    w as u32,
                    h as u32,
                    premultiplied(Rgba {
                        a: 255,
                        ..dip.color
                    }),
                );
                let whole = Rect::new(0.0, 0.0, w as f64, h as f64);
                canvas = frame.composite(
                    canvas,
                    &solid,
                    &Affine::IDENTITY,
                    whole,
                    opacity as f32,
                    Blend::Normal,
                );
                frame.release(solid);
            }
        }
        if self.captions {
            for caption in plan.captions.iter().filter(|caption| caption.shows(at)) {
                let drawn = caption_on(caption, plan, scale, (w, h));
                let picture = frame.image(&drawn);
                let whole = Rect::new(0.0, 0.0, w as f64, h as f64);
                canvas = frame.composite(
                    canvas,
                    &picture,
                    &Affine::IDENTITY,
                    whole,
                    1.0,
                    Blend::Normal,
                );
                frame.release(picture);
            }
        }
        Some(canvas)
    }

    /// A clip over `canvas` at sequence time `at`, as [`Drawing::layer`] draws it.
    fn gpu_layer(
        &self,
        frame: &mut GpuFrame<'static>,
        canvas: GpuPicture,
        sequence: &SequencePlan,
        shown: &Shown,
        at: Time,
        scale: f64,
    ) -> Option<GpuPicture> {
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        if matches!(plan.source, Source::Nothing) {
            return Some(canvas);
        }
        let shape = self.gpu_shape(sequence, plan, time, scale);
        let ops = self.gpu_ops(sequence, plan, time, at, scale, &shape)?;
        if matches!(plan.source, Source::Adjustment) {
            return self.gpu_adjust(frame, canvas, (sequence, plan, time, scale), &ops);
        }
        // Media that cannot be read draws nothing, as on the CPU.
        let Some(picture) = self.gpu_source(frame, sequence, plan, time, scale, &shape)? else {
            return Some(canvas);
        };
        let mut picture = picture;
        let size = (f64::from(plan.size.0), f64::from(plan.size.1));
        let source_per = shape.source_per;
        let mut crop = crop_rect(
            plan,
            time,
            size,
            source_per,
            (picture.w as usize, picture.h as usize),
        );
        if plan.letterboxed {
            let place =
                Affine::scale(scale, scale).then_apply(&self.geometry(sequence, plan, time));
            let m = place.then_apply(&Affine::scale(1.0 / source_per, 1.0 / source_per));
            let whole = Rect::new(0.0, 0.0, picture.w as f64, picture.h as f64);
            let boxed = frame.filled(canvas.w, canvas.h, [0.0, 0.0, 0.0, 1.0]);
            let boxed = frame.composite(
                boxed,
                &picture,
                &m,
                crop.take().unwrap_or(whole),
                1.0,
                Blend::Normal,
            );
            frame.release(picture);
            picture = boxed;
        } else {
            let masks = mask::prepare(&clip.masks, size, source_per, time);
            if let Some(covered) = mask::coverage(&masks, picture.w as usize, picture.h as usize) {
                picture = frame.multiply(picture, &covered);
            }
        }
        for op in &ops {
            picture = match op {
                Op::Effect(op) => op.run(frame, picture)?,
                Op::Held(op) => op.run_held(frame, picture)?,
                Op::Gamut(op) => op.run_in_gamut(frame, picture)?,
                Op::Correct(correction) => frame.correct(picture, correction),
                Op::HeldCorrect(correction) => filmcraft::vfx::gpu::Kernel::held(
                    frame,
                    picture,
                    |frame, unit| Some(frame.correct(unit, correction)),
                )?,
            };
        }
        let reveal = shown
            .reveal
            .map(|(reveal, start, end)| (reveal, progress(at, start, end)));
        if let Some((Reveal::Wipe { angle, softness }, progress)) = reveal {
            let covered = wipe::coverage(
                picture.w as usize,
                picture.h as usize,
                angle,
                softness,
                progress,
            );
            picture = frame.multiply(picture, &covered);
        }
        let fade = match reveal {
            Some((Reveal::Dissolve, progress)) => progress,
            _ => 1.0,
        };
        let opacity = (number(&param(clip, "opacity").value_at(time)) * fade) as f32;
        let whole = Rect::new(0.0, 0.0, picture.w as f64, picture.h as f64);
        let canvas = frame.composite(
            canvas,
            &picture,
            &shape.to_output,
            crop.unwrap_or(whole),
            opacity,
            plan.blend,
        );
        frame.release(picture);
        Some(canvas)
    }

    /// An adjustment layer, as [`Drawing::adjust`] draws it: its effects on the picture beneath
    /// it, weighed against that picture by its opacity, masks, and frame.
    fn gpu_adjust(
        &self,
        frame: &mut GpuFrame<'static>,
        canvas: GpuPicture,
        placed: (&SequencePlan, &ClipPlan, Time, f64),
        ops: &[Op],
    ) -> Option<GpuPicture> {
        let (sequence, plan, time, scale) = placed;
        let (w, h) = (canvas.w as usize, canvas.h as usize);
        let mut adjusted = frame.copy(&canvas);
        for op in ops {
            adjusted = match op {
                Op::Effect(op) => op.run(frame, adjusted)?,
                Op::Held(op) => op.run_held(frame, adjusted)?,
                Op::Gamut(op) => op.run_in_gamut(frame, adjusted)?,
                Op::Correct(correction) => frame.correct(adjusted, correction),
                Op::HeldCorrect(correction) => filmcraft::vfx::gpu::Kernel::held(
                    frame,
                    adjusted,
                    |frame, unit| Some(frame.correct(unit, correction)),
                )?,
            };
        }
        let size = (f64::from(plan.size.0), f64::from(plan.size.1));
        let mut weight = frame.filled(canvas.w, canvas.h, [1.0; 4]);
        let masks = mask::prepare(&plan.clip.masks, size, scale, time);
        if let Some(covered) = mask::coverage(&masks, w, h) {
            weight = frame.multiply(weight, &covered);
        }
        let place = Affine::scale(scale, scale)
            .then_apply(&self.geometry(sequence, plan, time))
            .then_apply(&Affine::scale(1.0 / scale, 1.0 / scale));
        let crop = crop_rect(plan, time, size, scale, (w, h));
        if crop.is_some() || !identity(&place) {
            let whole = Rect::new(0.0, 0.0, w as f64, h as f64);
            let empty = frame.clear(canvas.w, canvas.h);
            let placed = frame.composite(
                empty,
                &weight,
                &place,
                crop.unwrap_or(whole),
                1.0,
                Blend::Normal,
            );
            frame.release(weight);
            weight = placed;
        }
        let opacity = number(&param(&plan.clip, "opacity").value_at(time)) as f32;
        let out = frame.mix_weighted(canvas, adjusted, &weight, opacity, plan.blend);
        frame.release(weight);
        Some(out)
    }

    /// A clip's picture before its masks and effects. `None` when the frame has to go to the CPU,
    /// and `Some(None)` when the clip draws nothing.
    fn gpu_source(
        &self,
        frame: &mut GpuFrame<'static>,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
        shape: &Shape,
    ) -> Option<Option<GpuPicture>> {
        let (cw, ch) = output_size(sequence.width, sequence.height, scale);
        Some(Some(match &plan.source {
            Source::Media { picture } => {
                let (w, h) = shape.source;
                let size = (w as u32, h as u32);
                let Some(decoded) = self.pictures.decoded(*picture, time, size) else {
                    return Some(None);
                };
                let rgba = &decoded.picture;
                if (rgba.width(), rgba.height()) != size {
                    return None;
                }
                let stride = rgba.stride(0) as u32;
                match decoded.deep {
                    Some(input) => frame.rgba16(rgba.data(0), size, stride, decoded.alpha, input),
                    None => frame.rgba8(rgba.data(0), size.0, size.1, stride, decoded.alpha),
                }
            }
            Source::Nested {
                sequence: nested,
                source_in,
                speed,
                duration,
                angle,
            } => {
                let inner = &self.sequences[*nested];
                let shown = nested_time(time, *source_in, speed, *duration, inner.rate);
                self.gpu_draw(frame, *nested, shown, scale, angle.as_ref())?
            }
            Source::Solid(color) => frame.filled(cw as u32, ch as u32, premultiplied(*color)),
            Source::Title(title) => frame.image(&title_on(title, (cw, ch), time)),
            Source::Motion(motion) => frame.image(&motion_on(*motion, &plan.clip, (cw, ch), time)),
            Source::Adjustment | Source::Nothing => return None,
        }))
    }
}
