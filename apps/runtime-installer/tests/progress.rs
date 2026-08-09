use safe_dedupe_runtime_installer::progress::{ItemState, ProgressBook};

#[test]
fn aggregate_progress_uses_only_real_bytes() {
    let book = ProgressBook::new([("a", 1_000), ("b", 3_000)]).expect("valid totals");
    book.set_existing_bytes("a", 500).expect("known item");
    book.record_network_bytes("b", 1_500, 1_000)
        .expect("known item");
    let snapshot = book.snapshot(1_000);

    assert_eq!(snapshot.required_download_bytes, 4_000);
    assert_eq!(snapshot.received_bytes, 2_000);
    assert_eq!(snapshot.network_bytes_this_run, 1_500);
    assert_eq!(snapshot.overall_basis_points, 5_000);
}

#[test]
fn rolling_speed_and_eta_follow_recorded_byte_samples() {
    let book = ProgressBook::new([("runtime", 10_000)]).expect("valid totals");
    book.record_network_bytes("runtime", 2_000, 1_000)
        .expect("known item");
    book.record_network_bytes("runtime", 2_000, 2_000)
        .expect("known item");

    let snapshot = book.snapshot(2_000);
    assert_eq!(snapshot.bytes_per_second, 2_000);
    assert_eq!(snapshot.eta_seconds, Some(3));
    assert_eq!(snapshot.received_bytes, 4_000);
}

#[test]
fn completed_cache_counts_without_network_bytes() {
    let book = ProgressBook::new([("runtime", 42)]).expect("valid totals");
    book.set_existing_bytes("runtime", 42).expect("known item");
    book.set_state("runtime", ItemState::CacheValid)
        .expect("known item");

    let snapshot = book.snapshot(0);
    assert_eq!(snapshot.received_bytes, 42);
    assert_eq!(snapshot.network_bytes_this_run, 0);
    assert_eq!(snapshot.overall_basis_points, 10_000);
    assert_eq!(snapshot.items[0].state, ItemState::CacheValid);
}

#[test]
fn rejects_overflow_and_unknown_items() {
    assert!(ProgressBook::new([("a", u64::MAX), ("b", 1)]).is_err());
    let book = ProgressBook::new([("a", 10)]).expect("valid totals");
    assert!(book.set_existing_bytes("missing", 1).is_err());
    assert!(book.set_existing_bytes("a", 11).is_err());
}

#[test]
fn installed_items_remain_visible_but_leave_download_totals() {
    let book = ProgressBook::new([("installed", 500), ("missing", 1_500)]).expect("valid totals");
    book.set_required("installed", false).expect("known item");
    book.set_existing_bytes("installed", 500)
        .expect("known item");
    book.set_state("installed", ItemState::InstalledValid)
        .expect("known item");

    let snapshot = book.snapshot(0);
    assert_eq!(snapshot.items.len(), 2);
    assert_eq!(snapshot.required_download_bytes, 1_500);
    assert_eq!(snapshot.received_bytes, 0);
}
