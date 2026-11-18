//! Independent public-API retirement-proof fault sweeps; Linux disposable fixtures only.
#[cfg(target_os = "linux")]
mod linux {
    use std::{
        collections::BTreeMap,
        fs::{self, File, FileTimes, OpenOptions},
        io,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };
    use tagrex_core::{
        media_mutation::{BoundMedia, Event, Observer, Published, Record, Store},
        resources::{Budget, Charge, Limits as ResourceLimits},
    };

    const MARKER: &str = "user.tagrex.media_recovery_v1";
    const BEFORE: &[u8] = b"old fixture image";
    const AFTER: &[u8] = b"new fixture image";
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tagrex-terminal-recheck-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::write(path.join("original.flac"), BEFORE).unwrap();
            Self(path)
        }
        fn original(&self) -> PathBuf {
            self.0.join("original.flac")
        }
        fn store(&self, budget: Budget) -> Store {
            Store::open(
                &self.0.join("store"),
                tagrex_core::media_mutation::Limits {
                    store_bytes: 1 << 20,
                    media_bytes: 1 << 16,
                    entries: 1,
                },
                budget,
            )
            .unwrap()
        }
        fn publish(&self, store: &Store) -> Published {
            store
                .prepare(
                    BoundMedia::open(&self.original()).unwrap(),
                    "flac",
                    0,
                    &mut sink,
                    &mut (),
                    |path| fs::write(path, AFTER),
                )
                .unwrap()
                .publish(&mut sink, &mut ())
                .unwrap()
        }
        fn terminal(&self, id: &str) -> PathBuf {
            self.0.join("store").join(format!("t-{}", &id[2..]))
        }
        fn entry(&self, id: &str) -> PathBuf {
            self.0.join("store").join(id)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn budget() -> Budget {
        Budget::new(ResourceLimits {
            retained: 1 << 20,
            transient: 1 << 20,
            total: 2 << 20,
            jobs: 2,
        })
    }
    fn sink(_: &Record) -> io::Result<()> {
        Ok(())
    }
    fn inventory(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        match fs::read_dir(dir) {
            Ok(entries) => entries
                .map(|entry| {
                    let entry = entry.unwrap();
                    (
                        entry.file_name().into_string().unwrap(),
                        fs::read(entry.path()).unwrap(),
                    )
                })
                .collect(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => panic!("fixture inventory: {error}"),
        }
    }
    struct Trace<'a> {
        terminal: &'a Path,
        events: Vec<(Event, bool)>,
    }
    impl Observer for Trace<'_> {
        fn event(&mut self, event: Event) -> io::Result<()> {
            self.events.push((event, self.terminal.exists()));
            Ok(())
        }
    }
    fn retirement_trace(resume: bool) -> Vec<(Event, bool)> {
        let f = Fixture::new();
        let store = f.store(budget());
        let published = f.publish(&store);
        let id = published.record().id().to_string();
        let terminal = f.terminal(&id);
        let mut trace = Trace {
            terminal: &terminal,
            events: Vec::new(),
        };
        if resume {
            leave_partial(published);
            store
                .finish_retirement(
                    &id,
                    &mut BoundMedia::open(&f.original()).unwrap(),
                    |_| Ok(()),
                    &mut trace,
                )
                .unwrap();
        } else {
            published.retire(|_| Ok(()), &mut sink, &mut trace).unwrap();
        }
        assert!(!terminal.exists());
        assert_eq!(store.usage(), (0, 0));
        trace.events
    }
    struct StopAfterRemoval;
    impl Observer for StopAfterRemoval {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if matches!(event, Event::ArtifactRemoved(_)) {
                Err(io::Error::other("interrupt after first owned unlink"))
            } else {
                Ok(())
            }
        }
    }
    fn leave_partial(published: Published) {
        assert!(published
            .retire(|_| Ok(()), &mut sink, &mut StopAfterRemoval)
            .is_err());
    }

    #[derive(Clone, Copy, Debug)]
    enum Damage {
        Checksum,
        RestoredMtime,
        Truncate,
        Permission,
        Marker,
        Replacement,
        Hardlink,
    }
    fn damage(path: &Path, kind: Damage, root: &Path) {
        match kind {
            Damage::Checksum | Damage::RestoredMtime => {
                let modified = fs::metadata(path).unwrap().modified().unwrap();
                let mut bytes = fs::read(path).unwrap();
                *bytes.last_mut().unwrap() ^= 1;
                fs::write(path, bytes).unwrap();
                if matches!(kind, Damage::RestoredMtime) {
                    OpenOptions::new()
                        .write(true)
                        .open(path)
                        .unwrap()
                        .set_times(FileTimes::new().set_modified(modified))
                        .unwrap();
                }
            }
            Damage::Truncate => fs::write(path, []).unwrap(),
            Damage::Permission => {
                fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
            }
            Damage::Marker => {
                rustix::fs::fremovexattr(File::open(path).unwrap(), MARKER).unwrap();
            }
            Damage::Hardlink => fs::hard_link(path, root.join("foreign-hardlink")).unwrap(),
            Damage::Replacement => {
                let bytes = fs::read(path).unwrap();
                fs::rename(path, root.join("displaced-terminal")).unwrap();
                fs::write(path, bytes).unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
                // Give the replacement a *correct* native marker, so the live
                // refusal cannot depend solely on an absent/copied marker.
                let file = File::open(path).unwrap();
                let m = file.metadata().unwrap();
                let birth = m
                    .created()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let mut marker = b"TRX-MEDIA-OBJECT\x01".to_vec();
                marker.extend(m.dev().to_le_bytes());
                marker.extend(m.ino().to_le_bytes());
                marker.extend(birth.to_le_bytes());
                rustix::fs::fsetxattr(&file, MARKER, &marker, rustix::fs::XattrFlags::CREATE)
                    .unwrap();
            }
        }
    }
    struct Disrupt<'a> {
        fixture: &'a Fixture,
        id: &'a str,
        target: usize,
        seen: usize,
        damage: Option<Damage>,
        snapshot: Option<BTreeMap<String, Vec<u8>>>,
        terminal_bytes: Option<Vec<u8>>,
    }
    impl Observer for Disrupt<'_> {
        fn event(&mut self, _: Event) -> io::Result<()> {
            let index = self.seen;
            self.seen += 1;
            if index == self.target {
                let terminal = self.fixture.terminal(self.id);
                if let Some(kind) = self.damage {
                    assert!(terminal.exists());
                    damage(&terminal, kind, &self.fixture.0);
                }
                self.snapshot = Some(inventory(&self.fixture.entry(self.id)));
                self.terminal_bytes = fs::read(terminal).ok();
                if self.damage.is_none() {
                    return Err(io::Error::other("independent retirement callback failure"));
                }
            }
            Ok(())
        }
    }

    #[test]
    fn changed_terminal_stops_at_every_creation_sync_and_cleanup_callback() {
        let mut cases = 0;
        for resume in [false, true] {
            let trace = retirement_trace(resume);
            for (target, (event, terminal_exists)) in trace.iter().enumerate() {
                if !terminal_exists {
                    continue;
                }
                for kind in [
                    Damage::Checksum,
                    Damage::RestoredMtime,
                    Damage::Truncate,
                    Damage::Permission,
                    Damage::Marker,
                    Damage::Replacement,
                    Damage::Hardlink,
                ] {
                    let f = Fixture::new();
                    let store = f.store(budget());
                    let published = f.publish(&store);
                    let id = published.record().id().to_string();
                    let charged = store.usage();
                    let mut observer = Disrupt {
                        fixture: &f,
                        id: &id,
                        target,
                        seen: 0,
                        damage: Some(kind),
                        snapshot: None,
                        terminal_bytes: None,
                    };
                    let outcome = if resume {
                        leave_partial(published);
                        store.finish_retirement(
                            &id,
                            &mut BoundMedia::open(&f.original()).unwrap(),
                            |_| Ok(()),
                            &mut observer,
                        )
                    } else {
                        published.retire(|_| Ok(()), &mut sink, &mut observer)
                    };
                    assert!(outcome.is_err(), "resume={resume} {event:?} {kind:?}");
                    assert_eq!(observer.seen, target + 1, "must stop immediately");
                    assert_eq!(
                        inventory(&f.entry(&id)),
                        observer.snapshot.unwrap(),
                        "cleanup after invalid proof: resume={resume} {event:?} {kind:?}"
                    );
                    assert_eq!(fs::read(f.terminal(&id)).ok(), observer.terminal_bytes);
                    assert_eq!(fs::read(f.original()).unwrap(), AFTER);
                    assert_eq!(store.usage(), charged);
                    cases += 1;
                }
            }
        }
        eprintln!("terminal mutation cases: {cases}");
    }

    #[test]
    fn each_callback_error_retains_current_evidence_then_reopens_and_finishes() {
        let mut cases = 0;
        for resume in [false, true] {
            for target in 0..retirement_trace(resume).len() {
                let f = Fixture::new();
                let store = f.store(budget());
                let published = f.publish(&store);
                let id = published.record().id().to_string();
                let charged = store.usage();
                let mut observer = Disrupt {
                    fixture: &f,
                    id: &id,
                    target,
                    seen: 0,
                    damage: None,
                    snapshot: None,
                    terminal_bytes: None,
                };
                let outcome = if resume {
                    leave_partial(published);
                    store.finish_retirement(
                        &id,
                        &mut BoundMedia::open(&f.original()).unwrap(),
                        |_| Ok(()),
                        &mut observer,
                    )
                } else {
                    published.retire(|_| Ok(()), &mut sink, &mut observer)
                };
                assert!(outcome.is_err());
                assert_eq!(observer.seen, target + 1);
                assert_eq!(inventory(&f.entry(&id)), observer.snapshot.unwrap());
                assert_eq!(fs::read(f.terminal(&id)).ok(), observer.terminal_bytes);
                assert_eq!(store.usage(), charged, "uncertain sync must retain charge");
                drop(store);
                let reopened = f.store(budget());
                if f.entry(&id).exists() || f.terminal(&id).exists() {
                    assert_eq!(reopened.usage(), charged);
                    let record = reopened.records().unwrap().remove(0);
                    if record.phase() == tagrex_core::media_mutation::Phase::Published {
                        reopened
                            .resume_published(&id, BoundMedia::open(&f.original()).unwrap())
                            .unwrap()
                            .retire(|_| Ok(()), &mut sink, &mut ())
                            .unwrap();
                    } else {
                        reopened
                            .finish_retirement(
                                &id,
                                &mut BoundMedia::open(&f.original()).unwrap(),
                                |_| Ok(()),
                                &mut (),
                            )
                            .unwrap();
                    }
                }
                assert_eq!(reopened.usage(), (0, 0));
                assert_eq!(fs::read(f.original()).unwrap(), AFTER);
                cases += 1;
            }
        }
        eprintln!("callback error/reopen cases: {cases}");
    }

    #[test]
    fn existing_terminal_confirmation_error_or_changed_proof_never_starts_next_unlink() {
        for corrupt in [false, true] {
            let f = Fixture::new();
            let store = f.store(budget());
            let published = f.publish(&store);
            let id = published.record().id().to_string();
            leave_partial(published);
            drop(store);
            let store = f.store(budget());
            let before = inventory(&f.entry(&id));
            let result = store.finish_retirement(
                &id,
                &mut BoundMedia::open(&f.original()).unwrap(),
                |_| {
                    if corrupt {
                        damage(&f.terminal(&id), Damage::RestoredMtime, &f.0);
                        Ok(())
                    } else {
                        Err(io::Error::other("journal finish is unconfirmed"))
                    }
                },
                &mut (),
            );
            assert!(result.is_err());
            assert_eq!(inventory(&f.entry(&id)), before);
            assert!(f.terminal(&id).exists());
            assert_eq!(fs::read(f.original()).unwrap(), AFTER);
        }
    }

    #[test]
    fn malformed_terminal_reopen_refuses_without_deleting_partial_evidence() {
        for kind in [
            Damage::Checksum,
            Damage::Truncate,
            Damage::Permission,
            Damage::Marker,
            Damage::Hardlink,
        ] {
            let f = Fixture::new();
            let store = f.store(budget());
            let published = f.publish(&store);
            let id = published.record().id().to_string();
            leave_partial(published);
            drop(store);
            damage(&f.terminal(&id), kind, &f.0);
            let before = inventory(&f.entry(&id));
            let terminal = fs::read(f.terminal(&id)).unwrap();
            let reopened = Store::open(
                &f.0.join("store"),
                tagrex_core::media_mutation::Limits {
                    store_bytes: 1 << 20,
                    media_bytes: 1 << 16,
                    entries: 1,
                },
                budget(),
            );
            assert!(reopened.is_err(), "malformed terminal admitted: {kind:?}");
            assert_eq!(inventory(&f.entry(&id)), before);
            assert_eq!(fs::read(f.terminal(&id)).unwrap(), terminal);
            assert_eq!(fs::read(f.original()).unwrap(), AFTER);
        }
    }

    #[test]
    fn panics_at_terminal_start_partial_cleanup_and_final_sync_preserve_restart_proof() {
        struct PanicOnError<'a, 'b>(&'a mut Disrupt<'b>);
        impl Observer for PanicOnError<'_, '_> {
            fn event(&mut self, event: Event) -> io::Result<()> {
                if let Err(error) = self.0.event(event) {
                    panic!("owned terminal fixture panic: {error}");
                }
                Ok(())
            }
        }
        let mut cases = 0;
        for resume in [false, true] {
            let trace = retirement_trace(resume);
            let mut targets = vec![
                trace.iter().position(|(_, exists)| *exists).unwrap(),
                trace
                    .iter()
                    .position(|(event, _)| matches!(event, Event::ArtifactRemoved(_)))
                    .unwrap(),
                trace.iter().rposition(|(_, exists)| *exists).unwrap(),
                trace.len() - 1,
            ];
            targets.sort_unstable();
            targets.dedup();
            for target in targets {
                let f = Fixture::new();
                let store = f.store(budget());
                let mut published = Some(f.publish(&store));
                let id = published.as_ref().unwrap().record().id().to_string();
                if resume {
                    leave_partial(published.take().unwrap());
                }
                let mut observer = Disrupt {
                    fixture: &f,
                    id: &id,
                    target,
                    seen: 0,
                    damage: None,
                    snapshot: None,
                    terminal_bytes: None,
                };
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if resume {
                        store.finish_retirement(
                            &id,
                            &mut BoundMedia::open(&f.original()).unwrap(),
                            |_| Ok(()),
                            &mut PanicOnError(&mut observer),
                        )
                    } else {
                        published.take().unwrap().retire(
                            |_| Ok(()),
                            &mut sink,
                            &mut PanicOnError(&mut observer),
                        )
                    }
                }));
                assert!(outcome.is_err());
                assert_eq!(inventory(&f.entry(&id)), observer.snapshot.unwrap());
                assert_eq!(fs::read(f.terminal(&id)).ok(), observer.terminal_bytes);
                drop(store);
                let reopened = f.store(budget());
                if f.terminal(&id).exists() {
                    reopened
                        .finish_retirement(
                            &id,
                            &mut BoundMedia::open(&f.original()).unwrap(),
                            |_| Ok(()),
                            &mut (),
                        )
                        .unwrap();
                }
                assert_eq!(reopened.usage(), (0, 0));
                assert_eq!(fs::read(f.original()).unwrap(), AFTER);
                cases += 1;
            }
        }
        eprintln!("panic/reopen cases: {cases}");
    }

    #[test]
    fn partial_retirement_keeps_capacity_and_failed_working_admission_changes_nothing() {
        let f = Fixture::new();
        let budget = budget();
        let store = f.store(budget.clone());
        let published = f.publish(&store);
        let id = published.record().id().to_string();
        leave_partial(published);
        let charged = store.usage();
        let before = inventory(&f.entry(&id));
        let terminal = fs::read(f.terminal(&id)).unwrap();
        let held = budget
            .try_acquire(Charge {
                transient: 1 << 20,
                ..Default::default()
            })
            .unwrap();
        assert!(store
            .finish_retirement(
                &id,
                &mut BoundMedia::open(&f.original()).unwrap(),
                |_| panic!("unadmitted cleanup reached confirmation"),
                &mut (),
            )
            .is_err());
        assert_eq!(inventory(&f.entry(&id)), before);
        assert_eq!(fs::read(f.terminal(&id)).unwrap(), terminal);
        assert_eq!(store.usage(), charged);
        drop(held);
        let other = f.0.join("other.flac");
        fs::write(&other, BEFORE).unwrap();
        assert!(store
            .prepare(
                BoundMedia::open(&other).unwrap(),
                "flac",
                0,
                &mut sink,
                &mut (),
                |_| panic!("retired entry capacity reused before cleanup"),
            )
            .is_err());
        store
            .finish_retirement(
                &id,
                &mut BoundMedia::open(&f.original()).unwrap(),
                |_| Ok(()),
                &mut (),
            )
            .unwrap();
        assert_eq!(store.usage(), (0, 0));
        drop(f.publish(&store));
        assert_eq!(store.usage().1, 1);
    }
}
