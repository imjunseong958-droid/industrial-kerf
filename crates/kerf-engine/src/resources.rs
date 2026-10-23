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

pub fn extract_job_count() -> usize {
    budget().extract_parallelism
}

pub fn should_try_hardware_encode(prefer_hardware: bool, force_software: bool) -> bool {
    if force_software {
        return false;
    }
    if prefer_hardware {
        return true;
    }
    match budget().hardware_encode {
        HardwareEncodePolicy::Never => false,
        HardwareEncodePolicy::Auto | HardwareEncodePolicy::Always => true,
    }
}
