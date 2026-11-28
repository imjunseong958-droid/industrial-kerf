//! Guide passes on the GPU (ids, depth, normals, motion, hard edges, and motion finalization),
//! matching `kerf-compose/src/draw/{ids,output,three_d}.rs`, the CPU reference.

use super::*;

/// The kernels of this module, built on first asking so a picture-only frame does not pay for them.
pub struct Kernels {
    shade_flat: wgpu::ComputePipeline,
    shade_depth: wgpu::ComputePipeline,
    shade_motion_flat: wgpu::ComputePipeline,
    shade_motion_3d: wgpu::ComputePipeline,
    harden: wgpu::ComputePipeline,
    lay_hard: wgpu::ComputePipeline,
    whole_or_nothing: wgpu::ComputePipeline,
}

impl Kernels {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kerf-pass"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pass.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kerf-pass"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
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
            shade_flat: pipeline("shade_flat"),
            shade_depth: pipeline("shade_depth"),
            shade_motion_flat: pipeline("shade_motion_flat"),
            shade_motion_3d: pipeline("shade_motion_3d"),
            harden: pipeline("harden"),
            lay_hard: pipeline("lay_hard"),
            whole_or_nothing: pipeline("whole_or_nothing"),
        }
    }
}

impl Gpu {
    /// The kernels for guide passes, built once on first asking.
    pub fn pass_kernels(&self) -> Option<&Kernels> {
        self.pass
            .get_or_init(
                || match self.build(|device| Kernels::new(device, &self.layout)) {
                    Ok(kernels) => Some(kernels),
                    Err(error) => {
                        eprintln!("kerf-gpu: guide passes stay on the CPU: {error}");
                        None
                    }
                },
            )
            .as_ref()
    }
}

/// Depth-pass coefficients for a 3D plane (`ahead(u, v) = base + du * u + dv * v`), computed in f64
/// from the eye on the CPU.
#[derive(Clone, Copy, Debug)]
pub struct Depth {
    pub base: f32,
    pub du: f32,
    pub dv: f32,
    pub unit: f32,
}

/// 2D motion-pass displacement coefficients (`dx(u, v) = dx[0]*u + dx[1]*v + dx[2]`, same for `dy`).
#[derive(Clone, Copy, Debug)]
pub struct MotionFlat {
    pub dx: [f32; 3],
    pub dy: [f32; 3],
    pub most: f32,
}

/// 3D motion-pass projective coefficients for a plane at `now` and `next`.
#[derive(Clone, Copy, Debug)]
pub struct Motion3d {
    pub ahead_now: [f32; 3],
    pub rx_now: [f32; 3],
    pub ry_now: [f32; 3],
    pub ahead_next: [f32; 3],
    pub rx_next: [f32; 3],
    pub ry_next: [f32; 3],
    pub center_delta: [f32; 2],
    pub most: f32,
}

/// Uniform block matching `struct U` in `pass.wgsl` (112 bytes).
#[derive(Clone, Copy, Default)]
struct PassUniforms {
    a: [u32; 4],
    p: [[f32; 4]; 6],
}

impl PassUniforms {
    fn bytes(&self) -> [u8; 112] {
        let mut out = [0u8; 112];
        for (index, value) in self.a.iter().enumerate() {
            out[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut at = 16;
        for row in self.p {
            for value in row {
                out[at..at + 4].copy_from_slice(&value.to_le_bytes());
                at += 4;
            }
        }
        out
    }
}

fn pack_colors(colors: &[[f32; 4]]) -> (u32, [f32; 4], [f32; 4]) {
    debug_assert!(colors.len() <= 2, "at most two candidate id colors");
    let count = colors.len().min(2) as u32;
    let c0 = colors.first().copied().unwrap_or([0.0; 4]);
    let c1 = colors.get(1).copied().unwrap_or(c0);
    (count, c0, c1)
}

impl Frame<'_> {
    fn pass_run(
        &mut self,
        pipeline: &wgpu::ComputePipeline,
        uniforms: &PassUniforms,
        src: &Picture,
        aux: Option<&Picture>,
    ) -> Picture {
        let out = self.take(src.w, src.h);
        let layout = self.gpu.layout.clone();
        self.dispatch(
            pipeline,
            &layout,
            &uniforms.bytes(),
            (&src.view, &out.view, aux.map(|picture| &picture.view)),
            (src.w.div_ceil(16), src.h.div_ceil(16)),
        );
        out
    }

    /// Replace `picture`'s RGB with `color * alpha` (`ids::flat`, `three_d::shade_still`,
    /// `three_d::shade_by_facing`).
    pub fn shade_flat(&mut self, picture: Picture, color: [f32; 3]) -> Option<Picture> {
        let kernels = self.gpu.pass_kernels()?;
        let mut uniforms = PassUniforms::default();
        uniforms.p[0] = [color[0], color[1], color[2], 1.0];
        let out = self.pass_run(&kernels.shade_flat, &uniforms, &picture, None);
        self.release(picture);
        Some(out)
    }

    /// Shade `picture` by its 3D plane's nearness along the camera's forward vector
    /// (`three_d::shade_by_depth`).
    pub fn shade_depth(&mut self, picture: Picture, depth: &Depth) -> Option<Picture> {
        let kernels = self.gpu.pass_kernels()?;
        let mut uniforms = PassUniforms::default();
        uniforms.p[0] = [depth.base, depth.du, depth.dv, depth.unit];
        let out = self.pass_run(&kernels.shade_depth, &uniforms, &picture, None);
        self.release(picture);
        Some(out)
    }

    /// Shade `picture` by 2D affine motion (`three_d::shade_by_motion_flat`).
    pub fn shade_motion_flat(&mut self, picture: Picture, motion: &MotionFlat) -> Option<Picture> {
        let kernels = self.gpu.pass_kernels()?;
        let mut uniforms = PassUniforms::default();
        uniforms.p[0] = [motion.dx[0], motion.dx[1], motion.dx[2], motion.most];
        uniforms.p[1] = [motion.dy[0], motion.dy[1], motion.dy[2], 0.0];
        let out = self.pass_run(&kernels.shade_motion_flat, &uniforms, &picture, None);
        self.release(picture);
        Some(out)
    }

    /// Shade `picture` by 3D projective screen-space motion (`three_d::shade_by_motion`).
    pub fn shade_motion_3d(&mut self, picture: Picture, motion: &Motion3d) -> Option<Picture> {
        let kernels = self.gpu.pass_kernels()?;
        let mut uniforms = PassUniforms::default();
        uniforms.p[0] = [
            motion.ahead_now[0],
            motion.ahead_now[1],
            motion.ahead_now[2],
            motion.most,
        ];
        uniforms.p[1] = [
            motion.rx_now[0],
            motion.rx_now[1],
            motion.rx_now[2],
            motion.center_delta[0],
        ];
        uniforms.p[2] = [
            motion.ry_now[0],
            motion.ry_now[1],
            motion.ry_now[2],
            motion.center_delta[1],
        ];
        uniforms.p[3] = [
            motion.ahead_next[0],
            motion.ahead_next[1],
            motion.ahead_next[2],
            0.0,
        ];
        uniforms.p[4] = [motion.rx_next[0], motion.rx_next[1], motion.rx_next[2], 0.0];
        uniforms.p[5] = [motion.ry_next[0], motion.ry_next[1], motion.ry_next[2], 0.0];
        let out = self.pass_run(&kernels.shade_motion_3d, &uniforms, &picture, None);
        self.release(picture);
        Some(out)
    }

    /// Harden `picture` in an ids pass: pixels with `alpha * opacity >= 0.5` snap to the nearest
    /// linear color in `colors` with alpha 1, others become clear (`ids::harden`).
    pub fn harden(
        &mut self,
        picture: Picture,
        opacity: f32,
        colors: &[[f32; 4]],
    ) -> Option<Picture> {
        let kernels = self.gpu.pass_kernels()?;
        let (count, c0, c1) = pack_colors(colors);
        let mut uniforms = PassUniforms {
            a: [count, 0, 0, 0],
            ..PassUniforms::default()
        };
        uniforms.p[0] = c0;
        uniforms.p[1] = c1;
        uniforms.p[2] = [opacity, 0.0, 0.0, 0.0];
        let out = self.pass_run(&kernels.harden, &uniforms, &picture, None);
        self.release(picture);
        Some(out)
    }

    /// Lay `layer` hard over `canvas` (`ids::lay_hard`): pixels where `layer.a >= 0.5` take the
    /// nearest color in `colors`, others keep `canvas`.
    pub fn lay_hard(
        &mut self,
        canvas: Picture,
        layer: &Picture,
        colors: &[[f32; 4]],
    ) -> Option<Picture> {
        let kernels = self.gpu.pass_kernels()?;
        let (count, c0, c1) = pack_colors(colors);
        let mut uniforms = PassUniforms {
            a: [count, 0, 0, 0],
            ..PassUniforms::default()
        };
        uniforms.p[0] = c0;
        uniforms.p[1] = c1;
        let out = self.pass_run(&kernels.lay_hard, &uniforms, &canvas, Some(layer));
        self.release(canvas);
        Some(out)
    }

    /// Finalize a motion pass picture before encoding: unpremultiply where `alpha >= 0.5`, else
    /// clear (`output::whole_or_nothing`).
    pub fn whole_or_nothing(&mut self, picture: Picture) -> Option<Picture> {
        let kernels = self.gpu.pass_kernels()?;
        let uniforms = PassUniforms::default();
        let out = self.pass_run(&kernels.whole_or_nothing, &uniforms, &picture, None);
        self.release(picture);
        Some(out)
    }
}
