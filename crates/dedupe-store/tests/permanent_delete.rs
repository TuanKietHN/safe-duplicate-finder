//! Permanent-deletion integration and interruption-recovery safety gate.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use chrono::{Duration, Utc};
use dedupe_core::{
    DedupeError,
    control::ControlToken,
    full_hash,
    model::{ComparisonMode, FileMetadataSnapshot, ProvenFile},
    permanent_delete::{
        self, PermanentDeleteBatch, PermanentDeleteBatchState, PermanentDeleteItem,
        PermanentDeleteItemState, PermanentDeleteMode,
    },
    ports::{MetadataProvider, PermanentDeleteJournal, SafeDeleter},
    quarantine,
};
use dedupe_platform::PlatformFileSystem;
use dedupe_store::{
    Database, PermanentDeleteRepository, ProjectRepository, SqlitePermanentDeleteJournal,
    SqliteTransactionJournal, TransactionRepository,
};
use dedupe_testkit::Fixture;
use uuid::Uuid;

struct QuarantinedFixture {
    database: Database,
    project: Uuid,
    entry: Uuid,
    quarantine_path: std::path::PathBuf,
}

struct AddedQuarantine {
    entry: Uuid,
    quarantine_path: std::path::PathBuf,
}

#[derive(Debug, Default)]
struct FixtureDeleter;

impl SafeDeleter for FixtureDeleter {
    fn delete_exact(&self, expected: &FileMetadataSnapshot) -> dedupe_core::Result<()> {
        std::fs::remove_file(&expected.path).map_err(|error| {
            DedupeError::io("fixture delete exact preflight path", &expected.path, error)
        })
    }
}

fn quarantine_one(
    fixture: &Fixture,
    label: &str,
    bytes: &[u8],
) -> Result<QuarantinedFixture, Box<dyn std::error::Error>> {
    let database = Database::open(&fixture.path().join(format!("{label}.db")), &[])?;
    let project_name = format!("Delete {label}");
    let project =
        ProjectRepository::new(database.clone()).create(&project_name, ComparisonMode::Strict)?;
    let added = quarantine_into(fixture, &database, project, label, bytes)?;
    Ok(QuarantinedFixture {
        database,
        project,
        entry: added.entry,
        quarantine_path: added.quarantine_path,
    })
}

fn quarantine_into(
    fixture: &Fixture,
    database: &Database,
    project: Uuid,
    label: &str,
    bytes: &[u8],
) -> Result<AddedQuarantine, Box<dyn std::error::Error>> {
    let source = fixture.write(&format!("{label}-source/document.bin"), bytes)?;
    let quarantine_path = fixture
        .path()
        .join(format!("{label}-quarantine/document.bin"));
    let provider = PlatformFileSystem;
    let control = ControlToken::new();
    let proven = ProvenFile {
        metadata: provider.snapshot(&source)?,
        blake3: full_hash::blake3_file(&source, &provider, &control)?,
        sha256: full_hash::sha256_file(&source, &provider, &control)?,
    };
    let mut transaction = quarantine::planned_transaction(
        project,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &proven,
        quarantine_path.clone(),
    )?;
    transaction.session_id = None;
    transaction.plan_item_id = None;
    let transaction_journal = SqliteTransactionJournal::new(
        database.clone(),
        fixture.path().join(format!("{label}-transactions.jsonl")),
    )?;
    quarantine::execute(
        &mut transaction,
        &provider,
        &provider,
        &transaction_journal,
        &control,
    )?;
    let entry = TransactionRepository::new(database.clone())
        .list_quarantine(project)?
        .into_iter()
        .find(|entry| entry.quarantine_path == quarantine_path)
        .ok_or("new quarantine entry missing")?
        .id;
    Ok(AddedQuarantine {
        entry,
        quarantine_path,
    })
}

fn expire_retention(database: &Database, entry: Uuid) -> rusqlite::Result<usize> {
    database.connection().execute(
        "UPDATE quarantine_entries SET retain_until=?1 WHERE id=?2",
        rusqlite::params![
            (Utc::now() - Duration::days(1)).to_rfc3339(),
            entry.to_string()
        ],
    )
}

#[test]
fn rejects_non_registry_retention_token_and_phrase_then_deletes_only_selected_quarantine()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let quarantined = quarantine_one(&fixture, "gate", b"irreversible fixture bytes")?;
    let unrelated_source = fixture.write("outside/source-document.bin", b"never delete source")?;
    let repository = PermanentDeleteRepository::new(quarantined.database.clone());
    let journal_path = fixture.path().join("permanent-delete.jsonl");
    let journal = SqlitePermanentDeleteJournal::new(quarantined.database.clone(), &journal_path)?;

    // The API accepts only registry UUIDs, never a source path. An unknown UUID fails closed.
    assert!(matches!(
        repository.selected_entries(&[Uuid::new_v4()]),
        Err(DedupeError::Safety(_))
    ));
    let retained = repository.selected_entries(&[quarantined.entry])?;
    assert!(matches!(
        permanent_delete::prepare(retained, &journal, Utc::now()),
        Err(DedupeError::Safety(_))
    ));
    assert!(quarantined.quarantine_path.exists());
    assert!(unrelated_source.exists());

    expire_retention(&quarantined.database, quarantined.entry)?;
    let entries = repository.selected_entries(&[quarantined.entry])?;
    let expected_bytes = entries[0].size_bytes;
    let challenge = permanent_delete::prepare(entries, &journal, Utc::now())?;
    assert_eq!(challenge.entry_count, 1);
    assert_eq!(challenge.total_bytes, expected_bytes);
    assert!(challenge.confirmation_phrase.contains("1 TỆP ĐÃ CÁCH LY"));
    assert!(
        challenge
            .confirmation_phrase
            .contains(&format!("({expected_bytes} BYTE)"))
    );

    let automatic: i64 = quarantined.database.connection().query_row(
        "SELECT automatic_permanent_delete FROM projects WHERE id=?1",
        [quarantined.project.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(automatic, 0);
    assert!(quarantined.quarantine_path.exists());

    let provider = PlatformFileSystem;
    assert!(matches!(
        permanent_delete::execute(
            challenge.batch_id,
            "wrong-token",
            &challenge.confirmation_phrase,
            &journal,
            &provider,
            &provider,
            &ControlToken::new(),
            Utc::now(),
        ),
        Err(DedupeError::Safety(_))
    ));
    assert!(quarantined.quarantine_path.exists());
    assert!(matches!(
        permanent_delete::execute(
            challenge.batch_id,
            &challenge.token,
            "XÓA VĨNH VIỄN",
            &journal,
            &provider,
            &provider,
            &ControlToken::new(),
            Utc::now(),
        ),
        Err(DedupeError::Safety(_))
    ));
    assert!(quarantined.quarantine_path.exists());

    let outcome = permanent_delete::execute(
        challenge.batch_id,
        &challenge.token,
        &challenge.confirmation_phrase,
        &journal,
        &provider,
        &FixtureDeleter,
        &ControlToken::new(),
        Utc::now(),
    )?;
    assert_eq!(outcome.deleted_entries, 1);
    assert_eq!(outcome.deleted_bytes, expected_bytes);
    assert!(!quarantined.quarantine_path.exists());
    assert!(unrelated_source.exists());
    assert_eq!(
        TransactionRepository::new(quarantined.database.clone())
            .list_quarantine(quarantined.project)?[0]
            .state,
        "deleted"
    );
    assert!(
        TransactionRepository::new(quarantined.database.clone())
            .verified_quarantine_transaction(quarantined.entry)
            .is_err()
    );
    assert!(journal_path.metadata()?.len() > 0);
    Ok(())
}

#[test]
fn immediate_mode_deletes_a_selected_retained_entry_only_after_its_exact_challenge()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let quarantined = quarantine_one(&fixture, "immediate", b"explicit immediate deletion")?;
    let unrelated = fixture.write("immediate-outside/keep.bin", b"must remain")?;
    let repository = PermanentDeleteRepository::new(quarantined.database.clone());
    let journal = SqlitePermanentDeleteJournal::new(
        quarantined.database.clone(),
        fixture.path().join("immediate-delete.jsonl"),
    )?;

    let retained = repository.selected_entries(&[quarantined.entry])?;
    assert!(retained[0].retain_until > Utc::now());
    let challenge = permanent_delete::prepare_immediate(retained, &journal, Utc::now())?;
    assert_eq!(challenge.mode, PermanentDeleteMode::Immediate);
    assert!(
        challenge
            .confirmation_phrase
            .starts_with("XÓA NGAY VĨNH VIỄN")
    );
    assert!(quarantined.quarantine_path.exists());
    assert!(unrelated.exists());

    assert!(matches!(
        permanent_delete::execute(
            challenge.batch_id,
            &challenge.token,
            &challenge.confirmation_phrase.replacen("XÓA NGAY ", "", 1),
            &journal,
            &PlatformFileSystem,
            &FixtureDeleter,
            &ControlToken::new(),
            Utc::now(),
        ),
        Err(DedupeError::Safety(_))
    ));
    assert!(quarantined.quarantine_path.exists());

    let outcome = permanent_delete::execute(
        challenge.batch_id,
        &challenge.token,
        &challenge.confirmation_phrase,
        &journal,
        &PlatformFileSystem,
        &FixtureDeleter,
        &ControlToken::new(),
        Utc::now(),
    )?;
    assert_eq!(outcome.deleted_entries, 1);
    assert!(!quarantined.quarantine_path.exists());
    assert!(unrelated.exists());
    Ok(())
}

#[test]
fn unused_short_lived_token_expires_without_deleting_or_reserving_the_entry()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let quarantined = quarantine_one(&fixture, "expiry", b"short lived token")?;
    expire_retention(&quarantined.database, quarantined.entry)?;
    let journal = SqlitePermanentDeleteJournal::new(
        quarantined.database.clone(),
        fixture.path().join("expiry.jsonl"),
    )?;
    let prepared_at = Utc::now();
    let challenge = permanent_delete::prepare_with_ttl(
        PermanentDeleteRepository::new(quarantined.database.clone())
            .selected_entries(&[quarantined.entry])?,
        &journal,
        prepared_at,
        Duration::seconds(1),
    )?;
    assert!(matches!(
        permanent_delete::execute(
            challenge.batch_id,
            &challenge.token,
            &challenge.confirmation_phrase,
            &journal,
            &PlatformFileSystem,
            &FixtureDeleter,
            &ControlToken::new(),
            prepared_at + Duration::seconds(2),
        ),
        Err(DedupeError::Safety(_))
    ));
    assert!(quarantined.quarantine_path.exists());
    assert_eq!(
        journal.load_batch(challenge.batch_id)?.state,
        PermanentDeleteBatchState::Expired
    );
    assert_eq!(
        TransactionRepository::new(quarantined.database).list_quarantine(quarantined.project)?[0]
            .permanent_delete_state,
        "active"
    );
    Ok(())
}

#[derive(Debug)]
struct FailDeletedProjectionOnce {
    inner: SqlitePermanentDeleteJournal,
    fail: AtomicBool,
}

impl FailDeletedProjectionOnce {
    fn new(inner: SqlitePermanentDeleteJournal) -> Self {
        Self {
            inner,
            fail: AtomicBool::new(true),
        }
    }
}

impl PermanentDeleteJournal for FailDeletedProjectionOnce {
    fn create_batch(&self, batch: &PermanentDeleteBatch) -> dedupe_core::Result<()> {
        self.inner.create_batch(batch)
    }

    fn load_batch(&self, batch_id: Uuid) -> dedupe_core::Result<PermanentDeleteBatch> {
        self.inner.load_batch(batch_id)
    }

    fn transition_batch(
        &self,
        batch: &PermanentDeleteBatch,
        next: PermanentDeleteBatchState,
        error: Option<&str>,
    ) -> dedupe_core::Result<()> {
        self.inner.transition_batch(batch, next, error)
    }

    fn transition_item(
        &self,
        batch: &PermanentDeleteBatch,
        item: &PermanentDeleteItem,
        next: PermanentDeleteItemState,
        error: Option<&str>,
    ) -> dedupe_core::Result<()> {
        if next == PermanentDeleteItemState::Deleted && self.fail.swap(false, Ordering::SeqCst) {
            return Err(DedupeError::Durability(
                "injected projection failure after delete system call".into(),
            ));
        }
        self.inner.transition_item(batch, item, next, error)
    }
}

#[test]
fn durable_deleting_intent_reconciles_post_delete_interruption_and_is_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let quarantined = quarantine_one(&fixture, "interrupt", b"recover deletion exactly")?;
    expire_retention(&quarantined.database, quarantined.entry)?;
    let manifest = fixture.path().join("interrupt-permanent-delete.jsonl");
    let inner = SqlitePermanentDeleteJournal::new(quarantined.database.clone(), &manifest)?;
    let journal = FailDeletedProjectionOnce::new(inner);
    let entries = PermanentDeleteRepository::new(quarantined.database.clone())
        .selected_entries(&[quarantined.entry])?;
    let bytes = entries[0].size_bytes;
    let challenge = permanent_delete::prepare(entries, &journal, Utc::now())?;
    let provider = PlatformFileSystem;

    let interrupted = permanent_delete::execute(
        challenge.batch_id,
        &challenge.token,
        &challenge.confirmation_phrase,
        &journal,
        &provider,
        &FixtureDeleter,
        &ControlToken::new(),
        Utc::now(),
    );
    assert!(matches!(interrupted, Err(DedupeError::Durability(_))));
    assert!(!quarantined.quarantine_path.exists());
    let item_state: String = quarantined.database.connection().query_row(
        "SELECT status FROM permanent_delete_items WHERE batch_id=?1 AND entry_id=?2",
        rusqlite::params![
            challenge.batch_id.to_string(),
            quarantined.entry.to_string()
        ],
        |row| row.get(0),
    )?;
    assert_eq!(item_state, "deleting");
    let manifest_before_retry = std::fs::read_to_string(&manifest)?;
    assert!(manifest_before_retry.contains("\"to\":\"deleting\""));

    let recovered = permanent_delete::execute(
        challenge.batch_id,
        &challenge.token,
        &challenge.confirmation_phrase,
        &journal,
        &provider,
        &FixtureDeleter,
        &ControlToken::new(),
        Utc::now(),
    )?;
    assert_eq!(recovered.deleted_entries, 1);
    assert_eq!(recovered.deleted_bytes, bytes);
    let repeated = permanent_delete::execute(
        challenge.batch_id,
        &challenge.token,
        &challenge.confirmation_phrase,
        &journal,
        &provider,
        &FixtureDeleter,
        &ControlToken::new(),
        Utc::now(),
    )?;
    assert_eq!(repeated, recovered);

    let connection = quarantined.database.connection();
    let event_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM permanent_delete_events WHERE batch_id=?1",
        [challenge.batch_id.to_string()],
        |row| row.get(0),
    )?;
    let audit_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM audit_events WHERE project_id=?1
         AND event_type LIKE 'permanent_delete.%'",
        [quarantined.project.to_string()],
        |row| row.get(0),
    )?;
    assert!(event_count >= 4);
    assert!(audit_count >= 4);
    assert!(
        connection
            .execute(
                "UPDATE permanent_delete_events SET to_status='rewritten' WHERE batch_id=?1",
                [challenge.batch_id.to_string()],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "DELETE FROM permanent_delete_events WHERE batch_id=?1",
                [challenge.batch_id.to_string()],
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn every_selected_entry_is_preflighted_before_the_first_delete()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let database = Database::open(&fixture.path().join("all-preflight.db"), &[])?;
    let project =
        ProjectRepository::new(database.clone()).create("All preflight", ComparisonMode::Strict)?;
    let first = quarantine_into(
        &fixture,
        &database,
        project,
        "all-preflight-first",
        b"first",
    )?;
    let second = quarantine_into(
        &fixture,
        &database,
        project,
        "all-preflight-second",
        b"second",
    )?;
    expire_retention(&database, first.entry)?;
    expire_retention(&database, second.entry)?;
    let journal = SqlitePermanentDeleteJournal::new(
        database.clone(),
        fixture.path().join("all-preflight.jsonl"),
    )?;
    let challenge = permanent_delete::prepare(
        PermanentDeleteRepository::new(database.clone())
            .selected_entries(&[first.entry, second.entry])?,
        &journal,
        Utc::now(),
    )?;
    std::fs::write(&second.quarantine_path, b"change")?;
    let provider = PlatformFileSystem;
    assert!(
        permanent_delete::execute(
            challenge.batch_id,
            &challenge.token,
            &challenge.confirmation_phrase,
            &journal,
            &provider,
            &provider,
            &ControlToken::new(),
            Utc::now(),
        )
        .is_err()
    );
    assert!(first.quarantine_path.exists());
    assert!(second.quarantine_path.exists());
    let deleting_items: i64 = database.connection().query_row(
        "SELECT COUNT(*) FROM permanent_delete_items
         WHERE batch_id=?1 AND status='deleting'",
        [challenge.batch_id.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(deleting_items, 0);
    Ok(())
}

#[derive(Debug, Default)]
struct RejectFirstDelete {
    calls: AtomicUsize,
}

impl SafeDeleter for RejectFirstDelete {
    fn delete_exact(&self, expected: &FileMetadataSnapshot) -> dedupe_core::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(DedupeError::io(
            "injected locked-file delete",
            &expected.path,
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected lock"),
        ))
    }
}

#[test]
fn one_delete_failure_stops_the_batch_before_any_different_path()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let database = Database::open(&fixture.path().join("stop-batch.db"), &[])?;
    let project =
        ProjectRepository::new(database.clone()).create("Stop batch", ComparisonMode::Strict)?;
    let first = quarantine_into(&fixture, &database, project, "stop-first", b"first stop")?;
    let second = quarantine_into(&fixture, &database, project, "stop-second", b"second stop")?;
    expire_retention(&database, first.entry)?;
    expire_retention(&database, second.entry)?;
    let journal = SqlitePermanentDeleteJournal::new(
        database.clone(),
        fixture.path().join("stop-batch.jsonl"),
    )?;
    let challenge = permanent_delete::prepare(
        PermanentDeleteRepository::new(database.clone())
            .selected_entries(&[first.entry, second.entry])?,
        &journal,
        Utc::now(),
    )?;
    let reject = RejectFirstDelete::default();
    assert!(
        permanent_delete::execute(
            challenge.batch_id,
            &challenge.token,
            &challenge.confirmation_phrase,
            &journal,
            &PlatformFileSystem,
            &reject,
            &ControlToken::new(),
            Utc::now(),
        )
        .is_err()
    );
    assert_eq!(reject.calls.load(Ordering::SeqCst), 1);
    assert!(first.quarantine_path.exists());
    assert!(second.quarantine_path.exists());
    let states = journal.load_batch(challenge.batch_id)?;
    assert_eq!(states.state, PermanentDeleteBatchState::RecoveryRequired);
    assert_eq!(
        states
            .items
            .iter()
            .filter(|item| item.state == PermanentDeleteItemState::Failed)
            .count(),
        1
    );
    assert_eq!(
        states
            .items
            .iter()
            .filter(|item| item.state == PermanentDeleteItemState::Planned)
            .count(),
        1
    );
    let failed_path = states
        .items
        .iter()
        .find(|item| item.state == PermanentDeleteItemState::Failed)
        .ok_or("failed delete item missing")?
        .entry
        .quarantine_path
        .clone();
    std::fs::write(&failed_path, b"changed after failed delete")?;
    for _ in 0..2 {
        assert!(matches!(
            permanent_delete::execute(
                challenge.batch_id,
                &challenge.token,
                &challenge.confirmation_phrase,
                &journal,
                &PlatformFileSystem,
                &FixtureDeleter,
                &ControlToken::new(),
                Utc::now(),
            ),
            Err(DedupeError::Safety(_))
        ));
    }
    assert!(first.quarantine_path.exists());
    assert!(second.quarantine_path.exists());
    Ok(())
}

#[derive(Debug, Default)]
struct DeleteThenReportError;

impl SafeDeleter for DeleteThenReportError {
    fn delete_exact(&self, expected: &FileMetadataSnapshot) -> dedupe_core::Result<()> {
        FixtureDeleter.delete_exact(expected)?;
        Err(DedupeError::io(
            "injected ambiguous post-delete result",
            &expected.path,
            std::io::Error::other("injected ambiguous result"),
        ))
    }
}

#[test]
fn failed_state_with_missing_path_reconciles_only_after_prior_durable_intent()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let quarantined = quarantine_one(&fixture, "ambiguous", b"ambiguous system call")?;
    expire_retention(&quarantined.database, quarantined.entry)?;
    let journal = SqlitePermanentDeleteJournal::new(
        quarantined.database.clone(),
        fixture.path().join("ambiguous.jsonl"),
    )?;
    let challenge = permanent_delete::prepare(
        PermanentDeleteRepository::new(quarantined.database.clone())
            .selected_entries(&[quarantined.entry])?,
        &journal,
        Utc::now(),
    )?;
    assert!(
        permanent_delete::execute(
            challenge.batch_id,
            &challenge.token,
            &challenge.confirmation_phrase,
            &journal,
            &PlatformFileSystem,
            &DeleteThenReportError,
            &ControlToken::new(),
            Utc::now(),
        )
        .is_err()
    );
    assert!(!quarantined.quarantine_path.exists());
    assert_eq!(
        journal.load_batch(challenge.batch_id)?.items[0].state,
        PermanentDeleteItemState::Failed
    );
    let outcome = permanent_delete::execute(
        challenge.batch_id,
        &challenge.token,
        &challenge.confirmation_phrase,
        &journal,
        &PlatformFileSystem,
        &FixtureDeleter,
        &ControlToken::new(),
        Utc::now(),
    )?;
    assert_eq!(outcome.deleted_entries, 1);
    Ok(())
}
