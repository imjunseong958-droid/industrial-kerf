//! A camera's blur of a clip in depth on the GPU, as `kerf-compose/src/draw/focus.rs` does it on the
//! CPU, which stays the reference and says what to ask for here.

use super::depth::Depths;
use super::*;

/// How many sums a row has either side of a picture: a gathered radius is under 8, so its kernel reaches
/// 8 rows and columns, and 2 more for reading between two sums.
const ROOM: u32 = 10;
/// How much wider than a picture its row sums are.
pub const SUMS_WIDER: usize = 2 * ROOM as usize + 1;
/// The most levels a blur has, as many as the kernel has textures for.
const MOST_LEVELS: usize = 7;
/// The most rows of a disc: a gathered radius is under 8.
const MOST_SPANS: usize = 17;

/// What says how far a pixel spreads: all in pixels.
#[derive(Clone, Copy, Debug)]
pub struct Focus {
    pub focus: f32,
    pub aperture: f32,
    pub most: f32,
    /// How far a blur reaches past what it has: this much of its radius, and this much more.
    pub margin: (f32, f32),
}

/// Where a drawn clip has anything, and how far it and what is around it spread.
#[derive(Clone, Copy, Debug)]
pub struct Found {
    /// Where its alpha is, all included: `right` is under `left` when it has none.
    pub left: usize,
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
    /// The largest radius of what has alpha.
    pub reach: f32,
    /// The least and most radius within `margin` of that.
    pub least: f32,
    pub most: f32,
}

/// The kernels of this module, built the first time a camera asks for them.
pub struct Kernels {
    layout: wgpu::BindGroupLayout,
    find_reach: wgpu::ComputePipeline,
    find_range: wgpu::ComputePipeline,
    shrink: wgpu::ComputePipeline,
    row_sums: wgpu::ComputePipeline,
    gather: wgpu::ComputePipeline,
    focused: wgpu::ComputePipeline,
    /// What takes the place of what a kernel does not use.
    no_found: wgpu::Buffer,
    no_picture: wgpu::TextureView,
}

impl Kernels {
    fn new(device: &wgpu::Device) -> Self {
        let mut entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            texture_entry(1),
            storage_entry(2, WORKING_FORMAT),
            texture_entry(3),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        entries.extend((0..MOST_LEVELS as u32).map(|level| texture_entry(5 + level)));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kerf-focus"),
            entries: &entries,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kerf-focus"),
            source: wgpu::ShaderSource::Wgsl(include_str!("focus.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kerf-focus"),
            bind_group_layouts: &[Some(&layout)],
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
        let no_found = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kerf-no-found"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let no_picture = device
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
                format: WORKING_FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self {
            find_reach: pipeline("find_reach"),
            find_range: pipeline("find_range"),
            shrink: pipeline("shrink"),
            row_sums: pipeline("row_sums"),
            gather: pipeline("gather"),
            focused: pipeline("focused"),
            layout,
            no_found,
            no_picture,
        }
    }
}

impl Gpu {
    fn focus_kernels(&self) -> Option<&Kernels> {
        self.focus
            .get_or_init(|| match self.build(Kernels::new) {
                Ok(kernels) => Some(kernels),
                Err(error) => {
                    eprintln!("kerf-gpu: a camera's blur stays on the CPU: {error}");
                    None
                }
            })
            .as_ref()
    }
}

/// The numbers `focus.wgsl` reads.
#[derive(Clone, Copy, Default)]
struct Numbers {
    size: [i32; 4],
    area: [i32; 4],
    gather: [f32; 4],
    counts: [i32; 4],
    levels: [[f32; 4]; MOST_LEVELS],
    spans: [Span; MOST_SPANS],
}

#[derive(Clone, Copy, Default)]
struct Span {
    dy: i32,
    high: i32,
    low: i32,
    high_part: f32,
    low_part: f32,
}

/// The rows of a disc of `radius`, with their ends past the row sums' [`ROOM`], and its area: half of
/// each row is `(r² - dy²)^½`, with a row's middle standing for the whole of it (`focus::gathered`).
fn spans_of(radius: f32) -> ([Span; MOST_SPANS], usize, f32) {
    let reach = radius.ceil() as i32;
    debug_assert!(reach + 2 <= ROOM as i32, "{radius} is too wide to gather");
    let mut spans = [Span::default(); MOST_SPANS];
    let mut count = 0;
    let mut area = 0.0;
    for dy in -reach..=reach {
        let half = radius * radius - (dy * dy) as f32;
        if half < 0.0 || count == MOST_SPANS {
            continue;
        }
        let half = half.sqrt() + 0.5;
        let end = |along: f32| {
            let at = along + ROOM as f32;
            let whole = at.floor();
            (whole as i32, at - whole)
        };
        let (high, high_part) = end(0.5 + half);
        let (low, low_part) = end(0.5 - half);
        spans[count] = Span {
            dy,
            high,
            low,
            high_part,
            low_part,
        };
        count += 1;
        area += 2.0 * half;
    }
    (spans, count, area)
}

/// What a pass of `focus.wgsl` reads and writes.
#[derive(Clone, Copy)]
struct Pass<'p> {
    pipeline: &'p wgpu::ComputePipeline,
    src: Option<&'p wgpu::TextureView>,
    dst: Option<&'p wgpu::TextureView>,
    found: Option<&'p wgpu::Buffer>,
    levels: &'p [&'p wgpu::TextureView],
    groups: (u32, u32),
}

impl Frame<'_> {
    fn focus_pass(&mut self, depths: &Depths, focus: &Focus, numbers: &Numbers, pass: Pass<'_>) {
        let gpu = self.gpu;
        let Some(kernels) = gpu.focus_kernels() else {
            return;
        };
        let whole = |values: [i32; 4]| values.map(i32::to_le_bytes).concat();
        let float = |values: [f32; 4]| values.map(f32::to_le_bytes).concat();
        let mut uniforms = Vec::with_capacity(752);
        let size = [
            numbers.size[0],
            numbers.size[1],
            numbers.size[2],
            depths.first_row as i32,
        ];
        uniforms.extend(whole(size));
        uniforms.extend(whole(numbers.area));
        uniforms.extend(float([focus.focus, focus.aperture, focus.most, 0.0]));
        uniforms.extend(float([focus.margin.0, focus.margin.1, 0.0, 0.0]));
        uniforms.extend(float(numbers.gather));
        for level in numbers.levels {
            uniforms.extend(float(level));
        }
        uniforms.extend(whole(numbers.counts));
        for span in numbers.spans {
            uniforms.extend(whole([span.dy, span.high, span.low, 0]));
            uniforms.extend(float([span.high_part, span.low_part, 0.0, 0.0]));
        }
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
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            view(1, pass.src.unwrap_or(&gpu.dummy)),
            view(2, pass.dst.unwrap_or(&kernels.no_picture)),
            view(3, &depths.view),
            wgpu::BindGroupEntry {
                binding: 4,
                resource: pass.found.unwrap_or(&kernels.no_found).as_entire_binding(),
            },
        ];
        for level in 0..MOST_LEVELS {
            let read = pass.levels.get(level).copied().unwrap_or(&gpu.dummy);
            entries.push(view(5 + level as u32, read));
        }
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kerf-focus"),
            layout: &kernels.layout,
            entries: &entries,
        });
        let mut compute = self
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kerf-focus"),
                timestamp_writes: None,
            });
        compute.set_pipeline(pass.pipeline);
        compute.set_bind_group(0, &group, &[]);
        compute.dispatch_workgroups(pass.groups.0, pass.groups.1, 1);
    }

    /// Run what is recorded so far and find what `drawn` needs of a blur. The levels it comes to decide
    /// what size of texture to take, which nothing but the frame so far can say.
    pub fn focus_found(
        &mut self,
        drawn: &Picture,
        depths: &Depths,
        focus: &Focus,
    ) -> Option<Found> {
        let gpu = self.gpu;
        let kernels = gpu.focus_kernels()?;
        let start: [u32; 8] = [drawn.w, drawn.h, 0, 0, 0, f32::MAX.to_bits(), 0, 0];
        let found = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kerf-focus-found"),
                contents: &start.map(u32::to_le_bytes).concat(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
        let numbers = Numbers {
            size: [drawn.w as i32, drawn.h as i32, 0, 0],
            ..Numbers::default()
        };
        let tiles = (drawn.w.div_ceil(16), drawn.h.div_ceil(16));
        for pipeline in [&kernels.find_reach, &kernels.find_range] {
            self.focus_pass(
                depths,
                focus,
                &numbers,
                Pass {
                    pipeline,
                    src: Some(&drawn.view),
                    dst: None,
                    found: Some(&found),
                    levels: &[],
                    groups: tiles,
                },
            );
        }
        let staging = self.staging(32);
        self.encoder
            .copy_buffer_to_buffer(&found, 0, &staging, 0, 32);
        self.read_midway(staging, |data| {
            let at = |index: usize| {
                u32::from_le_bytes([
                    data[index * 4],
                    data[index * 4 + 1],
                    data[index * 4 + 2],
                    data[index * 4 + 3],
                ])
            };
            Found {
                left: at(0) as usize,
                top: at(1) as usize,
                right: at(2) as usize,
                bottom: at(3) as usize,
                reach: f32::from_bits(at(4)),
                least: f32::from_bits(at(5)),
                most: f32::from_bits(at(6)),
            }
        })
    }

    /// `picture` gathered through a round iris of `radius` pixels at a whole `by` smaller, over `area`
    /// of the frame, for the pixels whose radius is within `reads`: the others read something else. As
    /// `focus::shrunk`, `focus::running_sums`, and `focus::gathered` have it.
    pub fn focus_gathered(
        &mut self,
        picture: &Picture,
        depths: &Depths,
        focus: &Focus,
        (radius, by): (f32, u32),
        area: [usize; 4],
        reads: (f32, f32),
    ) -> Option<Picture> {
        let kernels = self.gpu.focus_kernels()?;
        let (w, h) = (picture.w.div_ceil(by), picture.h.div_ceil(by));
        // What it gathers, in the pixels it is gathered at: a row or column more than it takes, for
        // reading between pixels when it grows again.
        let around = |at: usize, wanted: bool, end: u32| {
            let small = at / by as usize;
            match (by, wanted) {
                (1, _) => at as i32,
                (_, false) => small.saturating_sub(1) as i32,
                (_, true) => (small + 1).min(end as usize - 1) as i32,
            }
        };
        let gathered = [
            around(area[0], false, w),
            around(area[1], false, h),
            around(area[2], true, w),
            around(area[3], true, h),
        ];
        // And all that reads, which goes no further than the row sums have room for.
        let room = ROOM as i32;
        let reads_from = [
            (gathered[0] - room).max(0),
            (gathered[1] - room).max(0),
            (gathered[2] + room).min(w as i32 - 1),
            (gathered[3] + room).min(h as i32 - 1),
        ];
        let shrunk = (by > 1).then(|| {
            let out = self.take(w, h);
            let numbers = Numbers {
                size: [picture.w as i32, picture.h as i32, 0, 0],
                area: reads_from,
                counts: [0, by as i32, 0, 0],
                ..Numbers::default()
            };
            self.focus_pass(
                depths,
                focus,
                &numbers,
                Pass {
                    pipeline: &kernels.shrink,
                    src: Some(&picture.view),
                    dst: Some(&out.view),
                    found: None,
                    levels: &[],
                    groups: (w.div_ceil(16), h.div_ceil(16)),
                },
            );
            out
        });
        let reading = shrunk.as_ref().unwrap_or(picture);
        let sums = self.take(w + SUMS_WIDER as u32, h);
        let size = [w as i32, h as i32, room, 0];
        self.focus_pass(
            depths,
            focus,
            &Numbers {
                size,
                area: reads_from,
                ..Numbers::default()
            },
            Pass {
                pipeline: &kernels.row_sums,
                src: Some(&reading.view),
                dst: Some(&sums.view),
                found: None,
                levels: &[],
                groups: ((reads_from[3] - reads_from[1] + 1) as u32, 1),
            },
        );
        let (spans, count, disc) = spans_of(radius / by as f32);
        let out = self.take(w, h);
        self.focus_pass(
            depths,
            focus,
            &Numbers {
                size,
                area: gathered,
                gather: [disc, reads.0, reads.1, count as f32],
                counts: [0, by as i32, 0, 0],
                spans,
                ..Numbers::default()
            },
            Pass {
                pipeline: &kernels.gather,
                src: Some(&sums.view),
                dst: Some(&out.view),
                found: None,
                levels: &[],
                groups: (w.div_ceil(16), h.div_ceil(16)),
            },
        );
        self.release(sums);
        if let Some(shrunk) = shrunk {
            self.release(shrunk);
        }
        Some(out)
    }

    /// Each pixel of `area` mixed from the two `levels` around its own radius, and the rest
    /// transparent: each level is a picture, its radius, and how much smaller it was gathered.
    pub fn focused(
        &mut self,
        depths: &Depths,
        focus: &Focus,
        levels: &[(&Picture, f32, u32)],
        area: [usize; 4],
        (w, h): (u32, u32),
    ) -> Option<Picture> {
        let kernels = self.gpu.focus_kernels()?;
        if levels.is_empty() || levels.len() > MOST_LEVELS {
            return None;
        }
        let mut numbers = Numbers {
            size: [w as i32, h as i32, 0, 0],
            area: area.map(|at| at as i32),
            counts: [levels.len() as i32, 0, 0, 0],
            ..Numbers::default()
        };
        for (out, (picture, radius, by)) in numbers.levels.iter_mut().zip(levels) {
            *out = [*radius, *by as f32, picture.w as f32, picture.h as f32];
        }
        let views: Vec<&wgpu::TextureView> =
            levels.iter().map(|(picture, ..)| &picture.view).collect();
        let out = self.take(w, h);
        self.focus_pass(
            depths,
            focus,
            &numbers,
            Pass {
                pipeline: &kernels.focused,
                src: None,
                dst: Some(&out.view),
                found: None,
                levels: &views,
                groups: (w.div_ceil(16), h.div_ceil(16)),
            },
        );
        Some(out)
    }
}
