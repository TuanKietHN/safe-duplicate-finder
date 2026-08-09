#![cfg(windows)]

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use safe_dedupe_runtime_installer::download::{
    DownloadError, download_artifact, validate_final_redirect_scheme,
};
use safe_dedupe_runtime_installer::manifest::{DetectionRule, RuntimeArtifact};
use safe_dedupe_runtime_installer::progress::ProgressBook;
use sha2::{Digest, Sha256};

fn artifact(url: String, bytes: &[u8]) -> RuntimeArtifact {
    RuntimeArtifact {
        id: "fixture-runtime".into(),
        display_name: "Fixture Runtime".into(),
        architecture: "x64".into(),
        url,
        size_bytes: u64::try_from(bytes.len()).expect("fixture length"),
        sha256: hex::encode_upper(Sha256::digest(bytes)),
        cache_file_name: "fixture-runtime.exe".into(),
        install_args: vec!["/silent".into()],
        detection: DetectionRule::Webview2Registry {
            app_guid: "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}".into(),
        },
        max_retries: 1,
    }
}

fn serve_once(
    body: Vec<u8>,
    partial_from: Option<usize>,
    ignore_range: bool,
) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fixture request");
        let mut request = vec![0_u8; 8_192];
        let count = stream.read(&mut request).expect("read request");
        let request = String::from_utf8_lossy(&request[..count]).into_owned();
        let start = partial_from.unwrap_or(0);
        if partial_from.is_some() && !ignore_range {
            let suffix = &body[start..];
            write!(
                stream,
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                suffix.len(),
                start,
                body.len() - 1,
                body.len()
            )
            .expect("write response headers");
            stream.write_all(suffix).expect("write response body");
        } else {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write response headers");
            stream.write_all(&body).expect("write response body");
        }
        request
    });
    (format!("http://{address}/runtime.exe"), handle)
}

fn part_path(cache: &Path, artifact: &RuntimeArtifact) -> std::path::PathBuf {
    cache.join(format!("{}.part", artifact.cache_file_name))
}

#[test]
fn downloads_fresh_bytes_and_verifies_sha256() {
    let bytes = b"verified fixture runtime".repeat(32);
    let (url, server) = serve_once(bytes.clone(), None, false);
    let artifact = artifact(url, &bytes);
    let cache = tempfile::tempdir().expect("cache dir");
    let progress = ProgressBook::new([(&artifact.id, artifact.size_bytes)]).expect("progress");

    let outcome = download_artifact(&artifact, cache.path(), &progress, &AtomicBool::new(false))
        .expect("download succeeds");

    assert_eq!(
        fs::read(outcome.complete_path).expect("read completed"),
        bytes
    );
    assert!(!outcome.reused_cache);
    assert!(
        server
            .join()
            .expect("server thread")
            .contains("GET /runtime.exe")
    );
    assert_eq!(progress.snapshot(1_000).received_bytes, artifact.size_bytes);
}

#[test]
fn resumes_only_missing_suffix_on_206() {
    let bytes = b"0123456789abcdef".repeat(128);
    let offset = bytes.len() / 2;
    let (url, server) = serve_once(bytes.clone(), Some(offset), false);
    let artifact = artifact(url, &bytes);
    let cache = tempfile::tempdir().expect("cache dir");
    fs::write(part_path(cache.path(), &artifact), &bytes[..offset]).expect("seed partial");
    let progress = ProgressBook::new([(&artifact.id, artifact.size_bytes)]).expect("progress");

    let outcome = download_artifact(&artifact, cache.path(), &progress, &AtomicBool::new(false))
        .expect("resume succeeds");

    assert_eq!(
        fs::read(outcome.complete_path).expect("read completed"),
        bytes
    );
    let request = server.join().expect("server thread");
    assert!(request.contains(&format!("Range: bytes={offset}-")));
    assert_eq!(
        outcome.network_bytes,
        artifact.size_bytes - u64::try_from(offset).expect("offset")
    );
}

#[test]
fn restarts_item_when_server_ignores_range() {
    let bytes = b"range ignored safely".repeat(64);
    let offset = bytes.len() / 3;
    let (url, server) = serve_once(bytes.clone(), Some(offset), true);
    let artifact = artifact(url, &bytes);
    let cache = tempfile::tempdir().expect("cache dir");
    fs::write(part_path(cache.path(), &artifact), &bytes[..offset]).expect("seed partial");
    let progress = ProgressBook::new([(&artifact.id, artifact.size_bytes)]).expect("progress");

    let outcome = download_artifact(&artifact, cache.path(), &progress, &AtomicBool::new(false))
        .expect("clean restart succeeds");

    assert_eq!(
        fs::read(outcome.complete_path).expect("read completed"),
        bytes
    );
    assert!(
        server
            .join()
            .expect("server thread")
            .contains(&format!("Range: bytes={offset}-"))
    );
    assert_eq!(outcome.network_bytes, artifact.size_bytes);
    assert_eq!(progress.snapshot(1_000).received_bytes, artifact.size_bytes);
}

#[test]
fn rejects_wrong_digest_before_promotion() {
    let expected = b"expected runtime".repeat(8);
    let altered = b"tampered runtime".repeat(8);
    let (url, server) = serve_once(altered, None, false);
    let artifact = artifact(url, &expected);
    let cache = tempfile::tempdir().expect("cache dir");
    let progress = ProgressBook::new([(&artifact.id, artifact.size_bytes)]).expect("progress");

    let error = download_artifact(&artifact, cache.path(), &progress, &AtomicBool::new(false))
        .expect_err("tampered bytes must fail");
    assert!(matches!(
        error,
        DownloadError::LengthMismatch { .. } | DownloadError::DigestMismatch { .. }
    ));
    assert!(!cache.path().join(&artifact.cache_file_name).exists());
    server.join().expect("server thread");
}

#[test]
fn reuses_completed_verified_cache_without_network() {
    let bytes = b"already complete".repeat(20);
    let artifact = artifact("http://127.0.0.1:9/not-used".into(), &bytes);
    let cache = tempfile::tempdir().expect("cache dir");
    fs::write(cache.path().join(&artifact.cache_file_name), &bytes).expect("seed completed");
    let progress = ProgressBook::new([(&artifact.id, artifact.size_bytes)]).expect("progress");

    let outcome = download_artifact(&artifact, cache.path(), &progress, &AtomicBool::new(false))
        .expect("cache is reused");
    assert!(outcome.reused_cache);
    assert_eq!(outcome.network_bytes, 0);
}

#[test]
fn pre_cancelled_download_opens_no_network_connection() {
    let bytes = b"cancel before connect".repeat(8);
    let artifact = artifact("http://127.0.0.1:9/must-not-connect".into(), &bytes);
    let cache = tempfile::tempdir().expect("cache dir");
    let progress = ProgressBook::new([(&artifact.id, artifact.size_bytes)]).expect("progress");
    let cancelled = AtomicBool::new(true);

    let error = download_artifact(&artifact, cache.path(), &progress, &cancelled)
        .expect_err("pre-cancelled transfer must stop before WinHTTP");

    assert!(matches!(error, DownloadError::Cancelled));
    assert_eq!(progress.snapshot(1_000).network_bytes_this_run, 0);
}

#[test]
fn truncated_response_is_retained_only_as_partial() {
    let bytes = b"truncated response fixture".repeat(128);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let half = bytes.len() / 2;
    let server_bytes = bytes.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fixture request");
        let mut request = [0_u8; 8_192];
        let _ = stream.read(&mut request).expect("read request");
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            server_bytes.len()
        )
        .expect("write response headers");
        stream
            .write_all(&server_bytes[..half])
            .expect("write truncated body");
    });
    let artifact = artifact(format!("http://{address}/runtime.exe"), &bytes);
    let cache = tempfile::tempdir().expect("cache dir");
    let progress = ProgressBook::new([(&artifact.id, artifact.size_bytes)]).expect("progress");

    let error = download_artifact(&artifact, cache.path(), &progress, &AtomicBool::new(false))
        .expect_err("truncated transfer must fail closed");

    assert!(matches!(
        error,
        DownloadError::LengthMismatch { .. } | DownloadError::WinHttp { .. }
    ));
    assert!(!cache.path().join(&artifact.cache_file_name).exists());
    let retained = fs::metadata(part_path(cache.path(), &artifact))
        .expect("partial is retained for resume")
        .len();
    assert!(retained > 0 && retained < artifact.size_bytes);
    server.join().expect("server thread");
}

#[test]
fn retry_resumes_bytes_retained_from_a_truncated_attempt() {
    let bytes = b"retry and resume fixture".repeat(256);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let first_count = bytes.len() / 3;
    let server_bytes = bytes.clone();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().expect("accept first request");
        let mut request = [0_u8; 8_192];
        let _ = first.read(&mut request).expect("read first request");
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            server_bytes.len()
        )
        .expect("write first headers");
        first
            .write_all(&server_bytes[..first_count])
            .expect("write first prefix");
        drop(first);

        let (mut second, _) = listener.accept().expect("accept retry request");
        let count = second.read(&mut request).expect("read retry request");
        let retry_request = String::from_utf8_lossy(&request[..count]);
        assert!(retry_request.contains(&format!("Range: bytes={first_count}-")));
        let suffix = &server_bytes[first_count..];
        write!(
            second,
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
            suffix.len(),
            first_count,
            server_bytes.len() - 1,
            server_bytes.len()
        )
        .expect("write retry headers");
        second.write_all(suffix).expect("write retry suffix");
    });
    let mut artifact = artifact(format!("http://{address}/runtime.exe"), &bytes);
    artifact.max_retries = 2;
    let cache = tempfile::tempdir().expect("cache dir");
    let progress = ProgressBook::new([(&artifact.id, artifact.size_bytes)]).expect("progress");
    let cancelled = AtomicBool::new(false);

    let outcome = download_artifact(&artifact, cache.path(), &progress, &cancelled)
        .expect("retry should resume retained prefix");

    assert_eq!(
        fs::read(outcome.complete_path).expect("completed file"),
        bytes
    );
    assert_eq!(outcome.network_bytes, artifact.size_bytes);
    assert!(!cancelled.load(Ordering::Acquire));
    server.join().expect("server thread");
}

#[test]
fn rejects_https_to_http_redirect_scheme() {
    let error = validate_final_redirect_scheme(
        "https://download.example.invalid/runtime.exe",
        "http://mirror.example.invalid/runtime.exe",
    )
    .expect_err("HTTPS downgrade must fail closed");
    assert!(matches!(error, DownloadError::InsecureRedirect));
    validate_final_redirect_scheme(
        "https://download.example.invalid/runtime.exe",
        "https://cdn.example.invalid/runtime.exe",
    )
    .expect("HTTPS redirect remains valid");
}
