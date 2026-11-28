//! The OpenCL device kernels run on: found once, its context and one queue that runs what it is
//! handed in order.

use opencl3::command_queue::CommandQueue;
use opencl3::context::Context;
use opencl3::device::{
    get_all_devices, Device, CL_DEVICE_TYPE_ACCELERATOR, CL_DEVICE_TYPE_CPU, CL_DEVICE_TYPE_GPU,
};
use opencl3::kernel::Kernel;
use opencl3::memory::{Buffer, CL_MEM_READ_WRITE};
use opencl3::program::Program;
use std::collections::HashMap;
use std::sync::Arc;

/// Bytes on the device.
pub(crate) type Memory = Buffer<u8>;

pub(crate) struct Cl {
    pub context: Context,
    pub queue: CommandQueue,
    /// What the device calls itself.
    pub name: String,
    /// It is a CPU that runs OpenCL, as a GPU's driver on a CPU is to wgpu.
    pub on_cpu: bool,
    /// It has doubles, which a float's quotients, roots and powers are worked out through.
    pub exact: bool,
    /// The most bytes one buffer holds.
    pub most_bytes: u64,
    /// The most a kernel's work group holds.
    pub most_in_group: usize,
    /// Programs built, by their source.
    programs: HashMap<String, Arc<Program>>,
}

impl Cl {
    /// The first OpenCL device that is a GPU or an accelerator, or with `software` any, a CPU
    /// last. `None` where there is no OpenCL library, or no such device, or it will not open.
    pub fn open(software: bool) -> Option<Self> {
        let mut found = None;
        let mut kinds = vec![CL_DEVICE_TYPE_GPU, CL_DEVICE_TYPE_ACCELERATOR];
        if software {
            kinds.push(CL_DEVICE_TYPE_CPU);
        }
        for kind in kinds {
            // No library, and no platform, are both an error here: there is nothing to run on.
            let Ok(devices) = get_all_devices(kind) else {
                return None;
            };
            if let Some(id) = devices.first() {
                found = Some((Device::new(*id), kind == CL_DEVICE_TYPE_CPU));
                break;
            }
        }
        let (device, on_cpu) = found?;
        let context = Context::from_device(&device).ok()?;
        let queue = CommandQueue::create_default_with_properties(&context, 0, 0).ok()?;
        let extensions = device.extensions().unwrap_or_default();
        Some(Self {
            name: device.name().unwrap_or_default().trim().to_owned(),
            on_cpu,
            exact: extensions
                .split_whitespace()
                .any(|name| name == "cl_khr_fp64"),
            most_bytes: device.max_mem_alloc_size().unwrap_or(0),
            most_in_group: device.max_work_group_size().unwrap_or(1),
            context,
            queue,
            programs: HashMap::new(),
        })
    }

    /// `source` built, or what its compiler said.
    pub fn program(&mut self, source: &str) -> Result<Arc<Program>, String> {
        if let Some(built) = self.programs.get(source) {
            return Ok(built.clone());
        }
        let built = Arc::new(Program::create_and_build_from_source(
            &self.context,
            source,
            "",
        )?);
        self.programs.insert(source.to_owned(), built.clone());
        Ok(built)
    }

    pub fn kernel(&mut self, source: &str, name: &str) -> Result<Kernel, String> {
        let program = self.program(source)?;
        Kernel::create(&program, name).map_err(|error| error.to_string())
    }

    /// `bytes` of memory, every one of them 0.
    pub fn memory(&self, bytes: usize) -> Result<Memory, String> {
        // OpenCL has no buffer of nothing.
        let bytes = bytes.max(1);
        #[expect(
            unsafe_code,
            reason = "OpenCL's buffer is made through its C interface; no host memory is handed to it"
        )]
        let mut memory = unsafe {
            Buffer::<u8>::create(
                &self.context,
                CL_MEM_READ_WRITE,
                bytes,
                std::ptr::null_mut(),
            )
        }
        .map_err(|error| error.to_string())?;
        #[expect(
            unsafe_code,
            reason = "fills the buffer just made, within its length, from a pattern that lives through the call"
        )]
        unsafe {
            self.queue
                .enqueue_fill_buffer(&mut memory, &[0u8], 0, bytes, &[])
        }
        .map_err(|error| error.to_string())?;
        Ok(memory)
    }
}
