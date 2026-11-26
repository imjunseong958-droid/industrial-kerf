//! The sound of clips that change speed, for what reads a program's sound through its graph.

use crate::process::ffmpeg::{hidden_command, resolve_ffmpeg_bin};
use crate::process::sidecar::CancellationToken;
use crate::program::{compile_program, ProgramOptions, SAMPLE_RATE};
use kerf_compose::ramps::Read;
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Catalog, Id, Project, Time};
use std::borrow::Cow;
use std::sync::Arc;

/// `project` with what ramps on its sound tracks made into sound of its own. Nested sequences are
/// mixed by FFmpeg first; one inside another that ramps is heard at its speed overall. Once
/// `cancellation` is asked for, a nested mix under way is stopped and no other is made, so their
/// sound is left out: what reads the sound must stop on the token too.
pub fn with_sound<'a>(project: &'a Project, cancellation: &CancellationToken) -> Cow<'a, Project> {
    kerf_compose::ramps::with_sound(project, |sequence, from, to| {
        mixed(project, sequence, from, to, cancellation)
    })
}

/// `[from, to)` of a nested sequence's mix, as stereo floats, and the frame they start at.
fn mixed(
    project: &Project,
    sequence: &Id,
    from: Time,
    to: Time,
    cancellation: &CancellationToken,
) -> Option<Read> {
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
        },
    )
    .ok()?;
    // A sidecar, stopped with the work: it mixes as much of the sequence's sound as the range holds.
    let output = crate::process::sidecar::output_cancellable(
        hidden_command(resolve_ffmpeg_bin())
            .args(compiled.argv.iter().skip(1))
            .args(["-f", "f32le", "-c:a", "pcm_f32le", "-"]),
        cancellation,
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let samples: Vec<f32> = output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .collect();
    let first = low.flicks() / (FLICKS_PER_SECOND / i64::from(SAMPLE_RATE));
    Some((Arc::new(samples), first))
}
