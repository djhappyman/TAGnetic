//! Real SQLITE_FULL on the exact journal connection used for native publication.
use super::*;
use crate::model::TagEngine;
use crate::plan::{Executor, FieldChange};
use std::sync::atomic::{AtomicU64, Ordering};
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn flac() -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut comments = 0u32.to_le_bytes().to_vec();
    comments.extend(2u32.to_le_bytes());
    for item in ["TITLE=Saved", "ARTIST=A; B"] {
        comments.extend((item.len() as u32).to_le_bytes());
        comments.extend(item.as_bytes());
    }
    let n = comments.len();
    out.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(comments);
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out
}
struct Full {
    inner: SqliteJournal,
    fired: bool,
    kind: u8,
    db: PathBuf,
}
impl UndoJournal for Full {
    fn native_tags(&mut self) -> Option<&mut dyn NativeJournal> {
        Some(self)
    }
    fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
        self.inner.record(b)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.inner.batches()
    }
    fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
        self.inner.batch(id)
    }
    fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
        self.inner.finish(id, b)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.inner.rollback(id)
    }
}
impl NativeJournal for Full {
    fn store(&mut self) -> Result<Store, JournalError> {
        NativeJournal::store(&mut self.inner)
    }
    fn record_native(&mut self, b: &AppliedBatch, i: &[Intent]) -> Result<BatchId, JournalError> {
        NativeJournal::record_native(&mut self.inner, b, i)
    }
    fn entries(&self, b: &AppliedBatch) -> Result<Entries, JournalError> {
        NativeJournal::entries(&self.inner, b)
    }
    fn begin(&mut self, b: &AppliedBatch, i: usize, o: u8) -> Result<String, JournalError> {
        NativeJournal::begin(&mut self.inner, b, i, o)
    }
    fn checkpoint(
        &mut self,
        b: &AppliedBatch,
        position: (usize, u8),
        r: &Record,
        expected: &Intent,
        receipts: Option<&[u8]>,
        done: bool,
    ) -> Result<(), JournalError> {
        let (i, o) = position;
        if !self.fired && o == 0 && r.phase() == media::Phase::Prepared {
            self.fired = true;
            self.inner
                .conn
                .execute_batch("CREATE TABLE native_pressure(data BLOB);")
                .unwrap();
            let locker = if self.kind == 1 {
                self.inner
                    .conn
                    .busy_timeout(std::time::Duration::from_millis(25))
                    .unwrap();
                let conn = rusqlite::Connection::open(&self.db).unwrap();
                conn.execute_batch("BEGIN EXCLUSIVE;").unwrap();
                Some(conn)
            } else {
                None
            };
            if self.kind == 0 {
                self.inner.conn.execute_batch("CREATE TRIGGER full_native BEFORE UPDATE OF record ON native_tag_files BEGIN INSERT INTO native_pressure VALUES(zeroblob(1048576));END;").unwrap();
                let pages: i64 = self
                    .inner
                    .conn
                    .query_row("PRAGMA page_count", [], |r| r.get(0))
                    .unwrap();
                self.inner
                    .conn
                    .pragma_update(None, "max_page_count", pages)
                    .unwrap();
            } else if self.kind == 2 {
                self.inner.conn.execute_batch("CREATE TRIGGER abort_native BEFORE UPDATE OF record ON native_tag_files BEGIN INSERT INTO native_pressure VALUES(zeroblob(64));SELECT RAISE(ABORT,'owned native abort');END;").unwrap();
            }
            let error =
                NativeJournal::checkpoint(&mut self.inner, b, (i, o), r, expected, receipts, done)
                    .unwrap_err();
            if let Some(locker) = locker {
                locker.execute_batch("ROLLBACK;").unwrap();
            }
            let expected_error = ["full", "locked", "owned native abort"][self.kind as usize];
            assert!(
                error.to_string().to_lowercase().contains(expected_error),
                "{error}"
            );
            return Err(error);
        }
        NativeJournal::checkpoint(&mut self.inner, b, (i, o), r, expected, receipts, done)
    }
    fn confirm(
        &self,
        b: &AppliedBatch,
        position: (usize, u8),
        r: &Record,
        expected: &Intent,
        receipts: &[u8],
    ) -> Result<(), JournalError> {
        NativeJournal::confirm(&self.inner, b, position, r, expected, receipts)
    }
    fn finish_native(&mut self, b: &AppliedBatch) -> Result<(), JournalError> {
        NativeJournal::finish_native(&mut self.inner, b)
    }
    fn retain_native(&mut self, b: &AppliedBatch) -> Result<(), JournalError> {
        NativeJournal::retain_native(&mut self.inner, b)
    }
    fn remove_native(&mut self, id: BatchId) -> Result<(), JournalError> {
        NativeJournal::remove_native(&mut self.inner, id)
    }
}

#[test]
fn native_entries_use_one_sqlite_read_snapshot() {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "tagrex-native-read-snapshot-{}",
        std::process::id()
    )));
    std::fs::create_dir(&fixture.0).unwrap();
    let path = fixture.0.join("track.flac");
    std::fs::write(&path, flac()).unwrap();
    let db = fixture.0.join("journal.sqlite");
    let mut journal = SqliteJournal::open(&db).unwrap();
    journal
        .conn
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: path.clone(),
            tag_changes: vec![FieldChange {
                field: TagField::Title,
                old: Some("Saved".into()),
                new: Some("Changed".into()),
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap();
    assert!(load_entries(&journal.conn, &batch).is_err());
    assert_eq!(
        NativeJournal::entries(&journal, &batch).unwrap().rows.len(),
        1
    );
    assert!(journal.conn.is_autocommit());

    let read = journal.conn.unchecked_transaction().unwrap();
    check_batch_header(&read, &batch).unwrap();
    let mut other = SqliteJournal::open(&db).unwrap();
    UndoJournal::rollback(&mut other, batch.id).unwrap();

    assert_eq!(load_entries(&read, &batch).unwrap().rows.len(), 1);
    read.commit().unwrap();
    assert!(NativeJournal::entries(&journal, &batch).is_err());
    assert!(journal.conn.is_autocommit());
}

#[test]
fn real_native_sqlite_full_busy_and_abort_preserve_media_prior_history_and_no_completion() {
    for kind in 0..3 {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let fixture = Fixture(std::env::temp_dir().join(format!(
            "tagrex-native-real-full-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        std::fs::create_dir(&fixture.0).unwrap();
        let path = fixture.0.join("track.flac");
        std::fs::write(&path, flac()).unwrap();
        let db = fixture.0.join("journal.sqlite");
        let mut inner = SqliteJournal::open(&db).unwrap();
        let plan = |field: TagField, new: &str| ChangePlan {
            changes: vec![FileChange {
                path: path.clone(),
                tag_changes: vec![FieldChange {
                    old: TagEngine::read(&path).unwrap().tags.get(&field).cloned(),
                    field,
                    new: Some(new.into()),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let prior = Executor::apply(
            &plan(TagField::Title, "Prior"),
            &mut inner,
            std::slice::from_ref(&fixture.0),
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let mut journal = Full {
            inner,
            fired: false,
            kind,
            db: db.clone(),
        };
        let mut completed = 0;
        let result = Executor::apply_with(
            &plan(TagField::Artist, "New"),
            &mut journal,
            std::slice::from_ref(&fixture.0),
            &mut |p| {
                if p.completed.is_some() {
                    completed += 1;
                }
            },
            &std::sync::atomic::AtomicBool::new(false),
        );
        assert!(journal.fired);
        assert!(result.is_err());
        assert_eq!(completed, 0);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let batches = journal.batches().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].id, prior.id);
        assert_eq!(
            journal
                .inner
                .conn
                .query_row("SELECT count(*) FROM native_pressure", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
