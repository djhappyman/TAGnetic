//! Fixture-only probes at the actual native source-stat/rename boundary.
use super::*;
use crate::journal::SqliteJournal;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const OWNED: &[u8] = b"owned transfer payload";
const FOREIGN: &[u8] = b"foreign source replacement";
struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-move-stage-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn move_scope(batch: &AppliedBatch) -> (MoveContextScope, pinned_paths::Scope) {
    let paths = pinned_paths::Scope::enter(
        &batch.allowed_roots,
        operation_paths(&batch.plan, &batch.copy_stamps),
    )
    .unwrap();
    let context = MoveContextScope::enter(
        batch.id,
        &batch.plan,
        &batch.copy_stamps,
        &batch.allowed_roots,
    )
    .unwrap();
    (context, paths)
}
fn seeded(root: &Path) -> (SqliteJournal, AppliedBatch) {
    seeded_at(root, &root.join("journal.sqlite"))
}
fn seeded_at(root: &Path, database: &Path) -> (SqliteJournal, AppliedBatch) {
    let source = root.join("source.bin");
    let target = root.join("target.bin");
    std::fs::write(&source, OWNED).unwrap();
    let scope = pinned_paths::Scope::enter(&[root.to_path_buf()], [source.clone(), target.clone()])
        .unwrap();
    let proof = CopyStamp::for_move(&source, &target).unwrap();
    drop(scope);
    let mut batch = AppliedBatch {
        native_tags: false,
        id: BatchId(0),
        description: "move stage fixture".into(),
        message: None,
        applied_at: 0,
        plan: ChangePlan {
            changes: vec![FileChange {
                path: source,
                rename_to: Some(target),
                ..Default::default()
            }],
            ..Default::default()
        },
        created_dirs: vec![],
        removed_dirs: vec![],
        allowed_roots: vec![root.to_path_buf()],
        copy_stamps: vec![proof],
        applied_values: vec![],
        block_fingerprints: vec![],
        interrupted: true,
        id3v2_revisions: vec![],
    };
    let mut journal = SqliteJournal::open(database).unwrap();
    batch.id = journal.record(&batch).unwrap();
    (journal, batch)
}
#[test]
fn source_swap_after_final_native_stat_is_restored_without_false_ownership() {
    for block_restore in [false, true] {
        let fixture = Fixture::new("source-stat-race");
        let (mut journal, batch) = seeded(&fixture.0);
        let source = fixture.0.join("source.bin");
        let target = fixture.0.join("target.bin");
        let saved = fixture.0.join("saved.bin");
        let _scope = move_scope(&batch);
        let hook_source = source.clone();
        let hook_saved = saved.clone();
        let mut swapped = false;
        let _hook = pinned_paths::rename_hook(Box::new(move |from, to| {
            if !swapped && from == hook_source && to.file_name().is_some_and(|name| name == "entry")
            {
                std::fs::rename(&hook_source, &hook_saved).unwrap();
                std::fs::write(&hook_source, FOREIGN).unwrap();
                swapped = true;
            } else if swapped && block_restore && to == hook_source && !hook_source.exists() {
                std::fs::write(&hook_source, b"independent restored-name collision").unwrap();
            }
        }));
        let mut attempted = false;
        let result = staged_move(
            &mut journal,
            batch.id,
            &batch.copy_stamps[0],
            &source,
            &target,
            &mut attempted,
        );
        assert!(attempted);
        assert!(
            matches!(result, Err(PlanError::RecoveryRequired { .. })),
            "{result:?}"
        );
        assert_eq!(std::fs::read(&saved).unwrap(), OWNED);
        assert!(!target.exists());
        let stages = journal.move_stages(batch.id).unwrap();
        assert_eq!(stages.len(), 1);
        if block_restore {
            assert_eq!(
                std::fs::read(&source).unwrap(),
                b"independent restored-name collision"
            );
            assert_eq!(std::fs::read(&stages[0].path).unwrap(), FOREIGN);
        } else {
            assert_eq!(std::fs::read(&source).unwrap(), FOREIGN);
        }
        assert!(journal.batch(batch.id).unwrap().is_some());
    }
}
#[test]
fn destination_race_cannot_clobber_and_owned_private_payload_can_recover() {
    let fixture = Fixture::new("destination-race");
    let (mut journal, batch) = seeded(&fixture.0);
    let source = fixture.0.join("source.bin");
    let target = fixture.0.join("target.bin");
    let _scope = move_scope(&batch);
    let hook_target = target.clone();
    let _hook = pinned_paths::rename_hook(Box::new(move |_, to| {
        if to == hook_target && !hook_target.exists() {
            std::fs::write(&hook_target, FOREIGN).unwrap();
        }
    }));
    let result = staged_move(
        &mut journal,
        batch.id,
        &batch.copy_stamps[0],
        &source,
        &target,
        &mut false,
    );
    assert!(
        matches!(result, Err(PlanError::RenameCollision(_))),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), FOREIGN);
    let stages = journal.move_stages(batch.id).unwrap();
    assert!(!stages[0].path.exists());
    assert_eq!(std::fs::read(&source).unwrap(), OWNED);
    recover_move_stages(&stages).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), OWNED);
    assert_eq!(std::fs::read(&target).unwrap(), FOREIGN);
}
#[test]
fn move_stage_child() {
    let Some(root) = std::env::var_os("TAGREX_MOVE_STAGE_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let phase = std::env::var("TAGREX_MOVE_STAGE_TEST_PHASE").unwrap();
    let undo = std::env::var_os("TAGREX_MOVE_STAGE_TEST_UNDO").is_some();
    let mut journal = SqliteJournal::open(&root.join("journal.sqlite")).unwrap();
    let batch = journal.batches().unwrap().remove(0);
    let _scope = move_scope(&batch);
    let source = root.join("source.bin");
    let target = root.join("target.bin");
    let (from, to) = if undo {
        (&target, &source)
    } else {
        (&source, &target)
    };
    if std::env::var_os("TAGREX_MOVE_STAGE_TEST_PAIR").is_some() {
        let first = &batch.plan.changes[0];
        staged_move(
            &mut journal,
            batch.id,
            &batch.copy_stamps[0],
            &first.path,
            first.rename_to.as_ref().unwrap(),
            &mut false,
        )
        .unwrap();
        let second = &batch.plan.changes[1];
        staged_move_with(
            &mut journal,
            batch.id,
            &batch.copy_stamps[1],
            &second.path,
            second.rename_to.as_ref().unwrap(),
            &mut false,
            |at| {
                if at == phase {
                    std::process::exit(83);
                }
            },
        )
        .unwrap();
        panic!("pair phase was not reached");
    }
    let _swap_hook = if std::env::var_os("TAGREX_MOVE_STAGE_TEST_SWAP").is_some() {
        let hook_from = from.to_path_buf();
        let saved = root.join("preserved-original.bin");
        let mut changed = false;
        Some(pinned_paths::rename_hook(Box::new(
            move |source, destination| {
                if !changed
                    && source == hook_from
                    && destination.file_name().is_some_and(|name| name == "entry")
                {
                    std::fs::rename(source, &saved).unwrap();
                    std::fs::write(source, FOREIGN).unwrap();
                    changed = true;
                }
            },
        )))
    } else {
        None
    };
    staged_move_with(
        &mut journal,
        batch.id,
        &batch.copy_stamps[0],
        from,
        to,
        &mut false,
        |at| {
            if at == phase {
                std::process::exit(83);
            }
        },
    )
    .unwrap();
    panic!("move phase not reached");
}
#[test]
fn process_death_at_each_move_phase_leaves_recoverable_original_object() {
    for undo in [false, true] {
        for phase in [
            "intent",
            "checked",
            "staged",
            "verified",
            "publish_checked",
            "published",
            "cleaned",
        ] {
            let fixture = Fixture::new(phase);
            let (journal, batch) = seeded(&fixture.0);
            drop(journal);
            if undo {
                std::fs::rename(fixture.0.join("source.bin"), fixture.0.join("target.bin"))
                    .unwrap();
            }
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "plan::move_staging_tests::move_stage_child",
                    "--nocapture",
                ])
                .env("TAGREX_MOVE_STAGE_TEST_ROOT", &fixture.0)
                .env("TAGREX_MOVE_STAGE_TEST_PHASE", phase)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit());
            if undo {
                command.env("TAGREX_MOVE_STAGE_TEST_UNDO", "1");
            }
            let mut child = command.spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert_eq!(status.code(), Some(83), "{undo}:{phase}");
                    break;
                }
                if Instant::now() > deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("move child timed out");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
            assert_eq!(journal.move_stages(batch.id).unwrap().len(), 1);
            Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
            assert_eq!(std::fs::read(fixture.0.join("source.bin")).unwrap(), OWNED);
            assert!(!fixture.0.join("target.bin").exists());
            assert!(journal.batches().unwrap().is_empty());
            assert!(!std::fs::read_dir(&fixture.0).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tagrex-move-")));
        }
    }
}

#[test]
fn blocked_publish_restoration_retains_owned_and_both_foreign_entries() {
    let fixture = Fixture::new("blocked-publish-restore");
    let (mut journal, batch) = seeded(&fixture.0);
    let source = fixture.0.join("source.bin");
    let target = fixture.0.join("target.bin");
    let _scope = move_scope(&batch);
    let hook_source = source.clone();
    let hook_target = target.clone();
    let mut injected = false;
    let _hook = pinned_paths::rename_hook(Box::new(move |_, to| {
        if !injected && to == hook_target {
            std::fs::write(&hook_target, b"destination collision").unwrap();
            std::fs::write(&hook_source, b"source collision").unwrap();
            injected = true;
        }
    }));
    let result = staged_move(
        &mut journal,
        batch.id,
        &batch.copy_stamps[0],
        &source,
        &target,
        &mut false,
    );
    assert!(
        matches!(result, Err(PlanError::RecoveryRequired { .. })),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&source).unwrap(), b"source collision");
    assert_eq!(std::fs::read(&target).unwrap(), b"destination collision");
    let stages = journal.move_stages(batch.id).unwrap();
    assert_eq!(std::fs::read(&stages[0].path).unwrap(), OWNED);
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[test]
fn private_source_swap_after_publish_stat_cannot_be_reported_as_success() {
    let fixture = Fixture::new("publish-source-race");
    let (mut journal, batch) = seeded(&fixture.0);
    let source = fixture.0.join("source.bin");
    let target = fixture.0.join("target.bin");
    let _scope = move_scope(&batch);
    let hook_target = target.clone();
    let mut injected = false;
    let _hook = pinned_paths::rename_hook(Box::new(move |from, to| {
        if !injected && to == hook_target {
            std::fs::rename(from, from.with_file_name("preserved-owned")).unwrap();
            std::fs::write(from, FOREIGN).unwrap();
            injected = true;
        }
    }));
    let result = staged_move(
        &mut journal,
        batch.id,
        &batch.copy_stamps[0],
        &source,
        &target,
        &mut false,
    );
    assert!(
        matches!(result, Err(PlanError::RecoveryRequired { .. })),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), FOREIGN);
    let stages = journal.move_stages(batch.id).unwrap();
    let preserved = stages[0].path.with_file_name("preserved-owned");
    assert_eq!(std::fs::read(&preserved).unwrap(), OWNED);
    // The sealed recovery directory must remain discoverable, including when
    // its usual entry name disappeared during the race.
    assert!(recover_move_stages(&stages).is_err());
    assert_eq!(std::fs::read(&preserved).unwrap(), OWNED);
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[test]
fn replaced_private_directory_after_restart_is_refused_and_retry_is_safe() {
    let fixture = Fixture::new("private-directory-restart");
    let (mut journal, batch) = seeded(&fixture.0);
    let source = fixture.0.join("source.bin");
    let target = fixture.0.join("target.bin");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = move_scope(&batch);
        staged_move_with(
            &mut journal,
            batch.id,
            &batch.copy_stamps[0],
            &source,
            &target,
            &mut false,
            |phase| {
                if phase == "staged" {
                    panic!("fixture-only staged interruption");
                }
            },
        )
        .unwrap();
    }));
    assert!(result.is_err());
    let stage = journal.move_stages(batch.id).unwrap().remove(0);
    drop(journal);
    let directory = stage.path.parent().unwrap();
    let saved = fixture.0.join("preserved-reservation");
    std::fs::rename(directory, &saved).unwrap();
    std::fs::create_dir(directory).unwrap();
    std::fs::write(&stage.path, OWNED).unwrap();
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let error =
        Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap_err();
    assert!(
        matches!(error, PlanError::RecoveryRequired { .. }),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&stage.path).unwrap(), OWNED);
    assert_eq!(std::fs::read(saved.join("entry")).unwrap(), OWNED);
    assert!(!source.exists());
    assert!(!target.exists());
    assert!(journal.batch(batch.id).unwrap().is_some());
    std::fs::remove_file(&stage.path).unwrap();
    std::fs::remove_dir(directory).unwrap();
    std::fs::rename(&saved, directory).unwrap();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), OWNED);
    assert!(journal.move_stages(batch.id).unwrap().is_empty());
}

#[test]
fn real_sqlite_full_during_stage_intent_does_not_move_any_payload() {
    let fixture = Fixture::new("sqlite-full-stage");
    let mut nested = fixture.0.clone();
    // A long valid source directory makes the journal stage row require an
    // overflow page. The actual SQLite pager cap then produces SQLITE_FULL.
    for index in 0..7 {
        nested.push(format!("{index}-{}", "x".repeat(180)));
        std::fs::create_dir(&nested).unwrap();
    }
    let (mut journal, batch) = seeded_at(&nested, &fixture.0.join("journal.sqlite"));
    journal.test_limit_to_current_pages().unwrap();
    let source = nested.join("source.bin");
    let target = nested.join("target.bin");
    let _scope = move_scope(&batch);
    let mut attempted = false;
    let error = staged_move(
        &mut journal,
        batch.id,
        &batch.copy_stamps[0],
        &source,
        &target,
        &mut attempted,
    )
    .unwrap_err();
    assert!(
        matches!(&error,PlanError::Journal(JournalError::Storage(message)) if message.contains("full")),
        "{error:?}"
    );
    assert!(!attempted);
    assert_eq!(std::fs::read(&source).unwrap(), OWNED);
    assert!(!target.exists());
    assert!(journal.move_stages(batch.id).unwrap().is_empty());
    assert!(journal.batch(batch.id).unwrap().is_some());
    assert!(!std::fs::read_dir(&nested).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tagrex-move-")));
}

#[test]
fn process_death_with_an_unverified_foreign_capture_keeps_every_payload_and_history() {
    for undo in [false, true] {
        let fixture = Fixture::new("foreign-capture-crash");
        let (journal, batch) = seeded(&fixture.0);
        drop(journal);
        let source = fixture.0.join("source.bin");
        let target = fixture.0.join("target.bin");
        if undo {
            std::fs::rename(&source, &target).unwrap();
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "plan::move_staging_tests::move_stage_child",
                "--nocapture",
            ])
            .env("TAGREX_MOVE_STAGE_TEST_ROOT", &fixture.0)
            .env("TAGREX_MOVE_STAGE_TEST_PHASE", "staged")
            .env("TAGREX_MOVE_STAGE_TEST_SWAP", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if undo {
            command.env("TAGREX_MOVE_STAGE_TEST_UNDO", "1");
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(83));
                break;
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("foreign-capture child timed out");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        let stage = journal.move_stages(batch.id).unwrap().remove(0);
        assert_eq!(std::fs::read(&stage.path).unwrap(), FOREIGN);
        let error =
            Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap_err();
        assert!(
            matches!(error, PlanError::RecoveryRequired { .. }),
            "{error:?}"
        );
        assert_eq!(std::fs::read(&stage.from).unwrap(), FOREIGN);
        assert_eq!(
            std::fs::read(fixture.0.join("preserved-original.bin")).unwrap(),
            OWNED
        );
        assert!(journal.batch(batch.id).unwrap().is_some());
    }
}

#[test]
fn apply_retains_recovery_required_evidence_even_if_owned_source_is_restored_during_race() {
    let fixture = Fixture::new("recovery-cause-retained");
    let source = fixture.0.join("source.wav");
    let target = fixture.0.join("target.wav");
    let saved = fixture.0.join("preserved-original.wav");
    let foreign_saved = fixture.0.join("preserved-foreign.bin");
    let mut bytes = b"RIFF".to_vec();
    bytes.extend(356u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(8000u32.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(320u32.to_le_bytes());
    bytes.extend([0; 320]);
    std::fs::write(&source, &bytes).unwrap();
    let hook_source = source.clone();
    let hook_saved = saved.clone();
    let hook_foreign = foreign_saved.clone();
    let mut swapped = false;
    let _hook = pinned_paths::rename_hook(Box::new(move |from, to| {
        if !swapped && from == hook_source && to.file_name().is_some_and(|name| name == "entry") {
            std::fs::rename(&hook_source, &hook_saved).unwrap();
            std::fs::write(&hook_source, FOREIGN).unwrap();
            swapped = true;
        } else if swapped
            && to == hook_source
            && from.file_name().is_some_and(|name| name == "entry")
        {
            std::fs::rename(from, &hook_foreign).unwrap();
            std::fs::hard_link(&hook_saved, &hook_source).unwrap();
        }
    }));
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let result = Executor::apply(
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
    );
    assert!(
        matches!(result, Err(PlanError::RecoveryRequired { .. })),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert_eq!(std::fs::read(&saved).unwrap(), bytes);
    assert_eq!(std::fs::read(&foreign_saved).unwrap(), FOREIGN);
    assert!(!target.exists());
    assert_eq!(journal.batches().unwrap().len(), 1);
}

#[test]
fn a_foreign_journal_without_explicit_staging_support_refuses_before_payload_changes() {
    struct Foreign(SqliteJournal);
    impl UndoJournal for Foreign {
        fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError> {
            self.0.record(batch)
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            self.0.batches()
        }
        fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError> {
            self.0.finish(id, batch)
        }
        fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
            self.0.rollback(id)
        }
    }
    let fixture = Fixture::new("foreign-journal-refusal");
    let (journal, batch) = seeded(&fixture.0);
    let mut journal = Foreign(journal);
    let source = fixture.0.join("source.bin");
    let target = fixture.0.join("target.bin");
    let _scope = move_scope(&batch);
    let mut attempted = false;
    let error = staged_move(
        &mut journal,
        batch.id,
        &batch.copy_stamps[0],
        &source,
        &target,
        &mut attempted,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        PlanError::Journal(JournalError::NotRollbackable(_))
    ));
    assert!(!attempted);
    assert_eq!(std::fs::read(&source).unwrap(), OWNED);
    assert!(!target.exists());
    assert!(journal.0.move_stages(batch.id).unwrap().is_empty());
    assert!(!std::fs::read_dir(&fixture.0).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tagrex-move-")));
}

fn paused_stage(journal: &mut SqliteJournal, batch: &AppliedBatch, phase: &str) -> MoveStage {
    let change = &batch.plan.changes[0];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = move_scope(batch);
        staged_move_with(
            journal,
            batch.id,
            &batch.copy_stamps[0],
            &change.path,
            change.rename_to.as_ref().unwrap(),
            &mut false,
            |at| {
                if at == phase {
                    panic!("fixture-only paused stage");
                }
            },
        )
        .unwrap();
    }));
    assert!(result.is_err());
    journal.move_stages(batch.id).unwrap().pop().unwrap()
}

#[test]
fn duplicate_and_malformed_later_stage_rows_refuse_before_cleanup_or_restore() {
    for damage in [
        "duplicate-row",
        "duplicate-directory",
        "bad-version",
        "bad-entry-name",
        "bad-parent",
        "directory-payload",
    ] {
        let fixture = Fixture::new(damage);
        let (mut journal, batch) = seeded(&fixture.0);
        let first = paused_stage(&mut journal, &batch, "intent");
        let second = paused_stage(&mut journal, &batch, "staged");
        let mut rows = vec![first.clone(), second.clone()];
        match damage {
            "duplicate-row" => rows.push(second.clone()),
            "duplicate-directory" => rows[1].directory_identity = first.directory_identity.clone(),
            "bad-version" => rows[1].directory_identity[5] = b'9',
            "bad-entry-name" => rows[1].path = second.path.with_file_name("other-entry"),
            "bad-parent" => {
                rows[1].path = fixture
                    .0
                    .join("unapproved-parent")
                    .join(second.path.parent().unwrap().file_name().unwrap())
                    .join("entry")
            }
            "directory-payload" => {
                std::fs::rename(&second.path, fixture.0.join("preserved-owned.bin")).unwrap();
                std::fs::create_dir(&second.path).unwrap();
            }
            _ => unreachable!(),
        }
        let _scope = move_scope(&batch);
        let error = recover_move_stages(&rows).unwrap_err();
        assert!(
            matches!(error, PlanError::RecoveryRequired { .. }),
            "{damage}: {error:?}"
        );
        assert!(
            first.path.parent().unwrap().exists(),
            "earlier empty reservation must not be cleaned before rejecting later context"
        );
        if damage == "directory-payload" {
            assert!(second.path.is_dir());
            assert_eq!(
                std::fs::read(fixture.0.join("preserved-owned.bin")).unwrap(),
                OWNED
            );
        } else {
            assert_eq!(std::fs::read(&second.path).unwrap(), OWNED);
        }
        assert!(!fixture.0.join("source.bin").exists());
        assert!(!fixture.0.join("target.bin").exists());
        assert!(journal.batch(batch.id).unwrap().is_some());
    }
}

#[test]
fn matching_but_malformed_persisted_proof_and_stage_identity_never_authorize_recovery() {
    for mutation in [
        "unsupported-version",
        "truncated",
        "extra-byte",
        "zero-object",
        "invalid-entry-kind",
        "invalid-birth",
    ] {
        let fixture = Fixture::new(mutation);
        let (mut journal, batch) = seeded(&fixture.0);
        let stage = paused_stage(&mut journal, &batch, "staged");
        let mut identity = stage.identity.clone();
        match mutation {
            "unsupported-version" => identity[5] = b'9',
            "truncated" => {
                identity.pop();
            }
            "extra-byte" => identity.push(0),
            #[cfg(unix)]
            "zero-object" => identity[14..22].fill(0),
            #[cfg(windows)]
            "zero-object" => identity[14..30].fill(0),
            #[cfg(unix)]
            "invalid-entry-kind" => identity[38] = 3,
            #[cfg(windows)]
            "invalid-entry-kind" => identity[5] = b'9',
            #[cfg(unix)]
            "invalid-birth" => identity[22..38].copy_from_slice(&i128::MIN.to_le_bytes()),
            #[cfg(windows)]
            "invalid-birth" => identity[30..46].copy_from_slice(&i128::MIN.to_le_bytes()),
            _ => unreachable!(),
        }
        let sql = rusqlite::Connection::open(fixture.0.join("journal.sqlite")).unwrap();
        sql.execute(
            "UPDATE move_stages SET identity=?1 WHERE batch_id=?2",
            rusqlite::params![identity, batch.id.0],
        )
        .unwrap();
        sql.execute(
            "UPDATE copy_stamps SET identity=?1 WHERE batch_id=?2",
            rusqlite::params![identity, batch.id.0],
        )
        .unwrap();
        drop(sql);
        let error =
            Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap_err();
        assert!(
            matches!(error, PlanError::RecoveryRequired { .. }),
            "{mutation}: {error:?}"
        );
        assert_eq!(std::fs::read(&stage.path).unwrap(), OWNED);
        assert!(!fixture.0.join("source.bin").exists());
        assert!(!fixture.0.join("target.bin").exists());
        assert!(journal.batch(batch.id).unwrap().is_some());
    }
}

#[test]
fn foreign_journal_duplicate_collection_is_rejected_before_any_filesystem_effect() {
    struct Duplicate(SqliteJournal);
    impl UndoJournal for Duplicate {
        fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError> {
            self.0.record(batch)
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            self.0.batches()
        }
        fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError> {
            self.0.finish(id, batch)
        }
        fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
            self.0.rollback(id)
        }
        fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
            let mut stages = self.0.move_stages(id)?;
            stages.push(stages[0].clone());
            Ok(stages)
        }
    }
    let fixture = Fixture::new("foreign-duplicate-context");
    let (mut journal, batch) = seeded(&fixture.0);
    let stage = paused_stage(&mut journal, &batch, "staged");
    let mut foreign = Duplicate(journal);
    assert!(Executor::undo(&mut foreign, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    assert_eq!(std::fs::read(&stage.path).unwrap(), OWNED);
    assert!(!fixture.0.join("source.bin").exists());
    assert!(!fixture.0.join("target.bin").exists());
    assert!(foreign.0.batch(batch.id).unwrap().is_some());
}

#[test]
fn actual_crash_with_completed_and_pending_stages_validates_even_completed_context_before_recovery()
{
    for corrupt_completed in [false, true] {
        let fixture = Fixture::new("mixed-stage-context-crash");
        let (mut journal, mut batch) = seeded(&fixture.0);
        journal.rollback(batch.id).unwrap();
        let second_source = fixture.0.join("second-source.bin");
        let second_target = fixture.0.join("second-target.bin");
        let second_bytes = b"second owned transfer bytes";
        std::fs::write(&second_source, second_bytes).unwrap();
        let scope = pinned_paths::Scope::enter(
            &batch.allowed_roots,
            [second_source.clone(), second_target.clone()],
        )
        .unwrap();
        let proof = CopyStamp::for_move(&second_source, &second_target).unwrap();
        drop(scope);
        batch.plan.changes.push(FileChange {
            path: second_source.clone(),
            rename_to: Some(second_target.clone()),
            ..Default::default()
        });
        batch.copy_stamps.push(proof);
        batch.id = journal.record(&batch).unwrap();
        drop(journal);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "plan::move_staging_tests::move_stage_child",
                "--nocapture",
            ])
            .env("TAGREX_MOVE_STAGE_TEST_ROOT", &fixture.0)
            .env("TAGREX_MOVE_STAGE_TEST_PHASE", "staged")
            .env("TAGREX_MOVE_STAGE_TEST_PAIR", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(83));
                break;
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("mixed stage child timed out");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        let stages = journal.move_stages(batch.id).unwrap();
        assert_eq!(stages.len(), 2);
        assert!(!stages[0].path.exists());
        assert_eq!(std::fs::read(&stages[1].path).unwrap(), second_bytes);
        if corrupt_completed {
            let sql = rusqlite::Connection::open(fixture.0.join("journal.sqlite")).unwrap();
            sql.execute(
                "UPDATE move_stages SET source=?1 WHERE batch_id=?2 AND path=?3",
                rusqlite::params![
                    fixture.0.join("never-approved.bin").to_str().unwrap(),
                    batch.id.0,
                    stages[0].path.to_str().unwrap()
                ],
            )
            .unwrap();
            drop(sql);
            assert!(
                Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err()
            );
            assert!(!fixture.0.join("source.bin").exists());
            assert_eq!(std::fs::read(fixture.0.join("target.bin")).unwrap(), OWNED);
            assert!(!second_source.exists());
            assert!(!second_target.exists());
            assert_eq!(std::fs::read(&stages[1].path).unwrap(), second_bytes);
            assert!(journal.batch(batch.id).unwrap().is_some());
        } else {
            Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
            assert_eq!(std::fs::read(fixture.0.join("source.bin")).unwrap(), OWNED);
            assert_eq!(std::fs::read(&second_source).unwrap(), second_bytes);
            assert!(!fixture.0.join("target.bin").exists());
            assert!(!second_target.exists());
            assert!(journal.batches().unwrap().is_empty());
        }
    }
}

#[test]
fn invalid_stored_stage_context_stops_rollback_before_reversing_any_completed_move() {
    let fixture = Fixture::new("rollback-invalid-collection");
    let (mut journal, batch) = seeded(&fixture.0);
    let source = fixture.0.join("source.bin");
    let target = fixture.0.join("target.bin");
    let _scope = move_scope(&batch);
    staged_move(
        &mut journal,
        batch.id,
        &batch.copy_stamps[0],
        &source,
        &target,
        &mut false,
    )
    .unwrap();
    let sql = rusqlite::Connection::open(fixture.0.join("journal.sqlite")).unwrap();
    sql.execute(
        "UPDATE move_stages SET source=?1 WHERE batch_id=?2",
        rusqlite::params![
            fixture.0.join("unplanned.bin").to_str().unwrap(),
            batch.id.0
        ],
    )
    .unwrap();
    drop(sql);
    let mut written = Written::new(1);
    written.transferred.push(0);
    written
        .copy_stamps
        .insert(target.clone(), batch.copy_stamps[0].clone());
    let failed = written.roll_back(&batch.plan, &[], &mut journal, batch.id);
    assert!(!failed.is_empty());
    assert!(!source.exists());
    assert_eq!(std::fs::read(&target).unwrap(), OWNED);
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[test]
fn stale_empty_intents_over_retry_budget_refuse_without_cleaning_any_reservation() {
    let fixture = Fixture::new("retry-context-budget");
    let (mut journal, batch) = seeded(&fixture.0);
    for _ in 0..65 {
        paused_stage(&mut journal, &batch, "intent");
    }
    let stages = journal.move_stages(batch.id).unwrap();
    assert_eq!(stages.len(), 65);
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    assert_eq!(std::fs::read(fixture.0.join("source.bin")).unwrap(), OWNED);
    assert!(!fixture.0.join("target.bin").exists());
    assert!(stages
        .iter()
        .all(|stage| stage.path.parent().unwrap().is_dir()));
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[test]
fn oversized_stored_stage_cells_are_refused_before_loading_or_recovering_payloads() {
    for column in ["source", "identity"] {
        let fixture = Fixture::new("oversized-stage-cell");
        let (mut journal, batch) = seeded(&fixture.0);
        let stage = paused_stage(&mut journal, &batch, "staged");
        let sql = rusqlite::Connection::open(fixture.0.join("journal.sqlite")).unwrap();
        if column == "source" {
            sql.execute(
                "UPDATE move_stages SET source=?1 WHERE batch_id=?2",
                rusqlite::params!["x".repeat(131_073), batch.id.0],
            )
            .unwrap();
        } else {
            sql.execute(
                "UPDATE move_stages SET identity=zeroblob(65) WHERE batch_id=?1",
                [batch.id.0],
            )
            .unwrap();
        }
        drop(sql);
        assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
        assert_eq!(std::fs::read(&stage.path).unwrap(), OWNED);
        assert!(!fixture.0.join("source.bin").exists());
        assert!(!fixture.0.join("target.bin").exists());
        assert!(journal.batch(batch.id).unwrap().is_some());
    }
}

#[test]
fn windows_identity_codec_requires_explicit_file_or_directory_kind() {
    // This verifies the versioned wire contract on any host; it is not a native
    // Windows filesystem or FileIdInfo certification.
    for kind in 0..=2 {
        let mut bytes = b"TRX-W2".to_vec();
        bytes.extend(9u64.to_le_bytes());
        bytes.extend([1; 16]);
        bytes.extend(0i128.to_le_bytes());
        bytes.push(kind);
        let identity = NativeIdentity::decode(&bytes).unwrap();
        assert_eq!(identity.file_role(false), kind == 0);
        assert_eq!(identity.file_role(true), kind <= 1);
        assert_eq!(identity.directory_role(), kind == 2);
        bytes[46] = 3;
        assert!(NativeIdentity::decode(&bytes).is_none());
        bytes.pop();
        bytes[5] = b'1';
        let legacy = NativeIdentity::decode(&bytes).unwrap();
        assert!(!legacy.file_role(true));
        assert!(!legacy.directory_role());
    }
}

#[test]
fn new_intents_are_admitted_against_the_same_retry_budget_used_for_recovery() {
    let fixture = Fixture::new("new-intent-retry-budget");
    let (mut journal, batch) = seeded(&fixture.0);
    let source = fixture.0.join("source.bin");
    let target = fixture.0.join("target.bin");
    let _scope = move_scope(&batch);
    for _ in 0..64 {
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| staged_move_with(
                &mut journal,
                batch.id,
                &batch.copy_stamps[0],
                &source,
                &target,
                &mut false,
                |phase| {
                    if phase == "intent" {
                        panic!("fixture-only committed empty intent");
                    }
                }
            )))
            .is_err()
        );
    }
    let mut attempted = false;
    let error = staged_move(
        &mut journal,
        batch.id,
        &batch.copy_stamps[0],
        &source,
        &target,
        &mut attempted,
    )
    .unwrap_err();
    assert!(matches!(error, PlanError::RecoveryRequired { .. }));
    assert!(!attempted);
    assert_eq!(journal.move_stages(batch.id).unwrap().len(), 64);
    assert_eq!(std::fs::read(&source).unwrap(), OWNED);
    assert!(!target.exists());
    let directories = std::fs::read_dir(&fixture.0)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tagrex-move-")
        })
        .count();
    assert_eq!(
        directories, 64,
        "refused intent must not leak an extra empty reservation"
    );
}

#[test]
fn unix_u2_roles_are_distinct_and_u1_false_is_not_typed_authority() {
    for kind in 0..=2 {
        let mut bytes = b"TRX-U2".to_vec();
        bytes.extend(7u64.to_le_bytes());
        bytes.extend(11u64.to_le_bytes());
        bytes.extend(0i128.to_le_bytes());
        bytes.push(kind);
        let identity = NativeIdentity::decode(&bytes).unwrap();
        assert_eq!(identity.file_role(false), kind == 0);
        assert_eq!(identity.file_role(true), kind <= 1);
        assert_eq!(identity.directory_role(), kind == 2);
        bytes[38] = 3;
        assert!(NativeIdentity::decode(&bytes).is_none());
        bytes[5] = b'1';
        bytes[38] = 0;
        let legacy = NativeIdentity::decode(&bytes).unwrap();
        assert!(!legacy.file_role(true));
        assert!(!legacy.directory_role());
        bytes[38] = 1;
        let link = NativeIdentity::decode(&bytes).unwrap();
        assert!(link.file_role(true));
        assert!(!link.file_role(false));
        assert!(!link.directory_role());
        bytes[5] = b'9';
        assert!(NativeIdentity::decode(&bytes).is_none());
    }
}

#[cfg(unix)]
#[test]
fn legacy_unix_nonlink_proofs_and_directory_seals_stay_readable_without_recovery_effects() {
    for field in ["file-proof", "directory-seal"] {
        let fixture = Fixture::new("legacy-u1-ambiguous");
        let (mut journal, batch) = seeded(&fixture.0);
        let stage = paused_stage(&mut journal, &batch, "staged");
        drop(journal);
        let mut legacy = if field == "file-proof" {
            stage.identity.clone()
        } else {
            stage.directory_identity.clone()
        };
        assert_eq!(&legacy[..6], b"TRX-U2");
        legacy[5] = b'1';
        legacy[38] = 0;
        let sql = rusqlite::Connection::open(fixture.0.join("journal.sqlite")).unwrap();
        if field == "file-proof" {
            sql.execute(
                "UPDATE copy_stamps SET identity=?1 WHERE batch_id=?2",
                rusqlite::params![legacy, batch.id.0],
            )
            .unwrap();
            sql.execute(
                "UPDATE move_stages SET identity=?1 WHERE batch_id=?2",
                rusqlite::params![legacy, batch.id.0],
            )
            .unwrap();
        } else {
            sql.execute(
                "UPDATE move_stages SET directory_identity=?1 WHERE batch_id=?2",
                rusqlite::params![legacy, batch.id.0],
            )
            .unwrap();
        }
        drop(sql);
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        assert!(journal.batch(batch.id).unwrap().is_some());
        assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
        assert_eq!(std::fs::read(&stage.path).unwrap(), OWNED);
        assert!(!fixture.0.join("source.bin").exists());
        assert!(!fixture.0.join("target.bin").exists());
        assert!(stage.path.parent().unwrap().is_dir());
        assert!(journal.batch(batch.id).unwrap().is_some());
    }
}

#[cfg(unix)]
#[test]
fn genuine_unix_file_identity_cannot_authorize_reservation_directory_cleanup() {
    let fixture = Fixture::new("u2-file-as-directory");
    let (mut journal, batch) = seeded(&fixture.0);
    let stage = paused_stage(&mut journal, &batch, "staged");
    let sql = rusqlite::Connection::open(fixture.0.join("journal.sqlite")).unwrap();
    sql.execute(
        "UPDATE move_stages SET directory_identity=identity WHERE batch_id=?1",
        [batch.id.0],
    )
    .unwrap();
    drop(sql);
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    assert_eq!(std::fs::read(&stage.path).unwrap(), OWNED);
    assert!(stage.path.parent().unwrap().is_dir());
    assert!(!fixture.0.join("source.bin").exists());
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[cfg(target_os = "linux")]
fn role_wave(sample: u8) -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend(356u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(8000u32.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(320u32.to_le_bytes());
    bytes.extend([sample; 320]);
    bytes
}
#[cfg(target_os = "linux")]
fn role_link_plan(root: &Path, include_copy: bool) -> ChangePlan {
    let mut changes = vec![FileChange {
        path: root.join("first.wav"),
        rename_to: Some(root.join("moved/first.wav")),
        sidecar_renames: vec![(root.join("first.cue"), root.join("moved/first.cue"))],
        ..Default::default()
    }];
    if include_copy {
        changes.push(FileChange {
            path: root.join("second.wav"),
            rename_to: Some(root.join("moved/copy.wav")),
            copy: true,
            sidecar_renames: vec![(root.join("second.lrc"), root.join("moved/copy.lrc"))],
            ..Default::default()
        });
    }
    ChangePlan {
        changes,
        ..Default::default()
    }
}
#[cfg(target_os = "linux")]
#[test]
fn unix_legacy_link_child() {
    let Some(root) = std::env::var_os("TAGREX_U1_LINK_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    struct PauseLink(SqliteJournal);
    impl UndoJournal for PauseLink {
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
        fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
            self.0.move_stages(id)
        }
        fn prepare_move_stage(
            &mut self,
            id: BatchId,
            stage: &MoveStage,
        ) -> Result<(), JournalError> {
            self.0.prepare_move_stage(id, stage)?;
            if stage
                .target
                .extension()
                .is_some_and(|extension| extension == "cue")
            {
                std::fs::rename(&stage.from, &stage.path).unwrap();
                std::process::exit(83);
            }
            Ok(())
        }
    }
    let mut journal = PauseLink(SqliteJournal::open(&root.join("journal.sqlite")).unwrap());
    Executor::apply(
        &role_link_plan(&root, false),
        &mut journal,
        std::slice::from_ref(&root),
    )
    .unwrap();
    panic!("link capture checkpoint not reached");
}
#[cfg(target_os = "linux")]
#[test]
fn explicit_u1_link_identity_replays_with_u2_seals_across_crash_and_mixed_copy_restart() {
    use std::os::unix::fs::symlink;
    for interrupted in [false, true] {
        let fixture = Fixture::new("typed-u1-link");
        let first = fixture.0.join("first.wav");
        let second = fixture.0.join("second.wav");
        std::fs::write(&first, role_wave(1)).unwrap();
        std::fs::write(&second, role_wave(2)).unwrap();
        std::fs::write(
            fixture.0.join("cue-content"),
            b"must not follow or change link target",
        )
        .unwrap();
        std::fs::write(fixture.0.join("second.lrc"), b"original copy sidecar").unwrap();
        symlink("cue-content", fixture.0.join("first.cue")).unwrap();
        if interrupted {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "plan::move_staging_tests::unix_legacy_link_child",
                    "--nocapture",
                ])
                .env("TAGREX_U1_LINK_TEST_ROOT", &fixture.0)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert_eq!(status.code(), Some(83));
                    break;
                }
                if Instant::now() > deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("legacy link child timed out");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        } else {
            let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
            Executor::apply(
                &role_link_plan(&fixture.0, true),
                &mut journal,
                std::slice::from_ref(&fixture.0),
            )
            .unwrap();
        }
        let journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        let batch = journal.batches().unwrap().remove(0);
        let target = fixture.0.join("moved/first.cue");
        let mut legacy = batch
            .copy_stamps
            .iter()
            .find(|stamp| stamp.path == target)
            .unwrap()
            .identity
            .clone()
            .unwrap();
        assert_eq!(&legacy[..6], b"TRX-U2");
        assert_eq!(legacy[38], 1);
        legacy[5] = b'1';
        drop(journal);
        let sql = rusqlite::Connection::open(fixture.0.join("journal.sqlite")).unwrap();
        sql.execute(
            "UPDATE copy_stamps SET identity=?1 WHERE batch_id=?2 AND path=?3",
            rusqlite::params![legacy, batch.id.0, target.to_str().unwrap()],
        )
        .unwrap();
        sql.execute(
            "UPDATE move_stages SET identity=?1 WHERE batch_id=?2 AND target=?3",
            rusqlite::params![legacy, batch.id.0, target.to_str().unwrap()],
        )
        .unwrap();
        drop(sql);
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        assert!(journal
            .move_stages(batch.id)
            .unwrap()
            .iter()
            .all(|stage| stage.directory_identity.starts_with(b"TRX-U2")));
        Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), role_wave(1));
        assert_eq!(std::fs::read(&second).unwrap(), role_wave(2));
        assert_eq!(
            std::fs::read_link(fixture.0.join("first.cue")).unwrap(),
            Path::new("cue-content")
        );
        assert_eq!(
            std::fs::read(fixture.0.join("cue-content")).unwrap(),
            b"must not follow or change link target"
        );
        assert_eq!(
            std::fs::read(fixture.0.join("second.lrc")).unwrap(),
            b"original copy sidecar"
        );
        assert!(std::fs::symlink_metadata(&target).is_err());
        assert!(!fixture.0.join("moved/first.wav").exists());
        assert!(!fixture.0.join("moved/copy.wav").exists());
        assert!(!fixture.0.join("moved/copy.lrc").exists());
        assert!(journal.batches().unwrap().is_empty());
    }
}

#[cfg(unix)]
#[test]
fn copied_file_with_ambiguous_u1_native_evidence_is_kept_for_inspection() {
    let fixture = Fixture::new("legacy-u1-copy");
    let (mut journal, mut batch) = seeded(&fixture.0);
    journal.rollback(batch.id).unwrap();
    let target = fixture.0.join("target.bin");
    std::fs::write(&target, OWNED).unwrap();
    batch.plan.changes[0].copy = true;
    batch.interrupted = false;
    batch.copy_stamps = vec![CopyStamp::of(&target).unwrap()];
    batch.id = journal.record(&batch).unwrap();
    drop(journal);
    let mut legacy = batch.copy_stamps[0].identity.clone().unwrap();
    assert_eq!(&legacy[..6], b"TRX-U2");
    legacy[5] = b'1';
    legacy[38] = 0;
    let sql = rusqlite::Connection::open(fixture.0.join("journal.sqlite")).unwrap();
    sql.execute(
        "UPDATE copy_stamps SET identity=?1 WHERE batch_id=?2",
        rusqlite::params![legacy, batch.id.0],
    )
    .unwrap();
    drop(sql);
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    assert!(journal.batch(batch.id).unwrap().is_some());
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), OWNED);
    assert_eq!(std::fs::read(fixture.0.join("source.bin")).unwrap(), OWNED);
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Manual persistent-SQL/native exclusive component profile; no speed threshold"]
fn profile_staging_native_components() {
    use std::cell::Cell;
    #[derive(Clone, Copy, Default, Debug)]
    struct Cost {
        calls: usize,
        elapsed: Duration,
    }
    fn measured<T>(clock: &Cell<Cost>, work: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = work();
        let mut value = clock.get();
        value.calls += 1;
        value.elapsed += start.elapsed();
        clock.set(value);
        out
    }
    struct Timed {
        inner: SqliteJournal,
        intent: Cell<Cost>,
        other: Cell<Cost>,
    }
    impl UndoJournal for Timed {
        fn prepare_move_stage(
            &mut self,
            id: BatchId,
            stage: &MoveStage,
        ) -> Result<(), JournalError> {
            measured(&self.intent, || self.inner.prepare_move_stage(id, stage))
        }
        fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
            measured(&self.other, || self.inner.move_stages(id))
        }
        fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
            measured(&self.other, || self.inner.record(b))
        }
        fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
            measured(&self.other, || self.inner.batch(id))
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            measured(&self.other, || self.inner.batches())
        }
        fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
            measured(&self.other, || self.inner.finish(id, b))
        }
        fn retain_interrupted(
            &mut self,
            id: BatchId,
            b: &AppliedBatch,
        ) -> Result<(), JournalError> {
            measured(&self.other, || self.inner.retain_interrupted(id, b))
        }
        fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
            measured(&self.other, || self.inner.rollback(id))
        }
    }
    use sha2::{Digest, Sha256};
    let hex = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    eprintln!(
        "MOVE_NATIVE_SOURCE plan={} journal={} paths={}",
        hex(include_bytes!("plan.rs")),
        hex(include_bytes!("journal.rs")),
        hex(include_bytes!("plan_paths.rs"))
    );
    for fanout in [false, true] {
        let fixture = Fixture::new("native-profile");
        let library = fixture.0.join("library");
        std::fs::create_dir(&library).unwrap();
        let bytes = role_wave(0);
        let mut changes = Vec::new();
        for index in 0..1000 {
            let parent = library.join(if fanout {
                format!("album-{index}")
            } else {
                "album".into()
            });
            std::fs::create_dir_all(&parent).unwrap();
            let source = parent.join(format!("{index}.wav"));
            std::fs::write(&source, &bytes).unwrap();
            changes.push(FileChange {
                path: source,
                rename_to: Some(library.join(format!("moved/{index}.wav"))),
                ..Default::default()
            });
        }
        let plan = ChangePlan {
            changes,
            ..Default::default()
        };
        let mut journal = Timed {
            inner: SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap(),
            intent: Cell::default(),
            other: Cell::default(),
        };
        for cache in ["data-advised", "warm-repeat"] {
            if cache == "data-advised" {
                for change in &plan.changes {
                    let file = std::fs::File::open(&change.path).unwrap();
                    file.sync_all().unwrap();
                    rustix::fs::fadvise(&file, 0, None, rustix::fs::Advice::DontNeed).unwrap();
                }
            }
            journal.intent.set(Cost::default());
            journal.other.set(Cost::default());
            move_metrics::take();
            let baseline = std::fs::read_dir("/proc/self/fd").unwrap().count();
            let mut extra = 0usize;
            let mut progress = |event: &ApplyProgress| {
                if event.done.is_multiple_of(128) {
                    extra = extra.max(
                        std::fs::read_dir("/proc/self/fd")
                            .unwrap()
                            .count()
                            .saturating_sub(baseline),
                    );
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
            let apply = start.elapsed();
            let native = move_metrics::take();
            let intent = journal.intent.replace(Cost::default());
            let other = journal.other.replace(Cost::default());
            assert_eq!(native.capture.calls, 1000);
            assert_eq!(native.publication.calls, 1000);
            eprintln!("MOVE_NATIVE cache={cache} metadata_cache=uncontrolled fanout={fanout} phase=apply n=1000 total={apply:?} sql_intent={intent:?} sql_other={other:?} native={native:?} unclassified={:?}",apply.saturating_sub(intent.elapsed+other.elapsed+native.total()));
            let start = Instant::now();
            Executor::undo_with(
                &mut journal,
                batch.id,
                std::slice::from_ref(&library),
                &mut progress,
            )
            .unwrap();
            let undo = start.elapsed();
            let native = move_metrics::take();
            let intent = journal.intent.replace(Cost::default());
            let other = journal.other.replace(Cost::default());
            assert_eq!(native.capture.calls, 1000);
            assert_eq!(native.publication.calls, 1000);
            eprintln!("MOVE_NATIVE cache={cache} metadata_cache=uncontrolled fanout={fanout} phase=undo n=1000 total={undo:?} sql_intent={intent:?} sql_other={other:?} native={native:?} unclassified={:?} extra_callback_fds={extra}",undo.saturating_sub(intent.elapsed+other.elapsed+native.total()));
            assert!(extra < 64);
            assert!(journal.batches().unwrap().is_empty());
            for change in &plan.changes {
                assert_eq!(std::fs::read(&change.path).unwrap(), bytes);
                assert!(!change.rename_to.as_ref().unwrap().exists());
            }
            assert!(!library.join("moved").exists());
        }
    }
}
