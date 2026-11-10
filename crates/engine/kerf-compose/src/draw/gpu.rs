//! A frame drawn on the GPU: the same walk as the CPU's, through the sequence's tracks and each
//! clip's picture, masks, effects, and placing, with the numbers worked out by the same functions.
//! [`Drawing::gpu_can_draw`] decides for a whole frame before anything is decoded, and whatever it
//! refuses (Morph Cut, effects without a kernel) the CPU draws.

use super::*;
use kerf_fx::filmcraft::catalog::Headroom;
use kerf_fx::filmcraft::gpufx::FxOp;
use kerf_fx::filmcraft::instance::{GpuEffect, Picture};
use kerf_fx::transitions::gpu::Kernel as TransitionKernel;
use kerf_gpu::{Frame as GpuFrame, Gpu, Picture as GpuPicture};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

/// Frames with fewer pixels than this gain nothing: the CPU draws them before a GPU round trip is
/// over.
const SMALLEST_PIXELS: usize = 640 * 360;

/// The least share of a canvas whose frame has [`SMALLEST_PIXELS`], once rounded to whole pixels.
fn smallest_on_gpu_of(width: u32, height: u32) -> f64 {
    let area = f64::from(width.max(1)) * f64::from(height.max(1));
    let mut scale = (SMALLEST_PIXELS as f64 / area).sqrt().min(1.0);
    while scale < 1.0 && {
        let (w, h) = output_size(width, height, scale);
        w * h < SMALLEST_PIXELS
    } {
        scale = (scale * 1.01).min(1.0);
    }
    scale
}

/// The device, once it and every kernel are built.
fn ready_gpu() -> Option<&'static Gpu> {
    kerf_gpu::shared().filter(|gpu| {
        filmcraft::gpu::available(gpu)
            && kerf_fx::transitions::gpu::available(gpu)
            && gpu.depth_kernels().is_some()
    })
}

/// A clip's picture with all done to it that is done to the picture itself, as [`Ready`] on the CPU.
struct GpuReady {
    picture: GpuPicture,
    to_output: Affine,
    crop: Option<Rect>,
    opacity: f32,
    /// Nothing can have left a hole in it.
    opaque: bool,
}

/// How the clips in depth of a sequence are seen and lit at a moment, and what they read ready.
struct Depth {
    scene: three_d::Scene,
    built: kerf_gpu::depth::Scene,
    /// The pictures of those that stand in a light's way, which drawing them need not make again.
    ready: Vec<(kerf_model::Id, GpuReady)>,
    /// A number for each clip, to tell the one drawn from those in a light's way.
    numbers: Vec<kerf_model::Id>,
    /// The distance fields of those with a bevel that stand in a light's way, which drawing them need
    /// not make again.
    distances: Vec<(kerf_model::Id, kerf_gpu::depth::Distance)>,
    /// Where a clip that does not cross another writes how far off it is, for a camera's blur.
    depths: Option<kerf_gpu::depth::Depths>,
}

impl Depth {
    fn number(&mut self, clip: &kerf_model::Id) -> u32 {
        let place = self.numbers.iter().position(|id| id == clip);
        place.unwrap_or_else(|| {
            self.numbers.push(clip.clone());
            self.numbers.len() - 1
        }) as u32
    }
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

    /// The least of the canvas [`Self::gpu_frame_as`] goes to the GPU for at [`Accel::Auto`].
    pub(crate) fn smallest_on_gpu(&self) -> f64 {
        let plan = &self.sequences[0];
        smallest_on_gpu_of(plan.width, plan.height)
    }

    /// Why the CPU draws the frame at `at`, one line a cause; none where the GPU draws it.
    pub(crate) fn why_on_cpu(&self, at: Time, scale: f64) -> Vec<String> {
        let scale = scale.clamp(1.0 / 16.0, 1.0);
        let Some(gpu) = ready_gpu() else {
            return vec!["no GPU on this machine".to_owned()];
        };
        if self.gpu_can_draw(gpu, 0, at, scale) {
            return Vec::new();
        }
        let plan = &self.sequences[0];
        let (w, h) = output_size(plan.width, plan.height, scale);
        let mut why = Vec::new();
        if !gpu.fits(w, h) {
            why.push(format!("{w}x{h} is larger than the GPU can hold"));
        }
        if !self.gpu_can_draw_depth(gpu, plan, at, scale, None) {
            why.push("more clips in depth at once than the GPU can hold".to_owned());
        }
        for track in plan.tracks.iter().filter(|track| track.drawn(None)) {
            for (index, shown) in track.shown.iter().enumerate() {
                if !shown.shows(at) {
                    continue;
                }
                let size = output_size(plan.width, plan.height, scale);
                if let Some(pair) = track.pair_at(index, at) {
                    if transition_kernel(pair, at, size, scale).is_none() {
                        why.push(format!(
                            "a transition at clip `{}` has no kernel",
                            shown.plan.clip.id
                        ));
                    }
                }
                self.why_clip_on_cpu(gpu, plan, shown, at, scale, &mut why);
            }
        }
        if why.is_empty() {
            why.push("something in it has no kernel".to_owned());
        }
        why
    }

    fn why_clip_on_cpu(
        &self,
        gpu: &Gpu,
        sequence: &SequencePlan,
        shown: &Shown,
        at: Time,
        scale: f64,
        why: &mut Vec<String>,
    ) {
        if self.gpu_can_draw_clip(gpu, sequence, shown, at, scale) {
            return;
        }
        let plan = &shown.plan;
        let clip = plan.clip.id.as_str();
        if plan.clip.retime != kerf_model::Retime::Frames {
            why.push(format!(
                "clip `{clip}` makes frames between frames with `retime`"
            ));
            return;
        }
        let time = at - plan.clip.start;
        let shape = self.gpu_shape(sequence, plan, time, scale);
        if !gpu.fits(shape.source.0, shape.source.1) {
            why.push(format!("clip `{clip}` is larger than the GPU can hold"));
        }
        let mut named = false;
        for (index, effect) in plan.effects.iter().enumerate() {
            if self
                .gpu_ops_of(sequence, plan, (time, at, scale), &shape, Some(index))
                .is_none()
            {
                why.push(format!("clip `{clip}`: `{}` has no kernel here", effect.fx));
                named = true;
            }
        }
        if !named && matches!(plan.source, Source::Nested { .. }) {
            why.push(format!(
                "clip `{clip}`: something in its sequence has no kernel"
            ));
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
        self.gpu_can_draw_depth(gpu, plan, at, scale, angle)
            && plan
                .tracks
                .iter()
                .filter(|track| track.drawn(angle))
                .all(|track| self.gpu_can_draw_track(gpu, plan, track, at, scale))
    }

    /// Whether the kernels can draw what is in depth in this sequence at its time `at`. A camera's
    /// blur needs a little more room than a picture; and a light all around may read a track that does
    /// not show.
    fn gpu_can_draw_depth(
        &self,
        gpu: &Gpu,
        plan: &SequencePlan,
        at: Time,
        scale: f64,
        angle: Option<&kerf_model::Id>,
    ) -> bool {
        let tracks = || plan.tracks.iter().filter(|track| track.drawn(angle));
        let in_depth = tracks().filter(|track| {
            track
                .shown
                .iter()
                .any(|shown| shown.shows(at) && shown.plan.three_d)
        });
        let count = in_depth.count();
        if count == 0 {
            return true;
        }
        let (w, h) = output_size(plan.width, plan.height, scale);
        if count > kerf_gpu::depth::MOST_LAYERS || !gpu.fits(w, h * count) {
            return false;
        }
        let scene = three_d::Scene::at(plan, at, (w, h));
        if scene.has_aperture() && !gpu.fits(w + kerf_gpu::focus::SUMS_WIDER, h) {
            return false;
        }
        scene.environments().iter().all(|wanted| {
            plan.tracks
                .iter()
                .find(|track| track.id.as_str() == wanted)
                .and_then(|track| track.shown.iter().find(|shown| shown.shows(at)))
                .is_none_or(|shown| self.gpu_can_draw_clip(gpu, plan, shown, at, scale))
        })
    }

    fn gpu_can_draw_track(
        &self,
        gpu: &Gpu,
        sequence: &SequencePlan,
        track: &TrackPlan,
        at: Time,
        scale: f64,
    ) -> bool {
        let size = output_size(sequence.width, sequence.height, scale);
        track.shown.iter().enumerate().all(|(index, shown)| {
            if !shown.shows(at) {
                return true;
            }
            track
                .pair_at(index, at)
                .is_none_or(|pair| transition_kernel(pair, at, size, scale).is_some())
                && self.gpu_can_draw_clip(gpu, sequence, shown, at, scale)
        })
    }

    fn gpu_can_draw_clip(
        &self,
        gpu: &Gpu,
        sequence: &SequencePlan,
        shown: &Shown,
        at: Time,
        scale: f64,
    ) -> bool {
        let plan = &shown.plan;
        // Frames between frames are made on the CPU.
        if plan.clip.retime != kerf_model::Retime::Frames {
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
        self.gpu_ops_of(sequence, plan, (time, at, scale), shape, None)
    }

    /// [`Self::gpu_ops`] for all of a clip's effects, or for `only` the one, to say which has no kernel.
    /// `moment` is the time in the clip, the time in the sequence, and the scale.
    fn gpu_ops_of(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        (time, at, scale): (Time, Time, f64),
        shape: &Shape,
        only: Option<usize>,
    ) -> Option<Vec<Op>> {
        let mut ops = Vec::with_capacity(plan.effects.len());
        let density = scale / shape.to_output.determinant().abs().sqrt().max(1e-6);
        for (index, effect) in plan.effects.iter().enumerate() {
            if only.is_some_and(|only| only != index) {
                continue;
            }
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
                "noise.addgrain" => {
                    let seed =
                        kerf_core::raster::seed(plan.clip.id.as_str()).wrapping_add(index as u32);
                    let kernel = kerf_fx::kept::grain_kernel(
                        effect,
                        time,
                        shape.size,
                        shape.per_source,
                        seed,
                    )?;
                    ops.push(Op::Effect(GpuEffect::Kernel(kernel)));
                }
                // Only FilmCraft's effects have kernels beyond those above.
                id if filmcraft::catalog::carried(id).is_none() => return None,
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
                    let library = filmcraft::instance::luts(effect, &self.files);
                    let run = filmcraft::instance::Run {
                        working: self.working(),
                        at: time,
                        per_source: shape.per_source as f32,
                        timecode: &timecode,
                        clip_name: &plan.name,
                        library: library.as_ref(),
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
        let tracks = self.in_order(plan, at, scale, angle);
        let in_depth = tracks.iter().any(|track| {
            track
                .shown
                .iter()
                .any(|shown| shown.shows(at) && shown.plan.three_d)
        });
        let mut depth = if in_depth {
            Some(self.gpu_depth(frame, plan, &tracks, at, scale, (w, h))?)
        } else {
            None
        };
        let mut next = 0;
        while next < tracks.len() {
            let crossing = match &depth {
                Some(depth) => {
                    let moment = Moment {
                        at,
                        scale,
                        scene: Some(&depth.scene),
                    };
                    self.crossing_run(plan, &tracks[next..], &moment)
                }
                None => Vec::new(),
            };
            if let (false, Some(depth)) = (crossing.is_empty(), &mut depth) {
                next += crossing.len();
                canvas = self.gpu_crossing(frame, canvas, plan, &crossing, depth, (at, scale))?;
                continue;
            }
            let track = tracks[next];
            next += 1;
            for (index, shown) in track.shown.iter().enumerate() {
                if !shown.shows(at) {
                    continue;
                }
                canvas = match track.pair_at(index, at) {
                    // Drawn once, at the first of its clips that shows.
                    Some(pair) if index == pair.from || !track.shown[pair.from].shows(at) => {
                        let on = (plan, track);
                        self.gpu_pair(frame, canvas, on, pair, depth.as_mut(), (at, scale))?
                    }
                    Some(_) => canvas,
                    None => {
                        let (on, moment) = ((plan, shown), (at, scale));
                        let blend = shown.plan.blend;
                        self.gpu_layer(frame, canvas, on, depth.as_mut(), moment, blend)?
                    }
                };
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
                let key = caption_key(caption, plan, scale, (w, h));
                let picture = frame.image_kept(&key, || caption_on(caption, plan, scale, (w, h)));
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

    /// Two clips mixed by a transition at sequence time `at`, over `canvas`, as [`Drawing::pair`]
    /// draws them.
    fn gpu_pair(
        &self,
        frame: &mut GpuFrame<'static>,
        canvas: GpuPicture,
        (sequence, track): (&SequencePlan, &TrackPlan),
        pair: &Pair,
        mut depth: Option<&mut Depth>,
        (at, scale): (Time, f64),
    ) -> Option<GpuPicture> {
        let (w, h) = (canvas.w, canvas.h);
        let kernel = transition_kernel(pair, at, (w as usize, h as usize), scale)?;
        let (from, to) = (&track.shown[pair.from], &track.shown[pair.to]);
        let mut side = |frame: &mut GpuFrame<'static>, shown: Option<&Shown>| {
            let empty = frame.clear(w, h);
            match shown.filter(|shown| shown.shows(at)) {
                Some(shown) => self.gpu_layer(
                    frame,
                    empty,
                    (sequence, shown),
                    depth.as_deref_mut(),
                    (at, scale),
                    Blend::Normal,
                ),
                None => Some(empty),
            }
        };
        // One clip alone: it enters from nothing, or leaves into it.
        let (outgoing, incoming) = match (pair.from == pair.to, pair.from_nothing) {
            (false, _) => (Some(from), Some(to)),
            (true, true) => (None, Some(to)),
            (true, false) => (Some(from), None),
        };
        let a = side(frame, outgoing)?;
        let b = side(frame, incoming)?;
        let mixed = kernel.run(frame, &a, &b)?;
        frame.release(a);
        frame.release(b);
        // A fade from or to nothing keeps the one clip's blend mode; between two they mix normally.
        let blend = match (from.shows(at), to.shows(at)) {
            (true, false) => from.plan.blend,
            (false, true) => to.plan.blend,
            _ if pair.from == pair.to => from.plan.blend,
            _ => Blend::Normal,
        };
        let whole = Rect::new(0.0, 0.0, f64::from(w), f64::from(h));
        let out = frame.composite(canvas, &mixed, &Affine::IDENTITY, whole, 1.0, blend);
        frame.release(mixed);
        Some(out)
    }

    /// A clip over `canvas` at sequence time `at` with `blend`, as [`Drawing::layer`] draws it.
    fn gpu_layer(
        &self,
        frame: &mut GpuFrame<'static>,
        canvas: GpuPicture,
        (sequence, shown): (&SequencePlan, &Shown),
        depth: Option<&mut Depth>,
        (at, scale): (Time, f64),
        blend: Blend,
    ) -> Option<GpuPicture> {
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        if matches!(plan.source, Source::Nothing) {
            return Some(canvas);
        }
        if matches!(plan.source, Source::Adjustment) {
            let shape = self.gpu_shape(sequence, plan, time, scale);
            let ops = self.gpu_ops(sequence, plan, time, at, scale, &shape)?;
            return self.gpu_adjust(frame, canvas, (sequence, plan, time, scale), &ops);
        }
        let size = (canvas.w, canvas.h);
        if let (true, Some(depth)) = (plan.three_d, depth) {
            let ready = match depth.ready.iter().position(|(id, _)| *id == clip.id) {
                Some(place) => Some(depth.ready.swap_remove(place).1),
                None => self.gpu_ready(frame, sequence, shown, at, scale, size)?,
            };
            // Media that cannot be read draws nothing, as on the CPU.
            let Some(ready) = ready else {
                return Some(canvas);
            };
            let drawn = self.gpu_project(frame, depth, (plan, time), &ready, size, None)?;
            let whole = Rect::new(0.0, 0.0, f64::from(size.0), f64::from(size.1));
            let canvas = frame.composite(
                canvas,
                &drawn,
                &Affine::IDENTITY,
                whole,
                ready.opacity,
                blend,
            );
            frame.release(drawn);
            frame.release(ready.picture);
            return Some(canvas);
        }
        let Some(ready) = self.gpu_ready(frame, sequence, shown, at, scale, size)? else {
            return Some(canvas);
        };
        let whole = Rect::new(
            0.0,
            0.0,
            f64::from(ready.picture.w),
            f64::from(ready.picture.h),
        );
        let canvas = frame.composite(
            canvas,
            &ready.picture,
            &ready.to_output,
            ready.crop.unwrap_or(whole),
            ready.opacity,
            blend,
        );
        frame.release(ready.picture);
        Some(canvas)
    }

    /// How the clips in depth of `tracks` are seen and lit at `at`, with their pictures ready when a
    /// light casts shadows, and only then, and the pictures of the lights all around.
    fn gpu_depth(
        &self,
        frame: &mut GpuFrame<'static>,
        plan: &SequencePlan,
        tracks: &[&TrackPlan],
        at: Time,
        scale: f64,
        size: (usize, usize),
    ) -> Option<Depth> {
        let scene = three_d::Scene::at(plan, at, size);
        let canvas = (size.0 as u32, size.1 as u32);
        let mut depth_numbers = Vec::new();
        let mut distances = Vec::new();
        let mut ready: Vec<(kerf_model::Id, GpuReady)> = Vec::new();
        let mut casters = Vec::new();
        if scene.casts_shadows() {
            for shown in tracks
                .iter()
                .flat_map(|track| &track.shown)
                .filter(|shown| {
                    shown.shows(at)
                        && shown.plan.three_d
                        && !matches!(shown.plan.source, Source::Nothing | Source::Adjustment)
                })
            {
                let Some(one) = self.gpu_ready(frame, plan, shown, at, scale, canvas)? else {
                    continue;
                };
                let time = at - shown.plan.clip.start;
                let (pw, ph) = (f64::from(one.picture.w), f64::from(one.picture.h));
                let plane = three_d::Plane::of(&scene, &shown.plan, (pw, ph), &one.to_output, time);
                let height = size.1 as f64;
                let deep = three_d::depth_of(&shown.plan, time, height);
                let bevel = three_d::bevel_of(&shown.plan, time, height);
                let round = three_d::bevel_is_round(&shown.plan);
                let number = depth_numbers.len() as u32;
                depth_numbers.push(shown.plan.clip.id.clone());
                let crop = one.crop.unwrap_or_else(|| Rect::new(0.0, 0.0, pw, ph));
                let distance = if bevel > 0.0 {
                    let width = bevel / plane.pixel_size();
                    let distance = gpu_distance(frame, &one.picture, crop, width, plane.stretch())?;
                    distances.push((shown.plan.clip.id.clone(), distance.clone()));
                    Some(distance)
                } else {
                    None
                };
                // One stand-in to each way its outline is taken in, as few as a bevel makes of them.
                let mut by_inset: Vec<(f64, Vec<kerf_gpu::depth::Plane>)> = Vec::new();
                for (plane, inset) in scene.casting_planes(&plane, deep, (bevel, round)) {
                    match by_inset.iter_mut().find(|(kept, _)| *kept == inset) {
                        Some((_, planes)) => planes.push(plane),
                        None => by_inset.push((inset, vec![plane])),
                    }
                }
                let place = ready.len();
                for (inset, planes) in by_inset {
                    let taken_in = match (&distance, inset > 0.0) {
                        (Some(distance), true) => {
                            let eroded = frame.eroded(&one.picture, distance, inset as f32)?;
                            Some(eroded)
                        }
                        _ => None,
                    };
                    casters.push((number, planes, crop, place, taken_in));
                }
                ready.push((shown.plan.clip.id.clone(), one));
            }
        }
        let mut around = Vec::new();
        for (light, wanted) in scene.environments_by_light() {
            let shown = plan
                .tracks
                .iter()
                .find(|track| track.id.as_str() == wanted)
                .and_then(|track| track.shown.iter().find(|shown| shown.shows(at)));
            let Some(shown) = shown else {
                continue;
            };
            if let Some(one) = self.gpu_ready(frame, plan, shown, at, scale, canvas)? {
                around.push((light, one.picture));
            }
        }
        let standing: Vec<kerf_gpu::depth::Caster<'_>> = casters
            .iter()
            .map(|(clip, planes, crop, place, taken_in)| {
                let one = &ready[*place].1;
                kerf_gpu::depth::Caster {
                    clip: *clip,
                    picture: taken_in.as_ref().unwrap_or(&one.picture),
                    opaque: one.opaque && taken_in.is_none(),
                    opacity: one.opacity,
                    crop: edges(*crop),
                    planes: planes.clone(),
                }
            })
            .collect();
        let environments: Vec<kerf_gpu::depth::Environment<'_>> = around
            .iter()
            .map(|(light, picture)| kerf_gpu::depth::Environment {
                light: *light,
                picture,
            })
            .collect();
        let built = frame.depth_scene(&scene.seen(), &standing, &environments)?;
        drop(standing);
        drop(environments);
        for (_, picture) in around {
            frame.release(picture);
        }
        for (.., taken_in) in casters {
            if let Some(picture) = taken_in {
                frame.release(picture);
            }
        }
        Some(Depth {
            scene,
            built,
            ready,
            numbers: depth_numbers,
            distances,
            depths: None,
        })
    }

    /// A clip in depth, on a picture of `size`, transparent around it, out of focus as far as the camera
    /// has it.
    fn gpu_project(
        &self,
        frame: &mut GpuFrame<'static>,
        depth: &mut Depth,
        (plan, time): (&ClipPlan, Time),
        ready: &GpuReady,
        size: (u32, u32),
        depths: Option<kerf_gpu::depth::Depths>,
    ) -> Option<GpuPicture> {
        let (pw, ph) = (f64::from(ready.picture.w), f64::from(ready.picture.h));
        let number = depth.number(&plan.clip.id);
        let depths = depths.or_else(|| {
            depth.scene.has_aperture().then(|| {
                depth
                    .depths
                    .get_or_insert_with(|| frame.depths(size))
                    .clone()
            })
        });
        let scene = &depth.scene;
        let plane = three_d::Plane::of(scene, plan, (pw, ph), &ready.to_output, time);
        let smaller = three_d::read_smaller(scene, &plane, (pw, ph)).map(|by| {
            let (sw, sh) = three_d::smaller_size((pw, ph), by);
            let empty = frame.clear(sw as u32, sh as u32);
            let whole = Rect::new(0.0, 0.0, pw, ph);
            let small = frame.composite(
                empty,
                &ready.picture,
                &Affine::scale(by, by),
                whole,
                1.0,
                Blend::Normal,
            );
            (small, by)
        });
        let height = f64::from(size.1);
        let deep = three_d::depth_of(plan, time, height);
        let bevel = three_d::bevel_of(plan, time, height);
        let crop = ready.crop.unwrap_or_else(|| Rect::new(0.0, 0.0, pw, ph));
        let by = smaller.as_ref().map_or(1.0, |(_, by)| *by);
        let reading = smaller.as_ref().map_or(&ready.picture, |(small, _)| small);
        // In the pixels it is read at.
        let per = plane.pixel_size() / by;
        let distance = if bevel > 0.0 {
            let kept = depth
                .distances
                .iter()
                .find(|(id, _)| *id == plan.clip.id && by == 1.0)
                .map(|(_, distance)| distance.clone());
            let read_crop = Rect::new(crop.x * by, crop.y * by, crop.w * by, crop.h * by);
            Some(match kept {
                Some(distance) => distance,
                None => gpu_distance(frame, reading, read_crop, bevel / per, plane.stretch())?,
            })
        } else {
            None
        };
        let drawn = frame.project(
            &depth.built,
            &kerf_gpu::depth::Project {
                picture: reading,
                by: by as f32,
                clip: number,
                plane: scene.plane_for_gpu(&plane),
                back: (deep > 0.0).then(|| (scene.back_for_gpu(&plane, deep), ready.opaque)),
                crop: edges(crop),
                bevel: distance.as_ref().map(|distance| kerf_gpu::depth::Bevel {
                    distance,
                    width: (bevel / per) as f32,
                    depth: (deep / per) as f32,
                    round: three_d::bevel_is_round(plan),
                    stretch: plane.stretch() as f32,
                    least_steps: three_d::bevel_steps(deep / per) as f32,
                }),
            },
            size,
            depths.as_ref(),
        );
        if let Some((small, _)) = smaller {
            frame.release(small);
        }
        match (drawn, depths) {
            (Some(drawn), Some(depths)) if scene.has_aperture() => {
                gpu_out_of_focus(frame, scene, drawn, &depths)
            }
            (drawn, _) => drawn,
        }
    }

    /// Clips in depth that cross, each cut where it meets another, over `canvas`.
    fn gpu_crossing(
        &self,
        frame: &mut GpuFrame<'static>,
        canvas: GpuPicture,
        sequence: &SequencePlan,
        run: &[&Shown],
        depth: &mut Depth,
        (at, scale): (Time, f64),
    ) -> Option<GpuPicture> {
        let size = (canvas.w, canvas.h);
        let mut layers = frame.depth_layers(size, run.len() as u32);
        for shown in run {
            let id = &shown.plan.clip.id;
            let ready = match depth.ready.iter().position(|(kept, _)| kept == id) {
                Some(place) => Some(depth.ready.swap_remove(place).1),
                None => self.gpu_ready(frame, sequence, shown, at, scale, size)?,
            };
            let Some(ready) = ready else {
                continue;
            };
            let depths = layers.next_depths();
            let time = at - shown.plan.clip.start;
            let drawn = self.gpu_project(
                frame,
                depth,
                (&shown.plan, time),
                &ready,
                size,
                Some(depths),
            )?;
            frame.lay(&mut layers, &drawn, ready.opacity, shown.plan.blend);
            frame.release(drawn);
            frame.release(ready.picture);
        }
        frame.by_depth(canvas, layers)
    }

    /// A shown clip's picture with its effects and reveal done, as [`Drawing::ready`] makes it. `None`
    /// when the frame has to go to the CPU, and `Some(None)` when the clip draws nothing.
    fn gpu_ready(
        &self,
        frame: &mut GpuFrame<'static>,
        sequence: &SequencePlan,
        shown: &Shown,
        at: Time,
        scale: f64,
        canvas: (u32, u32),
    ) -> Option<Option<GpuReady>> {
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        let shape = self.gpu_shape(sequence, plan, time, scale);
        let ops = self.gpu_ops(sequence, plan, time, at, scale, &shape)?;
        // Media that cannot be read draws nothing, as on the CPU.
        let Some(picture) = self.gpu_source(frame, sequence, plan, time, scale, &shape)? else {
            return Some(None);
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
            let boxed = frame.filled(canvas.0, canvas.1, [0.0, 0.0, 0.0, 1.0]);
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
                Op::HeldCorrect(correction) => {
                    filmcraft::vfx::gpu::Kernel::held(frame, picture, |frame, unit| {
                        Some(frame.correct(unit, correction))
                    })?
                }
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
        let opaque = ops.is_empty()
            && reveal.is_none()
            && clip.masks.is_empty()
            && match &plan.source {
                Source::Solid(color) => color.a == 255,
                _ => plan.letterboxed,
            };
        Some(Some(GpuReady {
            picture,
            to_output: shape.to_output,
            crop,
            opacity,
            opaque,
        }))
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
                Op::HeldCorrect(correction) => {
                    filmcraft::vfx::gpu::Kernel::held(frame, adjusted, |frame, unit| {
                        Some(frame.correct(unit, correction))
                    })?
                }
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
                match (decoded.deep, crate::footage::yuv_of(rgba)) {
                    (Some(input), Some(yuv)) => frame.yuv16(&yuv, input),
                    (None, Some(yuv)) => frame.yuv(&yuv),
                    (Some(input), None) => {
                        frame.rgba16(rgba.data(0), size, stride, decoded.alpha, input)
                    }
                    (None, None) => {
                        frame.rgba8(rgba.data(0), size.0, size.1, stride, decoded.alpha)
                    }
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
            Source::Title(title) => {
                let key = title_key(title, (cw, ch), time);
                frame.image_kept(&key, || title_on(title, (cw, ch), time))
            }
            Source::Motion(motion) => frame.image(&motion_on(*motion, &plan.clip, (cw, ch), time)),
            Source::Adjustment | Source::Nothing => return None,
        }))
    }
}

/// The kernel of a transition at sequence time `at`, on pictures of `size`, or `None` when it stays
/// on the CPU.
fn transition_kernel(
    pair: &Pair,
    at: Time,
    size: (usize, usize),
    scale: f64,
) -> Option<TransitionKernel> {
    let fx = Fx::new(&pair.def, &pair.params)?;
    let p = progress(at, pair.start, pair.end) as f32;
    TransitionKernel::of(&fx, size, p, scale as f32)
}

/// A rectangle by its edges: left, top, right, bottom.
fn edges(rect: Rect) -> [f32; 4] {
    [
        rect.x as f32,
        rect.y as f32,
        rect.right() as f32,
        rect.bottom() as f32,
    ]
}

/// How far each pixel of `picture` is from its outline, as far as a bevel `width` pixels wide asks.
/// `drawn` with each pixel blurred by its own radius, as `focus::blurred` makes it on the CPU: the same
/// box, radii, and sizes, from what the frame has drawn so far, which takes waiting for it.
fn gpu_out_of_focus(
    frame: &mut GpuFrame<'static>,
    scene: &three_d::Scene,
    drawn: GpuPicture,
    depths: &kerf_gpu::depth::Depths,
) -> Option<GpuPicture> {
    let focus = scene.focus_for_gpu();
    let found = frame.focus_found(&drawn, depths, &focus)?;
    let (w, h) = (drawn.w as usize, drawn.h as usize);
    // With no alpha there is nothing to spread, and with no radius nowhere to spread it.
    if found.right < found.left || found.most < super::focus::NO_BLUR {
        return Some(drawn);
    }
    let margin = super::focus::margin(found.reach);
    let area = [
        found.left.saturating_sub(margin),
        found.top.saturating_sub(margin),
        (found.right + margin).min(w - 1),
        (found.bottom + margin).min(h - 1),
    ];
    let levels = super::focus::levels(found.least, found.most);
    let mut gathered = Vec::with_capacity(levels.len());
    for (index, radius) in levels.iter().enumerate() {
        // A radius this small is the picture itself.
        if *radius < super::focus::NO_BLUR {
            gathered.push((None, *radius, 1));
            continue;
        }
        let by = super::focus::gathered_by(*radius);
        // A pixel reads the levels either side of its radius.
        let least = index.checked_sub(1).map_or(-1.0, |below| levels[below]);
        let most = levels.get(index + 1).copied().unwrap_or(f32::MAX);
        let picture = frame.focus_gathered(
            &drawn,
            depths,
            &focus,
            (*radius, by as u32),
            area,
            (least, most),
        )?;
        gathered.push((Some(picture), *radius, by));
    }
    let reading: Vec<(&GpuPicture, f32, u32)> = gathered
        .iter()
        .map(|(picture, radius, by)| (picture.as_ref().unwrap_or(&drawn), *radius, *by as u32))
        .collect();
    let out = frame.focused(depths, &focus, &reading, area, (drawn.w, drawn.h));
    drop(reading);
    for (picture, ..) in gathered {
        if let Some(picture) = picture {
            frame.release(picture);
        }
    }
    frame.release(drawn);
    out
}

fn gpu_distance(
    frame: &mut GpuFrame<'static>,
    picture: &GpuPicture,
    crop: Rect,
    width: f64,
    stretch: f64,
) -> Option<kerf_gpu::depth::Distance> {
    let first = bevel::first_jump(bevel::reaching(width, stretch));
    frame.distance(
        picture,
        edges(crop),
        &bevel::jumps(first),
        bevel::reach(first),
        stretch as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::{output_size, smallest_on_gpu_of, SMALLEST_PIXELS};

    #[test]
    fn the_least_share_on_the_gpu_has_the_pixels_it_needs() {
        for (width, height) in [
            (1920, 1080),
            (3840, 2160),
            (1080, 1920),
            (1280, 720),
            (1001, 777),
        ] {
            let scale = smallest_on_gpu_of(width, height);
            let (w, h) = output_size(width, height, scale);
            assert!(
                w * h >= SMALLEST_PIXELS,
                "{width}x{height} at {scale} is {w}x{h}"
            );
            assert!(scale < 1.0);
        }
        assert!((smallest_on_gpu_of(1920, 1080) - 1.0 / 3.0).abs() < 1e-3);
        // A canvas already under it has nowhere further to go.
        assert_eq!(smallest_on_gpu_of(320, 180), 1.0);
    }
}
