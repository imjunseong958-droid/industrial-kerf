//! This crate's effects as the compositor runs them, each a
//! [`PictureEffect`]: the color correction and the Gaussian blur every Kerf
//! has had, the EffectCraft effects Kerf keeps ([`crate::kept`]), and the
//! video effects carried from FilmCraft ([`crate::filmcraft`]). The
//! compositor finds them by id beside the other crates' effects, and need
//! not know which of these one is.

use crate::filmcraft::catalog::CARRIED;
use crate::filmcraft::instance::{self, Picture};
use crate::filmcraft::project::EffectInstance;
use crate::filmcraft::vfx::{self, FxEnv};
use crate::kept;
use kerf_draw::correct::Correction;
use kerf_draw::effect::{EffectHost, EffectRun, Files, PictureEffect};
use kerf_draw::{blur, Affine, Image};
use kerf_model::{Effect, Time};
use std::collections::HashMap;
use std::sync::OnceLock;

/// This crate's effect with the catalog id `id`.
pub fn effect(id: &str) -> Option<&'static dyn PictureEffect> {
    all().get(id).map(|effect| &**effect as &dyn PictureEffect)
}

/// Every effect of this crate.
pub fn effects() -> impl Iterator<Item = &'static dyn PictureEffect> {
    all().values().map(|effect| &**effect as &dyn PictureEffect)
}

type Boxed = Box<dyn PictureEffect + Send>;

fn all() -> &'static HashMap<&'static str, Boxed> {
    static ALL: OnceLock<HashMap<&'static str, Boxed>> = OnceLock::new();
    ALL.get_or_init(|| {
        let basics: [Boxed; 2] = [Box::new(Correct), Box::new(Gaussian)];
        let kept = kept::KEPT.iter().map(|id| Box::new(Kept(id)) as Boxed);
        let carried = CARRIED
            .iter()
            .map(|(_, id, ..)| Box::new(Carried(id)) as Boxed);
        let mut all = HashMap::new();
        for effect in basics.into_iter().chain(kept).chain(carried) {
            all.entry(effect.id()).or_insert(effect);
        }
        all
    })
}

/// `color.basic`: brightness, contrast, saturation, and gamma.
struct Correct;

impl PictureEffect for Correct {
    fn id(&self) -> &'static str {
        "color.basic"
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun) {
        let correction = Correction::new(&|name| run.number(effect, name));
        if !correction.is_identity() {
            picture.map_encoded(|rgb| correction.apply(rgb));
        }
    }
}

/// `blur.gaussian`.
struct Gaussian;

impl PictureEffect for Gaussian {
    fn id(&self) -> &'static str {
        "blur.gaussian"
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun) {
        // Half the radius, in canvas pixels, as the render graph blurred.
        let sigma = (run.number(effect, "radius") / 2.0 * run.per_canvas) as f32;
        blur::gaussian(picture, sigma, sigma, true);
    }
}

/// An EffectCraft effect Kerf keeps.
struct Kept(&'static str);

impl PictureEffect for Kept {
    fn id(&self) -> &'static str {
        self.0
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun) {
        let run = kept::Run {
            at: run.at,
            sequence_time: run.sequence_at.seconds(),
            frame_rate: run.frame_rate,
            index: run.index,
            seed: run.seed,
            per_source: run.per_source,
            masks: run.masks,
            sounds: run.sounds,
            host: run.host,
        };
        kept::apply(picture, effect, &run);
    }
}

/// A video effect carried from FilmCraft.
struct Carried(&'static str);

impl PictureEffect for Carried {
    fn id(&self) -> &'static str {
        self.0
    }

    fn prepare(&self, effect: &Effect, files: &mut Files) -> Result<(), String> {
        instance::read_luts(effect, files)
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun) {
        let on = Picture {
            size: (picture.w, picture.h),
            per_source: run.per_source as f32,
        };
        let Some(instance) = carried_instance(effect, run, on) else {
            return;
        };
        // One that reads the clip's other frames is left out of a picture drawn for another.
        if run.nested && vfx::is_temporal(&instance.effect) {
            return;
        }
        let env = run.host.map(|host| Env { host, run });
        let library = instance::luts(effect, run.files);
        let run = instance::Run {
            working: run.working,
            at: run.at,
            per_source: run.per_source as f32,
            timecode: run.timecode,
            clip_name: run.clip_name,
            library: library.as_ref(),
            env: env.as_ref().map(|env| env as &dyn FxEnv),
            frame_rate: run.frame_rate,
        };
        instance::apply(picture, &instance, &run);
    }
}

/// `effect` as the FilmCraft effect it carries, on `picture` as `run` has it. A picture track
/// goes by its place from 1, as FilmCraft's `Video 1`.
fn carried_instance(effect: &Effect, run: &EffectRun, on: Picture) -> Option<EffectInstance> {
    let track = |id: &str| {
        run.tracks
            .iter()
            .position(|track| track == id)
            .map(|place| place + 1)
    };
    instance::instance(effect, run.at, on, &track)
}

/// What Kerf's host gives, as FilmCraft's effects ask for it.
struct Env<'a> {
    host: &'a dyn EffectHost,
    run: &'a EffectRun<'a>,
}

impl FxEnv for Env<'_> {
    fn frame(&self, dt: f64, _: &EffectInstance) -> Option<Image> {
        let at = self.run.at + Time::from_seconds(dt)?;
        self.host.picture_at(at, self.run.index)
    }

    fn source_frame(&self, dt: f64, scale: f32) -> Option<Image> {
        self.host
            .source_at(self.run.at + Time::from_seconds(dt)?, scale)
    }

    fn track(&self, index: usize) -> Option<Image> {
        self.host.track(self.run.tracks.get(index)?)
    }

    fn layer_to_output(&self) -> Affine {
        self.host.to_output()
    }

    fn clip_seconds(&self) -> f64 {
        self.run.duration.seconds()
    }

    fn clip_offset(&self) -> f64 {
        self.run.at.seconds()
    }

    fn frame_rate(&self) -> f64 {
        self.run.frame_rate
    }

    fn media_timecode(&self) -> String {
        self.host.source_timecode()
    }

    fn file_name(&self) -> String {
        self.run.clip_name.to_owned()
    }

    fn sequence_name(&self) -> String {
        self.host.sequence_name()
    }

    fn sequence_size(&self) -> (u32, u32) {
        self.host.sequence_size()
    }

    fn source_size(&self) -> (u32, u32) {
        self.host.source_size()
    }

    fn clip_key(&self) -> u64 {
        self.host.clip_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_model::{Catalog, Domain, FxKind};

    #[test]
    fn an_id_names_one_effect() {
        let count = 2 + kept::KEPT.len() + CARRIED.len();
        assert_eq!(all().len(), count);
        for (id, effect) in all() {
            assert_eq!(effect.id(), *id);
        }
    }

    #[test]
    fn each_is_a_video_effect_of_the_catalog() {
        for effect in effects() {
            let def = Catalog::builtin()
                .fx(effect.id())
                .unwrap_or_else(|| panic!("{} is not in the catalog", effect.id()));
            assert_eq!((def.kind, def.domain), (FxKind::Effect, Domain::Video));
        }
    }
}
