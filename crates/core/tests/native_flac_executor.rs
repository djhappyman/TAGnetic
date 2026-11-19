#![cfg(target_os = "linux")]
//! Untouched native metadata must survive a scalar edit through Executor.
//! Raw fixture encoding/inspection is independent of the production tag reader.
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    journal::{SqliteJournal, UndoJournal},
    model::TagField,
    plan::{ChangePlan, Executor, FieldChange, FileChange, UndoMode},
};

static CASE: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Fixture(PathBuf, std::sync::MutexGuard<'static, ()>);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-metadata-preservation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path, CASE.lock().unwrap_or_else(|p| p.into_inner()))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = &self.1;
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn flac(items: &[(&str, &str)]) -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut comments = 4u32.to_le_bytes().to_vec();
    comments.extend(b"test");
    comments.extend((items.len() as u32).to_le_bytes());
    for (key, value) in items {
        let item = format!("{key}={value}");
        comments.extend((item.len() as u32).to_le_bytes());
        comments.extend(item.as_bytes());
    }
    let n = comments.len() as u32;
    out.extend([4, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(comments);
    out.extend([2, 0, 0, 8]);
    out.extend(b"TEST\0\xff\x01\x02");
    out.extend([0x81, 0, 0, 128]);
    out.extend([0; 128]);
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out
}
fn inspect(bytes: &[u8]) -> (BTreeMap<String, Vec<String>>, Vec<u8>, Vec<u8>) {
    assert_eq!(&bytes[..4], b"fLaC");
    let mut at = 4;
    let mut tags = BTreeMap::new();
    let mut application = Vec::new();
    loop {
        let flag = bytes[at];
        let length = ((bytes[at + 1] as usize) << 16)
            | ((bytes[at + 2] as usize) << 8)
            | bytes[at + 3] as usize;
        at += 4;
        let payload = &bytes[at..at + length];
        at += length;
        if flag & 127 == 2 {
            application.extend(payload);
        }
        if flag & 127 == 4 {
            let number = |offset: usize| {
                u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize
            };
            let mut offset = 4 + number(0);
            let count = number(offset);
            offset += 4;
            for _ in 0..count {
                let len = number(offset);
                offset += 4;
                let item = std::str::from_utf8(&payload[offset..offset + len]).unwrap();
                offset += len;
                let (key, value) = item.split_once('=').unwrap();
                tags.entry(key.to_ascii_uppercase())
                    .or_insert_with(Vec::new)
                    .push(value.to_string());
            }
        }
        if flag & 128 != 0 {
            break;
        }
    }
    (tags, application, bytes[at..].to_vec())
}

fn fixture() -> (Fixture, PathBuf, PathBuf, Vec<u8>) {
    let f = Fixture::new();
    let path = f.0.join("track.flac");
    let db = f.0.join("journal.sqlite");
    let original = flac(&[
        ("TITLE", "Saved"),
        ("ARTIST", "A; B"),
        ("ARTIST", "a; b"),
        ("ARTIST", ""),
        ("ARTIST", "尾"),
        ("GENRE", "House"),
        ("GENRE", "house"),
        ("CUSTOM", ""),
        ("CUSTOM", "尾"),
    ]);
    std::fs::write(&path, &original).unwrap();
    (f, path, db, original)
}
fn change(path: &std::path::Path, field: TagField, new: Option<&str>) -> FieldChange {
    FieldChange {
        old: tagrex_core::model::TagEngine::read(path)
            .unwrap()
            .tags
            .get(&field)
            .cloned(),
        field,
        new: new.map(str::to_string),
    }
}
fn plan(path: &std::path::Path, fields: Vec<FieldChange>) -> ChangePlan {
    ChangePlan {
        changes: vec![FileChange {
            path: path.into(),
            tag_changes: fields,
            ..Default::default()
        }],
        ..Default::default()
    }
}
fn raw_set(path: &std::path::Path, name: &str, values: &[&str]) {
    let bytes = std::fs::read(path).unwrap();
    let (mut tags, _, _) = inspect(&bytes);
    tags.insert(name.into(), values.iter().map(|s| s.to_string()).collect());
    let fields: Vec<_> = tags
        .iter()
        .flat_map(|(k, v)| v.iter().map(move |v| (k.as_str(), v.as_str())))
        .collect();
    std::fs::write(path, flac(&fields)).unwrap();
}
#[test]
fn repeated_target_values_restore_exactly_after_restart_and_reapply() {
    let (f, path, db, original) = fixture();
    let before = inspect(&original);
    let p = plan(
        &path,
        vec![change(&path, TagField::Artist, Some("New; Other"))],
    );
    let mut journal = SqliteJournal::open(&db).unwrap();
    let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    assert!(batch.native_tags);
    drop(journal);
    let mut journal = SqliteJournal::open(&db).unwrap();
    assert_eq!(
        journal.headers(std::slice::from_ref(&f.0)).unwrap()[0].paths,
        std::slice::from_ref(&path)
    );
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), before);
    let next = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    Executor::undo(&mut journal, next.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), before);
}
#[test]
fn composed_fields_absence_and_empty_native_members_survive_restart_undo() {
    let (f, path, db, original) = fixture();
    let p = plan(
        &path,
        vec![
            change(&path, TagField::Title, Some("New")),
            change(&path, TagField::Artist, Some("")),
            change(&path, TagField::Custom("ADDED".into()), Some("尾")),
            change(&path, TagField::Genre, None),
        ],
    );
    let mut journal = SqliteJournal::open(&db).unwrap();
    let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    let tags = inspect(&std::fs::read(&path).unwrap()).0;
    assert_eq!(tags["ARTIST"], [""]);
    assert_eq!(tags["ADDED"], ["尾"]);
    assert!(!tags.contains_key("GENRE"));
    drop(journal);
    let mut journal = SqliteJournal::open(&db).unwrap();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), inspect(&original));
}
#[test]
fn native_conflict_modes_preserve_unrelated_current_metadata() {
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        let (f, path, db, original) = fixture();
        let p = plan(
            &path,
            vec![
                change(&path, TagField::Title, Some("New")),
                change(&path, TagField::Artist, Some("Updated")),
            ],
        );
        let mut journal = SqliteJournal::open(&db).unwrap();
        let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
        raw_set(&path, "TITLE", &["External"]);
        raw_set(&path, "EXTERNAL", &["Unrelated 尾"]);
        let externally_changed = std::fs::read(&path).unwrap();
        drop(journal);
        let mut journal = SqliteJournal::open(&db).unwrap();
        let result = Executor::undo_with_mode(
            &mut journal,
            batch.id,
            std::slice::from_ref(&f.0),
            mode,
            &mut |_| {},
        );
        if matches!(mode, UndoMode::Strict) {
            assert!(result.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), externally_changed);
            assert!(journal.batch(batch.id).unwrap().is_some());
            continue;
        }
        let report = result.unwrap();
        let tags = inspect(&std::fs::read(&path).unwrap()).0;
        assert_eq!(tags["EXTERNAL"], ["Unrelated 尾"]);
        assert_eq!(tags["ARTIST"], inspect(&original).0["ARTIST"]);
        if matches!(mode, UndoMode::KeepChanged) {
            assert_eq!(tags["TITLE"], ["External"]);
            assert_eq!(report.kept.len(), 1);
        } else {
            assert_eq!(tags["TITLE"], ["Saved"]);
            assert!(report.kept.is_empty());
        }
    }
}
#[test]
fn old_text_path_reader_refuses_new_native_history_before_undo() {
    let (f, path, db, _) = fixture();
    let p = plan(&path, vec![change(&path, TagField::Title, Some("New"))]);
    let mut journal = SqliteJournal::open(&db).unwrap();
    let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    let current = std::fs::read(&path).unwrap();
    drop(journal);
    let conn = rusqlite::Connection::open(&db).unwrap();
    let old_read = conn.query_row(
        "SELECT path FROM file_changes WHERE batch_id=?1",
        [batch.id.0],
        |r| r.get::<_, String>(0),
    );
    assert!(matches!(
        old_read,
        Err(rusqlite::Error::InvalidColumnType(..))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), current);
}
#[test]
fn duplicate_and_alias_fields_refuse_without_media_or_history_effects() {
    for alias in [false, true] {
        let (f, path, db, original) = fixture();
        let mut fields = vec![change(&path, TagField::Title, Some("First"))];
        fields.push(change(
            &path,
            if alias {
                TagField::Custom("TITLE".into())
            } else {
                TagField::Title
            },
            Some("Second"),
        ));
        let p = plan(&path, fields);
        let mut journal = SqliteJournal::open(&db).unwrap();
        assert!(Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(journal.batches().unwrap().is_empty());
    }
}
#[test]
fn more_than_store_capacity_files_retire_between_publications() {
    let f = Fixture::new();
    let db = f.0.join("journal.sqlite");
    let mut changes = Vec::new();
    for i in 0..160 {
        let path = f.0.join(format!("{i}.flac"));
        std::fs::write(&path, flac(&[("TITLE", "Saved"), ("ARTIST", "A; B")])).unwrap();
        changes.push(FileChange {
            tag_changes: vec![change(&path, TagField::Title, Some("New"))],
            path,
            ..Default::default()
        });
    }
    let p = ChangePlan {
        changes,
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&db).unwrap();
    let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    let store = f.0.join("journal.sqlite.native-media");
    assert_eq!(std::fs::read_dir(&store).unwrap().count(), 1);
    drop(journal);
    let mut journal = SqliteJournal::open(&db).unwrap();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    for change in p.changes {
        assert_eq!(
            inspect(&std::fs::read(change.path).unwrap()).0["TITLE"],
            ["Saved"]
        );
    }
    assert_eq!(std::fs::read_dir(&store).unwrap().count(), 1);
}

use tagrex_core::{
    journal::{
        native::{Entries, Intent, NativeJournal},
        AppliedBatch, BatchId, JournalError,
    },
    media_mutation::{Phase, Record, Store},
};
struct FaultJournal {
    inner: SqliteJournal,
    phase: Option<Phase>,
    after: bool,
    fired: bool,
    kill: bool,
    finish: bool,
    tamper: Option<PathBuf>,
}
impl UndoJournal for FaultJournal {
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
impl NativeJournal for FaultJournal {
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
        if r.phase() == Phase::Publishing {
            if let Some(db) = self.tamper.take() {
                rusqlite::Connection::open(db).unwrap().execute(
                    "UPDATE native_tag_files SET intents=zeroblob(length(intents)) WHERE batch_id=?1 AND ordinal=?2",
                    rusqlite::params![b.id.0, i as i64]).unwrap();
            }
        }
        let fire = !self.fired && self.phase == Some(r.phase());
        if fire && !self.after {
            self.fired = true;
            return Err(JournalError::Storage(
                "owned checkpoint-before failure".into(),
            ));
        }
        NativeJournal::checkpoint(&mut self.inner, b, (i, o), r, expected, receipts, done)?;
        if fire {
            self.fired = true;
            if self.kill {
                std::process::exit(73);
            }
            return Err(JournalError::Storage(
                "owned checkpoint-after failure".into(),
            ));
        }
        Ok(())
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
        if self.finish && !self.fired {
            self.fired = true;
            return Err(JournalError::Storage("owned finish failure".into()));
        }
        NativeJournal::finish_native(&mut self.inner, b)
    }
    fn retain_native(&mut self, b: &AppliedBatch) -> Result<(), JournalError> {
        NativeJournal::retain_native(&mut self.inner, b)
    }
    fn remove_native(&mut self, id: BatchId) -> Result<(), JournalError> {
        if self.finish && !self.fired {
            self.fired = true;
            return Err(JournalError::Storage("owned removal failure".into()));
        }
        NativeJournal::remove_native(&mut self.inner, id)
    }
}
#[test]
fn errors_before_and_after_each_durable_phase_rollback_with_exact_receipts() {
    for phase in [
        Phase::Prepared,
        Phase::Publishing,
        Phase::Published,
        Phase::Retired,
    ] {
        for after in [false, true] {
            let (f, path, db, original) = fixture();
            let p = plan(&path, vec![change(&path, TagField::Artist, Some("New"))]);
            let mut journal = FaultJournal {
                inner: SqliteJournal::open(&db).unwrap(),
                phase: Some(phase),
                after,
                fired: false,
                kill: false,
                finish: false,
                tamper: None,
            };
            let result = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0));
            assert!(journal.fired);
            assert!(result.is_err());
            assert_eq!(
                inspect(&std::fs::read(&path).unwrap()),
                inspect(&original),
                "{phase:?} after={after} {result:?}"
            );
            assert!(
                journal.batches().unwrap().is_empty(),
                "{phase:?} after={after} {result:?}"
            );
        }
    }
    let (f, path, db, original) = fixture();
    let p = plan(&path, vec![change(&path, TagField::Artist, Some("New"))]);
    let mut journal = FaultJournal {
        inner: SqliteJournal::open(&db).unwrap(),
        phase: None,
        after: false,
        fired: false,
        kill: false,
        finish: true,
        tamper: None,
    };
    assert!(Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).is_err());
    assert!(journal.fired);
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), inspect(&original));
    assert!(journal.batches().unwrap().is_empty());
}
#[test]
fn failed_undo_acknowledgements_restore_actual_external_baseline_and_keep_history() {
    for phase in [
        Phase::Prepared,
        Phase::Publishing,
        Phase::Published,
        Phase::Retired,
    ] {
        for after in [false, true] {
            let (f, path, db, original) = fixture();
            let p = plan(&path, vec![change(&path, TagField::Artist, Some("New"))]);
            let mut journal = FaultJournal {
                inner: SqliteJournal::open(&db).unwrap(),
                phase: None,
                after,
                fired: false,
                kill: false,
                finish: false,
                tamper: None,
            };
            let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
            raw_set(
                &path,
                "ARTIST",
                &["External; literal", "", "External duplicate"],
            );
            raw_set(&path, "EXTERNAL", &["Unrelated"]);
            let baseline = inspect(&std::fs::read(&path).unwrap());
            journal.phase = Some(phase);
            let result = Executor::undo_with_mode(
                &mut journal,
                batch.id,
                std::slice::from_ref(&f.0),
                UndoMode::Overwrite,
                &mut |_| {},
            );
            assert!(journal.fired, "{phase:?} after={after} {result:?}");
            assert!(result.is_err());
            assert_eq!(
                inspect(&std::fs::read(&path).unwrap()),
                baseline,
                "{phase:?} after={after} {result:?}"
            );
            assert!(journal.batch(batch.id).unwrap().is_some());
            drop(journal);
            let mut journal = SqliteJournal::open(&db).unwrap();
            Executor::undo_with_mode(
                &mut journal,
                batch.id,
                std::slice::from_ref(&f.0),
                UndoMode::Overwrite,
                &mut |_| {},
            )
            .unwrap_or_else(|e| panic!("reopened {phase:?} after={after}: {e:?}"));
            let restored = inspect(&std::fs::read(&path).unwrap());
            assert_eq!(restored.0["ARTIST"], inspect(&original).0["ARTIST"]);
            assert_eq!(restored.0["EXTERNAL"], ["Unrelated"]);
        }
    }
}
#[test]
fn undo_history_removal_failure_rolls_media_back_to_external_baseline() {
    let (f, path, db, original) = fixture();
    let p = plan(&path, vec![change(&path, TagField::Artist, Some("New"))]);
    let mut journal = FaultJournal {
        inner: SqliteJournal::open(&db).unwrap(),
        phase: None,
        after: false,
        fired: false,
        kill: false,
        finish: false,
        tamper: None,
    };
    let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    raw_set(&path, "ARTIST", &["External; literal", ""]);
    let baseline = inspect(&std::fs::read(&path).unwrap());
    journal.finish = true;
    let result = Executor::undo_with_mode(
        &mut journal,
        batch.id,
        std::slice::from_ref(&f.0),
        UndoMode::Overwrite,
        &mut |_| {},
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("owned removal failure"));
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), baseline);
    assert!(journal.batch(batch.id).unwrap().is_some());
    drop(journal);
    let mut journal = SqliteJournal::open(&db).unwrap();
    Executor::undo_with_mode(
        &mut journal,
        batch.id,
        std::slice::from_ref(&f.0),
        UndoMode::Overwrite,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), inspect(&original));
}
#[test]
fn changed_durable_intent_refuses_before_publication_and_preserves_evidence() {
    let (f, path, db, original) = fixture();
    let p = plan(&path, vec![change(&path, TagField::Artist, Some("New"))]);
    let mut journal = FaultJournal {
        inner: SqliteJournal::open(&db).unwrap(),
        phase: None,
        after: false,
        fired: false,
        kill: false,
        finish: false,
        tamper: Some(db.clone()),
    };
    let result = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0));
    assert!(result.is_err());
    assert!(journal.tamper.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(journal.batches().unwrap().len(), 1);
    assert!(
        std::fs::read_dir(f.0.join("journal.sqlite.native-media"))
            .unwrap()
            .count()
            > 1
    );
}
#[test]
fn native_checkpoint_process_child() {
    let Ok(root) = std::env::var("TAGREX_NATIVE_CHILD_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let path = root.join("track.flac");
    let p = plan(&path, vec![change(&path, TagField::Artist, Some("New"))]);
    let phase = match std::env::var("TAGREX_NATIVE_CHILD_PHASE").unwrap().as_str() {
        "prepared" => Phase::Prepared,
        "publishing" => Phase::Publishing,
        "published" => Phase::Published,
        "retired" => Phase::Retired,
        _ => panic!("invalid fixture phase"),
    };
    let mut journal = FaultJournal {
        inner: SqliteJournal::open(&root.join("journal.sqlite")).unwrap(),
        phase: Some(phase),
        after: true,
        fired: false,
        kill: true,
        finish: false,
        tamper: None,
    };
    let _ = Executor::apply(&p, &mut journal, &[root]);
    panic!("fixture must exit at actual durable checkpoint");
}
#[test]
fn actual_process_death_at_each_checkpoint_recovers_through_reopened_executor() {
    for phase in ["prepared", "publishing", "published", "retired"] {
        let (f, path, db, original) = fixture();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_checkpoint_process_child", "--nocapture"])
            .env("TAGREX_NATIVE_CHILD_ROOT", &f.0)
            .env("TAGREX_NATIVE_CHILD_PHASE", phase)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73));
        let mut journal = SqliteJournal::open(&db).unwrap();
        let batches = journal.batches().unwrap();
        assert_eq!(batches.len(), 1);
        assert!(batches[0].interrupted);
        Executor::undo(&mut journal, batches[0].id, std::slice::from_ref(&f.0))
            .unwrap_or_else(|e| panic!("{phase}: {e}"));
        assert_eq!(
            inspect(&std::fs::read(&path).unwrap()),
            inspect(&original),
            "{phase}"
        );
        assert!(journal.batches().unwrap().is_empty());
    }
}
#[test]
fn overwritten_external_baseline_is_restored_when_later_undo_file_is_replaced() {
    let (f, path, db, _) = fixture();
    let second = f.0.join("second.flac");
    std::fs::write(&second, flac(&[("TITLE", "Saved"), ("ARTIST", "Before")])).unwrap();
    let p = ChangePlan {
        changes: vec![
            plan(&path, vec![change(&path, TagField::Title, Some("New"))])
                .changes
                .remove(0),
            plan(&second, vec![change(&second, TagField::Title, Some("New"))])
                .changes
                .remove(0),
        ],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&db).unwrap();
    let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    raw_set(&path, "TITLE", &["External first"]);
    let baseline = inspect(&std::fs::read(&path).unwrap());
    let backup = f.0.join("second-saved.flac");
    let foreign = flac(&[("TITLE", "Foreign"), ("ARTIST", "Other")]);
    let mut changed = false;
    let result = Executor::undo_with_mode(
        &mut journal,
        batch.id,
        std::slice::from_ref(&f.0),
        UndoMode::Overwrite,
        &mut |progress| {
            if progress.path == second && !changed {
                std::fs::rename(&second, &backup).unwrap();
                std::fs::write(&second, &foreign).unwrap();
                changed = true;
            }
        },
    );
    assert!(changed);
    assert!(result.is_err());
    assert_eq!(
        inspect(&std::fs::read(&path).unwrap()),
        baseline,
        "the earlier overwritten target needs its actual external baseline back"
    );
    assert_eq!(std::fs::read(&second).unwrap(), foreign);
    assert!(journal.batch(batch.id).unwrap().is_some());
}
#[test]
fn malformed_later_receipt_refuses_before_any_recovery_or_store_recreation() {
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        let (f, path, db, _) = fixture();
        let second = f.0.join("second.flac");
        std::fs::write(&second, flac(&[("TITLE", "Saved")])).unwrap();
        let p = ChangePlan {
            changes: vec![
                plan(&path, vec![change(&path, TagField::Title, Some("New"))])
                    .changes
                    .remove(0),
                plan(&second, vec![change(&second, TagField::Title, Some("New"))])
                    .changes
                    .remove(0),
            ],
            ..Default::default()
        };
        let mut journal = SqliteJournal::open(&db).unwrap();
        let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
        drop(journal);
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE native_tag_files SET receipts=zeroblob(64) WHERE ordinal=1",
            [],
        )
        .unwrap();
        let before1 = std::fs::read(&path).unwrap();
        let before2 = std::fs::read(&second).unwrap();
        let store = f.0.join("journal.sqlite.native-media");
        std::fs::remove_dir_all(&store).unwrap();
        drop(conn);
        let mut journal = SqliteJournal::open(&db).unwrap();
        let mut callbacks = 0;
        assert!(Executor::undo_with_mode(
            &mut journal,
            batch.id,
            std::slice::from_ref(&f.0),
            mode,
            &mut |_| callbacks += 1
        )
        .is_err());
        assert_eq!(callbacks, 0);
        assert_eq!(std::fs::read(&path).unwrap(), before1);
        assert_eq!(std::fs::read(&second).unwrap(), before2);
        assert!(!store.exists());
        assert!(journal.batch(batch.id).unwrap().is_some());
    }
}
#[test]
#[ignore = "manual: actual historical 9da core reader executable supplied by fixture build"]
fn actual_historical_reader_refuses_native_history_without_effects() {
    let reader = std::env::var_os("TAGREX_HISTORICAL_NATIVE_READER")
        .expect("historical reader fixture path");
    let (f, path, db, _) = fixture();
    let p = plan(&path, vec![change(&path, TagField::Title, Some("New"))]);
    let mut journal = SqliteJournal::open(&db).unwrap();
    let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    let before = std::fs::read(&path).unwrap();
    drop(journal);
    let output = std::process::Command::new(reader)
        .arg(&db)
        .arg(batch.id.0.to_string())
        .arg(&f.0)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("HISTORICAL_UNDO_REFUSED") && error.contains("Blob"),
        "{error}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut journal = SqliteJournal::open(&db).unwrap();
    assert!(journal.batch(batch.id).unwrap().unwrap().native_tags);
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
}
#[test]
#[ignore = "manual independent ffmpeg/ffprobe/Mutagen native FLAC round trip"]
fn independent_real_flac_packets_and_native_values_survive_apply_restart_undo() {
    let f = Fixture::new();
    let path = f.0.join("real.flac");
    let db = f.0.join("journal.sqlite");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100",
            "-t",
            "0.08",
            "-c:a",
            "flac",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let mut bytes = std::fs::read(&path).unwrap();
    let mut at = 4usize;
    loop {
        let flag = bytes[at];
        let len = ((bytes[at + 1] as usize) << 16)
            | ((bytes[at + 2] as usize) << 8)
            | bytes[at + 3] as usize;
        if flag & 127 == 4 {
            let comments = flac(&[
                ("TITLE", "Saved"),
                ("ARTIST", "A; B"),
                ("ARTIST", "a; b"),
                ("ARTIST", ""),
                ("ARTIST", "尾"),
                ("CUSTOM", ""),
                ("CUSTOM", "尾"),
            ]);
            let mut c = 4;
            loop {
                let n = ((comments[c + 1] as usize) << 16)
                    | ((comments[c + 2] as usize) << 8)
                    | comments[c + 3] as usize;
                if comments[c] & 127 == 4 {
                    let mut replacement = vec![flag, (n >> 16) as u8, (n >> 8) as u8, n as u8];
                    replacement.extend_from_slice(&comments[c + 4..c + 4 + n]);
                    bytes.splice(at..at + 4 + len, replacement);
                    break;
                }
                c += 4 + n;
            }
            break;
        }
        assert_eq!(flag & 128, 0, "fixture must carry a Vorbis comment block");
        at += 4 + len;
    }
    std::fs::write(&path, &bytes).unwrap();
    let packets = |path: &std::path::Path| {
        let o = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "packet=data_hash",
                "-show_data_hash",
                "sha256",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(o.status.success());
        assert!(String::from_utf8_lossy(&o.stdout).contains("SHA256:"));
        o.stdout
    };
    let native = |path: &std::path::Path| {
        let o=std::process::Command::new(std::env::var_os("TAGREX_TEST_PYTHON").unwrap_or_else(||"python3".into())).args(["-c","import sys,json;from mutagen.flac import FLAC;print(json.dumps(dict(FLAC(sys.argv[1]).tags),ensure_ascii=False,sort_keys=True))"]).arg(path).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o.stdout
    };
    let before_packets = packets(&path);
    let before_native = native(&path);
    let before = inspect(&bytes);
    let p = plan(
        &path,
        vec![
            change(&path, TagField::Title, Some("New")),
            change(&path, TagField::Artist, Some("New; Other")),
        ],
    );
    let mut journal = SqliteJournal::open(&db).unwrap();
    let b = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(packets(&path), before_packets);
    let after = native(&path);
    assert!(String::from_utf8_lossy(&after).contains("New"));
    drop(journal);
    let mut journal = SqliteJournal::open(&db).unwrap();
    Executor::undo(&mut journal, b.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), before);
    assert_eq!(native(&path), before_native);
    assert_eq!(packets(&path), before_packets);
}
#[test]
fn cancelled_batch_rolls_back_completed_native_file_and_keeps_later_file_unchanged() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (f, path, db, original) = fixture();
    let second = f.0.join("second.flac");
    let second_before = flac(&[("TITLE", "Saved"), ("ARTIST", "Other")]);
    std::fs::write(&second, &second_before).unwrap();
    let p = ChangePlan {
        changes: vec![
            plan(&path, vec![change(&path, TagField::Title, Some("New"))])
                .changes
                .remove(0),
            plan(&second, vec![change(&second, TagField::Title, Some("New"))])
                .changes
                .remove(0),
        ],
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let mut journal = SqliteJournal::open(&db).unwrap();
    let mut completed = 0;
    let result = Executor::apply_with(
        &p,
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |progress| {
            if progress.completed.is_some() {
                completed += 1;
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(matches!(
        result,
        Err(tagrex_core::plan::PlanError::Cancelled)
    ));
    assert_eq!(completed, 1);
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), inspect(&original));
    assert_eq!(std::fs::read(&second).unwrap(), second_before);
    assert!(journal.batches().unwrap().is_empty());
}
#[test]
fn shared_job_pressure_refuses_before_intent_and_retry_releases_all_native_owners() {
    use tagrex_core::resources::{Budget, Charge};
    let (f, path, db, original) = fixture();
    let p = plan(&path, vec![change(&path, TagField::Title, Some("New"))]);
    let budget = Budget::process();
    let baseline = budget.usage().0;
    let mut journal = SqliteJournal::open(&db).unwrap();
    let pressure = budget
        .try_acquire(Charge {
            jobs: 8,
            ..Default::default()
        })
        .unwrap();
    assert!(Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(journal.batches().unwrap().is_empty());
    assert!(!f.0.join("journal.sqlite.native-media").exists());
    drop(pressure);
    assert_eq!(budget.usage().0, baseline);
    let b = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    Executor::undo(&mut journal, b.id, std::slice::from_ref(&f.0)).unwrap();
    drop(journal);
    assert_eq!(budget.usage().0, baseline);
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), inspect(&original));
}
#[test]
fn replaced_profile_parent_is_refused_before_native_record_or_foreign_store_effects() {
    let (f, path, _, original) = fixture();
    let profile = f.0.join("profile");
    std::fs::create_dir(&profile).unwrap();
    let db = profile.join("journal.sqlite");
    let p = plan(&path, vec![change(&path, TagField::Title, Some("New"))]);
    let mut journal = SqliteJournal::open(&db).unwrap();
    let saved = f.0.join("saved-profile");
    let mut changed = false;
    let result = Executor::apply_with(
        &p,
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |_| {
            if !changed {
                std::fs::rename(&profile, &saved).unwrap();
                std::fs::create_dir(&profile).unwrap();
                std::fs::write(profile.join("foreign"), b"sentinel").unwrap();
                changed = true;
            }
        },
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert!(changed);
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read(profile.join("foreign")).unwrap(), b"sentinel");
    assert_eq!(std::fs::read_dir(&profile).unwrap().count(), 1);
    assert!(journal.batches().unwrap().is_empty());
    assert!(!saved.join("journal.sqlite.native-media").exists());
}
#[test]
fn later_preflight_callback_cannot_endorse_an_earlier_native_source_replacement() {
    let (f, path, db, original) = fixture();
    let second = f.0.join("second.flac");
    std::fs::write(&second, &original).unwrap();
    let p = ChangePlan {
        changes: vec![
            plan(&path, vec![change(&path, TagField::Title, Some("New"))])
                .changes
                .remove(0),
            plan(&second, vec![change(&second, TagField::Title, Some("New"))])
                .changes
                .remove(0),
        ],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&db).unwrap();
    let saved = f.0.join("original-saved.flac");
    let mut changed = false;
    let result = Executor::apply_with(
        &p,
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |progress| {
            if progress.path == second && !changed {
                std::fs::rename(&path, &saved).unwrap();
                std::fs::write(&path, &original).unwrap();
                changed = true;
            }
        },
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert!(changed);
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read(&saved).unwrap(), original);
    assert_eq!(std::fs::read(&second).unwrap(), original);
    assert!(journal.batches().unwrap().is_empty());
    assert!(!f.0.join("journal.sqlite.native-media").exists());
}
#[test]
fn ordinary_other_container_keeps_legacy_workflow_and_history_representation() {
    let f = Fixture::new();
    let path = f.0.join("track.wav");
    let db = f.0.join("journal.sqlite");
    let mut wav = b"RIFF".to_vec();
    wav.extend(40u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(4u32.to_le_bytes());
    wav.extend([0u8; 4]);
    std::fs::write(&path, wav).unwrap();
    let p = plan(&path, vec![change(&path, TagField::Title, Some("New"))]);
    let mut journal = SqliteJournal::open(&db).unwrap();
    let batch = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    assert!(!batch.native_tags);
    assert_eq!(
        tagrex_core::model::TagEngine::read(&path).unwrap().tags[&TagField::Title],
        "New"
    );
    let conn = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT typeof(path) FROM file_changes WHERE batch_id=?1",
            [batch.id.0],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "text"
    );
    drop(conn);
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert!(!tagrex_core::model::TagEngine::read(&path)
        .unwrap()
        .tags
        .contains_key(&TagField::Title));
    assert!(!f.0.join("journal.sqlite.native-media").exists());
}
#[test]
fn clearing_the_last_text_field_keeps_native_absence_readable_and_undoable() {
    let f = Fixture::new();
    let path = f.0.join("last.flac");
    let db = f.0.join("journal.sqlite");
    let original = flac(&[("TITLE", "Saved")]);
    std::fs::write(&path, &original).unwrap();
    let p = plan(&path, vec![change(&path, TagField::Title, None)]);
    let mut journal = SqliteJournal::open(&db).unwrap();
    let b = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    assert!(inspect(&std::fs::read(&path).unwrap()).0.is_empty());
    drop(journal);
    let mut journal = SqliteJournal::open(&db).unwrap();
    Executor::undo(&mut journal, b.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), inspect(&original));
}
#[test]
fn plain_text_aliases_and_facade_url_keep_correct_native_field_ownership() {
    let f = Fixture::new();
    let path = f.0.join("aliases.flac");
    let db = f.0.join("journal.sqlite");
    let original = flac(&[
        ("TITLE", "Saved"),
        ("ORGANIZATION", "Publisher"),
        ("WWW", "https://old.example"),
        ("ISRC", "GB0123456789"),
        ("INITIALKEY", "Am"),
        ("CATALOGNUMBER", "CAT001"),
        ("MEDIA", "Vinyl"),
    ]);
    std::fs::write(&path, &original).unwrap();
    let read = tagrex_core::model::TagEngine::read_exact(&path).unwrap();
    assert_eq!(
        read.text()
            .resolve_field(&TagField::Url)
            .unwrap()
            .unwrap()
            .target,
        tagrex_core::model::exact::NativeTextTarget::Vorbis { name: "WWW".into() }
    );
    drop(read);
    let p = plan(
        &path,
        vec![
            change(&path, TagField::Publisher, Some("New publisher")),
            change(&path, TagField::Url, Some("https://new.example")),
            change(&path, TagField::Isrc, Some("GB9876543210")),
            change(&path, TagField::InitialKey, Some("Cm")),
            change(&path, TagField::CatalogNumber, Some("CAT002")),
            change(&path, TagField::MediaType, Some("CD")),
        ],
    );
    let mut journal = SqliteJournal::open(&db).unwrap();
    let b = Executor::apply(&p, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    assert!(b.native_tags);
    let tags = tagrex_core::model::TagEngine::read(&path).unwrap().tags;
    for field in &p.changes[0].tag_changes {
        assert_eq!(
            tags.get(&field.field),
            field.new.as_ref(),
            "{:?}",
            field.field
        );
    }
    drop(journal);
    let mut journal = SqliteJournal::open(&db).unwrap();
    Executor::undo(&mut journal, b.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(inspect(&std::fs::read(&path).unwrap()), inspect(&original));
}
#[test]
fn parallel_native_sessions_complete_without_raising_shared_resource_quotas() {
    let f = Fixture::new();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for index in 0..2 {
        let root = f.0.clone();
        let barrier = barrier.clone();
        let path = root.join(format!("parallel-{index}.flac"));
        let original = flac(&[("TITLE", "Saved"), ("ARTIST", "A; B")]);
        std::fs::write(&path, &original).unwrap();
        workers.push(std::thread::spawn(move || {
            let mut journal =
                SqliteJournal::open(&root.join(format!("parallel-{index}.sqlite"))).unwrap();
            barrier.wait();
            for _ in 0..3 {
                let p = plan(&path, vec![change(&path, TagField::Title, Some("New"))]);
                let b = Executor::apply(&p, &mut journal, std::slice::from_ref(&root)).unwrap();
                Executor::undo(&mut journal, b.id, std::slice::from_ref(&root)).unwrap();
                assert_eq!(inspect(&std::fs::read(&path).unwrap()), inspect(&original));
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
}
