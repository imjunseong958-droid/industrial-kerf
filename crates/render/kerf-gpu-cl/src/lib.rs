//! Kerf's GPU kernels on OpenCL.
//!
//! kerf-gpu draws through wgpu, which reaches a GPU by its graphics queue. An accelerator that only
//! computes has none: an AMD Instinct, whose Vulkan driver turns it down for that. What such a
//! card runs is OpenCL. This crate is a device for wgpu that runs on it: kerf-gpu and every kernel
//! written against wgpu stay as they are, and each kernel's WGSL is written out as OpenCL C
//! ([`translate`]) when its pipeline is made.
//!
//! The system's OpenCL library is found when first asked for. Where there is none, nothing here
//! does anything.

mod backend;
mod cl;
pub mod format;
pub mod translate;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

/// A device for wgpu on this machine's OpenCL, and its queue: the first GPU or accelerator
/// OpenCL has, or with `software` a CPU that runs OpenCL where there is neither, which is only
/// for checking kernels. `None` where there is no OpenCL to run on.
pub fn open(software: bool) -> Option<(wgpu::Device, wgpu::Queue)> {
    let cl = cl::Cl::open(software)?;
    let adapter = wgpu::Adapter::from_custom(backend::Adapter::new(cl));
    now(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// Every kernel of a WGSL module written out, to see that each can be: how many there are, or
/// the first that cannot be and why. One that cannot runs nowhere OpenCL does the drawing.
pub fn write_out_all(source: &str) -> Result<usize, String> {
    let (module, info) = parse(source).map_err(|error| error.0)?;
    for point in &module.entry_points {
        for exact in [false, true] {
            translate::translate(&module, &info, &point.name, exact)
                .map_err(|error| format!("`{}`: {error}", point.name))?;
        }
    }
    Ok(module.entry_points.len())
}

/// What `future` comes to. Nothing here waits on anything else, so it is there when asked.
fn now<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return out;
        }
        std::thread::yield_now();
    }
}

/// A WGSL module read and checked, ready for [`translate::translate`].
pub fn parse(
    source: &str,
) -> Result<(naga::Module, naga::valid::ModuleInfo), translate::Unsupported> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| translate::Unsupported(error.emit_to_string(source)))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|error| translate::Unsupported(error.emit_to_string(source)))?;
    Ok((module, info))
}
