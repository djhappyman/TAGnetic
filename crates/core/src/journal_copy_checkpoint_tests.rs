//! Exercise SQLite's genuine SQLITE_FULL on the actual checkpoint connection.
use super::*;
use crate::plan::{ChangePlan, Executor, FileChange, PlanError};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
struct Full(SqliteJournal);
impl UndoJournal for Full {
    fn supports_copy_checkpoints(&self) -> bool {
        true
    }
    fn checkpoint_copy(
        &mut self,
        id: BatchId,
        s: &CopyStamp,
        p: Option<&CopyStamp>,
    ) -> Result<(), JournalError> {
        self.0.conn.execute_batch("CREATE TABLE pressure(data BLOB); CREATE TRIGGER full_checkpoint BEFORE INSERT ON copy_stamps WHEN NEW.moved=0 BEGIN INSERT INTO pressure VALUES(zeroblob(1048576)); END;").unwrap();
        let pages: i64 = self
            .0
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        self.0
            .conn
            .pragma_update(None, "max_page_count", pages)
            .unwrap();
        let error = self.0.checkpoint_copy(id, s, p).unwrap_err();
        assert!(error.to_string().to_lowercase().contains("full"), "{error}");
        Err(error)
    }
    fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
        self.0.record(b)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.0.batches()
    }
    fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
        self.0.finish(id, b)
    }
    fn retain_interrupted(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
        self.0.retain_interrupted(id, b)
    }
    fn prepare_copy_removal(
        &mut self,
        id: BatchId,
        t: &Path,
        u: &Path,
    ) -> Result<(), JournalError> {
        self.0.prepare_copy_removal(id, t, u)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.0.rollback(id)
    }
}
#[test]
fn sqlite_full_checkpoint_retains_payloads_and_history_without_completion() {
    static N: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tagrex-copy-full-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let source = root.join("source.wav");
    let target = root.join("copy.wav");
    let mut wav = b"RIFF".to_vec();
    wav.extend(356u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(320u32.to_le_bytes());
    wav.extend([9; 320]);
    std::fs::write(&source, &wav).unwrap();
    let db = root.join("journal.sqlite");
    let mut j = Full(SqliteJournal::open(&db).unwrap());
    let prior =
        Executor::apply(&ChangePlan::default(), &mut j, std::slice::from_ref(&root)).unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            rename_to: Some(target.clone()),
            copy: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut complete = false;
    let error = Executor::apply_with(
        &plan,
        &mut j,
        std::slice::from_ref(&root),
        &mut |p| complete |= p.completed.is_some(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(matches!(error, PlanError::RollbackIncomplete { .. }));
    assert!(!complete);
    assert_eq!(std::fs::read(&source).unwrap(), wav);
    assert_eq!(std::fs::read(&target).unwrap(), wav);
    assert_eq!(j.batches().unwrap().len(), 2);
    drop(j);
    let mut j = SqliteJournal::open(&db).unwrap();
    assert!(j.batch(prior.id).unwrap().is_some());
    let failed = j.batches().unwrap().remove(0);
    assert!(failed.interrupted);
    assert!(failed.copy_stamps.is_empty());
    assert!(Executor::undo(&mut j, failed.id, std::slice::from_ref(&root)).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), wav);
    assert!(j.batch(failed.id).unwrap().is_some());
}
