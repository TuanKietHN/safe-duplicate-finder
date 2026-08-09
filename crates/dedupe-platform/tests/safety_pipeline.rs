//! Cross-layer safety tests against the real platform adapter.

use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use chrono::Utc;
use dedupe_core::{
    DedupeError, Result,
    control::ControlToken,
    duplicate_detector::{confirm_preliminary_group, confirm_preliminary_group_detailed},
    full_hash,
    model::{
        ComparisonMode, FileTransaction, KeepPolicy, MemberAction, OperationPlan, ProvenFile,
        TransactionKind, TransactionState,
    },
    ports::{MetadataProvider, SafeMover, TransactionJournal},
    quarantine, quick_hash,
};
use dedupe_platform::PlatformFileSystem;
use dedupe_testkit::{
    Fixture,
    faults::{FaultInjector, FaultPoint, FaultingJournal, FaultingMover},
};
use uuid::Uuid;

#[derive(Default)]
struct MemoryJournal {
    states: Mutex<Vec<TransactionState>>,
}

impl MemoryJournal {
    fn states(&self) -> Result<Vec<TransactionState>> {
        self.states
            .lock()
            .map(|states| states.clone())
            .map_err(|_| DedupeError::State("test journal lock poisoned".into()))
    }
}

impl TransactionJournal for MemoryJournal {
    fn create(&self, transaction: &FileTransaction) -> Result<()> {
        self.states
            .lock()
            .map_err(|_| DedupeError::State("test journal lock poisoned".into()))?
            .push(transaction.state);
        Ok(())
    }

    fn transition(
        &self,
        _transaction: &FileTransaction,
        next: TransactionState,
        _verification: Option<&str>,
        _error: Option<&str>,
    ) -> Result<()> {
        self.states
            .lock()
            .map_err(|_| DedupeError::State("test journal lock poisoned".into()))?
            .push(next);
        Ok(())
    }
}

struct FailingJournal {
    fail_create: bool,
    fail_on: Option<TransactionState>,
}

impl TransactionJournal for FailingJournal {
    fn create(&self, _transaction: &FileTransaction) -> Result<()> {
        if self.fail_create {
            Err(DedupeError::Durability(
                "injected journal create failure".into(),
            ))
        } else {
            Ok(())
        }
    }

    fn transition(
        &self,
        _transaction: &FileTransaction,
        next: TransactionState,
        _verification: Option<&str>,
        _error: Option<&str>,
    ) -> Result<()> {
        if self.fail_on == Some(next) {
            Err(DedupeError::Durability(format!(
                "injected journal transition failure at {next:?}"
            )))
        } else {
            Ok(())
        }
    }
}

struct CorruptingMover;

#[derive(Clone, Copy)]
enum InjectedMoveFailure {
    PermissionDenied,
    StorageFull,
    Disconnected,
    WrongVolume,
}

struct ClassifiedFailMover(InjectedMoveFailure);

impl SafeMover for ClassifiedFailMover {
    fn move_no_replace(
        &self,
        source: &Path,
        _destination: &Path,
        _expected: &dedupe_core::model::FileMetadataSnapshot,
    ) -> Result<()> {
        match self.0 {
            InjectedMoveFailure::PermissionDenied => Err(DedupeError::io(
                "injected permission denial",
                source,
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "antivirus denied move",
                ),
            )),
            InjectedMoveFailure::StorageFull => Err(DedupeError::io(
                "injected full quarantine volume",
                source,
                std::io::Error::new(std::io::ErrorKind::StorageFull, "quarantine volume full"),
            )),
            InjectedMoveFailure::Disconnected => Err(DedupeError::io(
                "injected volume disconnect",
                source,
                std::io::Error::new(std::io::ErrorKind::NotConnected, "volume disconnected"),
            )),
            InjectedMoveFailure::WrongVolume => Err(DedupeError::Safety(
                "source and destination are not on the same volume".into(),
            )),
        }
    }
}

impl SafeMover for CorruptingMover {
    fn move_no_replace(
        &self,
        source: &Path,
        destination: &Path,
        expected: &dedupe_core::model::FileMetadataSnapshot,
    ) -> Result<()> {
        PlatformFileSystem.move_no_replace(source, destination, expected)?;
        std::fs::write(destination, b"tampered")
            .map_err(|error| DedupeError::io("corrupt destination fixture", destination, error))
    }
}

struct SelectiveReadFailure {
    denied: PathBuf,
}

#[derive(Default)]
struct ChangingMetadata {
    calls: AtomicUsize,
}

impl MetadataProvider for ChangingMetadata {
    fn snapshot(&self, path: &Path) -> Result<dedupe_core::model::FileMetadataSnapshot> {
        let mut snapshot = PlatformFileSystem.snapshot(path)?;
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            snapshot.modified_ns = snapshot.modified_ns.saturating_add(1);
            snapshot.snapshot_token = dedupe_core::metadata::snapshot_token(
                snapshot.identity.as_ref(),
                snapshot.size_bytes,
                snapshot.modified_ns,
            );
        }
        Ok(snapshot)
    }
}

impl MetadataProvider for SelectiveReadFailure {
    fn snapshot(&self, path: &Path) -> Result<dedupe_core::model::FileMetadataSnapshot> {
        if path == self.denied {
            return Err(DedupeError::io(
                "injected metadata denial",
                path,
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected"),
            ));
        }
        PlatformFileSystem.snapshot(path)
    }
}

#[test]
fn same_name_and_size_with_one_different_byte_is_not_duplicate()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let first = fixture.write("a/book.pdf", b"0123456789")?;
    let second = fixture.write("b/book.pdf", b"0123456788")?;
    let provider = PlatformFileSystem;
    let candidates = vec![provider.snapshot(&first)?, provider.snapshot(&second)?];

    let groups = confirm_preliminary_group(
        ComparisonMode::Strict,
        &candidates,
        &provider,
        &ControlToken::new(),
    )?;

    assert!(groups.is_empty());
    Ok(())
}

#[test]
fn exact_independent_files_require_both_full_digests()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let bytes = vec![0x5a; 192 * 1024];
    let first = fixture.write("a/manual.epub", &bytes)?;
    let second = fixture.write("b/manual.epub", &bytes)?;
    let provider = PlatformFileSystem;
    let candidates = vec![provider.snapshot(&first)?, provider.snapshot(&second)?];

    let groups = confirm_preliminary_group(
        ComparisonMode::Strict,
        &candidates,
        &provider,
        &ControlToken::new(),
    )?;

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].members.len(), 2);
    assert_eq!(groups[0].blake3.len(), 32);
    assert_eq!(groups[0].sha256.len(), 32);
    Ok(())
}

#[test]
fn unreadable_candidate_is_isolated_without_hiding_readable_duplicates()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let first = fixture.write("a/isolated.pdf", b"same readable content")?;
    let second = fixture.write("b/isolated.pdf", b"same readable content")?;
    let denied = fixture.write("c/isolated.pdf", b"same readable content")?;
    let platform = PlatformFileSystem;
    let candidates = vec![
        platform.snapshot(&first)?,
        platform.snapshot(&second)?,
        platform.snapshot(&denied)?,
    ];
    let provider = SelectiveReadFailure {
        denied: denied.clone(),
    };

    let outcome = confirm_preliminary_group_detailed(
        ComparisonMode::Strict,
        &candidates,
        &provider,
        &ControlToken::new(),
    )?;

    assert_eq!(outcome.errors.len(), 1);
    assert_eq!(outcome.errors[0].path, denied);
    assert_eq!(outcome.errors[0].stage, "quick_hash");
    assert_eq!(outcome.groups.len(), 1);
    assert_eq!(outcome.groups[0].members.len(), 2);
    assert!(outcome.bytes_read > 0);
    Ok(())
}

#[test]
fn empty_and_multi_chunk_files_have_complete_stable_hashes()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let empty = fixture.write("hash/empty.bin", b"")?;
    let bytes = vec![0x6b; 2 * 1024 * 1024 + 17];
    let large = fixture.write("hash/large.bin", &bytes)?;
    let provider = PlatformFileSystem;
    let control = ControlToken::new();

    let empty_blake3 = full_hash::blake3_file(&empty, &provider, &control)?;
    let empty_sha256 = full_hash::sha256_file(&empty, &provider, &control)?;
    assert_eq!(empty_blake3.bytes_read, 0);
    assert_eq!(empty_sha256.bytes_read, 0);
    assert!(empty_blake3.stable && empty_sha256.stable);
    assert_eq!(empty_blake3.digest, blake3::hash(b"").as_bytes());

    let large_blake3 = full_hash::blake3_file(&large, &provider, &control)?;
    let large_sha256 = full_hash::sha256_file(&large, &provider, &control)?;
    assert_eq!(large_blake3.bytes_read, bytes.len() as u64);
    assert_eq!(large_sha256.bytes_read, bytes.len() as u64);
    assert!(large_blake3.stable && large_sha256.stable);
    assert_eq!(large_blake3.digest, blake3::hash(&bytes).as_bytes());
    Ok(())
}

#[test]
fn greater_than_four_gib_logical_file_uses_u64_offsets_without_full_read()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let path = fixture.path().join("hash/large-sparse.bin");
    std::fs::create_dir_all(path.parent().ok_or("missing sparse parent")?)?;
    let logical_size = u64::from(u32::MAX) + 65_537;
    let file = std::fs::File::create(&path)?;
    file.set_len(logical_size)?;
    file.sync_all()?;
    let provider = PlatformFileSystem;

    let snapshot = provider.snapshot(&path)?;
    let sampled = quick_hash::hash_file(&path, &provider, &ControlToken::new())?;

    assert_eq!(snapshot.size_bytes, logical_size);
    assert_eq!(sampled.bytes_read, quick_hash::SAMPLE_BYTES * 3);
    assert!(sampled.stable);
    Ok(())
}

#[test]
fn unicode_long_path_round_trips_through_native_identity_and_hashing()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let deep = (0..8)
        .map(|index| format!("thư-mục-資料-rất-dài-{index:02}"))
        .collect::<Vec<_>>()
        .join("/");
    let relative = format!("unicode/{deep}/sách-điện-tử-完全版.pdf");
    let path = fixture.write(&relative, "nội dung nguyên vẹn".as_bytes())?;
    assert!(path.as_os_str().len() > 260);
    let provider = PlatformFileSystem;

    let snapshot = provider.snapshot(&path)?;
    let blake3 = full_hash::blake3_file(&path, &provider, &ControlToken::new())?;
    let sha256 = full_hash::sha256_file(&path, &provider, &ControlToken::new())?;

    assert_eq!(snapshot.path, path);
    assert_eq!(snapshot.normalized_name, "sách-điện-tử-完全版.pdf");
    assert!(blake3.stable && sha256.stable);
    assert_eq!(blake3.bytes_read, "nội dung nguyên vẹn".len() as u64);
    Ok(())
}

#[test]
fn handle_bound_mover_accepts_a_destination_beyond_legacy_max_path()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("long-move/source.pdf", b"long destination remains exact")?;
    let expected = PlatformFileSystem.snapshot(&source)?;
    let deep = (0..9)
        .map(|index| format!("thư-mục-cách-ly-rất-dài-{index:02}"))
        .collect::<Vec<_>>()
        .join("/");
    let destination = fixture
        .path()
        .join(format!("quarantine/{deep}/tài-liệu-đích.pdf"));
    assert!(destination.as_os_str().len() > 260);

    PlatformFileSystem.move_no_replace(&source, &destination, &expected)?;

    assert!(!source.exists());
    assert_eq!(
        std::fs::read(&destination)?,
        b"long destination remains exact"
    );
    let moved = PlatformFileSystem.snapshot(&destination)?;
    assert_eq!(moved.identity, expected.identity);
    Ok(())
}

#[test]
fn full_hash_observes_pre_cancelled_control_without_reading()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let file = fixture.write("hash/cancelled.bin", &[0x77; 4096])?;
    let control = ControlToken::new();
    control.cancel();

    let result = full_hash::blake3_file(&file, &PlatformFileSystem, &control);

    assert!(matches!(result, Err(DedupeError::Cancelled)));
    assert_eq!(std::fs::metadata(file)?.len(), 4096);
    Ok(())
}

#[test]
fn file_changed_during_full_hash_is_unstable_and_never_positive_evidence()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let contents = vec![0x39; 256 * 1024];
    let file = fixture.write("hash/changing.bin", &contents)?;

    let result = full_hash::blake3_file(&file, &ChangingMetadata::default(), &ControlToken::new())?;

    assert!(!result.stable);
    assert_ne!(result.snapshot_before, result.snapshot_after);
    assert_eq!(std::fs::metadata(file)?.len(), 256 * 1024);
    Ok(())
}

#[test]
fn hard_link_aliases_are_not_independent_duplicates()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let first = fixture.write("hardlinks/a/book.pdf", b"same allocation")?;
    let second = fixture.path().join("hardlinks/b/book.pdf");
    std::fs::create_dir_all(second.parent().ok_or("missing parent")?)?;
    std::fs::hard_link(&first, &second)?;
    let provider = PlatformFileSystem;
    let first_snapshot = provider.snapshot(&first)?;
    let second_snapshot = provider.snapshot(&second)?;
    assert_eq!(first_snapshot.identity, second_snapshot.identity);

    let groups = confirm_preliminary_group(
        ComparisonMode::Strict,
        &[first_snapshot, second_snapshot],
        &provider,
        &ControlToken::new(),
    )?;

    assert!(groups.is_empty());
    Ok(())
}

#[test]
fn files_with_unselected_hardlink_aliases_never_enter_quarantine_groups()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let first = fixture.write("hardlink-count/a/book.pdf", b"same content")?;
    let second = fixture.write("hardlink-count/b/book.pdf", b"same content")?;
    let first_alias = fixture.path().join("outside/first-alias.pdf");
    let second_alias = fixture.path().join("outside/second-alias.pdf");
    std::fs::create_dir_all(first_alias.parent().ok_or("missing alias parent")?)?;
    std::fs::hard_link(&first, &first_alias)?;
    std::fs::hard_link(&second, &second_alias)?;
    let provider = PlatformFileSystem;
    let candidates = vec![provider.snapshot(&first)?, provider.snapshot(&second)?];

    let groups = confirm_preliminary_group(
        ComparisonMode::Strict,
        &candidates,
        &provider,
        &ControlToken::new(),
    )?;

    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.hardlink_count == Some(2))
    );
    assert!(groups.is_empty());
    Ok(())
}

#[test]
fn content_mode_can_prove_equal_files_with_different_names()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let first = fixture.write("content/first.pdf", b"same content")?;
    let second = fixture.write("content/renamed.epub", b"same content")?;
    let provider = PlatformFileSystem;
    let candidates = vec![provider.snapshot(&first)?, provider.snapshot(&second)?];

    let strict = confirm_preliminary_group(
        ComparisonMode::Strict,
        &candidates,
        &provider,
        &ControlToken::new(),
    )?;
    let content = confirm_preliminary_group(
        ComparisonMode::Content,
        &candidates,
        &provider,
        &ControlToken::new(),
    )?;

    assert!(strict.is_empty());
    assert_eq!(content.len(), 1);
    assert_eq!(content[0].members.len(), 2);
    assert!(content[0].normalized_name.is_none());
    Ok(())
}

#[test]
fn dry_run_is_zero_mutation_and_rejects_stale_sealed_evidence()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let first = fixture.write("dry-run/a/document.pdf", b"dry run payload")?;
    let second = fixture.write("dry-run/b/document.pdf", b"dry run payload")?;
    let provider = PlatformFileSystem;
    let candidates = vec![provider.snapshot(&first)?, provider.snapshot(&second)?];
    let mut groups = confirm_preliminary_group(
        ComparisonMode::Strict,
        &candidates,
        &provider,
        &ControlToken::new(),
    )?;
    assert_eq!(groups.len(), 1);
    dedupe_core::keep_policy::apply(&mut groups[0], &KeepPolicy::Oldest)?;
    let plan = OperationPlan {
        id: Uuid::new_v4(),
        groups,
        evidence_version: 1,
        created_at: Utc::now(),
    };
    let before_first = std::fs::read(&first)?;
    let before_second = std::fs::read(&second)?;

    dedupe_core::dry_run::validate_fresh(&plan.groups, &provider)?;
    let report = dedupe_core::dry_run::build(&plan)?;
    assert_eq!(report.items.len(), 1);
    assert_eq!(
        report.potential_reclaimable_bytes,
        b"dry run payload".len() as u64
    );
    assert_eq!(std::fs::read(&first)?, before_first);
    assert_eq!(std::fs::read(&second)?, before_second);

    let stale_path = plan.groups[0]
        .members
        .iter()
        .find(|member| member.action == MemberAction::Quarantine)
        .map(|member| member.file.metadata.path.clone())
        .ok_or("dry-run fixture has no quarantine member")?;
    std::fs::write(&stale_path, b"changed after plan sealing")?;
    assert!(matches!(
        dedupe_core::dry_run::validate_fresh(&plan.groups, &provider),
        Err(DedupeError::Safety(_))
    ));
    Ok(())
}

#[test]
fn stale_or_missing_last_keeper_blocks_quarantine_before_any_move()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let first = fixture.write("keeper/a/document.pdf", b"keeper payload")?;
    let second = fixture.write("keeper/b/document.pdf", b"keeper payload")?;
    let provider = PlatformFileSystem;
    let candidates = vec![provider.snapshot(&first)?, provider.snapshot(&second)?];
    let mut groups = confirm_preliminary_group(
        ComparisonMode::Strict,
        &candidates,
        &provider,
        &ControlToken::new(),
    )?;
    dedupe_core::keep_policy::apply(&mut groups[0], &KeepPolicy::ShortestPath)?;
    let keeper_path = groups[0]
        .members
        .iter()
        .find(|member| member.action == MemberAction::Keep)
        .map(|member| member.file.metadata.path.clone())
        .ok_or("keeper policy selected no keeper")?;
    std::fs::write(&keeper_path, b"keeper changed after plan")?;
    assert!(matches!(
        quarantine::verify_live_keeper(&groups[0], &provider, &ControlToken::new()),
        Err(DedupeError::Safety(_))
    ));
    assert!(first.exists() && second.exists());

    for member in &mut groups[0].members {
        member.action = MemberAction::Manual;
    }
    assert!(matches!(
        quarantine::verify_live_keeper(&groups[0], &provider, &ControlToken::new()),
        Err(DedupeError::Safety(_))
    ));
    assert!(first.exists() && second.exists());
    Ok(())
}

#[test]
fn changed_source_is_rejected_before_move() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("source/document.pdf", b"stable evidence")?;
    let destination = fixture.path().join("quarantine/document.pdf");
    let provider = PlatformFileSystem;
    let mut transaction = transaction_for(&source, destination.clone(), &provider)?;
    std::fs::write(&source, b"changed after evidence")?;
    let journal = MemoryJournal::default();

    let result = quarantine::execute(
        &mut transaction,
        &provider,
        &provider,
        &journal,
        &ControlToken::new(),
    );

    assert!(result.is_err());
    assert!(source.exists());
    assert!(!destination.exists());
    assert_eq!(
        journal.states()?,
        vec![TransactionState::Planned, TransactionState::PreflightFailed]
    );
    Ok(())
}

#[test]
fn handle_bound_mover_rejects_a_replaced_source_path()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("race/source.pdf", b"original allocation")?;
    let destination = fixture.path().join("race-quarantine/source.pdf");
    let provider = PlatformFileSystem;
    let expected = provider.snapshot(&source)?;
    std::fs::remove_file(&source)?;
    std::fs::write(&source, b"replacement bytes")?;

    let result = provider.move_no_replace(&source, &destination, &expected);

    assert!(result.is_err());
    assert_eq!(std::fs::read(source)?, b"replacement bytes");
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn corrupt_destination_never_reaches_verified()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("source/document.pdf", b"original")?;
    let destination = fixture.path().join("quarantine/document.pdf");
    let provider = PlatformFileSystem;
    let mut transaction = transaction_for(&source, destination.clone(), &provider)?;
    let journal = MemoryJournal::default();

    let result = quarantine::execute(
        &mut transaction,
        &provider,
        &CorruptingMover,
        &journal,
        &ControlToken::new(),
    );

    assert!(result.is_err());
    assert!(destination.exists());
    assert_eq!(transaction.state, TransactionState::RecoveryRequired);
    let states = journal.states()?;
    assert!(!states.contains(&TransactionState::Verified));
    assert!(states.contains(&TransactionState::MovedUnverified));
    assert_eq!(states.last(), Some(&TransactionState::RecoveryRequired));
    Ok(())
}

#[test]
fn verified_quarantine_can_be_restored_without_overwrite()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("source/document.pdf", b"recoverable")?;
    let destination = fixture.path().join("quarantine/document.pdf");
    let provider = PlatformFileSystem;
    let journal = MemoryJournal::default();
    let mut quarantine_tx = transaction_for(&source, destination.clone(), &provider)?;
    quarantine::execute(
        &mut quarantine_tx,
        &provider,
        &provider,
        &journal,
        &ControlToken::new(),
    )?;
    assert_eq!(quarantine_tx.state, TransactionState::Verified);

    let mut restore_tx = dedupe_core::restore::planned_transaction(&quarantine_tx)?;
    dedupe_core::restore::execute(
        &mut restore_tx,
        &provider,
        &provider,
        &journal,
        &ControlToken::new(),
    )?;

    assert_eq!(restore_tx.state, TransactionState::Verified);
    assert!(source.exists());
    assert!(!destination.exists());
    assert_eq!(std::fs::read(source)?, b"recoverable");
    Ok(())
}

#[test]
fn occupied_quarantine_destination_is_never_overwritten()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("source/document.pdf", b"source payload")?;
    let destination = fixture.write("quarantine/document.pdf", b"existing payload")?;
    let provider = PlatformFileSystem;
    let mut transaction = transaction_for(&source, destination.clone(), &provider)?;

    let result = quarantine::execute(
        &mut transaction,
        &provider,
        &provider,
        &MemoryJournal::default(),
        &ControlToken::new(),
    );

    assert!(result.is_err());
    assert_eq!(transaction.state, TransactionState::RecoveryRequired);
    assert_eq!(std::fs::read(source)?, b"source payload");
    assert_eq!(std::fs::read(destination)?, b"existing payload");
    Ok(())
}

#[test]
fn denied_full_disconnected_and_wrong_volume_moves_preserve_source()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    for failure in [
        InjectedMoveFailure::PermissionDenied,
        InjectedMoveFailure::StorageFull,
        InjectedMoveFailure::Disconnected,
        InjectedMoveFailure::WrongVolume,
    ] {
        let fixture = Fixture::new()?;
        let source = fixture.write("classified/source.pdf", b"preserve on move failure")?;
        let destination = fixture.path().join("classified-quarantine/source.pdf");
        let provider = PlatformFileSystem;
        let mut transaction = transaction_for(&source, destination.clone(), &provider)?;
        let result = quarantine::execute(
            &mut transaction,
            &provider,
            &ClassifiedFailMover(failure),
            &MemoryJournal::default(),
            &ControlToken::new(),
        );
        assert!(result.is_err());
        assert_eq!(transaction.state, TransactionState::RecoveryRequired);
        assert!(source.exists());
        assert!(!destination.exists());
        assert_eq!(std::fs::read(source)?, b"preserve on move failure");
    }
    Ok(())
}

#[test]
fn occupied_restore_destination_preserves_both_copies()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("source/document.pdf", b"recoverable")?;
    let destination = fixture.path().join("quarantine/document.pdf");
    let provider = PlatformFileSystem;
    let journal = MemoryJournal::default();
    let mut quarantine_tx = transaction_for(&source, destination.clone(), &provider)?;
    quarantine::execute(
        &mut quarantine_tx,
        &provider,
        &provider,
        &journal,
        &ControlToken::new(),
    )?;
    std::fs::write(&source, b"new occupant")?;
    let mut restore_tx = dedupe_core::restore::planned_transaction(&quarantine_tx)?;

    let result = dedupe_core::restore::execute(
        &mut restore_tx,
        &provider,
        &provider,
        &journal,
        &ControlToken::new(),
    );

    assert!(result.is_err());
    assert_eq!(restore_tx.state, TransactionState::RecoveryRequired);
    assert_eq!(std::fs::read(source)?, b"new occupant");
    assert_eq!(std::fs::read(destination)?, b"recoverable");
    Ok(())
}

#[test]
fn journal_failure_before_mutation_leaves_source_untouched()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("source/document.pdf", b"durable first")?;
    let destination = fixture.path().join("quarantine/document.pdf");
    let provider = PlatformFileSystem;
    let mut transaction = transaction_for(&source, destination.clone(), &provider)?;
    let journal = FailingJournal {
        fail_create: true,
        fail_on: None,
    };

    let result = quarantine::execute(
        &mut transaction,
        &provider,
        &provider,
        &journal,
        &ControlToken::new(),
    );

    assert!(result.is_err());
    assert_eq!(transaction.state, TransactionState::Planned);
    assert!(source.exists());
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn crash_window_after_move_is_reconciled_and_verified()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("source/document.pdf", b"recover after move")?;
    let destination = fixture.path().join("quarantine/document.pdf");
    let provider = PlatformFileSystem;
    let mut transaction = transaction_for(&source, destination.clone(), &provider)?;
    let failing = FailingJournal {
        fail_create: false,
        fail_on: Some(TransactionState::MovedUnverified),
    };
    let result = quarantine::execute(
        &mut transaction,
        &provider,
        &provider,
        &failing,
        &ControlToken::new(),
    );
    assert!(result.is_err());
    assert_eq!(transaction.state, TransactionState::Moving);
    assert!(!source.exists());
    assert!(destination.exists());

    let outcome = dedupe_core::recovery::reconcile(
        &mut transaction,
        &provider,
        &MemoryJournal::default(),
        &ControlToken::new(),
    )?;
    assert_eq!(
        outcome,
        dedupe_core::recovery::Reconciliation::DestinationVerified
    );
    assert_eq!(transaction.state, TransactionState::Verified);
    Ok(())
}

#[test]
fn quarantine_fault_matrix_preserves_a_copy_at_every_durable_boundary()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let points = [
        FaultPoint::JournalCreate,
        FaultPoint::PreflightTransition,
        FaultPoint::MovingTransition,
        FaultPoint::FilesystemMove,
        FaultPoint::MovedTransition,
        FaultPoint::VerifiedTransition,
    ];
    for point in points {
        let fixture = Fixture::new()?;
        let source = fixture.write("matrix/source.pdf", b"fault matrix payload")?;
        let destination = fixture.path().join("matrix-quarantine/source.pdf");
        let provider = PlatformFileSystem;
        let mut transaction = transaction_for(&source, destination.clone(), &provider)?;
        let faults = FaultInjector::default();
        faults.fail_on(point, 1)?;
        let journal = FaultingJournal {
            inner: MemoryJournal::default(),
            faults: faults.clone(),
        };
        let mover = FaultingMover {
            inner: provider,
            faults,
        };

        let result = quarantine::execute(
            &mut transaction,
            &provider,
            &mover,
            &journal,
            &ControlToken::new(),
        );

        assert!(result.is_err(), "fault did not trigger at {point:?}");
        assert!(
            source.exists() || destination.exists(),
            "all copies disappeared at {point:?}"
        );
        if matches!(
            point,
            FaultPoint::MovedTransition | FaultPoint::VerifiedTransition
        ) {
            assert!(
                !source.exists(),
                "source unexpectedly remained at {point:?}"
            );
            assert!(destination.exists(), "destination missing at {point:?}");
            let recovery = dedupe_core::recovery::reconcile(
                &mut transaction,
                &provider,
                &MemoryJournal::default(),
                &ControlToken::new(),
            )?;
            assert_eq!(
                recovery,
                dedupe_core::recovery::Reconciliation::DestinationVerified
            );
            assert_eq!(transaction.state, TransactionState::Verified);
        } else {
            assert!(source.exists(), "source missing at {point:?}");
            assert!(!destination.exists(), "unexpected destination at {point:?}");
        }
    }
    Ok(())
}

#[test]
fn restore_fault_matrix_preserves_quarantine_or_verified_original_at_every_boundary()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let points = [
        FaultPoint::JournalCreate,
        FaultPoint::PreflightTransition,
        FaultPoint::MovingTransition,
        FaultPoint::FilesystemMove,
        FaultPoint::MovedTransition,
        FaultPoint::VerifiedTransition,
    ];
    for point in points {
        let fixture = Fixture::new()?;
        let original = fixture.write("restore-matrix/original.pdf", b"restore matrix payload")?;
        let quarantined = fixture
            .path()
            .join("restore-matrix-quarantine/original.pdf");
        let provider = PlatformFileSystem;
        let mut quarantine_transaction =
            transaction_for(&original, quarantined.clone(), &provider)?;
        quarantine::execute(
            &mut quarantine_transaction,
            &provider,
            &provider,
            &MemoryJournal::default(),
            &ControlToken::new(),
        )
        .map_err(|error| format!("setup quarantine failed before {point:?}: {error}"))?;
        assert!(!original.exists());
        assert!(quarantined.exists());
        let mut restore_transaction =
            dedupe_core::restore::planned_transaction(&quarantine_transaction)?;
        let faults = FaultInjector::default();
        faults.fail_on(point, 1)?;
        let journal = FaultingJournal {
            inner: MemoryJournal::default(),
            faults: faults.clone(),
        };
        let mover = FaultingMover {
            inner: provider,
            faults,
        };

        let result = dedupe_core::restore::execute(
            &mut restore_transaction,
            &provider,
            &mover,
            &journal,
            &ControlToken::new(),
        );

        assert!(
            result.is_err(),
            "restore fault did not trigger at {point:?}"
        );
        assert!(
            original.exists() || quarantined.exists(),
            "all restore copies disappeared at {point:?}"
        );
        if matches!(
            point,
            FaultPoint::MovedTransition | FaultPoint::VerifiedTransition
        ) {
            assert!(original.exists(), "restored original missing at {point:?}");
            assert!(
                !quarantined.exists(),
                "quarantine source unexpectedly remained at {point:?}"
            );
            let recovery = dedupe_core::recovery::reconcile(
                &mut restore_transaction,
                &provider,
                &MemoryJournal::default(),
                &ControlToken::new(),
            )?;
            assert_eq!(
                recovery,
                dedupe_core::recovery::Reconciliation::DestinationVerified
            );
            assert_eq!(restore_transaction.state, TransactionState::Verified);
        } else {
            assert!(!original.exists(), "unexpected original at {point:?}");
            assert!(quarantined.exists(), "quarantine copy missing at {point:?}");
        }
    }
    Ok(())
}

#[test]
fn recovery_records_source_only_both_and_missing_dispositions()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let provider = PlatformFileSystem;

    let source_only = fixture.write("source-only/file.pdf", b"one")?;
    let mut source_tx = transaction_for(
        &source_only,
        fixture.path().join("quarantine/source-only.pdf"),
        &provider,
    )?;
    source_tx.state = TransactionState::Moving;
    let outcome = dedupe_core::recovery::reconcile(
        &mut source_tx,
        &provider,
        &MemoryJournal::default(),
        &ControlToken::new(),
    )?;
    assert_eq!(outcome, dedupe_core::recovery::Reconciliation::SourceOnly);
    assert_eq!(source_tx.state, TransactionState::ReconciledSourceOnly);

    let both_source = fixture.write("both/source.pdf", b"two")?;
    let both_destination = fixture.write("both/destination.pdf", b"two")?;
    let mut both_tx = transaction_for(&both_source, both_destination, &provider)?;
    both_tx.state = TransactionState::RecoveryRequired;
    let outcome = dedupe_core::recovery::reconcile(
        &mut both_tx,
        &provider,
        &MemoryJournal::default(),
        &ControlToken::new(),
    )?;
    assert_eq!(outcome, dedupe_core::recovery::Reconciliation::Both);
    assert_eq!(both_tx.state, TransactionState::ReconciledBoth);

    let missing_source = fixture.write("missing/source.pdf", b"three")?;
    let mut missing_tx = transaction_for(
        &missing_source,
        fixture.path().join("missing/destination.pdf"),
        &provider,
    )?;
    std::fs::remove_file(missing_source)?;
    missing_tx.state = TransactionState::Moving;
    let result = dedupe_core::recovery::reconcile(
        &mut missing_tx,
        &provider,
        &MemoryJournal::default(),
        &ControlToken::new(),
    );
    assert!(result.is_err());
    assert_eq!(missing_tx.state, TransactionState::ReconciledMissing);
    Ok(())
}

fn transaction_for(
    source: &Path,
    destination: std::path::PathBuf,
    provider: &dyn MetadataProvider,
) -> Result<FileTransaction> {
    let control = ControlToken::new();
    let metadata = provider.snapshot(source)?;
    let identity = metadata
        .identity
        .clone()
        .ok_or_else(|| DedupeError::Safety("test requires physical identity".into()))?;
    let blake3 = full_hash::blake3_file(source, provider, &control)?;
    let sha256 = full_hash::sha256_file(source, provider, &control)?;
    let proven = ProvenFile {
        metadata,
        blake3,
        sha256,
    };
    Ok(FileTransaction {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        session_id: Some(Uuid::new_v4()),
        plan_item_id: Some(Uuid::new_v4()),
        kind: TransactionKind::Quarantine,
        state: TransactionState::Planned,
        source: proven.metadata.path.clone(),
        destination,
        identity,
        size_bytes: proven.metadata.size_bytes,
        blake3: proven.blake3.digest,
        sha256: proven.sha256.digest,
        snapshot_token: proven.metadata.snapshot_token,
        started_at: Utc::now(),
    })
}
