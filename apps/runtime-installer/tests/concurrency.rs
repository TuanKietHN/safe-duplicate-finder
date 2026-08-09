use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use safe_dedupe_runtime_installer::download::READ_BUFFER_SIZE;
use safe_dedupe_runtime_installer::scheduler::run_bounded;

#[test]
fn overlaps_work_but_never_exceeds_two_workers() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let active_for_work = Arc::clone(&active);
    let peak_for_work = Arc::clone(&peak);

    let output = run_bounded(vec![1, 2, 3, 4], 2, move |item| {
        let now = active_for_work.fetch_add(1, Ordering::SeqCst) + 1;
        peak_for_work.fetch_max(now, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(30));
        active_for_work.fetch_sub(1, Ordering::SeqCst);
        item * 2
    })
    .expect("bounded scheduler");

    assert_eq!(output, vec![2, 4, 6, 8]);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[test]
fn rejects_zero_or_unreasonably_large_worker_limits() {
    assert!(run_bounded(vec![1], 0, |item| item).is_err());
    assert!(run_bounded(vec![1], 3, |item| item).is_err());
}

#[test]
fn each_worker_uses_one_fixed_64_kib_streaming_buffer() {
    assert_eq!(READ_BUFFER_SIZE, 64 * 1_024);
    const { assert!(READ_BUFFER_SIZE * 2 < 1024 * 1024) };
}
