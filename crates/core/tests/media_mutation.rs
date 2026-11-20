//! Public-API durability probes. Every original and artifact is disposable.
#![cfg(target_os = "linux")]
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};
use tagrex_core::{
    media_mutation::{
        self as media, BoundMedia, CopyKind, Current, Error, Event, Limits, Observer, Phase,
        Record, Store, SyncKind, BUFFER_BYTES,
    },
    resources::{Budget, Limits as ResourceLimits},
};
// A fork temporarily inherits other threads' open-file descriptions, including
// flock locks, until exec closes them. Keep subprocess tests separate from tests
// that require dropping the final Store handle to permit an immediate reopen.
static PROCESS_BOUNDARY: Mutex<()> = Mutex::new(());
fn process_boundary() -> MutexGuard<'static, ()> {
    PROCESS_BOUNDARY.lock().unwrap()
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-media-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
    fn store(&self, limits: Limits, budget: Budget) -> Store {
        Store::open(&self.0.join("store"), limits, budget).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn budget() -> Budget {
    Budget::new(ResourceLimits {
        retained: 64 << 20,
        transient: 4 << 20,
        total: 68 << 20,
        jobs: 8,
    })
}
fn limits() -> Limits {
    Limits {
        store_bytes: 32 << 20,
        entries: 16,
        media_bytes: 8 << 20,
    }
}
fn sink(_: &Record) -> io::Result<()> {
    Ok(())
}
fn entry(store: &Path, id: &str, name: &str) -> PathBuf {
    store.join(id).join(name)
}
fn prepare(store: &Store, path: &Path, bytes: &[u8]) -> media::Prepared {
    store
        .prepare(
            BoundMedia::open(path).unwrap(),
            "flac",
            bytes.len() as u64,
            &mut sink,
            &mut (),
            |candidate| {
                assert_eq!(candidate.extension().unwrap(), "flac");
                fs::write(candidate, bytes)
            },
        )
        .unwrap()
}
#[test]
fn process_boundary_isolation_prevents_inherited_store_lock_contention() {
    use std::os::unix::{net::UnixStream, process::CommandExt};
    let _boundary = process_boundary();
    let f = Fixture::new();
    let store = f.store(limits(), budget());
    let (mut parent, mut child_gate) = UnixStream::pair().unwrap();
    parent
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    child_gate
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let child = std::thread::spawn(move || {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.arg("--help").stdout(std::process::Stdio::null());
        // Only pipe I/O occurs between fork and exec.
        unsafe {
            command.pre_exec(move || {
                child_gate.write_all(b"r")?;
                child_gate.read_exact(&mut [0])
            });
        }
        command.spawn().unwrap().wait().unwrap()
    });
    parent.read_exact(&mut [0]).unwrap();
    drop(store);
    let inherited_lock = Store::open(&f.0.join("store"), limits(), budget());
    parent.write_all(b"x").unwrap();
    assert!(child.join().unwrap().success());
    assert!(matches!(inherited_lock, Err(Error::Capacity)));
    let reopened = f.store(limits(), budget());
    assert!(reopened.records().unwrap().is_empty());
}
#[test]
fn publishing_grows_and_shrinks_the_same_inode_and_retirement_requires_durable_confirmation() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    for after in [
        vec![b'B'; 2 * BUFFER_BYTES + 7],
        b"small".to_vec(),
        Vec::new(),
    ] {
        let f = Fixture::new();
        let before = vec![b'A'; BUFFER_BYTES + 9];
        let path = f.put("original.flac", &before);
        let link = f.0.join("linked.flac");
        fs::hard_link(&path, &link).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let original = fs::metadata(&path).unwrap();
        let store = f.store(limits(), budget());
        let prepared = prepare(&store, &path, &after);
        assert_eq!(fs::read(&path).unwrap(), before);
        let phases = Arc::new(Mutex::new(Vec::new()));
        let capture = phases.clone();
        let mut state = move |record: &Record| {
            capture.lock().unwrap().push(record.phase());
            Ok(())
        };
        let published = prepared.publish(&mut state, &mut ()).unwrap();
        let id = published.record().id().to_string();
        assert_eq!(fs::read(&path).unwrap(), after);
        assert_eq!(fs::read(&link).unwrap(), after);
        let now = fs::metadata(&path).unwrap();
        assert_eq!(
            (
                now.dev(),
                now.ino(),
                now.mode(),
                now.uid(),
                now.gid(),
                now.nlink()
            ),
            (
                original.dev(),
                original.ino(),
                original.mode(),
                original.uid(),
                original.gid(),
                original.nlink()
            )
        );
        assert_eq!(
            *phases.lock().unwrap(),
            [Phase::Publishing, Phase::Published]
        );
        assert!(published
            .retire(
                |_| Err(io::Error::other("journal finish failed")),
                &mut sink,
                &mut ()
            )
            .is_err());
        assert_eq!(store.records().unwrap()[0].phase(), Phase::Published);
        let published = store
            .resume_published(&id, BoundMedia::open(&path).unwrap())
            .unwrap();
        published.retire(|_| Ok(()), &mut sink, &mut ()).unwrap();
        assert_eq!(store.usage(), (0, 0));
        assert_eq!(fs::read(&path).unwrap(), after);
    }
}
#[test]
fn publication_preserves_extended_attributes() {
    let f = Fixture::new();
    let path = f.put("x.flac", b"original");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    rustix::fs::fsetxattr(
        &file,
        "user.tagrex-fixture",
        b"private fixture",
        rustix::fs::XattrFlags::empty(),
    )
    .unwrap();
    let store = f.store(limits(), budget());
    prepare(&store, &path, b"changed")
        .publish(&mut sink, &mut ())
        .unwrap()
        .retire(|_| Ok(()), &mut sink, &mut ())
        .unwrap();
    let mut bytes = [0; 64];
    let n = rustix::fs::fgetxattr(&file, "user.tagrex-fixture", &mut bytes).unwrap();
    assert_eq!(&bytes[..n], b"private fixture");
}
#[test]
fn callback_errors_panics_and_preintent_faults_leave_original_and_release_all_charges() {
    struct Fail(Event);
    impl Observer for Fail {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if event == self.0 {
                Err(io::Error::from_raw_os_error(28))
            } else {
                Ok(())
            }
        }
    }
    let f = Fixture::new();
    let before = vec![b'A'; 2 * BUFFER_BYTES];
    let path = f.put("original.flac", &before);
    let resource = budget();
    let store = f.store(limits(), resource.clone());
    let baseline = resource.usage().0;
    for event in [
        Event::CopyStarted(CopyKind::Preimage),
        Event::Copied(CopyKind::Preimage, BUFFER_BYTES as u64),
        Event::BeforeSync(SyncKind::Preimage),
        Event::CopyStarted(CopyKind::Candidate),
        Event::BeforeEdit,
        Event::BeforeSync(SyncKind::Candidate),
        Event::BeforeCallback(Phase::Prepared),
    ] {
        assert!(store
            .prepare(
                BoundMedia::open(&path).unwrap(),
                "flac",
                0,
                &mut sink,
                &mut Fail(event),
                |_| Ok(())
            )
            .is_err());
        assert_eq!(store.usage(), (0, 0));
        assert_eq!(resource.usage().0, baseline);
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    assert!(store
        .prepare(
            BoundMedia::open(&path).unwrap(),
            "flac",
            0,
            &mut sink,
            &mut (),
            |_| Err(io::Error::other("writer refused"))
        )
        .is_err());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.prepare(
            BoundMedia::open(&path).unwrap(),
            "flac",
            0,
            &mut sink,
            &mut (),
            |_| panic!("candidate writer panic")
        )))
        .is_err()
    );
    let prepared = prepare(&store, &path, b"temporary");
    drop(prepared);
    assert_eq!(store.usage(), (0, 0));
    assert_eq!(resource.usage().0, baseline);
    assert_eq!(fs::read(&path).unwrap(), before);
}
#[test]
fn intent_acknowledgement_failure_and_postwrite_failure_retain_recovery_images() {
    let _boundary = process_boundary();
    for failure in [Phase::Publishing, Phase::Published] {
        let f = Fixture::new();
        let path = f.put("original.flac", b"before");
        let store = f.store(limits(), budget());
        let prepared = prepare(&store, &path, b"after!");
        let id = prepared.record().id().to_string();
        let mut fail = |record: &Record| {
            if record.phase() == failure {
                Err(io::Error::other("uncertain durable ack"))
            } else {
                Ok(())
            }
        };
        assert!(prepared.publish(&mut fail, &mut ()).is_err());
        let expected = if failure == Phase::Publishing {
            Current::Preimage
        } else {
            Current::Candidate
        };
        assert_eq!(
            store
                .inspect(&id, &mut BoundMedia::open(&path).unwrap())
                .unwrap(),
            expected
        );
        for name in ["preimage", "candidate.flac"] {
            assert!(entry(&f.0.join("store"), &id, name).exists());
        }
        let charged = store.usage();
        drop(store);
        let reopened = f.store(limits(), budget());
        assert_eq!(reopened.usage(), charged);
        assert_eq!(
            reopened
                .inspect(&id, &mut BoundMedia::open(&path).unwrap())
                .unwrap(),
            expected
        );
    }
}

#[test]
fn a_panicking_intent_sink_retains_images_but_releases_jobs_and_the_original_lock() {
    let f = Fixture::new();
    let path = f.put("x.flac", b"before");
    let resource = budget();
    let store = f.store(limits(), resource.clone());
    let baseline = resource.usage().0;
    let prepared = prepare(&store, &path, b"after!");
    let id = prepared.record().id().to_string();
    let mut panic_sink = |record: &Record| {
        assert_eq!(record.phase(), Phase::Publishing);
        panic!("durable sink unwind probe");
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prepared.publish(&mut panic_sink, &mut ())
    }));
    assert!(result.is_err());
    assert_eq!(resource.usage().0, baseline);
    assert_eq!(fs::read(&path).unwrap(), b"before");
    assert_eq!(
        store
            .inspect(&id, &mut BoundMedia::open(&path).unwrap())
            .unwrap(),
        Current::Preimage
    );
    assert!(entry(&f.0.join("store"), &id, "preimage").exists());
    assert!(entry(&f.0.join("store"), &id, "candidate.flac").exists());
}
#[test]
fn short_writes_retry_and_write_zero_or_disk_full_after_intent_never_destroy_evidence() {
    struct Fault {
        limit: usize,
        fail_after: Option<usize>,
        writes: usize,
    }
    impl Observer for Fault {
        fn write_limit(&mut self, kind: CopyKind, n: usize) -> io::Result<usize> {
            if kind == CopyKind::Publish {
                self.writes += 1;
                if self.fail_after == Some(self.writes) {
                    return Err(io::Error::from_raw_os_error(28));
                }
                Ok(n.min(self.limit))
            } else {
                Ok(n)
            }
        }
    }
    for (limit, fail) in [(7, None), (0, None), (7, Some(20))] {
        let f = Fixture::new();
        let path = f.put("x.flac", &vec![b'A'; 2 * BUFFER_BYTES + 3]);
        let after = vec![b'B'; 2 * BUFFER_BYTES + 3];
        let store = f.store(limits(), budget());
        let prepared = prepare(&store, &path, &after);
        let id = prepared.record().id().to_string();
        let result = prepared.publish(
            &mut sink,
            &mut Fault {
                limit,
                fail_after: fail,
                writes: 0,
            },
        );
        if limit != 0 && fail.is_none() {
            result
                .unwrap()
                .retire(|_| Ok(()), &mut sink, &mut ())
                .unwrap();
            assert_eq!(fs::read(&path).unwrap(), after);
        } else {
            assert!(result.is_err());
            let current = store
                .inspect(&id, &mut BoundMedia::open(&path).unwrap())
                .unwrap();
            assert_eq!(
                current,
                if limit == 0 {
                    Current::Preimage
                } else {
                    Current::Ambiguous
                }
            );
            assert!(entry(&f.0.join("store"), &id, "preimage").exists());
        }
    }
}
#[test]
fn changed_original_and_replaced_artifacts_are_refused_and_foreign_bytes_survive() {
    for target in ["source", "candidate.flac", "preimage", "1-prepared"] {
        let f = Fixture::new();
        let path = f.put("x.flac", b"original");
        let store = f.store(limits(), budget());
        let prepared = prepare(&store, &path, b"modified");
        let id = prepared.record().id().to_string();
        let replacement = if target == "source" {
            path.clone()
        } else {
            entry(&f.0.join("store"), &id, target)
        };
        fs::remove_file(&replacement).unwrap();
        let mut options = OpenOptions::new();
        use std::os::unix::fs::OpenOptionsExt;
        let mut foreign = options
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&replacement)
            .unwrap();
        foreign.write_all(b"foreign bytes").unwrap();
        drop(foreign);
        assert!(prepared.publish(&mut sink, &mut ()).is_err());
        assert_eq!(fs::read(&replacement).unwrap(), b"foreign bytes");
        if target != "source" {
            assert_eq!(fs::read(&path).unwrap(), b"original");
        }
    }
}
#[test]
fn sink_cannot_redirect_publication_by_changing_source_or_private_parent() {
    for which in ["source", "store"] {
        let f = Fixture::new();
        let path = f.put("x.flac", b"original");
        let store = f.store(limits(), budget());
        let prepared = prepare(&store, &path, b"modified");
        let held = f.0.join("held-store");
        let mut state = |record: &Record| {
            if record.phase() == Phase::Publishing {
                if which == "source" {
                    fs::write(&path, b"foreign edit")?;
                } else {
                    fs::rename(f.0.join("store"), &held)?;
                    fs::create_dir(f.0.join("store"))?;
                    fs::write(f.0.join("store/foreign"), b"keep")?;
                }
            }
            Ok(())
        };
        assert!(prepared.publish(&mut state, &mut ()).is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            if which == "source" {
                b"foreign edit".as_slice()
            } else {
                b"original".as_slice()
            }
        );
        if which == "store" {
            assert_eq!(fs::read(f.0.join("store/foreign")).unwrap(), b"keep");
            assert!(fs::read_dir(&held).unwrap().count() > 1);
        }
    }
}
#[test]
fn invalid_permissions_links_names_and_oversized_work_fail_before_original_mutation() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = Fixture::new();
    let path = f.put("x.flac", b"original");
    let link = f.0.join("linked.flac");
    symlink(&path, &link).unwrap();
    assert!(BoundMedia::open(&link).is_err());
    assert!(BoundMedia::open(&f.0).is_err());
    let wrong = f.put("wrong.flac", b"other");
    assert!(BoundMedia::from_file(File::open(&path).unwrap(), &path).is_err());
    assert!(BoundMedia::from_file(
        OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .unwrap(),
        &path
    )
    .is_err());
    assert!(BoundMedia::from_file(File::open(&wrong).unwrap(), &path).is_err());
    fs::create_dir(f.0.join("broad")).unwrap();
    fs::set_permissions(f.0.join("broad"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Store::open(&f.0.join("broad"), limits(), budget()).is_err());
    let store = f.store(limits(), budget());
    for hint in ["", "../x", "flac/evil", "フラック"] {
        assert!(store
            .prepare(
                BoundMedia::open(&path).unwrap(),
                hint,
                0,
                &mut sink,
                &mut (),
                |_| panic!("must refuse first")
            )
            .is_err());
    }
    assert!(store
        .prepare(
            BoundMedia::open(&path).unwrap(),
            "flac",
            u64::MAX,
            &mut sink,
            &mut (),
            |_| panic!("must refuse first")
        )
        .is_err());
    assert!(store
        .prepare(
            BoundMedia::open(&path).unwrap(),
            "flac",
            0,
            &mut sink,
            &mut (),
            |candidate| fs::write(candidate, vec![0; 64])
        )
        .is_err());
    assert_eq!(store.usage(), (0, 0));
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert!(Store::open(&f.0.join("store"), limits(), budget()).is_err());
}

#[test]
fn publication_refuses_permission_changes_after_the_original_capability_was_opened() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let path = f.put("x.flac", b"before");
    let store = f.store(limits(), budget());
    let prepared = prepare(&store, &path, b"after!");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    assert!(prepared.publish(&mut sink, &mut ()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"before");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o444
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn original_file_admission_is_exclusive_even_for_cloned_caller_handles_and_different_stores() {
    let f = Fixture::new();
    let path = f.put("x.flac", b"before");
    let caller = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let original = BoundMedia::from_file(caller.try_clone().unwrap(), &path).unwrap();
    assert!(matches!(
        BoundMedia::from_file(caller.try_clone().unwrap(), &path),
        Err(Error::Capacity)
    ));
    assert!(matches!(BoundMedia::open(&path), Err(Error::Capacity)));
    let store = f.store(limits(), budget());
    let prepared = store
        .prepare(original, "flac", 8, &mut sink, &mut (), |candidate| {
            fs::write(candidate, b"after!")
        })
        .unwrap();
    assert!(matches!(BoundMedia::open(&path), Err(Error::Capacity)));
    drop(prepared);
    let original = BoundMedia::open(&path).unwrap();
    drop(original);
    assert!(BoundMedia::from_file(caller, &path).is_ok());
}
#[test]
fn concurrent_disk_and_resource_admission_is_bounded_and_recovers_after_errors() {
    let f = Fixture::new();
    let paths: Vec<_> = (0..8)
        .map(|i| f.put(&format!("{i}.flac"), &vec![b'A'; BUFFER_BYTES]))
        .collect();
    let resource = Budget::new(ResourceLimits {
        retained: 1 << 20,
        transient: 2 * (2 * BUFFER_BYTES + 8192),
        total: 2 << 20,
        jobs: 2,
    });
    let store = f.store(
        Limits {
            store_bytes: 400_000,
            entries: 8,
            media_bytes: 1 << 20,
        },
        resource.clone(),
    );
    let baseline = resource.usage().0;
    let released = Arc::new((Mutex::new(false), Condvar::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for path in paths {
            let store = store.clone();
            let released = released.clone();
            let tx = tx.clone();
            handles.push(scope.spawn(move || {
                let entered = std::cell::Cell::new(false);
                let result = store.prepare(
                    BoundMedia::open(&path).unwrap(),
                    "flac",
                    0,
                    &mut sink,
                    &mut (),
                    |_| {
                        entered.set(true);
                        tx.send(true).unwrap();
                        let (lock, wake) = &*released;
                        let mut ready = lock.lock().unwrap();
                        while !*ready {
                            ready = wake.wait(ready).unwrap();
                        }
                        Ok(())
                    },
                );
                if !entered.get() {
                    assert!(matches!(result, Err(Error::Capacity | Error::Resource(_))));
                    tx.send(false).unwrap();
                }
                drop(result);
            }));
        }
        let admitted = (0..8)
            .map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(admitted, 2);
        assert_eq!(store.usage().1, 2);
        assert!(store.usage().0 <= 400_000);
        assert_eq!(resource.usage().0.jobs, 2);
        let (lock, wake) = &*released;
        *lock.lock().unwrap() = true;
        wake.notify_all();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    assert_eq!(store.usage(), (0, 0));
    assert_eq!(resource.usage().0, baseline);
}

fn flac(title: &str) -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut comments = 4u32.to_le_bytes().to_vec();
    comments.extend(b"test");
    comments.extend(1u32.to_le_bytes());
    let value = format!("TITLE={title}");
    comments.extend((value.len() as u32).to_le_bytes());
    comments.extend(value.as_bytes());
    let n = comments.len();
    out.extend([4, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(comments);
    out.extend([0x81, 0, 0, 128]);
    out.extend([0; 128]);
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out
}
fn raw_flac(bytes: &[u8]) -> (String, Vec<u8>) {
    assert_eq!(&bytes[..4], b"fLaC");
    let mut at = 4;
    let mut title = None;
    loop {
        let flag = bytes[at];
        let n = ((bytes[at + 1] as usize) << 16)
            | ((bytes[at + 2] as usize) << 8)
            | bytes[at + 3] as usize;
        let block = &bytes[at + 4..at + 4 + n];
        at += 4 + n;
        if flag & 127 == 4 {
            let number =
                |at: usize| u32::from_le_bytes(block[at..at + 4].try_into().unwrap()) as usize;
            let mut pos = 4 + number(0);
            let count = number(pos);
            pos += 4;
            for _ in 0..count {
                let n = number(pos);
                pos += 4;
                let item = std::str::from_utf8(&block[pos..pos + n]).unwrap();
                pos += n;
                if let Some(value) = item.strip_prefix("TITLE=") {
                    title = Some(value.to_string());
                }
            }
        }
        if flag & 128 != 0 {
            break;
        }
    }
    (title.unwrap(), bytes[at..].to_vec())
}
#[test]
fn existing_tag_engine_can_edit_an_extension_hinted_candidate_before_original_publication() {
    use tagrex_core::model::{TagEngine, TagField};
    let f = Fixture::new();
    let before = flac("Before");
    let path = f.put("track.flac", &before);
    let store = f.store(limits(), budget());
    let prepared = store
        .prepare(
            BoundMedia::open(&path).unwrap(),
            "flac",
            4096,
            &mut sink,
            &mut (),
            |candidate| {
                let mut track = TagEngine::read(candidate).map_err(io::Error::other)?;
                track.tags.insert(TagField::Title, "After 日本語".into());
                TagEngine::write(&track).map_err(io::Error::other)?;
                assert_eq!(fs::read(&path)?, before);
                Ok(())
            },
        )
        .unwrap();
    let after = fs::read(entry(
        &f.0.join("store"),
        prepared.record().id(),
        "candidate.flac",
    ))
    .unwrap();
    assert_eq!(raw_flac(&after).0, "After 日本語");
    assert_eq!(raw_flac(&after).1, raw_flac(&before).1);
    prepared
        .publish(&mut sink, &mut ())
        .unwrap()
        .retire(|_| Ok(()), &mut sink, &mut ())
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), after);
}
#[test]
fn publication_progress_callbacks_cannot_overwrite_a_new_foreign_edit() {
    struct Edit(PathBuf);
    impl Observer for Edit {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if event == Event::CopyStarted(CopyKind::Publish) {
                fs::write(&self.0, b"foreign edit")?;
            }
            Ok(())
        }
    }
    let f = Fixture::new();
    let path = f.put("x.flac", b"original");
    let store = f.store(limits(), budget());
    let prepared = prepare(&store, &path, b"modified");
    let id = prepared.record().id().to_string();
    assert!(prepared
        .publish(&mut sink, &mut Edit(path.clone()))
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"foreign edit");
    assert_eq!(
        store
            .inspect(&id, &mut BoundMedia::open(&path).unwrap())
            .unwrap(),
        Current::Ambiguous
    );
}
#[test]
fn every_sync_failure_before_retirement_is_conservative() {
    struct Fail {
        at: SyncKind,
        seen: bool,
    }
    impl Observer for Fail {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if event == Event::BeforeSync(self.at) && !self.seen {
                self.seen = true;
                return Err(io::Error::other("injected fsync failure"));
            }
            Ok(())
        }
    }
    for sync in [
        SyncKind::Manifest(Phase::Preparing),
        SyncKind::Preimage,
        SyncKind::Candidate,
        SyncKind::Manifest(Phase::Prepared),
        SyncKind::Manifest(Phase::Publishing),
        SyncKind::Original,
        SyncKind::Manifest(Phase::Published),
    ] {
        let f = Fixture::new();
        let before = b"before";
        let after = b"after!";
        let path = f.put("x.flac", before);
        let store = f.store(limits(), budget());
        let mut observer = Fail {
            at: sync,
            seen: false,
        };
        let result = store
            .prepare(
                BoundMedia::open(&path).unwrap(),
                "flac",
                0,
                &mut sink,
                &mut observer,
                |candidate| fs::write(candidate, after),
            )
            .and_then(|prepared| prepared.publish(&mut sink, &mut observer));
        assert!(result.is_err());
        let current = fs::read(&path).unwrap();
        assert_eq!(
            current,
            if matches!(
                sync,
                SyncKind::Original | SyncKind::Manifest(Phase::Published)
            ) {
                after
            } else {
                before
            }
        );
        if matches!(
            sync,
            SyncKind::Manifest(Phase::Publishing)
                | SyncKind::Original
                | SyncKind::Manifest(Phase::Published)
        ) {
            let records = store.records().unwrap();
            assert_eq!(records.len(), 1);
            assert!(entry(&f.0.join("store"), records[0].id(), "preimage").exists());
        } else {
            assert_eq!(store.usage(), (0, 0));
        }
    }
}
#[test]
fn corrupt_manifests_refuse_reopen_without_deleting_or_mutating_any_evidence() {
    let _boundary = process_boundary();
    let f = Fixture::new();
    let path = f.put("x.flac", b"before");
    let store = f.store(limits(), budget());
    let prepared = prepare(&store, &path, b"after!");
    let id = prepared.record().id().to_string();
    let published = prepared.publish(&mut sink, &mut ()).unwrap();
    drop(published);
    drop(store);
    let manifest = entry(&f.0.join("store"), &id, "2-publishing");
    let original = fs::read(&manifest).unwrap();
    for offset in [0, 8, 9, 10, 32, original.len() - 1] {
        let mut bytes = original.clone();
        bytes[offset] ^= 0xff;
        fs::write(&manifest, &bytes).unwrap();
        assert!(Store::open(&f.0.join("store"), limits(), budget()).is_err());
        assert_eq!(fs::read(&manifest).unwrap(), bytes);
        assert_eq!(fs::read(&path).unwrap(), b"after!");
        assert_eq!(
            fs::read(entry(&f.0.join("store"), &id, "preimage")).unwrap(),
            b"before"
        );
    }
    fs::write(&manifest, &original).unwrap();
    let reopened = f.store(limits(), budget());
    let published = reopened
        .resume_published(&id, BoundMedia::open(&path).unwrap())
        .unwrap();
    published.retire(|_| Ok(()), &mut sink, &mut ()).unwrap();
}
#[test]
fn retired_manifest_cannot_smuggle_a_traversal_into_cleanup() {
    use sha2::{Digest, Sha256};
    struct Stop;
    impl Observer for Stop {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if matches!(event, Event::ArtifactRemoved(_)) {
                Err(io::Error::other("stop cleanup"))
            } else {
                Ok(())
            }
        }
    }
    let f = Fixture::new();
    let path = f.put("x.flac", b"before");
    let store = f.store(limits(), budget());
    let prepared = prepare(&store, &path, b"after!");
    let id = prepared.record().id().to_string();
    assert!(prepared
        .publish(&mut sink, &mut ())
        .unwrap()
        .retire(|_| Ok(()), &mut sink, &mut Stop)
        .is_err());
    let terminal = f.0.join("store").join(format!("t-{}", &id[2..]));
    let mut bytes = fs::read(&terminal).unwrap();
    let at = bytes
        .windows(8)
        .position(|part| part == b"preimage")
        .unwrap();
    bytes[at..at + 8].copy_from_slice(b"../owned");
    let end = bytes.len() - 32;
    let hash = Sha256::digest(&bytes[..end]);
    bytes[end..].copy_from_slice(&hash);
    fs::write(&terminal, &bytes).unwrap();
    let foreign = f.put("store/owned", b"foreign");
    assert!(store
        .finish_retirement(
            &id,
            &mut BoundMedia::open(&path).unwrap(),
            |_| Ok(()),
            &mut ()
        )
        .is_err());
    assert_eq!(fs::read(foreign).unwrap(), b"foreign");
}

struct Trace {
    events: Vec<Event>,
}
impl Observer for Trace {
    fn event(&mut self, event: Event) -> io::Result<()> {
        self.events.push(event);
        Ok(())
    }
}
struct StopAt {
    index: usize,
    count: usize,
    marker: PathBuf,
}
impl Observer for StopAt {
    fn event(&mut self, _event: Event) -> io::Result<()> {
        self.count += 1;
        if self.count == self.index {
            let mut file = File::create(&self.marker)?;
            file.write_all(b"ready")?;
            file.sync_all()?;
            loop {
                std::thread::park();
            }
        }
        Ok(())
    }
}
fn kill_flow(root: &Path, observer: &mut dyn Observer) {
    let store = Store::open(&root.join("store"), limits(), budget()).unwrap();
    let path = root.join("original.flac");
    let after = vec![b'B'; 2 * BUFFER_BYTES + 11];
    let prepared = store
        .prepare(
            BoundMedia::open(&path).unwrap(),
            "flac",
            32,
            &mut sink,
            observer,
            |candidate| fs::write(candidate, &after),
        )
        .unwrap();
    prepared
        .publish(&mut sink, observer)
        .unwrap()
        .retire(|_| Ok(()), &mut sink, observer)
        .unwrap();
}
#[test]
fn process_kill_child() {
    let Ok(root) = std::env::var("TAGREX_MEDIA_KILL_ROOT") else {
        return;
    };
    let index = std::env::var("TAGREX_MEDIA_KILL_INDEX")
        .unwrap()
        .parse()
        .unwrap();
    kill_flow(
        Path::new(&root),
        &mut StopAt {
            index,
            count: 0,
            marker: Path::new(&root).join("ready"),
        },
    );
}
#[test]
fn actual_process_kill_at_every_copy_state_and_sync_boundary_retains_evidence_or_finished_cleanup()
{
    let _boundary = process_boundary();
    let original = vec![b'A'; 2 * BUFFER_BYTES + 3];
    let after = vec![b'B'; 2 * BUFFER_BYTES + 11];
    let baseline = Fixture::new();
    baseline.put("original.flac", &original);
    let mut trace = Trace { events: Vec::new() };
    kill_flow(&baseline.0, &mut trace);
    assert!(trace.events.len() > 40);
    println!("MEDIA_SIGKILL_BOUNDARIES={}", trace.events.len());
    for (index, event) in trace.events.iter().enumerate() {
        let f = Fixture::new();
        let path = f.put("original.flac", &original);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process_kill_child", "--nocapture"])
            .env("TAGREX_MEDIA_KILL_ROOT", &f.0)
            .env("TAGREX_MEDIA_KILL_INDEX", (index + 1).to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !f.0.join("ready").exists() {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("child exited {status} before boundary {index}: {event:?}");
            }
            assert!(Instant::now() < deadline, "child timeout at {index}");
            std::thread::sleep(Duration::from_millis(2));
        }
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        let current = fs::read(&path).unwrap();
        let store = f.store(limits(), budget());
        let records = store.records().unwrap();
        if let Some(record) = records.first() {
            if record.phase() < Phase::Publishing {
                assert_eq!(current, original, "boundary {index} {event:?}");
            }
            if record.phase() == Phase::Publishing || record.phase() == Phase::Published {
                assert_eq!(
                    fs::read(entry(&f.0.join("store"), record.id(), "preimage")).unwrap(),
                    original
                );
                assert_eq!(
                    fs::read(entry(&f.0.join("store"), record.id(), "candidate.flac")).unwrap(),
                    after
                );
                let class = store
                    .inspect(record.id(), &mut BoundMedia::open(&path).unwrap())
                    .unwrap();
                assert_eq!(
                    class,
                    if current == original {
                        Current::Preimage
                    } else if current == after {
                        Current::Candidate
                    } else {
                        Current::Ambiguous
                    }
                );
            }
            if record.phase() == Phase::Retired {
                assert_eq!(current, after);
                store
                    .finish_retirement(
                        record.id(),
                        &mut BoundMedia::open(&path).unwrap(),
                        |_| Ok(()),
                        &mut (),
                    )
                    .unwrap();
                assert_eq!(store.usage(), (0, 0));
            }
        } else {
            assert_eq!(current, after);
        }
    }
}

#[test]
#[ignore = "manual streamed media-store timing and resident-memory observation on disposable 1/16/32 MiB files"]
fn bounded_streaming_profile() {
    use std::io::{Seek, SeekFrom};
    struct Measure {
        max: usize,
        chunks: usize,
    }
    impl Observer for Measure {
        fn write_limit(&mut self, _: CopyKind, requested: usize) -> io::Result<usize> {
            self.max = self.max.max(requested);
            Ok(requested)
        }
        fn event(&mut self, event: Event) -> io::Result<()> {
            if matches!(event, Event::Copied(_, _)) {
                self.chunks += 1;
            }
            Ok(())
        }
    }
    let rss = || {
        fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    for mib in [1u64, 16, 32] {
        let f = Fixture::new();
        let path = f.0.join("stream.flac");
        let mut file = File::create(&path).unwrap();
        let block = [b'Q'; BUFFER_BYTES];
        for _ in 0..mib * 16 {
            file.write_all(&block).unwrap();
        }
        file.sync_all().unwrap();
        drop(file);
        let resource = budget();
        let store = f.store(
            Limits {
                store_bytes: 128 << 20,
                entries: 1,
                media_bytes: 64 << 20,
            },
            resource.clone(),
        );
        let baseline = resource.usage().0;
        let before_rss = rss();
        let started = Instant::now();
        let mut measured = Measure { max: 0, chunks: 0 };
        let prepared = store
            .prepare(
                BoundMedia::open(&path).unwrap(),
                "flac",
                0,
                &mut sink,
                &mut measured,
                |candidate| {
                    let mut file = OpenOptions::new().write(true).open(candidate)?;
                    file.seek(SeekFrom::Start(0))?;
                    file.write_all(b"PATCHED")
                },
            )
            .unwrap();
        let prepare_ms = started.elapsed().as_secs_f64() * 1000.;
        let publish = Instant::now();
        let published = prepared.publish(&mut sink, &mut measured).unwrap();
        let publish_ms = publish.elapsed().as_secs_f64() * 1000.;
        let retire = Instant::now();
        published
            .retire(|_| Ok(()), &mut sink, &mut measured)
            .unwrap();
        let retire_ms = retire.elapsed().as_secs_f64() * 1000.;
        let mut file = File::open(&path).unwrap();
        let mut prefix = [0; 7];
        file.read_exact(&mut prefix).unwrap();
        assert_eq!(&prefix, b"PATCHED");
        let mut bytes = [0; BUFFER_BYTES];
        let mut count = 7;
        loop {
            let n = file.read(&mut bytes).unwrap();
            if n == 0 {
                break;
            }
            assert!(bytes[..n].iter().all(|b| *b == b'Q'));
            count += n;
        }
        assert_eq!(count as u64, mib << 20);
        assert_eq!(measured.max, BUFFER_BYTES);
        assert_eq!(store.usage(), (0, 0));
        assert_eq!(resource.usage().0, baseline);
        println!("MEDIA_STREAM mib={mib} prepare_ms={prepare_ms:.3} publish_ms={publish_ms:.3} retire_ms={retire_ms:.3} chunks={} max_write={} before={before_rss:?} after={:?}",measured.chunks,measured.max,rss());
    }
}

#[test]
fn sealed_images_are_checked_at_every_publication_event_before_any_more_original_bytes_change() {
    struct Change {
        at: usize,
        count: usize,
        path: PathBuf,
        foreign: Vec<u8>,
    }
    impl Observer for Change {
        fn event(&mut self, _event: Event) -> io::Result<()> {
            self.count += 1;
            if self.count == self.at {
                fs::write(&self.path, &self.foreign)?;
            }
            Ok(())
        }
    }
    let original = vec![b'A'; 2 * BUFFER_BYTES + 7];
    let candidate = vec![b'B'; original.len()];
    let foreign = vec![b'C'; original.len()];
    let base = Fixture::new();
    let path = base.put("original.flac", &original);
    let store = base.store(limits(), budget());
    let mut trace = Trace { events: Vec::new() };
    drop(
        prepare(&store, &path, &candidate)
            .publish(&mut sink, &mut trace)
            .unwrap(),
    );
    for target in ["preimage", "candidate.flac"] {
        for index in 0..trace.events.len() {
            let f = Fixture::new();
            let path = f.put("original.flac", &original);
            let store = f.store(limits(), budget());
            let prepared = prepare(&store, &path, &candidate);
            let id = prepared.record().id().to_string();
            let artifact = entry(&f.0.join("store"), &id, target);
            let mut observer = Change {
                at: index + 1,
                count: 0,
                path: artifact.clone(),
                foreign: foreign.clone(),
            };
            let result = prepared.publish(&mut sink, &mut observer);
            assert!(
                result.is_err(),
                "{target}: {:?} falsely succeeded",
                trace.events[index]
            );
            assert!(observer.count >= observer.at);
            let written = trace.events[..=index]
                .iter()
                .filter_map(|event| {
                    if let Event::Copied(CopyKind::Publish, n) = event {
                        Some(*n as usize)
                    } else {
                        None
                    }
                })
                .max()
                .unwrap_or(0);
            let mut expected = original.clone();
            expected[..written].copy_from_slice(&candidate[..written]);
            assert_eq!(
                fs::read(&path).unwrap(),
                expected,
                "{target}: {:?} wrote after stale proof",
                trace.events[index]
            );
            assert_eq!(fs::read(&artifact).unwrap(), foreign);
            assert!(entry(&f.0.join("store"), &id, "preimage").exists());
            assert!(entry(&f.0.join("store"), &id, "candidate.flac").exists());
        }
    }
}

#[test]
fn sealed_image_changes_in_write_allowance_and_durable_acknowledgements_refuse() {
    struct Change(PathBuf);
    impl Observer for Change {
        fn write_limit(&mut self, kind: CopyKind, n: usize) -> io::Result<usize> {
            if kind == CopyKind::Publish {
                let old = fs::metadata(&self.0)?.modified()?;
                fs::write(&self.0, b"foreign!")?;
                File::options()
                    .write(true)
                    .open(&self.0)?
                    .set_times(fs::FileTimes::new().set_modified(old))?;
            }
            Ok(n)
        }
    }
    for target in ["preimage", "candidate.flac"] {
        let f = Fixture::new();
        let path = f.put("original.flac", b"original");
        let store = f.store(limits(), budget());
        let prepared = prepare(&store, &path, b"modified");
        let artifact = entry(&f.0.join("store"), prepared.record().id(), target);
        assert!(prepared.publish(&mut sink, &mut Change(artifact)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        for phase in [Phase::Publishing, Phase::Published] {
            let f = Fixture::new();
            let path = f.put("original.flac", b"original");
            let store = f.store(limits(), budget());
            let prepared = prepare(&store, &path, b"modified");
            let artifact = entry(&f.0.join("store"), prepared.record().id(), target);
            let mut ack = |record: &Record| {
                if record.phase() == phase {
                    fs::write(&artifact, b"foreign!")?;
                }
                Ok(())
            };
            assert!(prepared.publish(&mut ack, &mut ()).is_err());
            assert_eq!(
                fs::read(&path).unwrap(),
                if phase == Phase::Publishing {
                    b"original"
                } else {
                    b"modified"
                }
            );
            assert_eq!(fs::read(artifact).unwrap(), b"foreign!");
        }
    }
}

#[test]
fn retirement_acknowledgement_and_cleanup_callbacks_keep_remaining_image_proofs() {
    for target in ["preimage", "candidate.flac"] {
        let f = Fixture::new();
        let path = f.put("original.flac", b"original");
        let store = f.store(limits(), budget());
        let prepared = prepare(&store, &path, b"modified");
        let id = prepared.record().id().to_string();
        let artifact = entry(&f.0.join("store"), &id, target);
        let published = prepared.publish(&mut sink, &mut ()).unwrap();
        let mut ack = |record: &Record| {
            if record.phase() == Phase::Retired {
                fs::write(&artifact, b"foreign!")?;
            }
            Ok(())
        };
        assert!(published.retire(|_| Ok(()), &mut ack, &mut ()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"modified");
        assert_eq!(fs::read(artifact).unwrap(), b"foreign!");
        assert_eq!(store.usage().1, 1);
        struct Change {
            path: PathBuf,
            at: usize,
            count: usize,
            removed: bool,
        }
        impl Observer for Change {
            fn event(&mut self, event: Event) -> io::Result<()> {
                if event == Event::BeforeSync(SyncKind::Manifest(Phase::Retired)) {
                    self.count += 1;
                }
                if (!self.removed
                    && event == Event::BeforeSync(SyncKind::Manifest(Phase::Retired))
                    && self.count == self.at)
                    || (self.removed && event == Event::ArtifactRemoved(0))
                {
                    fs::write(&self.path, b"foreign!")?;
                }
                Ok(())
            }
        }
        for (at, removed) in [(1, false), (2, false), (0, true)] {
            let f = Fixture::new();
            let path = f.put("original.flac", b"original");
            let store = f.store(limits(), budget());
            let prepared = prepare(&store, &path, b"modified");
            let id = prepared.record().id().to_string();
            let published = prepared.publish(&mut sink, &mut ()).unwrap();
            let artifact = entry(&f.0.join("store"), &id, target);
            assert!(published
                .retire(
                    |_| Ok(()),
                    &mut sink,
                    &mut Change {
                        path: artifact.clone(),
                        at,
                        count: 0,
                        removed
                    }
                )
                .is_err());
            assert_eq!(fs::read(&path).unwrap(), b"modified");
            assert_eq!(fs::read(artifact).unwrap(), b"foreign!");
            assert!(entry(&f.0.join("store"), &id, "preimage").exists());
            assert!(entry(&f.0.join("store"), &id, "candidate.flac").exists());
        }
    }
}

#[test]
fn protected_artifact_child() {
    let Ok(root) = std::env::var("TAGREX_PROTECTED_ARTIFACT_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let path = root.join("hardlink.flac");
    let store = Store::open(&root.join("second-store"), limits(), budget()).unwrap();
    let result = store.prepare(
        BoundMedia::open(&path).unwrap(),
        "flac",
        0,
        &mut sink,
        &mut (),
        |_| panic!("protected image reached editing"),
    );
    assert!(result.is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[test]
fn recovery_object_identity_exclusion_survives_cross_store_aliases_hardlinks_and_processes() {
    let _boundary = process_boundary();
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let path = f.put("original.flac", b"original");
    let store = f.store(limits(), budget());
    let prepared = prepare(&store, &path, b"modified");
    let id = prepared.record().id().to_string();
    drop(prepared.publish(&mut sink, &mut ()).unwrap());
    let protected = entry(&f.0.join("store"), &id, "preimage");
    let alias = f.0.join("alias");
    symlink(f.0.join("store"), &alias).unwrap();
    let hardlink = f.0.join("hardlink.flac");
    fs::hard_link(&protected, &hardlink).unwrap();
    let other = Store::open(&f.0.join("second-store"), limits(), budget()).unwrap();
    for input in [&protected, &alias.join(&id).join("preimage"), &hardlink] {
        assert!(other
            .prepare(
                BoundMedia::open(input).unwrap(),
                "flac",
                0,
                &mut sink,
                &mut (),
                |_| panic!("protected image reached editing")
            )
            .is_err());
    }
    assert_eq!(other.usage(), (0, 0));
    drop(other);
    drop(store);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "protected_artifact_child", "--nocapture"])
        .env("TAGREX_PROTECTED_ARTIFACT_ROOT", &f.0)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read(&protected).unwrap(), b"original");
    assert_eq!(fs::read(&path).unwrap(), b"modified");
    fs::remove_file(hardlink).unwrap();
    assert_eq!(f.store(limits(), budget()).records().unwrap().len(), 1);
}
