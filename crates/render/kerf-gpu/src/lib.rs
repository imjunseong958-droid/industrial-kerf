//! Kerf's drawing on the GPU.
//!
//! A [`Frame`] records one frame's work: footage goes up as the 8-bit bytes it decoded to, every
//! working [`Picture`] is premultiplied linear-light RGBA f32 in a texture, as `kerf_draw::Image`
//! is on the CPU, and only the finished bytes come back. Going up and back once a frame is what
//! makes this worth doing: moving a 4K float picture across for each effect costs as much as
//! running the effect on the CPU.
//!
//! `kerf_draw` stays the reference. Every kernel here does its arithmetic, and the compositor falls
//! back to it for a frame holding anything these kernels cannot draw. [`shared`] is `None` without
//! a real GPU, with `KERF_GPU=0` in the environment, or after the device fails, and the CPU then
//! draws everything.

use kerf_draw::correct::Correction;
use kerf_draw::{Affine, Blend, Image, Landing, Rect};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;
use wgpu::util::DeviceExt;

pub use wgpu;

/// The format of every working picture.
pub const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// How long to wait for a frame before giving it to the CPU.
const FRAME_TIMEOUT: Duration = Duration::from_secs(8);

/// Textures kept for reuse between frames. A frame allocates several full-size float pictures, and
/// allocating one costs more than compositing into it.
const POOL_BYTES: usize = 1536 << 20;

type TextureKey = (u32, u32, wgpu::TextureFormat, wgpu::TextureUsages);

#[derive(Default)]
struct Pool {
    free: HashMap<TextureKey, Vec<wgpu::Texture>>,
    staging: HashMap<u64, Vec<wgpu::Buffer>>,
    bytes: usize,
}

fn texture_bytes(key: TextureKey) -> usize {
    let per_pixel = match key.2 {
        wgpu::TextureFormat::Rgba32Float => 16,
        wgpu::TextureFormat::R32Float | wgpu::TextureFormat::Rgba8Unorm => 4,
        _ => 8,
    };
    key.0 as usize * key.1 as usize * per_pixel
}

/// The device and the kernels on it.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    decode: wgpu::ComputePipeline,
    fill: wgpu::ComputePipeline,
    multiply: wgpu::ComputePipeline,
    halve: wgpu::ComputePipeline,
    composite: wgpu::ComputePipeline,
    correct: wgpu::ComputePipeline,
    finish_layout: wgpu::BindGroupLayout,
    finish: wgpu::ComputePipeline,
    dummy: wgpu::TextureView,
    pool: Mutex<Pool>,
    failed: Arc<AtomicBool>,
}

/// The one device every compositor shares. `None` when the CPU has to draw.
pub fn shared() -> Option<&'static Gpu> {
    static SHARED: OnceLock<Option<Gpu>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            if std::env::var_os("KERF_GPU").is_some_and(|value| value == "0") {
                return None;
            }
            let gpu = Gpu::new();
            if gpu.is_none() {
                eprintln!("kerf-gpu: no usable GPU, drawing on the CPU");
            }
            gpu
        })
        .as_ref()
        .filter(|gpu| !gpu.failed.load(Ordering::Relaxed))
}

/// A future wgpu has already finished, as error scopes on these calls have.
fn now<F: std::future::Future>(future: F) -> F::Output {
    pollster::block_on(future)
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

/// The bind group layout of a kernel that reads `src` and `aux` and writes a working picture
/// through its uniforms, which effect kernels share.
pub fn kernel_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("kerf-kernel"),
        entries: &[
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
        ],
    })
}

impl Gpu {
    fn new() -> Option<Self> {
        if wgpu::Instance::enabled_backend_features().is_empty() {
            return None;
        }
        let instance = wgpu::Instance::default();
        let adapter = now(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        // A driver on the CPU is slower than the reference.
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            return None;
        }
        let (device, queue) = now(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("kerf"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .ok()?;
        let limits = device.limits();
        if limits.max_storage_textures_per_shader_stage < 1
            || limits.max_compute_invocations_per_workgroup < 256
            || limits.max_compute_workgroup_size_x < 64
        {
            return None;
        }
        // wgpu's default handler panics on a device error. A frame then goes to the CPU.
        let failed = Arc::new(AtomicBool::new(false));
        let on_error = failed.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            eprintln!("kerf-gpu: {error}; drawing on the CPU from here on");
            on_error.store(true, Ordering::Relaxed);
        }));
        let on_lost = failed.clone();
        device.set_device_lost_callback(move |_, message| {
            eprintln!("kerf-gpu: device lost: {message}; drawing on the CPU from here on");
            on_lost.store(true, Ordering::Relaxed);
        });

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kerf-picture"),
            source: wgpu::ShaderSource::Wgsl(include_str!("picture.wgsl").into()),
        });
        let layout = kernel_layout(&device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kerf-picture"),
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
        let (decode, fill, multiply, halve, composite, correct) = (
            kernel("decode"),
            kernel("fill"),
            kernel("multiply"),
            kernel("halve"),
            kernel("composite"),
            kernel("correct"),
        );
        let finish_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kerf-finish"),
            source: wgpu::ShaderSource::Wgsl(include_str!("finish.wgsl").into()),
        });
        let finish_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kerf-finish"),
            entries: &[
                texture_entry(1),
                storage_entry(2, wgpu::TextureFormat::Rgba8Unorm),
            ],
        });
        let finish_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("kerf-finish"),
                bind_group_layouts: &[Some(&finish_layout)],
                immediate_size: 0,
            });
        let finish = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("finish"),
            layout: Some(&finish_pipeline_layout),
            module: &finish_module,
            entry_point: Some("finish"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = now(scope.pop()) {
            eprintln!("kerf-gpu: kernels did not build: {error}");
            return None;
        }
        let dummy = device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("kerf-dummy"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[0; 4],
            )
            .create_view(&Default::default());
        Some(Self {
            device,
            queue,
            layout,
            decode,
            fill,
            multiply,
            halve,
            composite,
            correct,
            finish_layout,
            finish,
            dummy,
            pool: Mutex::default(),
            failed,
        })
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Whether a picture of this size fits in a texture.
    pub fn fits(&self, w: usize, h: usize) -> bool {
        let most = self.device.limits().max_texture_dimension_2d as usize;
        w > 0 && h > 0 && w <= most && h <= most
    }

    /// Start a frame.
    pub fn frame(&self) -> Frame<'_> {
        Frame {
            gpu: self,
            encoder: self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("kerf-frame"),
                }),
            working: HashMap::new(),
            used: Vec::new(),
        }
    }

    fn pool(&self) -> std::sync::MutexGuard<'_, Pool> {
        self.pool.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn texture(&self, key: TextureKey) -> wgpu::Texture {
        {
            let mut pool = self.pool();
            if let Some(texture) = pool.free.get_mut(&key).and_then(Vec::pop) {
                pool.bytes -= texture_bytes(key);
                return texture;
            }
        }
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("kerf-picture"),
            size: wgpu::Extent3d {
                width: key.0,
                height: key.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: key.2,
            usage: key.3,
            view_formats: &[],
        })
    }

    /// Give textures back once the commands that use them are submitted. Sizes in use stay; past
    /// the budget the rest go.
    fn give_back(&self, textures: Vec<(TextureKey, wgpu::Texture)>) {
        let mut pool = self.pool();
        for (key, texture) in textures {
            pool.bytes += texture_bytes(key);
            pool.free.entry(key).or_default().push(texture);
        }
        while pool.bytes > POOL_BYTES {
            let Some(key) = pool.free.keys().next().copied() else {
                break;
            };
            let texture = pool.free.get_mut(&key).and_then(Vec::pop);
            match texture {
                Some(_) => pool.bytes -= texture_bytes(key),
                None => {
                    pool.free.remove(&key);
                }
            }
        }
    }
}

/// A working picture: premultiplied linear-light RGBA f32 in a texture.
#[derive(Clone, Debug)]
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

/// One frame's work, recorded in order and submitted by [`Frame::finish`].
pub struct Frame<'a> {
    gpu: &'a Gpu,
    encoder: wgpu::CommandEncoder,
    /// Pictures free for this frame's later commands, which run after everything recorded before
    /// them.
    working: HashMap<(u32, u32), Vec<Picture>>,
    /// Every texture the frame holds, to give back when it is submitted.
    used: Vec<(TextureKey, wgpu::Texture)>,
}

const WORKING_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING
    .union(wgpu::TextureUsages::STORAGE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);

/// The uniforms of this crate's kernels (`U` in `picture.wgsl`).
#[derive(Default)]
struct Uniforms {
    a: [u32; 4],
    bounds: [u32; 4],
    m0: [f32; 4],
    m1: [f32; 4],
    rect: [f32; 4],
    range: [f32; 4],
    p: [f32; 4],
}

impl Uniforms {
    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(112);
        for value in self.a.iter().chain(&self.bounds) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in [self.m0, self.m1, self.rect, self.range, self.p]
            .iter()
            .flatten()
        {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }
}

impl<'a> Frame<'a> {
    pub fn gpu(&self) -> &'a Gpu {
        self.gpu
    }

    /// A working picture of this size, its contents whatever they were.
    pub fn take(&mut self, w: u32, h: u32) -> Picture {
        if let Some(picture) = self.working.get_mut(&(w, h)).and_then(Vec::pop) {
            return picture;
        }
        let key = (w, h, WORKING_FORMAT, WORKING_USAGE);
        let texture = self.gpu.texture(key);
        self.used.push((key, texture.clone()));
        let view = texture.create_view(&Default::default());
        Picture {
            w,
            h,
            texture,
            view,
        }
    }

    /// Done with `picture` for now: a later command of this frame may overwrite it.
    pub fn release(&mut self, picture: Picture) {
        self.working
            .entry((picture.w, picture.h))
            .or_default()
            .push(picture);
    }

    /// Run `pipeline` of a kernel from `src` (and `aux`) into `dst`, with `uniforms`, in groups
    /// of `groups`.
    pub fn dispatch(
        &mut self,
        pipeline: &wgpu::ComputePipeline,
        layout: &wgpu::BindGroupLayout,
        uniforms: &[u8],
        views: (
            &wgpu::TextureView,
            &wgpu::TextureView,
            Option<&wgpu::TextureView>,
        ),
        groups: (u32, u32),
    ) {
        let (src, dst, aux) = views;
        let buffer = self
            .gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("kerf-uniforms"),
                contents: uniforms,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let group = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("kerf-kernel"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(dst),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(
                            aux.unwrap_or(&self.gpu.dummy),
                        ),
                    },
                ],
            });
        let mut pass = self
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kerf-kernel"),
                timestamp_writes: None,
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(groups.0, groups.1, 1);
    }

    fn kernel(
        &mut self,
        pipeline: &wgpu::ComputePipeline,
        uniforms: &Uniforms,
        src: &wgpu::TextureView,
        aux: Option<&wgpu::TextureView>,
        size: (u32, u32),
    ) -> Picture {
        let out = self.take(size.0, size.1);
        let layout = self.gpu.layout.clone();
        self.dispatch(
            pipeline,
            &layout,
            &uniforms.bytes(),
            (src, &out.view, aux),
            (size.0.div_ceil(16), size.1.div_ceil(16)),
        );
        out
    }

    /// Upload bytes that only this frame reads. A texture written this way is written before the
    /// frame's commands run, so it is never one of the frame's working pictures.
    fn upload(
        &mut self,
        format: wgpu::TextureFormat,
        (w, h): (u32, u32),
        data: &[u8],
        row_bytes: u32,
    ) -> wgpu::Texture {
        let key = (
            w,
            h,
            format,
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
        );
        let texture = self.gpu.texture(key);
        self.gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.used.push((key, texture.clone()));
        texture
    }

    /// A transparent picture.
    pub fn clear(&mut self, w: u32, h: u32) -> Picture {
        self.filled(w, h, [0.0; 4])
    }

    /// A picture of one premultiplied color.
    pub fn filled(&mut self, w: u32, h: u32, color: [f32; 4]) -> Picture {
        let gpu = self.gpu;
        let uniforms = Uniforms {
            p: color,
            ..Uniforms::default()
        };
        let dummy = gpu.dummy.clone();
        self.kernel(&gpu.fill, &uniforms, &dummy, None, (w, h))
    }

    /// Straight sRGB-encoded RGBA bytes, `stride` bytes a row, in linear light. Opaque unless
    /// `alpha` says the picture's transparency counts.
    pub fn rgba8(&mut self, rgba: &[u8], w: u32, h: u32, stride: u32, alpha: bool) -> Picture {
        let gpu = self.gpu;
        let view = self
            .upload(wgpu::TextureFormat::Rgba8Unorm, (w, h), rgba, stride)
            .create_view(&Default::default());
        let uniforms = Uniforms {
            a: [0, u32::from(alpha), 0, 0],
            ..Uniforms::default()
        };
        self.kernel(&gpu.decode, &uniforms, &view, None, (w, h))
    }

    /// A picture the CPU drew, such as a title.
    pub fn image(&mut self, image: &Image) -> Picture {
        let (w, h) = (image.w as u32, image.h as u32);
        let bytes: Vec<u8> = image.px.iter().flat_map(|v| v.to_le_bytes()).collect();
        let source = self.upload(WORKING_FORMAT, (w, h), &bytes, w * 16);
        // Into a working picture, which later commands may write.
        let out = self.take(w, h);
        self.encoder.copy_texture_to_texture(
            source.as_image_copy(),
            out.texture.as_image_copy(),
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        out
    }

    /// `picture` with every value by `coverage`, one number a pixel.
    pub fn multiply(&mut self, picture: Picture, coverage: &[f32]) -> Picture {
        let gpu = self.gpu;
        let (w, h) = (picture.w, picture.h);
        let bytes: Vec<u8> = coverage.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self
            .upload(wgpu::TextureFormat::R32Float, (w, h), &bytes, w * 4)
            .create_view(&Default::default());
        let out = self.kernel(
            &gpu.multiply,
            &Uniforms::default(),
            &picture.view,
            Some(&view),
            (w, h),
        );
        self.release(picture);
        out
    }

    /// `picture` through `color.basic`'s `correction`.
    pub fn correct(&mut self, picture: Picture, correction: &Correction) -> Picture {
        let gpu = self.gpu;
        let parts = u32::from(correction.exposure != 0.0)
            | u32::from(correction.eq()) << 1
            | u32::from(correction.balance()) << 2
            | u32::from(correction.vibrance != 0.0) << 3;
        let [red, green, blue] = correction.shadows;
        let uniforms = Uniforms {
            a: [0, parts, 0, 0],
            p: [
                correction.gain,
                correction.contrast,
                correction.brightness,
                correction.saturation,
            ],
            rect: [
                correction.gamma,
                correction.highlights,
                correction.vibrance,
                0.0,
            ],
            range: [red, green, blue, 0.0],
            ..Uniforms::default()
        };
        let size = (picture.w, picture.h);
        let out = self.kernel(&gpu.correct, &uniforms, &picture.view, None, size);
        self.release(picture);
        out
    }

    /// `source` placed through `m`, its pixels to `canvas`'s, cropped to `rect` of it, over
    /// `canvas` with `opacity` and `mode`, as `kerf_draw::blend::composite_placed` does.
    pub fn composite(
        &mut self,
        canvas: Picture,
        source: &Picture,
        m: &Affine,
        rect: Rect,
        opacity: f32,
        mode: Blend,
    ) -> Picture {
        let size = (source.w as usize, source.h as usize);
        if Landing::halves(size, m) {
            // Shrinking to under half: from a picture halved first, as the CPU does.
            let gpu = self.gpu;
            let half = self.kernel(
                &gpu.halve,
                &Uniforms::default(),
                &source.view,
                None,
                ((source.w / 2).max(1), (source.h / 2).max(1)),
            );
            let m = m.then_apply(&Affine::scale(2.0, 2.0));
            let rect = Rect::new(rect.x / 2.0, rect.y / 2.0, rect.w / 2.0, rect.h / 2.0);
            let out = self.composite(canvas, &half, &m, rect, opacity, mode);
            self.release(half);
            return out;
        }
        let canvas_size = (canvas.w as usize, canvas.h as usize);
        let Some(at) = Landing::new(size, canvas_size, m, rect) else {
            return canvas;
        };
        let inverse = &at.inverse;
        let uniforms = Uniforms {
            a: [mode as u32, 0, 0, 0],
            bounds: [
                at.columns.start as u32,
                at.columns.end as u32,
                at.rows.start as u32,
                at.rows.end as u32,
            ],
            m0: [
                inverse.a as f32,
                inverse.b as f32,
                inverse.c as f32,
                inverse.d as f32,
            ],
            m1: [
                inverse.e as f32,
                inverse.f as f32,
                at.across as f32,
                at.down as f32,
            ],
            rect: [
                at.rect.x as f32,
                at.rect.y as f32,
                at.rect.right() as f32,
                at.rect.bottom() as f32,
            ],
            range: [
                at.lo[0] as f32,
                at.lo[1] as f32,
                at.hi[0] as f32,
                at.hi[1] as f32,
            ],
            p: [opacity, 0.0, 0.0, 0.0],
        };
        let gpu = self.gpu;
        let out = self.kernel(
            &gpu.composite,
            &uniforms,
            &canvas.view,
            Some(&source.view),
            (canvas.w, canvas.h),
        );
        self.release(canvas);
        out
    }

    /// Submit the frame and read `picture` back as sRGB-encoded RGBA bytes over black. `None` when
    /// the device failed, and the CPU should draw the frame.
    pub fn finish(mut self, picture: &Picture) -> Option<Vec<u8>> {
        let bytes = self.finish_to_rgba8(picture);
        if self.gpu.failed.load(Ordering::Relaxed) {
            return None;
        }
        bytes
    }

    /// Submit the frame and read `picture` back as it is, for tests of the kernels.
    pub fn read_floats(mut self, picture: &Picture) -> Option<Vec<f32>> {
        let row = align_row(picture.w * 16);
        let buffer = self.staging(u64::from(row) * u64::from(picture.h));
        self.copy_out(&picture.texture, &buffer, row, (picture.w, picture.h));
        let (w, h) = (picture.w as usize, picture.h as usize);
        let rows = self.submit_and_map(buffer, |data| {
            let mut out = Vec::with_capacity(w * h * 4);
            for line in data.chunks_exact(row as usize) {
                out.extend(
                    line[..w * 16]
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                );
            }
            out
        })?;
        Some(rows)
    }

    fn finish_to_rgba8(&mut self, picture: &Picture) -> Option<Vec<u8>> {
        let gpu = self.gpu;
        let (w, h) = (picture.w, picture.h);
        let key = (
            w,
            h,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let target = gpu.texture(key);
        let target_view = target.create_view(&Default::default());
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kerf-finish"),
            layout: &gpu.finish_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&picture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&target_view),
                },
            ],
        });
        {
            let mut pass = self
                .encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("kerf-finish"),
                    timestamp_writes: None,
                });
            pass.set_pipeline(&gpu.finish);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(w.div_ceil(16), h.div_ceil(16), 1);
        }
        let row = align_row(w * 4);
        let buffer = self.staging(u64::from(row) * u64::from(h));
        self.copy_out(&target, &buffer, row, (w, h));
        self.used.push((key, target));
        let tight = w as usize * 4;
        self.submit_and_map(buffer, |data| {
            let mut out = Vec::with_capacity(tight * h as usize);
            for line in data.chunks_exact(row as usize) {
                out.extend_from_slice(&line[..tight]);
            }
            out
        })
    }

    fn staging(&self, size: u64) -> wgpu::Buffer {
        if let Some(buffer) = self.gpu.pool().staging.get_mut(&size).and_then(Vec::pop) {
            return buffer;
        }
        self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kerf-readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }

    fn copy_out(
        &mut self,
        texture: &wgpu::Texture,
        buffer: &wgpu::Buffer,
        row: u32,
        (w, h): (u32, u32),
    ) {
        self.encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Submit what was recorded, give the frame's textures back, wait, and read `buffer`.
    fn submit_and_map<T>(
        &mut self,
        buffer: wgpu::Buffer,
        read: impl FnOnce(&[u8]) -> T,
    ) -> Option<T> {
        let encoder = std::mem::replace(
            &mut self.encoder,
            self.gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("kerf-unused"),
                }),
        );
        let submission = self.gpu.queue.submit([encoder.finish()]);
        self.working.clear();
        self.gpu.give_back(std::mem::take(&mut self.used));
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.gpu
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(FRAME_TIMEOUT),
            })
            .ok()?;
        receiver.recv_timeout(FRAME_TIMEOUT).ok()?.ok()?;
        let out = {
            let data = slice.get_mapped_range().ok()?;
            read(&data)
        };
        buffer.unmap();
        let size = buffer.size();
        self.gpu
            .pool()
            .staging
            .entry(size)
            .or_default()
            .push(buffer);
        Some(out)
    }
}

/// The bytes a row of a texture takes in a buffer.
fn align_row(bytes: u32) -> u32 {
    bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_draw::blend::composite_placed;

    /// A picture of premultiplied colors, varied over its pixels, with partial transparency.
    fn test_picture(w: usize, h: usize, seed: f32) -> Image {
        let mut image = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let u = x as f32 / w as f32;
                let v = y as f32 / h as f32;
                let a = if (x / 7 + y / 5) % 3 == 0 { 0.5 } else { 1.0 };
                let color = [
                    (u + seed).fract() * a,
                    v * a,
                    ((u + v) * 0.5 + seed * 0.3).fract() * a,
                    a,
                ];
                image.px[(y * w + x) * 4..][..4].copy_from_slice(&color);
            }
        }
        image
    }

    fn assert_close(gpu: &[f32], cpu: &[f32], tolerance: f32, what: &str) {
        assert_eq!(gpu.len(), cpu.len(), "{what}");
        let mut worst = 0.0f32;
        for (g, c) in gpu.iter().zip(cpu) {
            worst = worst.max((g - c).abs());
        }
        assert!(worst <= tolerance, "{what}: differs by {worst}");
    }

    #[test]
    fn every_blend_mode_placed_matches_the_cpu() {
        let Some(gpu) = shared() else {
            eprintln!("no GPU: nothing to compare");
            return;
        };
        let (cw, ch) = (96, 64);
        let source = test_picture(48, 32, 0.25);
        let backdrop = test_picture(cw, ch, 0.6);
        let placements = [
            (Affine::IDENTITY, "unmoved"),
            (Affine::translate(10.5, 7.25), "moved"),
            (
                Affine::motion(
                    kerf_draw::Vec2::new(40.0, 30.0),
                    kerf_draw::Vec2::new(1.3, 0.9),
                    17.0,
                    kerf_draw::Vec2::new(24.0, 16.0),
                ),
                "turned",
            ),
        ];
        let rect = Rect::new(2.0, 1.5, 40.0, 28.0);
        for (m, how) in &placements {
            for mode in Blend::ALL {
                let mut cpu = backdrop.clone();
                composite_placed(&mut cpu, &source, m, rect, 0.8, mode);
                let mut frame = gpu.frame();
                let canvas = frame.image(&backdrop);
                let layer = frame.image(&source);
                let out = frame.composite(canvas, &layer, m, rect, 0.8, mode);
                let floats = frame.read_floats(&out).expect("read back");
                // Blend modes through pow differ in the last bits; a step of a byte is 1/255.
                assert_close(&floats, &cpu.px, 2e-3, &format!("{mode:?} {how}"));
            }
        }
    }

    #[test]
    fn a_shrunk_picture_is_sampled_from_its_halves() {
        let Some(gpu) = shared() else {
            return;
        };
        let source = test_picture(64, 48, 0.1);
        let backdrop = test_picture(32, 24, 0.7);
        let m = Affine::scale(0.3, 0.3);
        let rect = Rect::new(0.0, 0.0, 64.0, 48.0);
        let mut cpu = backdrop.clone();
        composite_placed(&mut cpu, &source, &m, rect, 1.0, Blend::Normal);
        let mut frame = gpu.frame();
        let canvas = frame.image(&backdrop);
        let layer = frame.image(&source);
        let out = frame.composite(canvas, &layer, &m, rect, 1.0, Blend::Normal);
        let floats = frame.read_floats(&out).expect("read back");
        assert_close(&floats, &cpu.px, 1e-3, "halved");
    }

    #[test]
    fn bytes_go_up_and_come_back_as_the_cpu_makes_them() {
        let Some(gpu) = shared() else {
            return;
        };
        let (w, h) = (37usize, 11usize);
        let rgba: Vec<u8> = (0..w * h * 4)
            .map(|i| if i % 4 == 3 { 128 } else { (i * 3 % 256) as u8 })
            .collect();
        for alpha in [false, true] {
            let cpu = Image::from_rgba8(&rgba, w, h, w * 4, alpha);
            let mut frame = gpu.frame();
            let picture = frame.rgba8(&rgba, w as u32, h as u32, w as u32 * 4, alpha);
            let floats = frame.read_floats(&picture).expect("read back");
            assert_close(&floats, &cpu.px, 1e-4, "decoded");

            let mut frame = gpu.frame();
            let picture = frame.rgba8(&rgba, w as u32, h as u32, w as u32 * 4, alpha);
            let bytes = frame.finish(&picture).expect("finished");
            let expected = cpu.to_rgba8_over_black();
            assert_eq!(bytes.len(), expected.len());
            for (g, c) in bytes.iter().zip(&expected) {
                assert!(g.abs_diff(*c) <= 1, "{g} against {c}");
            }
        }
    }

    #[test]
    fn coverage_multiplies_every_value() {
        let Some(gpu) = shared() else {
            return;
        };
        let source = test_picture(16, 8, 0.4);
        let coverage: Vec<f32> = (0..16 * 8).map(|i| (i % 5) as f32 / 4.0).collect();
        let mut cpu = source.clone();
        cpu.mul_coverage(&coverage);
        let mut frame = gpu.frame();
        let picture = frame.image(&source);
        let out = frame.multiply(picture, &coverage);
        let floats = frame.read_floats(&out).expect("read back");
        assert_close(&floats, &cpu.px, 1e-5, "covered");
    }

    #[test]
    fn corrections_match_the_cpu() {
        let Some(gpu) = shared() else {
            return;
        };
        let source = test_picture(48, 32, 0.15);
        let cases: &[&[(&str, f64)]] = &[
            &[("exposure", 0.5)],
            &[("exposure", -1.2)],
            &[("contrast", 1.4)],
            &[("brightness", 0.1)],
            &[("saturation", 1.6)],
            &[("gamma", 1.4)],
            &[("contrast", 1.2), ("gamma", 0.8), ("saturation", 0.5)],
            &[("temperature", 400.0), ("tint", 0.2)],
            &[("shadows", 0.3), ("highlights", -0.2)],
            &[("vibrance", 0.6)],
            &[("vibrance", -0.6)],
            &[
                ("exposure", 0.3),
                ("contrast", 1.1),
                ("temperature", -300.0),
                ("vibrance", 0.3),
            ],
        ];
        for case in cases {
            let correction = Correction::new(&|name| {
                case.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| *value)
                    .unwrap_or(match name {
                        "mix" | "contrast" | "saturation" | "gamma" => 1.0,
                        _ => 0.0,
                    })
            });
            let mut cpu = source.clone();
            cpu.map_encoded(|rgb| correction.apply(rgb));
            let mut frame = gpu.frame();
            let picture = frame.image(&source);
            let out = frame.correct(picture, &correction);
            let floats = frame.read_floats(&out).expect("read back");
            assert_close(&floats, &cpu.px, 4e-3, &format!("{case:?}"));
        }
    }

    #[test]
    fn a_filled_picture_is_its_color() {
        let Some(gpu) = shared() else {
            return;
        };
        let mut frame = gpu.frame();
        let picture = frame.filled(5, 3, [0.1, 0.2, 0.3, 0.4]);
        let floats = frame.read_floats(&picture).expect("read back");
        for pixel in floats.chunks_exact(4) {
            assert_eq!(pixel, [0.1, 0.2, 0.3, 0.4]);
        }
    }
}
