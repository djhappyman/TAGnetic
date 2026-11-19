//! Public API overlap/alias tests. Every output belongs to disposable fixtures.
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Barrier,
    },
};
use tagrex_commands::publish_confined_outputs;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-output-public-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reversed_overlapping_public_batches_leave_a_coherent_committed_pair_or_no_pair() {
    // Different ordering exercises the two publishers each reaching one public
    // destination before encountering the other batch's destination.
    for round in 0..40 {
        let f = Fixture::new();
        let a = f.0.join("a.txt");
        let b = f.0.join("b.txt");
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for which in 0..2 {
            let (root, a, b, barrier) = (f.0.clone(), a.clone(), b.clone(), barrier.clone());
            workers.push(std::thread::spawn(move || {
                let bytes = if which == 0 {
                    vec![0x11; 128 * 1024]
                } else {
                    vec![0xee; 128 * 1024]
                };
                let outputs = if which == 0 {
                    vec![(a, bytes.clone()), (b, bytes)]
                } else {
                    vec![(b, bytes.clone()), (a, bytes)]
                };
                barrier.wait();
                publish_confined_outputs(&root, &outputs, false)
            }));
        }
        barrier.wait();
        let outcomes: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        match (fs::read(&a), fs::read(&b)) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a, b, "mixed batch contents in round {round}: {outcomes:?}");
                assert!(
                    outcomes.iter().any(Result::is_ok),
                    "uncommitted pair in round {round}: {outcomes:?}"
                );
            }
            (Err(a), Err(b)) => {
                assert_eq!(a.kind(), std::io::ErrorKind::NotFound);
                assert_eq!(b.kind(), std::io::ErrorKind::NotFound);
                assert!(outcomes.iter().all(Result::is_err));
            }
            other => panic!("partial pair in round {round}: {other:?}; {outcomes:?}"),
        }
    }
}

#[cfg(unix)]
#[test]
fn later_symlink_aliases_refuse_before_staging_or_overwriting_any_output() {
    let f = Fixture::new();
    let old = f.0.join("original.txt");
    let alias = f.0.join("alias.txt");
    let fresh = f.0.join("new.txt");
    fs::write(&old, b"original content").unwrap();
    std::os::unix::fs::symlink(&old, &alias).unwrap();
    for overwrite in [false, true] {
        let result = publish_confined_outputs(
            &f.0,
            &[
                (fresh.clone(), b"new".to_vec()),
                (alias.clone(), b"replacement".to_vec()),
            ],
            overwrite,
        );
        assert!(result.is_err());
        assert!(!fresh.exists());
        assert!(fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&old).unwrap(), b"original content");
    }
}

#[cfg(any(unix, windows))]
#[test]
fn later_hardlink_aliases_refuse_before_staging_or_overwriting_any_output() {
    let f = Fixture::new();
    let old = f.0.join("original.txt");
    let alias = f.0.join("alias.txt");
    let fresh = f.0.join("new.txt");
    fs::write(&old, b"original content").unwrap();
    fs::hard_link(&old, &alias).unwrap();
    let result = publish_confined_outputs(
        &f.0,
        &[
            (fresh.clone(), b"new".to_vec()),
            (old.clone(), b"one".to_vec()),
            (alias.clone(), b"two".to_vec()),
        ],
        true,
    );
    assert!(result.is_err());
    assert!(!fresh.exists());
    assert_eq!(fs::read(&old).unwrap(), b"original content");
    assert_eq!(fs::read(&alias).unwrap(), b"original content");
    assert_eq!(
        fs::read_dir(&f.0).unwrap().count(),
        2,
        "preflight created artifacts"
    );
}

#[cfg(unix)]
#[test]
fn native_byte_names_and_distinct_nested_names_publish_without_lossy_collisions() {
    #[cfg(not(target_vendor = "apple"))]
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let f = Fixture::new();
    #[cfg(not(target_vendor = "apple"))]
    let a = f.0.join(OsString::from_vec(b"report-\xff.txt".to_vec()));
    #[cfg(not(target_vendor = "apple"))]
    let b = f.0.join(OsString::from_vec(b"report-\xfe.txt".to_vec()));
    #[cfg(target_vendor = "apple")]
    let a = f.0.join("report-α.txt");
    #[cfg(target_vendor = "apple")]
    let b = f.0.join("report-β.txt");
    let sub = f.0.join("尾 ; quotes \" spaces");
    fs::create_dir(&sub).unwrap();
    let c = sub.join("report-ß.txt");
    publish_confined_outputs(
        &f.0,
        &[
            (a.clone(), b"one".to_vec()),
            (b.clone(), b"two".to_vec()),
            (c.clone(), b"three".to_vec()),
        ],
        false,
    )
    .unwrap();
    assert_eq!(fs::read(a).unwrap(), b"one");
    assert_eq!(fs::read(b).unwrap(), b"two");
    assert_eq!(fs::read(c).unwrap(), b"three");
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 3);
}

#[cfg(target_vendor = "apple")]
#[test]
fn unrepresentable_byte_names_refuse_without_publication_or_staging() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let f = Fixture::new();
    let original = f.0.join("original.txt");
    let fresh = f.0.join("new.txt");
    fs::write(&original, b"original").unwrap();
    for byte in [0xff, 0xfe] {
        let invalid = f.0.join(OsString::from_vec(vec![b'x', byte]));
        for overwrite in [false, true] {
            let error = publish_confined_outputs(
                &f.0,
                &[
                    (fresh.clone(), b"new".to_vec()),
                    (invalid.clone(), b"invalid".to_vec()),
                ],
                overwrite,
            )
            .unwrap_err();
            assert!(matches!(
                &error,
                tagrex_commands::OutputError::Io(error)
                    if error.raw_os_error() == Some(libc::EILSEQ)
            ));
            assert!(error.recovery_paths().is_empty());
            assert_eq!(fs::read(&original).unwrap(), b"original");
            assert!(!fresh.exists());
            assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
        }
    }
}

#[cfg(target_vendor = "apple")]
#[test]
fn normalized_unicode_aliases_share_one_original_and_refuse_duplicate_outputs() {
    let f = Fixture::new();
    let nfc = f.0.join("report-é.txt");
    let nfd = f.0.join("report-e\u{301}.txt");
    fs::write(&nfc, b"original").unwrap();
    assert_eq!(fs::read(&nfd).unwrap(), b"original");
    publish_confined_outputs(&f.0, &[(nfd.clone(), b"updated".to_vec())], true).unwrap();
    assert_eq!(fs::read(&nfc).unwrap(), b"updated");
    let fresh = f.0.join("new.txt");
    let error = publish_confined_outputs(
        &f.0,
        &[
            (fresh.clone(), b"new".to_vec()),
            (nfc.clone(), b"one".to_vec()),
            (nfd.clone(), b"two".to_vec()),
        ],
        true,
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.recovery_paths().is_empty());
    assert_eq!(fs::read(&nfc).unwrap(), b"updated");
    assert_eq!(fs::read(&nfd).unwrap(), b"updated");
    assert!(!fresh.exists());
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
}
