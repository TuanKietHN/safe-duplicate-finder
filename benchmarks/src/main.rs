//! Measure bounded enumeration, dual full hashing, and process peak working set.

use std::{path::PathBuf, time::Instant};

use clap::Parser;
use dedupe_core::{control::ControlToken, full_hash};
use dedupe_platform::PlatformFileSystem;
use serde::Serialize;
use walkdir::WalkDir;

#[derive(Debug, Parser)]
#[command(about = "Chạy benchmark chỉ đọc có thể tái lập và xuất JSON")]
struct Args {
    /// Source roots to enumerate without following links.
    #[arg(long, required = true)]
    root: Vec<PathBuf>,
    /// Maximum number of files to dual-hash; zero measures enumeration only.
    #[arg(long, default_value_t = 100)]
    hash_limit: usize,
}

#[derive(Debug, Serialize)]
struct ResultRecord {
    schema_version: u32,
    roots: Vec<PathBuf>,
    files: u64,
    logical_bytes: u64,
    enumeration_seconds: f64,
    hashed_files: u64,
    hash_bytes_read: u64,
    hashing_seconds: f64,
    hash_mib_per_second: f64,
    errors: u64,
    peak_working_set_bytes: Option<u64>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let enumeration_started = Instant::now();
    let mut files = 0_u64;
    let mut logical_bytes = 0_u64;
    let mut errors = 0_u64;
    let mut selected = Vec::with_capacity(args.hash_limit);
    for root in &args.root {
        for item in WalkDir::new(root).follow_links(false) {
            match item {
                Ok(entry) if entry.file_type().is_file() => match entry.metadata() {
                    Ok(metadata) => {
                        files = files.saturating_add(1);
                        logical_bytes = logical_bytes.saturating_add(metadata.len());
                        if selected.len() < args.hash_limit {
                            selected.push(entry.into_path());
                        }
                    }
                    Err(_) => errors = errors.saturating_add(1),
                },
                Ok(_) => {}
                Err(_) => errors = errors.saturating_add(1),
            }
        }
    }
    let enumeration_seconds = enumeration_started.elapsed().as_secs_f64();
    let hashing_started = Instant::now();
    let provider = PlatformFileSystem;
    let control = ControlToken::new();
    let mut hashed_files = 0_u64;
    let mut hash_bytes_read = 0_u64;
    for path in selected {
        match (
            full_hash::blake3_file(&path, &provider, &control),
            full_hash::sha256_file(&path, &provider, &control),
        ) {
            (Ok(blake3), Ok(sha256)) if blake3.stable && sha256.stable => {
                hashed_files = hashed_files.saturating_add(1);
                hash_bytes_read = hash_bytes_read
                    .saturating_add(blake3.bytes_read)
                    .saturating_add(sha256.bytes_read);
            }
            _ => errors = errors.saturating_add(1),
        }
    }
    let hashing_seconds = hashing_started.elapsed().as_secs_f64();
    let hash_mib_per_second = mib_per_second(hash_bytes_read, hashing_seconds);
    let record = ResultRecord {
        schema_version: 1,
        roots: args.root,
        files,
        logical_bytes,
        enumeration_seconds,
        hashed_files,
        hash_bytes_read,
        hashing_seconds,
        hash_mib_per_second,
        errors,
        peak_working_set_bytes: peak_working_set_bytes(),
    };
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}

#[cfg(windows)]
fn peak_working_set_bytes() -> Option<u64> {
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };

    // SAFETY: the current-process pseudo handle is always valid for this query, the structure is
    // initialized and its exact size is passed to the Windows API.
    unsafe {
        let mut counters = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();
        counters.cb = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
        let ok = GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, counters.cb);
        if ok != 0 {
            u64::try_from(counters.PeakWorkingSetSize).ok()
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn peak_working_set_bytes() -> Option<u64> {
    None
}

#[allow(clippy::cast_precision_loss)]
fn mib_per_second(bytes: u64, seconds: f64) -> f64 {
    if seconds > 0.0 {
        bytes as f64 / (1024.0 * 1024.0) / seconds
    } else {
        0.0
    }
}
