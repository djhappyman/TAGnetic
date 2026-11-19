//! Independent held-editor and acknowledgement interleavings; disposable Linux files only.
#![cfg(target_os = "linux")]
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tagrex_core::{
    media_mutation::{
        self as media, BoundMedia, Current, Event, Limits, Observer, Phase, Record, StateSink,
        Store,
    },
    model::{
        exact::{ExactTextState, NativeTextTarget},
        native_patch::{self, codec::EncodedReceipt, Receipt},
        TagEngine,
    },
    resources::{Budget, Limits as ResourceLimits},
};
const BEFORE: &[u8] = b"original";
const AFTER: &[u8] = b"edited!!";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-held-independent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn source(&self) -> PathBuf {
        self.0.join("source.flac")
    }
    fn store_path(&self) -> PathBuf {
        self.0.join("store")
    }
    fn store(&self, budget: Budget) -> Store {
        Store::open(&self.store_path(), limits(), budget).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn limits() -> Limits {
    Limits {
        store_bytes: 16 << 20,
        media_bytes: 4 << 20,
        entries: 8,
    }
}
fn budget() -> Budget {
    Budget::new(ResourceLimits {
        retained: 32 << 20,
        transient: 192 << 20,
        total: 224 << 20,
        jobs: 4,
    })
}
fn replace(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    file.write_all(bytes)?;
    file.set_len(bytes.len() as u64)
}
fn candidate(root: &Path) -> PathBuf {
    fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .flat_map(|p| fs::read_dir(p).unwrap())
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("candidate.")
        })
        .unwrap()
}
struct Ack {
    file: File,
    pending: Option<EncodedReceipt>,
    stop: Option<Phase>,
    phases: Vec<Phase>,
}
impl Ack {
    fn new(path: &Path) -> Self {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.sync_all().unwrap();
        File::open(path.parent().unwrap())
            .unwrap()
            .sync_all()
            .unwrap();
        Self {
            file,
            pending: None,
            stop: None,
            phases: Vec::new(),
        }
    }
    fn reopen(path: &Path, pending: EncodedReceipt) -> Self {
        Self {
            file: OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap(),
            pending: Some(pending),
            stop: None,
            phases: Vec::new(),
        }
    }
}
impl StateSink for Ack {
    fn persist(&mut self, record: &Record) -> io::Result<()> {
        let after = record
            .after()
            .ok_or_else(|| io::Error::other("unprepared callback"))?;
        let payload = self.pending.as_ref().map_or(&[][..], EncodedReceipt::bytes);
        let mut out = b"HR-ACK02".to_vec();
        out.push(record.phase() as u8);
        out.extend((record.id().len() as u16).to_le_bytes());
        out.extend(record.id().as_bytes());
        out.extend(record.before().len.to_le_bytes());
        out.extend(record.before().sha256);
        out.extend(after.len.to_le_bytes());
        out.extend(after.sha256);
        out.extend((payload.len() as u32).to_le_bytes());
        out.extend(payload);
        replace(&mut self.file, &out)?;
        self.file.sync_all()?;
        self.phases.push(record.phase());
        if self.stop == Some(record.phase()) {
            std::process::exit(77);
        }
        Ok(())
    }
}
fn read_ack(path: &Path) -> (u8, String, Vec<u8>) {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"HR-ACK02");
    let n = u16::from_le_bytes(bytes[9..11].try_into().unwrap()) as usize;
    let id = String::from_utf8(bytes[11..11 + n].to_vec()).unwrap();
    let at = 11 + n + 80;
    let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), at + 4 + len);
    (bytes[8], id, bytes[at + 4..].to_vec())
}
fn prepare(
    store: &Store,
    source: &Path,
    ack: &mut Ack,
    observer: &mut dyn Observer,
    legacy: bool,
) -> media::Result<media::Prepared> {
    let original = BoundMedia::open(source)?;
    if legacy {
        store.prepare(original, "flac", 0, ack, observer, |path| {
            fs::write(path, AFTER)
        })
    } else {
        store.prepare_file(original, "flac", 0, ack, observer, |held, _| {
            replace(held, AFTER)
        })
    }
}
#[derive(Default)]
struct Trace {
    events: Vec<Event>,
    fail: Option<usize>,
    panic: bool,
}
impl Observer for Trace {
    fn event(&mut self, event: Event) -> io::Result<()> {
        let at = self.events.len();
        self.events.push(event);
        if self.fail == Some(at) {
            if self.panic {
                panic!("owned after-editor fault at {at}")
            };
            return Err(io::Error::other("owned after-editor IO failure"));
        }
        Ok(())
    }
}
fn tail_trace() -> Vec<(usize, Event)> {
    let f = Fixture::new();
    fs::write(f.source(), BEFORE).unwrap();
    let store = f.store(budget());
    let mut ack = Ack::new(&f.0.join("ack"));
    let mut trace = Trace::default();
    drop(prepare(&store, &f.source(), &mut ack, &mut trace, false).unwrap());
    let start = trace
        .events
        .iter()
        .position(|e| *e == Event::AfterEdit)
        .unwrap();
    trace.events.into_iter().enumerate().skip(start).collect()
}
#[test]
fn every_post_editor_callback_error_and_unwind_preserves_source_and_releases_owners() {
    let positions = tail_trace();
    assert!(positions.len() > 8);
    let mut cases = 0;
    for legacy in [false, true] {
        for panic in [false, true] {
            for (at, event) in &positions {
                let f = Fixture::new();
                let source = f.source();
                fs::write(&source, BEFORE).unwrap();
                let alias = f.0.join("alias.flac");
                fs::hard_link(&source, &alias).unwrap();
                let inode = fs::metadata(&source).unwrap().ino();
                let resource = budget();
                let store = f.store(resource.clone());
                let baseline = resource.usage().0;
                let mut ack = Ack::new(&f.0.join("ack"));
                let mut trace = Trace {
                    fail: Some(*at),
                    panic,
                    ..Default::default()
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    prepare(&store, &source, &mut ack, &mut trace, legacy)
                }));
                assert!(
                    match result {
                        Ok(result) => !panic && result.is_err(),
                        Err(_) => panic,
                    },
                    "{legacy}/{panic}/{event:?}"
                );
                assert_eq!(trace.events[*at], *event);
                assert_eq!(fs::read(&source).unwrap(), BEFORE);
                assert_eq!(fs::read(alias).unwrap(), BEFORE);
                assert_eq!(fs::metadata(source).unwrap().ino(), inode);
                drop(ack);
                assert_eq!(store.usage(), (0, 0));
                assert_eq!(resource.usage().0, baseline);
                assert!(store.records().unwrap().is_empty());
                cases += 1;
            }
        }
    }
    eprintln!(
        "HELD_POST_EDITOR_FAULTS cases={cases} callbacks={}",
        positions.len()
    );
}
struct Mutate {
    at: usize,
    seen: usize,
    root: PathBuf,
    kind: u8,
    saved: Option<PathBuf>,
}
impl Observer for Mutate {
    fn event(&mut self, _event: Event) -> io::Result<()> {
        let at = self.seen;
        self.seen += 1;
        if at != self.at {
            return Ok(());
        }
        let path = candidate(&self.root);
        match self.kind {
            0 => {
                let old = fs::metadata(&path)?.modified()?;
                fs::write(&path, b"foreign!")?;
                File::options()
                    .write(true)
                    .open(&path)?
                    .set_times(fs::FileTimes::new().set_modified(old))?;
            }
            1 => fs::write(&path, b"foreign output grew")?,
            2 => {
                let saved = path.with_file_name("saved-candidate");
                fs::rename(&path, &saved)?;
                fs::write(&path, b"foreign!")?;
                self.saved = Some(saved);
            }
            3 => {
                let parent = path.parent().unwrap();
                let saved = self.root.parent().unwrap().join("saved-entry");
                fs::rename(parent, &saved)?;
                fs::create_dir(parent)?;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
                fs::write(parent.join("foreign"), b"foreign directory")?;
                self.saved = Some(saved);
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
#[test]
fn output_witness_is_not_recaptured_after_any_post_editor_boundary() {
    let positions = tail_trace();
    let mut cases = 0;
    for legacy in [false, true] {
        for kind in 0..4 {
            for (at, event) in &positions {
                let f = Fixture::new();
                fs::write(f.source(), BEFORE).unwrap();
                let resource = budget();
                let store = f.store(resource.clone());
                let baseline = resource.usage().0;
                let mut ack = Ack::new(&f.0.join("ack"));
                let mut mutate = Mutate {
                    at: *at,
                    seen: 0,
                    root: f.store_path(),
                    kind,
                    saved: None,
                };
                let result = prepare(&store, &f.source(), &mut ack, &mut mutate, legacy);
                assert!(
                    result.is_err(),
                    "accepted mutation legacy={legacy} kind={kind} event={event:?}"
                );
                assert_eq!(fs::read(f.source()).unwrap(), BEFORE);
                drop(ack);
                assert_eq!(resource.usage().0, baseline);
                if kind == 2 {
                    let saved = mutate.saved.as_ref().unwrap();
                    assert_eq!(fs::read(saved).unwrap(), AFTER);
                    assert_eq!(
                        fs::read(saved.with_file_name("candidate.flac")).unwrap(),
                        b"foreign!"
                    );
                    assert_eq!(store.usage().1, 1);
                }
                if kind == 3 {
                    let saved = mutate.saved.as_ref().unwrap();
                    assert_eq!(fs::read(saved.join("candidate.flac")).unwrap(), AFTER);
                    let dirs: Vec<_> = fs::read_dir(f.store_path())
                        .unwrap()
                        .map(|d| d.unwrap().path())
                        .filter(|p| p.is_dir())
                        .collect();
                    assert_eq!(dirs.len(), 1);
                    assert_eq!(
                        fs::read(dirs[0].join("foreign")).unwrap(),
                        b"foreign directory"
                    );
                    assert_eq!(store.usage().1, 1);
                }
                if kind < 2 {
                    assert_eq!(store.usage(), (0, 0));
                }
                cases += 1;
            }
        }
    }
    eprintln!("HELD_POST_EDITOR_MUTATIONS cases={cases}");
}
#[test]
fn nested_same_native_source_via_alias_refuses_without_disturbing_outer_lock() {
    for legacy in [false, true] {
        let f = Fixture::new();
        fs::write(f.source(), BEFORE).unwrap();
        let alias = f.0.join("alias.flac");
        fs::hard_link(f.source(), &alias).unwrap();
        let resource = budget();
        let store = f.store(resource.clone());
        let baseline = resource.usage().0;
        let mut ack = Ack::new(&f.0.join("ack"));
        if legacy {
            let prepared = store
                .prepare(
                    BoundMedia::open(&f.source()).unwrap(),
                    "flac",
                    0,
                    &mut ack,
                    &mut (),
                    |path| {
                        assert!(matches!(
                            BoundMedia::open(&alias),
                            Err(media::Error::Capacity)
                        ));
                        fs::write(path, AFTER)
                    },
                )
                .unwrap();
            assert!(matches!(
                BoundMedia::open(&alias),
                Err(media::Error::Capacity)
            ));
            drop(prepared);
        } else {
            let prepared = store
                .prepare_file(
                    BoundMedia::open(&f.source()).unwrap(),
                    "flac",
                    0,
                    &mut ack,
                    &mut (),
                    |held, _| {
                        assert!(matches!(
                            BoundMedia::open(&alias),
                            Err(media::Error::Capacity)
                        ));
                        replace(held, AFTER)
                    },
                )
                .unwrap();
            assert!(matches!(
                BoundMedia::open(&alias),
                Err(media::Error::Capacity)
            ));
            drop(prepared);
        }
        drop(BoundMedia::open(&alias).unwrap());
        assert_eq!(fs::read(alias).unwrap(), BEFORE);
        assert_eq!(resource.usage().0, baseline);
        assert_eq!(store.usage(), (0, 0));
    }
}
fn flac() -> Vec<u8> {
    let mut bytes = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    bytes.extend(info);
    let records = [
        "TITLE=Saved",
        "ARTIST=Literal ; / Artist",
        "CUSTOM=",
        "CUSTOM=尾",
        "GENRE=House",
        "GENRE=house",
    ];
    let mut vc = 4u32.to_le_bytes().to_vec();
    vc.extend(b"test");
    vc.extend((records.len() as u32).to_le_bytes());
    for record in records {
        vc.extend((record.len() as u32).to_le_bytes());
        vc.extend(record.as_bytes());
    }
    let n = vc.len();
    bytes.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    bytes.extend(vc);
    bytes.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    bytes
}
struct ExitAt {
    stage: String,
}
impl Observer for ExitAt {
    fn event(&mut self, event: Event) -> io::Result<()> {
        if self.stage == "before-prepared-ack" && event == Event::BeforeCallback(Phase::Prepared) {
            std::process::exit(77);
        }
        Ok(())
    }
}
#[test]
fn held_process_fixture_entry() {
    let Ok(root) = std::env::var("TAGREX_HELD_REVIEW_ROOT") else {
        return;
    };
    let stage = std::env::var("TAGREX_HELD_REVIEW_STAGE").unwrap();
    let root = PathBuf::from(root);
    let original = root.join("source.flac");
    let resource = budget();
    let store = Store::open(&root.join("store"), limits(), resource.clone()).unwrap();
    let target = NativeTextTarget::Vorbis {
        name: "TITLE".into(),
    };
    let intent = TagEngine::read_exact(&original)
        .unwrap()
        .text()
        .prepare_patch(
            &target,
            ExactTextState::try_from(vec!["New 尾".into(), "".into()]).unwrap(),
        )
        .unwrap();
    let mut driver = Ack::new(&root.join("ack"));
    driver.stop = match stage.as_str() {
        "prepared-ack" => Some(Phase::Prepared),
        "publishing-ack" => Some(Phase::Publishing),
        "published-ack" => Some(Phase::Published),
        _ => None,
    };
    let prepared = store
        .prepare_file(
            BoundMedia::open(&original).unwrap(),
            "flac",
            1024,
            &mut driver,
            &mut ExitAt {
                stage: stage.clone(),
            },
            |held, driver| {
                let receipt = native_patch::apply_prepared_with_budget(
                    held,
                    &intent,
                    resource.clone(),
                    &mut (),
                )
                .map_err(io::Error::other)?;
                driver.pending = Some(
                    receipt
                        .encode_for_journal_with_budget(resource.clone())
                        .map_err(io::Error::other)?,
                );
                held.sync_all()?;
                if stage == "editor" {
                    std::process::exit(77);
                }
                Ok(())
            },
        )
        .unwrap();
    if stage == "prepared-return" {
        std::process::exit(77);
    }
    let _published = prepared.publish(&mut driver, &mut ()).unwrap();
    if stage == "published-return" {
        std::process::exit(77);
    }
    panic!("unrecognized fixture stage {stage}");
}
fn exit_child(root: &Path, stage: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "held_process_fixture_entry", "--nocapture"])
        .env("TAGREX_HELD_REVIEW_ROOT", root)
        .env("TAGREX_HELD_REVIEW_STAGE", stage)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let mut error = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut error)
                .unwrap();
            assert_eq!(status.code(), Some(77), "{stage}: {error}");
            break;
        }
        if Instant::now() >= until {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("owned child exceeded deadline for {stage}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn hard_exit_before_and_after_typed_acknowledgements_reopens_with_proved_images() {
    for stage in [
        "editor",
        "before-prepared-ack",
        "prepared-ack",
        "prepared-return",
        "publishing-ack",
        "published-ack",
        "published-return",
    ] {
        let f = Fixture::new();
        let before = flac();
        fs::write(f.source(), &before).unwrap();
        let source_inode = fs::metadata(f.source()).unwrap().ino();
        exit_child(&f.0, stage);
        let resource = budget();
        let store = f.store(resource.clone());
        let baseline = resource.usage().0;
        let records = store.records().unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        let phase = match stage {
            "editor" => Phase::Preparing,
            "publishing-ack" => Phase::Publishing,
            "published-ack" | "published-return" => Phase::Published,
            _ => Phase::Prepared,
        };
        assert_eq!(record.phase(), phase);
        let path = f.store_path().join(record.id());
        assert_eq!(fs::read(path.join("preimage")).unwrap(), before);
        let edited = fs::read(path.join("candidate.flac")).unwrap();
        assert_ne!(edited, before);
        assert_eq!(fs::metadata(f.source()).unwrap().ino(), source_inode);
        let current = store
            .inspect(record.id(), &mut BoundMedia::open(&f.source()).unwrap())
            .unwrap();
        if phase == Phase::Published {
            assert_eq!(current, Current::Candidate);
            assert_eq!(fs::read(f.source()).unwrap(), edited);
        } else {
            assert_eq!(fs::read(f.source()).unwrap(), before);
            assert_eq!(
                current,
                if phase == Phase::Preparing {
                    Current::Ambiguous
                } else {
                    Current::Preimage
                }
            );
        }
        if matches!(stage, "editor" | "before-prepared-ack") {
            assert_eq!(fs::metadata(f.0.join("ack")).unwrap().len(), 0);
        } else {
            let (ack_phase, id, raw) = read_ack(&f.0.join("ack"));
            assert_eq!(ack_phase, phase as u8);
            assert_eq!(id, record.id());
            let receipt = Receipt::decode_for_journal_with_budget(&raw, resource.clone()).unwrap();
            assert_eq!(receipt.old().values().unwrap().as_slice(), ["Saved"]);
            assert_eq!(
                receipt.new_state().values().unwrap().as_slice(),
                ["New 尾", ""]
            );
        }
        assert_eq!(resource.usage().0, baseline);
        assert_eq!(store.usage().1, 1);
    }
}
#[test]
fn published_child_receipt_can_retire_then_drive_a_second_protected_restore() {
    let f = Fixture::new();
    let before = flac();
    fs::write(f.source(), &before).unwrap();
    let inode = fs::metadata(f.source()).unwrap().ino();
    exit_child(&f.0, "published-ack");
    let resource = budget();
    let store = f.store(resource.clone());
    let baseline = resource.usage().0;
    let (_, id, raw) = read_ack(&f.0.join("ack"));
    let receipt = Receipt::decode_for_journal_with_budget(&raw, resource.clone())
        .unwrap()
        .rebind_current_policy()
        .unwrap();
    let encoded = receipt
        .encode_for_journal_with_budget(resource.clone())
        .unwrap();
    let mut driver = Ack::reopen(&f.0.join("ack"), encoded);
    store
        .resume_published(&id, BoundMedia::open(&f.source()).unwrap())
        .unwrap()
        .retire(
            |record| {
                let (phase, ack_id, _) = read_ack(&f.0.join("ack"));
                assert_eq!(phase, Phase::Published as u8);
                assert_eq!(ack_id, record.id());
                Ok(())
            },
            &mut driver,
            &mut (),
        )
        .unwrap();
    assert_eq!(store.usage(), (0, 0));
    assert!(store.records().unwrap().is_empty());
    let mut restore_driver = Ack::new(&f.0.join("restore-ack"));
    let prepared = store
        .prepare_file(
            BoundMedia::open(&f.source()).unwrap(),
            "flac",
            1024,
            &mut restore_driver,
            &mut (),
            |held, driver| {
                let reverse =
                    native_patch::restore_with_budget(held, &receipt, resource.clone(), &mut ())
                        .map_err(io::Error::other)?;
                driver.pending = Some(
                    reverse
                        .encode_for_journal_with_budget(resource.clone())
                        .map_err(io::Error::other)?,
                );
                Ok(())
            },
        )
        .unwrap();
    let published = prepared.publish(&mut restore_driver, &mut ()).unwrap();
    assert_eq!(fs::read(f.source()).unwrap(), before);
    assert_eq!(fs::metadata(f.source()).unwrap().ino(), inode);
    published
        .retire(|_| Ok(()), &mut restore_driver, &mut ())
        .unwrap();
    drop(driver);
    drop(restore_driver);
    drop(receipt);
    assert_eq!(resource.usage().0, baseline);
    assert_eq!(store.usage(), (0, 0));
}

#[test]
fn retirement_pressure_retains_published_evidence_and_retry_uses_the_same_typed_driver() {
    let f = Fixture::new();
    let a = f.source();
    let b = f.0.join("b.flac");
    fs::write(&a, BEFORE).unwrap();
    fs::write(&b, BEFORE).unwrap();
    let resource = Budget::new(ResourceLimits {
        retained: 32 << 20,
        transient: 192 << 20,
        total: 224 << 20,
        jobs: 1,
    });
    let store = f.store(resource.clone());
    let baseline = resource.usage().0;
    let mut ack_a = Ack::new(&f.0.join("ack-a"));
    let mut ack_b = Ack::new(&f.0.join("ack-b"));
    let first = prepare(&store, &a, &mut ack_a, &mut (), false)
        .unwrap()
        .publish(&mut ack_a, &mut ())
        .unwrap();
    let id = first.record().id().to_string();
    let second = prepare(&store, &b, &mut ack_b, &mut (), false).unwrap();
    let error = first.retire(|_| Ok(()), &mut ack_a, &mut ());
    assert!(matches!(
        error,
        Err(media::Error::Resource(
            tagrex_core::resources::AdmissionError::Busy
        ))
    ));
    assert_eq!(read_ack(&f.0.join("ack-a")).0, Phase::Published as u8);
    assert_eq!(fs::read(&a).unwrap(), AFTER);
    assert_eq!(fs::read(&b).unwrap(), BEFORE);
    assert_eq!(store.usage().1, 2);
    assert_eq!(
        store
            .records()
            .unwrap()
            .iter()
            .find(|r| r.id() == id)
            .unwrap()
            .phase(),
        Phase::Published
    );
    drop(second);
    assert_eq!(resource.usage().0, baseline);
    store
        .resume_published(&id, BoundMedia::open(&a).unwrap())
        .unwrap()
        .retire(
            |record| {
                let (phase, ack_id, _) = read_ack(&f.0.join("ack-a"));
                assert_eq!(phase, Phase::Published as u8);
                assert_eq!(ack_id, record.id());
                Ok(())
            },
            &mut ack_a,
            &mut (),
        )
        .unwrap();
    assert_eq!(read_ack(&f.0.join("ack-a")).0, Phase::Retired as u8);
    assert_eq!(resource.usage().0, baseline);
    assert_eq!(store.usage(), (0, 0));
    assert_eq!(fs::read(a).unwrap(), AFTER);
    assert_eq!(fs::read(b).unwrap(), BEFORE);
}
