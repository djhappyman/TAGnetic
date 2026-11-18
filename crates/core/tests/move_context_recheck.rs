//! Fresh whole-collection recovery checks through public Executor and SQLite.
//! Every path and byte belongs to these disposable fixtures.
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
            "tagrex-move-context-fresh-{}-{}",
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
    let mut out = b"RIFF".to_vec();
    out.extend(356u32.to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(8000u32.to_le_bytes());
    out.extend(16000u32.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(320u32.to_le_bytes());
    out.extend([sample; 320]);
    out
}
struct Interrupt {
    journal: SqliteJournal,
    stop: usize,
    calls: usize,
}
impl UndoJournal for Interrupt {
    fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError> {
        self.journal.record(batch)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.journal.batches()
    }
    fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
        self.journal.batch(id)
    }
    fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError> {
        self.journal.finish(id, batch)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.journal.rollback(id)
    }
    fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
        self.journal.move_stages(id)
    }
    fn prepare_move_stage(&mut self, id: BatchId, stage: &MoveStage) -> Result<(), JournalError> {
        self.journal.prepare_move_stage(id, stage)?;
        self.calls += 1;
        if self.calls == self.stop {
            std::fs::rename(&stage.from, &stage.path).unwrap();
            std::process::exit(73);
        }
        Ok(())
    }
}
struct Pending {
    id: BatchId,
    stages: Vec<MoveStage>,
    sources: Vec<PathBuf>,
    targets: Vec<PathBuf>,
    bytes: Vec<Vec<u8>>,
}
fn pending(fixture: &Fixture, count: usize) -> Pending {
    let mut sources = Vec::new();
    let mut targets = Vec::new();
    let mut bytes = Vec::new();
    let mut changes = Vec::new();
    for i in 0..count {
        let source = fixture.0.join(format!("source-{i}.wav"));
        let target = fixture.0.join(format!("target-{i}.wav"));
        let data = wav(i as u8);
        std::fs::write(&source, &data).unwrap();
        changes.push(FileChange {
            path: source.clone(),
            rename_to: Some(target.clone()),
            ..Default::default()
        });
        sources.push(source);
        targets.push(target);
        bytes.push(data);
    }
    let plan = ChangePlan {
        changes,
        ..Default::default()
    };
    let seed_dir = fixture.0.join("crash-plan-seed");
    std::fs::create_dir(&seed_dir).unwrap();
    let mut seed = SqliteJournal::open(&seed_dir.join("plan.sqlite")).unwrap();
    seed.record(&AppliedBatch {
        native_tags: false,
        id: BatchId(0),
        description: plan.description.clone(),
        message: plan.message.clone(),
        applied_at: 0,
        plan,
        allowed_roots: vec![fixture.0.clone()],
        created_dirs: Vec::new(),
        removed_dirs: Vec::new(),
        copy_stamps: Vec::new(),
        applied_values: Vec::new(),
        block_fingerprints: Vec::new(),
        interrupted: true,
        id3v2_revisions: Vec::new(),
    })
    .unwrap();
    drop(seed);
    let log_path = seed_dir.join("child.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "move_context_interruption_process_child",
            "--nocapture",
        ])
        .env("TAGREX_MOVE_CONTEXT_CRASH_ROOT", &fixture.0)
        .env("TAGREX_MOVE_CONTEXT_CRASH_COUNT", count.to_string())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(30) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => {
                let _ = child.kill();
                let reaped = child.wait();
                panic!(
                    "move child timed out or wait failed: {result:?}; reaped={reaped:?}\n{}",
                    String::from_utf8_lossy(&std::fs::read(&log_path).unwrap())
                );
            }
        }
    };
    assert_eq!(
        status.code(),
        Some(73),
        "child missed the durable intent/capture boundary: {}",
        String::from_utf8_lossy(&std::fs::read(&log_path).unwrap())
    );
    std::fs::remove_dir_all(seed_dir).unwrap();
    let journal = SqliteJournal::open(&fixture.database()).unwrap();
    let batch = journal.batches().unwrap().remove(0);
    let stages = journal.move_stages(batch.id).unwrap();
    assert_eq!(stages.len(), count);
    Pending {
        id: batch.id,
        stages,
        sources,
        targets,
        bytes,
    }
}
/// Dispatcher only; parent probes assert exact exit73 and reopened authority.
#[test]
fn move_context_interruption_process_child() {
    let Some(root) = std::env::var_os("TAGREX_MOVE_CONTEXT_CRASH_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let seed = SqliteJournal::open(&root.join("crash-plan-seed/plan.sqlite")).unwrap();
    let plan = seed.batches().unwrap().remove(0).plan;
    drop(seed);
    let mut journal = Interrupt {
        journal: SqliteJournal::open(&root.join("journal.sqlite")).unwrap(),
        stop: std::env::var("TAGREX_MOVE_CONTEXT_CRASH_COUNT")
            .unwrap()
            .parse()
            .unwrap(),
        calls: 0,
    };
    Executor::apply(&plan, &mut journal, &[root]).unwrap();
    panic!("child did not reach durable intent/capture boundary");
}

fn assert_unchanged_pending(fixture: &Fixture, state: &Pending) {
    for i in 0..state.sources.len() {
        assert!(
            !state.sources[i].exists(),
            "recovery moved row {i} before refusing full context"
        );
        let actual = if i + 1 == state.sources.len() {
            &state.stages[i].path
        } else {
            &state.targets[i]
        };
        assert_eq!(std::fs::read(actual).unwrap(), state.bytes[i]);
    }
    let journal = SqliteJournal::open(&fixture.database()).unwrap();
    assert!(journal.batch(state.id).unwrap().is_some());
}
#[test]
fn genuine_directory_identity_cannot_be_promoted_to_file_move_authority() {
    let fixture = Fixture::new();
    let state = pending(&fixture, 1);
    let stage = &state.stages[0];
    let sql = rusqlite::Connection::open(fixture.database()).unwrap();
    // These are actual directory identity bytes produced by the application,
    // not a guessed/reimplemented native encoding or a future-version blob.
    sql.execute(
        "UPDATE copy_stamps SET identity=?1 WHERE batch_id=?2 AND moved=1",
        rusqlite::params![stage.directory_identity, state.id.0],
    )
    .unwrap();
    sql.execute(
        "UPDATE move_stages SET identity=directory_identity WHERE batch_id=?1",
        [state.id.0],
    )
    .unwrap();
    drop(sql);
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    let result = Executor::undo(&mut journal, state.id, std::slice::from_ref(&fixture.0));
    assert!(result.is_err());
    assert_unchanged_pending(&fixture, &state);
}
#[test]
fn oversized_later_sql_cell_refuses_before_recovering_completed_or_pending_rows() {
    for column in ["source", "path", "identity"] {
        let fixture = Fixture::new();
        let state = pending(&fixture, 2);
        let sql = rusqlite::Connection::open(fixture.database()).unwrap();
        let size = if column == "identity" { 65 } else { 131073 };
        let query = format!("UPDATE move_stages SET {column}=?1 WHERE batch_id=?2 AND path=?3");
        sql.execute(
            &query,
            rusqlite::params![
                "x".repeat(size),
                state.id.0,
                state.stages[1].path.to_str().unwrap()
            ],
        )
        .unwrap();
        drop(sql);
        let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
        assert!(Executor::undo(&mut journal, state.id, std::slice::from_ref(&fixture.0)).is_err());
        assert_unchanged_pending(&fixture, &state);
    }
}
#[test]
fn duplicate_native_reservation_alias_refuses_the_entire_mixed_collection() {
    let fixture = Fixture::new();
    let state = pending(&fixture, 2);
    let sql = rusqlite::Connection::open(fixture.database()).unwrap();
    let alias = format!(
        "{}/./entry",
        state.stages[1].path.parent().unwrap().display()
    );
    sql.execute("INSERT INTO move_stages(batch_id,target,source,destination,path,directory_identity,identity) SELECT batch_id,target,source,destination,?3,directory_identity,identity FROM move_stages WHERE batch_id=?1 AND path=?2",rusqlite::params![state.id.0,state.stages[1].path.to_str().unwrap(),alias]).unwrap();
    drop(sql);
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    assert!(Executor::undo(&mut journal, state.id, std::slice::from_ref(&fixture.0)).is_err());
    assert_unchanged_pending(&fixture, &state);
}
#[test]
fn mixed_completed_pending_recovery_preserves_external_same_object_edits_and_siblings() {
    let fixture = Fixture::new();
    let state = pending(&fixture, 2);
    let edited = wav(9);
    std::fs::write(&state.targets[0], &edited).unwrap();
    let sibling = fixture.0.join("ordinary-data");
    std::fs::write(&sibling, b"untargeted fixture").unwrap();
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    Executor::undo(&mut journal, state.id, std::slice::from_ref(&fixture.0)).unwrap();
    assert_eq!(std::fs::read(&state.sources[0]).unwrap(), edited);
    assert_eq!(std::fs::read(&state.sources[1]).unwrap(), state.bytes[1]);
    assert_eq!(std::fs::read(sibling).unwrap(), b"untargeted fixture");
    assert!(journal.batches().unwrap().is_empty());
    for target in state.targets {
        assert!(!target.exists());
    }
    for stage in state.stages {
        assert!(!stage.path.parent().unwrap().exists());
    }
}

#[test]
fn mixed_history_with_wrong_later_move_kind_refuses_without_any_recovery_effect() {
    let fixture = Fixture::new();
    let state = pending(&fixture, 2);
    let sql = rusqlite::Connection::open(fixture.database()).unwrap();
    sql.execute(
        "UPDATE copy_stamps SET moved=0 WHERE batch_id=?1 AND path=?2",
        rusqlite::params![state.id.0, state.stages[1].target.to_str().unwrap()],
    )
    .unwrap();
    drop(sql);
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    assert!(Executor::undo(&mut journal, state.id, std::slice::from_ref(&fixture.0)).is_err());
    assert_unchanged_pending(&fixture, &state);
}

#[cfg(unix)]
#[test]
fn recovery_moves_only_the_recorded_hardlink_entry_and_keeps_the_other_name() {
    let fixture = Fixture::new();
    let state = pending(&fixture, 1);
    let link = fixture.0.join("independent-hardlink.wav");
    std::fs::hard_link(&state.stages[0].path, &link).unwrap();
    let edited = wav(7);
    std::fs::write(&link, &edited).unwrap();
    let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
    Executor::undo(&mut journal, state.id, std::slice::from_ref(&fixture.0)).unwrap();
    assert_eq!(std::fs::read(&state.sources[0]).unwrap(), edited);
    assert_eq!(std::fs::read(&link).unwrap(), edited);
    use std::os::unix::fs::MetadataExt;
    let restored = std::fs::metadata(&state.sources[0]).unwrap();
    let other = std::fs::metadata(&link).unwrap();
    assert_eq!((restored.dev(), restored.ino()), (other.dev(), other.ino()));
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn future_or_unknown_native_roles_refuse_before_any_mixed_collection_effect() {
    for future in [false, true] {
        let fixture = Fixture::new();
        let state = pending(&fixture, 2);
        let mut proof = state.stages[1].identity.clone();
        if future {
            proof[5] = b'9';
        } else {
            *proof.last_mut().unwrap() = 255;
        }
        let sql = rusqlite::Connection::open(fixture.database()).unwrap();
        sql.execute(
            "UPDATE copy_stamps SET identity=?1 WHERE batch_id=?2 AND path=?3",
            rusqlite::params![proof, state.id.0, state.stages[1].target.to_str().unwrap()],
        )
        .unwrap();
        sql.execute(
            "UPDATE move_stages SET identity=?1 WHERE batch_id=?2 AND path=?3",
            rusqlite::params![proof, state.id.0, state.stages[1].path.to_str().unwrap()],
        )
        .unwrap();
        drop(sql);
        let mut journal = SqliteJournal::open(&fixture.database()).unwrap();
        assert!(Executor::undo(&mut journal, state.id, std::slice::from_ref(&fixture.0)).is_err());
        assert_unchanged_pending(&fixture, &state);
    }
}

#[test]
fn nested_unwinding_executor_restores_outer_move_context_and_directory_authority() {
    let fixture = Fixture::new();
    let state = pending(&fixture, 2);
    let inner = Fixture::new();
    let inner_source = inner.0.join("nested.wav");
    let inner_target = inner.0.join("nested-target.wav");
    let bytes = wav(5);
    std::fs::write(&inner_source, &bytes).unwrap();
    let mut nested_journal = SqliteJournal::open(&inner.database()).unwrap();
    let mut outer_journal = SqliteJournal::open(&fixture.database()).unwrap();
    let mut called = false;
    Executor::undo_with(
        &mut outer_journal,
        state.id,
        std::slice::from_ref(&fixture.0),
        &mut |_| {
            if called {
                return;
            }
            called = true;
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Executor::apply_with(
                    &ChangePlan {
                        changes: vec![FileChange {
                            path: inner_source.clone(),
                            rename_to: Some(inner_target.clone()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    &mut nested_journal,
                    std::slice::from_ref(&inner.0),
                    &mut |event| {
                        if event.done > 0 {
                            panic!("nested fixture interruption with active context");
                        }
                    },
                    &std::sync::atomic::AtomicBool::new(false),
                )
            }));
            assert!(interrupted.is_err());
            // The process survived this unwind: Apply must clean its own
            // interrupted operation before restoring the outer thread context.
            assert!(nested_journal.batches().unwrap().is_empty());
            assert_eq!(std::fs::read(&inner_source).unwrap(), bytes);
            assert!(!inner_target.exists());
        },
    )
    .unwrap();
    assert!(called);
    for i in 0..2 {
        assert_eq!(std::fs::read(&state.sources[i]).unwrap(), state.bytes[i]);
        assert!(!state.targets[i].exists());
    }
    assert_eq!(std::fs::read(inner_source).unwrap(), bytes);
    assert!(!inner_target.exists());
    assert!(outer_journal.batches().unwrap().is_empty());
    assert!(nested_journal.batches().unwrap().is_empty());
}
