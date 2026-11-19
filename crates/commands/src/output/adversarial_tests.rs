//! Real publisher interleavings using only disposable text files.
use super::{
    ownership::{self, Event},
    write_batch, OutputError,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_OUTPUT_CASE").as_deref() == Ok(name) {
        return false;
    }
    let full = format!("output::adversarial_tests::{name}");
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &full, "--include-ignored", "--nocapture"])
        .env("TAGREX_OUTPUT_CASE", name)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    true
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-output-ownership-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        ownership::set_observer(None);
        super::PUBLICATION_HOOK.with(|slot| slot.replace(None));
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn recovery(error: &OutputError) -> Vec<PathBuf> {
    assert!(
        matches!(error, OutputError::RecoveryRequired { .. }),
        "{error:?}"
    );
    let paths = error.recovery_paths().to_vec();
    assert!(
        paths.iter().any(|path| path.is_dir()),
        "must retain an inspectable artifact directory: {paths:?}"
    );
    paths
}
fn files_below(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_owned()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                dirs.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
    }
    files
}
fn exists_bytes(root: &Path, bytes: &[u8]) -> bool {
    files_below(root)
        .iter()
        .any(|path| fs::read(path).ok().as_deref() == Some(bytes))
}

#[test]
fn raced_future_target_is_not_overwritten_even_when_overwrite_was_requested() {
    if isolated("raced_future_target_is_not_overwritten_even_when_overwrite_was_requested") {
        return;
    }
    let f = Fixture::new();
    let first = f.path("first.txt");
    let second = f.path("second.txt");
    fs::write(&first, b"old first").unwrap();
    fs::write(&second, b"old second").unwrap();
    let target = second.clone();
    ownership::set_observer(Some(Box::new(move |event, path| {
        if event == Event::BeforePublish && path == target {
            fs::write(&target, b"new external second").unwrap();
        }
        Ok(())
    })));
    let outcome = write_batch(
        &f.0,
        &[
            (first.clone(), b"new first".to_vec()),
            (second.clone(), b"new second".to_vec()),
        ],
        true,
    );
    assert!(outcome.is_err());
    assert_eq!(fs::read(&first).unwrap(), b"old first");
    assert_eq!(fs::read(&second).unwrap(), b"new external second");
}
#[test]
fn published_same_inode_content_change_is_preserved_during_failed_batch_rollback() {
    if isolated("published_same_inode_content_change_is_preserved_during_failed_batch_rollback") {
        return;
    }
    let f = Fixture::new();
    let first = f.path("first.txt");
    let second = f.path("second.txt");
    fs::write(&first, b"old first").unwrap();
    let (a, b) = (first.clone(), second.clone());
    super::PUBLICATION_HOOK.with(|slot| {
        slot.replace(Some(Box::new(move |path| {
            if path == a {
                let before = fs::metadata(&a).unwrap();
                fs::write(&a, b"external!").unwrap();
                fs::OpenOptions::new()
                    .write(true)
                    .open(&a)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(before.modified().unwrap()))
                    .unwrap();
                fs::write(&b, b"raced").unwrap();
            }
        })))
    });
    let error = write_batch(
        &f.0,
        &[
            (first.clone(), b"new first".to_vec()),
            (second.clone(), b"new second".to_vec()),
        ],
        true,
    )
    .unwrap_err();
    recovery(&error);
    assert_eq!(fs::read(first).unwrap(), b"external!");
    assert_eq!(fs::read(second).unwrap(), b"raced");
    assert!(exists_bytes(&f.0, b"old first"));
}
#[test]
fn substituted_backup_is_neither_deleted_nor_restored_as_the_original() {
    if isolated("substituted_backup_is_neither_deleted_nor_restored_as_the_original") {
        return;
    }
    let f = Fixture::new();
    let first = f.path("first.txt");
    fs::write(&first, b"saved original").unwrap();
    ownership::set_observer(Some(Box::new(move |event, path| {
        if event == Event::BackupCaptured {
            fs::rename(path, path.with_extension("saved")).unwrap();
            fs::write(path, b"foreign backup replacement").unwrap();
        }
        Ok(())
    })));
    let error =
        write_batch(&f.0, &[(first.clone(), b"proposed output".to_vec())], true).unwrap_err();
    recovery(&error);
    assert!(exists_bytes(&f.0, b"saved original"));
    assert!(exists_bytes(&f.0, b"foreign backup replacement"));
    assert!(!first.exists());
    let manifests: Vec<_> = files_below(&f.0)
        .into_iter()
        .filter(|p| p.file_name() == Some(std::ffi::OsStr::new("recovery.json")))
        .collect();
    assert!(!manifests.is_empty());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifests[0]).unwrap()).unwrap();
    use sha2::Digest;
    assert_eq!(
        manifest["outputs"][0]["before"]["sha256"],
        format!("{:x}", sha2::Sha256::digest(b"saved original"))
    );
}
#[test]
fn staged_candidate_replacement_is_kept_and_no_public_output_is_written() {
    if isolated("staged_candidate_replacement_is_kept_and_no_public_output_is_written") {
        return;
    }
    let f = Fixture::new();
    let target = f.path("result.txt");
    fs::write(&target, b"old").unwrap();
    ownership::set_observer(Some(Box::new(move |event, path| {
        if event == Event::Staged {
            fs::rename(path, path.with_extension("saved")).unwrap();
            fs::write(path, b"foreign staged replacement").unwrap();
        }
        Ok(())
    })));
    let error = write_batch(&f.0, &[(target.clone(), b"new".to_vec())], true).unwrap_err();
    recovery(&error);
    assert_eq!(fs::read(target).unwrap(), b"old");
    assert!(exists_bytes(&f.0, b"foreign staged replacement"));
    assert!(exists_bytes(&f.0, b"new"));
}
#[test]
fn source_slot_recreated_after_publication_is_not_owned_even_with_the_same_inode() {
    if isolated("source_slot_recreated_after_publication_is_not_owned_even_with_the_same_inode") {
        return;
    }
    let f = Fixture::new();
    let target = f.path("result.txt");
    let staged = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let holder = staged.clone();
    let public = target.clone();
    ownership::set_observer(Some(Box::new(move |event, path| {
        if event == Event::CandidateCaptured {
            *holder.lock().unwrap() = Some(path.to_path_buf());
        }
        if event == Event::BeforeCleanup {
            let name = holder.lock().unwrap().clone().unwrap();
            fs::hard_link(&public, name).unwrap();
        }
        Ok(())
    })));
    let error = write_batch(&f.0, &[(target.clone(), b"new output".to_vec())], false).unwrap_err();
    recovery(&error);
    assert_eq!(fs::read(target).unwrap(), b"new output");
    let name = staged.lock().unwrap().clone().unwrap();
    assert_eq!(fs::read(name).unwrap(), b"new output");
}
#[cfg(unix)]
#[test]
fn foreign_regular_file_at_private_directory_slot_is_restored_after_capture_refusal() {
    if isolated("foreign_regular_file_at_private_directory_slot_is_restored_after_capture_refusal")
    {
        return;
    }
    let f = Fixture::new();
    let target = f.path("result.txt");
    let foreign = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let holder = foreign.clone();
    ownership::set_observer(Some(Box::new(move |event, path| {
        if event == Event::BeforePrivateRemove {
            let moved = path.with_extension("moved");
            fs::rename(path, &moved).unwrap();
            fs::write(path, b"foreign directory-slot file").unwrap();
            *holder.lock().unwrap() = Some(path.to_path_buf());
        }
        Ok(())
    })));
    let error = write_batch(&f.0, &[(target.clone(), b"new output".to_vec())], false).unwrap_err();
    recovery(&error);
    assert_eq!(
        fs::read(foreign.lock().unwrap().as_ref().unwrap()).unwrap(),
        b"foreign directory-slot file"
    );
    assert_eq!(fs::read(target).unwrap(), b"new output");
}
#[cfg(unix)]
#[test]
fn moved_parent_and_replacement_symlink_cannot_redirect_batch_publication() {
    if isolated("moved_parent_and_replacement_symlink_cannot_redirect_batch_publication") {
        return;
    }
    let f = Fixture::new();
    let album = f.path("album");
    let outside = f.path("other");
    let moved = f.path("moved-album");
    fs::create_dir(&album).unwrap();
    fs::create_dir(&outside).unwrap();
    let target = album.join("result.txt");
    fs::write(&target, b"old output").unwrap();
    let (a, b, c) = (album.clone(), outside.clone(), moved.clone());
    ownership::set_observer(Some(Box::new(move |event, _| {
        if event == Event::Staged {
            fs::rename(&a, &c).unwrap();
            std::os::unix::fs::symlink(&b, &a).unwrap();
        }
        Ok(())
    })));
    assert!(write_batch(&f.0, &[(target, b"new output".to_vec())], true).is_err());
    assert_eq!(fs::read(moved.join("result.txt")).unwrap(), b"old output");
    assert!(!outside.join("result.txt").exists());
}
#[test]
fn hardlinked_aliases_are_refused_before_staging_and_an_untargeted_link_survives_overwrite() {
    if isolated(
        "hardlinked_aliases_are_refused_before_staging_and_an_untargeted_link_survives_overwrite",
    ) {
        return;
    }
    let f = Fixture::new();
    let first = f.path("first.txt");
    let alias = f.path("alias.txt");
    fs::write(&first, b"old").unwrap();
    fs::hard_link(&first, &alias).unwrap();
    assert!(write_batch(
        &f.0,
        &[
            (first.clone(), b"a".to_vec()),
            (alias.clone(), b"b".to_vec())
        ],
        true
    )
    .is_err());
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 2);
    assert_eq!(fs::read(&first).unwrap(), b"old");
    write_batch(&f.0, &[(first.clone(), b"new".to_vec())], true).unwrap();
    assert_eq!(fs::read(first).unwrap(), b"new");
    assert_eq!(fs::read(alias).unwrap(), b"old");
}
#[test]
fn resource_limits_refuse_whole_batches_before_any_staging_or_publication() {
    if isolated("resource_limits_refuse_whole_batches_before_any_staging_or_publication") {
        return;
    }
    let f = Fixture::new();
    let outputs: Vec<_> = (0..129)
        .map(|i| (f.path(&format!("{i}.txt")), Vec::new()))
        .collect();
    assert!(matches!(
        write_batch(&f.0, &outputs, false),
        Err(OutputError::Resource(
            tagrex_core::resources::AdmissionError::TooLarge
        ))
    ));
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    let file = fs::File::create(f.path("huge.txt")).unwrap();
    file.set_len(65 * 1024 * 1024).unwrap();
    assert!(matches!(
        write_batch(&f.0, &[(f.path("huge.txt"), b"new".to_vec())], true),
        Err(OutputError::Resource(
            tagrex_core::resources::AdmissionError::TooLarge
        ))
    ));
    assert_eq!(
        fs::metadata(f.path("huge.txt")).unwrap().len(),
        65 * 1024 * 1024
    );
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
}
#[cfg(unix)]
#[test]
fn real_kernel_short_write_and_file_limit_failure_clean_only_the_owned_prefix() {
    const NAME:&str="output::adversarial_tests::real_kernel_short_write_and_file_limit_failure_clean_only_the_owned_prefix";
    if let Some(root) = std::env::var_os("TAGREX_OUTPUT_LIMIT_CHILD") {
        let root = PathBuf::from(root);
        let target = root.join("old.txt");
        let error = write_batch(&root, &[(target.clone(), vec![b'x'; 8192])], true).unwrap_err();
        assert!(
            !matches!(error, OutputError::RecoveryRequired { .. }),
            "owned short-write prefix should clean safely: {error:?}"
        );
        assert_eq!(fs::read(target).unwrap(), b"original");
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
        return;
    }
    let f = Fixture::new();
    fs::write(f.path("old.txt"), b"original").unwrap();
    let output = std::process::Command::new("sh")
        .args([
            "-c",
            "trap '' XFSZ; ulimit -f 1; exec \"$@\"",
            "tagrex-output-limit",
        ])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture"])
        .env("TAGREX_OUTPUT_LIMIT_CHILD", &f.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn process_exit_after_publication_retains_original_bytes_without_claiming_restart_recovery() {
    const NAME:&str="output::adversarial_tests::process_exit_after_publication_retains_original_bytes_without_claiming_restart_recovery";
    if let Some(root) = std::env::var_os("TAGREX_OUTPUT_CRASH_CHILD") {
        let root = PathBuf::from(root);
        super::PUBLICATION_HOOK
            .with(|slot| slot.replace(Some(Box::new(|_| std::process::exit(91)))));
        let _ = write_batch(
            &root,
            &[
                (root.join("first.txt"), b"new first".to_vec()),
                (root.join("second.txt"), b"new second".to_vec()),
            ],
            true,
        );
        panic!("crash boundary not reached");
    }
    let f = Fixture::new();
    fs::write(f.path("first.txt"), b"old first").unwrap();
    fs::write(f.path("second.txt"), b"old second").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture"])
        .env("TAGREX_OUTPUT_CRASH_CHILD", &f.0)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(91));
    assert_eq!(fs::read(f.path("first.txt")).unwrap(), b"new first");
    assert_eq!(fs::read(f.path("second.txt")).unwrap(), b"old second");
    assert!(exists_bytes(&f.0, b"old first"));
}

#[test]
fn eight_live_publishers_fill_admission_and_ninth_refuses_before_files_are_created() {
    if isolated("eight_live_publishers_fill_admission_and_ninth_refuses_before_files_are_created") {
        return;
    }
    let f = Fixture::new();
    let (sender, receiver) = std::sync::mpsc::channel();
    let release = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let mut workers = Vec::new();
    for n in 0..8 {
        let root = f.0.clone();
        let sender = sender.clone();
        let release = release.clone();
        workers.push(std::thread::spawn(move || {
            ownership::set_observer(Some(Box::new(move |event, _| {
                if event == Event::Staged {
                    sender.send(()).unwrap();
                    let (lock, changed) = &*release;
                    let mut done = lock.lock().unwrap();
                    while !*done {
                        done = changed.wait(done).unwrap();
                    }
                }
                Ok(())
            })));
            write_batch(
                &root,
                &[(root.join(format!("{n}.txt")), vec![n as u8; 100])],
                false,
            )
        }));
    }
    for _ in 0..8 {
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }
    let ninth = f.path("ninth.txt");
    assert!(matches!(
        write_batch(&f.0, &[(ninth.clone(), b"ninth".to_vec())], false),
        Err(OutputError::Resource(
            tagrex_core::resources::AdmissionError::Busy
        ))
    ));
    assert!(!ninth.exists());
    let (lock, changed) = &*release;
    *lock.lock().unwrap() = true;
    changed.notify_all();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 8);
    assert_eq!(ownership::test_descriptors(), 0);
    assert_eq!(
        tagrex_core::resources::Budget::process().usage().0,
        tagrex_core::resources::Charge::default()
    );
}

#[test]
fn backup_replaced_at_restoration_boundary_is_retained_without_becoming_public() {
    if isolated("backup_replaced_at_restoration_boundary_is_retained_without_becoming_public") {
        return;
    }
    let f = Fixture::new();
    let first = f.path("first.txt");
    let second = f.path("second.txt");
    fs::write(&first, b"saved original").unwrap();
    let raced = second.clone();
    super::PUBLICATION_HOOK.with(|slot| {
        slot.replace(Some(Box::new(move |_| {
            fs::write(&raced, b"raced second").unwrap();
        })))
    });
    ownership::set_observer(Some(Box::new(move |event, path| {
        if event == Event::BeforeRestore {
            fs::rename(path, path.with_extension("saved")).unwrap();
            fs::write(path, b"foreign restoration replacement").unwrap();
        }
        Ok(())
    })));
    let error = write_batch(
        &f.0,
        &[
            (first.clone(), b"new first".to_vec()),
            (second, b"new second".to_vec()),
        ],
        true,
    )
    .unwrap_err();
    recovery(&error);
    assert!(
        !first.exists(),
        "foreign replacement must never become public"
    );
    assert!(exists_bytes(&f.0, b"saved original"));
    assert!(exists_bytes(&f.0, b"foreign restoration replacement"));
}

#[test]
fn committed_cleanup_preserves_substituted_backup_and_keeps_every_new_output() {
    if isolated("committed_cleanup_preserves_substituted_backup_and_keeps_every_new_output") {
        return;
    }
    let f = Fixture::new();
    let first = f.path("first.txt");
    let second = f.path("second.txt");
    fs::write(&first, b"old first").unwrap();
    fs::write(&second, b"old second").unwrap();
    let backup = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let holder = backup.clone();
    ownership::set_observer(Some(Box::new(move |event, path| {
        if event == Event::BackupCaptured {
            *holder.lock().unwrap() = Some(path.to_path_buf());
        }
        if event == Event::BeforeCleanup {
            let path = holder.lock().unwrap().clone().unwrap();
            fs::rename(&path, path.with_extension("saved")).unwrap();
            fs::write(path, b"foreign cleanup replacement").unwrap();
        }
        Ok(())
    })));
    let error = write_batch(
        &f.0,
        &[
            (first.clone(), b"new first".to_vec()),
            (second.clone(), b"new second".to_vec()),
        ],
        true,
    )
    .unwrap_err();
    recovery(&error);
    assert_eq!(fs::read(first).unwrap(), b"new first");
    assert_eq!(fs::read(second).unwrap(), b"new second");
    assert!(exists_bytes(&f.0, b"old second"));
    assert!(exists_bytes(&f.0, b"foreign cleanup replacement"));
}

#[test]
fn injected_sync_failure_releases_admission_and_preserves_the_original() {
    if isolated("injected_sync_failure_releases_admission_and_preserves_the_original") {
        return;
    }
    let f = Fixture::new();
    let target = f.path("old.txt");
    fs::write(&target, b"old").unwrap();
    ownership::set_observer(Some(Box::new(|event, _| {
        if event == Event::BeforeSync {
            return Err(std::io::Error::other("injected sync boundary failure"));
        }
        Ok(())
    })));
    let error = write_batch(&f.0, &[(target.clone(), b"new".to_vec())], true).unwrap_err();
    assert!(
        !matches!(error, OutputError::RecoveryRequired { .. }),
        "{error:?}"
    );
    assert_eq!(fs::read(target).unwrap(), b"old");
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
    assert_eq!(ownership::test_descriptors(), 0);
    assert_eq!(
        tagrex_core::resources::Budget::process().usage().0,
        tagrex_core::resources::Charge::default()
    );
}

#[test]
fn recovery_error_retains_only_its_memory_lease_until_the_last_error_owner_drops() {
    if isolated("recovery_error_retains_only_its_memory_lease_until_the_last_error_owner_drops") {
        return;
    }
    let f = Fixture::new();
    let target = f.path("old.txt");
    fs::write(&target, b"old").unwrap();
    ownership::set_observer(Some(Box::new(|event, _| {
        if event == Event::BeforeCleanup {
            return Err(std::io::Error::other("retain evidence"));
        }
        Ok(())
    })));
    let error = write_batch(&f.0, &[(target, b"new".to_vec())], true).unwrap_err();
    recovery(&error);
    let charge = tagrex_core::resources::Budget::process().usage().0;
    assert!(charge.retained > 0);
    assert_eq!(charge.jobs, 0);
    assert_eq!(charge.transient, 0);
    assert_eq!(ownership::test_descriptors(), 0);
    drop(error);
    assert_eq!(
        tagrex_core::resources::Budget::process().usage().0,
        tagrex_core::resources::Charge::default()
    );
}

#[test]
fn overlapping_publishers_never_replace_the_first_committed_output() {
    if isolated("overlapping_publishers_never_replace_the_first_committed_output") {
        return;
    }
    let f = Fixture::new();
    let target = f.path("result.txt");
    let (staged, receiver) = std::sync::mpsc::channel();
    let release = std::sync::Arc::new(std::sync::Barrier::new(2));
    let gate = release.clone();
    let root = f.0.clone();
    let old_target = target.clone();
    let worker = std::thread::spawn(move || {
        ownership::set_observer(Some(Box::new(move |event, _| {
            if event == Event::Staged {
                staged.send(()).unwrap();
                gate.wait();
            }
            Ok(())
        })));
        write_batch(&root, &[(old_target, b"losing output".to_vec())], false)
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    write_batch(&f.0, &[(target.clone(), b"winning output".to_vec())], false).unwrap();
    release.wait();
    assert!(worker.join().unwrap().is_err());
    assert_eq!(fs::read(target).unwrap(), b"winning output");
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
}

#[test]
fn maximum_admitted_batch_publishes_and_retires_all_owned_artifacts() {
    if isolated("maximum_admitted_batch_publishes_and_retires_all_owned_artifacts") {
        return;
    }
    let f = Fixture::new();
    let outputs: Vec<_> = (0..128)
        .map(|n| {
            (
                f.path(&format!("{n}.txt")),
                format!("value {n}").into_bytes(),
            )
        })
        .collect();
    write_batch(&f.0, &outputs, false).unwrap();
    for (path, bytes) in &outputs {
        assert_eq!(&fs::read(path).unwrap(), bytes);
    }
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 128);
    assert_eq!(ownership::test_descriptors(), 0);
    assert_eq!(
        tagrex_core::resources::Budget::process().usage().0,
        tagrex_core::resources::Charge::default()
    );
}

#[cfg(windows)]
#[test]
fn private_directory_is_pinned_against_rename_until_its_own_retirement() {
    if isolated("private_directory_is_pinned_against_rename_until_its_own_retirement") {
        return;
    }
    let f = Fixture::new();
    ownership::set_observer(Some(Box::new(|event, path| {
        if event == Event::BeforePrivateRemove {
            assert!(fs::rename(path, path.with_extension("foreign-move")).is_err());
        }
        Ok(())
    })));
    let target = f.path("result.txt");
    write_batch(&f.0, &[(target.clone(), b"new".to_vec())], false).unwrap();
    assert_eq!(fs::read(target).unwrap(), b"new");
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
}
