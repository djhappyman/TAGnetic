//! Independent public-Executor/SQLite staging review; all payloads are fixtures.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    journal::{AppliedBatch, BatchId, JournalError, MoveStage, SqliteJournal, UndoJournal},
    plan::{ChangePlan, Executor, FileChange},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-move-staging-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("journal.sqlite")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wav(sample: u8) -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend(356u32.to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(320u32.to_le_bytes());
    b.extend([sample; 320]);
    b
}
struct InterruptAfterIntent {
    journal: SqliteJournal,
    capture_owned: bool,
}
impl UndoJournal for InterruptAfterIntent {
    fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError> {
        self.journal.record(batch)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.journal.batches()
    }
    fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError> {
        self.journal.finish(id, batch)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.journal.rollback(id)
    }
    fn prepare_move_stage(&mut self, id: BatchId, stage: &MoveStage) -> Result<(), JournalError> {
        self.journal.prepare_move_stage(id, stage)?;
        // Simulate an interruption with material in the reserved entry. No
        // production fault hook or real external path is used.
        if self.capture_owned {
            std::fs::rename(&stage.from, &stage.path).unwrap();
        } else {
            std::fs::write(&stage.path, b"foreign fixture material").unwrap();
        }
        panic!("fixture interruption after durable stage intent");
    }
    fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
        self.journal.move_stages(id)
    }
}
fn pending_named(
    fixture: &Fixture,
    name: &str,
    capture_owned: bool,
) -> (BatchId, MoveStage, PathBuf, PathBuf, Vec<u8>) {
    let source = fixture.0.join(format!("{name}-source.wav"));
    let target = fixture.0.join(format!("{name}-target.wav"));
    let original = wav(0);
    std::fs::write(&source, &original).unwrap();
    let mut journal = InterruptAfterIntent {
        journal: SqliteJournal::open(&fixture.database()).unwrap(),
        capture_owned,
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Executor::apply(
            &ChangePlan {
                changes: vec![FileChange {
                    path: source.clone(),
                    rename_to: Some(target.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            &mut journal,
            std::slice::from_ref(&fixture.0),
        )
    }));
    assert!(result.is_err());
    let batch = journal.journal.batches().unwrap().remove(0);
    let stage = journal.journal.move_stages(batch.id).unwrap().remove(0);
    (batch.id, stage, source, target, original)
}
fn pending(
    fixture: &Fixture,
    capture_owned: bool,
) -> (BatchId, MoveStage, PathBuf, PathBuf, Vec<u8>) {
    pending_named(fixture, "single", capture_owned)
}

#[test]
fn conflicting_foreign_private_payload_is_preserved_before_any_recovery_move() {
    let fixture = Fixture::new();
    let (id, stage, source, target, original) = pending(&fixture, false);
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    assert!(Executor::undo(&mut journal, id, std::slice::from_ref(&fixture.0)).is_err());
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert!(!target.exists());
    assert_eq!(
        std::fs::read(stage.path).unwrap(),
        b"foreign fixture material"
    );
    assert!(journal.batch(id).unwrap().is_some());
}
#[test]
fn damaged_stage_source_cannot_redirect_foreign_material_to_an_unplanned_name() {
    let fixture = Fixture::new();
    let (id, stage, source, target, original) = pending(&fixture, false);
    let unplanned = fixture.0.join("never-approved.wav");
    let sql = rusqlite::Connection::open(fixture.database()).unwrap();
    sql.execute(
        "UPDATE move_stages SET source=?1 WHERE batch_id=?2",
        rusqlite::params![unplanned.to_str().unwrap(), id.0],
    )
    .unwrap();
    drop(sql);
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    let result = Executor::undo(&mut journal, id, std::slice::from_ref(&fixture.0));
    assert!(result.is_err());
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert!(!target.exists());
    assert!(
        !unplanned.exists(),
        "damaged staging source redirected foreign payload; outcome {result:?}"
    );
    assert_eq!(
        std::fs::read(&stage.path).unwrap(),
        b"foreign fixture material"
    );
    assert!(journal.batch(id).unwrap().is_some());
}
#[test]
fn damaged_stage_source_cannot_redirect_owned_material_to_an_unplanned_name() {
    let fixture = Fixture::new();
    let (id, stage, source, target, original) = pending(&fixture, true);
    let unplanned = fixture.0.join("never-approved.wav");
    let sql = rusqlite::Connection::open(fixture.database()).unwrap();
    sql.execute(
        "UPDATE move_stages SET source=?1 WHERE batch_id=?2",
        rusqlite::params![unplanned.to_str().unwrap(), id.0],
    )
    .unwrap();
    drop(sql);
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    let result = Executor::undo(&mut journal, id, std::slice::from_ref(&fixture.0));
    assert!(result.is_err());
    assert!(!source.exists());
    assert!(!target.exists());
    assert!(
        !unplanned.exists(),
        "damaged staging source moved approved payload outside its plan; outcome {result:?}"
    );
    assert_eq!(std::fs::read(&stage.path).unwrap(), original);
    assert!(journal.batch(id).unwrap().is_some());
}
#[test]
fn owned_pending_stage_restarts_and_removes_only_its_empty_private_directory() {
    let fixture = Fixture::new();
    let (id, stage, source, target, original) = pending(&fixture, true);
    let sibling = stage
        .path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ordinary-sibling");
    std::fs::create_dir(&sibling).unwrap();
    std::fs::write(sibling.join("keep"), b"independent fixture").unwrap();
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    Executor::undo(&mut journal, id, std::slice::from_ref(&fixture.0)).unwrap();
    assert_eq!(std::fs::read(source).unwrap(), original);
    assert!(!target.exists());
    assert!(!stage.path.parent().unwrap().exists());
    assert_eq!(
        std::fs::read(sibling.join("keep")).unwrap(),
        b"independent fixture"
    );
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn malformed_staged_identity_refuses_before_restoring_any_payload() {
    let fixture = Fixture::new();
    let (id, stage, source, target, original) = pending(&fixture, true);
    let sql = rusqlite::Connection::open(fixture.database()).unwrap();
    sql.execute(
        "UPDATE move_stages SET identity=x'FF00' WHERE batch_id=?1",
        [id.0],
    )
    .unwrap();
    drop(sql);
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    let result = Executor::undo(&mut journal, id, std::slice::from_ref(&fixture.0));
    assert!(result.is_err());
    assert!(
        !source.exists(),
        "malformed recovery proof still moved its payload: {result:?}"
    );
    assert!(!target.exists());
    assert_eq!(std::fs::read(stage.path).unwrap(), original);
    assert!(journal.batch(id).unwrap().is_some());
}

#[test]
fn complete_stage_collection_is_validated_before_recovering_an_earlier_valid_row() {
    let fixture = Fixture::new();
    let (first, stage_a, source_a, _, bytes_a) = pending_named(&fixture, "a", true);
    let (second, stage_b, source_b, _, bytes_b) = pending_named(&fixture, "b", true);
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    // Add a second independently produced native stage row to this batch,
    // without reproducing identity encoding. Its target is absent from the
    // immutable first plan: validate the complete collection before row one.
    let mut combined = journal.batch(first).unwrap().unwrap();
    let other = journal.batch(second).unwrap().unwrap();
    combined.plan.changes.extend(other.plan.changes);
    combined.copy_stamps.extend(other.copy_stamps);
    journal.retain_interrupted(first, &combined).unwrap();
    drop(journal);
    let sql = rusqlite::Connection::open(fixture.database()).unwrap();
    sql.execute(
        "UPDATE move_stages SET batch_id=?1 WHERE batch_id=?2",
        rusqlite::params![first.0, second.0],
    )
    .unwrap();
    let unplanned = fixture.0.join("unplanned-later-row.wav");
    sql.execute(
        "UPDATE move_stages SET source=?1 WHERE batch_id=?2 AND path=?3",
        rusqlite::params![
            unplanned.to_str().unwrap(),
            first.0,
            stage_b.path.to_str().unwrap()
        ],
    )
    .unwrap();
    drop(sql);
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    journal.rollback(second).unwrap();
    let result = Executor::undo(&mut journal, first, std::slice::from_ref(&fixture.0));
    assert!(result.is_err(), "invalid collection was reported complete: {result:?}; source_a={}, source_b={}, unplanned={}, history={}", source_a.exists(),source_b.exists(),unplanned.exists(),journal.batches().unwrap().len());
    assert!(
        !source_a.exists(),
        "valid earlier stage was mutated before invalid later context was refused: {result:?}"
    );
    assert!(!source_b.exists());
    assert!(!unplanned.exists());
    assert_eq!(std::fs::read(stage_a.path).unwrap(), bytes_a);
    assert_eq!(std::fs::read(stage_b.path).unwrap(), bytes_b);
    assert!(journal.batch(first).unwrap().is_some());
}
