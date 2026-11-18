//! Independent public-caller probes; every original/artifact belongs to this fixture.
#[cfg(target_os = "linux")]
mod linux {
    use std::{
        fs, io,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };
    use tagrex_core::{
        media_mutation::{BoundMedia, CopyKind, Event, Limits, Observer, Phase, Record, Store},
        resources::{Budget, Charge, Limits as ResourceLimits},
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tagrex-media-independent-{}-{}",
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
        fn store(&self, budget: Budget) -> Store {
            Store::open(
                &self.0.join("store"),
                Limits {
                    store_bytes: 4 << 20,
                    entries: 8,
                    media_bytes: 1 << 20,
                },
                budget,
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn budget() -> Budget {
        Budget::new(ResourceLimits {
            retained: 8 << 20,
            transient: 4 << 20,
            total: 12 << 20,
            jobs: 8,
        })
    }
    fn sink(_: &Record) -> io::Result<()> {
        Ok(())
    }
    fn prepared(store: &Store, original: &Path) -> tagrex_core::media_mutation::Prepared {
        store
            .prepare(
                BoundMedia::open(original).unwrap(),
                "flac",
                64,
                &mut sink,
                &mut (),
                |candidate| fs::write(candidate, b"candidate image"),
            )
            .unwrap()
    }
    struct ChangeAtPublication {
        path: PathBuf,
        fired: bool,
    }
    impl Observer for ChangeAtPublication {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if event == Event::CopyStarted(CopyKind::Publish) {
                self.fired = true;
                // Models an independently changed artifact at the public
                // boundary; does not touch the original or keep an edit handle.
                fs::write(&self.path, b"foreign image!!")?;
            }
            Ok(())
        }
    }
    #[test]
    fn changed_candidate_at_publish_start_refuses_before_overwriting_the_original() {
        let f = Fixture::new();
        let original = f.put("original.flac", b"original image");
        let store = f.store(budget());
        let prepared = prepared(&store, &original);
        let id = prepared.record().id().to_string();
        let candidate = f.0.join("store").join(&id).join("candidate.flac");
        let mut observer = ChangeAtPublication {
            path: candidate.clone(),
            fired: false,
        };
        let result = prepared.publish(&mut sink, &mut observer);
        assert!(observer.fired);
        assert!(result.is_err());
        assert_eq!(fs::read(&candidate).unwrap(), b"foreign image!!");
        assert_eq!(
            fs::read(&original).unwrap(),
            b"original image",
            "an already-changed candidate must not be copied into the original before refusal"
        );
    }
    #[test]
    fn changed_preimage_at_publish_start_refuses_before_destroying_the_last_original_image() {
        let f = Fixture::new();
        let original = f.put("original.flac", b"original image");
        let store = f.store(budget());
        let prepared = prepared(&store, &original);
        let id = prepared.record().id().to_string();
        let preimage = f.0.join("store").join(&id).join("preimage");
        let mut observer = ChangeAtPublication {
            path: preimage.clone(),
            fired: false,
        };
        let result = prepared.publish(&mut sink, &mut observer);
        assert!(observer.fired);
        assert!(result.is_err());
        assert_eq!(fs::read(&preimage).unwrap(), b"foreign image!!");
        assert_eq!(
            fs::read(&original).unwrap(),
            b"original image",
            "a changed sole preimage must be caught before the original is overwritten"
        );
    }
    #[test]
    fn a_recovery_artifact_cannot_be_adopted_as_new_media_through_a_parent_alias() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let original = f.put("original.flac", b"original image");
        let store = f.store(budget());
        let published = prepared(&store, &original)
            .publish(&mut sink, &mut ())
            .unwrap();
        let id = published.record().id().to_string();
        drop(published);
        let alias = f.0.join("alias");
        symlink(f.0.join("store"), &alias).unwrap();
        let protected = f.0.join("store").join(&id).join("preimage");
        let capability = BoundMedia::open(&alias.join(&id).join("preimage")).unwrap();
        let result = store.prepare(capability, "flac", 0, &mut sink, &mut (), |candidate| {
            fs::write(candidate, b"replacement")
        });
        assert!(result.is_err(), "recovery artifact exclusion must follow held directory identity rather than only the logical path prefix");
        assert_eq!(fs::read(&protected).unwrap(), b"original image");
        assert_eq!(fs::read(&original).unwrap(), b"candidate image");
        assert_eq!(store.records().unwrap().len(), 1);
    }
    #[test]
    fn aliased_artifact_publication_must_not_overwrite_an_unretired_preimage() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let original = f.put("original.flac", b"original image");
        let store = f.store(budget());
        let published = prepared(&store, &original)
            .publish(&mut sink, &mut ())
            .unwrap();
        let id = published.record().id().to_string();
        drop(published); // Publication is deliberately not retired/confirmed.
        let alias = f.0.join("alias");
        symlink(f.0.join("store"), &alias).unwrap();
        let protected = f.0.join("store").join(&id).join("preimage");
        let aliased = alias.join(&id).join("preimage");
        let result = store.prepare(
            BoundMedia::open(&aliased).unwrap(),
            "flac",
            0,
            &mut sink,
            &mut (),
            |candidate| fs::write(candidate, b"replacement"),
        );
        let adopted = result.is_ok();
        if let Ok(prepared) = result {
            // Demonstrate the actual integrity effect if preparation was
            // admitted. No fixture path leaves this test's owned directory.
            let published = prepared.publish(&mut sink, &mut ()).unwrap();
            drop(published);
        }
        assert_eq!(fs::read(&original).unwrap(), b"candidate image");
        assert_eq!(fs::read(&protected).unwrap(), b"original image",
            "a second operation used a parent alias to overwrite the only old preimage of an unretired publication");
        assert!(
            !adopted,
            "recovery artifacts must not be admitted through aliases"
        );
    }
    #[test]
    fn reentrant_state_sink_can_inspect_and_prepare_another_file_without_deadlock_or_leaking_reservations(
    ) {
        let f = Fixture::new();
        let original = f.put("original.flac", b"original image");
        let other = f.put("other.flac", b"other image");
        let budget = budget();
        let store = f.store(budget.clone());
        let baseline = budget.usage().0;
        let mut calls = 0;
        let mut state = |record: &Record| {
            let records = store.records().unwrap();
            assert!(records
                .iter()
                .any(|stored| stored.id() == record.id() && stored.phase() == record.phase()));
            if record.phase() == Phase::Prepared {
                let nested = store
                    .prepare(
                        BoundMedia::open(&other).unwrap(),
                        "flac",
                        0,
                        &mut sink,
                        &mut (),
                        |_| Ok(()),
                    )
                    .unwrap();
                assert_eq!(store.usage().1, 2);
                drop(nested);
                assert_eq!(store.usage().1, 1);
            }
            calls += 1;
            Ok(())
        };
        let result = store
            .prepare(
                BoundMedia::open(&original).unwrap(),
                "flac",
                64,
                &mut state,
                &mut (),
                |candidate| fs::write(candidate, b"candidate image"),
            )
            .unwrap()
            .publish(&mut state, &mut ())
            .unwrap();
        result.retire(|_| Ok(()), &mut state, &mut ()).unwrap();
        assert_eq!(calls, 4);
        assert_eq!(store.usage(), (0, 0));
        assert_eq!(budget.usage().0, baseline);
        drop(store);
        assert_eq!(budget.usage().0, Charge::default());
        assert_eq!(fs::read(&other).unwrap(), b"other image");
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn non_linux_store_remains_an_explicit_opt_out() {
    use tagrex_core::{
        media_mutation::{Error, Limits, Store},
        resources::Budget,
    };
    const { assert!(!tagrex_core::media_mutation::SUPPORTED) };
    assert!(matches!(
        Store::open(
            std::path::Path::new("."),
            Limits::default(),
            Budget::process()
        ),
        Err(Error::Unsupported)
    ));
}
