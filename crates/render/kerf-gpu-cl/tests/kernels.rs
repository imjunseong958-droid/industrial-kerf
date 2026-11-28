//! A kernel through this crate, as kerf-gpu sends one through wgpu, against the same arithmetic
//! done here. It needs an OpenCL device, and where there is none there is nothing to run. That
//! each of Kerf's kernels can be written out as OpenCL C, which needs no device, is held by the
//! crates whose kernels they are (`kerf_gpu::written_for_opencl`).

use std::future::Future;
use std::task::{Context, Poll, Waker};

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
/// picture: a quotient, a power, a smooth step, a loop and a switch, a number of a vector picked
/// when run, a load from outside the texture and a store outside the picture.
const KERNEL: &str = "
struct U {
    scale: vec4<f32>,
    turns: u32,
    mode: u32,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
// Another kernel's, were there one: this one's bind group does not hold it.
@group(0) @binding(7) var never: texture_2d<f32>;

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
    // The number of `picked` that `mode` names is set, and read back the same way.
    var picked = vec3(0.0, 0.0, 0.0);
    picked[u.mode] = last + smoothstep(0.5, 4.5, v);
    let third = vec3(picked.x, picked.y, picked.z)[u.mode];
    // Outside the picture nothing is stored.
    textureStore(dst, p, vec4(raised(v), sum, v * u.scale.y, third));
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
            let t = ((v - 0.5) / 4.0).clamp(0.0, 1.0);
            let third = 0.5 + t * t * (3.0 - 2.0 * t);
            let wanted = [v.max(0.0).powf(2.4) / 3.0, sum, v * scale[1], third];
            for (got, wanted) in got.iter().zip(wanted) {
                assert!(
                    (got - wanted).abs() <= wanted.abs() * 1e-6,
                    "at {x}, {y}: {got} for {wanted}"
                );
            }
        }
    }
}

/// A kernel of buffers: numbers in an array whose length its buffer says, a matrix's columns, and
/// the most of them found in step by every run of the kernel at once.
const BUFFERS: &str = "
struct Scale {
    by: vec2<f32>,
    count: u32,
};

@group(0) @binding(0) var<uniform> scale: Scale;
@group(0) @binding(1) var<storage, read> numbers: array<f32>;
@group(0) @binding(2) var<storage, read_write> doubled: array<f32>;
@group(0) @binding(3) var<storage, read_write> most: array<atomic<u32>>;

fn sides(by: vec2<f32>) -> mat2x3<f32> {
    return mat2x3(vec3(by.x, 0.0, 1.0), vec3(0.0, by.y, 2.0));
}

@compute @workgroup_size(64)
fn run(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= scale.count { return; }
    let two = sides(scale.by);
    // Past the end of `numbers` it reads the last of them.
    let v = numbers[id.x + 1u];
    doubled[id.x] = v * two[0].x + two[1].z + f32(arrayLength(&numbers));
    atomicMax(&most[0], u32(v));
    atomicMin(&most[1], u32(v));
}
";

#[test]
fn a_kernel_of_buffers_runs_on_opencl_as_its_arithmetic_says() {
    let Some((device, queue)) = kerf_gpu_cl::open(false) else {
        eprintln!("no OpenCL device: nothing to run");
        return;
    };
    let numbers: Vec<f32> = (0..100).map(|i| ((i * 37) % 101) as f32 + 0.25).collect();
    let floats =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|n| n.to_le_bytes()).collect() };
    let buffer = |bytes: &[u8], usage| {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes.len() as u64,
            usage,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytes);
        buffer
    };
    use wgpu::BufferUsages as Usage;
    let mut uniforms = floats(&[3.0, 5.0]);
    uniforms.extend_from_slice(&(numbers.len() as u32).to_le_bytes());
    uniforms.resize(16, 0);
    let scale = buffer(&uniforms, Usage::UNIFORM | Usage::COPY_DST);
    let read = buffer(&floats(&numbers), Usage::STORAGE | Usage::COPY_DST);
    let doubled = buffer(
        &vec![0u8; numbers.len() * 4],
        Usage::STORAGE | Usage::COPY_SRC | Usage::COPY_DST,
    );
    let mut found = 0u32.to_le_bytes().to_vec();
    found.extend_from_slice(&u32::MAX.to_le_bytes());
    let most = buffer(&found, Usage::STORAGE | Usage::COPY_SRC | Usage::COPY_DST);

    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(BUFFERS.into()),
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
    let entries: Vec<wgpu::BindGroupEntry<'_>> = [&scale, &read, &doubled, &most]
        .into_iter()
        .enumerate()
        .map(|(binding, buffer)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
    });
    let back = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (numbers.len() * 4 + 8) as u64,
        usage: Usage::COPY_DST | Usage::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        // Two groups of 64 for a hundred numbers.
        pass.dispatch_workgroups(2, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&doubled, 0, &back, 0, Some((numbers.len() * 4) as u64));
    encoder.copy_buffer_to_buffer(&most, 0, &back, (numbers.len() * 4) as u64, Some(8));
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
    let words: Vec<[u8; 4]> = data.as_chunks::<4>().0.to_vec();
    let read_at = |i: usize| numbers[(i + 1).min(numbers.len() - 1)];
    for (i, word) in words.iter().take(numbers.len()).enumerate() {
        let wanted = read_at(i) * 3.0 + 2.0 + numbers.len() as f32;
        assert_eq!(f32::from_le_bytes(*word), wanted, "at {i}");
    }
    let seen: Vec<u32> = (0..numbers.len()).map(|i| read_at(i) as u32).collect();
    assert_eq!(
        u32::from_le_bytes(words[numbers.len()]),
        *seen.iter().max().expect("some"),
        "the most"
    );
    assert_eq!(
        u32::from_le_bytes(words[numbers.len() + 1]),
        *seen.iter().min().expect("some"),
        "the least"
    );
}

#[test]
fn a_submission_runs_with_none_of_another_between_its_commands() {
    let Some((device, queue)) = kerf_gpu_cl::open(false) else {
        eprintln!("no OpenCL device: nothing to run");
        return;
    };
    use wgpu::BufferUsages as Usage;
    const BYTES: u64 = 1024;
    const TIMES: u64 = 16;
    let buffer = |size, usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    // Every thread's submissions write here on their way, so it holds what the last one wrote.
    let shared = buffer(BYTES, Usage::COPY_SRC | Usage::COPY_DST);
    std::thread::scope(|threads| {
        for thread in 1..=8u8 {
            let (device, queue, shared, buffer) = (&device, &queue, &shared, &buffer);
            threads.spawn(move || {
                let own = buffer(BYTES, Usage::COPY_SRC | Usage::COPY_DST);
                queue.write_buffer(&own, 0, &[thread; BYTES as usize]);
                for round in 0..50 {
                    let back = buffer(BYTES * TIMES, Usage::COPY_DST | Usage::MAP_READ);
                    let mut encoder = device.create_command_encoder(&Default::default());
                    for time in 0..TIMES {
                        encoder.copy_buffer_to_buffer(&own, 0, shared, 0, Some(BYTES));
                        encoder.copy_buffer_to_buffer(shared, 0, &back, time * BYTES, Some(BYTES));
                    }
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
                    let others = data.iter().filter(|byte| **byte != thread).count();
                    assert_eq!(
                        others, 0,
                        "thread {thread}, round {round}: bytes another wrote"
                    );
                }
            });
        }
    });
}
