//! A clip's picture made ready to lay down: drawn from its source, a generator in depth as far past
//! its frame as the camera may see of it ([`Reach`]), masked and cropped or letterboxed on black as
//! the graph pads it, and with its effects, its reveal, and the pass drawn done to it.

use super::*;

/// A clip's picture with all done to it that is done to the picture itself, and how it goes down.
#[derive(Clone)]
pub(super) struct Ready {
    pub(super) picture: Arc<Image>,
    /// Its pixels to output pixels.
    pub(super) to_output: Affine,
    /// What its crop leaves of it, in its pixels.
    pub(super) crop: Option<Rect>,
    pub(super) opacity: f32,
    /// Where it was shared with the drawings of other passes before this one's was made of it,
    /// which share what the camera sees of it too.
    pub(super) share: Option<ReadyKey>,
}

/// A clip's picture as its effects get it.
pub(super) struct Input {
    pub(super) picture: Arc<Image>,
    /// Its pixels to output pixels.
    pub(super) to_output: Affine,
    /// What its crop leaves of it, in its pixels.
    pub(super) crop: Option<Rect>,
    /// How many of its pixels a source pixel is.
    pub(super) per_source: f64,
    /// Where its frame is in it, in its pixels, for a generator in depth whose picture reaches past
    /// it ([`Reach`]).
    pub(super) frame: Option<Rect>,
}

/// The most pixels a generator's picture in depth that reaches past its frame is drawn with, as
/// canvases of them: one that would be larger is drawn coarser.
pub(super) const MOST_CANVASES_BEYOND: f64 = 4.0;
/// And at most this many, however large the canvas.
pub(super) const MOST_PIXELS_BEYOND: f64 = 4096.0 * 4096.0;
/// And no longer than this on either side, which any GPU holds.
pub(super) const LONGEST_BEYOND: f64 = 8192.0;
/// How far past the frame what a camera sees of a clip is read, in output pixels: as far as a pixel
/// at the frame's edge reads of its picture.
pub(super) const SEEN_PAST: f64 = 2.0;

/// How far past its frame a generator's picture in depth reaches, so that all of its outline that
/// is to be seen is in it ([`Drawing::reach`]). The frame stays in its middle, so that the clip turns
/// about the same point as ever.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Reach {
    /// How far past the frame on each side, across and down, in the frame's pixels.
    pub(super) beyond: (f64, f64),
    /// What share of the canvas's pixels across it is drawn with: under 1 where it reaches so far
    /// that it would otherwise be too large.
    pub(super) per: f64,
}

impl Reach {
    /// What its generator is drawn on at clip time `time`, for a canvas of `canvas` pixels drawn at
    /// `scale` of the sequence: the canvas at its share, and that many of its pixels past it.
    pub(super) fn run(&self, time: Time, (cw, ch): (usize, usize), scale: f64) -> SourceRun {
        let pixels = |frame: usize| ((frame as f64 * self.per).round() as usize).max(1);
        let past = |beyond: f64| (beyond * scale * self.per).ceil() as usize;
        SourceRun {
            at: time,
            size: (pixels(cw), pixels(ch)),
            beyond: (past(self.beyond.0), past(self.beyond.1)),
        }
    }
}

impl Drawing {
    /// How far past its frame a generator in depth is drawn at clip time `time` and `scale`, so that
    /// all of it that the camera of `scene` may see is drawn: as far as its own outline goes, cut where
    /// its crop cuts, and no further than the camera sees. All of its outline where its effects run on
    /// its picture, which is not to change as the camera moves, and where a light casts shadows, which
    /// what is out of sight may cast too.
    ///
    /// None for a clip out of depth, for a solid, which is its frame, for a generator that cannot say
    /// where it draws, as particles cannot, and for one that stays within its frame. Its picture is
    /// drawn coarser where it reaches so far that it would be too large.
    pub(super) fn reach(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
        scene: Option<&three_d::Scene>,
    ) -> Option<Reach> {
        let scene = scene.filter(|_| plan.three_d && !plan.letterboxed)?;
        let size = (plan.size.0 as usize, plan.size.1 as usize);
        let own = match &plan.source {
            Source::Title(title) => title_extent(title, size, time)?,
            Source::Sound(show) => sound_extent(show, size),
            Source::Drawn(source) => source.extent(&plan.clip, &SourceRun::new(time, size))?.rect,
            _ => return None,
        };
        let (w, h) = (f64::from(plan.size.0), f64::from(plan.size.1));
        let frame = Rect::new(0.0, 0.0, w, h);
        let kept = crop_past(plan, time, frame, own).unwrap_or(own);
        if kept.w <= 0.0 || kept.h <= 0.0 {
            return None;
        }
        let (cw, ch) = output_size(sequence.width, sequence.height, scale);
        let kept = if plan.effects.is_empty() && !scene.casts_shadows() {
            let place =
                Affine::scale(scale, scale).then_apply(&self.geometry(sequence, plan, time));
            let plane = three_d::Plane::of(scene, plan, (w, h), &place, time);
            let deep = three_d::depth_of(plan, time, ch as f64);
            three_d::seen_of(scene, &plane, deep, kept, SEEN_PAST)?
        } else {
            kept
        };
        // And a pixel more, for its edge to be read whole.
        let past = |low: f64, high: f64, side: f64| {
            let past = (-low).max(high - side);
            if past > 1e-6 {
                past + 1.0
            } else {
                0.0
            }
        };
        let beyond = (
            past(kept.x, kept.right(), w),
            past(kept.y, kept.bottom(), h),
        );
        if beyond == (0.0, 0.0) {
            return None;
        }
        let (pw, ph) = ((w + 2.0 * beyond.0) * scale, (h + 2.0 * beyond.1) * scale);
        let most = (cw as f64 * ch as f64 * MOST_CANVASES_BEYOND).min(MOST_PIXELS_BEYOND);
        let per = (most / (pw * ph))
            .sqrt()
            .min(LONGEST_BEYOND / pw.max(ph))
            .min(1.0);
        Some(Reach { beyond, per })
    }

    /// A shown clip's picture at sequence time `at` with its effects and reveal done, and the pass
    /// drawn made of it, and how it goes down, for a canvas of `(w, h)`. A generator in depth is
    /// drawn as far past its frame as `scene` may see of it ([`Self::reach`]). What is shared with the
    /// drawings of other passes is drawn once for them all.
    pub(super) fn ready(
        &self,
        sequence: &SequencePlan,
        shown: &Shown,
        at: Time,
        scale: f64,
        size: (usize, usize),
        scene: Option<&three_d::Scene>,
    ) -> Option<Ready> {
        let plan = &shown.plan;
        let reach = self.reach(sequence, plan, at - plan.clip.start, scale, scene);
        let before = || self.before_pass(sequence, shown, (at, scale), size, reach);
        let mut ready = match self.share_of(sequence, shown, (at, scale), size, reach) {
            Some(share) => self.shared_ready(share, || {
                before().map(|ready| Ready {
                    share: Some(share.ready),
                    ..ready
                })
            })?,
            None => before()?,
        };
        if self.pass != Pass::Picture {
            let picture = Arc::make_mut(&mut ready.picture);
            self.as_pass(sequence, plan, picture, &ready.to_output, (at, scale), size);
        }
        Some(ready)
    }

    /// [`Self::ready`] before the pass drawn is made of it, drawn `reach` past its frame.
    fn before_pass(
        &self,
        sequence: &SequencePlan,
        shown: &Shown,
        (at, scale): (Time, f64),
        (w, h): (usize, usize),
        reach: Option<Reach>,
    ) -> Option<Ready> {
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        let Input {
            mut picture,
            mut to_output,
            mut crop,
            per_source,
            mut frame,
        } = self.input(sequence, plan, time, scale, (w, h), reach)?;
        if !plan.effects.is_empty() {
            let placed = Placed {
                sequence,
                at,
                scale,
                to_output,
                per_source,
                env: true,
                reach,
            };
            self.effects(
                Arc::make_mut(&mut picture),
                plan,
                time,
                &placed,
                plan.effects.len(),
            );
        }
        let tall = frame.map_or(f64::from(plan.size.1) * per_source, |frame| frame.h);
        if let Some((width, color)) = border(plan, time, tall) {
            let (framed, pad, card) = bordered(&picture, crop, width, color);
            picture = Arc::new(framed);
            to_output = to_output.then_apply(&Affine::translate(-pad, -pad));
            crop = Some(card);
            // A wipe goes across the card.
            frame = Some(card);
        }
        let reveal = shown
            .reveal
            .map(|(reveal, start, end)| (reveal, progress(at, start, end)));
        if let Some((Reveal::Wipe { angle, softness }, progress)) = reveal {
            // Across its frame, wherever that is in its picture.
            let (pw, ph) = (picture.w, picture.h);
            let covered = match frame {
                Some(frame) => wipe::coverage_within(pw, ph, frame, angle, softness, progress),
                None => wipe::coverage(pw, ph, angle, softness, progress),
            };
            Arc::make_mut(&mut picture).mul_coverage(&covered);
        }
        let fade = match reveal {
            Some((Reveal::Dissolve, progress)) => progress,
            _ => 1.0,
        };
        let opacity = (number(&param(clip, "opacity").value_at(time)) * fade) as f32;
        Some(Ready {
            picture,
            to_output,
            crop,
            opacity,
            share: None,
        })
    }

    /// A clip's picture at clip time `time` as its effects get it, on a
    /// `canvas` of output pixels: masked and cropped, or, letterboxed, on
    /// black the size of the canvas. A generator is drawn as far past its
    /// frame as `reach` says.
    pub(super) fn input(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
        (w, h): (usize, usize),
        reach: Option<Reach>,
    ) -> Option<Input> {
        // Shared with the frame cache until something changes it.
        let (mut picture, per_source) = self.picture(sequence, plan, time, scale, reach)?;
        let size = (f64::from(plan.size.0), f64::from(plan.size.1));
        let place = Affine::scale(scale, scale).then_apply(&self.geometry(sequence, plan, time));
        let frame = reach.map(|reach| {
            let canvas = output_size(sequence.width, sequence.height, scale);
            frame_in(&reach.run(time, canvas, scale))
        });
        let mut crop = match frame {
            Some(frame) => {
                let whole = Rect::new(0.0, 0.0, picture.w as f64, picture.h as f64);
                crop_past(plan, time, frame, whole)
            }
            None => crop_rect(plan, time, size, per_source, (picture.w, picture.h)),
        };
        Some(if plan.letterboxed {
            // The fitted picture on black, the size of the canvas.
            let mut boxed = Image::filled(w, h, [0.0, 0.0, 0.0, 1.0]);
            let whole = Rect::new(0.0, 0.0, picture.w as f64, picture.h as f64);
            composite_placed(
                &mut boxed,
                &picture,
                &place.then_apply(&Affine::scale(1.0 / per_source, 1.0 / per_source)),
                crop.take().unwrap_or(whole),
                1.0,
                Blend::Normal,
            );
            Input {
                picture: Arc::new(boxed),
                to_output: Affine::IDENTITY,
                crop,
                per_source: place.determinant().abs().sqrt(),
                frame: None,
            }
        } else {
            let (x, y) = frame.map_or((0.0, 0.0), |frame| (frame.x, frame.y));
            let masks = past_frame(
                mask::prepare(&plan.clip.masks, size, per_source, time),
                (x, y),
            );
            if let Some(covered) = mask::coverage(&masks, picture.w, picture.h) {
                Arc::make_mut(&mut picture).mul_coverage(&covered);
            }
            Input {
                picture,
                to_output: place
                    .then_apply(&Affine::scale(1.0 / per_source, 1.0 / per_source))
                    .then_apply(&Affine::translate(-x, -y)),
                crop,
                per_source,
                frame,
            }
        })
    }

    /// A clip's picture at clip time `time` before its masks and effects,
    /// with how many of its pixels a source pixel is. A generator is drawn
    /// as far past its frame as `reach` says, at its share of the canvas.
    pub(super) fn picture(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
        reach: Option<Reach>,
    ) -> Option<(Arc<Image>, f64)> {
        let (cw, ch) = output_size(sequence.width, sequence.height, scale);
        let (run, per) = match reach {
            Some(reach) => (reach.run(time, (cw, ch), scale), scale * reach.per),
            None => (SourceRun::new(time, (cw, ch)), scale),
        };
        Some(match &plan.source {
            Source::Media { picture } => {
                let factor = plan.shown * scale;
                let size = (
                    ((f64::from(plan.size.0) * factor).round() as u32).max(1),
                    ((f64::from(plan.size.1) * factor).round() as u32).max(1),
                );
                let decoded = self.pictures.picture(*picture, time, size)?;
                let per_source = decoded.w as f64 / f64::from(plan.size.0);
                (decoded, per_source)
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
                let image = self.draw_of(*nested, shown, scale, angle.as_ref());
                let per_source = image.w as f64 / f64::from(inner.width);
                (Arc::new(image), per_source)
            }
            Source::Solid(color) => (
                Arc::new(Image::filled(cw, ch, premultiplied(*color))),
                scale,
            ),
            Source::Title(title) => (Arc::new(title_on(title, &run)), per),
            Source::Sound(show) => (Arc::new(self.sound_on(sequence, plan, show, &run)), per),
            Source::Drawn(source) => (Arc::new(source.draw(&plan.clip, &run)), per),
            Source::Adjustment | Source::Nothing | Source::Mesh => return None,
        })
    }
}

/// How far past what its crop leaves on each side, in its picture's pixels, and of what color,
/// linear and premultiplied, the card behind a clip's picture is at clip time `time`, for a
/// picture whose frame is `tall` of its pixels: none without a `border`, or letterboxed, where its
/// picture is the canvas.
pub(super) fn border(plan: &ClipPlan, time: Time, tall: f64) -> Option<(f64, [f32; 4])> {
    if plan.letterboxed {
        return None;
    }
    let width = number(&param(&plan.clip, "border").value_at(time)) * tall;
    if width.is_nan() || width <= 0.0 {
        return None;
    }
    let color = match param(&plan.clip, "border_color").value_at(time) {
        Value::Color(color) => color,
        _ => Rgba::WHITE,
    };
    Some((width, kerf_draw::source::premultiplied(color)))
}

/// `picture` on a card of `color` `width` of its pixels past what `crop` leaves of it on each
/// side: the picture that makes, how far into it `picture` moved across and down, and the card's
/// place in it.
fn bordered(
    picture: &Image,
    crop: Option<Rect>,
    width: f64,
    color: [f32; 4],
) -> (Image, f64, Rect) {
    let whole = Rect::new(0.0, 0.0, picture.w as f64, picture.h as f64);
    let kept = crop.unwrap_or(whole);
    let pad = width.ceil();
    let card = Rect::new(
        kept.x + pad - width,
        kept.y + pad - width,
        kept.w + 2.0 * width,
        kept.h + 2.0 * width,
    );
    let mut framed = Image::new(picture.w + 2 * pad as usize, picture.h + 2 * pad as usize);
    let solid = Image::filled(framed.w, framed.h, color);
    composite_placed(
        &mut framed,
        &solid,
        &Affine::IDENTITY,
        card,
        1.0,
        Blend::Normal,
    );
    let moved = Affine::translate(pad, pad);
    composite_placed(&mut framed, picture, &moved, kept, 1.0, Blend::Normal);
    (framed, pad, card)
}
