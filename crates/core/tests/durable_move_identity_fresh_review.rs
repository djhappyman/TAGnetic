//! Independent durable-transfer probes. All paths and bytes are disposable.
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    journal::{SqliteJournal, UndoJournal},
    plan::{ChangePlan, Executor, FileChange},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-durable-move-fresh-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("journal.sqlite")
    }
    fn journal(&self) -> SqliteJournal {
        SqliteJournal::open(&self.database()).unwrap()
    }
    fn write(&self, name: &str, value: u8) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, wav(value)).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wav(value: u8) -> Vec<u8> {
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
    bytes.extend([value; 320]);
    bytes
}
fn move_plan(source: &Path, target: &Path) -> ChangePlan {
    ChangePlan {
        changes: vec![FileChange {
            path: source.to_owned(),
            rename_to: Some(target.to_owned()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn restart_undo_preserves_same_object_content_changes_and_retries_replacement_refusal() {
    let f = Fixture::new("same-object");
    let source = f.write("音源.wav", 0);
    let target = f.0.join("moved.wav");
    let mut journal = f.journal();
    let batch = Executor::apply(
        &move_plan(&source, &target),
        &mut journal,
        std::slice::from_ref(&f.0),
    )
    .unwrap();
    drop(journal);
    let before_time = std::fs::metadata(&target).unwrap().modified().unwrap();
    let saved = f.0.join("saved.wav");
    std::fs::rename(&target, &saved).unwrap();
    // Same bytes and restored timestamp do not make a replacement our object.
    std::fs::write(&target, wav(0)).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(before_time))
        .unwrap();
    let mut journal = f.journal();
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).is_err());
    assert!(journal.batch(batch.id).unwrap().is_some());
    assert!(!source.exists());
    assert_eq!(std::fs::read(&target).unwrap(), wav(0));
    std::fs::remove_file(&target).unwrap();
    std::fs::rename(&saved, &target).unwrap();
    // In-place user changes retain the moved object's identity and must survive.
    std::fs::write(&target, wav(7)).unwrap();
    drop(journal);
    let mut journal = f.journal();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), wav(7));
    assert!(!target.exists());
    assert!(journal.batch(batch.id).unwrap().is_none());
}

#[test]
fn malformed_or_future_native_identity_never_authorizes_a_transfer_or_discards_history() {
    let f = Fixture::new("identity-schema");
    let source = f.write("source.wav", 0);
    let target = f.0.join("target.wav");
    let mut journal = f.journal();
    let batch = Executor::apply(
        &move_plan(&source, &target),
        &mut journal,
        std::slice::from_ref(&f.0),
    )
    .unwrap();
    drop(journal);
    let sql = rusqlite::Connection::open(f.database()).unwrap();
    let original: Vec<u8> = sql
        .query_row(
            "SELECT identity FROM copy_stamps WHERE batch_id=?1",
            [batch.id.0],
            |r| r.get(0),
        )
        .unwrap();
    for bad in [
        vec![],
        b"TRX-U9".to_vec(),
        original[..original.len() - 1].to_vec(),
        vec![0xff; 128],
    ] {
        sql.execute(
            "UPDATE copy_stamps SET identity=?1 WHERE batch_id=?2",
            rusqlite::params![bad, batch.id.0],
        )
        .unwrap();
        let mut journal = f.journal();
        assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).is_err());
        assert!(journal.batch(batch.id).unwrap().is_some());
        assert!(!source.exists());
        assert_eq!(std::fs::read(&target).unwrap(), wav(0));
    }
    sql.execute(
        "UPDATE copy_stamps SET identity=?1,moved=0,sha256=zeroblob(32) WHERE batch_id=?2",
        rusqlite::params![original, batch.id.0],
    )
    .unwrap();
    let mut journal = f.journal();
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).is_err());
    assert!(journal.batch(batch.id).unwrap().is_some());
    drop(journal);
    sql.execute(
        "UPDATE copy_stamps SET moved=1,sha256=NULL WHERE batch_id=?1",
        [batch.id.0],
    )
    .unwrap();
    drop(sql);
    let mut journal = f.journal();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(std::fs::read(source).unwrap(), wav(0));
}

#[test]
fn process_exit_after_first_move_uses_initial_record_identity_to_reverse_only_reached_steps() {
    const MODE: &str = "TAGREX_DURABLE_MOVE_FRESH_CHILD";
    if let Ok(directory) = std::env::var(MODE) {
        let directory = PathBuf::from(directory);
        let first = directory.join("first.wav");
        let second = directory.join("second.wav");
        let plan = ChangePlan {
            changes: vec![
                FileChange {
                    path: first.clone(),
                    rename_to: Some(directory.join("first-moved.wav")),
                    ..Default::default()
                },
                FileChange {
                    path: second,
                    rename_to: Some(directory.join("second-moved.wav")),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut journal = SqliteJournal::open(&directory.join("journal.sqlite")).unwrap();
        let result = Executor::apply_with(
            &plan,
            &mut journal,
            std::slice::from_ref(&directory),
            &mut |p| {
                if p.completed.is_some() && p.path == first {
                    std::process::exit(73);
                }
            },
            &std::sync::atomic::AtomicBool::new(false),
        );
        panic!("expected real process exit after first move, got {result:?}");
    }
    let f = Fixture::new("process-exit");
    let first = f.write("first.wav", 0);
    let second = f.write("second.wav", 1);
    let result=std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","process_exit_after_first_move_uses_initial_record_identity_to_reverse_only_reached_steps","--nocapture"])
        .env(MODE,&f.0).output().unwrap();
    assert_eq!(
        result.status.code(),
        Some(73),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!first.exists());
    assert!(f.0.join("first-moved.wav").exists());
    assert!(second.exists());
    let mut journal = f.journal();
    let batches = journal.batches().unwrap();
    assert_eq!(batches.len(), 1);
    assert!(batches[0].interrupted);
    assert_eq!(batches[0].copy_stamps.len(), 2);
    Executor::undo(&mut journal, batches[0].id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(std::fs::read(first).unwrap(), wav(0));
    assert_eq!(std::fs::read(second).unwrap(), wav(1));
    assert!(!f.0.join("first-moved.wav").exists());
    assert!(!f.0.join("second-moved.wav").exists());
    assert!(journal.batches().unwrap().is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn stamped_sidecar_link_can_be_undone_after_its_referent_disappears() {
    let f = Fixture::new("dangling-link");
    let source = f.write("source.wav", 0);
    let target = f.0.join("moved.wav");
    let referent = f.0.join("text.txt");
    std::fs::write(&referent, b"keep original data").unwrap();
    let link = f.0.join("source.cue");
    let moved = f.0.join("moved.cue");
    std::os::unix::fs::symlink("text.txt", &link).unwrap();
    let mut plan = move_plan(&source, &target);
    plan.changes[0]
        .sidecar_renames
        .push((link.clone(), moved.clone()));
    let mut journal = f.journal();
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    drop(journal);
    // This changes the referent's lifetime, not the stamped link entry.
    let saved = f.0.join("saved-text.txt");
    std::fs::rename(&referent, &saved).unwrap();
    assert!(std::fs::symlink_metadata(&moved)
        .unwrap()
        .file_type()
        .is_symlink());
    let mut journal = f.journal();
    let outcome = Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0));
    assert!(
        outcome.is_ok(),
        "Undo should move the owned link entry without resolving its absent referent: {outcome:?}"
    );
    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("text.txt"));
    assert!(std::fs::symlink_metadata(&moved).is_err());
    assert_eq!(std::fs::read(saved).unwrap(), b"keep original data");
    assert!(journal.batch(batch.id).unwrap().is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn already_restored_dangling_link_is_recognized_during_restart_retry() {
    let f = Fixture::new("link-retry");
    let source = f.write("source.wav", 0);
    let target = f.0.join("moved.wav");
    let referent = f.0.join("text.txt");
    std::fs::write(&referent, b"owned fixture text").unwrap();
    let link = f.0.join("source.cue");
    let moved = f.0.join("moved.cue");
    std::os::unix::fs::symlink("text.txt", &link).unwrap();
    let mut plan = move_plan(&source, &target);
    plan.changes[0]
        .sidecar_renames
        .push((link.clone(), moved.clone()));
    let mut journal = f.journal();
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    drop(journal);
    // Represents successful transfers from an Undo interrupted before journal
    // removal. Both original object identities are restored, not replacements.
    std::fs::rename(&target, &source).unwrap();
    std::fs::rename(&moved, &link).unwrap();
    let saved = f.0.join("saved-text.txt");
    std::fs::rename(&referent, &saved).unwrap();
    let mut journal = f.journal();
    let result = Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0));
    assert!(
        result.is_ok(),
        "restored stamped link entry must count as restored independent of referent: {result:?}"
    );
    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("text.txt"));
    assert!(std::fs::symlink_metadata(&moved).is_err());
    assert_eq!(std::fs::read(saved).unwrap(), b"owned fixture text");
    assert!(journal.batch(batch.id).unwrap().is_none());
}

#[test]
fn corrupt_move_proof_refuses_a_mixed_batch_before_deleting_any_copy() {
    let f = Fixture::new("mixed-proof");
    let copied_source = f.write("copy-source.wav", 0);
    let moved_source = f.write("move-source.wav", 1);
    let copied_target = f.0.join("copy-output.wav");
    let moved_target = f.0.join("move-output.wav");
    let plan = ChangePlan {
        changes: vec![
            FileChange {
                path: copied_source.clone(),
                rename_to: Some(copied_target.clone()),
                copy: true,
                ..Default::default()
            },
            FileChange {
                path: moved_source.clone(),
                rename_to: Some(moved_target.clone()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut journal = f.journal();
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    drop(journal);
    let sql = rusqlite::Connection::open(f.database()).unwrap();
    assert_eq!(
        sql.execute(
            "UPDATE copy_stamps SET identity=?1 WHERE batch_id=?2 AND moved=1",
            rusqlite::params![b"TRX-future-native-format".as_slice(), batch.id.0]
        )
        .unwrap(),
        1
    );
    drop(sql);
    let mut journal = f.journal();
    assert!(Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).is_err());
    assert_eq!(std::fs::read(copied_source).unwrap(), wav(0));
    assert_eq!(std::fs::read(copied_target).unwrap(), wav(0));
    assert_eq!(std::fs::read(moved_target).unwrap(), wav(1));
    assert!(!moved_source.exists());
    assert!(journal.batch(batch.id).unwrap().is_some());
}
