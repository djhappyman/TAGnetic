//! Process-death and failure tests use only disposable, independently inspected
//! FLAC fixtures. Child exit intentionally bypasses every Rust destructor.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tagrex_core::journal::{AppliedBatch, BatchId, JournalError, SqliteJournal, UndoJournal};
use tagrex_core::model::TagField;
use tagrex_core::plan::{
    ApplyStep, ChangePlan, Executor, FieldChange, FileChange, PlanError, UndoMode,
};

const FRAME: &[u8] = &[
    0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
];
const EXIT: i32 = 73;

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-recovery-stress-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let fixture = Self(path);
        for name in ["a.flac", "b.flac"] {
            std::fs::write(fixture.0.join(name), flac()).unwrap();
        }
        std::fs::write(fixture.0.join("a.lrc"), b"original sidecar\n").unwrap();
        fixture
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn flac() -> Vec<u8> {
    // STREAMINFO and one valid stereo constant frame, with our own Vorbis
    // comment encoder so the baseline does not depend on the writer under test.
    let mut bytes = b"fLaC\x00\x00\x00\x22".to_vec();
    bytes.extend([
        0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    let vendor = b"TagRex disposable crash fixture";
    let mut comments = (vendor.len() as u32).to_le_bytes().to_vec();
    comments.extend(vendor);
    comments.extend(2u32.to_le_bytes());
    for value in ["ARTIST=Before", "TITLE=Keep Jóga"] {
        comments.extend((value.len() as u32).to_le_bytes());
        comments.extend(value.as_bytes());
    }
    bytes.push(0x84);
    bytes.extend(&(comments.len() as u32).to_be_bytes()[1..]);
    bytes.extend(comments);
    bytes.extend(FRAME);
    bytes
}

fn inspect(path: &Path, expected_artist: &str) {
    // Independent raw FLAC/Vorbis decoder: check exact encoded payload and an
    // unrelated Unicode field, not a readback through TagEngine/Lofty.
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"fLaC");
    let mut offset = 4;
    let mut fields = BTreeMap::new();
    loop {
        let kind = bytes[offset];
        let len = u32::from_be_bytes([0, bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
            as usize;
        let block = &bytes[offset + 4..offset + 4 + len];
        if kind & 0x7f == 4 {
            let mut at = 0;
            let number = |at: &mut usize| {
                let value = u32::from_le_bytes(block[*at..*at + 4].try_into().unwrap()) as usize;
                *at += 4;
                value
            };
            let vendor = number(&mut at);
            at += vendor;
            let count = number(&mut at);
            for _ in 0..count {
                let len = number(&mut at);
                let value = std::str::from_utf8(&block[at..at + len]).unwrap();
                at += len;
                let (key, value) = value.split_once('=').unwrap();
                fields.insert(key.to_ascii_uppercase(), value.to_string());
            }
        }
        offset += len + 4;
        if kind & 0x80 != 0 {
            break;
        }
    }
    assert_eq!(&bytes[offset..], FRAME, "encoded audio was changed");
    assert_eq!(
        fields.get("ARTIST").map(String::as_str),
        Some(expected_artist)
    );
    assert_eq!(fields.get("TITLE").map(String::as_str), Some("Keep Jóga"));
}

fn plan(root: &Path, scenario: &str) -> ChangePlan {
    let moving = scenario.contains("move") || scenario.contains("sidecar");
    ChangePlan {
        description: format!("crash probe {scenario}"),
        changes: ["a.flac", "b.flac"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| FileChange {
                path: root.join(name),
                tag_changes: if moving {
                    Vec::new()
                } else {
                    vec![FieldChange {
                        field: TagField::Artist,
                        old: Some("Before".into()),
                        new: Some("After 東京".into()),
                    }]
                },
                rename_to: moving.then(|| root.join(format!("moved/{name}"))),
                sidecar_renames: if moving && index == 0 {
                    vec![(root.join("a.lrc"), root.join("moved/a.lrc"))]
                } else {
                    Vec::new()
                },
                ..FileChange::default()
            })
            .collect(),
        ..ChangePlan::default()
    }
}

struct CrashJournal {
    inner: SqliteJournal,
    scenario: String,
}
impl UndoJournal for CrashJournal {
    fn supports_copy_checkpoints(&self) -> bool {
        self.inner.supports_copy_checkpoints()
    }
    fn checkpoint_copy(
        &mut self,
        id: BatchId,
        stamp: &tagrex_core::journal::CopyStamp,
        previous: Option<&tagrex_core::journal::CopyStamp>,
    ) -> Result<(), tagrex_core::journal::JournalError> {
        self.inner.checkpoint_copy(id, stamp, previous)
    }

    fn prepare_move_stage(
        &mut self,
        id: BatchId,
        stage: &tagrex_core::journal::MoveStage,
    ) -> Result<(), tagrex_core::journal::JournalError> {
        self.inner.prepare_move_stage(id, stage)
    }
    fn move_stages(
        &self,
        id: BatchId,
    ) -> Result<Vec<tagrex_core::journal::MoveStage>, tagrex_core::journal::JournalError> {
        self.inner.move_stages(id)
    }

    fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError> {
        if self.scenario == "before-record" {
            std::process::exit(EXIT);
        }
        let id = self.inner.record(batch)?;
        if self.scenario == "after-record" {
            std::process::exit(EXIT);
        }
        Ok(id)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.inner.batches()
    }
    fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
        self.inner.batch(id)
    }
    fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError> {
        if self.scenario.ends_with("before-finish") {
            std::process::exit(EXIT);
        }
        self.inner.finish(id, batch)?;
        if self.scenario == "after-finish" {
            std::process::exit(EXIT);
        }
        Ok(())
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        if self.scenario == "undo-before-delete" {
            std::process::exit(EXIT);
        }
        self.inner.rollback(id)
    }
}

#[test]
fn crash_child_entry() {
    let Some(root) = std::env::var_os("TAGREX_CRASH_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let scenario = std::env::var("TAGREX_CRASH_TEST_SCENARIO").unwrap();
    let mut journal = CrashJournal {
        inner: SqliteJournal::open(&root.join("journal.sqlite")).unwrap(),
        scenario: scenario.clone(),
    };
    let plan = plan(&root, &scenario);
    let roots = [root.clone()];
    if scenario.starts_with("undo-") {
        let batch = Executor::apply(&plan, &mut journal, &roots).unwrap();
        Executor::undo_with(&mut journal, batch.id, &roots, &mut |progress| {
            if scenario == "undo-after-first" && progress.done == 1 {
                std::process::exit(EXIT);
            }
        })
        .unwrap();
    } else {
        Executor::apply_with(
            &plan,
            &mut journal,
            &roots,
            &mut |progress| {
                let crash = match scenario.as_str() {
                    "after-first-write" => progress.completed == Some(ApplyStep::Tags),
                    "move-after-first" => progress.completed == Some(ApplyStep::Transfer),
                    "sidecar-after-transfer" => {
                        progress.completed == Some(ApplyStep::Transfer)
                            && progress.path.extension().is_some_and(|ext| ext == "lrc")
                    }
                    _ => false,
                };
                if crash {
                    std::process::exit(EXIT);
                }
            },
            &AtomicBool::new(false),
        )
        .unwrap();
    }
    panic!("crash boundary was never reached: {scenario}");
}

fn crash(root: &Path, scenario: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child_entry", "--nocapture"])
        .env("TAGREX_CRASH_TEST_ROOT", root)
        .env("TAGREX_CRASH_TEST_SCENARIO", scenario)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(EXIT), "{scenario}");
            break;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("child timed out: {scenario}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn real_process_death_preserves_journal_and_retry_boundaries() {
    for round in 0..3 {
        for scenario in [
            "before-record",
            "after-record",
            "after-first-write",
            "before-finish",
            "after-finish",
            "move-after-first",
            "move-before-finish",
            "sidecar-after-transfer",
            "undo-after-first",
            "undo-before-delete",
        ] {
            let fixture = Fixture::new(&format!("{round}-{scenario}"));
            crash(&fixture.0, scenario);
            let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
            let batches = journal.batches().unwrap();
            for (index, name) in ["a.flac", "b.flac"].iter().enumerate() {
                let original = fixture.0.join(name);
                let actual = if original.exists() {
                    original
                } else {
                    fixture.0.join("moved").join(name)
                };
                let applied = scenario == "after-finish"
                    || scenario == "before-finish"
                    || (scenario == "after-first-write" && index == 0)
                    || (scenario == "undo-after-first" && index == 1);
                inspect(&actual, if applied { "After 東京" } else { "Before" });
            }
            if scenario == "before-record" {
                assert!(batches.is_empty());
            } else {
                assert_eq!(batches.len(), 1, "{scenario}");
                let batch = &batches[0];
                assert_eq!(
                    batch.interrupted,
                    !scenario.starts_with("undo-") && scenario != "after-finish",
                    "{scenario}"
                );
                if ["after-first-write", "before-finish"].contains(&scenario) {
                    let before: Vec<_> = ["a.flac", "b.flac"]
                        .iter()
                        .map(|name| std::fs::read(fixture.0.join(name)).unwrap())
                        .collect();
                    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
                        assert!(
                            matches!(
                                Executor::undo_with_mode(
                                    &mut journal,
                                    batch.id,
                                    std::slice::from_ref(&fixture.0),
                                    mode,
                                    &mut |_| {}
                                ),
                                Err(PlanError::ChangedSinceApplied(_))
                            ),
                            "unknown outcomes must remain fail-closed: {scenario}"
                        );
                        for (index, name) in ["a.flac", "b.flac"].iter().enumerate() {
                            assert_eq!(std::fs::read(fixture.0.join(name)).unwrap(), before[index]);
                        }
                    }
                    // Explicitly restore the exact known source evidence, then
                    // prove that retrying ordinary Undo is safe and idempotent.
                    for name in ["a.flac", "b.flac"] {
                        std::fs::write(fixture.0.join(name), flac()).unwrap();
                    }
                }
                Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
                assert!(journal.batches().unwrap().is_empty());
                assert!(matches!(
                    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)),
                    Err(PlanError::Journal(JournalError::UnknownBatch(_)))
                ));
            }
            for name in ["a.flac", "b.flac"] {
                inspect(&fixture.0.join(name), "Before");
            }
            assert_eq!(
                std::fs::read(fixture.0.join("a.lrc")).unwrap(),
                b"original sidecar\n"
            );
            assert!(!fixture.0.join("moved/a.flac").exists());
            assert!(!fixture.0.join("moved/a.lrc").exists());
        }
    }
}

#[test]
fn cancellation_after_one_sidecar_rolls_back_all_transfers() {
    let fixture = Fixture::new("sidecar-cancel");
    let mut plan = plan(&fixture.0, "sidecar");
    for index in 0..128 {
        let from = fixture.0.join(format!("sidecar-{index}.txt"));
        std::fs::write(&from, format!("sidecar {index}")).unwrap();
        plan.changes[0]
            .sidecar_renames
            .push((from, fixture.0.join(format!("moved/sidecar-{index}.txt"))));
    }
    let cancel = AtomicBool::new(false);
    let mut seen = 0;
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let result = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if progress.completed == Some(ApplyStep::Transfer)
                && progress.path.extension().is_some_and(|ext| ext == "lrc")
            {
                seen += 1;
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(
        matches!(result, Err(PlanError::Cancelled)),
        "sidecar cancellation was ignored"
    );
    assert_eq!(seen, 1);
    assert!(journal.batches().unwrap().is_empty());
    for change in &plan.changes {
        inspect(&change.path, "Before");
        assert!(!change.rename_to.as_ref().unwrap().exists());
        for (from, to) in &change.sidecar_renames {
            assert!(from.is_file());
            assert!(!to.exists());
        }
    }
}

#[test]
fn recovery_never_overwrites_a_previous_damaged_database() {
    let fixture = Fixture::new("quarantine-collision");
    let db = fixture.0.join("journal.sqlite");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let existing: Vec<_> = (now.saturating_sub(1)..=now + 10)
        .map(|stamp| {
            let path = fixture.0.join(format!("journal.sqlite.corrupt-{stamp}"));
            std::fs::write(&path, b"previous irreplaceable recovery evidence").unwrap();
            path
        })
        .collect();
    std::fs::write(&db, b"new damaged database").unwrap();
    let (_, aside) = SqliteJournal::open_or_recover(&db).unwrap();
    assert_eq!(
        std::fs::read(aside.unwrap()).unwrap(),
        b"new damaged database"
    );
    for path in existing {
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"previous irreplaceable recovery evidence"
        );
    }
}

#[test]
fn finish_retries_replace_outcomes_instead_of_accumulating() {
    let fixture = Fixture::new("finish-retry");
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let batch = Executor::apply(
        &plan(&fixture.0, "finish"),
        &mut journal,
        std::slice::from_ref(&fixture.0),
    )
    .unwrap();
    for _ in 0..5 {
        journal.finish(batch.id, &batch).unwrap();
        let stored = journal.batch(batch.id).unwrap().unwrap();
        assert_eq!(
            stored.allowed_roots, batch.allowed_roots,
            "finish must replace initial and previously retained outcomes"
        );
        assert_eq!(stored.applied_values, batch.applied_values);
    }
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
    for name in ["a.flac", "b.flac"] {
        inspect(&fixture.0.join(name), "Before");
    }
}

#[test]
fn damaged_recovery_refuses_nonregular_sqlite_companions() {
    for suffix in ["-wal", "-shm", "-journal"] {
        let fixture = Fixture::new("companion-type");
        let db = fixture.0.join("journal.sqlite");
        std::fs::write(&db, b"damaged database").unwrap();
        let companion = fixture.0.join(format!("journal.sqlite{suffix}"));
        std::fs::create_dir(&companion).unwrap();
        std::fs::write(companion.join("unrelated.txt"), b"unrelated contents").unwrap();
        assert!(SqliteJournal::open_or_recover(&db).is_err());
        assert_eq!(std::fs::read(db).unwrap(), b"damaged database");
        assert_eq!(
            std::fs::read(companion.join("unrelated.txt")).unwrap(),
            b"unrelated contents"
        );
    }
}

#[test]
fn sqlite_completion_write_failure_rolls_media_back_and_allows_retry() {
    let fixture = Fixture::new("sqlite-completion-failure");
    let db = fixture.0.join("journal.sqlite");
    let mut journal = SqliteJournal::open(&db).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE OF interrupted ON batches WHEN NEW.interrupted = 0 BEGIN SELECT RAISE(FAIL, 'injected journal completion failure'); END;").unwrap();
    let plan = plan(&fixture.0, "finish-failure");
    let error = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap_err();
    assert!(error
        .to_string()
        .contains("injected journal completion failure"));
    assert!(journal.batches().unwrap().is_empty());
    for name in ["a.flac", "b.flac"] {
        inspect(&fixture.0.join(name), "Before");
    }
    connection
        .execute_batch("DROP TRIGGER reject_completion")
        .unwrap();
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap();
    for name in ["a.flac", "b.flac"] {
        inspect(&fixture.0.join(name), "After 東京");
    }
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
    for name in ["a.flac", "b.flac"] {
        inspect(&fixture.0.join(name), "Before");
    }
}

#[test]
fn sidecar_target_created_after_preflight_survives_failed_apply() {
    for copy in [false, true] {
        let fixture = Fixture::new("sidecar-collision");
        let mut plan = plan(&fixture.0, "sidecar");
        let source = fixture.0.join("second.txt");
        let collision = fixture.0.join("moved/second.txt");
        std::fs::write(&source, b"original second sidecar").unwrap();
        plan.changes[0]
            .sidecar_renames
            .push((source.clone(), collision.clone()));
        for change in &mut plan.changes {
            change.copy = copy;
        }
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        let error = Executor::apply_with(
            &plan,
            &mut journal,
            std::slice::from_ref(&fixture.0),
            &mut |progress| {
                if progress.completed == Some(ApplyStep::Transfer)
                    && progress.path.extension().is_some_and(|ext| ext == "lrc")
                {
                    std::fs::write(&collision, b"unrelated new contents").unwrap();
                }
            },
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(matches!(error, PlanError::RenameCollision(_)));
        assert_eq!(
            std::fs::read(&collision).unwrap(),
            b"unrelated new contents"
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"original second sidecar");
        assert_eq!(
            std::fs::read(fixture.0.join("a.lrc")).unwrap(),
            b"original sidecar\n"
        );
        for change in &plan.changes {
            inspect(&change.path, "Before");
            assert!(!change.rename_to.as_ref().unwrap().exists());
        }
        assert!(journal.batches().unwrap().is_empty());
    }
}

#[test]
fn copy_replacement_after_undo_preflight_is_not_deleted() {
    let fixture = Fixture::new("undo-copy-race");
    let target = fixture.0.join("copy.flac");
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: fixture.0.join("a.flac"),
            rename_to: Some(target.clone()),
            copy: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap();
    let modified = std::fs::metadata(&target).unwrap().modified().unwrap();
    let mut external = flac();
    *external.last_mut().unwrap() ^= 1;
    let mut injected = false;
    let result = Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&fixture.0),
        &mut |_| {
            if !injected {
                std::fs::remove_file(&target).unwrap();
                std::fs::write(&target, &external).unwrap();
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&target)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(modified))
                    .unwrap();
                injected = true;
            }
        },
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(&target).unwrap(), external);
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[test]
fn failed_apply_does_not_remove_an_externally_replaced_copy() {
    let fixture = Fixture::new("rollback-copy-race");
    let target = fixture.0.join("copy/a.flac");
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: fixture.0.join("a.flac"),
            rename_to: Some(target.clone()),
            copy: true,
            sidecar_renames: vec![(fixture.0.join("a.lrc"), fixture.0.join("copy/a.lrc"))],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let cancel = AtomicBool::new(false);
    let external = b"irreplaceable external replacement";
    let error = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if progress.completed == Some(ApplyStep::Transfer)
                && progress.path == fixture.0.join("a.flac")
            {
                std::fs::remove_file(&target).unwrap();
                std::fs::write(&target, external).unwrap();
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert!(matches!(error, PlanError::RollbackIncomplete { .. }));
    assert_eq!(std::fs::read(&target).unwrap(), external);
    let batch = journal.batches().unwrap().remove(0);
    assert!(batch.interrupted);
    assert_eq!(batch.copy_stamps.len(), 1);
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    assert_eq!(std::fs::read(target).unwrap(), external);
}

#[test]
fn failed_move_rollback_never_overwrites_a_recreated_original() {
    let fixture = Fixture::new("rollback-move-race");
    let source = fixture.0.join("a.flac");
    let target = fixture.0.join("move/a.flac");
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            rename_to: Some(target.clone()),
            sidecar_renames: vec![(fixture.0.join("a.lrc"), fixture.0.join("move/a.lrc"))],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let cancel = AtomicBool::new(false);
    let error = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if progress.completed == Some(ApplyStep::Transfer) && progress.path == source {
                std::fs::write(&source, b"new original path occupant").unwrap();
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert!(matches!(error, PlanError::RollbackIncomplete { .. }));
    assert_eq!(
        std::fs::read(&source).unwrap(),
        b"new original path occupant"
    );
    inspect(&target, "Before");
    assert_eq!(journal.batches().unwrap().len(), 1);
}

#[test]
fn unversioned_migration_failure_is_transactional() {
    let fixture = Fixture::new("schema-rollback");
    let db = fixture.0.join("journal.sqlite");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TABLE batches(id INTEGER PRIMARY KEY, description TEXT NOT NULL, applied_at INTEGER NOT NULL); INSERT INTO batches VALUES(1,'original history',1); CREATE VIEW file_changes AS SELECT id FROM batches;").unwrap();
    drop(connection);
    let before = std::fs::read(&db).unwrap();
    assert!(SqliteJournal::open_or_recover(&db).is_err());
    assert_eq!(std::fs::read(db).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn non_unicode_paths_are_refused_before_any_journal_or_media_mutation() {
    use std::os::unix::ffi::OsStringExt;
    fn create_native_entry(result: std::io::Result<()>) {
        match result {
            Ok(()) => {}
            Err(error) if cfg!(target_vendor = "apple") && error.raw_os_error() == Some(92) => {
                // APFS cannot create invalid UTF-8 filenames. The invalid path
                // still enters the public plan and must fail before mutation.
            }
            Err(error) => panic!("could not create native-byte fixture entry: {error}"),
        }
    }
    for variant in ["audio", "destination", "sidecar", "root"] {
        let fixture = Fixture::new("native-path-refusal");
        let invalid = fixture
            .0
            .join(std::ffi::OsString::from_vec(b"invalid-\xff.flac".to_vec()));
        let mut roots = vec![fixture.0.clone()];
        let mut plan = ChangePlan {
            changes: vec![FileChange {
                path: fixture.0.join("a.flac"),
                rename_to: Some(fixture.0.join("target.flac")),
                ..Default::default()
            }],
            ..Default::default()
        };
        match variant {
            "audio" => {
                create_native_entry(std::fs::write(&invalid, flac()));
                plan.changes[0].path = invalid.clone();
            }
            "destination" => plan.changes[0].rename_to = Some(invalid.clone()),
            "sidecar" => {
                create_native_entry(std::fs::write(&invalid, b"sidecar"));
                plan.changes[0]
                    .sidecar_renames
                    .push((invalid.clone(), fixture.0.join("target.txt")));
            }
            "root" => {
                create_native_entry(std::fs::create_dir(&invalid));
                roots = vec![invalid.clone()];
            }
            _ => unreachable!(),
        }
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        let watched: Vec<_> = ["a.flac", "b.flac", "a.lrc"]
            .into_iter()
            .map(|name| {
                let path = fixture.0.join(name);
                let bytes = std::fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        let invalid_before = std::fs::read(&invalid).ok();
        let invalid_exists = std::fs::symlink_metadata(&invalid).is_ok();
        let error = Executor::apply(&plan, &mut journal, &roots).unwrap_err();
        assert!(error.to_string().contains("Unicode"));
        for (path, before) in watched {
            assert_eq!(std::fs::read(path).unwrap(), before);
        }
        assert_eq!(std::fs::read(&invalid).ok(), invalid_before);
        assert_eq!(std::fs::symlink_metadata(&invalid).is_ok(), invalid_exists);
        if variant == "root" && invalid_exists {
            assert!(std::fs::read_dir(&invalid).unwrap().next().is_none());
        }
        assert!(journal.batches().unwrap().is_empty());
        assert!(!fixture.0.join("target.flac").exists());
        assert!(!fixture.0.join("target.txt").exists());
    }
}

#[cfg(unix)]
#[test]
fn ancestor_replacement_cannot_redirect_apply_or_metadata_undo() {
    use tagrex_core::model::{TagBlockKind, TagEngine};
    use tagrex_core::plan::BlockChange;
    for operation in [
        "apply-tags",
        "undo-tags",
        "apply-move",
        "apply-copy",
        "apply-block",
        "undo-block",
    ] {
        let fixture = Fixture::new("ancestor-metadata");
        let allowed = fixture.0.join("allowed");
        let sub = allowed.join("sub");
        let outside = fixture.0.join("outside");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let source = sub.join("track.flac");
        let target = allowed.join("destination/track.flac");
        std::fs::write(&source, flac()).unwrap();
        let mut change = FileChange {
            path: source.clone(),
            ..Default::default()
        };
        if operation.ends_with("tags") {
            change.tag_changes.push(FieldChange {
                field: TagField::Artist,
                old: Some("Before".into()),
                new: Some("After 東京".into()),
            });
        } else if operation.ends_with("block") {
            change.block_changes.push(BlockChange::removal(
                TagBlockKind::VorbisComments,
                TagEngine::read_block(&source, TagBlockKind::VorbisComments)
                    .unwrap()
                    .unwrap(),
            ));
        } else {
            change.rename_to = Some(target.clone());
            change.copy = operation == "apply-copy";
        }
        let plan = ChangePlan {
            changes: vec![change],
            ..Default::default()
        };
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        let undo = operation.starts_with("undo");
        let id = if undo {
            Some(
                Executor::apply(&plan, &mut journal, std::slice::from_ref(&allowed))
                    .unwrap()
                    .id,
            )
        } else {
            None
        };
        let saved = std::fs::read(&source).unwrap();
        let outside_file = outside.join("track.flac");
        std::fs::write(&outside_file, &saved).unwrap();
        let backup = allowed.join("sub-before");
        let mut injected = false;
        let mut progress = |event: &tagrex_core::plan::ApplyProgress| {
            if !injected
                && event.path == source
                && event.completed.is_none()
                && (undo || event.done >= plan.changes.len())
            {
                std::fs::rename(&sub, &backup).unwrap();
                std::os::unix::fs::symlink(&outside, &sub).unwrap();
                injected = true;
            }
        };
        let result = match id {
            Some(id) => Executor::undo_with(
                &mut journal,
                id,
                std::slice::from_ref(&allowed),
                &mut progress,
            )
            .map(|_| ()),
            None => Executor::apply_with(
                &plan,
                &mut journal,
                std::slice::from_ref(&allowed),
                &mut progress,
                &AtomicBool::new(false),
            )
            .map(|_| ()),
        };
        assert!(injected, "{operation}");
        assert!(result.is_err(), "redirected {operation} must fail");
        assert_eq!(
            std::fs::read(&outside_file).unwrap(),
            saved,
            "{operation} touched unrelated fixture"
        );
        assert_eq!(
            std::fs::read(backup.join("track.flac")).unwrap(),
            saved,
            "{operation} changed pinned old file after refusal"
        );
        assert!(!target.exists());
        if let Some(id) = id {
            assert!(journal.batch(id).unwrap().is_some());
        }
    }
}

#[cfg(unix)]
#[test]
fn metadata_write_refuses_a_final_file_link_installed_after_preflight() {
    let fixture = Fixture::new("leaf-link");
    let source = fixture.0.join("a.flac");
    let outside = fixture.0.join("outside.flac");
    std::fs::write(&outside, flac()).unwrap();
    let saved = std::fs::read(&outside).unwrap();
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            tag_changes: vec![FieldChange {
                field: TagField::Artist,
                old: Some("Before".into()),
                new: Some("After".into()),
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut injected = false;
    let result = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |event| {
            if !injected && event.done >= 1 && event.completed.is_none() {
                std::fs::rename(&source, fixture.0.join("saved.flac")).unwrap();
                std::os::unix::fs::symlink(&outside, &source).unwrap();
                injected = true;
            }
        },
        &AtomicBool::new(false),
    );
    assert!(injected && result.is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), saved);
    assert_eq!(std::fs::read(fixture.0.join("saved.flac")).unwrap(), saved);
}

#[test]
fn late_cover_edits_survive_apply_and_undo_refusal() {
    use tagrex_core::model::{CoverArt, CoverKind, TagEngine};
    use tagrex_core::plan::CoverChange;
    fn cover(value: u8) -> CoverArt {
        let image = image::DynamicImage::ImageRgb8(image::ImageBuffer::from_pixel(
            1,
            1,
            image::Rgb([value, 0, 0]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        CoverArt {
            mime: "image/png".into(),
            data: bytes.into_inner().into(),
            kind: CoverKind::Front,
            description: format!("cover {value}"),
        }
    }
    for undo in [false, true] {
        let fixture = Fixture::new("late-cover");
        let source = fixture.0.join("a.flac");
        TagEngine::write_covers(&source, &[cover(20)]).unwrap();
        let plan = ChangePlan {
            changes: vec![FileChange {
                path: source.clone(),
                cover_change: Some(CoverChange {
                    old: TagEngine::read_covers(&source).unwrap(),
                    new: vec![cover(40)],
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        let id = undo.then(|| {
            Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0))
                .unwrap()
                .id
        });
        let mut external_bytes = None;
        let mut callback = |event: &tagrex_core::plan::ApplyProgress| {
            if external_bytes.is_none() && event.completed.is_none() && (undo || event.done >= 1) {
                TagEngine::write_covers(&source, &[cover(90)]).unwrap();
                external_bytes = Some(std::fs::read(&source).unwrap());
            }
        };
        let result = if let Some(id) = id {
            Executor::undo_with(
                &mut journal,
                id,
                std::slice::from_ref(&fixture.0),
                &mut callback,
            )
            .map(|_| ())
        } else {
            Executor::apply_with(
                &plan,
                &mut journal,
                std::slice::from_ref(&fixture.0),
                &mut callback,
                &AtomicBool::new(false),
            )
            .map(|_| ())
        };
        assert!(result.is_err());
        assert_eq!(std::fs::read(&source).unwrap(), external_bytes.unwrap());
        assert!(std::fs::read(&source).unwrap().ends_with(FRAME));
        if let Some(id) = id {
            assert!(journal.batch(id).unwrap().is_some());
        }
    }
}

#[test]
fn late_whole_block_private_bytes_survive_apply_and_undo_refusal() {
    use tagrex_core::model::{TagBlockKind, TagEngine};
    use tagrex_core::plan::BlockChange;
    for undo in [false, true] {
        let fixture = Fixture::new("late-private-block");
        let source = fixture.0.join("a.flac");
        let old = TagEngine::read_block(&source, TagBlockKind::VorbisComments)
            .unwrap()
            .unwrap();
        let mut block = BlockChange::removal(TagBlockKind::VorbisComments, old.clone());
        block.old_bytes = TagEngine::dump_vorbis(&source).unwrap();
        let mut new = old;
        new.tags.insert(TagField::Artist, "Planned".into());
        block.new = Some(new);
        let plan = ChangePlan {
            changes: vec![FileChange {
                path: source.clone(),
                block_changes: vec![block],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        let id = undo.then(|| {
            Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0))
                .unwrap()
                .id
        });
        let mut external_bytes = None;
        let mut callback = |event: &tagrex_core::plan::ApplyProgress| {
            if external_bytes.is_none() && event.completed.is_none() && (undo || event.done >= 1) {
                let mut bytes = std::fs::read(&source).unwrap();
                let mut at = 4;
                loop {
                    let kind = bytes[at] & 0x7f;
                    let len = u32::from_be_bytes([0, bytes[at + 1], bytes[at + 2], bytes[at + 3]])
                        as usize;
                    if kind == 4 {
                        let vendor =
                            u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
                        bytes[at + 8..at + 8 + vendor].fill(b'X');
                        break;
                    }
                    at += len + 4;
                }
                std::fs::write(&source, &bytes).unwrap();
                external_bytes = Some(bytes);
            }
        };
        let result = if let Some(id) = id {
            Executor::undo_with(
                &mut journal,
                id,
                std::slice::from_ref(&fixture.0),
                &mut callback,
            )
            .map(|_| ())
        } else {
            Executor::apply_with(
                &plan,
                &mut journal,
                std::slice::from_ref(&fixture.0),
                &mut callback,
                &AtomicBool::new(false),
            )
            .map(|_| ())
        };
        assert!(result.is_err());
        assert_eq!(std::fs::read(&source).unwrap(), external_bytes.unwrap());
        if let Some(id) = id {
            assert!(journal.batch(id).unwrap().is_some());
        }
    }
}

#[test]
fn rollback_never_promotes_a_later_external_field_into_owned_outcomes() {
    use tagrex_core::model::TagEngine;
    let fixture = Fixture::new("rollback-external-outcome");
    let first = fixture.0.join("a.flac");
    let plan = plan(&fixture.0, "fields");
    let cancel = AtomicBool::new(false);
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let mut external = None;
    let error = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |event| {
            if external.is_none() && event.path == first && event.completed == Some(ApplyStep::Tags)
            {
                let mut file = TagEngine::read(&first).unwrap();
                file.tags.insert(TagField::Artist, "External".into());
                TagEngine::write(&file).unwrap();
                external = Some(std::fs::read(&first).unwrap());
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert!(matches!(error, PlanError::RollbackIncomplete { .. }));
    assert_eq!(std::fs::read(&first).unwrap(), *external.as_ref().unwrap());
    let batch = journal.batches().unwrap().remove(0);
    assert!(batch.applied_values.iter().any(|value| value.path == first
        && value.field == TagField::Artist
        && value.value.as_deref() == Some("After 東京")));
    assert!(batch
        .applied_values
        .iter()
        .all(|value| value.value.as_deref() != Some("External")));
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    assert_eq!(std::fs::read(first).unwrap(), external.unwrap());
    inspect(&fixture.0.join("b.flac"), "Before");
}

#[test]
fn success_records_our_write_before_a_later_completion_callback_edit() {
    use tagrex_core::model::TagEngine;
    let fixture = Fixture::new("success-external-outcome");
    let first = fixture.0.join("a.flac");
    let mut plan = plan(&fixture.0, "fields");
    plan.changes.truncate(1);
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let batch = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |event| {
            if event.completed == Some(ApplyStep::Tags) {
                let mut file = TagEngine::read(&first).unwrap();
                file.tags.insert(TagField::Artist, "External".into());
                TagEngine::write(&file).unwrap();
            }
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(batch
        .applied_values
        .iter()
        .all(|value| value.value.as_deref() != Some("External")));
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    inspect(&first, "External");
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "manual 5000-folder authority/descriptor fan-out probe"]
fn mutation_authority_five_thousand_folder_probe() {
    fn descriptors() -> usize {
        std::fs::read_dir("/proc/self/fd").unwrap().count()
    }
    let fixture = Fixture::new("folder-fanout");
    let library = fixture.0.join("library");
    std::fs::create_dir(&library).unwrap();
    let bytes = flac();
    let mut changes = Vec::new();
    for index in 0..5000 {
        let folder = library.join(format!("album-{index}"));
        std::fs::create_dir(&folder).unwrap();
        let source = folder.join("track.flac");
        std::fs::write(&source, &bytes).unwrap();
        changes.push(FileChange {
            path: source,
            rename_to: Some(library.join(format!("moved/{index}.flac"))),
            ..Default::default()
        });
    }
    let plan = ChangePlan {
        changes,
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let baseline = descriptors();
    let mut observed = baseline;
    let mut progress = |event: &tagrex_core::plan::ApplyProgress| {
        if event.done.is_multiple_of(128) {
            observed = observed.max(descriptors());
        }
    };
    let start = Instant::now();
    let batch = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&library),
        &mut progress,
        &AtomicBool::new(false),
    )
    .unwrap();
    let applied = start.elapsed();
    let start = Instant::now();
    Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&library),
        &mut progress,
    )
    .unwrap();
    let undone = start.elapsed();
    assert!(
        observed - baseline < 64,
        "directory fan-out retained too many handles: baseline={baseline}, observed={observed}"
    );
    assert!(journal.batches().unwrap().is_empty());
    for change in &plan.changes {
        assert_eq!(std::fs::read(&change.path).unwrap(), bytes);
        assert!(!change.rename_to.as_ref().unwrap().exists());
    }
    assert!(!library.join("moved").exists());
    eprintln!("5000 folders /5000 files: Apply {applied:?}; Undo {undone:?}; callback-observed extra descriptors {}",observed-baseline);
}

#[test]
#[ignore = "independent FFprobe metadata and encoded-packet verification"]
fn ffprobe_verifies_crash_apply_and_restart_undo() {
    fn probe(path: &Path, packets: bool) -> String {
        let mut command = Command::new("ffprobe");
        command.args(["-v", "error"]);
        if packets {
            command.args([
                "-select_streams",
                "a",
                "-show_packets",
                "-show_entries",
                "packet=data_hash",
                "-show_data_hash",
                "sha256",
                "-of",
                "csv=p=0",
            ]);
        } else {
            command.args([
                "-show_entries",
                "format_tags=artist,title",
                "-of",
                "default=noprint_wrappers=1",
            ]);
        }
        let output = command.arg(path).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    let fixture = Fixture::new("ffprobe");
    let path = fixture.0.join("a.flac");
    let original = probe(&path, false);
    assert!(original.contains("Before") && original.contains("Keep Jóga"));
    let packets = probe(&path, true);
    assert!(
        packets.contains("SHA256:"),
        "the independent packet probe must actually hash audio"
    );
    crash(&fixture.0, "after-finish");
    let changed = probe(&path, false);
    assert!(changed.contains("After 東京") && changed.contains("Keep Jóga"));
    assert_eq!(probe(&path, true), packets);
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let id = journal.batches().unwrap()[0].id;
    Executor::undo(&mut journal, id, std::slice::from_ref(&fixture.0)).unwrap();
    assert_eq!(probe(&path, false), original);
    assert_eq!(probe(&path, true), packets);
}

#[test]
fn pure_move_undo_after_restart_preserves_same_inode_external_content_edits() {
    let fixture = Fixture::new("move-same-object");
    let source = fixture.0.join("a.flac");
    let target = fixture.0.join("moved.flac");
    let database = fixture.0.join("journal.sqlite");
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            rename_to: Some(target.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&database).unwrap();
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap();
    assert!(batch
        .copy_stamps
        .iter()
        .all(|stamp| stamp.moved && stamp.identity.is_some()));
    drop(journal);
    // Truncating an existing file preserves its native object identity. A
    // path-only Undo must move these later bytes rather than restore old bytes.
    let mut changed = flac();
    let index = changed
        .windows(6)
        .position(|bytes| bytes == b"Before")
        .unwrap();
    changed[index..index + 6].copy_from_slice(b"Edited");
    std::fs::write(&target, &changed).unwrap();
    let mut journal = SqliteJournal::open(&database).unwrap();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), changed);
    assert!(!target.exists());
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn identical_replacement_cannot_inherit_durable_move_ownership() {
    let fixture = Fixture::new("move-identical-replacement");
    let source = fixture.0.join("a.flac");
    let target = fixture.0.join("moved.flac");
    let preserved = fixture.0.join("preserved.flac");
    let database = fixture.0.join("journal.sqlite");
    let mut journal = SqliteJournal::open(&database).unwrap();
    let batch = Executor::apply(
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
    .unwrap();
    drop(journal);
    std::fs::rename(&target, &preserved).unwrap();
    std::fs::write(&target, flac()).unwrap();
    let mut journal = SqliteJournal::open(&database).unwrap();
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    assert!(!source.exists());
    assert_eq!(std::fs::read(&target).unwrap(), flac());
    assert_eq!(std::fs::read(&preserved).unwrap(), flac());
    assert_eq!(journal.batches().unwrap().len(), 1);
    // Restoring the actual object makes retry safe without discarding history.
    std::fs::remove_file(&target).unwrap();
    std::fs::rename(&preserved, &target).unwrap();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), flac());
}

#[cfg(target_os = "linux")]
#[test]
fn sidecar_link_entry_ownership_survives_restart_without_following_target() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new("move-link-entry");
    let source = fixture.0.join("a.flac");
    let target = fixture.0.join("moved.flac");
    let link = fixture.0.join("a.cue");
    let output = fixture.0.join("moved.cue");
    let saved_link = fixture.0.join("saved.cue");
    let link_target = fixture.0.join("cue-content.txt");
    std::fs::write(&link_target, b"must remain unchanged").unwrap();
    symlink("cue-content.txt", &link).unwrap();
    let database = fixture.0.join("journal.sqlite");
    let mut journal = SqliteJournal::open(&database).unwrap();
    let batch = Executor::apply(
        &ChangePlan {
            changes: vec![FileChange {
                path: source.clone(),
                rename_to: Some(target.clone()),
                sidecar_renames: vec![(link.clone(), output.clone())],
                ..Default::default()
            }],
            ..Default::default()
        },
        &mut journal,
        std::slice::from_ref(&fixture.0),
    )
    .unwrap();
    drop(journal);
    assert_eq!(
        std::fs::read_link(&output).unwrap(),
        Path::new("cue-content.txt")
    );
    std::fs::rename(&output, &saved_link).unwrap();
    symlink("cue-content.txt", &output).unwrap();
    let mut journal = SqliteJournal::open(&database).unwrap();
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    assert!(!source.exists());
    assert!(target.exists());
    assert_eq!(journal.batches().unwrap().len(), 1);
    std::fs::remove_file(&output).unwrap();
    std::fs::rename(&saved_link, &output).unwrap();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        Path::new("cue-content.txt")
    );
    assert_eq!(
        std::fs::read(&link_target).unwrap(),
        b"must remain unchanged"
    );
}

#[test]
fn rollback_does_not_claim_a_recreated_original_as_a_completed_move_reversal() {
    let fixture = Fixture::new("rollback-false-completion");
    let source = fixture.0.join("a.flac");
    let target = fixture.0.join("moved.flac");
    let saved = fixture.0.join("saved.flac");
    let blocked = fixture.0.join("blocked.flac");
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let mut replaced = false;
    let outcome = Executor::apply_with(
        &ChangePlan {
            changes: vec![
                FileChange {
                    path: source.clone(),
                    rename_to: Some(target.clone()),
                    ..Default::default()
                },
                FileChange {
                    path: fixture.0.join("b.flac"),
                    rename_to: Some(blocked.clone()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if !replaced && progress.path == source && progress.completed.is_some() {
                std::fs::rename(&target, &saved).unwrap();
                std::fs::write(&source, b"unrelated original replacement").unwrap();
                std::fs::write(&blocked, b"unrelated collision").unwrap();
                replaced = true;
            }
        },
        &AtomicBool::new(false),
    );
    assert!(replaced);
    assert!(matches!(outcome, Err(PlanError::RollbackIncomplete { .. })));
    assert_eq!(
        std::fs::read(&source).unwrap(),
        b"unrelated original replacement"
    );
    assert_eq!(std::fs::read(&saved).unwrap(), flac());
    assert_eq!(std::fs::read(&blocked).unwrap(), b"unrelated collision");
    assert!(!target.exists());
    assert_eq!(journal.batches().unwrap().len(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn mixed_copy_move_and_link_sidecar_batches_restore_after_cancel_and_restart() {
    use std::os::unix::fs::symlink;
    for cancel_after_link in [false, true] {
        let fixture = Fixture::new("mixed-move-copy-stage");
        let first = fixture.0.join("a.flac");
        let second = fixture.0.join("b.flac");
        let first_target = fixture.0.join("moved/a.flac");
        let second_target = fixture.0.join("moved/b.flac");
        let link = fixture.0.join("a.cue");
        let link_target = fixture.0.join("moved/a.cue");
        let linked_content = fixture.0.join("cue-content.txt");
        std::fs::write(&linked_content, b"linked content must not change").unwrap();
        symlink("cue-content.txt", &link).unwrap();
        let copy_sidecar = fixture.0.join("b.lrc");
        let copy_sidecar_target = fixture.0.join("moved/b.lrc");
        std::fs::write(&copy_sidecar, b"copy lyric bytes").unwrap();
        let plan = ChangePlan {
            changes: vec![
                FileChange {
                    path: first.clone(),
                    rename_to: Some(first_target.clone()),
                    sidecar_renames: vec![(link.clone(), link_target.clone())],
                    ..Default::default()
                },
                FileChange {
                    path: second.clone(),
                    rename_to: Some(second_target.clone()),
                    copy: true,
                    sidecar_renames: vec![(copy_sidecar.clone(), copy_sidecar_target.clone())],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let database = fixture.0.join("journal.sqlite");
        let mut journal = SqliteJournal::open(&database).unwrap();
        let cancel = AtomicBool::new(false);
        let outcome = Executor::apply_with(
            &plan,
            &mut journal,
            std::slice::from_ref(&fixture.0),
            &mut |progress| {
                if cancel_after_link
                    && progress.path == link
                    && progress.completed == Some(ApplyStep::Transfer)
                {
                    cancel.store(true, Ordering::SeqCst);
                }
            },
            &cancel,
        );
        if cancel_after_link {
            assert!(matches!(outcome, Err(PlanError::Cancelled)), "{outcome:?}");
            assert!(journal.batches().unwrap().is_empty());
        } else {
            let batch = outcome.unwrap();
            assert!(
                !link_target.exists(),
                "relative link is intentionally dangling at moved location"
            );
            assert_eq!(
                std::fs::read_link(&link_target).unwrap(),
                Path::new("cue-content.txt")
            );
            assert!(journal.move_stages(batch.id).unwrap().len() >= 2);
            drop(journal);
            journal = SqliteJournal::open(&database).unwrap();
            Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
            assert!(journal.move_stages(batch.id).unwrap().is_empty());
        }
        assert_eq!(std::fs::read(&first).unwrap(), flac());
        assert_eq!(std::fs::read(&second).unwrap(), flac());
        assert!(!first_target.exists());
        assert!(!second_target.exists());
        assert!(!copy_sidecar_target.exists());
        assert_eq!(std::fs::read(&copy_sidecar).unwrap(), b"copy lyric bytes");
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            Path::new("cue-content.txt")
        );
        assert_eq!(
            std::fs::read(&linked_content).unwrap(),
            b"linked content must not change"
        );
        assert!(!fixture.0.join("moved").exists());
    }
}

#[test]
fn later_row_preflight_callback_cannot_endorse_an_earlier_move_source_replacement() {
    let fixture = Fixture::new("earlier-row-source-proof");
    let first = fixture.0.join("a.flac");
    let second = fixture.0.join("b.flac");
    let saved = fixture.0.join("preserved-original.flac");
    let first_target = fixture.0.join("moved/a.flac");
    let second_target = fixture.0.join("moved/b.flac");
    let plan = ChangePlan {
        changes: vec![
            FileChange {
                path: first.clone(),
                rename_to: Some(first_target.clone()),
                ..Default::default()
            },
            FileChange {
                path: second.clone(),
                rename_to: Some(second_target.clone()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let mut swapped = false;
    let outcome = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if !swapped
                && progress.path == second
                && progress.done == 1
                && progress.completed.is_none()
            {
                std::fs::rename(&first, &saved).unwrap();
                std::fs::write(&first, flac()).unwrap();
                swapped = true;
            }
        },
        &AtomicBool::new(false),
    );
    assert!(swapped);
    assert!(
        outcome.is_err(),
        "an identical replacement must not inherit an earlier source's approval"
    );
    assert_eq!(std::fs::read(&first).unwrap(), flac());
    assert_eq!(std::fs::read(&saved).unwrap(), flac());
    assert_eq!(std::fs::read(&second).unwrap(), flac());
    assert!(!first_target.exists());
    assert!(!second_target.exists());
}
