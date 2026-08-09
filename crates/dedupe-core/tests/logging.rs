//! Dual-format local logging smoke test.

#[test]
fn jsonl_and_text_logs_are_flushed_on_shutdown()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let guards = dedupe_core::logging::init(directory.path())?;
    tracing::warn!(
        operation = "scan",
        files = 2_u64,
        "safe metadata-only test event"
    );
    drop(guards);

    let mut json = String::new();
    let mut text = String::new();
    for entry in std::fs::read_dir(directory.path())? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let contents = std::fs::read_to_string(entry.path())?;
        if name.contains("jsonl") {
            json.push_str(&contents);
        } else if name.contains("safe-dedupe.log") {
            text.push_str(&contents);
        }
    }
    assert!(
        json.contains("safe metadata-only test event"),
        "JSON log content was: {json:?}; text was: {text:?}"
    );
    assert!(
        json.contains("\"operation\":\"scan\""),
        "JSON log content was: {json:?}"
    );
    assert!(
        text.contains("safe metadata-only test event"),
        "text log content was: {text:?}"
    );
    Ok(())
}
