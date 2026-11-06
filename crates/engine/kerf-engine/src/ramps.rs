//! The sound of clips that change speed, for what reads a program's sound through its graph.

use crate::ffmpeg::{hidden_command, resolve_ffmpeg_bin};
use crate::program::{compile_program, ProgramOptions, SAMPLE_RATE};
use kerf_compose::ramps::Read;
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Catalog, Id, Project, Time};
use std::borrow::Cow;
use std::sync::Arc;

/// `project` with what ramps on its sound tracks made into sound of its own. Nested sequences are
/// mixed by FFmpeg first; one inside another that ramps is heard at its speed overall.
pub fn with_sound(project: &Project) -> Cow<'_, Project> {
    kerf_compose::ramps::with_sound(project, |sequence, from, to| {
        mixed(project, sequence, from, to)
    })
}

/// `[from, to)` of a nested sequence's mix, as stereo floats, and the frame they start at.
fn mixed(project: &Project, sequence: &Id, from: Time, to: Time) -> Option<Read> {
    let length = project.sequence(sequence.as_str())?.duration();
    let (low, high) = (from.max(Time::ZERO), to.min(length));
    if low >= high {
        return None;
    }
    let compiled = compile_program(
        project,
        Catalog::builtin(),
        &ProgramOptions {
            sequence: Some(sequence.to_string()),
            range: Some((low, high)),
            audio_only: true,
            ..ProgramOptions::default()
        },
    )
    .ok()?;
    let output = hidden_command(resolve_ffmpeg_bin())
        .args(compiled.argv.iter().skip(1))
        .args(["-f", "f32le", "-c:a", "pcm_f32le", "-"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let samples: Vec<f32> = output
        .stdout
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect();
    let first = low.flicks() / (FLICKS_PER_SECOND / i64::from(SAMPLE_RATE));
    Some((Arc::new(samples), first))
}
