//! Safely generate bounded physical fixtures plus a 1 TB-class logical scenario manifest.

use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use chrono::Utc;
use clap::Parser;
use serde::Serialize;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[derive(Debug, Parser)]
#[command(about = "Tạo dữ liệu benchmark có giới hạn và manifest kích thước logic rõ ràng")]
struct Args {
    /// One or more roots, repeated to model folders or volumes.
    #[arg(long, required = true)]
    destination: Vec<PathBuf>,
    /// Manifest path; may live outside every generated source root.
    #[arg(long)]
    manifest: PathBuf,
    /// Small-file population in the reference scenario.
    #[arg(long, default_value_t = 100_000)]
    small_files: u64,
    /// PDF population in the reference scenario.
    #[arg(long, default_value_t = 10_000)]
    pdf_files: u64,
    /// Logical large-file population (sizes cycle from 1 through 20 GiB).
    #[arg(long, default_value_t = 88)]
    large_files: u64,
    /// Actually create the files. Without this flag only the safe manifest is written.
    #[arg(long)]
    materialize: bool,
    /// Physical bytes written per file when materializing.
    #[arg(long, default_value_t = 4096)]
    bytes_per_file: u64,
    /// Hard cap on aggregate physical bytes written by this invocation.
    #[arg(long, default_value_t = 512 * MIB)]
    max_materialized_bytes: u64,
}

#[derive(Debug, Serialize)]
struct Manifest {
    schema_version: u32,
    generated_at: String,
    destinations: Vec<PathBuf>,
    small_files: u64,
    pdf_files: u64,
    large_files: u64,
    logical_bytes: u64,
    materialized: bool,
    materialized_bytes: u64,
    note: &'static str,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args
        .destination
        .iter()
        .any(|root| root.as_os_str().is_empty())
    {
        bail!("Đích benchmark không được để trống");
    }
    let file_count = args
        .small_files
        .saturating_add(args.pdf_files)
        .saturating_add(args.large_files);
    let requested_physical = file_count.saturating_mul(args.bytes_per_file);
    if args.materialize && requested_physical > args.max_materialized_bytes {
        bail!("Từ chối ghi {requested_physical} byte; hãy chủ động tăng --max-materialized-bytes");
    }
    let logical_bytes = logical_bytes(&args);
    if args.materialize {
        for root in &args.destination {
            std::fs::create_dir_all(root)
                .with_context(|| format!("Tạo thư mục gốc benchmark {}", root.display()))?;
        }
        materialize_category(&args, "small", args.small_files, "bin")?;
        materialize_category(&args, "pdf", args.pdf_files, "pdf")?;
        materialize_category(&args, "large", args.large_files, "blob")?;
    }
    if let Some(parent) = args.manifest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Tạo thư mục manifest {}", parent.display()))?;
    }
    let manifest = Manifest {
        schema_version: 1,
        generated_at: Utc::now().to_rfc3339(),
        destinations: args.destination,
        small_files: args.small_files,
        pdf_files: args.pdf_files,
        large_files: args.large_files,
        logical_bytes,
        materialized: args.materialize,
        materialized_bytes: if args.materialize {
            requested_physical
        } else {
            0
        },
        note: "Kích thước logic mô tả kịch bản tham chiếu; số byte được ghi vật lý luôn được báo cáo riêng.",
    };
    serde_json::to_writer_pretty(File::create(&args.manifest)?, &manifest)?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

fn logical_bytes(args: &Args) -> u64 {
    let small = args.small_files.saturating_mul(16 * 1024);
    let pdf = args.pdf_files.saturating_mul(8 * MIB);
    let large = (0..args.large_files).fold(0_u64, |total, index| {
        total.saturating_add((index % 20 + 1).saturating_mul(GIB))
    });
    small.saturating_add(pdf).saturating_add(large)
}

fn materialize_category(
    args: &Args,
    category: &str,
    count: u64,
    extension: &str,
) -> anyhow::Result<()> {
    let destination_count = u64::try_from(args.destination.len())?;
    for index in 0..count {
        let root_index = usize::try_from(index % destination_count)?;
        let root = &args.destination[root_index];
        let duplicate_group = index / 2;
        let path = root
            .join(category)
            .join(format!("shard-{:03}", index % 128))
            .join(format!("copy-{}", index % 2))
            .join(format!("item-{duplicate_group:08}.{extension}"));
        write_deterministic(&path, args.bytes_per_file, duplicate_group)?;
    }
    Ok(())
}

fn write_deterministic(path: &Path, bytes: u64, seed: u64) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Tạo thư mục dữ liệu mẫu {}", parent.display()))?;
    }
    let mut writer = BufWriter::new(
        File::create(path).with_context(|| format!("Tạo dữ liệu mẫu {}", path.display()))?,
    );
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    for (offset, value) in buffer.iter_mut().enumerate() {
        *value = seed.wrapping_add(u64::try_from(offset)?).to_le_bytes()[0];
    }
    let mut remaining = bytes;
    while remaining != 0 {
        let buffer_bytes = u64::try_from(buffer.len())?;
        let chunk = usize::try_from(remaining.min(buffer_bytes))?;
        writer.write_all(&buffer[..chunk])?;
        remaining -= u64::try_from(chunk)?;
    }
    writer.flush()?;
    Ok(())
}
