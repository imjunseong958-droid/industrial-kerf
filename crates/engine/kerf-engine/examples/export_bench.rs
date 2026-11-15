//! Where the time of an export goes: drawing, reading kept frames, and
//! encoding.
//!
//!     export_bench render <project.kerf> <out> [standard|review|prores_hq] [edit] [silent]
//!     export_bench draw <project.kerf>
//!
//! `render` is an export as `kerf-engine render` makes one, of the document
//! after `edit` when one is given (`ripple-delete c3`, `set c30 opacity=0.5`),
//! and without its sound when `silent` follows. `draw` draws
//! every frame the same export would, on as many threads, and encodes
//! nothing, so the difference between the two is what encoding costs. Each
//! prints how long it took and what the folder of kept frames did
//! (`KERF_FRAME_CACHE`, `KERF_FRAME_CACHE_GB`): frames read back, frames
//! drawn, written, dropped unwritten, and put out to make room; and what a
//! render did with its encoded stretches (`KERF_SECTION_CACHE`): how many
//! it joined as they were kept and how many it encoded, whether it gave up
//! on them and encoded the picture whole, and what it kept and made of the
//! sound: clips read decoded or decoded, pieces of encoded sound used or
//! encoded, and whether one encoder made it all.

use kerf_compose::{default_frame_folder, frame_cache_stats, ComposeOptions, Compositor};
use kerf_engine::render::{render_program, RenderOptions, RenderPreset};
use kerf_model::{Catalog, Project};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// As many as a render draws at once.
const DRAWERS: usize = 3;

fn project(path: &Path) -> Project {
    let text = std::fs::read_to_string(path).expect("the document reads");
    let mut project = kerf_lang::parse_document(&text, Catalog::builtin())
        .expect("the document parses")
        .project;
    project.resolve_paths(Catalog::builtin(), path.parent().unwrap_or(Path::new(".")));
    project
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, document) = (args[0].as_str(), Path::new(&args[1]));
    let mut project = project(document);
    if let Some(edit) = args.get(4).filter(|edit| !edit.is_empty()) {
        project = kerf_lang::apply_script(&project, Catalog::builtin(), edit)
            .expect("the edit applies")
            .project;
    }
    let sequence = &project.sequences[0];
    let frames = sequence.rate.frames_floor(sequence.duration()) as usize;
    let started = Instant::now();
    match mode {
        "render" => {
            let preset = args.get(3).map_or(RenderPreset::Standard, |name| {
                RenderPreset::parse(name).expect("a preset")
            });
            render_program::<fn(f64)>(
                &project,
                None,
                None,
                &RenderOptions {
                    output: args[2].clone(),
                    preset,
                    sound: args.get(5).is_none_or(|word| word != "silent"),
                    project_dir: document.parent().map(Path::to_path_buf),
                    ..RenderOptions::default()
                },
                None,
            )
            .expect("renders");
        }
        "draw" => {
            let compositor = Compositor::new(
                &project,
                Catalog::builtin(),
                &ComposeOptions {
                    captions: true,
                    frame_folder: default_frame_folder(),
                    ..Default::default()
                },
            )
            .expect("composes");
            let next = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for _ in 0..DRAWERS {
                    scope.spawn(|| loop {
                        let frame = next.fetch_add(1, Ordering::Relaxed);
                        if frame >= frames {
                            break;
                        }
                        let drawn = compositor.frame(sequence.rate.time_of(frame as i64), 1.0);
                        std::hint::black_box(drawn.rgba.len());
                    });
                }
            });
            compositor.flush_frames();
        }
        other => panic!("{other}? use render or draw"),
    }
    let seconds = started.elapsed().as_secs_f64();
    let stats = default_frame_folder()
        .map(|folder| frame_cache_stats(&folder))
        .unwrap_or_default();
    let sections = kerf_engine::sections::section_stats();
    println!(
        "{{\"mode\":\"{mode}\",\"frames\":{frames},\"seconds\":{seconds:.2},\"fps\":{:.1},\
         \"read\":{},\"drawn\":{},\"written\":{},\"dropped\":{},\"evicted\":{},\
         \"stretchesJoined\":{},\"framesJoined\":{},\"stretchesEncoded\":{},\
         \"framesEncoded\":{},\"soundKept\":{},\"soundMixed\":{},\
         \"soundClipsKept\":{},\"soundClipsDecoded\":{},\"soundPiecesKept\":{},\
         \"soundPiecesEncoded\":{},\"soundWhole\":{},\"soundUnjoined\":{},\
         \"encodedWhole\":{}}}",
        frames as f64 / seconds,
        stats.hits,
        stats.misses,
        stats.writes,
        stats.dropped,
        stats.evicted,
        sections.kept,
        sections.kept_frames,
        sections.encoded,
        sections.encoded_frames,
        sections.sound_kept,
        sections.sound_mixed,
        sections.sound_clips_kept,
        sections.sound_clips_decoded,
        sections.sound_pieces_kept,
        sections.sound_pieces_encoded,
        sections.sound_whole,
        sections.sound_unjoined,
        sections.whole,
    );
}
