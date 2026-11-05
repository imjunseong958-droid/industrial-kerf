//! The effects of the areas that draw through
//! [`PictureEffect`](kerf_draw::effect::PictureEffect): the one place the
//! compositor names them. An area joins by listing its crate here.

use kerf_draw::effect::PictureEffect;

const AREAS: &[&[&dyn PictureEffect]] = &[kerf_image::EFFECTS];

/// The area's effect with the catalog id `id`.
pub(crate) fn effect(id: &str) -> Option<&'static dyn PictureEffect> {
    AREAS
        .iter()
        .flat_map(|effects| effects.iter())
        .find(|effect| effect.id() == id)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_model::{Area, Catalog};

    #[test]
    fn the_catalog_and_the_areas_name_the_same_effects() {
        for effects in AREAS {
            for effect in *effects {
                assert!(
                    Catalog::builtin().fx(effect.id()).is_some(),
                    "{} is not in the catalog",
                    effect.id()
                );
            }
        }
        for def in Catalog::builtin().all() {
            if def.area() == Area::Image {
                assert!(effect(def.id.as_str()).is_some(), "{} has no code", def.id);
            }
        }
    }
}
