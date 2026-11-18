//! Held editor/typed durable sink seam, with owned synthetic media only.
#![cfg(target_os = "linux")]
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    media_mutation::{
        self as media, BoundMedia, Event, Limits, Observer, Phase, Record, StateSink, Store,
    },
    resources::{Budget, Limits as ResourceLimits},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-held-editor-{}-{}",
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
fn limits() -> Limits {
    Limits {
        store_bytes: 32 << 20,
        entries: 16,
        media_bytes: 8 << 20,
    }
}
fn budget() -> Budget {
    Budget::new(ResourceLimits {
        retained: 64 << 20,
        transient: 256 << 20,
        total: 320 << 20,
        jobs: 8,
    })
}
fn sink(_: &Record) -> io::Result<()> {
    Ok(())
}
fn candidate(root: &Path) -> PathBuf {
    fs::read_dir(root)
        .unwrap()
        .map(|d| d.unwrap().path())
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
fn replace_bytes(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    file.write_all(bytes)?;
    file.set_len(bytes.len() as u64)
}
#[test]
fn after_edit_mutation_cannot_be_adopted_as_the_prepared_output() {
    struct Mutate(PathBuf);
    impl Observer for Mutate {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if event == Event::AfterEdit {
                fs::write(candidate(&self.0), b"foreign!")?;
            }
            Ok(())
        }
    }
    let f = Fixture::new();
    let original = f.put("original.flac", b"original");
    let store = f.store();
    let result = store.prepare(
        BoundMedia::open(&original).unwrap(),
        "flac",
        0,
        &mut sink,
        &mut Mutate(f.0.join("store")),
        |path| fs::write(path, b"edited!!"),
    );
    assert!(
        matches!(result, Err(media::Error::Changed)),
        "post-editor mutation must not become a new output baseline"
    );
    assert_eq!(fs::read(original).unwrap(), b"original");
}

use tagrex_core::model::{
    exact::{ExactTextState, NativeTextTarget},
    native_patch::{self, codec::EncodedReceipt, Receipt},
    TagEngine,
};
struct Driver {
    pending: Option<EncodedReceipt>,
    file: File,
    phases: Vec<Phase>,
    fail: Option<Phase>,
}
impl Driver {
    fn new(path: &Path) -> Self {
        Self {
            pending: None,
            file: OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(path)
                .unwrap(),
            phases: Vec::new(),
            fail: None,
        }
    }
}
impl StateSink for Driver {
    fn persist(&mut self, record: &Record) -> io::Result<()> {
        if self.fail == Some(record.phase()) {
            return Err(io::Error::other("fixture durable acknowledgement failed"));
        }
        let receipt = self
            .pending
            .as_ref()
            .ok_or_else(|| io::Error::other("missing exact rollback receipt"))?;
        let mut envelope = b"HELDACK1".to_vec();
        envelope.push(record.phase() as u8);
        envelope.extend((record.id().len() as u16).to_le_bytes());
        envelope.extend(record.id().as_bytes());
        envelope.extend(record.before().len.to_le_bytes());
        envelope.extend(record.before().sha256);
        let after = record
            .after()
            .ok_or_else(|| io::Error::other("missing candidate proof"))?;
        envelope.extend(after.len.to_le_bytes());
        envelope.extend(after.sha256);
        envelope.extend((receipt.bytes().len() as u32).to_le_bytes());
        envelope.extend(receipt.bytes());
        replace_bytes(&mut self.file, &envelope)?;
        self.file.sync_all()?;
        self.phases.push(record.phase());
        Ok(())
    }
}
fn ack(path: &Path) -> (u8, String, Vec<u8>) {
    let raw = fs::read(path).unwrap();
    assert_eq!(&raw[..8], b"HELDACK1");
    let n = u16::from_le_bytes(raw[9..11].try_into().unwrap()) as usize;
    let id = String::from_utf8(raw[11..11 + n].to_vec()).unwrap();
    let pos = 11 + n + 80;
    let len = u32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap()) as usize;
    assert_eq!(raw.len(), pos + 4 + len);
    (raw[8], id, raw[pos + 4..].to_vec())
}
fn flac() -> Vec<u8> {
    let mut raw = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    raw.extend(info);
    let values = [
        "TITLE=Saved",
        "ARTIST=A; B",
        "GENRE=House",
        "GENRE=house",
        "CUSTOM=",
        "CUSTOM=尾",
    ];
    let mut vc = 4u32.to_le_bytes().to_vec();
    vc.extend(b"test");
    vc.extend((values.len() as u32).to_le_bytes());
    for value in values {
        vc.extend((value.len() as u32).to_le_bytes());
        vc.extend(value.as_bytes());
    }
    let n = vc.len();
    raw.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    raw.extend(vc);
    raw.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    raw
}
fn metadata(bytes: &[u8]) -> Vec<String> {
    let body = &bytes[46..];
    let mut at = 4 + u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
    let count = u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
    at += 4;
    let mut values = Vec::new();
    for _ in 0..count {
        let n = u32::from_le_bytes(body[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        values.push(String::from_utf8(body[at..at + n].to_vec()).unwrap());
        at += n;
    }
    values
}
#[test]
fn typed_driver_persists_real_native_receipt_before_publish_then_restores_from_disk() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new();
    let before = flac();
    let original = f.put("original.flac", &before);
    let inode = fs::metadata(&original).unwrap().ino();
    let target = NativeTextTarget::Vorbis {
        name: "TITLE".into(),
    };
    let intent = TagEngine::read_exact(&original)
        .unwrap()
        .text()
        .prepare_patch(
            &target,
            ExactTextState::try_from(vec!["New 日本語".into(), "".into()]).unwrap(),
        )
        .unwrap();
    let resources = budget();
    let store = Store::open(&f.0.join("store"), limits(), resources.clone()).unwrap();
    let baseline = resources.usage().0;
    let journal = f.0.join("fixture-journal");
    let mut driver = Driver::new(&journal);
    let mut observed = |record: &Record| -> io::Result<()> {
        assert!(record.phase() >= Phase::Prepared);
        Ok(())
    };
    let prepared = store
        .prepare_file(
            BoundMedia::open(&original).unwrap(),
            "flac",
            1024,
            &mut driver,
            &mut (),
            |candidate, driver| {
                let receipt = native_patch::apply_prepared_with_budget(
                    candidate,
                    &intent,
                    resources.clone(),
                    &mut (),
                )
                .map_err(io::Error::other)?;
                driver.pending = Some(
                    receipt
                        .encode_for_journal_with_budget(resources.clone())
                        .map_err(io::Error::other)?,
                );
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(fs::read(&original).unwrap(), before);
    let (phase, id, bytes) = ack(&journal);
    assert_eq!(phase, Phase::Prepared as u8);
    assert_eq!(id, prepared.record().id());
    let decoded = Receipt::decode_for_journal_with_budget(&bytes, resources.clone()).unwrap();
    assert_eq!(decoded.old().values().unwrap().as_slice(), ["Saved"]);
    drop(decoded);
    // Publication cannot call this sink until the same driver's Prepared
    // receipt has been synced. Verify that actual disk evidence here.
    struct CheckAck<'a> {
        path: &'a Path,
        driver: &'a mut Driver,
    }
    impl StateSink for CheckAck<'_> {
        fn persist(&mut self, record: &Record) -> io::Result<()> {
            if record.phase() == Phase::Publishing {
                let (phase, id, raw) = ack(self.path);
                assert_eq!(phase, Phase::Prepared as u8);
                assert_eq!(id, record.id());
                Receipt::decode_for_journal(&raw).map_err(io::Error::other)?;
            }
            self.driver.persist(record)
        }
    }
    let published = prepared
        .publish(
            &mut CheckAck {
                path: &journal,
                driver: &mut driver,
            },
            &mut (),
        )
        .unwrap();
    assert_eq!(fs::metadata(&original).unwrap().ino(), inode);
    let after = fs::read(&original).unwrap();
    let mut expected = metadata(&before);
    expected.splice(0..1, ["TITLE=New 日本語".into(), "TITLE=".into()]);
    assert_eq!(metadata(&after), expected);
    assert_eq!(&after[after.len() - 14..], &before[before.len() - 14..]);
    published
        .retire(
            |record| {
                let (phase, id, _) = ack(&journal);
                assert_eq!(phase, Phase::Published as u8);
                assert_eq!(id, record.id());
                Ok(())
            },
            &mut driver,
            &mut (),
        )
        .unwrap();
    assert_eq!(
        driver.phases,
        [
            Phase::Prepared,
            Phase::Publishing,
            Phase::Published,
            Phase::Retired
        ]
    );
    assert_eq!(store.usage(), (0, 0));
    let (_, _, persisted) = ack(&journal);
    drop(driver);
    // This demonstrates a second protected operation, not Executor/history
    // integration. Only the receipt comes from the synced fixture journal.
    let receipt = Receipt::decode_for_journal_with_budget(&persisted, resources.clone())
        .unwrap()
        .rebind_current_policy()
        .unwrap();
    let restored = store
        .prepare_file(
            BoundMedia::open(&original).unwrap(),
            "flac",
            1024,
            &mut observed,
            &mut (),
            |candidate, _| {
                drop(
                    native_patch::restore_with_budget(
                        candidate,
                        &receipt,
                        resources.clone(),
                        &mut (),
                    )
                    .map_err(io::Error::other)?,
                );
                Ok(())
            },
        )
        .unwrap()
        .publish(&mut observed, &mut ())
        .unwrap();
    assert_eq!(fs::read(&original).unwrap(), before);
    restored.retire(|_| Ok(()), &mut observed, &mut ()).unwrap();
    drop(receipt);
    assert_eq!(resources.usage().0, baseline);
}

#[test]
fn held_editor_keeps_the_original_candidate_object_when_the_private_name_is_replaced() {
    let f = Fixture::new();
    let original = f.put("original.flac", b"original");
    let store = f.store();
    let root = f.0.join("store");
    let mut saved = None;
    let result = store.prepare_file(
        BoundMedia::open(&original).unwrap(),
        "flac",
        0,
        &mut sink,
        &mut (),
        |held, _| {
            let named = candidate(&root);
            let aside = named.with_file_name("held-aside");
            fs::rename(&named, &aside)?;
            fs::write(&named, b"foreign!")?;
            replace_bytes(held, b"edited!!")?;
            assert_eq!(fs::read(&named)?, b"foreign!");
            assert_eq!(fs::read(&aside)?, b"edited!!");
            saved = Some((named, aside));
            Ok(())
        },
    );
    assert!(
        result.is_err(),
        "a replaced private artifact cannot be prepared"
    );
    let (named, aside) = saved.unwrap();
    assert_eq!(fs::read(named).unwrap(), b"foreign!");
    assert_eq!(fs::read(aside).unwrap(), b"edited!!");
    assert_eq!(fs::read(original).unwrap(), b"original");
    assert_eq!(store.usage().1, 1);
}

#[test]
fn typed_editor_errors_panics_cancellation_and_prepared_ack_failure_leave_original_unchanged() {
    let f = Fixture::new();
    let original = f.put("original.flac", b"original");
    let resource = budget();
    let store = Store::open(&f.0.join("store"), limits(), resource.clone()).unwrap();
    let baseline = resource.usage().0;
    for action in 0..3 {
        let mut driver = Driver::new(&f.0.join(format!("ack-{action}")));
        driver.fail = Some(Phase::Prepared);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.prepare_file(
                BoundMedia::open(&original).unwrap(),
                "flac",
                0,
                &mut driver,
                &mut (),
                |held, _| {
                    replace_bytes(held, b"edited!!")?;
                    match action {
                        0 => Err(io::Error::other("editor cancelled")),
                        1 => panic!("fixture editor panic"),
                        _ => Ok(()),
                    }
                },
            )
        }));
        assert!(match result {
            Ok(result) => result.is_err(),
            Err(_) => action == 1,
        });
        assert_eq!(fs::read(&original).unwrap(), b"original");
        assert_eq!(store.usage(), (0, 0));
        assert_eq!(resource.usage().0, baseline);
    }
}

#[test]
fn typed_editor_nested_separate_operations_short_writes_and_admission_are_isolated() {
    struct Short;
    impl Observer for Short {
        fn write_limit(&mut self, _: media::CopyKind, n: usize) -> io::Result<usize> {
            Ok(n.min(3))
        }
    }
    let f = Fixture::new();
    let a = f.put("a.flac", b"original");
    let b = f.put("b.flac", b"original");
    let resource = budget();
    let store = Store::open(&f.0.join("store"), limits(), resource.clone()).unwrap();
    let baseline = resource.usage().0;
    let outer = store
        .prepare_file(
            BoundMedia::open(&a).unwrap(),
            "flac",
            0,
            &mut sink,
            &mut Short,
            |held, _| {
                store
                    .prepare_file(
                        BoundMedia::open(&b).unwrap(),
                        "flac",
                        0,
                        &mut sink,
                        &mut Short,
                        |inner, _| replace_bytes(inner, b"inside!!"),
                    )
                    .unwrap()
                    .publish(&mut sink, &mut Short)
                    .unwrap()
                    .retire(|_| Ok(()), &mut sink, &mut Short)
                    .unwrap();
                replace_bytes(held, b"outside!")
            },
        )
        .unwrap();
    assert_eq!(fs::read(&a).unwrap(), b"original");
    assert_eq!(fs::read(&b).unwrap(), b"inside!!");
    outer
        .publish(&mut sink, &mut Short)
        .unwrap()
        .retire(|_| Ok(()), &mut sink, &mut Short)
        .unwrap();
    assert_eq!(fs::read(&a).unwrap(), b"outside!");
    assert_eq!(resource.usage().0, baseline);
    let occupied = resource
        .try_acquire(tagrex_core::resources::Charge {
            transient: 256 << 20,
            ..Default::default()
        })
        .unwrap();
    let result = store.prepare_file(
        BoundMedia::open(&a).unwrap(),
        "flac",
        0,
        &mut sink,
        &mut (),
        |_, _| panic!("admission must happen before editing"),
    );
    assert!(matches!(result, Err(media::Error::Resource(_))));
    assert_eq!(fs::read(&a).unwrap(), b"outside!");
    drop(occupied);
    assert_eq!(store.usage(), (0, 0));
}

#[test]
fn native_editor_cancellation_and_shared_pressure_release_prepare_resources() {
    struct Cancel;
    impl native_patch::Control for Cancel {
        fn cancelled(&mut self) -> bool {
            true
        }
    }
    let f = Fixture::new();
    let before = flac();
    let original = f.put("original.flac", &before);
    let intent = TagEngine::read_exact(&original)
        .unwrap()
        .text()
        .prepare_patch(
            &NativeTextTarget::Vorbis {
                name: "TITLE".into(),
            },
            ExactTextState::try_from(vec!["Changed".into()]).unwrap(),
        )
        .unwrap();
    for pressure in [false, true] {
        let resource = if pressure {
            Budget::new(ResourceLimits {
                retained: 64 << 20,
                transient: 1 << 20,
                total: 65 << 20,
                jobs: 8,
            })
        } else {
            budget()
        };
        let root = f.0.join(format!("store-{pressure}"));
        let store = Store::open(&root, limits(), resource.clone()).unwrap();
        let baseline = resource.usage().0;
        let mut called = false;
        let result = store.prepare_file(
            BoundMedia::open(&original).unwrap(),
            "flac",
            1024,
            &mut sink,
            &mut (),
            |candidate, _| {
                called = true;
                let result = if pressure {
                    native_patch::apply_prepared_with_budget(
                        candidate,
                        &intent,
                        resource.clone(),
                        &mut (),
                    )
                } else {
                    native_patch::apply_prepared_with_budget(
                        candidate,
                        &intent,
                        resource.clone(),
                        &mut Cancel,
                    )
                };
                if pressure {
                    assert!(matches!(result, Err(native_patch::Error::Resource(_))));
                } else {
                    assert!(matches!(result, Err(native_patch::Error::Cancelled)));
                }
                result.map(drop).map_err(io::Error::other)
            },
        );
        assert!(called && result.is_err());
        assert_eq!(fs::read(&original).unwrap(), before);
        assert_eq!(store.usage(), (0, 0));
        assert_eq!(resource.usage().0, baseline);
    }
}

#[test]
fn output_mutation_in_prepared_acknowledgement_is_refused_before_return_or_publication() {
    struct Mutate {
        root: PathBuf,
        edited: bool,
    }
    impl StateSink for Mutate {
        fn persist(&mut self, record: &Record) -> io::Result<()> {
            assert_eq!(record.phase(), Phase::Prepared);
            assert!(self.edited);
            fs::write(candidate(&self.root), b"foreign!")
        }
    }
    let f = Fixture::new();
    let original = f.put("original.flac", b"original");
    let store = f.store();
    let mut driver = Mutate {
        root: f.0.join("store"),
        edited: false,
    };
    let result = store.prepare_file(
        BoundMedia::open(&original).unwrap(),
        "flac",
        0,
        &mut driver,
        &mut (),
        |candidate, driver| {
            replace_bytes(candidate, b"edited!!")?;
            driver.edited = true;
            Ok(())
        },
    );
    assert!(matches!(result, Err(media::Error::Changed)));
    assert_eq!(fs::read(original).unwrap(), b"original");
    assert_eq!(store.usage(), (0, 0));
}
