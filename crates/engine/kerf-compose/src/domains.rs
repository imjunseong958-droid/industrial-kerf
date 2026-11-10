//! The effects the compositor runs on a clip's picture, each a
//! [`PictureEffect`](kerf_draw::effect::PictureEffect), and the one place
//! the compositor names the crates they are in. A crate joins by being
//! listed here.

use kerf_draw::effect::PictureEffect;

/// The effect with the catalog id `id`.
pub(crate) fn effect(id: &str) -> Option<&'static dyn PictureEffect> {
    kerf_image::EFFECTS
        .iter()
        .find(|effect| effect.id() == id)
        .copied()
        .or_else(|| kerf_fx::picture::effect(id))
}

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
}
