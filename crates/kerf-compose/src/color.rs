//! `color.basic` as an EffectCraft plug-in, with the graph's arithmetic
//! (`kerf_draw::correct`).

use effectcraft_effects::plugin::{
    register_plugin, EffectPlugin, PluginFrame, PluginManifest, PluginParam, PluginParamKind,
    PluginParams, PLUGIN_API_VERSION,
};
use effectcraft_effects::EffectSpec;
use kerf_draw::correct::Correction;
use kerf_model::Catalog;
use rayon::prelude::*;
use std::sync::{Arc, OnceLock};

/// The plug-in's effect id.
pub(crate) const COLOR_BASIC: &str = "kerf.color.basic";

/// The registered effect, registered on first use.
pub(crate) fn color_basic() -> &'static EffectSpec {
    static SPEC: OnceLock<&'static EffectSpec> = OnceLock::new();
    SPEC.get_or_init(|| {
        register_plugin(Arc::new(ColorBasic::new())).expect("kerf.color.basic registers once")
    })
}

struct ColorBasic {
    manifest: PluginManifest,
}

impl ColorBasic {
    fn new() -> Self {
        let def = Catalog::builtin()
            .fx("color.basic")
            .expect("the catalog defines color.basic");
        let params = def
            .params
            .iter()
            .map(|param| {
                let default = param.default.as_number().unwrap_or(0.0);
                let (min, max) = (
                    param.min.unwrap_or(default.min(-1000.0)),
                    param.max.unwrap_or(default.max(1000.0)),
                );
                PluginParam {
                    id: param.name.clone(),
                    name: param.name.clone(),
                    kind: PluginParamKind::Slider {
                        default,
                        min,
                        max,
                        slider_min: None,
                        slider_max: None,
                        decimals: 3,
                    },
                }
            })
            .collect();
        Self {
            manifest: PluginManifest {
                api: PLUGIN_API_VERSION,
                id: COLOR_BASIC.to_owned(),
                name: "Basic Color".to_owned(),
                category: "Color Correction".to_owned(),
                version: String::new(),
                author: "Kerf".to_owned(),
                description: def.doc.clone(),
                params,
            },
        }
    }
}

impl EffectPlugin for ColorBasic {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn render(
        &self,
        frame: &mut PluginFrame,
        params: &PluginParams,
        _time: f64,
    ) -> Result<(), String> {
        let correction = Correction::new(&|name| params.f(name));
        if correction.is_identity() {
            return Ok(());
        }
        frame.pixels.par_iter_mut().for_each(|pixel| {
            let alpha = pixel[3];
            if alpha <= 0.0 {
                return;
            }
            let straight = [pixel[0], pixel[1], pixel[2]].map(|c| (c / alpha).clamp(0.0, 1.0));
            let [r, g, b] = correction.apply(straight);
            *pixel = [r * alpha, g * alpha, b * alpha, alpha];
        });
        Ok(())
    }
}
