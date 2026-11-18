//! Real process exits across the durable copy-removal protocol. The private
//! checkpoint callback is inert in production; no environment hook ships.
use super::*;
use crate::journal::SqliteJournal;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CONTENT: &[u8] = b"owned copy recovery fixture";
const EXTERNAL: &[u8] = b"new unrelated file at the old copy path";

struct Fixture(PathBuf);
impl Fixture {
    fn new(stage: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-copy-removal-{stage}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn seed(root: &Path) {
    let source = root.join("original.bin");
    let target = root.join("copy.bin");
    std::fs::write(&source, CONTENT).unwrap();
    std::fs::write(&target, CONTENT).unwrap();
    let batch = AppliedBatch {
        native_tags: false,
        id: BatchId(0),
        description: "copy deletion process probe".into(),
        message: None,
        applied_at: 0,
        plan: ChangePlan {
            changes: vec![FileChange {
                path: source,
                rename_to: Some(target.clone()),
                copy: true,
                ..Default::default()
            }],
            ..Default::default()
        },
        created_dirs: vec![],
        removed_dirs: vec![],
        allowed_roots: vec![root.to_path_buf()],
        copy_stamps: vec![CopyStamp::of(&target).unwrap()],
        applied_values: vec![],
        block_fingerprints: vec![],
        interrupted: false,
        id3v2_revisions: vec![],
    };
    SqliteJournal::open(&root.join("journal.sqlite"))
        .unwrap()
        .record(&batch)
        .unwrap();
}

#[test]
fn copy_remove_child() {
    let Some(root) = std::env::var_os("TAGREX_COPY_REMOVAL_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let stage = std::env::var("TAGREX_COPY_REMOVAL_TEST_STAGE").unwrap();
    let mut journal = SqliteJournal::open(&root.join("journal.sqlite")).unwrap();
    let batch = journal.batches().unwrap().remove(0);
    let stamps = batch
        .copy_stamps
        .iter()
        .map(|stamp| (stamp.path.as_path(), stamp))
        .collect();
    remove_recorded_copy_with(
        &mut journal,
        batch.id,
        &root.join("copy.bin"),
        &stamps,
        |at| {
            if at == stage {
                std::process::exit(79);
            }
        },
    )
    .unwrap();
    panic!("copy-removal checkpoint was never reached");
}

#[test]
fn copy_removal_crash_intent_move_verify_delete_is_retryable() {
    for _ in 0..2 {
        for stage in ["intent", "checked", "moved", "verified", "removed"] {
            let fixture = Fixture::new(stage);
            seed(&fixture.0);
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "plan::copy_removal_tests::copy_remove_child",
                    "--nocapture",
                ])
                .env("TAGREX_COPY_REMOVAL_TEST_ROOT", &fixture.0)
                .env("TAGREX_COPY_REMOVAL_TEST_STAGE", stage)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert_eq!(status.code(), Some(79), "{stage}");
                    break;
                }
                if Instant::now() > deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("copy-removal child timed out");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
            let batch = journal.batches().unwrap().remove(0);
            let undo_path = batch.copy_stamps[0].undo_path.as_ref().unwrap();
            assert_eq!(undo_path.exists(), ["moved", "verified"].contains(&stage));
            let recreated = ["moved", "verified"].contains(&stage);
            if recreated {
                std::fs::write(fixture.0.join("copy.bin"), EXTERNAL).unwrap();
            }
            Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
            assert!(journal.batches().unwrap().is_empty());
            assert_eq!(
                std::fs::read(fixture.0.join("original.bin")).unwrap(),
                CONTENT
            );
            if recreated {
                assert_eq!(std::fs::read(fixture.0.join("copy.bin")).unwrap(), EXTERNAL);
            } else {
                assert!(!fixture.0.join("copy.bin").exists());
            }
            assert!(!undo_path.exists());
            assert!(!undo_path.parent().unwrap().exists());
        }
    }
}

#[test]
fn replacement_between_digest_check_and_move_is_retained_and_tracked() {
    let fixture = Fixture::new("replacement-before-move");
    seed(&fixture.0);
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let batch = journal.batches().unwrap().remove(0);
    let target = fixture.0.join("copy.bin");
    let stamps = batch
        .copy_stamps
        .iter()
        .map(|stamp| (stamp.path.as_path(), stamp))
        .collect();
    let error = remove_recorded_copy_with(&mut journal, batch.id, &target, &stamps, |at| {
        if at == "checked" {
            std::fs::write(&target, EXTERNAL).unwrap();
        }
        if at == "moved" {
            let stored = SqliteJournal::open(&fixture.0.join("journal.sqlite"))
                .unwrap()
                .batch(batch.id)
                .unwrap()
                .unwrap();
            let private = stored.copy_stamps[0].undo_path.as_ref().unwrap();
            assert_eq!(std::fs::read(private).unwrap(), EXTERNAL);
            std::fs::write(&target, b"separate new target").unwrap();
        }
    })
    .unwrap_err();
    assert!(matches!(error, PlanError::Stale(_)));
    let stored = journal.batch(batch.id).unwrap().unwrap();
    let recovery = stored.copy_stamps[0].undo_path.as_ref().unwrap();
    assert_eq!(std::fs::read(recovery).unwrap(), EXTERNAL);
    assert_eq!(std::fs::read(target).unwrap(), b"separate new target");
}

#[test]
fn concurrent_destination_creation_and_atomic_move_never_clobber() {
    use std::sync::{Arc, Barrier};
    let fixture = Fixture::new("no-replace-race");
    for index in 0..64 {
        let source = fixture.0.join(format!("source-{index}"));
        let target = fixture.0.join(format!("target-{index}"));
        std::fs::write(&source, CONTENT).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let contender = std::thread::spawn({
            let barrier = barrier.clone();
            let target = target.clone();
            move || {
                barrier.wait();
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(target)
                {
                    Ok(mut file) => {
                        use std::io::Write;
                        file.write_all(EXTERNAL).unwrap();
                        true
                    }
                    Err(error) => {
                        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
                        false
                    }
                }
            }
        });
        barrier.wait();
        let moved = rename_noreplace(&source, &target);
        let external_won = contender.join().unwrap();
        if external_won {
            assert_eq!(moved.unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
            assert_eq!(std::fs::read(&source).unwrap(), CONTENT);
            assert_eq!(std::fs::read(target).unwrap(), EXTERNAL);
        } else {
            moved.unwrap();
            assert!(!source.exists());
            assert_eq!(std::fs::read(target).unwrap(), CONTENT);
        }
    }
}

#[test]
#[ignore = "manual 256-file/64-MiB copy-removal load and conflict probe"]
fn copy_removal_large_batch_probe() {
    let fixture = Fixture::new("large-batch");
    seed(&fixture.0);
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let mut batch = journal.batches().unwrap().remove(0);
    journal.rollback(batch.id).unwrap();
    std::fs::remove_file(fixture.0.join("copy.bin")).unwrap();
    batch.plan.changes.clear();
    batch.copy_stamps.clear();
    let source = fixture.0.join("original.bin");
    let bytes = vec![0x5au8; 256 * 1024];
    std::fs::write(&source, &bytes).unwrap();
    for index in 0..256 {
        let target = fixture.0.join(format!("copy-{index}.bin"));
        std::fs::copy(&source, &target).unwrap();
        batch.copy_stamps.push(CopyStamp::of(&target).unwrap());
        batch.plan.changes.push(FileChange {
            path: source.clone(),
            rename_to: Some(target),
            copy: true,
            ..Default::default()
        });
    }
    batch.id = journal.record(&batch).unwrap();
    let conflict = &batch.copy_stamps[128];
    let modified = std::fs::metadata(&conflict.path)
        .unwrap()
        .modified()
        .unwrap();
    std::fs::write(&conflict.path, vec![0xa5u8; bytes.len()]).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&conflict.path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let rejected_at = Instant::now();
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).is_err());
    let reject_time = rejected_at.elapsed();
    assert!(
        batch.copy_stamps.iter().all(|stamp| stamp.path.exists()),
        "preflight conflict cannot partially delete earlier copies"
    );
    std::fs::write(&conflict.path, &bytes).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&conflict.path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let undo_at = Instant::now();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
    let undo_time = undo_at.elapsed();
    assert!(batch.copy_stamps.iter().all(|stamp| !stamp.path.exists()));
    assert!(journal.batches().unwrap().is_empty());
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    eprintln!("copy-removal 256 files / 64 MiB: midpoint conflict {reject_time:?}; complete verified Undo {undo_time:?}");
}
