//! Fresh public-API checks after the sealed-image/physical-alias corrections.
#[cfg(target_os = "linux")]
mod linux {
    use std::{
        fs::{self, File, FileTimes, OpenOptions},
        io,
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };
    use tagrex_core::{
        media_mutation::{BoundMedia, Current, Error, Limits, Phase, Record, Store},
        resources::{Budget, Limits as ResourceLimits},
    };

    const MARKER: &str = "user.tagrex.media_recovery_v1";
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tagrex-media-fresh-witness-{}-{}",
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
        fn store(&self) -> Store {
            Store::open(&self.0.join("store"), limits(), budget()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn budget() -> Budget {
        Budget::new(ResourceLimits {
            retained: 16 << 20,
            transient: 4 << 20,
            total: 20 << 20,
            jobs: 4,
        })
    }
    fn limits() -> Limits {
        Limits {
            store_bytes: 8 << 20,
            media_bytes: 2 << 20,
            entries: 8,
        }
    }
    fn sink(_: &Record) -> io::Result<()> {
        Ok(())
    }
    fn prepare(store: &Store, path: &Path, bytes: &[u8]) -> tagrex_core::media_mutation::Prepared {
        store
            .prepare(
                BoundMedia::open(path).unwrap(),
                "flac",
                bytes.len() as u64,
                &mut sink,
                &mut (),
                |candidate| fs::write(candidate, bytes),
            )
            .unwrap()
    }
    fn set_marker(path: &Path, bytes: &[u8]) {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        rustix::fs::fsetxattr(&file, MARKER, bytes, rustix::fs::XattrFlags::empty()).unwrap();
    }
    fn marker(path: &Path) -> Vec<u8> {
        let file = File::open(path).unwrap();
        let mut out = vec![0; 256];
        let n = rustix::fs::fgetxattr(&file, MARKER, &mut out).unwrap();
        out.truncate(n);
        out
    }

    #[test]
    fn malformed_or_copied_native_marker_never_admits_a_protected_inode() {
        let f = Fixture::new();
        let original = f.put("original.flac", b"original image");
        let store = f.store();
        let prepared = prepare(&store, &original, b"candidate image");
        let id = prepared.record().id().to_string();
        drop(prepared.publish(&mut sink, &mut ()).unwrap());
        let preimage = f.0.join("store").join(&id).join("preimage");
        let valid = marker(&preimage);
        let alias = f.0.join("outside-hardlink.flac");
        fs::hard_link(&preimage, &alias).unwrap();
        let other = Store::open(&f.0.join("other-store"), limits(), budget()).unwrap();
        let mut wrong_identity = valid.clone();
        *wrong_identity.last_mut().unwrap() ^= 1;
        for malformed in [
            Vec::new(),
            b"TRX-MEDIA-OBJECT\x02".to_vec(),
            wrong_identity,
            vec![b'X'; 256],
        ] {
            set_marker(&preimage, &malformed);
            let result = other.prepare(
                BoundMedia::open(&alias).unwrap(),
                "flac",
                64,
                &mut sink,
                &mut (),
                |_| panic!("invalid protected marker reached editor"),
            );
            assert!(result.is_err());
            assert_eq!(other.usage(), (0, 0));
            assert_eq!(fs::read(&preimage).unwrap(), b"original image");
            set_marker(&preimage, &valid);
        }
        let ordinary = f.put("copied-attribute.flac", b"ordinary image");
        set_marker(&ordinary, &valid);
        assert!(other
            .prepare(
                BoundMedia::open(&ordinary).unwrap(),
                "flac",
                64,
                &mut sink,
                &mut (),
                |_| panic!("copied native marker reached editor")
            )
            .is_err());
        assert_eq!(fs::read(&ordinary).unwrap(), b"ordinary image");
        fs::remove_file(alias).unwrap();
        assert_eq!(store.records().unwrap().len(), 1);
        assert_eq!(fs::read(original).unwrap(), b"candidate image");
    }

    #[test]
    fn moving_the_store_and_aliasing_its_parent_keeps_physical_exclusion_and_restart_proof() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let original = f.put("original.flac", b"original image");
        let ancestor = f.0.join("ancestor");
        fs::create_dir(&ancestor).unwrap();
        fs::create_dir(ancestor.join("nested")).unwrap();
        let store_path = ancestor.join("nested/store");
        let store = Store::open(&store_path, limits(), budget()).unwrap();
        let prepared = prepare(&store, &original, b"candidate image");
        let id = prepared.record().id().to_string();
        drop(prepared.publish(&mut sink, &mut ()).unwrap());
        let relocated = f.0.join("relocated");
        fs::rename(&ancestor, &relocated).unwrap();
        assert!(store.records().is_err());
        drop(store);
        let alias = f.0.join("parent-alias");
        symlink(&relocated, &alias).unwrap();
        let through_alias = alias.join("nested/store");
        let reopened = Store::open(&through_alias, limits(), budget()).unwrap();
        let other = f.store();
        let protected = through_alias.join(&id).join("candidate.flac");
        assert!(other
            .prepare(
                BoundMedia::open(&protected).unwrap(),
                "flac",
                0,
                &mut sink,
                &mut (),
                |_| panic!("relocated artifact reached editing")
            )
            .is_err());
        let record = reopened.records().unwrap().remove(0);
        assert_eq!(record.id(), id);
        assert_eq!(
            reopened
                .inspect(&id, &mut BoundMedia::open(&original).unwrap())
                .unwrap(),
            Current::Candidate
        );
        reopened
            .resume_published(&id, BoundMedia::open(&original).unwrap())
            .unwrap()
            .retire(|_| Ok(()), &mut sink, &mut ())
            .unwrap();
        assert_eq!(reopened.usage(), (0, 0));
        assert_eq!(fs::read(&original).unwrap(), b"candidate image");
    }

    #[test]
    fn publishing_acknowledgement_error_or_panic_preserves_unwritten_original_and_both_images() {
        for panic_instead in [false, true] {
            let f = Fixture::new();
            let before = b"old image";
            let after = b"new image";
            let original = f.put("original.flac", before);
            let store = f.store();
            let prepared = prepare(&store, &original, after);
            let id = prepared.record().id().to_string();
            let mut acknowledger = |record: &Record| -> io::Result<()> {
                assert_eq!(record.phase(), Phase::Publishing);
                assert_eq!(store.records().unwrap()[0].phase(), Phase::Publishing);
                if panic_instead {
                    panic!("owned fixture durable-ack panic");
                }
                Err(io::Error::other("durable acknowledgement uncertain"))
            };
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                prepared.publish(&mut acknowledger, &mut ())
            }));
            if panic_instead {
                assert!(outcome.is_err());
            } else {
                assert!(outcome.unwrap().is_err());
            }
            assert_eq!(fs::read(&original).unwrap(), before);
            assert_eq!(
                fs::read(f.0.join("store").join(&id).join("preimage")).unwrap(),
                before
            );
            assert_eq!(
                fs::read(f.0.join("store").join(&id).join("candidate.flac")).unwrap(),
                after
            );
            assert_eq!(
                store
                    .inspect(&id, &mut BoundMedia::open(&original).unwrap())
                    .unwrap(),
                Current::Preimage
            );
            assert!(store
                .resume_published(&id, BoundMedia::open(&original).unwrap())
                .is_err());
            drop(store);
            let reopened = f.store();
            assert_eq!(reopened.records().unwrap()[0].phase(), Phase::Publishing);
            assert_eq!(
                reopened
                    .inspect(&id, &mut BoundMedia::open(&original).unwrap())
                    .unwrap(),
                Current::Preimage
            );
        }
    }

    #[test]
    fn a_same_length_restored_mtime_source_change_in_a_failing_ack_is_never_endorsed() {
        let f = Fixture::new();
        let original = f.put("original.flac", b"old image");
        let store = f.store();
        let prepared = prepare(&store, &original, b"new image");
        let id = prepared.record().id().to_string();
        let modified = fs::metadata(&original).unwrap().modified().unwrap();
        let mut acknowledger = |_: &Record| -> io::Result<()> {
            fs::write(&original, b"foreign!!")?;
            OpenOptions::new()
                .write(true)
                .open(&original)?
                .set_times(FileTimes::new().set_modified(modified))?;
            Err(io::Error::other("ack failed after external fixture edit"))
        };
        assert!(prepared.publish(&mut acknowledger, &mut ()).is_err());
        assert_eq!(fs::read(&original).unwrap(), b"foreign!!");
        assert_eq!(
            fs::read(f.0.join("store").join(&id).join("preimage")).unwrap(),
            b"old image"
        );
        assert_eq!(
            store
                .inspect(&id, &mut BoundMedia::open(&original).unwrap())
                .unwrap(),
            Current::Ambiguous
        );
        assert_eq!(store.records().unwrap().len(), 1);
    }

    #[test]
    fn corrupted_retirement_tombstone_is_revalidated_before_any_cleanup_mutation() {
        use tagrex_core::media_mutation::{Event, Observer, SyncKind};
        struct Corrupt {
            root: PathBuf,
            id: String,
            manifests: usize,
            changed: bool,
            removed: bool,
        }
        impl Observer for Corrupt {
            fn event(&mut self, event: Event) -> io::Result<()> {
                if event == Event::BeforeSync(SyncKind::Manifest(Phase::Retired)) {
                    self.manifests += 1;
                    if self.manifests == 2 {
                        let terminal = self.root.join(format!("t-{}", &self.id[2..]));
                        let mut bytes = fs::read(&terminal)?;
                        *bytes.last_mut().unwrap() ^= 1;
                        fs::write(terminal, bytes)?;
                        self.changed = true;
                    }
                }
                if matches!(event, Event::ArtifactRemoved(_)) {
                    self.removed = true;
                    return Err(io::Error::other("stop at first cleanup mutation"));
                }
                Ok(())
            }
        }
        let f = Fixture::new();
        let original = f.put("original.flac", b"old image");
        let store = f.store();
        let published = prepare(&store, &original, b"new image")
            .publish(&mut sink, &mut ())
            .unwrap();
        let id = published.record().id().to_string();
        let mut observer = Corrupt {
            root: f.0.join("store"),
            id: id.clone(),
            manifests: 0,
            changed: false,
            removed: false,
        };
        let result = published.retire(|_| Ok(()), &mut sink, &mut observer);
        assert!(
            observer.changed,
            "must corrupt the root tombstone after its bytes were written"
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&original).unwrap(), b"new image");
        assert_eq!(
            fs::read(f.0.join("store").join(&id).join("preimage")).unwrap(),
            b"old image"
        );
        assert!(
            !observer.removed,
            "cleanup began using an already-corrupted durable retirement proof: {result:?}"
        );
        assert!(f.0.join("store").join(&id).join("0-preparing").exists());
    }

    #[test]
    fn independent_lock_child() {
        let Ok(root) = std::env::var("TAGREX_FRESH_MEDIA_LOCK_ROOT") else {
            return;
        };
        let root = Path::new(&root);
        let locked = std::env::var("TAGREX_FRESH_MEDIA_LOCK_STATE").unwrap() == "locked";
        if locked {
            assert!(matches!(
                BoundMedia::open(&root.join("original.flac")),
                Err(Error::Capacity)
            ));
            assert!(matches!(
                Store::open(&root.join("store"), limits(), budget()),
                Err(Error::Capacity)
            ));
        } else {
            let store = Store::open(&root.join("store"), limits(), budget()).unwrap();
            let records = store.records().unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(
                store
                    .inspect(
                        records[0].id(),
                        &mut BoundMedia::open(&root.join("original.flac")).unwrap()
                    )
                    .unwrap(),
                Current::Candidate
            );
        }
    }
    #[test]
    fn original_and_store_locks_are_process_owned_and_release_after_published_handle_drop() {
        let f = Fixture::new();
        let original = f.put("original.flac", b"old image");
        let store = f.store();
        let published = prepare(&store, &original, b"new image")
            .publish(&mut sink, &mut ())
            .unwrap();
        let run = |state: &str| {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "linux::independent_lock_child", "--nocapture"])
                .env("TAGREX_FRESH_MEDIA_LOCK_ROOT", &f.0)
                .env("TAGREX_FRESH_MEDIA_LOCK_STATE", state)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run("locked");
        drop(store);
        run("locked");
        drop(published);
        run("released");
        assert_eq!(fs::read(&original).unwrap(), b"new image");
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn unsupported_platform_refuses_the_opt_in_store_without_publication() {
    use tagrex_core::{
        media_mutation::{Error, Limits, Store},
        resources::Budget,
    };
    assert!(matches!(
        Store::open(
            std::path::Path::new("."),
            Limits::default(),
            Budget::process()
        ),
        Err(Error::Unsupported)
    ));
}
