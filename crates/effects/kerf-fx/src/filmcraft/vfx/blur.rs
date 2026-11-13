//! Blur & Sharpen effects.

use crate::filmcraft::project::EffectInstance;

use super::*;
use crate::filmcraft::effects::gaussian;

/// Reduce Interlace Flicker: a vertical-only softening across fields.
pub fn interlace_flicker(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let s = fv(e, "softness", cx) * cx.px_scale.max(0.25);
    gaussian(img, 0.0, s, true);
}
