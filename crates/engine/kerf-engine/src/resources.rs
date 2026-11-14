//! Laptop-aware CPU budget shared by ffmpeg, probing, and encoding.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareEncodePolicy {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceBudget {
    pub logical_cpus: usize,
    pub ffmpeg_threads: usize,
    /// Threads a transcription runs on. whisper.cpp takes four whatever the
    /// machine has, which is all of a four-core laptop for minutes.
    pub whisper_threads: usize,
    pub extract_parallelism: usize,
    pub software_preset: &'static str,
    pub software_crf: u8,
    pub hardware_encode: HardwareEncodePolicy,
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()?
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
}

fn env_preset() -> Option<&'static str> {
    let raw = std::env::var("KERF_PRESET").ok()?;
    match raw.trim() {
        "ultrafast" => Some("ultrafast"),
        "superfast" => Some("superfast"),
        "veryfast" => Some("veryfast"),
        "faster" => Some("faster"),
        "fast" => Some("fast"),
        "medium" => Some("medium"),
        "slow" => Some("slow"),
        _ => None,
    }
}

pub fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(2)
        .max(1)
}

/// Budget derived from an explicit CPU count (tests and `KERF_CPUS`).
pub fn budget_for_cpus(cpus: usize) -> ResourceBudget {
    let cpus = cpus.max(1);
    let ffmpeg_threads = env_usize("KERF_FFMPEG_THREADS").unwrap_or(match cpus {
        1 => 1,
        2 => 1,
        3..=4 => 2,
        5..=8 => 4,
        _ => 8.min(cpus.saturating_sub(1)),
    });
    // Half the machine, and no more than whisper.cpp's own four. Measured
    // on four cores with the small model: two threads transcribe at 1.2
    // times real time on half the CPU, a proxy made beside them takes 23%
    // longer and an index 12%, and a composited program keeps a picture 750
    // to 940 wide; four threads transcribe at 2.2 times real time on all of
    // it, a proxy takes 42% longer, an index 64%, and the picture is drawn
    // 600 to 750 wide.
    let whisper_threads = env_usize("KERF_WHISPER_THREADS").unwrap_or((cpus / 2).clamp(1, 4));
    let extract_parallelism = env_usize("KERF_EXTRACT_JOBS").unwrap_or(match cpus {
        1..=2 => 1,
        3..=4 => 2,
        5..=8 => 3,
        _ => 4,
    });
    let software_preset = env_preset().unwrap_or(match cpus {
        1..=4 => "veryfast",
        5..=8 => "faster",
        _ => "medium",
    });
    let software_crf = env_usize("KERF_CRF")
        .map(|value| value.min(51) as u8)
        .unwrap_or(match cpus {
            1..=4 => 23,
            5..=8 => 22,
            _ => 20,
        });
    let hardware_encode = match std::env::var("KERF_ENCODER") {
        Ok(value) if value.trim().eq_ignore_ascii_case("software") => HardwareEncodePolicy::Never,
        Ok(value) if value.trim().eq_ignore_ascii_case("hardware") => HardwareEncodePolicy::Always,
        _ => HardwareEncodePolicy::Auto,
    };
    ResourceBudget {
        logical_cpus: cpus,
        ffmpeg_threads: ffmpeg_threads.max(1),
        whisper_threads: whisper_threads.max(1),
        extract_parallelism: extract_parallelism.max(1),
        software_preset,
        software_crf,
        hardware_encode,
    }
}

fn compute_budget() -> ResourceBudget {
    budget_for_cpus(env_usize("KERF_CPUS").unwrap_or_else(logical_cpus))
}

/// Process-wide budget. Env overrides are read once on first use.
pub fn budget() -> &'static ResourceBudget {
    static BUDGET: OnceLock<ResourceBudget> = OnceLock::new();
    BUDGET.get_or_init(compute_budget)
}

pub fn ffmpeg_thread_count() -> usize {
    budget().ffmpeg_threads
}

pub fn whisper_thread_count() -> usize {
    budget().whisper_threads
}

pub fn extract_job_count() -> usize {
    budget().extract_parallelism
}

/// Whether an encode tries the machine's own encoder. Asked for (`prefer_hardware`, or
/// `KERF_ENCODER=hardware`) it does; left to Kerf it does, but not on a Mac in a virtual machine,
/// whose VideoToolbox can wait for ever as its decoding does (see
/// [`crate::ffmpeg::runs_in_a_guest`]).
pub fn should_try_hardware_encode(prefer_hardware: bool, force_software: bool) -> bool {
    tries_hardware(
        budget().hardware_encode,
        prefer_hardware,
        force_software,
        crate::ffmpeg::runs_in_a_guest,
    )
}

fn tries_hardware(
    policy: HardwareEncodePolicy,
    prefer_hardware: bool,
    force_software: bool,
    in_a_guest: impl Fn() -> bool,
) -> bool {
    if force_software {
        return false;
    }
    if prefer_hardware {
        return true;
    }
    match policy {
        HardwareEncodePolicy::Never => false,
        HardwareEncodePolicy::Always => true,
        HardwareEncodePolicy::Auto => !in_a_guest(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guest_encodes_in_software_unless_hardware_is_asked_for() {
        use HardwareEncodePolicy::{Always, Auto, Never};
        let tries =
            |policy, prefer, force, guest: bool| tries_hardware(policy, prefer, force, || guest);
        assert!(tries(Auto, false, false, false));
        assert!(!tries(Auto, false, false, true));
        assert!(tries(Always, false, false, true));
        assert!(tries(Auto, true, false, true));
        assert!(!tries(Never, false, false, false));
        assert!(!tries(Always, true, true, false));
    }
}
