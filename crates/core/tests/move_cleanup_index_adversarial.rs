//! Independent authority-retirement checks. All paths and bytes are disposable.
#![cfg(unix)]
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::{
    cell::Cell,
    collections::BTreeMap,
    fs,
    os::unix::fs::{symlink, MetadataExt},
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};
use tagrex_core::{
    journal::{AppliedBatch, BatchId, JournalError, MoveStage, SqliteJournal, UndoJournal},
    plan::{ApplyStep, ChangePlan, Executor, FileChange},
};
// Exercise the exact private module interface without changing production
// visibility or adding a production fault hook. Its own two unit tests are
// inherited from source and are reported separately from this review's probes.
#[allow(dead_code)]
#[path = "../src/plan_paths.rs"]
mod capabilities;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-cleanup-index-fresh-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn db(&self) -> PathBuf {
        self.0.join("journal.sqlite")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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
fn move_change(path: PathBuf, target: PathBuf) -> FileChange {
    FileChange {
        path,
        rename_to: Some(target),
        ..Default::default()
    }
}
#[test]
fn failed_directory_removal_keeps_native_parent_authority() {
    let f = Fixture::new();
    let dir = f.0.join("album");
    fs::create_dir(&dir).unwrap();
    let path = dir.join("track");
    fs::write(&path, b"same object").unwrap();
    let scope = capabilities::Scope::enter(std::slice::from_ref(&f.0), [path.clone()]).unwrap();
    capabilities::directory(&dir).unwrap();
    assert!(capabilities::remove(&dir, true).is_err());
    // Keep the exact file inode through a hardlink; losing the failed rmdir's
    // directory proof would make a file-only check mistakenly accept this.
    let saved = f.0.join("saved");
    fs::rename(&dir, &saved).unwrap();
    fs::create_dir(&dir).unwrap();
    fs::hard_link(saved.join("track"), &path).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().ino(),
        fs::metadata(saved.join("track")).unwrap().ino()
    );
    assert!(capabilities::entry(&path).is_err());
    assert!(capabilities::validate().is_err());
    assert_eq!(fs::read(saved.join("track")).unwrap(), b"same object");
    drop(scope);
}
#[test]
fn successful_retirement_forgets_all_aliases_but_keeps_adjacent_authority() {
    let f = Fixture::new();
    let real = f.0.join("real");
    let other = f.0.join("real-0");
    fs::create_dir(&real).unwrap();
    fs::create_dir(&other).unwrap();
    let retired = real.join("a");
    let neighbor = real.join("a-0");
    let unicode = real.join("a\u{0301}");
    for dir in [&retired, &neighbor, &unicode] {
        fs::create_dir(dir).unwrap();
    }
    fs::write(neighbor.join("keep"), b"neighbor").unwrap();
    fs::write(unicode.join("keep"), b"unicode").unwrap();
    let alias1 = f.0.join("alias1");
    let alias2 = f.0.join("alias2");
    symlink(&real, &alias1).unwrap();
    symlink(&real, &alias2).unwrap();
    let scope = capabilities::Scope::enter(
        std::slice::from_ref(&f.0),
        [
            retired.join("future"),
            neighbor.join("keep"),
            unicode.join("keep"),
            alias1.join("a/future"),
            alias2.join("a/future"),
        ],
    )
    .unwrap();
    for path in [&retired, &alias1.join("a"), &alias2.join("a")] {
        capabilities::directory(path).unwrap();
    }
    capabilities::remove(&alias1.join("a"), true).unwrap();
    fs::create_dir(&retired).unwrap();
    fs::write(retired.join("future"), b"new authorized generation").unwrap();
    // Both formerly recorded aliases must be learnable after legitimate owned
    // retirement, rather than preserving a stale generation under either name.
    for path in [
        retired.join("future"),
        alias1.join("a/future"),
        alias2.join("a/future"),
    ] {
        capabilities::entry(&path).unwrap();
    }
    let saved = real.join("saved-a-0");
    fs::rename(&neighbor, &saved).unwrap();
    fs::hard_link(saved.join("keep"), other.join("keep")).unwrap();
    symlink(&other, &neighbor).unwrap();
    assert!(capabilities::entry(&neighbor.join("keep")).is_err());
    assert_eq!(fs::read(unicode.join("keep")).unwrap(), b"unicode");
    drop(scope);
}
#[test]
#[cfg(target_os = "linux")]
fn native_byte_component_neighbors_do_not_retire_each_others_proofs() {
    let f = Fixture::new();
    let names = [
        vec![b'a'],
        vec![b'a', 0xff],
        vec![b'a', 0xfe],
        vec![b'a', b'.'],
        vec![b'a', b'0'],
        vec![b'a', b' ', b'b'],
    ];
    let dirs: Vec<_> = names
        .into_iter()
        .map(|name| f.0.join(std::ffi::OsString::from_vec(name)))
        .collect();
    for dir in &dirs {
        fs::create_dir(dir).unwrap();
    }
    let scope = capabilities::Scope::enter(
        std::slice::from_ref(&f.0),
        dirs.iter().map(|dir| dir.join("future")),
    )
    .unwrap();
    for dir in &dirs {
        capabilities::directory(dir).unwrap();
    }
    capabilities::remove(&dirs[0], true).unwrap();
    for (i, dir) in dirs.iter().enumerate().skip(1) {
        let saved = f.0.join(format!("saved-{i}"));
        fs::rename(dir, &saved).unwrap();
        fs::create_dir(dir).unwrap();
        assert!(
            capabilities::directory(dir).is_err(),
            "forgot neighbor {dir:?}"
        );
    }
    drop(scope);
}
#[test]
fn retired_reservation_does_not_allow_later_same_inode_ancestor_redirection() {
    let f = Fixture::new();
    let first = f.0.join("first");
    let later = f.0.join("later");
    let foreign = f.0.join("foreign");
    for dir in [&first, &later, &foreign] {
        fs::create_dir(dir).unwrap();
    }
    let a = first.join("a.wav");
    let b = later.join("b.wav");
    let at = f.0.join("out/a.wav");
    let bt = f.0.join("out/b.wav");
    fs::write(&a, wav(1)).unwrap();
    fs::write(&b, wav(2)).unwrap();
    fs::hard_link(&b, foreign.join("b.wav")).unwrap();
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    let mut fired = false;
    let result = Executor::apply_with(
        &ChangePlan {
            changes: vec![
                move_change(a.clone(), at.clone()),
                move_change(b.clone(), bt.clone()),
            ],
            ..Default::default()
        },
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if !fired && event.path == a && event.completed == Some(ApplyStep::Transfer) {
                fired = true;
                fs::rename(&later, f.0.join("saved-later")).unwrap();
                symlink(&foreign, &later).unwrap();
            }
        },
        &AtomicBool::new(false),
    );
    assert!(fired);
    assert!(result.is_err());
    assert!(!bt.exists());
    assert_eq!(fs::read(f.0.join("saved-later/b.wav")).unwrap(), wav(2));
    assert_eq!(fs::read(foreign.join("b.wav")).unwrap(), wav(2));
    let locations = [&a, &at];
    assert_eq!(locations.iter().filter(|p| p.exists()).count(), 1);
    for path in locations {
        if path.exists() {
            assert_eq!(fs::read(path).unwrap(), wav(1));
        }
    }
}
struct Clutter {
    inner: SqliteJournal,
    blocker: Option<PathBuf>,
}
impl UndoJournal for Clutter {
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
    fn retain_interrupted(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
        self.inner.retain_interrupted(id, b)
    }
    fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
        self.inner.move_stages(id)
    }
    fn prepare_move_stage(&mut self, id: BatchId, s: &MoveStage) -> Result<(), JournalError> {
        self.inner.prepare_move_stage(id, s)?;
        if self.blocker.is_none() {
            let path = s.path.parent().unwrap().join("foreign-sibling");
            fs::write(&path, b"preserve foreign sibling").unwrap();
            self.blocker = Some(path);
        }
        Ok(())
    }
}
#[test]
fn failed_private_cleanup_preserves_foreign_sibling_and_reopened_retry() {
    let f = Fixture::new();
    let source = f.0.join("track.wav");
    let target = f.0.join("out/track.wav");
    fs::write(&source, wav(3)).unwrap();
    let mut journal = Clutter {
        inner: SqliteJournal::open(&f.db()).unwrap(),
        blocker: None,
    };
    let result = Executor::apply(
        &ChangePlan {
            changes: vec![move_change(source.clone(), target.clone())],
            ..Default::default()
        },
        &mut journal,
        std::slice::from_ref(&f.0),
    );
    assert!(result.is_err());
    let blocker = journal.blocker.take().unwrap();
    assert_eq!(fs::read(&blocker).unwrap(), b"preserve foreign sibling");
    let batch = journal.inner.batches().unwrap().remove(0);
    drop(journal);
    let mut reopened = SqliteJournal::open(&f.db()).unwrap();
    let snapshot = || {
        [&source, &target, &blocker]
            .iter()
            .map(|path| match fs::symlink_metadata(path) {
                Ok(metadata) => Some((metadata.dev(), metadata.ino(), fs::read(path).unwrap())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => panic!("fixture path inspection: {error}"),
            })
            .collect::<Vec<_>>()
    };
    let before_paths = snapshot();
    let before_stages = reopened.move_stages(batch.id).unwrap();
    let before_batch = format!("{:?}", reopened.batch(batch.id).unwrap());
    assert!(Executor::undo(&mut reopened, batch.id, std::slice::from_ref(&f.0)).is_err());
    assert_eq!(snapshot(), before_paths);
    assert_eq!(reopened.move_stages(batch.id).unwrap(), before_stages);
    assert_eq!(
        format!("{:?}", reopened.batch(batch.id).unwrap()),
        before_batch
    );
    assert_eq!(fs::read(&blocker).unwrap(), b"preserve foreign sibling");
    fs::remove_file(&blocker).unwrap();
    Executor::undo(&mut reopened, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(fs::read(&source).unwrap(), wav(3));
    assert!(!target.exists());
    assert!(reopened.batches().unwrap().is_empty());
}
#[test]
fn cancellation_after_many_retirements_restores_prefix_names_and_history() {
    let f = Fixture::new();
    let mut changes = Vec::new();
    let names = ["a", "a-0", "a.foo", "a0", "aa", "álbum", "a b", "a\\b"];
    for i in 0..64 {
        let source = f.0.join(names[i % names.len()]).join(format!("{i}.wav"));
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, wav(i as u8)).unwrap();
        changes.push(move_change(source, f.0.join(format!("out/{i}.wav"))));
    }
    let cancel = AtomicBool::new(false);
    let mut completed = 0;
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    let result = Executor::apply_with(
        &ChangePlan {
            changes: changes.clone(),
            prune_empty_dirs: true,
            ..Default::default()
        },
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed == Some(ApplyStep::Transfer) {
                completed += 1;
                if completed == 31 {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        },
        &cancel,
    );
    assert!(result.is_err());
    assert_eq!(completed, 31);
    for (i, change) in changes.iter().enumerate() {
        assert_eq!(fs::read(&change.path).unwrap(), wav(i as u8));
        assert!(!change.rename_to.as_ref().unwrap().exists());
    }
    assert!(journal.batches().unwrap().is_empty());
}
#[test]
#[ignore = "Manual independent public-Executor count/preservation/profile; no speed threshold"]
fn paired_prefix_layout_reopened_undo_preserves_all_work_and_aliases() {
    let f = Fixture::new();
    let library = f.0.join("library");
    fs::create_dir(&library).unwrap();
    let names = ["a", "a-0", "a.foo", "a0", "aa", "álbum", "a b", "a\\b"];
    let mut changes = Vec::new();
    let mut originals = BTreeMap::new();
    for i in 0..512 {
        let path = library
            .join(names[i % names.len()])
            .join(format!("disc-{i:04}/track.wav"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = wav(i as u8);
        fs::write(&path, &bytes).unwrap();
        originals.insert(path.clone(), bytes);
        changes.push(move_change(path, library.join(format!("out/{i}.wav"))));
    }
    let hardlink = f.0.join("outside-hardlink.wav");
    fs::hard_link(&changes[0].path, &hardlink).unwrap();
    for iteration in 0..2 {
        let journal = SqliteJournal::open(&f.db()).unwrap();
        struct Counted {
            inner: SqliteJournal,
            count: Cell<usize>,
        }
        impl UndoJournal for Counted {
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
            fn retain_interrupted(
                &mut self,
                id: BatchId,
                b: &AppliedBatch,
            ) -> Result<(), JournalError> {
                self.inner.retain_interrupted(id, b)
            }
            fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
                self.inner.move_stages(id)
            }
            fn prepare_move_stage(
                &mut self,
                id: BatchId,
                s: &MoveStage,
            ) -> Result<(), JournalError> {
                self.count.set(self.count.get() + 1);
                self.inner.prepare_move_stage(id, s)
            }
        }
        let mut counted = Counted {
            inner: journal,
            count: Cell::new(0),
        };
        let start = Instant::now();
        let batch = Executor::apply(
            &ChangePlan {
                changes: changes.clone(),
                prune_empty_dirs: true,
                ..Default::default()
            },
            &mut counted,
            std::slice::from_ref(&library),
        )
        .unwrap();
        let apply = start.elapsed();
        assert_eq!(counted.count.get(), 512);
        drop(counted);
        for change in &changes {
            assert!(!change.path.exists());
            assert_eq!(
                fs::read(change.rename_to.as_ref().unwrap()).unwrap(),
                originals[&change.path]
            );
        }
        // Same-object external data edits through an independent name survive
        // Undo. This checks that fast cleanup did not weaken native ownership.
        let edit = wav(0xf1);
        fs::write(&hardlink, &edit).unwrap();
        originals.insert(changes[0].path.clone(), edit.clone());
        let mut counted = Counted {
            inner: SqliteJournal::open(&f.db()).unwrap(),
            count: Cell::new(0),
        };
        let start = Instant::now();
        Executor::undo(&mut counted, batch.id, std::slice::from_ref(&library)).unwrap();
        let undo = start.elapsed();
        assert_eq!(counted.count.get(), 512);
        for change in &changes {
            assert_eq!(fs::read(&change.path).unwrap(), originals[&change.path]);
            assert!(!change.rename_to.as_ref().unwrap().exists());
        }
        assert_eq!(fs::read(&hardlink).unwrap(), edit);
        assert!(counted.batches().unwrap().is_empty());
        assert!(!library.join("out").exists());
        eprintln!("CLEANUP_INDEX_INDEPENDENT iteration={iteration} rows=512 apply={apply:?} undo={undo:?} apply_sql_intents=512 undo_sql_intents=512");
    }
}
