//! What each step of getting a decoded frame to the GPU costs, to tell what skipping one would save.
//!
//! ```text
//! ffmpeg -f lavfi -i testsrc2=s=3840x2160:r=24:d=4 -pix_fmt yuv420p clip.mp4
//! cargo run --release -p kerf-compose --example upload_bench -- clip.mp4 [more.mov …]
//! ```
//!
//! `decode` is left out of the table: frame threads read ahead, so a frame's turn says nothing of the
//! work. For that, and for what VideoToolbox would take instead, time the whole file:
//!
//! ```text
//! ffmpeg -i clip.mp4 -f null -
//! ffmpeg -hwaccel videotoolbox -hwaccel_output_format videotoolbox_vld -i clip.mp4 -f null -
//! ```

use ffmpeg_next as ffmpeg;
use std::time::Instant;

fn median(times: &mut [f64]) -> f64 {
    times.sort_by(f64::total_cmp);
    times.get(times.len() / 2).copied().unwrap_or(0.0)
}

fn main() {
    ffmpeg::init().expect("FFmpeg starts");
    ffmpeg::log::set_level(ffmpeg::log::Level::Fatal);
    let gpu = kerf_gpu::shared().expect("a GPU to upload to");
    println!(
        "{:<24}{:>10}{:>10}{:>13}{:>12}",
        "clip", "to RGBA", "upload", "kernel, back", "whole frame"
    );
    for path in std::env::args().skip(1) {
        let mut input = ffmpeg::format::input(&path).expect("a clip to read");
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .expect("a picture in it");
        let index = stream.index();
        let mut context =
            ffmpeg::codec::context::Context::from_parameters(stream.parameters()).expect("opens");
        // As `footage.rs` decodes.
        context.set_threading(ffmpeg::codec::threading::Config::kind(
            ffmpeg::codec::threading::Type::Frame,
        ));
        let mut decoder = context.decoder().video().expect("a decoder");
        let (w, h) = (decoder.width(), decoder.height());
        let name = decoder
            .format()
            .descriptor()
            .map(|descriptor| descriptor.name().to_owned())
            .expect("a pixel format");
        let deep = name.contains("10") || name.contains("12");
        let mut graph = ffmpeg::filter::Graph::new();
        graph
            .add(
                &ffmpeg::filter::find("buffer").expect("the buffer filter"),
                "in",
                &format!("video_size={w}x{h}:pix_fmt={name}:time_base=1/1:pixel_aspect=1/1"),
            )
            .expect("a source");
        graph
            .add(
                &ffmpeg::filter::find("buffersink").expect("the sink filter"),
                "out",
                "",
            )
            .expect("a sink");
        graph
            .output("in", 0)
            .and_then(|parser| parser.input("out", 0))
            .and_then(|parser| {
                parser.parse(if deep {
                    "format=rgba64le"
                } else {
                    "format=rgba"
                })
            })
            .expect("a graph");
        graph.validate().expect("a graph that runs");
        let packets: Vec<_> = input
            .packets()
            .filter(|(stream, _)| stream.index() == index)
            .map(|(_, packet)| packet)
            .collect();
        let (mut converts, mut uploads, mut kernels) = (Vec::new(), Vec::new(), Vec::new());
        let mut frame = ffmpeg::frame::Video::empty();
        let begun = Instant::now();
        let mut count = 0u32;
        for packet in packets.iter().map(Some).chain([None]) {
            match packet {
                Some(packet) => decoder.send_packet(packet).expect("a packet goes in"),
                None => decoder.send_eof().expect("the end goes in"),
            }
            while decoder.receive_frame(&mut frame).is_ok() {
                let at = Instant::now();
                graph
                    .get("in")
                    .expect("the source")
                    .source()
                    .add(&frame)
                    .expect("a frame goes in");
                let mut rgba = ffmpeg::frame::Video::empty();
                graph
                    .get("out")
                    .expect("the sink")
                    .sink()
                    .frame(&mut rgba)
                    .expect("a frame comes out");
                converts.push(at.elapsed().as_secs_f64() * 1000.0);
                let mut on_gpu = gpu.frame();
                let at = Instant::now();
                // A deep frame as bytes alone: what takes it into light is the same however it came.
                let width = if deep { w * 2 } else { w };
                let picture = on_gpu.rgba8(rgba.data(0), width, h, rgba.stride(0) as u32, false);
                uploads.push(at.elapsed().as_secs_f64() * 1000.0);
                let at = Instant::now();
                std::hint::black_box(on_gpu.finish(&picture));
                kernels.push(at.elapsed().as_secs_f64() * 1000.0);
                count += 1;
            }
        }
        let whole = begun.elapsed().as_secs_f64() * 1000.0 / f64::from(count.max(1));
        println!(
            "{path:<24}{:>10.2}{:>10.2}{:>13.2}{:>12.2}",
            median(&mut converts),
            median(&mut uploads),
            median(&mut kernels),
            whole,
        );
    }
}
