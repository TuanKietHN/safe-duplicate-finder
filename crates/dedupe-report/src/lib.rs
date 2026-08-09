//! Streaming CSV, JSON, and escaped HTML report writers.

use std::io::Write;

use dedupe_core::model::{DuplicateGroup, MemberAction};

/// Report generation failure.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// I/O failed.
    #[error("I/O báo cáo thất bại: {0}")]
    Io(#[from] std::io::Error),
    /// CSV encoding failed.
    #[error("Mã hóa CSV thất bại: {0}")]
    Csv(#[from] csv::Error),
    /// JSON encoding failed.
    #[error("Mã hóa JSON thất bại: {0}")]
    Json(#[from] serde_json::Error),
}

/// Write one row per duplicate member without loading the report into memory.
pub fn write_csv(groups: &[DuplicateGroup], writer: impl Write) -> Result<(), ReportError> {
    let mut csv = csv::Writer::from_writer(writer);
    csv.write_record([
        "group_id",
        "path",
        "size_bytes",
        "action",
        "reason",
        "blake3",
        "sha256",
        "verified",
    ])?;
    for group in groups {
        for member in &group.members {
            csv.write_record([
                group.id.to_string(),
                member.file.metadata.path.to_string_lossy().into_owned(),
                member.file.metadata.size_bytes.to_string(),
                action_name(member.action).to_owned(),
                member.reason.clone(),
                hex::encode(&group.blake3),
                hex::encode(&group.sha256),
                (member.file.blake3.stable && member.file.sha256.stable).to_string(),
            ])?;
        }
    }
    csv.flush()?;
    Ok(())
}

/// Write structured JSON.
pub fn write_json(groups: &[DuplicateGroup], writer: impl Write) -> Result<(), ReportError> {
    serde_json::to_writer_pretty(writer, groups)?;
    Ok(())
}

/// Write a self-contained escaped HTML table.
pub fn write_html(groups: &[DuplicateGroup], mut writer: impl Write) -> Result<(), ReportError> {
    writer.write_all("<!doctype html><html lang=\"vi\"><meta charset=\"utf-8\"><title>Báo cáo tệp trùng lặp an toàn</title><style>body{font-family:system-ui;margin:2rem}table{border-collapse:collapse;width:100%}th,td{border:1px solid #bbb;padding:.4rem;text-align:left}th{background:#e7eef8}.keep{background:#e4f4df}.quarantine{background:#fff0d5}</style><h1>Báo cáo tệp trùng lặp an toàn</h1><table><thead><tr><th>Nhóm</th><th>Đường dẫn</th><th>Số byte</th><th>Thao tác</th><th>Lý do</th></tr></thead><tbody>".as_bytes())?;
    for group in groups {
        for member in &group.members {
            let action = action_name(member.action);
            write!(
                writer,
                "<tr class=\"{}\"><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                html_escape::encode_double_quoted_attribute(action),
                group.id,
                html_escape::encode_text(&member.file.metadata.path.to_string_lossy()),
                member.file.metadata.size_bytes,
                html_escape::encode_text(action_label(member.action)),
                html_escape::encode_text(&member.reason),
            )?;
        }
    }
    writer.write_all(b"</tbody></table>")?;
    Ok(())
}

fn action_name(action: MemberAction) -> &'static str {
    match action {
        MemberAction::Keep => "keep",
        MemberAction::Quarantine => "quarantine",
        MemberAction::Manual => "manual",
    }
}

fn action_label(action: MemberAction) -> &'static str {
    match action {
        MemberAction::Keep => "Giữ lại",
        MemberAction::Quarantine => "Cách ly",
        MemberAction::Manual => "Xem xét thủ công",
    }
}
