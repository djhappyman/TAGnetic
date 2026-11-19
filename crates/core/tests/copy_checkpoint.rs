//! Completed-copy recovery through real process exits; disposable fixtures only.
use std::{
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
    plan::{ApplyStep, ChangePlan, Executor, FieldChange, FileChange},
};
const FRAME: &[u8] = &[
    0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
];
fn flac() -> Vec<u8> {
    let mut b = b"fLaC\x00\x00\x00\x22".to_vec();
    b.extend([
        0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    let v = b"Copy checkpoint fixture";
    let mut c = (v.len() as u32).to_le_bytes().to_vec();
    c.extend(v);
    c.extend(2u32.to_le_bytes());
    for s in ["ARTIST=Before", "TITLE=Keep Jóga"] {
        c.extend((s.len() as u32).to_le_bytes());
        c.extend(s.as_bytes());
    }
    b.push(0x84);
    b.extend(&(c.len() as u32).to_be_bytes()[1..]);
    b.extend(c);
    b.extend(FRAME);
    b
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "tagrex-copy-checkpoint-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("source.flac"), flac()).unwrap();
        std::fs::write(p.join("source.cue"), b"cue original").unwrap();
        Self(p)
    }
    fn db(&self) -> PathBuf {
        self.0.join("journal.sqlite")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn plan(root: &Path, tags: bool) -> ChangePlan {
    ChangePlan {
        changes: vec![FileChange {
            path: root.join("source.flac"),
            rename_to: Some(root.join("copy.flac")),
            copy: true,
            sidecar_renames: vec![(root.join("source.cue"), root.join("copy.cue"))],
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
        }],
        ..Default::default()
    }
}
fn inspect(path: &Path, artist: &str) {
    let b = std::fs::read(path).unwrap();
    let mut at = 4;
    let mut found = false;
    loop {
        let kind = b[at];
        let len = u32::from_be_bytes([0, b[at + 1], b[at + 2], b[at + 3]]) as usize;
        let block = &b[at + 4..at + 4 + len];
        if kind & 127 == 4 {
            let mut p = 0;
            let number = |p: &mut usize| {
                let n = u32::from_le_bytes(block[*p..*p + 4].try_into().unwrap()) as usize;
                *p += 4;
                n
            };
            let n = number(&mut p);
            p += n;
            let count = number(&mut p);
            let mut values = vec![];
            for _ in 0..count {
                let n = number(&mut p);
                values.push(std::str::from_utf8(&block[p..p + n]).unwrap());
                p += n;
            }
            assert!(values.contains(&format!("ARTIST={artist}").as_str()));
            assert!(values.contains(&"TITLE=Keep Jóga"));
            found = true;
        }
        at += 4 + len;
        if kind & 128 != 0 {
            break;
        }
    }
    assert!(found);
    assert_eq!(&b[at..], FRAME);
}
struct Proxy {
    inner: SqliteJournal,
    mode: String,
    calls: usize,
}
impl UndoJournal for Proxy {
    fn prepare_move_stage(
        &mut self,
        id: BatchId,
        s: &tagrex_core::journal::MoveStage,
    ) -> Result<(), JournalError> {
        self.inner.prepare_move_stage(id, s)
    }
    fn move_stages(
        &self,
        id: BatchId,
    ) -> Result<Vec<tagrex_core::journal::MoveStage>, JournalError> {
        self.inner.move_stages(id)
    }

    fn supports_copy_checkpoints(&self) -> bool {
        true
    }
    fn checkpoint_copy(
        &mut self,
        id: BatchId,
        s: &CopyStamp,
        p: Option<&CopyStamp>,
    ) -> Result<(), JournalError> {
        self.calls += 1;
        if self.mode == "error-before-sql"
            || (self.mode == "error-before-refresh" && self.calls == 3)
        {
            return Err(JournalError::Storage(
                "checkpoint unavailable before commit".into(),
            ));
        }
        if self.mode == "before-sql" || (self.mode == "before-refresh" && self.calls == 3) {
            std::process::exit(81);
        }
        self.inner.checkpoint_copy(id, s, p)?;
        if self.mode == "after-sql" {
            std::process::exit(81);
        }
        if self.mode == "error-after-sql" {
            return Err(JournalError::Storage(
                "uncertain committed checkpoint".into(),
            ));
        }
        Ok(())
    }
    fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
        let id = self.inner.record(b)?;
        if self.mode == "partial" {
            std::fs::write(b.plan.changes[0].rename_to.as_ref().unwrap(), &flac()[..20]).unwrap();
            std::process::exit(81);
        }
        Ok(id)
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
    fn retain_interrupted(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
        self.inner.retain_interrupted(id, b)
    }
    fn prepare_copy_removal(
        &mut self,
        id: BatchId,
        t: &Path,
        u: &Path,
    ) -> Result<(), JournalError> {
        self.inner.prepare_copy_removal(id, t, u)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.inner.rollback(id)
    }
}
#[test]
fn checkpoint_child() {
    let Some(root) = std::env::var_os("TAGREX_COPY_CHECKPOINT_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var("TAGREX_COPY_CHECKPOINT_MODE").unwrap();
    let mut j = Proxy {
        inner: SqliteJournal::open(&root.join("journal.sqlite")).unwrap(),
        mode: mode.clone(),
        calls: 0,
    };
    let mut changes = plan(&root, mode != "mixed");
    if mode == "cover" {
        changes.changes[0].cover_change = Some(tagrex_core::plan::CoverChange {
            old: vec![],
            new: vec![tagrex_core::model::CoverArt {
                mime: "image/png".into(),
                data: PNG.to_vec().into(),
                ..Default::default()
            }],
        });
    }
    if mode == "block" {
        let old = tagrex_core::model::TagEngine::read_block(
            &root.join("source.flac"),
            tagrex_core::model::TagBlockKind::VorbisComments,
        )
        .unwrap()
        .unwrap();
        let mut new = old.clone();
        new.tags.insert(TagField::Artist, "After".into());
        changes.changes[0].tag_changes.clear();
        changes.changes[0]
            .block_changes
            .push(tagrex_core::plan::BlockChange {
                kind: tagrex_core::model::TagBlockKind::VorbisComments,
                revision: None,
                old_revision: None,
                old_bytes: None,
                old: Some(old),
                new: Some(new),
                text_to_unicode: false,
            });
    }
    if mode == "mixed" {
        std::fs::write(root.join("moving.flac"), flac()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("source.cue", root.join("moving.cue")).unwrap();
        #[cfg(not(unix))]
        std::fs::write(root.join("moving.cue"), b"ordinary sidecar").unwrap();
        changes.changes.push(FileChange {
            path: root.join("moving.flac"),
            rename_to: Some(root.join("moved.flac")),
            sidecar_renames: vec![(root.join("moving.cue"), root.join("moved.cue"))],
            ..Default::default()
        });
    }
    Executor::apply_with(
        &changes,
        &mut j,
        std::slice::from_ref(&root),
        &mut |p| {
            if (mode == "audio"
                && p.completed == Some(ApplyStep::Transfer)
                && p.path.extension().is_some_and(|e| e == "flac"))
                || (mode == "sidecar"
                    && p.completed == Some(ApplyStep::Transfer)
                    && p.path.extension().is_some_and(|e| e == "cue"))
                || (["tags", "cover", "block"].contains(&mode.as_str())
                    && p.completed == Some(ApplyStep::Tags))
                || (mode == "mixed"
                    && p.completed == Some(ApplyStep::Transfer)
                    && p.path == root.join("moving.cue"))
            {
                std::process::exit(81);
            }
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    panic!("exit phase not reached");
}
fn interrupted(mode: &str) -> (Fixture, BatchId) {
    let f = Fixture::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "checkpoint_child", "--nocapture"])
        .env("TAGREX_COPY_CHECKPOINT_ROOT", &f.0)
        .env("TAGREX_COPY_CHECKPOINT_MODE", mode)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(81));
            break;
        }
        if start.elapsed() > Duration::from_secs(15) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("fixture process exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let j = SqliteJournal::open(&f.db()).unwrap();
    let batch = j.batches().unwrap().remove(0);
    assert!(batch.interrupted);
    (f, batch.id)
}
#[test]
fn completed_audio_sidecar_and_metadata_checkpoints_survive_hard_exit() {
    for mode in ["audio", "sidecar", "tags", "after-sql"] {
        let (f, id) = interrupted(mode);
        let mut j = SqliteJournal::open(&f.db()).unwrap();
        let b = j.batch(id).unwrap().unwrap();
        let copy = b
            .copy_stamps
            .iter()
            .find(|s| s.path == f.0.join("copy.flac"))
            .unwrap();
        assert!(copy.sha256.as_ref().is_some_and(|h| h.len() == 32));
        assert!(copy.identity.is_some());
        assert_eq!(
            b.copy_stamps.len(),
            if ["sidecar", "tags"].contains(&mode) {
                2
            } else {
                1
            }
        );
        inspect(
            &f.0.join("copy.flac"),
            if mode == "tags" { "After" } else { "Before" },
        );
        inspect(&f.0.join("source.flac"), "Before");
        Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).unwrap();
        assert!(!f.0.join("copy.flac").exists());
        assert!(!f.0.join("copy.cue").exists());
        inspect(&f.0.join("source.flac"), "Before");
        assert_eq!(
            std::fs::read(f.0.join("source.cue")).unwrap(),
            b"cue original"
        );
        assert!(j.batches().unwrap().is_empty());
    }
}
#[test]
fn unknown_before_checkpoint_and_partial_copy_preserve_data_and_history() {
    for mode in ["before-sql", "partial"] {
        let (f, id) = interrupted(mode);
        let before = std::fs::read(f.0.join("copy.flac")).unwrap();
        let mut j = SqliteJournal::open(&f.db()).unwrap();
        assert!(j.batch(id).unwrap().unwrap().copy_stamps.is_empty());
        assert!(Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).is_err());
        assert_eq!(std::fs::read(f.0.join("copy.flac")).unwrap(), before);
        inspect(&f.0.join("source.flac"), "Before");
        assert!(j.batch(id).unwrap().is_some());
    }
}
#[test]
fn identical_foreign_replacement_requires_native_ownership_even_for_legacy_hash() {
    for legacy in [false, true] {
        let (f, id) = interrupted("sidecar");
        let target = f.0.join("copy.flac");
        let old = std::fs::metadata(&target).unwrap().modified().unwrap();
        std::fs::rename(&target, f.0.join("saved-owned.flac")).unwrap();
        std::fs::write(
            &target,
            std::fs::read(f.0.join("saved-owned.flac")).unwrap(),
        )
        .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&target)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
        if legacy {
            let conn = rusqlite::Connection::open(f.db()).unwrap();
            conn.execute(
                "UPDATE copy_stamps SET identity=NULL WHERE path=?1",
                [target.to_str().unwrap()],
            )
            .unwrap();
        }
        let mut j = SqliteJournal::open(&f.db()).unwrap();
        assert!(
            Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).is_err(),
            "foreign ownership accepted, legacy={legacy}"
        );
        assert!(target.exists());
        assert!(f.0.join("saved-owned.flac").exists());
        assert!(f.0.join("copy.cue").exists());
        assert!(j.batch(id).unwrap().is_some());
    }
}
#[test]
fn same_inode_edit_after_checkpoint_is_not_deleted() {
    let (f, id) = interrupted("tags");
    let p = f.0.join("copy.flac");
    let mut b = std::fs::read(&p).unwrap();
    *b.last_mut().unwrap() ^= 1;
    std::fs::write(&p, &b).unwrap();
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    assert!(Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).is_err());
    assert_eq!(std::fs::read(p).unwrap(), b);
    assert!(j.batch(id).unwrap().is_some());
}
#[test]
fn checkpoint_errors_roll_back_and_never_report_the_failed_step_complete() {
    for mode in [
        "error-before-sql",
        "error-after-sql",
        "error-before-refresh",
    ] {
        let f = Fixture::new();
        let mut j = Proxy {
            inner: SqliteJournal::open(&f.db()).unwrap(),
            mode: mode.into(),
            calls: 0,
        };
        let mut completions = 0;
        assert!(Executor::apply_with(
            &plan(&f.0, true),
            &mut j,
            std::slice::from_ref(&f.0),
            &mut |p| completions += usize::from(p.completed.is_some()),
            &AtomicBool::new(false)
        )
        .is_err());
        assert_eq!(
            completions,
            if mode == "error-before-refresh" { 2 } else { 0 }
        );
        assert!(!f.0.join("copy.flac").exists());
        assert!(!f.0.join("copy.cue").exists());
        assert!(j.batches().unwrap().is_empty());
        inspect(&f.0.join("source.flac"), "Before");
    }
}
#[test]
fn cancellation_after_checkpoint_rolls_back_audio_and_sidecar() {
    let f = Fixture::new();
    let cancel = AtomicBool::new(false);
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    assert!(Executor::apply_with(
        &plan(&f.0, true),
        &mut j,
        std::slice::from_ref(&f.0),
        &mut |p| {
            if p.completed == Some(ApplyStep::Transfer)
                && p.path.extension().is_some_and(|e| e == "cue")
            {
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel
    )
    .is_err());
    assert!(!f.0.join("copy.flac").exists());
    assert!(!f.0.join("copy.cue").exists());
    assert!(j.batches().unwrap().is_empty());
    inspect(&f.0.join("source.flac"), "Before");
}
#[test]
fn checkpoint_binds_plan_and_same_object_and_keeps_removal_intents() {
    let f = Fixture::new();
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let b = Executor::apply(&plan(&f.0, false), &mut j, std::slice::from_ref(&f.0)).unwrap();
    let stamp = b
        .copy_stamps
        .iter()
        .find(|s| s.path == f.0.join("copy.flac"))
        .unwrap()
        .clone();
    assert!(j.checkpoint_copy(b.id, &stamp, None).is_ok());
    let mut forged = stamp.clone();
    forged.path = f.0.join("unapproved.flac");
    assert!(j.checkpoint_copy(b.id, &forged, None).is_err());
    forged = stamp.clone();
    forged.identity = None;
    assert!(j.checkpoint_copy(b.id, &forged, Some(&stamp)).is_err());
    let removal = f.0.join("reserved/copy");
    j.prepare_copy_removal(b.id, &stamp.path, &removal).unwrap();
    assert!(j.checkpoint_copy(b.id, &stamp, Some(&stamp)).is_err());
    assert_eq!(
        j.batch(b.id)
            .unwrap()
            .unwrap()
            .copy_stamps
            .iter()
            .find(|s| s.path == stamp.path)
            .unwrap()
            .undo_path
            .as_ref(),
        Some(&removal)
    );
}
#[test]
fn unsupported_adapter_refuses_before_record_or_copy() {
    struct Foreign(VecJournal);
    impl UndoJournal for Foreign {
        fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
            self.0.record(b)
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            self.0.batches()
        }
        fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
            self.0.finish(id, b)
        }
        fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
            self.0.rollback(id)
        }
    }
    let f = Fixture::new();
    let mut j = Foreign(VecJournal::new());
    assert!(Executor::apply(&plan(&f.0, false), &mut j, std::slice::from_ref(&f.0)).is_err());
    assert!(j.batches().unwrap().is_empty());
    assert!(!f.0.join("copy.flac").exists());
}

#[test]
fn crash_after_copy_metadata_before_its_refresh_refuses_unknown_final_bytes() {
    let (f, id) = interrupted("before-refresh");
    let target = f.0.join("copy.flac");
    inspect(&target, "After");
    let bytes = std::fs::read(&target).unwrap();
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    assert!(Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).is_err());
    assert_eq!(std::fs::read(target).unwrap(), bytes);
    assert!(j.batch(id).unwrap().is_some());
    inspect(&f.0.join("source.flac"), "Before");
}

#[test]
fn sqlite_busy_checkpoint_preserves_all_data_and_previous_history() {
    struct Locked {
        inner: SqliteJournal,
        lock: Option<rusqlite::Connection>,
        db: PathBuf,
    }
    impl UndoJournal for Locked {
        fn supports_copy_checkpoints(&self) -> bool {
            true
        }
        fn checkpoint_copy(
            &mut self,
            id: BatchId,
            s: &CopyStamp,
            p: Option<&CopyStamp>,
        ) -> Result<(), JournalError> {
            let lock = rusqlite::Connection::open(&self.db).unwrap();
            lock.execute_batch("BEGIN IMMEDIATE").unwrap();
            self.lock = Some(lock);
            self.inner.checkpoint_copy(id, s, p)
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
            t: &Path,
            u: &Path,
        ) -> Result<(), JournalError> {
            self.inner.prepare_copy_removal(id, t, u)
        }
        fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
            self.inner.rollback(id)
        }
    }
    let f = Fixture::new();
    let mut inner = SqliteJournal::open(&f.db()).unwrap();
    let prior = Executor::apply(
        &ChangePlan::default(),
        &mut inner,
        std::slice::from_ref(&f.0),
    )
    .unwrap();
    let mut j = Locked {
        inner,
        lock: None,
        db: f.db(),
    };
    let mut completions = 0;
    let error = Executor::apply_with(
        &plan(&f.0, false),
        &mut j,
        std::slice::from_ref(&f.0),
        &mut |p| completions += usize::from(p.completed.is_some()),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        tagrex_core::plan::PlanError::RollbackIncomplete { .. }
    ));
    assert_eq!(completions, 0);
    drop(j.lock.take());
    drop(j);
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    assert!(j.batch(prior.id).unwrap().is_some());
    let failed = j.batches().unwrap().remove(0);
    assert!(failed.interrupted);
    assert!(failed.copy_stamps.is_empty());
    assert!(Executor::undo(&mut j, failed.id, std::slice::from_ref(&f.0)).is_err());
    inspect(&f.0.join("copy.flac"), "Before");
    inspect(&f.0.join("source.flac"), "Before");
    assert_eq!(j.batches().unwrap().len(), 2);
}

#[test]
fn many_folder_copy_checkpoints_are_counted_bounded_and_recoverable() {
    let f = Fixture::new();
    let mut changes = vec![];
    for n in 0..256 {
        let folder = f.0.join(format!("album-{n}"));
        std::fs::create_dir(&folder).unwrap();
        let source = folder.join("source.flac");
        std::fs::write(&source, flac()).unwrap();
        changes.push(FileChange {
            path: source,
            rename_to: Some(folder.join("copy.flac")),
            copy: true,
            ..Default::default()
        });
    }
    let plan = ChangePlan {
        changes,
        ..Default::default()
    };
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let start = Instant::now();
    let count_fds = || {
        std::fs::read_dir("/proc/self/fd")
            .ok()
            .map(|entries| entries.count())
    };
    let before_fds = count_fds();
    let mut peak_fds = before_fds;
    let mut completed = 0;
    let batch = Executor::apply_with(
        &plan,
        &mut j,
        std::slice::from_ref(&f.0),
        &mut |p| {
            completed += usize::from(p.completed.is_some());
            if let Some(current) = count_fds() {
                peak_fds = Some(peak_fds.unwrap_or(0).max(current));
            }
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(completed, 256);
    assert_eq!(batch.copy_stamps.len(), 256);
    let applied = start.elapsed();
    drop(j);
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let start = Instant::now();
    Executor::undo(&mut j, batch.id, std::slice::from_ref(&f.0)).unwrap();
    eprintln!(
        "256-folder durable copies: apply={applied:?},FD_before={before_fds:?},FD_peak={peak_fds:?},undo={:?}",
        start.elapsed()
    );
    for change in &plan.changes {
        inspect(&change.path, "Before");
        assert!(!change.rename_to.as_ref().unwrap().exists());
    }
    assert!(j.batches().unwrap().is_empty());
}

#[test]
fn mixed_copy_move_and_link_entry_recover_after_process_exit() {
    let (f, id) = interrupted("mixed");
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let b = j.batch(id).unwrap().unwrap();
    assert_eq!(b.copy_stamps.iter().filter(|s| !s.moved).count(), 2);
    assert_eq!(b.copy_stamps.iter().filter(|s| s.moved).count(), 2);
    assert!(f.0.join("moved.flac").exists());
    Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).unwrap();
    inspect(&f.0.join("moving.flac"), "Before");
    inspect(&f.0.join("source.flac"), "Before");
    assert!(!f.0.join("moved.flac").exists());
    assert!(!f.0.join("copy.flac").exists());
    assert!(!f.0.join("copy.cue").exists());
    #[cfg(unix)]
    assert_eq!(
        std::fs::read_link(f.0.join("moving.cue")).unwrap(),
        Path::new("source.cue")
    );
    assert_eq!(
        std::fs::read(f.0.join("source.cue")).unwrap(),
        b"cue original"
    );
    assert!(j.batches().unwrap().is_empty());
}

#[test]
fn malformed_checkpoint_profiles_and_stale_refresh_tokens_preserve_existing_evidence() {
    let f = Fixture::new();
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let b = Executor::apply(&plan(&f.0, false), &mut j, std::slice::from_ref(&f.0)).unwrap();
    let old = b
        .copy_stamps
        .iter()
        .find(|s| s.path == f.0.join("copy.flac"))
        .unwrap()
        .clone();
    let mut variants = vec![];
    for hash in [None, Some(vec![0; 31]), Some(vec![0; 33])] {
        let mut s = old.clone();
        s.sha256 = hash;
        variants.push(s);
    }
    let mut s = old.clone();
    s.moved = true;
    variants.push(s);
    let mut s = old.clone();
    s.undo_path = Some(f.0.join("unapproved/copy"));
    variants.push(s);
    let mut s = old.clone();
    s.identity = None;
    variants.push(s);
    for kind in [1, 2, 255] {
        let mut s = old.clone();
        *s.identity.as_mut().unwrap().last_mut().unwrap() = kind;
        variants.push(s);
    }
    let mut s = old.clone();
    s.identity.as_mut().unwrap()[5] = b'9';
    variants.push(s);
    for s in variants {
        assert!(j.checkpoint_copy(b.id, &s, Some(&old)).is_err());
        assert_eq!(j.batch(b.id).unwrap().unwrap().copy_stamps, b.copy_stamps);
    }
    let mut changed = old.clone();
    changed.modified_ns += 1;
    let mut stale = old.clone();
    stale.modified_ns -= 1;
    assert!(j.checkpoint_copy(b.id, &changed, Some(&stale)).is_err());
    assert_eq!(j.batch(b.id).unwrap().unwrap().copy_stamps, b.copy_stamps);
    let mut weak_previous = old.clone();
    weak_previous.sha256 = None;
    assert!(j.checkpoint_copy(b.id, &old, Some(&weak_previous)).is_err());
    assert_eq!(j.batch(b.id).unwrap().unwrap().copy_stamps, b.copy_stamps);
    // Duplicate stored rows cannot be used to overwrite one selected witness.
    let conn = rusqlite::Connection::open(f.db()).unwrap();
    conn.execute("INSERT INTO copy_stamps(batch_id,path,len,modified_ns,sha256,undo_path,identity,moved) SELECT batch_id,path,len,modified_ns,sha256,undo_path,identity,moved FROM copy_stamps WHERE batch_id=?1 AND path=?2",rusqlite::params![b.id.0,old.path.to_str().unwrap()]).unwrap();
    assert!(j.checkpoint_copy(b.id, &old, None).is_err());
    assert_eq!(
        j.batch(b.id).unwrap().unwrap().copy_stamps.len(),
        b.copy_stamps.len() + 1
    );
}

const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 16, 84, 50, 254, 15, 0, 2,
    20, 1, 102, 4, 224, 222, 127, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
#[test]
fn copied_cover_and_block_outcomes_checkpoint_before_completion() {
    for mode in ["cover", "block"] {
        let (f, id) = interrupted(mode);
        let target = f.0.join("copy.flac");
        inspect(&target, "After");
        if mode == "cover" {
            let b = std::fs::read(&target).unwrap();
            assert!(b.windows(PNG.len()).any(|window| window == PNG));
            assert!(!std::fs::read(f.0.join("source.flac"))
                .unwrap()
                .windows(PNG.len())
                .any(|window| window == PNG));
        }
        let mut j = SqliteJournal::open(&f.db()).unwrap();
        Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).unwrap();
        assert!(!target.exists());
        inspect(&f.0.join("source.flac"), "Before");
        assert!(j.batches().unwrap().is_empty());
    }
}
#[test]
fn large_sidecar_streaming_checkpoint_survives_reopen_and_cleanup() {
    let f = Fixture::new();
    let bytes = vec![0x5a; 8 * 1024 * 1024];
    std::fs::write(f.0.join("source.cue"), &bytes).unwrap();
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let batch = Executor::apply(&plan(&f.0, false), &mut j, std::slice::from_ref(&f.0)).unwrap();
    let stamp = batch
        .copy_stamps
        .iter()
        .find(|s| s.path == f.0.join("copy.cue"))
        .unwrap();
    assert_eq!(stamp.len, bytes.len() as u64);
    assert_eq!(std::fs::read(f.0.join("copy.cue")).unwrap(), bytes);
    drop(j);
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    Executor::undo(&mut j, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(std::fs::read(f.0.join("source.cue")).unwrap(), bytes);
    assert!(!f.0.join("copy.cue").exists());
    assert!(j.batches().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn actual_file_size_limit_death_keeps_partial_sidecar_unproved_and_all_history() {
    use std::os::unix::process::ExitStatusExt;
    let f = Fixture::new();
    std::fs::write(f.0.join("source.cue"), vec![0x6b; 8 * 1024 * 1024]).unwrap();
    // A per-child kernel file-size limit, after fixture creation. This does not
    // change a mount or any other process's limits and disables core files.
    let status = Command::new("bash")
        .args([
            "-c",
            "ulimit -c 0\nulimit -f 1024\nexec \"$1\" --exact checkpoint_child --nocapture",
            "copy-size-fixture",
        ])
        .arg(std::env::current_exe().unwrap())
        .env("TAGREX_COPY_CHECKPOINT_ROOT", &f.0)
        .env("TAGREX_COPY_CHECKPOINT_MODE", "size-limit")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(
        status.signal(),
        Some(25),
        "expected SIGXFSZ from this owned child's write limit: {status:?}"
    );
    let partial = f.0.join("copy.cue");
    let before = std::fs::read(&partial).unwrap();
    assert!(!before.is_empty());
    assert!(before.len() < 8 * 1024 * 1024);
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let b = j.batches().unwrap().remove(0);
    assert!(b.interrupted);
    assert_eq!(b.copy_stamps.len(), 1);
    assert_eq!(b.copy_stamps[0].path, f.0.join("copy.flac"));
    assert!(Executor::undo(&mut j, b.id, std::slice::from_ref(&f.0)).is_err());
    assert_eq!(std::fs::read(partial).unwrap(), before);
    inspect(&f.0.join("copy.flac"), "Before");
    inspect(&f.0.join("source.flac"), "Before");
    assert_eq!(
        std::fs::metadata(f.0.join("source.cue")).unwrap().len(),
        8 * 1024 * 1024
    );
    assert!(j.batch(b.id).unwrap().is_some());
}
