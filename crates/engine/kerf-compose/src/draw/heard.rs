//! A sound track as a picture (`gen=sound`): how loud it is at each pitch as bars, or its wave as a
//! line, over a window of it about the frame; and the sound of a track as its clips mix it, which
//! effects that draw sound read too.

use super::*;
use kerf_motion::{Figure, Paint, Shape};
use std::f64::consts::PI;

/// How a sound track is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Heard {
    /// Bars, low pitches to the left, as tall as the sound is loud at each.
    Spectrum,
    /// A line through the wave, about the box's middle.
    Waveform,
}

/// A `gen=sound` clip, as its parameters say; none of them animates.
pub(super) struct SoundShow {
    pub(super) track: String,
    pub(super) heard: Heard,
    /// In frame heights.
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) bars: usize,
    pub(super) color: Rgba,
    /// In frame heights.
    pub(super) line: f64,
    /// In seconds.
    pub(super) window: f64,
}

/// The lowest and highest pitch a spectrum shows, in hertz.
const PITCHES: (f64, f64) = (40.0, 16_000.0);

/// How far down from full loudness a spectrum's bars reach, in decibels: quieter is no bar.
const RANGE_DB: f64 = 60.0;

/// The box a sound draws in, on a picture of `size` with the frame's middle at its middle.
pub(super) fn sound_extent(show: &SoundShow, (w, h): (usize, usize)) -> Rect {
    let tall = h as f64;
    let (wide, high) = (show.width * tall, show.height * tall);
    Rect::new(
        w as f64 / 2.0 - wide / 2.0 - 1.0,
        tall / 2.0 - high / 2.0 - 1.0,
        wide + 2.0,
        high + 2.0,
    )
}

impl Drawing {
    /// `frames` stereo frames, interleaved, of sound track `track` of `sequence` from `start`
    /// seconds of it at `rate`: its media clips mixed as they play, silence where none does.
    pub(super) fn heard(
        &self,
        sequence: &SequencePlan,
        track: &str,
        start: f64,
        frames: usize,
        rate: u32,
    ) -> Option<Vec<f32>> {
        let track = sequence.sounds.iter().find(|t| t.id == track)?;
        let mut mixed = vec![0.0; frames * 2];
        let kept = f64::from(crate::sound::RATE);
        for clip in &track.clips {
            let Some(sound) = self.sounds.of(&clip.path) else {
                continue;
            };
            let length = sound.len() / 2;
            let (from, duration) = (clip.start.seconds(), clip.duration.seconds());
            for (index, out) in mixed.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let offset = start + index as f64 / f64::from(rate.max(1)) - from;
                if !(0.0..duration).contains(&offset) {
                    continue;
                }
                let played = clip.speed.played_seconds(offset);
                let played = if clip.speed.is_reverse() {
                    clip.speed.played_seconds(duration) - played
                } else {
                    played
                };
                let at = (clip.source_in.seconds() + played) * kept;
                match at as usize {
                    _ if at < 0.0 => {}
                    sample if sample < length => {
                        out[0] += sound[2 * sample];
                        out[1] += sound[2 * sample + 1];
                    }
                    _ => {}
                }
            }
        }
        Some(mixed)
    }

    /// A `gen=sound` clip drawn on the picture `run` says, its sound read about the moment of the
    /// sequence its clip time is.
    pub(super) fn sound_on(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        show: &SoundShow,
        run: &SourceRun,
    ) -> Image {
        let (w, h) = run.picture();
        let mut image = Image::new(w, h);
        let rate = crate::sound::RATE;
        let frames = ((show.window * f64::from(rate)) as usize).clamp(16, 1 << 16);
        let at = (plan.clip.start + run.at).seconds() - show.window / 2.0;
        let Some(stereo) = self.heard(sequence, &show.track, at, frames, rate) else {
            return image;
        };
        let wave: Vec<f64> = stereo
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[left, right]| f64::from(left + right) / 2.0)
            .collect();
        let tall = run.size.1 as f64;
        let (wide, high) = (show.width * tall, show.height * tall);
        let [middle_x, middle_y] = run.middle();
        let color = premultiplied(show.color);
        let bars = show.bars.max(2);
        match show.heard {
            Heard::Spectrum => {
                let levels = loudness(&wave, f64::from(rate), bars);
                let slot = wide / bars as f64;
                for (index, level) in levels.into_iter().enumerate() {
                    let bar = level * high;
                    if bar <= 0.0 {
                        continue;
                    }
                    let x = middle_x - wide / 2.0 + slot * (index as f64 + 0.5);
                    let y = middle_y + high / 2.0 - bar / 2.0;
                    let figure = Figure::Rect {
                        width: (slot * 0.7) as f32,
                        height: bar as f32,
                        round: 0.0,
                    };
                    drawn(figure, color, [0.0; 4], 0.0).draw(&mut image, [x as f32, y as f32]);
                }
            }
            Heard::Waveform => {
                let last = (wave.len() - 1) as f64;
                let points = (0..bars)
                    .map(|index| {
                        let along = index as f64 / (bars - 1) as f64;
                        let value = wave[(along * last).round() as usize].clamp(-1.0, 1.0);
                        [((along - 0.5) * wide) as f32, (-value * high / 2.0) as f32]
                    })
                    .collect();
                let figure = Figure::Path {
                    points,
                    closed: false,
                    smooth: false,
                };
                drawn(figure, [0.0; 4], color, (show.line * tall) as f32)
                    .draw(&mut image, [middle_x as f32, middle_y as f32]);
            }
        }
        image
    }
}

/// `figure` filled in `fill` and stroked `width` wide in `stroke`, all of it, untapered.
fn drawn(figure: Figure, fill: [f32; 4], stroke: [f32; 4], width: f32) -> Shape {
    Shape {
        figure,
        outlines: Vec::new(),
        offset: 0.0,
        fill: Paint::Solid(fill),
        stroke,
        stroke_width: width,
        trim: (0.0, 1.0, 0.0),
        taper: (0.0, 0.0),
    }
}

/// How loud `wave`, sampled at `rate`, is in each of `bands` pitches spread evenly by octave over
/// [`PITCHES`], each from nothing at [`RANGE_DB`] under full loudness to 1 at it: the loudest of
/// what a Hann-windowed Fourier transform finds in it.
fn loudness(wave: &[f64], rate: f64, bands: usize) -> Vec<f64> {
    let count = wave.len();
    let size = count.next_power_of_two().max(64);
    let (mut real, mut imaginary) = (vec![0.0; size], vec![0.0; size]);
    let last = (count.max(2) - 1) as f64;
    for (index, sample) in wave.iter().enumerate() {
        real[index] = sample * (0.5 - 0.5 * (2.0 * PI * index as f64 / last).cos());
    }
    fourier(&mut real, &mut imaginary);
    // A full-scale sine through the window comes to 1.
    let scale = 4.0 / count.max(1) as f64;
    let per_bin = rate / size as f64;
    let (low, high) = (PITCHES.0, PITCHES.1.min(rate / 2.0));
    (0..bands)
        .map(|band| {
            let pitch = |share: f64| low * (high / low).powf(share);
            let (from, to) = (
                pitch(band as f64 / bands as f64),
                pitch((band + 1) as f64 / bands as f64),
            );
            let first = (from / per_bin).floor() as usize;
            let last = ((to / per_bin).ceil() as usize)
                .max(first + 1)
                .min(size / 2);
            let loudest = (first..last)
                .map(|bin| real[bin].hypot(imaginary[bin]) * scale)
                .fold(0.0, f64::max);
            if loudest <= 0.0 {
                return 0.0;
            }
            (1.0 + 20.0 * loudest.log10() / RANGE_DB).clamp(0.0, 1.0)
        })
        .collect()
}

/// An in-place radix-2 Fourier transform; both lengths a power of two.
fn fourier(real: &mut [f64], imaginary: &mut [f64]) {
    let count = real.len();
    let mut swapped = 0;
    for index in 1..count {
        let mut bit = count >> 1;
        while swapped & bit != 0 {
            swapped ^= bit;
            bit >>= 1;
        }
        swapped |= bit;
        if index < swapped {
            real.swap(index, swapped);
            imaginary.swap(index, swapped);
        }
    }
    let mut length = 2;
    while length <= count {
        let (sin, cos) = (-2.0 * PI / length as f64).sin_cos();
        for start in (0..count).step_by(length) {
            let (mut turn_real, mut turn_imaginary) = (1.0, 0.0);
            for step in 0..length / 2 {
                let (near, far) = (start + step, start + step + length / 2);
                let real_far = real[far] * turn_real - imaginary[far] * turn_imaginary;
                let imaginary_far = real[far] * turn_imaginary + imaginary[far] * turn_real;
                real[far] = real[near] - real_far;
                imaginary[far] = imaginary[near] - imaginary_far;
                real[near] += real_far;
                imaginary[near] += imaginary_far;
                let next = turn_real * cos - turn_imaginary * sin;
                turn_imaginary = turn_real * sin + turn_imaginary * cos;
                turn_real = next;
            }
        }
        length <<= 1;
    }
}
