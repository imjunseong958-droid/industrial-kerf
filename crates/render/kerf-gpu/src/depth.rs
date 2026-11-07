//! Clips in depth on the GPU, as `kerf-compose/src/draw/three_d.rs` draws them on the CPU, which stays
//! the reference. The compositor works out the scene's numbers in 64 bits, measured from the eye, and
//! hands them over; [`Frame::depth_scene`] makes what stands in a light's way into buffers, since one
//! kernel has to read any number of pictures; and [`Frame::project`] draws a clip through them.

use super::*;

/// How long a side a buffer's size goes up to, so that a frame's buffers come back for the next one
/// when what they hold changes a little.
const BUFFER_STEP: u64 = 1 << 20;
/// What a [`Pool`] keeps of buffers between frames.
const POOL_BUFFER_BYTES: u64 = 768 << 20;
/// How many clips cross at most, as many as a 1080p picture can stack in the tallest texture.
pub const MOST_LAYERS: usize = 15;

/// The kernels of this module, built the first time a frame in depth may need them.
pub struct Kernels {
    project_layout: wgpu::BindGroupLayout,
    flat: wgpu::ComputePipeline,
    solid: wgpu::ComputePipeline,
    prep_layout: wgpu::BindGroupLayout,
    alpha_of: wgpu::ComputePipeline,
    table_rows: wgpu::ComputePipeline,
    table_columns: wgpu::ComputePipeline,
    radiance: wgpu::ComputePipeline,
    irradiance: wgpu::ComputePipeline,
    by_depth_layout: wgpu::BindGroupLayout,
    by_depth: wgpu::ComputePipeline,
    distance_layout: wgpu::BindGroupLayout,
    outline: wgpu::ComputePipeline,
    jump: wgpu::ComputePipeline,
    signed_distance: wgpu::ComputePipeline,
    eroded: wgpu::ComputePipeline,
    /// What a kernel writes depth to when nobody wants it.
    no_depth: wgpu::TextureView,
    /// What a kernel reads and writes in place of a texture it does not use.
    no_distance: wgpu::TextureView,
    no_places: wgpu::TextureView,
    no_picture: wgpu::TextureView,
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn buffer_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Kernels {
    fn new(device: &wgpu::Device) -> Self {
        let pipelines = |label: &str, source: String, layout: &wgpu::BindGroupLayout| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            (module, pipeline_layout)
        };
        let pipeline = |(module, layout): &(wgpu::ShaderModule, wgpu::PipelineLayout),
                        entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let project_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kerf-depth"),
            entries: &[
                uniform_entry(0),
                texture_entry(1),
                storage_entry(2, WORKING_FORMAT),
                storage_entry(3, wgpu::TextureFormat::R32Float),
                buffer_entry(4, true),
                buffer_entry(5, true),
                buffer_entry(6, true),
                buffer_entry(7, true),
                buffer_entry(8, true),
                texture_entry(9),
            ],
        });
        let project = pipelines(
            "kerf-depth",
            include_str!("depth.wgsl").to_owned(),
            &project_layout,
        );
        let prep_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kerf-depth-prep"),
            entries: &[
                uniform_entry(0),
                texture_entry(1),
                buffer_entry(2, false),
                buffer_entry(3, false),
            ],
        });
        let prep = pipelines(
            "kerf-depth-prep",
            include_str!("depth_prep.wgsl").to_owned(),
            &prep_layout,
        );
        let by_depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kerf-by-depth"),
            entries: &[
                uniform_entry(0),
                texture_entry(1),
                storage_entry(2, WORKING_FORMAT),
                texture_entry(3),
                texture_entry(4),
                buffer_entry(5, true),
            ],
        });
        let by_depth = pipelines(
            "kerf-by-depth",
            format!(
                "{}{}",
                include_str!("picture.wgsl"),
                include_str!("by_depth.wgsl")
            ),
            &by_depth_layout,
        );
        let distance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kerf-distance"),
            entries: &[
                uniform_entry(0),
                texture_entry(1),
                texture_entry(2),
                storage_entry(3, wgpu::TextureFormat::Rg32Float),
                storage_entry(4, wgpu::TextureFormat::R32Float),
                storage_entry(5, WORKING_FORMAT),
            ],
        });
        let distance = pipelines(
            "kerf-distance",
            include_str!("distance.wgsl").to_owned(),
            &distance_layout,
        );
        let tiny = |format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("kerf-tiny"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let both = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let no_depth = tiny(wgpu::TextureFormat::R32Float, both);
        let no_places = tiny(wgpu::TextureFormat::Rg32Float, both);
        let no_distance = tiny(wgpu::TextureFormat::R32Float, both);
        let no_picture = tiny(WORKING_FORMAT, both);
        Self {
            flat: pipeline(&project, "flat"),
            solid: pipeline(&project, "solid"),
            project_layout,
            alpha_of: pipeline(&prep, "alpha_of"),
            table_rows: pipeline(&prep, "table_rows"),
            table_columns: pipeline(&prep, "table_columns"),
            radiance: pipeline(&prep, "radiance"),
            irradiance: pipeline(&prep, "irradiance"),
            prep_layout,
            by_depth: pipeline(&by_depth, "by_depth"),
            by_depth_layout,
            outline: pipeline(&distance, "outline"),
            jump: pipeline(&distance, "jump"),
            signed_distance: pipeline(&distance, "signed_distance"),
            eroded: pipeline(&distance, "eroded"),
            distance_layout,
            no_depth,
            no_distance,
            no_places,
            no_picture,
        }
    }
}

impl Gpu {
    /// The kernels for clips in depth, built on first asking: a shader this long takes its time, which
    /// a frame with none should not have to wait for.
    pub fn depth_kernels(&self) -> Option<&Kernels> {
        self.depth
            .get_or_init(|| match self.build(Kernels::new) {
                Ok(kernels) => Some(kernels),
                Err(error) => {
                    eprintln!("kerf-gpu: clips in depth stay on the CPU: {error}");
                    None
                }
            })
            .as_ref()
    }
}

/// What kind of light a [`Light`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    Ambient = 0,
    Point = 1,
    Directional = 2,
    Environment = 3,
}

/// A light, with the world measured from the eye.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub kind: LightKind,
    /// A point's place, or the way back along a directional's light.
    pub place: [f32; 3],
    pub color: [f32; 3],
    pub shadow: f32,
    /// A point's radius or a directional's spread.
    pub wide: f32,
}

/// How a scene is seen and lit, with the world measured from the eye.
#[derive(Clone, Debug)]
pub struct Seen {
    pub right: [f32; 3],
    pub down: [f32; 3],
    pub forward: [f32; 3],
    pub focal: f32,
    pub center: [f32; 2],
    /// How near is seen, in pixels.
    pub near: f32,
    pub lights: Vec<Light>,
    /// Where on a light of radius 1 each of its rays goes.
    pub spiral: Vec<[f32; 2]>,
    /// How far across each ray's share of a light of radius 1 is.
    pub share: f32,
}

/// A plane in the world, measured from the eye.
#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    /// What goes from a place on the plane, measured from its origin, to a picture's `u` and `v`.
    pub to_u: [f32; 3],
    pub to_v: [f32; 3],
    /// Which way its picture goes across and down, a unit long.
    pub across: [f32; 3],
    pub down: [f32; 3],
    /// How many of its pixels a pixel of the world is along each.
    pub per_across: f32,
    pub per_down: f32,
}

/// A clip's picture as what stands in a light's way.
pub struct Caster<'p> {
    /// Told apart from the clip being drawn, whose shadow does not fall on itself.
    pub clip: u32,
    pub picture: &'p Picture,
    /// Known to be opaque all over, which needs nothing read of it.
    pub opaque: bool,
    pub opacity: f32,
    pub crop: [f32; 4],
    /// A flat clip has one, and a solid's planes through its depth one each.
    pub planes: Vec<Plane>,
}

/// A light all around, and the picture it is.
pub struct Environment<'p> {
    pub light: usize,
    pub picture: &'p Picture,
}

/// A scene with what it reads made ready, for [`Frame::project`].
pub struct Scene {
    seen: wgpu::Buffer,
    casters: wgpu::Buffer,
    floats: wgpu::Buffer,
    tables: wgpu::Buffer,
}

/// How a clip is drawn by [`Frame::project`].
pub struct Project<'p> {
    /// The picture, read at `by` of the size the plane has it at.
    pub picture: &'p Picture,
    pub by: f32,
    pub clip: u32,
    pub plane: Plane,
    /// The plane of its back, when it has a depth, and whether its picture is known to be opaque.
    pub back: Option<([f32; 3], bool)>,
    pub crop: [f32; 4],
    /// Its bevel, when it has one.
    pub bevel: Option<Bevel<'p>>,
}

/// A solid's bevel, in the pixels its picture is read at.
pub struct Bevel<'p> {
    pub distance: &'p Distance,
    pub width: f32,
    pub depth: f32,
}

/// How far each pixel of a picture is from its outline, for a bevel.
#[derive(Clone)]
pub struct Distance {
    view: wgpu::TextureView,
}

/// Pictures of clips that cross, one below another, with how far off each is at each pixel.
pub struct Layers {
    pictures: Picture,
    depths_view: wgpu::TextureView,
    h: u32,
    laid: Vec<[u8; 16]>,
}

/// What a kernel of `distance.wgsl` writes.
enum DistanceOut<'v> {
    Places(&'v wgpu::TextureView),
    Distance(&'v wgpu::TextureView),
    Picture(&'v wgpu::TextureView),
}

impl Layers {
    /// How many are laid, which is the row where the next one's depth goes.
    pub fn laid(&self) -> u32 {
        self.laid.len() as u32
    }
}

fn push(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn push_whole(out: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn push_way(out: &mut Vec<u8>, way: [f32; 3], last: f32) {
    push(out, &[way[0], way[1], way[2], last]);
}

/// How many u32s a picture's summed table takes.
fn table_size(w: u32, h: u32) -> u64 {
    u64::from(w + 1) * u64::from(h + 1)
}

impl Frame<'_> {
    /// A buffer the frame's kernels may read and write, whatever was in it.
    fn working_buffer(&mut self, bytes: u64) -> wgpu::Buffer {
        let size = bytes.max(16).next_multiple_of(BUFFER_STEP);
        let kept = self.gpu.pool().buffers.get_mut(&size).and_then(Vec::pop);
        let buffer = kept.unwrap_or_else(|| {
            self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("kerf-depth"),
                size,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        });
        self.used_buffers.push(buffer.clone());
        buffer
    }

    /// Run a kernel of `depth_prep.wgsl`, in `groups` of 64 invocations.
    fn prep(
        &mut self,
        pipeline: &wgpu::ComputePipeline,
        kernels: &Kernels,
        (picture, at): (&Picture, u64),
        (floats, table): (&wgpu::Buffer, &wgpu::Buffer),
        invocations: u32,
    ) {
        let mut uniforms = Vec::with_capacity(16);
        push_whole(&mut uniforms, &[picture.w, picture.h, at as u32, 0]);
        let buffer = self
            .gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kerf-uniforms"),
                contents: &uniforms,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let group = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("kerf-depth-prep"),
                layout: &kernels.prep_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&picture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: floats.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: table.as_entire_binding(),
                    },
                ],
            });
        let mut pass = self
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kerf-depth-prep"),
                timestamp_writes: None,
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(invocations.div_ceil(64), 1, 1);
    }

    /// Write `picture`'s summed table into `table` from `at`.
    fn make_table(
        &mut self,
        kernels: &Kernels,
        picture: &Picture,
        at: u64,
        (floats, table): (&wgpu::Buffer, &wgpu::Buffer),
    ) {
        self.prep(
            &kernels.table_rows,
            kernels,
            (picture, at),
            (floats, table),
            picture.h,
        );
        self.prep(
            &kernels.table_columns,
            kernels,
            (picture, at),
            (floats, table),
            picture.w + 1,
        );
    }

    /// Make ready what a scene reads: the pictures of what stands in a light's way, as far as its
    /// lights read them, and how much light comes from those it has all around.
    pub fn depth_scene(
        &mut self,
        seen: &Seen,
        casters: &[Caster<'_>],
        environments: &[Environment<'_>],
    ) -> Option<Scene> {
        let gpu = self.gpu;
        let kernels = gpu.depth_kernels()?;
        let reads_alpha = seen
            .lights
            .iter()
            .any(|light| light.shadow > 0.0 && light.wide <= 0.0);
        let reads_table = seen
            .lights
            .iter()
            .any(|light| light.shadow > 0.0 && light.wide > 0.0);
        // Two tables' worth to a light all around: what comes from each way, then what reaches each.
        let cells = 32u64 * 16 * 4;
        let mut float_at = cells * 2 * environments.len() as u64;
        let mut table_at = 0u64;
        let mut places = Vec::with_capacity(casters.len());
        for caster in casters {
            let (w, h) = (caster.picture.w, caster.picture.h);
            let alpha = float_at;
            if reads_alpha && !caster.opaque {
                float_at += u64::from(w) * u64::from(h);
            }
            let table = table_at;
            if reads_table && !caster.opaque {
                table_at += table_size(w, h);
            }
            places.push((alpha, table));
        }
        let most = gpu.device.limits().max_storage_buffer_binding_size;
        if float_at * 4 > most || table_at * 4 > most {
            return None;
        }
        let floats = self.working_buffer(float_at * 4);
        let tables = self.working_buffer(table_at * 4);
        for (place, environment) in environments.iter().enumerate() {
            let at = place as u64 * cells * 2;
            let both = (&floats, &tables);
            let picture = (environment.picture, at);
            self.prep(&kernels.radiance, kernels, picture, both, 32 * 16);
            self.prep(&kernels.irradiance, kernels, picture, both, 32 * 16);
        }
        let mut stand_ins = Vec::new();
        let mut count = 0u32;
        for (caster, (alpha, table)) in casters.iter().zip(&places) {
            let (w, h) = (caster.picture.w, caster.picture.h);
            if !caster.opaque {
                let both = (&floats, &tables);
                if reads_alpha {
                    let at = (caster.picture, *alpha);
                    self.prep(&kernels.alpha_of, kernels, at, both, h);
                }
                if reads_table {
                    self.make_table(kernels, caster.picture, *table, both);
                }
            }
            for plane in &caster.planes {
                count += 1;
                push_whole(
                    &mut stand_ins,
                    &[
                        caster.clip,
                        u32::from(caster.opaque),
                        *alpha as u32,
                        *table as u32,
                        w,
                        h,
                        u32::from(reads_table && !caster.opaque),
                        0,
                    ],
                );
                push(&mut stand_ins, &caster.crop);
                push_way(&mut stand_ins, plane.origin, caster.opacity);
                push_way(&mut stand_ins, plane.normal, 0.0);
                push_way(&mut stand_ins, plane.to_u, 0.0);
                push_way(&mut stand_ins, plane.to_v, 0.0);
                push_way(&mut stand_ins, plane.across, plane.per_across);
                push_way(&mut stand_ins, plane.down, plane.per_down);
            }
        }
        if stand_ins.is_empty() {
            stand_ins.resize(144, 0);
        }
        let mut scene = Vec::with_capacity(432 + seen.lights.len() * 48);
        push_way(&mut scene, seen.right, seen.focal);
        push_way(&mut scene, seen.down, seen.center[0]);
        push_way(&mut scene, seen.forward, seen.center[1]);
        push_whole(&mut scene, &[seen.lights.len() as u32, count, 0, 0]);
        push(&mut scene, &[seen.near, seen.share, 0.0, 0.0]);
        for on in &seen.spiral {
            push(&mut scene, on);
        }
        for (index, light) in seen.lights.iter().enumerate() {
            push_way(&mut scene, light.place, light.shadow);
            push_way(&mut scene, light.color, light.wide);
            let table = environments
                .iter()
                .position(|environment| environment.light == index);
            // Past what it reads: what reaches each way, by `irradiance`.
            let at = table.map_or(0, |place| place as u64 * cells * 2 + cells);
            push_whole(
                &mut scene,
                &[light.kind as u32, at as u32, u32::from(table.is_some()), 0],
            );
        }
        if seen.lights.is_empty() {
            scene.resize(scene.len() + 48, 0);
        }
        let init = |label: &str, contents: &[u8]| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        Some(Scene {
            seen: init("kerf-depth-scene", &scene),
            casters: init("kerf-depth-casters", &stand_ins),
            floats,
            tables,
        })
    }

    /// Draw a clip in depth on a picture of `size`, transparent around it, writing how far off it is at
    /// each pixel to `depth`'s row of `layers` when asked to.
    pub fn project(
        &mut self,
        scene: &Scene,
        clip: &Project<'_>,
        size: (u32, u32),
        layers: Option<(&Layers, u32)>,
    ) -> Option<Picture> {
        let gpu = self.gpu;
        let kernels = gpu.depth_kernels()?;
        let picture = clip.picture;
        let own = match clip.back {
            Some((_, false)) => {
                let table = self.working_buffer(table_size(picture.w, picture.h) * 4);
                let floats = self.working_buffer(16);
                self.make_table(kernels, picture, 0, (&floats, &table));
                table
            }
            _ => self.working_buffer(16),
        };
        let summed = matches!(clip.back, Some((_, false)));
        let plane = &clip.plane;
        let mut uniforms = Vec::with_capacity(160);
        push_whole(
            &mut uniforms,
            &[
                size.0,
                size.1,
                picture.w,
                picture.h,
                clip.clip,
                u32::from(layers.is_some()),
                layers.map_or(0, |(layers, row)| row * layers.h),
                u32::from(summed),
            ],
        );
        push(&mut uniforms, &clip.crop);
        push_way(&mut uniforms, plane.origin, clip.by);
        push_way(
            &mut uniforms,
            clip.back.map_or(plane.origin, |back| back.0),
            0.0,
        );
        push_way(&mut uniforms, plane.normal, 0.0);
        push_way(&mut uniforms, plane.across, 0.0);
        push_way(&mut uniforms, plane.down, 0.0);
        push_way(&mut uniforms, plane.to_u, 0.0);
        push_way(&mut uniforms, plane.to_v, 0.0);
        let (width, deep) = clip
            .bevel
            .as_ref()
            .map_or((0.0, 0.0), |bevel| (bevel.width, bevel.depth));
        push(&mut uniforms, &[width, deep, 0.0, 0.0]);
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kerf-uniforms"),
                contents: &uniforms,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let out = self.take(size.0, size.1);
        let depth = layers.map_or(&kernels.no_depth, |(layers, _)| &layers.depths_view);
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kerf-depth"),
            layout: &kernels.project_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&picture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&out.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: scene.seen.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: scene.casters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: scene.floats.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: scene.tables.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: own.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(
                        clip.bevel
                            .as_ref()
                            .map_or(&kernels.no_distance, |bevel| &bevel.distance.view),
                    ),
                },
            ],
        });
        let mut pass = self
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kerf-depth"),
                timestamp_writes: None,
            });
        pass.set_pipeline(if clip.back.is_some() {
            &kernels.solid
        } else {
            &kernels.flat
        });
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(size.0.div_ceil(16), size.1.div_ceil(16), 1);
        drop(pass);
        Some(out)
    }

    /// Run a kernel of `distance.wgsl`.
    fn distance_pass(
        &mut self,
        pipeline: &wgpu::ComputePipeline,
        (picture, step): (&Picture, u32),
        (crop, far): ([f32; 4], [f32; 2]),
        read_in: Option<&wgpu::TextureView>,
        out: DistanceOut<'_>,
    ) {
        let gpu = self.gpu;
        let Some(kernels) = gpu.depth_kernels() else {
            return;
        };
        let mut uniforms = Vec::with_capacity(48);
        push_whole(&mut uniforms, &[picture.w, picture.h, step, 0]);
        push(&mut uniforms, &crop);
        push(&mut uniforms, &[far[0], far[1], 0.0, 0.0]);
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kerf-uniforms"),
                contents: &uniforms,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let (places, distance, taken_in) = match out {
            DistanceOut::Places(places) => (places, &kernels.no_distance, &kernels.no_picture),
            DistanceOut::Distance(distance) => (&kernels.no_places, distance, &kernels.no_picture),
            DistanceOut::Picture(picture) => (&kernels.no_places, &kernels.no_distance, picture),
        };
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kerf-distance"),
            layout: &kernels.distance_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                view(1, &picture.view),
                view(2, read_in.unwrap_or(&gpu.dummy)),
                view(3, places),
                view(4, distance),
                view(5, taken_in),
            ],
        });
        let mut pass = self
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kerf-distance"),
                timestamp_writes: None,
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(picture.w.div_ceil(16), picture.h.div_ceil(16), 1);
    }

    /// How far each pixel of `picture` is from the outline of its alpha within `crop`, by `jumps` from
    /// `bevel::distance_field`, out to `far`.
    pub fn distance(
        &mut self,
        picture: &Picture,
        crop: [f32; 4],
        jumps: &[u32],
        far: f32,
    ) -> Option<Distance> {
        let gpu = self.gpu;
        let kernels = gpu.depth_kernels()?;
        let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let working = |frame: &mut Self, format| {
            let key = (picture.w, picture.h, format, usage);
            let texture = frame.gpu.texture(key);
            frame.used.push((key, texture.clone()));
            texture.create_view(&Default::default())
        };
        let mut read = working(self, wgpu::TextureFormat::Rg32Float);
        let mut write = working(self, wgpu::TextureFormat::Rg32Float);
        let distance = working(self, wgpu::TextureFormat::R32Float);
        let numbers = (crop, [far, 0.0]);
        self.distance_pass(
            &kernels.outline,
            (picture, 0),
            numbers,
            None,
            DistanceOut::Places(&read),
        );
        for jump in jumps {
            self.distance_pass(
                &kernels.jump,
                (picture, *jump),
                numbers,
                Some(&read),
                DistanceOut::Places(&write),
            );
            std::mem::swap(&mut read, &mut write);
        }
        self.distance_pass(
            &kernels.signed_distance,
            (picture, 0),
            numbers,
            Some(&read),
            DistanceOut::Distance(&distance),
        );
        Some(Distance { view: distance })
    }

    /// `picture` with its outline taken in by `inset` pixels.
    pub fn eroded(
        &mut self,
        picture: &Picture,
        distance: &Distance,
        inset: f32,
    ) -> Option<Picture> {
        let kernels = self.gpu.depth_kernels()?;
        let out = self.take(picture.w, picture.h);
        self.distance_pass(
            &kernels.eroded,
            (picture, 0),
            ([0.0; 4], [0.0, inset]),
            Some(&distance.view),
            DistanceOut::Picture(&out.view),
        );
        Some(out)
    }

    /// Room for `count` clips that cross on a canvas of `(w, h)`.
    pub fn depth_layers(&mut self, (w, h): (u32, u32), count: u32) -> Layers {
        let key = (
            w,
            h * count,
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depths = self.gpu.texture(key);
        self.used.push((key, depths.clone()));
        let depths_view = depths.create_view(&Default::default());
        Layers {
            pictures: self.take(w, h * count),
            depths_view,
            h,
            laid: Vec::new(),
        }
    }

    /// `picture` as the next of `layers`, which is the row its [`Frame::project`] wrote depth to.
    pub fn lay(&mut self, layers: &mut Layers, picture: &Picture, opacity: f32, mode: Blend) {
        let row = layers.laid.len() as u32;
        self.encoder.copy_texture_to_texture(
            picture.texture.as_image_copy(),
            wgpu::TexelCopyTextureInfo {
                texture: &layers.pictures.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: row * layers.h,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: picture.w,
                height: picture.h,
                depth_or_array_layers: 1,
            },
        );
        let mut layer = [0u8; 16];
        layer[..4].copy_from_slice(&opacity.to_le_bytes());
        layer[4..8].copy_from_slice(&(mode as u32).to_le_bytes());
        layers.laid.push(layer);
    }

    /// `layers` over `canvas`, at each pixel in the order of their depth there, the nearest last.
    pub fn by_depth(&mut self, canvas: Picture, layers: Layers) -> Option<Picture> {
        let gpu = self.gpu;
        let kernels = gpu.depth_kernels()?;
        let Layers {
            pictures,
            depths_view,
            h: _,
            laid,
        } = layers;
        if laid.is_empty() {
            self.release(pictures);
            return Some(canvas);
        }
        let contents: Vec<u8> = laid.into_iter().flatten().collect();
        let list = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kerf-by-depth"),
                contents: &contents,
                usage: wgpu::BufferUsages::STORAGE,
            });
        let uniforms = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kerf-uniforms"),
                contents: &Uniforms::default().bytes(),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let out = self.take(canvas.w, canvas.h);
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kerf-by-depth"),
            layout: &kernels.by_depth_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&canvas.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&out.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&pictures.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&depths_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: list.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = self
                .encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("kerf-by-depth"),
                    timestamp_writes: None,
                });
            pass.set_pipeline(&kernels.by_depth);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(canvas.w.div_ceil(16), canvas.h.div_ceil(16), 1);
        }
        self.release(canvas);
        self.release(pictures);
        Some(out)
    }
}

impl Gpu {
    /// Keep a frame's buffers for the next one, as far as they are worth keeping.
    pub(super) fn keep_buffers(&self, buffers: Vec<wgpu::Buffer>) {
        let mut pool = self.pool();
        for buffer in buffers {
            pool.buffers.entry(buffer.size()).or_default().push(buffer);
        }
        let held: u64 = pool
            .buffers
            .values()
            .flatten()
            .map(wgpu::Buffer::size)
            .sum();
        if held > POOL_BUFFER_BYTES {
            pool.buffers.clear();
        }
    }
}
