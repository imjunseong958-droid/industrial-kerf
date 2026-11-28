//! Kerf's kernels through this crate. Writing one out as OpenCL C needs no device, so that is
//! checked everywhere. Running one needs an OpenCL device, and where there is one a kernel goes
//! through wgpu as kerf-gpu sends it, against the same arithmetic done here.

use kerf_gpu_cl::translate::translate;
use std::future::Future;
use std::task::{Context, Poll, Waker};

/// The kernels kerf-gpu draws every frame with.
const KERNELS: [(&str, &str); 7] = [
    (
        "picture.wgsl",
        include_str!("../../kerf-gpu/src/picture.wgsl"),
    ),
    ("input.wgsl", include_str!("../../kerf-gpu/src/input.wgsl")),
    (
        "finish.wgsl",
        include_str!("../../kerf-gpu/src/finish.wgsl"),
    ),
    (
        "finish16.wgsl",
        include_str!("../../kerf-gpu/src/finish16.wgsl"),
    ),
    (
        "finish_signal.wgsl",
        include_str!("../../kerf-gpu/src/finish_signal.wgsl"),
    ),
    (
        "planes.wgsl",
        include_str!("../../kerf-gpu/src/planes.wgsl"),
    ),
    (
        "output.wgsl",
        include_str!("../../kerf-gpu/src/output.wgsl"),
    ),
];

#[test]
fn every_kernel_kerf_gpu_draws_with_is_written_out() {
    for (name, source) in KERNELS {
        let (module, info) =
            kerf_gpu_cl::parse(source).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(!module.entry_points.is_empty(), "{name} has no kernel");
        for point in &module.entry_points {
            for exact in [false, true] {
                let written = translate(&module, &info, &point.name, exact)
                    .unwrap_or_else(|error| panic!("{name}, `{}`: {error}", point.name));
                assert_eq!(written.workgroup, point.workgroup_size, "{name}");
                assert!(
                    written
                        .source
                        .contains(&format!("__kernel void {}(", written.kernel)),
                    "{name}, `{}` has no kernel in what was written",
                    point.name
                );
            }
        }
    }
}

fn now<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return out;
        }
    }
}

/// A kernel of this test's own, which reads a one-number texture and a uniform and writes a
/// picture: a quotient, a power, a loop and a switch, a load from outside the texture and a store
/// outside the picture.
const KERNEL: &str = "
struct U {
    scale: vec4<f32>,
    turns: u32,
    mode: u32,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;

fn raised(v: f32) -> f32 {
    return pow(max(v, 0.0), 2.4) / 3.0;
}

@compute @workgroup_size(16, 16)
fn run(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    // Past the right edge it reads the last texel of its row.
    let v = textureLoad(src, p + vec2(1, 0), 0).r;
    var sum = 0.0;
    for (var i = 0u; i < u.turns; i++) {
        sum += f32(i) * v;
    }
    var last = 0.0;
    switch u.mode {
        case 1u: { last = 1.0; }
        case 2u, 3u: { last = 0.5; }
        default: { last = 0.25; }
    }
    // Outside the picture nothing is stored.
    textureStore(dst, p, vec4(raised(v), sum, v * u.scale.y, last));
}
";

#[test]
fn a_kernel_runs_on_opencl_as_its_arithmetic_says() {
    let Some((device, queue)) = kerf_gpu_cl::open(false) else {
        eprintln!("no OpenCL device: nothing to run");
        return;
    };
    let (w, h) = (5u32, 3u32);
    let numbers: Vec<f32> = (0..w * h).map(|i| 0.1 + i as f32 * 0.37).collect();
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    use wgpu::TextureUsages as Usage;
    let src = texture(
        wgpu::TextureFormat::R32Float,
        Usage::TEXTURE_BINDING | Usage::COPY_DST,
    );
    let dst = texture(
        wgpu::TextureFormat::Rgba32Float,
        Usage::STORAGE_BINDING | Usage::COPY_SRC,
    );
    let bytes: Vec<u8> = numbers.iter().flat_map(|n| n.to_le_bytes()).collect();
    queue.write_texture(
        src.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );

    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(KERNEL.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("run"),
        compilation_options: Default::default(),
        cache: None,
    });
    if let Some(error) = now(scope.pop()) {
        panic!("the kernel was not made: {error}");
    }

    let (scale, turns, mode) = ([0.0f32, 1.5, 0.0, 0.0], 4u32, 2u32);
    let mut uniforms = Vec::new();
    for value in scale {
        uniforms.extend_from_slice(&value.to_le_bytes());
    }
    uniforms.extend_from_slice(&turns.to_le_bytes());
    uniforms.extend_from_slice(&mode.to_le_bytes());
    // A struct of a `vec4` is a multiple of 16 bytes long.
    uniforms.resize(32, 0);
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: uniforms.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform, 0, &uniforms);
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&src.create_view(&Default::default())),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&dst.create_view(&Default::default())),
            },
        ],
    });
    // Rows come back as far apart as wgpu has a texture's rows in a buffer.
    let row = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let back = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        // One group of 16 by 16 covers the picture and a good way past it.
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_texture_to_buffer(
        dst.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &back,
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
    queue.submit([encoder.finish()]);
    let slice = back.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the queue ran");
    receiver
        .recv()
        .expect("the map was answered")
        .expect("the buffer came back");
    let data = slice.get_mapped_range().expect("its bytes");
    for y in 0..h {
        for x in 0..w {
            let at = (y * row + x * 16) as usize;
            let got: Vec<f32> = data[at..at + 16]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|bytes| f32::from_le_bytes(*bytes))
                .collect();
            let v = numbers[(y * w + (x + 1).min(w - 1)) as usize];
            let sum: f32 = (0..turns).fold(0.0, |sum, i| sum + i as f32 * v);
            let wanted = [v.max(0.0).powf(2.4) / 3.0, sum, v * scale[1], 0.5];
            for (got, wanted) in got.iter().zip(wanted) {
                assert!(
                    (got - wanted).abs() <= wanted.abs() * 1e-6,
                    "at {x}, {y}: {got} for {wanted}"
                );
            }
        }
    }
}
