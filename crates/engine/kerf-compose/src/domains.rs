//! The effects the compositor runs on a clip's picture, each a
//! [`PictureEffect`](kerf_draw::effect::PictureEffect), and the one place
//! the compositor names the crates they are in. A crate joins by being
//! listed here.

use kerf_draw::effect::PictureEffect;
use kerf_draw::source::PictureSource;

/// The effect with the catalog id `id`.
pub(crate) fn effect(id: &str) -> Option<&'static dyn PictureEffect> {
    kerf_image::EFFECTS
        .iter()
        .find(|effect| effect.id() == id)
        .copied()
        .or_else(|| kerf_fx::picture::effect(id))
}

/// The generator a crate draws with the catalog id `id`. A solid, a title,
/// and what draws nothing of its own (an adjustment layer, a null, a camera,
/// a light) are the compositor's.
pub(crate) fn source(id: &str) -> Option<&'static dyn PictureSource> {
    kerf_motion::SOURCES
        .iter()
        .find(|source| source.id() == id)
        .copied()
}

/// The generators the compositor draws or reads itself.
#[cfg(test)]
const OWN_GENERATORS: &[&str] = &[
    "solid",
    "title",
    "adjustment",
    "null",
    "camera",
    "light",
];

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_model::{Catalog, Domain, FxKind, SHAPE_OUTLINE};

    fn all() -> impl Iterator<Item = &'static dyn PictureEffect> {
        kerf_image::EFFECTS
            .iter()
            .copied()
            .chain(kerf_fx::picture::effects())
    }

    #[test]
    fn the_catalog_and_the_code_name_the_same_effects() {
        let catalog = Catalog::builtin();
        let mut ids: Vec<_> = all().map(|effect| effect.id()).collect();
        for id in &ids {
            assert!(catalog.fx(id).is_some(), "{id} is not in the catalog");
        }
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "two crates have an effect of one id");
        // A look is other effects by the time it is drawn, and an outline is part of its shape.
        let drawn = catalog.all().filter(|def| {
            (def.kind, def.domain) == (FxKind::Effect, Domain::Video)
                && catalog.looks().all(|look| look.id != def.id)
                && def.id.as_str() != SHAPE_OUTLINE
        });
        for def in drawn {
            assert!(effect(def.id.as_str()).is_some(), "{} has no code", def.id);
        }
    }

    #[test]
    fn the_catalog_and_the_code_name_the_same_generators() {
        let catalog = Catalog::builtin();
        for source in kerf_motion::SOURCES {
            let def = catalog.fx(source.id());
            assert!(
                def.is_some_and(|def| def.kind == FxKind::Generator),
                "{} is not a generator of the catalog",
                source.id()
            );
            assert!(!OWN_GENERATORS.contains(&source.id()), "{}", source.id());
        }
        let generators = catalog
            .all()
            .filter(|def| (def.kind, def.domain) == (FxKind::Generator, Domain::Video));
        for def in generators {
            let id = def.id.as_str();
            assert!(
                OWN_GENERATORS.contains(&id) || source(id).is_some(),
                "{id} has no code"
            );
        }
    }
}
