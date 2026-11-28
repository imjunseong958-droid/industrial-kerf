//! A frame drawn on the GPU: the same walk as the CPU's, through the sequence's tracks and each
//! clip's picture, masks, effects, and placing, with the numbers worked out by the same functions.
//! [`Drawing::gpu_can_draw`] decides for a whole frame before anything is decoded, and whatever it
//! refuses (Morph Cut, effects without a kernel) the CPU draws.

use super::*;
use kerf_fx::transitions::gpu::Kernel as TransitionKernel;
use kerf_fx::video::catalog::Headroom;
use kerf_fx::video::instance::GpuEffect;
use kerf_gpu::{Frame as GpuFrame, Gpu, Picture as GpuPicture};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, Once, PoisonError};
use std::time::Duration;

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

/// Whether the thread that builds the device and its kernels is at it, and what rings when it
/// is done.
static WARMING: Mutex<bool> = Mutex::new(false);
static WARMED: Condvar = Condvar::new();

fn warming() -> MutexGuard<'static, bool> {
    WARMING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The device being built, from when this is made until it is dropped.
struct Warming;

impl Warming {
    fn begin() -> Self {
        *warming() = true;
        Self
    }
}

impl Drop for Warming {
    fn drop(&mut self) {
        *warming() = false;
        WARMED.notify_all();
    }
}

/// Wait while the device and its kernels are being built, for at most `within`: `false` when
/// they still are.
///
/// The first time a viewer's compositor is asked for the GPU (to draw a frame large enough for
/// it, or to say whether it could draw one), it starts building them, once, on a thread of
/// their own, and that thread loads the GPU's libraries: the Vulkan loader, each driver it
/// finds, and LLVM with Mesa's. A process that exits meanwhile runs those libraries'
/// destructors under it, and now and then dies of it as it ends (a test binary whose tests had
/// all passed, of SIGSEGV). So whoever is about to exit waits here, once nothing of its own
/// can ask any more.
pub fn gpu_settled(within: Duration) -> bool {
    let (still, _) = WARMED
        .wait_timeout_while(warming(), within, |still| *still)
        .unwrap_or_else(PoisonError::into_inner);
    !*still
}

/// The device, once it and every kernel are built.
fn ready_gpu() -> Option<&'static Gpu> {
    kerf_gpu::shared().filter(|gpu| {
        video::gpu::available(gpu)
            && kerf_fx::transitions::gpu::available(gpu)
            && gpu.depth_kernels().is_some()
    })
}

/// A clip's picture with all done to it that is done to the picture itself, as [`Ready`] on the CPU.
struct GpuReady {
    picture: GpuPicture,
    /// When that is, and its picture's pixels to output pixels then.
    placement: Placement,
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
    /// What a generator in depth that reaches past its frame is drawn on ([`Reach`]).
    past: Option<SourceRun>,
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
                static STARTED: Once = Once::new();
                static READY: AtomicBool = AtomicBool::new(false);
                STARTED.call_once(|| {
                    // Counted from here, so that [`gpu_settled`] never finds it between
                    // started and running.
                    let warming = Warming::begin();
                    std::thread::spawn(move || {
                        let _warming = warming;
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
                if let Some(pair) = track.pair_at(index, at)
                    && transition_kernel(pair, at, size, scale).is_none()
                {
                    why.push(format!(
                        "a transition at clip `{}` has no kernel",
                        shown.plan.clip.id
                    ));
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
        if matches!(plan.source, Source::Mesh) {
            why.push(format!(
                "clip `{clip}` is a mesh, under a light that casts a shadow or shines from a picture, which the CPU draws"
            ));
            return;
        }
        if plan.clip.retime == kerf_model::Retime::Flow {
            why.push(format!(
                "clip `{clip}` makes frames between frames along their motion with `retime=flow`"
            ));
            return;
        }
        let time = plan.clip.time_at(at);
        let reach = self.gpu_reach(sequence, plan, at, scale);
        let shape = self.gpu_shape(sequence, plan, at, scale, reach);
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
        if self.nests_filmed(plan, time) {
            why.push(format!(
                "clip `{clip}`: a model films a clip of its sequence, whose place the CPU leaves clear"
            ));
            named = true;
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

    /// Whether the sequence clip `plan` holds shows, at the clip's time `time`, a clip a model
    /// films, or a clip of a sequence that does: it leaves its place clear in a sequence drawn
    /// as a clip of another, which the CPU does.
    fn nests_filmed(&self, plan: &ClipPlan, time: Time) -> bool {
        let Source::Nested {
            sequence,
            map,
            angle,
        } = &plan.source
        else {
            return false;
        };
        let inner = &self.sequences[*sequence];
        let shown = map.nested(time, inner.rate);
        inner
            .tracks
            .iter()
            .filter(|track| track.drawn(angle.as_ref()))
            .flat_map(|track| &track.shown)
            .any(|clip| {
                clip.shows(shown)
                    && (clip.plan.filmed
                        || self.nests_filmed(&clip.plan, clip.plan.clip.time_at(shown)))
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
        // Frames between frames, each moved along the motion between them, are made on the
        // CPU. Two cross-faded are made here.
        if plan.clip.retime == kerf_model::Retime::Flow {
            return false;
        }
        // A mesh is drawn on the CPU and handed over as a picture and how far off it is,
        // which does where its light is the scene's lights alone: a shadow is cast through
        // the GPU's own planes and a light from a picture is the GPU's to read.
        if matches!(plan.source, Source::Mesh) {
            let size = output_size(sequence.width, sequence.height, scale);
            let scene = three_d::Scene::at(sequence, at, size);
            return !scene.casts_shadows() && scene.environments().is_empty();
        }
        let time = plan.clip.time_at(at);
        if let Source::Nested {
            sequence: nested,
            map,
            angle,
        } = &plan.source
        {
            let inner = &self.sequences[*nested];
            let shown = map.nested(time, inner.rate);
            if self.nests_filmed(plan, time)
                || !self.gpu_can_draw_of(gpu, *nested, shown, scale, angle.as_ref())
            {
                return false;
            }
        }
        let reach = self.gpu_reach(sequence, plan, at, scale);
        let shape = self.gpu_shape(sequence, plan, at, scale, reach);
        // A card behind it makes its picture larger.
        let past = shape.past.as_ref().map(frame_in);
        let tall = past.map_or(f64::from(plan.size.1) * shape.source_per, |past| past.h);
        let pad = border(plan, time, tall).map_or(0, |(width, _)| 2 * width.ceil() as usize);
        gpu.fits(shape.source.0 + pad, shape.source.1 + pad)
            && self
                .gpu_ops(sequence, plan, time, at, scale, &shape)
                .is_some()
    }

    /// How far past its frame a clip in depth is drawn at sequence time `at`, as the scene then sees
    /// it ([`Drawing::reach`]), for deciding where the frame is drawn before it is.
    fn gpu_reach(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        at: Time,
        scale: f64,
    ) -> Option<Reach> {
        if !plan.three_d {
            return None;
        }
        let size = output_size(sequence.width, sequence.height, scale);
        let scene = three_d::Scene::at(sequence, at, size);
        self.reach(sequence, plan, at, scale, Some(&scene))
    }

    /// The numbers [`Drawing::input`] works out for a clip's picture at sequence time `at`, a
    /// generator's reaching as far past its frame as `reach` says.
    fn gpu_shape(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        at: Time,
        scale: f64,
        reach: Option<Reach>,
    ) -> Shape {
        let time = plan.clip.time_at(at);
        let canvas = output_size(sequence.width, sequence.height, scale);
        let past = match &plan.source {
            Source::Title(_) | Source::Sound(_) | Source::Drawn(_) => {
                reach.map(|reach| reach.run(time, canvas, scale))
            }
            _ => None,
        };
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
            Source::Solid(_) | Source::Title(_) | Source::Sound(_) | Source::Drawn(_) => {
                match (past, reach) {
                    (Some(run), Some(reach)) => (run.picture(), scale * reach.per),
                    _ => (canvas, scale),
                }
            }
            // It works on the canvas beneath it, unplaced.
            Source::Adjustment | Source::Nothing | Source::Mesh => {
                return Shape {
                    source: canvas,
                    source_per: scale,
                    size: canvas,
                    per_source: scale,
                    to_output: Affine::IDENTITY,
                    past: None,
                }
            }
        };
        let place = sequence.placement(plan, at).scaled(scale).to_output;
        if plan.letterboxed {
            Shape {
                source,
                source_per: per_source,
                size: canvas,
                per_source: place.determinant().abs().sqrt(),
                to_output: Affine::IDENTITY,
                past: None,
            }
        } else {
            let (x, y) = past.map_or((0.0, 0.0), |run| (run.beyond.0 as f64, run.beyond.1 as f64));
            Shape {
                source,
                source_per: per_source,
                size: source,
                per_source,
                to_output: place
                    .then_apply(&Affine::scale(1.0 / per_source, 1.0 / per_source))
                    .then_apply(&Affine::translate(-x, -y)),
                past,
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
            if video::instance::leaves_alone(effect, time, self.working()) {
                continue;
            }
            // Bringing a whole picture under 1 goes by its peak, which only the CPU knows before it
            // draws; each pixel under 1 needs no such thing.
            let way = if self.hdr.is_some() {
                video::catalog::headroom(effect.fx.as_str())
            } else {
                Headroom::Keeps
            };
            if way == Headroom::Whole {
                return None;
            }
            let before = ops.len();
            // The effect says what it is on the GPU, as it says what it is on the CPU: one this
            // crate does not find, or that has no kernel as it is set, sends the frame there.
            let track = |id: &str| {
                sequence
                    .tracks
                    .iter()
                    .position(|track| track.id == id)
                    .map(|place| place + 1)
            };
            let timecode = format_timecode(sequence.start + at, sequence.rate, sequence.drop_frame);
            let run = kerf_fx::picture::KernelRun {
                at: time,
                timecode: &timecode,
                size: shape.size,
                per_source: shape.per_source,
                per_canvas: density,
                seed: kerf_core::raster::seed(plan.clip.id.as_str()).wrapping_add(index as u32),
                working: self.working(),
                clip_name: &plan.name,
                frame_rate: sequence.rate.fps(),
                files: &self.files,
                track: &track,
            };
            match kerf_fx::picture::kernel(effect.fx.as_str())?.on_gpu(effect, &run)? {
                kerf_fx::picture::OnGpu::Nothing => {}
                kerf_fx::picture::OnGpu::Effect(op) => ops.push(Op::Effect(op)),
                kerf_fx::picture::OnGpu::Correct(correction) => ops.push(Op::Correct(correction)),
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
                        clears: false,
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
        if matches!(plan.source, Source::Nothing) {
            return Some(canvas);
        }
        if matches!(plan.source, Source::Adjustment) {
            let shape = self.gpu_shape(sequence, plan, at, scale, None);
            let ops = self.gpu_ops(sequence, plan, clip.time_at(at), at, scale, &shape)?;
            return self.gpu_adjust(frame, canvas, (sequence, plan, at, scale), &ops);
        }
        let size = (canvas.w, canvas.h);
        if matches!(plan.source, Source::Mesh) {
            // Drawn on the CPU, as the scene sees it, and laid over what is there.
            let moment = Moment {
                at,
                scale,
                scene: depth.as_deref().map(|depth| &depth.scene),
                clears: false,
            };
            let Some(mesh) =
                self.mesh(sequence, shown, &moment, (size.0 as usize, size.1 as usize))
            else {
                return Some(canvas);
            };
            let drawn = frame.image(&mesh.picture);
            let whole = Rect::new(0.0, 0.0, f64::from(size.0), f64::from(size.1));
            let over = frame.composite(
                canvas,
                &drawn,
                &Affine::IDENTITY,
                whole,
                mesh.opacity,
                blend,
            );
            frame.release(drawn);
            return Some(over);
        }
        if let (true, Some(depth)) = (plan.three_d, depth) {
            let ready = match depth.ready.iter().position(|(id, _)| *id == clip.id) {
                Some(place) => Some(depth.ready.swap_remove(place).1),
                None => {
                    let scene = Some(&depth.scene);
                    self.gpu_ready(frame, sequence, shown, (at, scale), size, scene)?
                }
            };
            // Media that cannot be read draws nothing, as on the CPU.
            let Some(ready) = ready else {
                return Some(canvas);
            };
            let drawn = self.gpu_project(frame, depth, plan, &ready, size, None)?;
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
        let Some(ready) = self.gpu_ready(frame, sequence, shown, (at, scale), size, None)? else {
            return Some(canvas);
        };
        let whole = Rect::new(
            0.0,
            0.0,
            f64::from(ready.picture.w),
            f64::from(ready.picture.h),
        );
        let kept = ready.crop.unwrap_or(whole);
        // A picture that moves while its shutter is open is smeared over where it goes; a pass,
        // which says one thing of each pixel, is not.
        let smeared = (self.pass == Pass::Picture)
            .then(|| sequence.smeared(plan, &ready.placement, scale, kept))
            .flatten();
        let canvas = match smeared {
            Some(places) => {
                frame.smeared(canvas, &ready.picture, &places, kept, ready.opacity, blend)
            }
            None => frame.composite(
                canvas,
                &ready.picture,
                &ready.placement.to_output,
                kept,
                ready.opacity,
                blend,
            ),
        };
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
                let Some(one) =
                    self.gpu_ready(frame, plan, shown, (at, scale), canvas, Some(&scene))?
                else {
                    continue;
                };
                let time = one.placement.time();
                let (pw, ph) = (f64::from(one.picture.w), f64::from(one.picture.h));
                let plane = three_d::Plane::of(&scene, &shown.plan, (pw, ph), &one.placement);
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
            if let Some(one) = self.gpu_ready(frame, plan, shown, (at, scale), canvas, None)? {
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

    /// A clip in depth, as `ready` has it, on a picture of `size`, transparent around it, out of focus
    /// as far as the camera has it.
    fn gpu_project(
        &self,
        frame: &mut GpuFrame<'static>,
        depth: &mut Depth,
        plan: &ClipPlan,
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
        let time = ready.placement.time();
        let plane = three_d::Plane::of(scene, plan, (pw, ph), &ready.placement);
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
            if matches!(shown.plan.source, Source::Mesh) {
                // Drawn on the CPU, as the scene sees it, and laid with the rest.
                let moment = Moment {
                    at,
                    scale,
                    scene: Some(&depth.scene),
                    clears: false,
                };
                let Some(mesh) =
                    self.mesh(sequence, shown, &moment, (size.0 as usize, size.1 as usize))
                else {
                    continue;
                };
                let drawn = frame.image(&mesh.picture);
                frame.lay_depths(&layers, &mesh.depth, size.0);
                frame.lay(&mut layers, &drawn, mesh.opacity, shown.plan.blend);
                frame.release(drawn);
                continue;
            }
            let ready = match depth.ready.iter().position(|(kept, _)| kept == id) {
                Some(place) => Some(depth.ready.swap_remove(place).1),
                None => {
                    let scene = Some(&depth.scene);
                    self.gpu_ready(frame, sequence, shown, (at, scale), size, scene)?
                }
            };
            let Some(ready) = ready else {
                continue;
            };
            let depths = layers.next_depths();
            let drawn = self.gpu_project(frame, depth, &shown.plan, &ready, size, Some(depths))?;
            frame.lay(&mut layers, &drawn, ready.opacity, shown.plan.blend);
            frame.release(drawn);
            frame.release(ready.picture);
        }
        frame.by_depth(canvas, layers)
    }

    /// A shown clip's picture with its effects and reveal done, as [`Drawing::ready`] makes it, a
    /// generator in depth reaching as far past its frame as `scene` may see of it. `None` when the
    /// frame has to go to the CPU, and `Some(None)` when the clip draws nothing.
    fn gpu_ready(
        &self,
        frame: &mut GpuFrame<'static>,
        sequence: &SequencePlan,
        shown: &Shown,
        (at, scale): (Time, f64),
        canvas: (u32, u32),
        scene: Option<&three_d::Scene>,
    ) -> Option<Option<GpuReady>> {
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = clip.time_at(at);
        let reach = self.reach(sequence, plan, at, scale, scene);
        let shape = self.gpu_shape(sequence, plan, at, scale, reach);
        let ops = self.gpu_ops(sequence, plan, time, at, scale, &shape)?;
        // Media that cannot be read draws nothing, as on the CPU.
        let Some(picture) = self.gpu_source(frame, sequence, plan, time, scale, &shape)? else {
            return Some(None);
        };
        let mut picture = picture;
        let size = (f64::from(plan.size.0), f64::from(plan.size.1));
        let source_per = shape.source_per;
        let past = shape.past.as_ref().map(frame_in);
        let mut crop = match past {
            Some(past) => {
                let whole = Rect::new(0.0, 0.0, f64::from(picture.w), f64::from(picture.h));
                crop_past(plan, time, past, whole)
            }
            None => crop_rect(
                plan,
                time,
                size,
                source_per,
                (picture.w as usize, picture.h as usize),
            ),
        };
        if plan.letterboxed {
            let place = sequence.placement(plan, at).scaled(scale).to_output;
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
            let at = past.map_or((0.0, 0.0), |past| (past.x, past.y));
            let masks = past_frame(mask::prepare(&clip.masks, size, source_per, time), at);
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
                    video::vfx::gpu::Kernel::held(frame, picture, |frame, unit| {
                        Some(frame.correct(unit, correction))
                    })?
                }
            };
        }
        let mut to_output = shape.to_output;
        let mut past = past;
        let tall = past.map_or(f64::from(plan.size.1) * source_per, |past| past.h);
        let bordered = border(plan, time, tall);
        if let Some((width, color)) = bordered {
            let whole = Rect::new(0.0, 0.0, f64::from(picture.w), f64::from(picture.h));
            let kept = crop.unwrap_or(whole);
            let pad = width.ceil();
            let card = Rect::new(
                kept.x + pad - width,
                kept.y + pad - width,
                kept.w + 2.0 * width,
                kept.h + 2.0 * width,
            );
            let (fw, fh) = (picture.w + 2 * pad as u32, picture.h + 2 * pad as u32);
            let solid = frame.filled(fw, fh, color);
            let framed = frame.clear(fw, fh);
            let framed =
                frame.composite(framed, &solid, &Affine::IDENTITY, card, 1.0, Blend::Normal);
            frame.release(solid);
            let moved = Affine::translate(pad, pad);
            let framed = frame.composite(framed, &picture, &moved, kept, 1.0, Blend::Normal);
            frame.release(picture);
            picture = framed;
            to_output = to_output.then_apply(&Affine::translate(-pad, -pad));
            crop = Some(card);
            // A wipe goes across the card.
            past = Some(card);
        }
        let reveal = shown
            .reveal
            .map(|(reveal, start, end)| (reveal, progress(at, start, end)));
        if let Some((Reveal::Wipe { angle, softness }, progress)) = reveal {
            // Across its frame, wherever that is in its picture.
            let (pw, ph) = (picture.w as usize, picture.h as usize);
            let covered = match past {
                Some(past) => wipe::coverage_within(pw, ph, past, angle, softness, progress),
                None => wipe::coverage(pw, ph, angle, softness, progress),
            };
            picture = frame.multiply(picture, &covered);
        }
        let fade = match reveal {
            Some((Reveal::Dissolve, progress)) => progress,
            _ => 1.0,
        };
        let opacity = (number(&param(clip, "opacity").value_at(time)) * fade) as f32;
        let opaque = ops.is_empty()
            && reveal.is_none()
            && bordered.is_none()
            && clip.masks.is_empty()
            && match &plan.source {
                Source::Solid(color) => color.a == 255,
                _ => plan.letterboxed,
            };
        Some(Some(GpuReady {
            picture,
            placement: Placement::of(plan, at, to_output),
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
        let (sequence, plan, at, scale) = placed;
        let (w, h) = (canvas.w as usize, canvas.h as usize);
        let mut adjusted = frame.copy(&canvas);
        for op in ops {
            adjusted = match op {
                Op::Effect(op) => op.run(frame, adjusted)?,
                Op::Held(op) => op.run_held(frame, adjusted)?,
                Op::Gamut(op) => op.run_in_gamut(frame, adjusted)?,
                Op::Correct(correction) => frame.correct(adjusted, correction),
                Op::HeldCorrect(correction) => {
                    video::vfx::gpu::Kernel::held(frame, adjusted, |frame, unit| {
                        Some(frame.correct(unit, correction))
                    })?
                }
            };
        }
        let placement = sequence.placement(plan, at).scaled(scale);
        let time = placement.time();
        let size = (f64::from(plan.size.0), f64::from(plan.size.1));
        let mut weight = frame.filled(canvas.w, canvas.h, [1.0; 4]);
        let masks = mask::prepare(&plan.clip.masks, size, scale, time);
        if let Some(covered) = mask::coverage(&masks, w, h) {
            weight = frame.multiply(weight, &covered);
        }
        let place = placement
            .to_output
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
        let run = shape.past.unwrap_or_else(|| SourceRun::new(time, (cw, ch)));
        Some(Some(match &plan.source {
            Source::Media { picture } => {
                let (w, h) = shape.source;
                let size = (w as u32, h as u32);
                let Some(decoded) = self.pictures.decoded(*picture, time, size) else {
                    return Some(None);
                };
                // A frame as it decoded, in linear light.
                let mut taken = |rgba: &ffmpeg_next::frame::Video| {
                    if (rgba.width(), rgba.height()) != size {
                        return None;
                    }
                    let stride = rgba.stride(0) as u32;
                    Some(match (decoded.deep, crate::footage::yuv_of(rgba)) {
                        (Some(input), Some(yuv)) => frame.yuv16(&yuv, input),
                        (None, Some(yuv)) => frame.yuv(&yuv),
                        (Some(input), None) => {
                            frame.rgba16(rgba.data(0), size, stride, decoded.alpha, input)
                        }
                        (None, None) => {
                            frame.rgba8(rgba.data(0), size.0, size.1, stride, decoded.alpha)
                        }
                    })
                };
                let shown = taken(&decoded.picture)?;
                match &decoded.toward {
                    // Between two frames: each in linear light, then the one toward the other.
                    Some((next, along)) => {
                        let next = taken(next)?;
                        let between = frame.between(shown, &next, *along);
                        frame.release(next);
                        between
                    }
                    None => shown,
                }
            }
            Source::Nested {
                sequence: nested,
                map,
                angle,
            } => {
                let inner = &self.sequences[*nested];
                let shown = map.nested(time, inner.rate);
                self.gpu_draw(frame, *nested, shown, scale, angle.as_ref())?
            }
            Source::Solid(color) => frame.filled(cw as u32, ch as u32, premultiplied(*color)),
            Source::Title(title) => {
                let key = title_key(title, &run);
                frame.image_kept(&key, || title_on(title, &run))
            }
            Source::Sound(show) => frame.image(&self.sound_on(sequence, plan, show, &run)),
            Source::Drawn(source) => frame.image(&source.draw(&plan.clip, &run)),
            Source::Adjustment | Source::Nothing | Source::Mesh => return None,
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
