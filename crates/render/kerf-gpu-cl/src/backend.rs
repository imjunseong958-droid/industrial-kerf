//! A device for wgpu that runs on OpenCL: what kerf-gpu and Kerf's kernels ask of wgpu, and no
//! more. There is no triangle here, no surface and no sampler, and asking for one panics with
//! what was asked.
//!
//! A buffer and a texture are each bytes on the device. A command encoder writes down what it is
//! told, and submitting it hands each thing in turn to the device's one queue, which runs them in
//! that order. Reading a buffer back waits for what was submitted before it.

use crate::cl::{Cl, Memory};
use crate::format::Texel;
use crate::translate::{self, Bound, Translated};
use opencl3::kernel::{ExecuteKernel, Kernel};
use opencl3::memory::ClMem;
use opencl3::types::{cl_int, cl_uint, CL_BLOCKING};
use std::ops::Range;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use wgpu::custom::{
    AdapterInterface, BindGroupInterface, BindGroupLayoutInterface, BufferInterface,
    BufferMapCallback, BufferMappedRangeInterface, CommandBufferInterface, CommandEncoderInterface,
    ComputePassInterface, ComputePipelineInterface, DeviceInterface, DispatchBindGroup,
    DispatchBindGroupLayout, DispatchBlas, DispatchBuffer, DispatchBufferMappedRange,
    DispatchCommandBuffer, DispatchCommandEncoder, DispatchComputePass, DispatchComputePipeline,
    DispatchDevice, DispatchExternalTexture, DispatchPipelineCache, DispatchPipelineLayout,
    DispatchQuerySet, DispatchQueue, DispatchQueueWriteBuffer, DispatchRenderBundleEncoder,
    DispatchRenderPass, DispatchRenderPipeline, DispatchSampler, DispatchShaderModule,
    DispatchSurface, DispatchSurfaceOutputDetail, DispatchTexture, DispatchTextureView,
    DispatchTlas, PipelineLayoutInterface, PopErrorScopeFuture, QueueInterface,
    RequestDeviceFuture, ShaderCompilationInfoFuture, ShaderModuleInterface, TextureInterface,
    TextureViewInterface,
};
use wgpu::WriteOnly;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What is asked of wgpu that Kerf's kernels never ask.
macro_rules! never {
    ($what:literal) => {
        unimplemented!(concat!(
            "kerf-gpu-cl runs compute kernels alone: no ",
            $what
        ))
    };
}

/// Why something here failed, as wgpu's errors carry it.
#[derive(Debug)]
struct Reason(String);

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Reason {}

#[derive(Default)]
struct Errors {
    /// One for each scope open, the innermost last: the first error in it.
    scopes: Vec<Option<wgpu::Error>>,
    handler: Option<Arc<dyn wgpu::UncapturedErrorHandler>>,
}

/// The device, as every object made on it reaches it.
pub(crate) struct Shared {
    cl: Mutex<Cl>,
    errors: Mutex<Errors>,
    /// What to tell once the queue has run what it was handed.
    waiting: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    submitted: AtomicU64,
    limits: wgpu::Limits,
    info: wgpu::AdapterInfo,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpenCL device `{}`", self.info.name)
    }
}

impl Shared {
    fn new(cl: Cl) -> Self {
        // The widest working picture a buffer holds, 16 bytes a texel.
        let widest = ((cl.most_bytes / 16) as f64).sqrt() as u32;
        let limits = wgpu::Limits {
            max_texture_dimension_2d: widest.min(16384),
            max_buffer_size: cl.most_bytes,
            ..wgpu::Limits::default()
        };
        let info = wgpu::AdapterInfo {
            name: cl.name.clone(),
            vendor: 0,
            device: 0,
            device_type: if cl.on_cpu {
                wgpu::DeviceType::Cpu
            } else {
                wgpu::DeviceType::DiscreteGpu
            },
            device_pci_bus_id: String::new(),
            driver: "OpenCL".to_owned(),
            driver_info: String::new(),
            backend: wgpu::Backend::Noop,
            subgroup_min_size: 1,
            subgroup_max_size: 1,
            transient_saves_memory: None,
            limit_bucket: None,
        };
        Self {
            cl: Mutex::new(cl),
            errors: Mutex::default(),
            waiting: Mutex::default(),
            submitted: AtomicU64::new(0),
            limits,
            info,
        }
    }

    /// Say that something failed: to the scope open, or to whoever watches the device.
    fn report(&self, what: impl Into<String>) {
        let what = what.into();
        let error = wgpu::Error::Validation {
            source: Box::new(Reason(what.clone())),
            description: what,
        };
        let mut errors = lock(&self.errors);
        if let Some(scope) = errors.scopes.last_mut() {
            scope.get_or_insert(error);
            return;
        }
        let handler = errors.handler.clone();
        drop(errors);
        match handler {
            Some(handler) => handler(error),
            None => eprintln!("kerf-gpu-cl: {error}"),
        }
    }
}

/// Bytes on the device and how many.
struct Held {
    memory: Mutex<Memory>,
    bytes: usize,
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bytes on the device", self.bytes)
    }
}

impl Held {
    fn new(shared: &Shared, bytes: usize) -> Option<Arc<Self>> {
        match lock(&shared.cl).memory(bytes) {
            Ok(memory) => Some(Arc::new(Self {
                memory: Mutex::new(memory),
                bytes,
            })),
            Err(why) => {
                shared.report(format!("no memory for {bytes} bytes: {why}"));
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------------- the device

/// What [`crate::open`] makes an adapter of.
#[derive(Debug)]
pub(crate) struct Adapter {
    shared: Arc<Shared>,
}

impl Adapter {
    pub(crate) fn new(cl: Cl) -> Self {
        Self {
            shared: Arc::new(Shared::new(cl)),
        }
    }
}

impl AdapterInterface for Adapter {
    fn request_device(
        &self,
        _desc: &wgpu::DeviceDescriptor<'_>,
    ) -> Pin<Box<dyn RequestDeviceFuture>> {
        let device = DispatchDevice::custom(Device {
            shared: self.shared.clone(),
        });
        let queue = DispatchQueue::custom(Queue {
            shared: self.shared.clone(),
        });
        Box::pin(std::future::ready(Ok((device, queue))))
    }

    fn is_surface_supported(&self, _surface: &DispatchSurface) -> bool {
        false
    }

    fn features(&self) -> wgpu::Features {
        wgpu::Features::empty()
    }

    fn limits(&self) -> wgpu::Limits {
        self.shared.limits.clone()
    }

    fn downlevel_capabilities(&self) -> wgpu::DownlevelCapabilities {
        wgpu::DownlevelCapabilities::default()
    }

    fn get_info(&self) -> wgpu::AdapterInfo {
        self.shared.info.clone()
    }

    fn get_texture_format_features(
        &self,
        _format: wgpu::TextureFormat,
    ) -> wgpu::TextureFormatFeatures {
        wgpu::TextureFormatFeatures {
            allowed_usages: wgpu::TextureUsages::all(),
            flags: wgpu::TextureFormatFeatureFlags::empty(),
        }
    }

    fn get_presentation_timestamp(&self) -> wgpu::PresentationTimestamp {
        wgpu::PresentationTimestamp::INVALID_TIMESTAMP
    }

    fn cooperative_matrix_properties(&self) -> Vec<wgpu::wgt::CooperativeMatrixProperties> {
        Vec::new()
    }
}

#[derive(Debug)]
struct Device {
    shared: Arc<Shared>,
}

#[expect(
    unsafe_code,
    reason = "wgpu's interface declares these functions unsafe; none of them does anything here but refuse"
)]
impl DeviceInterface for Device {
    fn features(&self) -> wgpu::Features {
        wgpu::Features::empty()
    }

    fn limits(&self) -> wgpu::Limits {
        self.shared.limits.clone()
    }

    fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.shared.info.clone()
    }

    fn create_shader_module(
        &self,
        desc: wgpu::ShaderModuleDescriptor<'_>,
        _shader_bound_checks: wgpu::ShaderRuntimeChecks,
    ) -> DispatchShaderModule {
        let parsed = match &desc.source {
            wgpu::ShaderSource::Wgsl(source) => crate::parse(source).map_err(|error| error.0),
            _ => Err("a kernel that is not WGSL".to_owned()),
        };
        DispatchShaderModule::custom(ShaderModule(match parsed {
            Ok(parsed) => Some(Arc::new(parsed)),
            Err(why) => {
                self.shared
                    .report(format!("a shader module was not read: {why}"));
                None
            }
        }))
    }

    unsafe fn create_shader_module_passthrough(
        &self,
        _desc: &wgpu::ShaderModuleDescriptorPassthrough<'_>,
    ) -> DispatchShaderModule {
        never!("shader handed over as built")
    }

    fn create_bind_group_layout(
        &self,
        _desc: &wgpu::BindGroupLayoutDescriptor<'_>,
    ) -> DispatchBindGroupLayout {
        DispatchBindGroupLayout::custom(BindGroupLayout)
    }

    fn create_bind_group(&self, desc: &wgpu::BindGroupDescriptor<'_>) -> DispatchBindGroup {
        let mut bound = Vec::with_capacity(desc.entries.len());
        for entry in desc.entries {
            let what = match &entry.resource {
                wgpu::BindingResource::Buffer(binding) => {
                    match binding.buffer.as_custom::<Buffer>() {
                        // A kernel gets a buffer whole: none of Kerf's binds a part of one.
                        Some(Buffer(inner)) if binding.offset == 0 => {
                            inner.held.clone().map(Bind::Buffer)
                        }
                        _ => None,
                    }
                }
                wgpu::BindingResource::TextureView(view) => view
                    .as_custom::<TextureView>()
                    .and_then(|view| view.0.clone())
                    .map(Bind::Texture),
                _ => None,
            };
            match what {
                Some(what) => bound.push((entry.binding, what)),
                None => self.shared.report(format!(
                    "binding {} is not something a kernel here is handed",
                    entry.binding
                )),
            }
        }
        DispatchBindGroup::custom(BindGroup(Arc::new(bound)))
    }

    fn create_pipeline_layout(
        &self,
        _desc: &wgpu::PipelineLayoutDescriptor<'_>,
    ) -> DispatchPipelineLayout {
        DispatchPipelineLayout::custom(PipelineLayout)
    }

    fn create_render_pipeline(
        &self,
        _desc: &wgpu::RenderPipelineDescriptor<'_>,
    ) -> DispatchRenderPipeline {
        never!("render pipeline")
    }

    fn create_mesh_pipeline(
        &self,
        _desc: &wgpu::MeshPipelineDescriptor<'_>,
    ) -> DispatchRenderPipeline {
        never!("mesh pipeline")
    }

    fn create_compute_pipeline(
        &self,
        desc: &wgpu::ComputePipelineDescriptor<'_>,
    ) -> DispatchComputePipeline {
        let built = (|| {
            let module = desc
                .module
                .as_custom::<ShaderModule>()
                .and_then(|module| module.0.clone())
                .ok_or_else(|| "its shader module was not read".to_owned())?;
            let (module, info) = &*module;
            let entry = match desc.entry_point {
                Some(entry) => entry.to_owned(),
                None => match module.entry_points.as_slice() {
                    [only] => only.name.clone(),
                    _ => return Err("it names none of its module's entry points".to_owned()),
                },
            };
            let mut cl = lock(&self.shared.cl);
            let translated = translate::translate(module, info, &entry, cl.exact)
                .map_err(|error| format!("`{entry}`: {error}"))?;
            let kernel = cl
                .kernel(&translated.source, &translated.kernel)
                .map_err(|why| format!("`{entry}` does not build as OpenCL C: {why}"))?;
            Ok(Arc::new(Pipeline {
                kernel: Mutex::new(kernel),
                translated,
            }))
        })();
        DispatchComputePipeline::custom(ComputePipeline(match built {
            Ok(pipeline) => Some(pipeline),
            Err(why) => {
                self.shared.report(format!("a kernel was not made: {why}"));
                None
            }
        }))
    }

    unsafe fn create_pipeline_cache(
        &self,
        _desc: &wgpu::PipelineCacheDescriptor<'_>,
    ) -> DispatchPipelineCache {
        never!("pipeline cache")
    }

    fn create_buffer(&self, desc: &wgpu::BufferDescriptor<'_>) -> DispatchBuffer {
        DispatchBuffer::custom(Buffer(Arc::new(BufferInner {
            held: Held::new(&self.shared, desc.size as usize),
            shared: self.shared.clone(),
            mapped: Mutex::new(desc.mapped_at_creation.then_some(Mapped::ToWrite)),
        })))
    }

    fn create_texture(&self, desc: &wgpu::TextureDescriptor<'_>) -> DispatchTexture {
        let plain = desc.dimension == wgpu::TextureDimension::D2
            && desc.size.depth_or_array_layers == 1
            && desc.mip_level_count == 1
            && desc.sample_count == 1;
        let inner = match Texel::of(desc.format).filter(|_| plain) {
            Some(texel) => {
                let (w, h) = (desc.size.width, desc.size.height);
                Held::new(&self.shared, w as usize * h as usize * texel.bytes())
                    .map(|held| Arc::new(TextureInner { held, w, h, texel }))
            }
            None => {
                self.shared.report(format!(
                    "a texture of {:?} that is not one flat picture of a kind kernels here read",
                    desc.format
                ));
                None
            }
        };
        DispatchTexture::custom(Texture(inner))
    }

    fn create_external_texture(
        &self,
        _desc: &wgpu::ExternalTextureDescriptor<'_>,
        _planes: &[&wgpu::TextureView],
    ) -> DispatchExternalTexture {
        never!("external texture")
    }

    fn create_blas(
        &self,
        _desc: &wgpu::CreateBlasDescriptor<'_>,
        _sizes: wgpu::BlasGeometrySizeDescriptors,
    ) -> (Option<u64>, DispatchBlas) {
        never!("acceleration structure")
    }

    fn create_tlas(&self, _desc: &wgpu::CreateTlasDescriptor<'_>) -> DispatchTlas {
        never!("acceleration structure")
    }

    fn create_sampler(&self, _desc: &wgpu::SamplerDescriptor<'_>) -> DispatchSampler {
        never!("sampler")
    }

    fn create_query_set(&self, _desc: &wgpu::QuerySetDescriptor<'_>) -> DispatchQuerySet {
        never!("query set")
    }

    fn create_command_encoder(
        &self,
        _desc: &wgpu::CommandEncoderDescriptor<'_>,
    ) -> DispatchCommandEncoder {
        DispatchCommandEncoder::custom(CommandEncoder {
            shared: self.shared.clone(),
            commands: Arc::default(),
        })
    }

    fn create_render_bundle_encoder(
        &self,
        _desc: &wgpu::RenderBundleEncoderDescriptor<'_>,
    ) -> DispatchRenderBundleEncoder {
        never!("render bundle")
    }

    fn set_device_lost_callback(&self, _device_lost_callback: wgpu::custom::BoxDeviceLostCallback) {
        // An OpenCL context is not lost the way a GPU's device is: what fails is reported.
    }

    fn on_uncaptured_error(&self, handler: Arc<dyn wgpu::UncapturedErrorHandler>) {
        lock(&self.shared.errors).handler = Some(handler);
    }

    fn push_error_scope(&self, _filter: wgpu::ErrorFilter) -> u32 {
        let mut errors = lock(&self.shared.errors);
        errors.scopes.push(None);
        errors.scopes.len() as u32 - 1
    }

    fn pop_error_scope(&self, index: u32) -> Pin<Box<dyn PopErrorScopeFuture>> {
        let mut errors = lock(&self.shared.errors);
        // Scopes close innermost first; one closed out of turn takes those inside it with it.
        let mut found = None;
        while errors.scopes.len() > index as usize {
            found = errors.scopes.pop().flatten().or(found);
        }
        Box::pin(std::future::ready(found))
    }

    unsafe fn start_graphics_debugger_capture(&self) {}

    unsafe fn stop_graphics_debugger_capture(&self) {}

    fn poll(
        &self,
        _poll_type: wgpu::wgt::PollType<u64>,
    ) -> Result<wgpu::PollStatus, wgpu::PollError> {
        if let Err(error) = lock(&self.shared.cl).queue.finish() {
            self.shared
                .report(format!("the queue did not finish: {error}"));
        }
        let waiting = std::mem::take(&mut *lock(&self.shared.waiting));
        for tell in waiting {
            tell();
        }
        Ok(wgpu::PollStatus::QueueEmpty)
    }

    fn get_internal_counters(&self) -> wgpu::InternalCounters {
        wgpu::InternalCounters::default()
    }

    fn generate_allocator_report(&self) -> Option<wgpu::AllocatorReport> {
        None
    }

    fn destroy(&self) {}
}

#[derive(Debug)]
struct Queue {
    shared: Arc<Shared>,
}

impl Queue {
    /// `data` into `held`, `at` bytes in, done before this returns.
    fn write(&self, held: &Held, at: usize, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if at
            .checked_add(data.len())
            .is_none_or(|end| end > held.bytes)
        {
            self.shared
                .report("a write past the end of what it writes into");
            return;
        }
        let cl = lock(&self.shared.cl);
        let mut memory = lock(&held.memory);
        #[expect(
            unsafe_code,
            reason = "a blocking write within the buffer's length, checked above, of bytes that live through the call"
        )]
        let wrote = unsafe {
            cl.queue
                .enqueue_write_buffer(&mut memory, CL_BLOCKING, at, data, &[])
        };
        if let Err(error) = wrote {
            self.shared
                .report(format!("a write did not go up: {error}"));
        }
    }
}

impl QueueInterface for Queue {
    fn write_buffer(&self, buffer: &DispatchBuffer, offset: wgpu::BufferAddress, data: &[u8]) {
        match buffer
            .as_custom::<Buffer>()
            .and_then(|buffer| buffer.0.held.as_ref())
        {
            Some(held) => self.write(held, offset as usize, data),
            None => self
                .shared
                .report("a write into a buffer that was not made"),
        }
    }

    fn create_staging_buffer(&self, _size: wgpu::BufferSize) -> Option<DispatchQueueWriteBuffer> {
        None
    }

    fn validate_write_buffer(
        &self,
        _buffer: &DispatchBuffer,
        _offset: wgpu::BufferAddress,
        _size: wgpu::BufferSize,
    ) -> Option<()> {
        None
    }

    fn write_staging_buffer(
        &self,
        _buffer: &DispatchBuffer,
        _offset: wgpu::BufferAddress,
        _staging_buffer: &DispatchQueueWriteBuffer,
    ) {
    }

    fn write_texture(
        &self,
        texture: wgpu::TexelCopyTextureInfo<'_>,
        data: &[u8],
        data_layout: wgpu::TexelCopyBufferLayout,
        size: wgpu::Extent3d,
    ) {
        let Some(inner) = texture
            .texture
            .as_custom::<Texture>()
            .and_then(|texture| texture.0.as_ref())
        else {
            self.shared
                .report("a write into a texture that was not made");
            return;
        };
        let Some(rows) = Rows::new(inner, texture.origin, size, &data_layout, data.len()) else {
            self.shared
                .report("a write that does not fit its texture or its bytes");
            return;
        };
        for span in rows.spans() {
            self.write(
                &inner.held,
                span.texture,
                &data[span.other..span.other + span.bytes],
            );
        }
    }

    fn submit(&self, command_buffers: &mut dyn Iterator<Item = DispatchCommandBuffer>) -> u64 {
        for buffer in command_buffers {
            let Some(buffer) = buffer.as_custom::<CommandBuffer>() else {
                continue;
            };
            let commands = std::mem::take(&mut *lock(&buffer.0));
            for command in commands {
                if let Err(why) = command.run(&self.shared) {
                    self.shared.report(why);
                }
            }
        }
        if let Err(error) = lock(&self.shared.cl).queue.flush() {
            self.shared
                .report(format!("the queue did not take its work: {error}"));
        }
        self.shared.submitted.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn get_timestamp_period(&self) -> f32 {
        1.0
    }

    fn on_submitted_work_done(&self, callback: wgpu::custom::BoxSubmittedWorkDoneCallback) {
        lock(&self.shared.waiting).push(callback);
    }

    fn compact_blas(&self, _blas: &DispatchBlas) -> (Option<u64>, DispatchBlas) {
        never!("acceleration structure")
    }

    fn present(&self, _detail: &DispatchSurfaceOutputDetail) {
        never!("surface")
    }
}

// --------------------------------------------------------------------------- what a kernel is

#[derive(Debug)]
struct ShaderModule(Option<Arc<(naga::Module, naga::valid::ModuleInfo)>>);

impl ShaderModuleInterface for ShaderModule {
    fn get_compilation_info(&self) -> Pin<Box<dyn ShaderCompilationInfoFuture>> {
        Box::pin(std::future::ready(wgpu::CompilationInfo {
            messages: Vec::new(),
        }))
    }
}

#[derive(Debug)]
struct BindGroupLayout;
impl BindGroupLayoutInterface for BindGroupLayout {}

#[derive(Debug)]
struct PipelineLayout;
impl PipelineLayoutInterface for PipelineLayout {}

/// What a binding is bound to.
#[derive(Debug, Clone)]
enum Bind {
    Buffer(Arc<Held>),
    Texture(Arc<TextureInner>),
}

/// What a bind group binds, each with its binding's number.
type Group = Arc<Vec<(u32, Bind)>>;

#[derive(Debug)]
struct BindGroup(Group);
impl BindGroupInterface for BindGroup {}

struct Pipeline {
    /// OpenCL sets a kernel's arguments on the kernel itself, so one run at a time holds it.
    kernel: Mutex<Kernel>,
    translated: Translated,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a kernel with {} bindings",
            self.translated.resources.len()
        )
    }
}

#[derive(Debug)]
struct ComputePipeline(Option<Arc<Pipeline>>);

impl ComputePipelineInterface for ComputePipeline {
    fn get_bind_group_layout(&self, _index: u32) -> DispatchBindGroupLayout {
        DispatchBindGroupLayout::custom(BindGroupLayout)
    }
}

// --------------------------------------------------------------------------- buffers, textures

/// What a buffer's mapping holds.
enum Mapped {
    /// Nothing yet: what is written through it goes up as each view of it is let go.
    ToWrite,
    /// What was read back, from `from` bytes in.
    Read { bytes: Vec<u8>, from: u64 },
}

struct BufferInner {
    shared: Arc<Shared>,
    /// None where the device had no memory for it.
    held: Option<Arc<Held>>,
    mapped: Mutex<Option<Mapped>>,
}

#[derive(Debug, Clone)]
struct Buffer(Arc<BufferInner>);

impl std::fmt::Debug for BufferInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a buffer, {:?}", self.held)
    }
}

impl BufferInterface for Buffer {
    fn map_async(
        &self,
        mode: wgpu::MapMode,
        range: Range<wgpu::BufferAddress>,
        callback: BufferMapCallback,
    ) {
        let inner = &self.0;
        let done = match (&inner.held, mode) {
            (Some(_), wgpu::MapMode::Write) => {
                *lock(&inner.mapped) = Some(Mapped::ToWrite);
                Ok(())
            }
            (Some(held), wgpu::MapMode::Read) => {
                let (from, to) = (range.start as usize, range.end as usize);
                if from > to || to > held.bytes {
                    Err("a map past the end of its buffer".to_owned())
                } else {
                    let mut bytes = vec![0u8; to - from];
                    let cl = lock(&inner.shared.cl);
                    let memory = lock(&held.memory);
                    // The queue runs in order, so this waits for all that was submitted before.
                    #[expect(
                        unsafe_code,
                        reason = "a blocking read within the buffer's length, checked above, into bytes that live through the call"
                    )]
                    let read = unsafe {
                        cl.queue
                            .enqueue_read_buffer(&memory, CL_BLOCKING, from, &mut bytes, &[])
                    };
                    match read {
                        Ok(_) => {
                            *lock(&inner.mapped) = Some(Mapped::Read {
                                bytes,
                                from: range.start,
                            });
                            Ok(())
                        }
                        Err(error) => Err(format!("a buffer did not come back: {error}")),
                    }
                }
            }
            (None, _) => Err("a map of a buffer that was not made".to_owned()),
        };
        let told = match done {
            Ok(()) => Ok(()),
            Err(why) => {
                inner.shared.report(why);
                Err(wgpu::BufferAsyncError)
            }
        };
        lock(&inner.shared.waiting).push(Box::new(move || callback(told)));
    }

    fn get_mapped_range(
        &self,
        sub_range: Range<wgpu::BufferAddress>,
    ) -> Result<DispatchBufferMappedRange, wgpu::MapRangeError> {
        let length = (sub_range.end - sub_range.start) as usize;
        let mut mapped = lock(&self.0.mapped);
        let bytes = match mapped.as_mut() {
            Some(Mapped::ToWrite) => vec![0u8; length],
            Some(Mapped::Read { bytes, from }) => {
                let start = sub_range.start.saturating_sub(*from) as usize;
                if sub_range.start == *from && length == bytes.len() {
                    // All of it: handed over as it is.
                    std::mem::take(bytes)
                } else {
                    bytes.get(start..start + length).unwrap_or(&[]).to_vec()
                }
            }
            None => Vec::new(),
        };
        let writes = matches!(mapped.as_ref(), Some(Mapped::ToWrite));
        Ok(DispatchBufferMappedRange::custom(MappedRange {
            bytes,
            into: writes.then(|| (self.0.clone(), sub_range.start as usize)),
        }))
    }

    fn unmap(&self) {
        *lock(&self.0.mapped) = None;
    }

    fn destroy(&self) {}
}

/// A view of a mapped buffer: bytes of its own, which go up as it is let go where the mapping
/// is one to write through.
struct MappedRange {
    bytes: Vec<u8>,
    into: Option<(Arc<BufferInner>, usize)>,
}

impl std::fmt::Debug for MappedRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} mapped bytes", self.bytes.len())
    }
}

#[expect(
    unsafe_code,
    reason = "wgpu's interface declares these functions unsafe; here they lend bytes this view owns"
)]
impl BufferMappedRangeInterface for MappedRange {
    fn len(&self) -> usize {
        self.bytes.len()
    }

    unsafe fn read_slice(&self) -> &[u8] {
        &self.bytes
    }

    unsafe fn write_slice(&mut self) -> WriteOnly<'_, [u8]> {
        WriteOnly::from_mut(self.bytes.as_mut_slice())
    }
}

impl Drop for MappedRange {
    fn drop(&mut self) {
        let Some((buffer, at)) = self.into.take() else {
            return;
        };
        if let Some(held) = &buffer.held {
            Queue {
                shared: buffer.shared.clone(),
            }
            .write(held, at, &self.bytes);
        }
    }
}

#[derive(Debug)]
struct TextureInner {
    held: Arc<Held>,
    w: u32,
    h: u32,
    texel: Texel,
}

#[derive(Debug)]
struct Texture(Option<Arc<TextureInner>>);

impl TextureInterface for Texture {
    fn create_view(&self, _desc: &wgpu::TextureViewDescriptor<'_>) -> DispatchTextureView {
        DispatchTextureView::custom(TextureView(self.0.clone()))
    }

    fn destroy(&self) {}
}

#[derive(Debug)]
struct TextureView(Option<Arc<TextureInner>>);
impl TextureViewInterface for TextureView {}

// ------------------------------------------------------------------------------------ commands

/// Bytes of a texture's rows beside bytes somewhere else: one run of bytes a row, or all the rows
/// as one run where nothing is between them on either side.
struct Rows {
    /// Where the first row's bytes start in the texture, and in the other.
    texture: usize,
    other: usize,
    /// A row's bytes, and from one row's start to the next in the texture, and in the other.
    bytes: usize,
    texture_step: usize,
    other_step: usize,
    count: usize,
}

struct Span {
    texture: usize,
    other: usize,
    bytes: usize,
}

impl Rows {
    /// `size` of `inner` from `origin`, beside bytes laid out as `layout` says, of which there
    /// are `other_bytes`. None where either side is too small for it.
    fn new(
        inner: &TextureInner,
        origin: wgpu::Origin3d,
        size: wgpu::Extent3d,
        layout: &wgpu::TexelCopyBufferLayout,
        other_bytes: usize,
    ) -> Option<Self> {
        let texel = inner.texel.bytes();
        let (w, h) = (size.width as usize, size.height as usize);
        let (x, y) = (origin.x as usize, origin.y as usize);
        if origin.z != 0 || size.depth_or_array_layers > 1 {
            return None;
        }
        if x + w > inner.w as usize || y + h > inner.h as usize {
            return None;
        }
        let bytes = w * texel;
        let other_step = layout.bytes_per_row.map_or(bytes, |step| step as usize);
        let other = layout.offset as usize;
        if h > 0
            && (other_step < bytes && h > 1 || other + other_step * (h - 1) + bytes > other_bytes)
        {
            return None;
        }
        Some(Self {
            texture: (y * inner.w as usize + x) * texel,
            other,
            bytes,
            texture_step: inner.w as usize * texel,
            other_step,
            count: h,
        })
    }

    fn spans(&self) -> Vec<Span> {
        if self.count == 0 || self.bytes == 0 {
            return Vec::new();
        }
        if self.texture_step == self.bytes && self.other_step == self.bytes {
            return vec![Span {
                texture: self.texture,
                other: self.other,
                bytes: self.bytes * self.count,
            }];
        }
        (0..self.count)
            .map(|row| Span {
                texture: self.texture + row * self.texture_step,
                other: self.other + row * self.other_step,
                bytes: self.bytes,
            })
            .collect()
    }
}

enum Command {
    Run {
        pipeline: Arc<Pipeline>,
        groups: Vec<Option<Group>>,
        count: [u32; 3],
    },
    /// Bytes from one place on the device to another: from, to, how many.
    Copy {
        from: Arc<Held>,
        to: Arc<Held>,
        runs: Vec<(usize, usize, usize)>,
    },
}

impl Command {
    fn run(self, shared: &Shared) -> Result<(), String> {
        let cl = lock(&shared.cl);
        match self {
            Command::Copy { from, to, runs } => {
                if Arc::ptr_eq(&from, &to) {
                    return Err("a copy from a thing into itself".to_owned());
                }
                let (source, mut target) = (lock(&from.memory), lock(&to.memory));
                for (at, into, bytes) in runs {
                    if bytes == 0 {
                        continue;
                    }
                    if at + bytes > from.bytes || into + bytes > to.bytes {
                        return Err("a copy past the end of what it copies".to_owned());
                    }
                    #[expect(
                        unsafe_code,
                        reason = "a copy between two buffers within both their lengths, checked above"
                    )]
                    unsafe {
                        cl.queue
                            .enqueue_copy_buffer(&source, &mut target, at, into, bytes, &[])
                    }
                    .map_err(|error| format!("a copy was not taken: {error}"))?;
                }
                Ok(())
            }
            Command::Run {
                pipeline,
                groups,
                count,
            } => {
                let translated = &pipeline.translated;
                let sizes: Vec<usize> = count
                    .iter()
                    .zip(translated.workgroup)
                    .map(|(count, group)| *count as usize * group as usize)
                    .collect();
                if sizes.contains(&0) {
                    return Ok(());
                }
                let kernel = lock(&pipeline.kernel);
                let mut run = ExecuteKernel::new(&kernel);
                for resource in &translated.resources {
                    let bound = groups
                        .get(resource.group as usize)
                        .and_then(Option::as_ref)
                        .and_then(|group| {
                            group
                                .iter()
                                .find(|(binding, _)| *binding == resource.binding)
                        })
                        .map(|(_, bound)| bound);
                    match (resource.bound, bound) {
                        (Bound::Buffer { least }, Some(Bind::Buffer(held)))
                            if held.bytes >= least as usize =>
                        {
                            let memory = lock(&held.memory).get();
                            let length = held.bytes as cl_uint;
                            #[expect(
                                unsafe_code,
                                reason = "hands the kernel a buffer and its length, the two arguments the kernel's source declares for this binding"
                            )]
                            unsafe {
                                run.set_arg(&memory).set_arg(&length);
                            }
                        }
                        (Bound::Texture, Some(Bind::Texture(texture))) => {
                            let memory = lock(&texture.held.memory).get();
                            let info: [cl_int; 4] = [
                                texture.w as cl_int,
                                texture.h as cl_int,
                                texture.texel.code(),
                                0,
                            ];
                            #[expect(
                                unsafe_code,
                                reason = "hands the kernel a texture's bytes and its width, height and kind, the two arguments the kernel's source declares for this binding"
                            )]
                            unsafe {
                                run.set_arg(&memory).set_arg(&info);
                            }
                        }
                        (wanted, found) => {
                            let found = match found {
                                Some(Bind::Buffer(held)) => {
                                    format!("a buffer of {} bytes", held.bytes)
                                }
                                Some(Bind::Texture(texture)) => {
                                    format!("a texture of {:?}", texture.texel)
                                }
                                None => "nothing".to_owned(),
                            };
                            let wanted = match wanted {
                                Bound::Buffer { least } => {
                                    format!("a buffer of at least {least} bytes")
                                }
                                Bound::Texture => "a texture".to_owned(),
                            };
                            return Err(format!(
                                "group {} binding {} is handed {found} where its kernel reads {wanted}",
                                resource.group, resource.binding
                            ));
                        }
                    }
                }
                if translated.resources.is_empty() {
                    let none: cl_int = 0;
                    #[expect(
                        unsafe_code,
                        reason = "the one argument a kernel with no binding declares"
                    )]
                    unsafe {
                        run.set_arg(&none);
                    }
                }
                run.set_global_work_sizes(&sizes);
                let group: Vec<usize> = translated
                    .workgroup
                    .iter()
                    .map(|size| *size as usize)
                    .collect();
                if group.iter().product::<usize>() <= cl.most_in_group {
                    run.set_local_work_sizes(&group);
                }
                #[expect(
                    unsafe_code,
                    reason = "every argument the kernel declares was set above, each to memory that lives until the queue has run it"
                )]
                unsafe { run.enqueue_nd_range(&cl.queue) }
                    .map_err(|error| format!("a kernel was not taken: {error}"))?;
                Ok(())
            }
        }
    }
}

type Commands = Arc<Mutex<Vec<Command>>>;

struct CommandEncoder {
    shared: Arc<Shared>,
    commands: Commands,
}

impl std::fmt::Debug for CommandEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} commands written down", lock(&self.commands).len())
    }
}

impl CommandEncoder {
    fn texture<'a>(
        &self,
        info: &'a wgpu::TexelCopyTextureInfo<'_>,
    ) -> Option<&'a Arc<TextureInner>> {
        let found = info
            .texture
            .as_custom::<Texture>()
            .and_then(|texture| texture.0.as_ref());
        if found.is_none() {
            self.shared.report("a copy of a texture that was not made");
        }
        found
    }

    fn buffer<'a>(&self, buffer: Option<&'a Buffer>) -> Option<&'a Arc<Held>> {
        let found = buffer.and_then(|buffer| buffer.0.held.as_ref());
        if found.is_none() {
            self.shared.report("a copy of a buffer that was not made");
        }
        found
    }
}

impl CommandEncoderInterface for CommandEncoder {
    fn copy_buffer_to_buffer(
        &self,
        source: &DispatchBuffer,
        source_offset: wgpu::BufferAddress,
        destination: &DispatchBuffer,
        destination_offset: wgpu::BufferAddress,
        copy_size: Option<wgpu::BufferAddress>,
    ) {
        let (Some(from), Some(to)) = (
            self.buffer(source.as_custom()),
            self.buffer(destination.as_custom()),
        ) else {
            return;
        };
        let bytes = copy_size.map_or(from.bytes.saturating_sub(source_offset as usize), |size| {
            size as usize
        });
        lock(&self.commands).push(Command::Copy {
            from: from.clone(),
            to: to.clone(),
            runs: vec![(source_offset as usize, destination_offset as usize, bytes)],
        });
    }

    fn copy_buffer_to_texture(
        &self,
        source: wgpu::TexelCopyBufferInfo<'_>,
        destination: wgpu::TexelCopyTextureInfo<'_>,
        copy_size: wgpu::Extent3d,
    ) {
        let (Some(from), Some(to)) = (
            self.buffer(source.buffer.as_custom()),
            self.texture(&destination),
        ) else {
            return;
        };
        match Rows::new(
            to,
            destination.origin,
            copy_size,
            &source.layout,
            from.bytes,
        ) {
            Some(rows) => lock(&self.commands).push(Command::Copy {
                from: from.clone(),
                to: to.held.clone(),
                runs: rows
                    .spans()
                    .iter()
                    .map(|span| (span.other, span.texture, span.bytes))
                    .collect(),
            }),
            None => self
                .shared
                .report("a copy that does not fit its texture or its buffer"),
        }
    }

    fn copy_texture_to_buffer(
        &self,
        source: wgpu::TexelCopyTextureInfo<'_>,
        destination: wgpu::TexelCopyBufferInfo<'_>,
        copy_size: wgpu::Extent3d,
    ) {
        let (Some(from), Some(to)) = (
            self.texture(&source),
            self.buffer(destination.buffer.as_custom()),
        ) else {
            return;
        };
        match Rows::new(
            from,
            source.origin,
            copy_size,
            &destination.layout,
            to.bytes,
        ) {
            Some(rows) => lock(&self.commands).push(Command::Copy {
                from: from.held.clone(),
                to: to.clone(),
                runs: rows
                    .spans()
                    .iter()
                    .map(|span| (span.texture, span.other, span.bytes))
                    .collect(),
            }),
            None => self
                .shared
                .report("a copy that does not fit its texture or its buffer"),
        }
    }

    fn copy_texture_to_texture(
        &self,
        source: wgpu::TexelCopyTextureInfo<'_>,
        destination: wgpu::TexelCopyTextureInfo<'_>,
        copy_size: wgpu::Extent3d,
    ) {
        let (Some(from), Some(to)) = (self.texture(&source), self.texture(&destination)) else {
            return;
        };
        // The source's rows, as if into bytes laid out as the target's rows are.
        let texel = to.texel.bytes();
        let fits = from.texel.bytes() == texel
            && destination.origin.z == 0
            && destination.origin.x + copy_size.width <= to.w
            && destination.origin.y + copy_size.height <= to.h;
        let layout = wgpu::TexelCopyBufferLayout {
            offset: (destination.origin.y as u64 * to.w as u64 + destination.origin.x as u64)
                * texel as u64,
            bytes_per_row: Some(to.w * texel as u32),
            rows_per_image: None,
        };
        match Rows::new(from, source.origin, copy_size, &layout, to.held.bytes).filter(|_| fits) {
            Some(rows) => lock(&self.commands).push(Command::Copy {
                from: from.held.clone(),
                to: to.held.clone(),
                runs: rows
                    .spans()
                    .iter()
                    .map(|span| (span.texture, span.other, span.bytes))
                    .collect(),
            }),
            None => self.shared.report("a copy that does not fit its textures"),
        }
    }

    fn begin_compute_pass(&self, _desc: &wgpu::ComputePassDescriptor<'_>) -> DispatchComputePass {
        DispatchComputePass::custom(ComputePass {
            commands: self.commands.clone(),
            pipeline: None,
            groups: Vec::new(),
        })
    }

    fn begin_render_pass(&self, _desc: &wgpu::RenderPassDescriptor<'_>) -> DispatchRenderPass {
        never!("render pass")
    }

    fn finish(&mut self) -> DispatchCommandBuffer {
        let commands = std::mem::take(&mut *lock(&self.commands));
        DispatchCommandBuffer::custom(CommandBuffer(Mutex::new(commands)))
    }

    fn clear_texture(
        &self,
        _texture: &DispatchTexture,
        _subresource_range: &wgpu::ImageSubresourceRange,
    ) {
        never!("clearing of a texture")
    }

    fn clear_buffer(
        &self,
        _buffer: &DispatchBuffer,
        _offset: wgpu::BufferAddress,
        _size: Option<wgpu::BufferAddress>,
    ) {
        never!("clearing of a buffer")
    }

    fn insert_debug_marker(&self, _label: &str) {}

    fn push_debug_group(&self, _label: &str) {}

    fn pop_debug_group(&self) {}

    fn write_timestamp(&self, _query_set: &DispatchQuerySet, _query_index: u32) {
        never!("query set")
    }

    fn resolve_query_set(
        &self,
        _query_set: &DispatchQuerySet,
        _first_query: u32,
        _query_count: u32,
        _destination: &DispatchBuffer,
        _destination_offset: wgpu::BufferAddress,
    ) {
        never!("query set")
    }

    fn mark_acceleration_structures_built<'a>(
        &self,
        _blas: &mut dyn Iterator<Item = &'a wgpu::Blas>,
        _tlas: &mut dyn Iterator<Item = &'a wgpu::Tlas>,
    ) {
        never!("acceleration structure")
    }

    fn build_acceleration_structures<'a>(
        &self,
        _blas: &mut dyn Iterator<Item = &'a wgpu::BlasBuildEntry<'a>>,
        _tlas: &mut dyn Iterator<Item = &'a wgpu::Tlas>,
    ) {
        never!("acceleration structure")
    }

    fn transition_resources<'a>(
        &mut self,
        _buffer_transitions: &mut dyn Iterator<
            Item = wgpu::wgt::BufferTransition<&'a DispatchBuffer>,
        >,
        _texture_transitions: &mut dyn Iterator<
            Item = wgpu::wgt::TextureTransition<&'a DispatchTexture>,
        >,
    ) {
    }
}

struct ComputePass {
    commands: Commands,
    pipeline: Option<Arc<Pipeline>>,
    groups: Vec<Option<Group>>,
}

impl std::fmt::Debug for ComputePass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a compute pass")
    }
}

impl ComputePassInterface for ComputePass {
    fn set_pipeline(&mut self, pipeline: &DispatchComputePipeline) {
        self.pipeline = pipeline
            .as_custom::<ComputePipeline>()
            .and_then(|pipeline| pipeline.0.clone());
    }

    fn set_bind_group(
        &mut self,
        index: u32,
        bind_group: Option<&DispatchBindGroup>,
        _offsets: &[wgpu::DynamicOffset],
    ) {
        let index = index as usize;
        if self.groups.len() <= index {
            self.groups.resize(index + 1, None);
        }
        self.groups[index] = bind_group
            .and_then(|group| group.as_custom::<BindGroup>())
            .map(|group| group.0.clone());
    }

    fn set_immediates(&mut self, _offset: u32, _data: &[u8]) {
        never!("immediate data")
    }

    fn insert_debug_marker(&mut self, _label: &str) {}

    fn push_debug_group(&mut self, _group_label: &str) {}

    fn pop_debug_group(&mut self) {}

    fn write_timestamp(&mut self, _query_set: &DispatchQuerySet, _query_index: u32) {
        never!("query set")
    }

    fn begin_pipeline_statistics_query(
        &mut self,
        _query_set: &DispatchQuerySet,
        _query_index: u32,
    ) {
        never!("query set")
    }

    fn end_pipeline_statistics_query(&mut self) {
        never!("query set")
    }

    fn dispatch_workgroups(&mut self, x: u32, y: u32, z: u32) {
        // A kernel that was not made runs nothing: its making has already said why.
        if let Some(pipeline) = &self.pipeline {
            lock(&self.commands).push(Command::Run {
                pipeline: pipeline.clone(),
                groups: self.groups.clone(),
                count: [x, y, z],
            });
        }
    }

    fn dispatch_workgroups_indirect(
        &mut self,
        _indirect_buffer: &DispatchBuffer,
        _indirect_offset: wgpu::BufferAddress,
    ) {
        never!("kernel run by what a buffer says")
    }

    fn transition_resources<'a>(
        &mut self,
        _buffer_transitions: &mut dyn Iterator<
            Item = wgpu::wgt::BufferTransition<&'a DispatchBuffer>,
        >,
        _texture_transitions: &mut dyn Iterator<
            Item = wgpu::wgt::TextureTransition<&'a DispatchTextureView>,
        >,
    ) {
    }
}

impl Drop for ComputePass {
    fn drop(&mut self) {
        // Each run was written down as it was asked for.
    }
}

struct CommandBuffer(Mutex<Vec<Command>>);

impl std::fmt::Debug for CommandBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} commands", lock(&self.0).len())
    }
}

impl CommandBufferInterface for CommandBuffer {}
