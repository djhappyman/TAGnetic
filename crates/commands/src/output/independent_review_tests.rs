//! Independent interleavings of the real publisher, using disposable paths.
use super::{
    ownership::{self, Event},
    write_batch, OutputError,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-output-independent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        ownership::set_observer(None);
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn isolate(name: &str) -> bool {
    if std::env::var("TAGREX_OUTPUT_REVIEW").as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("output::independent_review_tests::{name}"),
            "--nocapture",
        ])
        .env("TAGREX_OUTPUT_REVIEW", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[cfg(unix)]
#[test]
fn moved_library_root_is_not_reported_as_success_at_its_old_logical_path() {
    let name = "moved_library_root_is_not_reported_as_success_at_its_old_logical_path";
    if isolate(name) {
        return;
    }
    let f = Fixture::new();
    let root = f.0.join("library");
    fs::create_dir(&root).unwrap();
    let moved = f.0.join("moved-library");
    let (root_hook, moved_hook) = (root.clone(), moved.clone());
    let mut fired = false;
    ownership::set_observer(Some(Box::new(move |event, _| {
        if event == Event::Staged && !fired {
            fired = true;
            fs::rename(&root_hook, &moved_hook)?;
            fs::create_dir(&root_hook)?;
            fs::write(root_hook.join("foreign.txt"), b"new root contents")?;
        }
        Ok(())
    })));
    let result = write_batch(
        &root,
        &[(root.join("report.txt"), b"generated".to_vec())],
        false,
    );
    assert_eq!(
        fs::read(root.join("foreign.txt")).unwrap(),
        b"new root contents"
    );
    assert!(
        result.is_err(),
        "publisher returned success at a moved root: {result:?}"
    );
    assert!(!root.join("report.txt").exists());
    assert!(
        !moved.join("report.txt").exists(),
        "stale root must be refused before publication"
    );
}

fn has_bytes(root: &Path, wanted: &[u8]) -> bool {
    let mut dirs = vec![root.to_owned()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                dirs.push(entry.path());
            } else if fs::read(entry.path()).ok().as_deref() == Some(wanted) {
                return true;
            }
        }
    }
    false
}

#[test]
fn phase_failure_matrix_preserves_originals_or_the_verified_committed_outputs() {
    let name = "phase_failure_matrix_preserves_originals_or_the_verified_committed_outputs";
    if isolate(name) {
        return;
    }
    let phases = [
        Event::BeforeSync,
        Event::Staged,
        Event::BeforePublish,
        Event::CandidateCaptured,
        Event::BackupCaptured,
        Event::BeforeCommit,
        Event::BeforeCleanup,
        Event::BeforePrivateRemove,
        Event::BeforeRestore,
        Event::RestoreCaptured,
    ];
    let mut cases = 0;
    for phase in phases {
        let max = if matches!(phase, Event::BeforeCommit | Event::BeforeCleanup) {
            1
        } else {
            2
        };
        for occurrence in 1..=max {
            let f = Fixture::new();
            let a = f.0.join("one");
            let b = f.0.join("two");
            fs::create_dir(&a).unwrap();
            fs::create_dir(&b).unwrap();
            let first = a.join("report.txt");
            let second = b.join("report.txt");
            let original_a = vec![0x71; 131_071];
            let original_b = vec![0x8f; 65_537];
            fs::write(&first, &original_a).unwrap();
            fs::write(&second, &original_b).unwrap();
            let hit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let hit_hook = hit.clone();
            let mut seen = 0;
            ownership::set_observer(Some(Box::new(move |event, _| {
                if matches!(phase, Event::BeforeRestore | Event::RestoreCaptured)
                    && event == Event::BeforeCommit
                {
                    return Err(io::Error::other("fixture requests rollback"));
                }
                if event == phase {
                    seen += 1;
                    if seen == occurrence {
                        hit_hook.store(true, Ordering::Relaxed);
                        return Err(io::Error::other("independent phase failure"));
                    }
                }
                Ok(())
            })));
            let budget = tagrex_core::resources::Budget::process();
            let prior = budget.usage().0;
            let result = write_batch(
                &f.0,
                &[
                    (first.clone(), b"new one".to_vec()),
                    (second.clone(), b"new two".to_vec()),
                ],
                true,
            );
            assert!(
                hit.load(Ordering::Relaxed),
                "phase not reached: {phase:?}/{occurrence}"
            );
            assert!(
                result.is_err(),
                "fault must not be successful: {phase:?}/{occurrence}"
            );
            if matches!(phase, Event::BeforeCleanup | Event::BeforePrivateRemove) {
                assert_eq!(
                    fs::read(&first).unwrap(),
                    b"new one",
                    "{phase:?}/{occurrence}"
                );
                assert_eq!(
                    fs::read(&second).unwrap(),
                    b"new two",
                    "{phase:?}/{occurrence}"
                );
            } else {
                assert!(
                    has_bytes(&f.0, &original_a),
                    "lost first original at {phase:?}/{occurrence}"
                );
                assert!(
                    has_bytes(&f.0, &original_b),
                    "lost second original at {phase:?}/{occurrence}"
                );
            }
            if let Err(OutputError::RecoveryRequired { paths, .. }) = &result {
                assert!(
                    paths.iter().any(|path| path.exists()),
                    "no inspectable recovery path at {phase:?}/{occurrence}: {paths:?}"
                );
            }
            drop(result);
            assert_eq!(
                budget.usage().0,
                prior,
                "resource leak at {phase:?}/{occurrence}"
            );
            assert_eq!(
                ownership::test_descriptors(),
                0,
                "descriptor leak at {phase:?}/{occurrence}"
            );
            cases += 1;
        }
    }
    eprintln!("INDEPENDENT_OUTPUT_PHASE_FAILURE_CASES={cases}");
}

#[cfg(unix)]
#[test]
fn root_replacement_at_commit_refuses_and_restores_inside_the_pinned_original_tree() {
    let name = "root_replacement_at_commit_refuses_and_restores_inside_the_pinned_original_tree";
    if isolate(name) {
        return;
    }
    for nested in [false, true] {
        let f = Fixture::new();
        let root = f.0.join("library");
        let moved = f.0.join("moved-library");
        let parent = if nested {
            root.join("nested")
        } else {
            root.clone()
        };
        fs::create_dir_all(&parent).unwrap();
        fs::write(parent.join("report.txt"), b"old report").unwrap();
        let (root_hook, moved_hook) = (root.clone(), moved.clone());
        ownership::set_observer(Some(Box::new(move |event, _| {
            if event == Event::BeforeCommit {
                fs::rename(&root_hook, &moved_hook)?;
                fs::create_dir(&root_hook)?;
                fs::write(root_hook.join("foreign.txt"), b"replacement root")?;
            }
            Ok(())
        })));
        let result = write_batch(
            &root,
            &[(parent.join("report.txt"), b"new report".to_vec())],
            true,
        );
        assert!(
            result.is_err(),
            "root changed after publication but result was {result:?}; nested={nested}"
        );
        let restored = if nested {
            moved.join("nested/report.txt")
        } else {
            moved.join("report.txt")
        };
        assert_eq!(fs::read(restored).unwrap(), b"old report");
        assert_eq!(
            fs::read(root.join("foreign.txt")).unwrap(),
            b"replacement root"
        );
        assert!(!parent.join("report.txt").exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn partial_private_cleanup_reports_inspectable_paths_without_deleted_pseudo_names() {
    let name = "partial_private_cleanup_reports_inspectable_paths_without_deleted_pseudo_names";
    if isolate(name) {
        return;
    }
    let f = Fixture::new();
    let a = f.0.join("one");
    let b = f.0.join("two");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let first = a.join("report.txt");
    let second = b.join("report.txt");
    fs::write(&first, b"old one").unwrap();
    fs::write(&second, b"old two").unwrap();
    let mut cleanup = 0;
    ownership::set_observer(Some(Box::new(move |event, _| {
        if event == Event::BeforePrivateRemove {
            cleanup += 1;
            if cleanup == 2 {
                return Err(io::Error::other("second directory cleanup fails"));
            }
        }
        Ok(())
    })));
    let error = write_batch(
        &f.0,
        &[
            (first.clone(), b"new one".to_vec()),
            (second.clone(), b"new two".to_vec()),
        ],
        true,
    )
    .unwrap_err();
    assert!(matches!(error, OutputError::RecoveryRequired { .. }));
    assert_eq!(fs::read(&first).unwrap(), b"new one");
    assert_eq!(fs::read(&second).unwrap(), b"new two");
    let paths = error.recovery_paths();
    assert!(!paths.is_empty());
    assert!(
        paths.iter().all(|path| path.exists()),
        "recovery evidence includes uninspectable removed paths: {paths:?}"
    );
    for path in paths.iter().filter(|path| path.is_dir()) {
        assert!(
            path.join("recovery.json").is_file(),
            "retained directory lacks evidence: {path:?}"
        );
    }
}
