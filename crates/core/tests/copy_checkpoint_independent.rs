//! Fresh public-API copy checkpoint review. Every write targets owned fixtures.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tagrex_core::{
    journal::{
        AppliedBatch, BatchId, CopyStamp, JournalError, SqliteJournal, UndoJournal, VecJournal,
    },
    model::TagField,
    plan::{ApplyStep, ChangePlan, Executor, FieldChange, FileChange, UndoMode},
};
const FRAME: &[u8] = &[
    0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
];
fn flac() -> Vec<u8> {
    let mut bytes = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    bytes.extend(info);
    let mut comments = 0u32.to_le_bytes().to_vec();
    comments.extend(2u32.to_le_bytes());
    for entry in ["ARTIST=Before", "TITLE=Keep 尾"] {
        comments.extend((entry.len() as u32).to_le_bytes());
        comments.extend(entry.as_bytes());
    }
    bytes.extend([0x84, 0, (comments.len() >> 8) as u8, comments.len() as u8]);
    bytes.extend(comments);
    bytes.extend(FRAME);
    bytes
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-copy-independent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        for index in 0..3 {
            fs::write(root.join(format!("source-{index}.flac")), flac()).unwrap();
            fs::write(
                root.join(format!("source-{index}.cue")),
                format!("cue {index} 尾"),
            )
            .unwrap();
        }
        Self(root)
    }
    fn db(&self) -> PathBuf {
        self.0.join("journal.sqlite")
    }
    fn source(&self, i: usize) -> PathBuf {
        self.0.join(format!("source-{i}.flac"))
    }
    fn target(&self, i: usize) -> PathBuf {
        self.0.join(format!("copy-{i}.flac"))
    }
    fn cue(&self, i: usize) -> PathBuf {
        self.0.join(format!("copy-{i}.cue"))
    }
    fn plan(&self, count: usize, tags: bool) -> ChangePlan {
        ChangePlan {
            changes: (0..count)
                .map(|i| FileChange {
                    path: self.source(i),
                    rename_to: Some(self.target(i)),
                    copy: true,
                    sidecar_renames: vec![(self.0.join(format!("source-{i}.cue")), self.cue(i))],
                    tag_changes: if tags {
                        vec![FieldChange {
                            field: TagField::Artist,
                            old: Some("Before".into()),
                            new: Some("After".into()),
                        }]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }
    fn sources_unchanged(&self) {
        for index in 0..3 {
            assert_eq!(fs::read(self.source(index)).unwrap(), flac());
            assert_eq!(
                fs::read(self.0.join(format!("source-{index}.cue"))).unwrap(),
                format!("cue {index} 尾").as_bytes()
            );
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn raw_artist(path: &Path) -> String {
    let bytes = fs::read(path).unwrap();
    let mut at = 4;
    let mut artist = None;
    loop {
        let kind = bytes[at];
        let len = u32::from_be_bytes([0, bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let block = &bytes[at + 4..at + 4 + len];
        if kind & 127 == 4 {
            let number = |cursor: &mut usize| {
                let n =
                    u32::from_le_bytes(block[*cursor..*cursor + 4].try_into().unwrap()) as usize;
                *cursor += 4;
                n
            };
            let mut cursor = 0;
            let vendor = number(&mut cursor);
            cursor += vendor;
            let count = number(&mut cursor);
            let mut title = false;
            for _ in 0..count {
                let n = number(&mut cursor);
                let text = std::str::from_utf8(&block[cursor..cursor + n]).unwrap();
                cursor += n;
                if let Some(value) = text.strip_prefix("ARTIST=") {
                    artist = Some(value.to_owned());
                }
                title |= text == "TITLE=Keep 尾";
            }
            assert!(title);
        }
        at += 4 + len;
        if kind & 128 != 0 {
            break;
        }
    }
    assert_eq!(&bytes[at..], FRAME);
    artist.unwrap()
}
#[test]
fn same_inode_external_audio_change_before_metadata_is_not_adopted_by_refresh() {
    let f = Fixture::new();
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    let mut external = None;
    let mut tags_completed = false;
    let result = Executor::apply_with(
        &f.plan(1, true),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |progress| {
            if progress.completed == Some(ApplyStep::Transfer)
                && progress.path == f.0.join("source-0.cue")
            {
                let mut bytes = fs::read(f.target(0)).unwrap();
                *bytes.last_mut().unwrap() ^= 1;
                fs::write(f.target(0), &bytes).unwrap();
                external = Some(bytes);
            }
            tags_completed |= progress.completed == Some(ApplyStep::Tags);
        },
        &AtomicBool::new(false),
    );
    assert!(result.is_err());
    assert!(!tags_completed);
    assert_eq!(fs::read(f.target(0)).unwrap(), external.unwrap());
    assert!(fs::read(f.target(0))
        .unwrap()
        .windows(b"ARTIST=Before".len())
        .any(|w| w == b"ARTIST=Before"));
    f.sources_unchanged();
    let failed = journal.batches().unwrap().remove(0);
    assert!(failed.interrupted);
    drop(journal);
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        assert!(Executor::undo_with_mode(
            &mut journal,
            failed.id,
            std::slice::from_ref(&f.0),
            mode,
            &mut |_| {}
        )
        .is_err());
        assert!(f.target(0).exists());
        assert!(journal.batch(failed.id).unwrap().is_some());
    }
}
#[test]
fn late_sidecar_external_change_is_checked_before_any_copy_removal_in_every_mode() {
    let f = Fixture::new();
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    let batch =
        Executor::apply(&f.plan(3, false), &mut journal, std::slice::from_ref(&f.0)).unwrap();
    fs::write(f.cue(2), b"new external sidecar").unwrap();
    drop(journal);
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        let mut calls = 0;
        assert!(Executor::undo_with_mode(
            &mut journal,
            batch.id,
            std::slice::from_ref(&f.0),
            mode,
            &mut |_| calls += 1
        )
        .is_err());
        assert_eq!(calls, 0);
        for index in 0..3 {
            assert_eq!(fs::read(f.target(index)).unwrap(), flac());
            assert!(f.cue(index).exists());
        }
        assert_eq!(fs::read(f.cue(2)).unwrap(), b"new external sidecar");
        assert_eq!(
            journal.batch(batch.id).unwrap().unwrap().copy_stamps,
            batch.copy_stamps
        );
    }
    f.sources_unchanged();
}
#[test]
fn malformed_later_native_role_cannot_be_bypassed_by_undo_modes() {
    for variant in ["null", "legacy", "directory", "future"] {
        let f = Fixture::new();
        let mut journal = SqliteJournal::open(&f.db()).unwrap();
        let batch =
            Executor::apply(&f.plan(3, false), &mut journal, std::slice::from_ref(&f.0)).unwrap();
        let mut identity = batch
            .copy_stamps
            .iter()
            .find(|s| s.path == f.cue(2))
            .unwrap()
            .identity
            .clone()
            .unwrap();
        match variant {
            "legacy" => identity[5] = b'1',
            "directory" => *identity.last_mut().unwrap() = 2,
            "future" => identity[5] = b'9',
            _ => {}
        }
        let conn = rusqlite::Connection::open(f.db()).unwrap();
        let value = if variant == "null" {
            None
        } else {
            Some(identity)
        };
        conn.execute(
            "UPDATE copy_stamps SET identity=?1 WHERE path=?2",
            rusqlite::params![value, f.cue(2).to_str().unwrap()],
        )
        .unwrap();
        drop(conn);
        drop(journal);
        let mut journal = SqliteJournal::open(&f.db()).unwrap();
        for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
            let mut calls = 0;
            assert!(
                Executor::undo_with_mode(
                    &mut journal,
                    batch.id,
                    std::slice::from_ref(&f.0),
                    mode,
                    &mut |_| calls += 1
                )
                .is_err(),
                "accepted {variant}/{mode:?}"
            );
            assert_eq!(calls, 0);
            for index in 0..3 {
                assert!(f.target(index).exists());
                assert!(f.cue(index).exists());
            }
            assert!(journal.batch(batch.id).unwrap().is_some());
        }
        f.sources_unchanged();
    }
}
#[test]
fn foreign_replacement_during_undo_callback_is_preserved_with_history() {
    let f = Fixture::new();
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    let batch =
        Executor::apply(&f.plan(1, false), &mut journal, std::slice::from_ref(&f.0)).unwrap();
    let mut replaced = false;
    let backup = f.0.join("original-copy.flac");
    assert!(Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&f.0),
        &mut |progress| {
            if progress.path == f.target(0) && !replaced {
                fs::rename(f.target(0), &backup).unwrap();
                fs::write(f.target(0), flac()).unwrap();
                replaced = true;
            }
        }
    )
    .is_err());
    assert!(replaced);
    assert_eq!(fs::read(f.target(0)).unwrap(), flac());
    assert_eq!(fs::read(backup).unwrap(), flac());
    assert!(f.cue(0).exists());
    assert!(journal.batch(batch.id).unwrap().is_some());
    f.sources_unchanged();
}
#[test]
fn hardlink_to_owned_copy_survives_undo_without_duplicate_payload_changes() {
    let f = Fixture::new();
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    let batch =
        Executor::apply(&f.plan(1, true), &mut journal, std::slice::from_ref(&f.0)).unwrap();
    let alias = f.0.join("kept-alias.flac");
    fs::hard_link(f.target(0), &alias).unwrap();
    assert_eq!(raw_artist(&alias), "After");
    let before = fs::read(&alias).unwrap();
    drop(journal);
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert!(!f.target(0).exists());
    assert!(!f.cue(0).exists());
    assert_eq!(fs::read(&alias).unwrap(), before);
    assert!(journal.batches().unwrap().is_empty());
    f.sources_unchanged();
}
fn exercise_contract(journal: &mut dyn UndoJournal, f: &Fixture) {
    let batch = Executor::apply(&f.plan(1, false), journal, std::slice::from_ref(&f.0)).unwrap();
    let old = batch
        .copy_stamps
        .iter()
        .find(|s| s.path == f.target(0))
        .unwrap()
        .clone();
    let empty =
        Executor::apply(&ChangePlan::default(), journal, std::slice::from_ref(&f.0)).unwrap();
    let mut next = old.clone();
    next.modified_ns += 5;
    assert!(journal.checkpoint_copy(empty.id, &old, None).is_err());
    assert!(journal
        .checkpoint_copy(BatchId(i64::MAX), &old, None)
        .is_err());
    assert!(journal.checkpoint_copy(batch.id, &next, None).is_err());
    assert!(journal.checkpoint_copy(batch.id, &next, Some(&old)).is_ok());
    // A delayed identical retry must not duplicate a row, even after refresh.
    assert!(journal.checkpoint_copy(batch.id, &next, Some(&old)).is_ok());
    assert!(journal.checkpoint_copy(batch.id, &old, Some(&old)).is_err());
    let now = journal.batch(batch.id).unwrap().unwrap();
    assert_eq!(now.copy_stamps.len(), 2);
    assert_eq!(
        now.copy_stamps.iter().find(|s| s.path == old.path).unwrap(),
        &next
    );
    assert!(journal
        .batch(empty.id)
        .unwrap()
        .unwrap()
        .copy_stamps
        .is_empty());
    assert!(journal.checkpoint_copy(batch.id, &old, Some(&next)).is_ok());
    Executor::undo(journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert!(journal.batch(empty.id).unwrap().is_some());
    f.sources_unchanged();
}
#[test]
fn sqlite_and_memory_adapters_agree_on_cross_batch_refresh_and_idempotency() {
    let f = Fixture::new();
    exercise_contract(&mut VecJournal::new(), &f);
    let f = Fixture::new();
    exercise_contract(&mut SqliteJournal::open(&f.db()).unwrap(), &f);
}
struct Foreign {
    inner: VecJournal,
    records: usize,
}
impl UndoJournal for Foreign {
    fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError> {
        self.records += 1;
        self.inner.record(batch)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.inner.batches()
    }
    fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError> {
        self.inner.finish(id, batch)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.inner.rollback(id)
    }
}
#[test]
fn unsupported_adapter_preflight_is_side_effect_free_but_metadata_workflow_remains_available() {
    let f = Fixture::new();
    let mut journal = Foreign {
        inner: VecJournal::new(),
        records: 0,
    };
    let mut copy = f.plan(1, false);
    // A sidecar-only copying plan must also opt in; no audio rename is required.
    copy.changes[0].rename_to = None;
    let mut callbacks = 0;
    assert!(Executor::apply_with(
        &copy,
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |_| callbacks += 1,
        &AtomicBool::new(false)
    )
    .is_err());
    assert_eq!(callbacks, 0);
    assert_eq!(journal.records, 0);
    assert!(!f.cue(0).exists());
    f.sources_unchanged();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: f.source(0),
            tag_changes: vec![FieldChange {
                field: TagField::Artist,
                old: Some("Before".into()),
                new: Some("After".into()),
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(journal.records, 1);
    assert_eq!(raw_artist(&f.source(0)), "After");
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(raw_artist(&f.source(0)), "Before");
}
#[test]
fn independent_multi_file_copy_child() {
    let Some(root) = std::env::var_os("TAGREX_INDEPENDENT_COPY_ROOT") else {
        return;
    };
    // This fixture belongs to the parent; never remove it from the child.
    let root = PathBuf::from(root);
    let f = std::mem::ManuallyDrop::new(Fixture(root));
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    Executor::apply_with(
        &f.plan(3, true),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |progress| {
            if progress.completed == Some(ApplyStep::Tags) && progress.path == f.source(1) {
                std::process::exit(83);
            }
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    panic!("owned exit checkpoint not reached");
}
#[test]
fn multiple_refreshed_and_unedited_copies_recover_together_after_process_exit() {
    let f = Fixture::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "independent_multi_file_copy_child",
            "--nocapture",
        ])
        .env("TAGREX_INDEPENDENT_COPY_ROOT", &f.0)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(83));
            break;
        }
        if started.elapsed() > Duration::from_secs(20) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("owned child deadline exceeded");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    for index in 0..3 {
        assert_eq!(
            raw_artist(&f.target(index)),
            if index < 2 { "After" } else { "Before" }
        );
    }
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    let batch = journal.batches().unwrap().remove(0);
    assert!(batch.interrupted);
    assert_eq!(batch.copy_stamps.len(), 6);
    for stamp in &batch.copy_stamps {
        assert_eq!(stamp, &CopyStamp::of(&stamp.path).unwrap());
    }
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    for index in 0..3 {
        assert!(!f.target(index).exists());
        assert!(!f.cue(index).exists());
    }
    f.sources_unchanged();
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn sqlite_refresh_transaction_abort_does_not_emit_completion_or_leak_partial_sql() {
    struct AbortRefresh {
        inner: SqliteJournal,
        db: PathBuf,
        armed: bool,
    }
    impl UndoJournal for AbortRefresh {
        fn supports_copy_checkpoints(&self) -> bool {
            true
        }
        fn checkpoint_copy(
            &mut self,
            id: BatchId,
            stamp: &CopyStamp,
            previous: Option<&CopyStamp>,
        ) -> Result<(), JournalError> {
            if previous.is_some() && !self.armed {
                let conn = rusqlite::Connection::open(&self.db).unwrap();
                conn.execute_batch("CREATE TABLE independent_audit(n INTEGER); CREATE TRIGGER independent_abort_refresh AFTER UPDATE OF len,modified_ns,sha256 ON copy_stamps BEGIN INSERT INTO independent_audit VALUES(1); SELECT RAISE(ABORT,'independent refresh transaction rejection'); END;").unwrap();
                self.armed = true;
            }
            self.inner.checkpoint_copy(id, stamp, previous)
        }
        fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
            self.inner.record(b)
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            self.inner.batches()
        }
        fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
            self.inner.finish(id, b)
        }
        fn retain_interrupted(
            &mut self,
            id: BatchId,
            b: &AppliedBatch,
        ) -> Result<(), JournalError> {
            self.inner.retain_interrupted(id, b)
        }
        fn prepare_copy_removal(
            &mut self,
            id: BatchId,
            target: &Path,
            reservation: &Path,
        ) -> Result<(), JournalError> {
            self.inner.prepare_copy_removal(id, target, reservation)
        }
        fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
            self.inner.rollback(id)
        }
    }
    let f = Fixture::new();
    let mut journal = AbortRefresh {
        inner: SqliteJournal::open(&f.db()).unwrap(),
        db: f.db(),
        armed: false,
    };
    let prior = Executor::apply(
        &ChangePlan::default(),
        &mut journal,
        std::slice::from_ref(&f.0),
    )
    .unwrap();
    let mut transfers = 0;
    let mut tags = 0;
    let error = Executor::apply_with(
        &f.plan(1, true),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |progress| {
            transfers += usize::from(progress.completed == Some(ApplyStep::Transfer));
            tags += usize::from(progress.completed == Some(ApplyStep::Tags));
        },
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("independent refresh transaction rejection"),
        "{error}"
    );
    assert!(journal.armed);
    assert_eq!(transfers, 2);
    assert_eq!(tags, 0);
    assert!(!f.target(0).exists());
    assert!(!f.cue(0).exists());
    f.sources_unchanged();
    let conn = rusqlite::Connection::open(f.db()).unwrap();
    let audit: i64 = conn
        .query_row("SELECT count(*) FROM independent_audit", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(audit, 0);
    drop(journal);
    let journal = SqliteJournal::open(&f.db()).unwrap();
    assert_eq!(journal.batches().unwrap().len(), 1);
    assert!(journal.batch(prior.id).unwrap().is_some());
}

#[test]
fn conflicting_duplicate_copy_rows_refuse_before_any_output_is_removed() {
    for duplicate_identity in ["identity", "zeroblob(39)"] {
        let f = Fixture::new();
        let mut journal = SqliteJournal::open(&f.db()).unwrap();
        let batch =
            Executor::apply(&f.plan(3, false), &mut journal, std::slice::from_ref(&f.0)).unwrap();
        let conn = rusqlite::Connection::open(f.db()).unwrap();
        let sql = format!("INSERT INTO copy_stamps(batch_id,path,len,modified_ns,sha256,undo_path,identity,moved) SELECT batch_id,path,len,modified_ns,sha256,undo_path,{duplicate_identity},moved FROM copy_stamps WHERE batch_id=?1 AND path=?2");
        conn.execute(
            &sql,
            rusqlite::params![batch.id.0, f.cue(2).to_str().unwrap()],
        )
        .unwrap();
        drop(conn);
        drop(journal);
        let mut journal = SqliteJournal::open(&f.db()).unwrap();
        let before = journal.batch(batch.id).unwrap().unwrap().copy_stamps;
        assert_eq!(before.len(), 7);
        for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
            let mut callbacks = 0;
            let result = Executor::undo_with_mode(
                &mut journal,
                batch.id,
                std::slice::from_ref(&f.0),
                mode,
                &mut |_| callbacks += 1,
            );
            assert!(
                result.is_err(),
                "ambiguous copy ownership collection was accepted"
            );
            assert_eq!(callbacks, 0);
            for index in 0..3 {
                assert!(f.target(index).exists());
                assert!(f.cue(index).exists());
            }
            assert_eq!(
                journal.batch(batch.id).unwrap().unwrap().copy_stamps,
                before
            );
            f.sources_unchanged();
        }
    }
}

#[test]
fn duplicate_removal_reservation_refusal_does_not_rewrite_ambiguous_evidence() {
    let f = Fixture::new();
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    let batch =
        Executor::apply(&f.plan(1, false), &mut journal, std::slice::from_ref(&f.0)).unwrap();
    let conn = rusqlite::Connection::open(f.db()).unwrap();
    conn.execute("INSERT INTO copy_stamps(batch_id,path,len,modified_ns,sha256,undo_path,identity,moved) SELECT batch_id,path,len,modified_ns,sha256,undo_path,identity,moved FROM copy_stamps WHERE batch_id=?1 AND path=?2", rusqlite::params![batch.id.0, f.cue(0).to_str().unwrap()]).unwrap();
    drop(conn);
    let before = journal.batch(batch.id).unwrap().unwrap().copy_stamps;
    assert!(journal
        .prepare_copy_removal(batch.id, &f.cue(0), &f.0.join("reservation/copy"))
        .is_err());
    assert_eq!(
        journal.batch(batch.id).unwrap().unwrap().copy_stamps,
        before,
        "refused reservation rewrote ambiguous evidence"
    );
    assert!(f.target(0).exists());
    assert!(f.cue(0).exists());
    f.sources_unchanged();
}

#[test]
fn memory_duplicate_removal_reservation_also_refuses_without_changing_history() {
    let f = Fixture::new();
    let mut initial = VecJournal::new();
    let mut batch =
        Executor::apply(&f.plan(1, false), &mut initial, std::slice::from_ref(&f.0)).unwrap();
    batch.copy_stamps.push(
        batch
            .copy_stamps
            .iter()
            .find(|stamp| stamp.path == f.cue(0))
            .unwrap()
            .clone(),
    );
    let mut journal = VecJournal::new();
    let id = journal.record(&batch).unwrap();
    let before = journal.batch(id).unwrap().unwrap().copy_stamps;
    assert!(
        journal
            .prepare_copy_removal(id, &f.cue(0), &f.0.join("reservation/copy"))
            .is_err(),
        "memory adapter accepted duplicate ownership"
    );
    assert_eq!(journal.batch(id).unwrap().unwrap().copy_stamps, before);
    assert!(f.target(0).exists());
    assert!(f.cue(0).exists());
    f.sources_unchanged();
}
