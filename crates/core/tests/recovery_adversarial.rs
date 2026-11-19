//! Independent recovery probes. All paths belong to disposable fixture roots.
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use tagrex_core::journal::{AppliedBatch, BatchId, SqliteJournal, UndoJournal};
use tagrex_core::plan::{ChangePlan, Executor, FileChange};

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-recovery-adversarial-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn db(&self) -> PathBuf {
        self.0.join("journal.sqlite")
    }
    fn names(&self) -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn record(path: &Path) -> BatchId {
    let mut journal = SqliteJournal::open(path).unwrap();
    journal
        .record(&AppliedBatch {
            native_tags: false,
            id: BatchId(0),
            description: "Irreplaceable independent recovery canary".into(),
            message: None,
            applied_at: 7,
            plan: ChangePlan::default(),
            created_dirs: vec![],
            removed_dirs: vec![],
            allowed_roots: vec![],
            copy_stamps: vec![],
            applied_values: vec![],
            block_fingerprints: vec![],
            interrupted: false,
            id3v2_revisions: vec![],
        })
        .unwrap()
}

fn pcm_wav(sample: u8) -> Vec<u8> {
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
    wav.extend([sample; 320]);
    wav
}

#[test]
fn copy_undo_refuses_external_replacement_even_when_size_and_mtime_match() {
    let fixture = Fixture::new("copy-replaced");
    let source = fixture.0.join("source.wav");
    let target = fixture.0.join("copy.wav");
    std::fs::write(&source, pcm_wav(0)).unwrap();
    let mut journal = SqliteJournal::open(&fixture.db()).unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            rename_to: Some(target.clone()),
            copy: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap();
    let copied_metadata = std::fs::metadata(&target).unwrap();
    let modified = copied_metadata.modified().unwrap();
    std::fs::remove_file(&target).unwrap();
    let unrelated = pcm_wav(1);
    std::fs::write(&target, &unrelated).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(
        std::fs::metadata(&target).unwrap().len(),
        copied_metadata.len()
    );
    assert_eq!(
        std::fs::metadata(&target).unwrap().modified().unwrap(),
        modified
    );
    let outcome = Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0));
    assert!(
        outcome.is_err(),
        "size+mtime is not proof that a copied destination is still the file we created"
    );
    assert_eq!(std::fs::read(&target).unwrap(), unrelated);
    assert_eq!(std::fs::read(&source).unwrap(), pcm_wav(0));
}

#[test]
fn undo_move_never_overwrites_original_recreated_after_preflight() {
    let fixture = Fixture::new("undo-race");
    let source = fixture.0.join("source.wav");
    let target = fixture.0.join("moved.wav");
    let original = pcm_wav(0);
    let unrelated = pcm_wav(1);
    std::fs::write(&source, &original).unwrap();
    let mut journal = SqliteJournal::open(&fixture.db()).unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            rename_to: Some(target.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap();
    let mut injected = false;
    let result = Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if progress.path == target && progress.completed.is_none() && !injected {
                std::fs::write(&source, &unrelated).unwrap();
                injected = true;
            }
        },
    );
    assert!(injected);
    assert!(
        result.is_err(),
        "a destination created after Undo preflight must never be overwritten"
    );
    assert_eq!(std::fs::read(&source).unwrap(), unrelated);
    assert_eq!(std::fs::read(&target).unwrap(), original);
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[test]
fn exclusive_sqlite_lock_never_quarantines_healthy_history() {
    let fixture = Fixture::new("locked");
    let db = fixture.db();
    let id = record(&db);
    let lock = rusqlite::Connection::open(&db).unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let baseline = std::fs::read(&db).unwrap();
    let names = fixture.names();
    let start = Instant::now();
    let result = SqliteJournal::open_or_recover(&db);
    assert!(result.is_err(), "a held exclusive lock must fail safely");
    assert!(
        start.elapsed() < Duration::from_secs(15),
        "bounded SQLite busy waiting"
    );
    assert_eq!(std::fs::read(&db).unwrap(), baseline);
    assert_eq!(fixture.names(), names);
    lock.execute_batch("ROLLBACK").unwrap();
    assert!(SqliteJournal::open(&db)
        .unwrap()
        .batch(id)
        .unwrap()
        .is_some());
}

#[cfg(unix)]
#[test]
fn denied_read_permission_never_replaces_or_moves_history() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new("permissions");
    let db = fixture.db();
    let id = record(&db);
    let baseline = std::fs::read(&db).unwrap();
    let names = fixture.names();
    let permissions = std::fs::metadata(&db).unwrap().permissions();
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o0)).unwrap();
    let denied = std::fs::File::open(&db).is_err();
    let result = SqliteJournal::open_or_recover(&db);
    std::fs::set_permissions(&db, permissions).unwrap();
    // A privileged process can bypass mode bits. Do not count that as evidence
    // of exercising an actual permission error.
    if !denied {
        eprintln!("Permission-denial branch unexercised: process bypasses file mode bits");
        return;
    }
    assert!(result.is_err());
    assert_eq!(std::fs::read(&db).unwrap(), baseline);
    assert_eq!(fixture.names(), names);
    assert!(SqliteJournal::open(&db)
        .unwrap()
        .batch(id)
        .unwrap()
        .is_some());
}

#[test]
fn future_journal_schema_is_refused_without_mutation() {
    let fixture = Fixture::new("future-schema");
    let db = fixture.db();
    let id = record(&db);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .pragma_update(None, "user_version", 2_000_000_000i64)
        .unwrap();
    connection.execute_batch("CREATE TABLE future_native_identity (payload BLOB NOT NULL); INSERT INTO future_native_identity VALUES (x'CAFE');").unwrap();
    drop(connection);
    let baseline = std::fs::read(&db).unwrap();
    let names = fixture.names();
    assert!(
        SqliteJournal::open_or_recover(&db).is_err(),
        "unknown future schemas must not be interpreted by an older writer"
    );
    assert_eq!(std::fs::read(&db).unwrap(), baseline);
    assert_eq!(fixture.names(), names);
    let connection = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT description FROM batches WHERE id=?1",
                [id.0],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Irreplaceable independent recovery canary"
    );
}

#[cfg(unix)]
#[test]
fn damaged_main_database_symlink_is_refused_without_retargeting_recovery() {
    let fixture = Fixture::new("main-symlink");
    let db = fixture.db();
    let target = fixture.0.join("original-damaged.sqlite");
    std::fs::write(&target, b"Irreplaceable original damaged database").unwrap();
    std::os::unix::fs::symlink("original-damaged.sqlite", &db).unwrap();
    assert!(
        SqliteJournal::open_or_recover(&db).is_err(),
        "moving a relative symlink into another directory breaks its recovery target"
    );
    assert_eq!(
        std::fs::read_link(&db).unwrap(),
        Path::new("original-damaged.sqlite")
    );
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"Irreplaceable original damaged database"
    );
    assert_eq!(fixture.names().len(), 2);
}
