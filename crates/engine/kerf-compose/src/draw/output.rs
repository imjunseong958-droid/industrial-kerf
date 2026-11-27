//! A program's frames as they leave it: kept in memory and on disk by a number for all they are
//! drawn from ([`Drawing::frame_key`]), one clip alone untouched given as decoded, drawn on the GPU
//! where it can draw them and else on the CPU, and brought out of linear light, at 8 bits or 16,
//! a depth pass stretched to its own nearest and farthest and a matte made of an ids pass.

use super::*;

/// A motion pass's picture with every pixel a thing's or nothing's. One that things cover half
/// of or more says how far they go, as if they covered it all, and one they cover less of says
/// nothing: left as it is drawn, a pixel part covered over nothing is a darker color, which reads
/// as something going up and to the left.
fn whole_or_nothing(image: &mut Image) {
    image.px.par_chunks_mut(4).for_each(|pixel| {
        let covered = pixel[3];
        if covered >= 0.5 {
            for channel in &mut pixel[..3] {
                *channel /= covered;
            }
            pixel[3] = 1.0;
        } else {
            pixel.fill(0.0);
        }
    });
}

/// A frame being drawn, which the drawings that share their work are told the drawing is done
/// with once it is, whichever way it was drawn ([`Drawing::done_with`]).
struct DoneWith<'a>(&'a Drawing, Time);

impl Drop for DoneWith<'_> {
    fn drop(&mut self) {
        self.0.done_with(self.1);
    }
}

impl Drawing {
    /// The picture at sequence time `at` over black, at `scale` of the
    /// canvas (1 is full size).
    pub fn frame(&self, at: Time, scale: f64) -> Frame {
        let _done = DoneWith(self, at);
        let key = (self.keep_frames || self.disk.is_some())
            .then(|| self.frame_key(at, scale))
            .flatten();
        let kept = key.filter(|_| self.keep_frames);
        if let Some(frame) = kept.and_then(frames::get) {
            return frame;
        }
        let on_disk = self.disk_key(key, at, scale);
        if let Some((disk, key)) = self.disk.as_ref().zip(on_disk)
            && let Some(frame) = disk.get(key, false)
        {
            if let Some(kept) = kept {
                frames::put(kept, frame.clone());
            }
            return frame;
        }
        let frame = self.draw_frame(at, scale);
        if let Some(kept) = kept {
            frames::put(kept, frame.clone());
        }
        if let Some((disk, key)) = self.disk.as_ref().zip(on_disk) {
            disk.put(key, false, frame.clone());
        }
        frame
    }

    /// See [`Compositor::flush_frames`](crate::Compositor::flush_frames).
    pub fn flush_frames(&self) {
        if let Some(disk) = &self.disk {
            disk.flush();
        }
    }

    /// A frame's key on disk: longer than in memory, since it stays longer and there are more.
    pub(super) fn disk_key(&self, key: Option<u64>, at: Time, scale: f64) -> Option<u128> {
        self.disk.as_ref()?;
        let first = key?;
        let mut hash = DefaultHasher::new();
        first.hash(&mut hash);
        scale.clamp(1.0 / 16.0, 1.0).to_bits().hash(&mut hash);
        self.hdr.as_ref().map(|output| output.pq).hash(&mut hash);
        self.hash_sequence(&mut hash, 0, at)?;
        Some(u128::from(first) << 64 | u128::from(hash.finish()))
    }

    /// See [`Compositor::frame16`](crate::Compositor::frame16).
    pub fn frame16(&self, at: Time, scale: f64) -> Frame {
        let _done = DoneWith(self, at);
        let key = self
            .disk
            .is_some()
            .then(|| self.frame_key(at, scale))
            .flatten();
        let on_disk = self.disk_key(key, at, scale);
        if let Some((disk, key)) = self.disk.as_ref().zip(on_disk)
            && let Some(frame) = disk.get(key, true)
        {
            return frame;
        }
        let frame = self.draw_frame16(at, scale);
        if let Some((disk, key)) = self.disk.as_ref().zip(on_disk) {
            disk.put(key, true, frame.clone());
        }
        frame
    }

    /// The frame at `at` as the GPU draws it, or `None` where [`Drawing::frame`] would give it to the
    /// CPU, even partway through.
    pub(crate) fn frame_on_gpu(&self, at: Time, scale: f64) -> Option<Frame> {
        self.gpu_frame(at, scale.clamp(1.0 / 16.0, 1.0))
    }

    pub(super) fn draw_frame16(&self, at: Time, scale: f64) -> Frame {
        let scale = scale.clamp(1.0 / 16.0, 1.0);
        if let Some(frame) = self.gpu_frame16(at, scale) {
            return frame;
        }
        let mut image = self.draw(0, at, scale);
        if self.pass == Pass::Motion {
            whole_or_nothing(&mut image);
        }
        let mut rgba = match &self.hdr {
            Some(output) if self.pass == Pass::Picture => {
                image.to_signal16_over_black(|c| output.signal(c))
            }
            _ => image.to_rgba16_over_black(),
        };
        if self.pass == Pass::Relative {
            stretch_to_its_own16(&mut rgba);
        }
        if !self.matte.is_empty() {
            rgba.par_chunks_mut(8).for_each(|pixel| {
                let byte = |at: usize| {
                    let code = u16::from_le_bytes([pixel[at * 2], pixel[at * 2 + 1]]);
                    ((u32::from(code) * 255 + 32_767) / 65_535) as u8
                };
                let its = self
                    .matte
                    .iter()
                    .any(|color| (0..3).all(|at| color[at].abs_diff(byte(at)) <= NEAR));
                pixel[..6].fill(if its { u8::MAX } else { 0 });
            });
        }
        Frame {
            width: image.w as u32,
            height: image.h as u32,
            rgba,
        }
    }

    /// What effects are told the picture is.
    pub(crate) fn working(&self) -> WorkingSpace {
        match &self.hdr {
            Some(output) if output.pq => WorkingSpace::Rec2100Pq,
            Some(_) => WorkingSpace::Rec2100Hlg,
            None => WorkingSpace::Rec709,
        }
    }

    pub(super) fn draw_frame(&self, at: Time, scale: f64) -> Frame {
        if scale >= 1.0
            && self.hdr.is_none()
            && self.pass == Pass::Picture
            && let Some(frame) = self.plain_frame(at)
        {
            return frame;
        }
        let scale = scale.clamp(1.0 / 16.0, 1.0);
        if let Some(frame) = self.gpu_frame(at, scale) {
            return frame;
        }
        let mut image = self.draw(0, at, scale);
        if self.pass == Pass::Motion {
            whole_or_nothing(&mut image);
        }
        // A pass is numbers, which no display is to be fitted to.
        if let Some(output) = self.hdr.as_ref().filter(|_| self.pass == Pass::Picture) {
            image.px.par_chunks_mut(4).for_each(|pixel| {
                let sdr = output.sdr([pixel[0], pixel[1], pixel[2]]);
                pixel[..3].copy_from_slice(&sdr);
            });
        }
        let mut rgba = image.to_rgba8_over_black();
        if self.pass == Pass::Relative {
            stretch_to_its_own(&mut rgba);
        }
        if !self.matte.is_empty() {
            rgba.par_chunks_mut(4).for_each(|pixel| {
                let its = self.matte.iter().any(|color| {
                    color
                        .iter()
                        .zip(pixel.iter())
                        .all(|(a, b)| a.abs_diff(*b) <= NEAR)
                });
                pixel[..3].fill(if its { u8::MAX } else { 0 });
            });
        }
        Frame {
            width: image.w as u32,
            height: image.h as u32,
            rgba,
        }
    }

    /// See [`Compositor::frame_key`](crate::Compositor::frame_key).
    pub fn frame_key(&self, at: Time, scale: f64) -> Option<u64> {
        let mut hash = DefaultHasher::new();
        scale.clamp(1.0 / 16.0, 1.0).to_bits().hash(&mut hash);
        self.hdr.as_ref().map(|output| output.pq).hash(&mut hash);
        self.captions.hash(&mut hash);
        // Only where it is not the picture, so that a picture's number is what it was.
        if self.pass != Pass::Picture {
            self.pass.hash(&mut hash);
        }
        // A motion pass is of how far things go in a frame, however long that is, by the
        // height of what the file keeps of it.
        if self.pass == Pass::Motion {
            self.step.flicks().hash(&mut hash);
            self.kept.to_bits().hash(&mut hash);
        }
        self.matte.hash(&mut hash);
        // Only where it is, so that a pass drawn soft has the number it had.
        if self.hard {
            self.hard.hash(&mut hash);
        }
        // And so with this. Each clip left out keeps its place among the colors of an ids
        // pass, which the clips drawn do not say: a frame of the same clips with no other
        // in the sequence has other colors.
        if self.only_filmed {
            self.only_filmed.hash(&mut hash);
        }
        self.hash_sequence(&mut hash, 0, at)?;
        Some(hash.finish())
    }

    /// Hash everything sequence `index` at its time `at` draws from.
    pub(super) fn hash_sequence(
        &self,
        hash: &mut DefaultHasher,
        index: usize,
        at: Time,
    ) -> Option<()> {
        self.hash_sequence_of(hash, index, at, None)
    }

    /// [`Drawing::hash_sequence`], of the one track `angle` when it names one.
    pub(super) fn hash_sequence_of(
        &self,
        hash: &mut DefaultHasher,
        index: usize,
        at: Time,
        angle: Option<&Id>,
    ) -> Option<()> {
        let plan = &self.sequences[index];
        (
            plan.width,
            plan.height,
            plan.rate.fps().to_bits(),
            plan.drop_frame,
        )
            .hash(hash);
        for track in plan.tracks.iter().filter(|track| track.drawn(angle)) {
            for (place, shown) in track.shown.iter().enumerate() {
                if !shown.shows(at) {
                    continue;
                }
                self.hash_shown(hash, plan, shown, at)?;
                hash_pairs(hash, track, place, at);
            }
            for dip in track.dips.iter().filter(|dip| dip.shows(at)) {
                (dip.color.r, dip.color.g, dip.color.b, dip.color.a).hash(hash);
                progress(at, dip.from, dip.to).to_bits().hash(hash);
            }
        }
        if self.captions {
            for caption in plan.captions.iter().filter(|caption| caption.shows(at)) {
                caption.text.hash(hash);
            }
        }
        Some(())
    }

    /// Hash everything clip `shown` of sequence `plan` at its time `at` draws from, but for the
    /// transitions that mix it: the clip, its parents, how it enters, and a sequence it nests.
    fn hash_shown(
        &self,
        hash: &mut DefaultHasher,
        plan: &SequencePlan,
        shown: &Shown,
        at: Time,
    ) -> Option<()> {
        let clip = &shown.plan;
        let time = at - clip.clip.start;
        clip.digest?.hash(hash);
        time.flicks().hash(hash);
        clip.shown.to_bits().hash(hash);
        clip.letterboxed.hash(hash);
        // All of a parent, its place in depth with the rest, and when in it this is.
        for rig in plan.parents_of(&clip.clip) {
            rig.digest?.hash(hash);
            (at - rig.clip.start).flicks().hash(hash);
        }
        if clip.reads_sequence_time {
            at.flicks().hash(hash);
        }
        if let Some((reveal, start, end)) = shown.reveal {
            format!("{reveal:?}").hash(hash);
            progress(at, start, end).to_bits().hash(hash);
        }
        if let Source::Nested {
            sequence: nested,
            source_in,
            speed,
            duration,
            angle,
        } = &clip.source
        {
            let inner = &self.sequences[*nested];
            let shown = nested_time(time, *source_in, speed, *duration, inner.rate);
            self.hash_sequence_of(hash, *nested, shown, angle.as_ref())?;
        }
        Some(())
    }

    /// A frame that is one media clip alone, untouched, at full size: its
    /// picture as decoded, which drawing it would give again.
    pub(super) fn plain_frame(&self, at: Time) -> Option<Frame> {
        let plan = &self.sequences[0];
        if self.captions && plan.captions.iter().any(|caption| caption.shows(at)) {
            return None;
        }
        let mut showing = plan
            .tracks
            .iter()
            .filter(|track| track.shows)
            .flat_map(|track| {
                let shown = track.shown.iter().filter(|shown| shown.shows(at)).map(Some);
                let dips = track.dips.iter().filter(|dip| dip.shows(at)).map(|_| None);
                shown.chain(dips)
            });
        let shown = showing.next()??;
        if showing.next().is_some() || !shown.plan.plain {
            return None;
        }
        let Source::Media { picture } = shown.plan.source else {
            return None;
        };
        let rgba = self.pictures.rgba(picture, at - shown.plan.clip.start)?;
        Some(Frame {
            width: plan.width,
            height: plan.height,
            rgba,
        })
    }
}

/// Hash the transitions of `track` that mix its clip `place` at `at`, and how far each has got.
fn hash_pairs(hash: &mut DefaultHasher, track: &TrackPlan, place: usize, at: Time) {
    for pair in track.pairs.iter().filter(|pair| {
        pair.start <= at && at < pair.end && (pair.from == place || pair.to == place)
    }) {
        format!("{}{:?}", pair.def.id, pair.params).hash(hash);
        (pair.from == place, pair.from_nothing).hash(hash);
        progress(at, pair.start, pair.end).to_bits().hash(hash);
    }
}

/// A depth pass's greys stretched to the nearest and the farthest it has, as they are
/// written: what is in depth goes from one above black, the farthest, to white, the
/// nearest, and what is not stays black. A frame with one distance alone is white there.
fn stretch_to_its_own(rgba: &mut [u8]) {
    let (mut least, mut most) = (u8::MAX, 0u8);
    for pixel in rgba.as_chunks::<4>().0.iter().filter(|pixel| pixel[0] > 0) {
        (least, most) = (least.min(pixel[0]), most.max(pixel[0]));
    }
    if most == 0 {
        return;
    }
    let span = f64::from(most - least).max(1.0);
    rgba.par_chunks_mut(4).for_each(|pixel| {
        if pixel[0] > 0 {
            let grey = if most > least {
                (1.0 + f64::from(pixel[0] - least) / span * 254.0).round() as u8
            } else {
                u8::MAX
            };
            pixel[..3].fill(grey);
        }
    });
}

/// [`stretch_to_its_own`] for 16-bit RGBA, little-endian: the farthest is a 255th of white, a step
/// above black at 8 bits, so that it is not taken for black at any depth the file has.
fn stretch_to_its_own16(rgba: &mut [u8]) {
    let grey = |pixel: &[u8]| u16::from_le_bytes([pixel[0], pixel[1]]);
    let (mut least, mut most) = (u16::MAX, 0u16);
    for pixel in rgba
        .as_chunks::<8>()
        .0
        .iter()
        .filter(|pixel| grey(&pixel[..]) > 0)
    {
        (least, most) = (least.min(grey(pixel)), most.max(grey(pixel)));
    }
    if most == 0 {
        return;
    }
    const FARTHEST: f64 = 65_535.0 / 255.0;
    let span = f64::from(most - least).max(1.0);
    rgba.par_chunks_mut(8).for_each(|pixel| {
        let here = grey(pixel);
        if here > 0 {
            let stretched = if most > least {
                (FARTHEST + f64::from(here - least) / span * (65_535.0 - FARTHEST)).round() as u16
            } else {
                u16::MAX
            };
            for channel in 0..3 {
                pixel[channel * 2..channel * 2 + 2].copy_from_slice(&stretched.to_le_bytes());
            }
        }
    });
}
