//! The stage the kernels run on: the pipelines built once from `kernels.wgsl` for a GPU, and [`U`],
//! the numbers each of their passes reads.

use kerf_gpu::{wgpu, Frame, Gpu, Picture};
use std::sync::OnceLock;

/// `U` in `kernels.wgsl`.
#[derive(Default)]
pub(super) struct U {
    pub(super) a: [u32; 4],
    pub(super) b: [u32; 4],
    pub(super) p0: [f32; 4],
    pub(super) p1: [f32; 4],
    pub(super) p2: [f32; 4],
    pub(super) p3: [f32; 4],
    pub(super) p4: [f32; 4],
}

impl U {
    pub(super) fn floats(p0: [f32; 4]) -> Self {
        Self {
            p0,
            ..Self::default()
        }
    }

    pub(super) fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(112);
        for value in self.a.iter().chain(&self.b) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        let floats = [&self.p0, &self.p1, &self.p2, &self.p3, &self.p4];
        for value in floats.into_iter().flatten() {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }
}

/// The kernels' WGSL: what is written by hand, and on the end of it what `kerf_kernel` writes out.
fn source() -> String {
    format!(
        "{}{}{}",
        include_str!("../kernels.wgsl"),
        kerf_kernel::shadow_wgsl(),
        kerf_kernel::color_wgsl()
    )
}

/// The kernels' pipelines, each named for its entry point, and the layout they all bind.
pub(super) struct Stage {
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) noise: wgpu::ComputePipeline,
    pub(super) color_key: wgpu::ComputePipeline,
    pub(super) luma_key: wgpu::ComputePipeline,
    pub(super) logo_cutout: wgpu::ComputePipeline,
    pub(super) ultra_key: wgpu::ComputePipeline,
    pub(super) ultra_matte: wgpu::ComputePipeline,
    pub(super) ultra_choke: wgpu::ComputePipeline,
    pub(super) ultra_keyed: wgpu::ComputePipeline,
    pub(super) lumetri_input: wgpu::ComputePipeline,
    pub(super) lumetri_more: wgpu::ComputePipeline,
    pub(super) hsl_key: wgpu::ComputePipeline,
    pub(super) hsl_apply: wgpu::ComputePipeline,
    pub(super) add_grain: wgpu::ComputePipeline,
    pub(super) encode_kept: wgpu::ComputePipeline,
    pub(super) decode_kept: wgpu::ComputePipeline,
    pub(super) radial_zoom: wgpu::ComputePipeline,
    pub(super) to_unit: wgpu::ComputePipeline,
    pub(super) from_unit: wgpu::ComputePipeline,
    pub(super) to_gamut: wgpu::ComputePipeline,
    pub(super) from_gamut: wgpu::ComputePipeline,
    pub(super) shadow: wgpu::ComputePipeline,
    pub(super) mosaic_blocks: wgpu::ComputePipeline,
    pub(super) mosaic: wgpu::ComputePipeline,
    pub(super) kept_through: wgpu::ComputePipeline,
    pub(super) photo_filter: wgpu::ComputePipeline,
    pub(super) unmult: wgpu::ComputePipeline,
    pub(super) curves: wgpu::ComputePipeline,
    pub(super) hue_saturation: wgpu::ComputePipeline,
    pub(super) selective_color: wgpu::ComputePipeline,
    pub(super) spill: wgpu::ComputePipeline,
    pub(super) cineon: wgpu::ComputePipeline,
    pub(super) lens: wgpu::ComputePipeline,
    pub(super) sphere: wgpu::ComputePipeline,
    pub(super) optics: wgpu::ComputePipeline,
    pub(super) key_clean: wgpu::ComputePipeline,
    pub(super) choke_rows: wgpu::ComputePipeline,
    pub(super) choke_columns: wgpu::ComputePipeline,
    pub(super) choke_put: wgpu::ComputePipeline,
    pub(super) plane_to_sphere: wgpu::ComputePipeline,
}

impl Stage {
    fn new(gpu: &Gpu) -> Option<Self> {
        let built = gpu.build(|device| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("kerf-vfx"),
                source: wgpu::ShaderSource::Wgsl(source().into()),
            });
            let layout = kerf_gpu::kernel_layout(device);
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kerf-vfx"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let kernel = |entry: &str| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            Self {
                noise: kernel("noise"),
                color_key: kernel("color_key"),
                luma_key: kernel("luma_key"),
                logo_cutout: kernel("logo_cutout"),
                ultra_key: kernel("ultra_key"),
                ultra_matte: kernel("ultra_matte"),
                ultra_choke: kernel("ultra_choke"),
                ultra_keyed: kernel("ultra_keyed"),
                lumetri_input: kernel("lumetri_input"),
                lumetri_more: kernel("lumetri_more"),
                hsl_key: kernel("hsl_key"),
                hsl_apply: kernel("hsl_apply"),
                add_grain: kernel("add_grain"),
                encode_kept: kernel("encode_kept"),
                decode_kept: kernel("decode_kept"),
                radial_zoom: kernel("radial_zoom"),
                to_unit: kernel("to_unit"),
                from_unit: kernel("from_unit"),
                to_gamut: kernel("to_gamut"),
                from_gamut: kernel("from_gamut"),
                shadow: kernel("shadow"),
                mosaic_blocks: kernel("mosaic_blocks"),
                mosaic: kernel("mosaic"),
                kept_through: kernel("kept_through"),
                photo_filter: kernel("photo_filter"),
                unmult: kernel("unmult"),
                curves: kernel("curves"),
                hue_saturation: kernel("hue_saturation"),
                selective_color: kernel("selective_color"),
                spill: kernel("spill"),
                cineon: kernel("cineon"),
                lens: kernel("lens"),
                sphere: kernel("sphere"),
                optics: kernel("optics"),
                key_clean: kernel("key_clean"),
                choke_rows: kernel("choke_rows"),
                choke_columns: kernel("choke_columns"),
                choke_put: kernel("choke_put"),
                plane_to_sphere: kernel("plane_to_sphere"),
                layout,
            }
        });
        built
            .map_err(|error| eprintln!("kerf-fx: look kernels did not build: {error}"))
            .ok()
    }

    pub(super) fn pass(
        &self,
        frame: &mut Frame<'static>,
        pipeline: &wgpu::ComputePipeline,
        src: &Picture,
        aux: Option<&Picture>,
        uniforms: &U,
    ) -> Picture {
        let out = frame.take(src.w, src.h);
        frame.dispatch(
            pipeline,
            &self.layout,
            &uniforms.bytes(),
            (&src.view, &out.view, aux.map(|aux| &aux.view)),
            (src.w.div_ceil(16), src.h.div_ceil(16)),
        );
        out
    }
}

pub(super) fn stage(gpu: &'static Gpu) -> Option<&'static Stage> {
    static STAGE: OnceLock<Option<Stage>> = OnceLock::new();
    STAGE.get_or_init(|| Stage::new(gpu)).as_ref()
}
