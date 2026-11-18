//! Independent held-capability reader checks; all media is disposable.
use std::{
    cell::Cell,
    fs::{self, File},
    io::{Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tagrex_core::{
    model::{
        self,
        exact::{ExactReadError, ExactTrackRead},
        TagEngine, TagField,
    },
    resources::Budget,
};
static SERIAL: Mutex<()> = Mutex::new(());
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "tagrex-held-independent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn flac(title: &str) -> Vec<u8> {
    let mut b = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    b.extend(info);
    let items = [
        format!("TITLE={title}"),
        "ARTIST=A; B".into(),
        "CUSTOM=".into(),
        "CUSTOM=尾".into(),
    ];
    let mut v = 0u32.to_le_bytes().to_vec();
    v.extend(4u32.to_le_bytes());
    for item in items {
        v.extend((item.len() as u32).to_le_bytes());
        v.extend(item.as_bytes());
    }
    let n = v.len();
    b.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    b.extend(v);
    b.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    b
}
fn mp3() -> Vec<u8> {
    let payload = b"\x03Held\0\0Tail";
    let mut frame = b"TIT2".to_vec();
    frame.extend([0, 0, 0, payload.len() as u8, 0, 0]);
    frame.extend(payload);
    let mut out = b"ID3\x04\0\0".to_vec();
    out.extend([0, 0, 0, frame.len() as u8]);
    out.extend(frame);
    let mut audio = vec![0xff, 0xfb, 0x90, 0];
    audio.resize(417, 0);
    out.extend(audio.repeat(5));
    out
}
fn same(a: &ExactTrackRead, b: &ExactTrackRead) {
    assert_eq!(a.legacy().file.tags, b.legacy().file.tags);
    assert_eq!(a.legacy().file.format, b.legacy().file.format);
    assert_eq!(a.legacy().blocks, b.legacy().blocks);
    assert_eq!(a.text().entries().len(), b.text().entries().len());
    for (a, b) in a.text().entries().iter().zip(b.text().entries()) {
        assert_eq!(a.target, b.target);
        assert_eq!(a.state, b.state);
    }
}
fn checkpoints(p: &Path, label: &Path) -> usize {
    let n = Cell::new(0);
    TagEngine::read_exact_file_with_cancel(File::open(p).unwrap(), label, || {
        n.set(n.get() + 1);
        false
    })
    .unwrap();
    n.get()
}
#[test]
fn format_matrix_uses_held_bytes_with_absent_and_foreign_labels_and_nonzero_cursor() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let f = Fixture::new();
    for (ext, bytes) in [
        ("flac", flac("Held")),
        ("mp3", mp3()),
        ("m4a", include_bytes!("fixtures/minimal.m4a").to_vec()),
    ] {
        let actual = f.put(&format!("held.{ext}"), &bytes);
        let expected = TagEngine::read_exact(&actual).unwrap();
        let absent = f.0.join(format!("absent.{ext}"));
        let foreign = f.put(&format!("foreign.{ext}"), &flac("Foreign"));
        for label in [&absent, &foreign] {
            let mut file = File::open(&actual).unwrap();
            file.seek(SeekFrom::End(-3)).unwrap();
            let read = TagEngine::read_exact_file(file, label).unwrap();
            same(&expected, &read);
            assert_eq!(&read.legacy().file.path, label);
        }
        assert_eq!(fs::read(actual).unwrap(), bytes);
        assert_eq!(fs::read(foreign).unwrap(), flac("Foreign"));
        assert!(!absent.exists());
    }
}
#[cfg(unix)]
#[test]
fn unlinked_held_files_are_readable_without_adopting_replacements() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let f = Fixture::new();
    for (ext, bytes) in [
        ("flac", flac("Held")),
        ("mp3", mp3()),
        ("m4a", include_bytes!("fixtures/minimal.m4a").to_vec()),
    ] {
        let actual = f.put(&format!("unlinked.{ext}"), &bytes);
        let expected = TagEngine::read_exact(&actual).unwrap();
        let file = File::open(&actual).unwrap();
        fs::remove_file(&actual).unwrap();
        fs::write(&actual, flac("Foreign")).unwrap();
        same(
            &expected,
            &TagEngine::read_exact_file(file, &actual).unwrap(),
        );
        assert_eq!(fs::read(actual).unwrap(), flac("Foreign"));
    }
}
#[test]
fn final_checkpoint_cannot_accept_a_changed_held_source() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let f = Fixture::new();
    let actual = f.put("source.flac", &flac("Before"));
    let label = f.0.join("logical.flac");
    let total = checkpoints(&actual, &label);
    let n = Cell::new(0);
    let result =
        TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
            n.set(n.get() + 1);
            if n.get() == total {
                fs::write(&actual, flac("After!")).unwrap();
            }
            false
        });
    assert!(
        matches!(result, Err(ExactReadError::Changed)),
        "changed at final callback: {result:?}"
    );
    assert_eq!(fs::read(actual).unwrap(), flac("After!"));
}
#[test]
fn final_checkpoint_cannot_accept_a_changed_read_policy() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    struct Restore(String);
    impl Drop for Restore {
        fn drop(&mut self) {
            model::set_multi_value_separator(&self.0);
        }
    }
    let _restore = Restore(model::multi_value_separator());
    let f = Fixture::new();
    let actual = f.put("source.flac", &flac("Held"));
    let label = f.0.join("logical.flac");
    let total = checkpoints(&actual, &label);
    let n = Cell::new(0);
    let result =
        TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
            n.set(n.get() + 1);
            if n.get() == total {
                model::set_multi_value_separator("~~independent~~");
            }
            false
        });
    assert!(
        matches!(result, Err(ExactReadError::Changed)),
        "policy changed at final callback: {result:?}"
    );
}
#[test]
fn repeated_success_error_cancel_and_unwind_release_descriptors_and_owners() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let f = Fixture::new();
    let actual = f.put("source.flac", &flac("Held"));
    let malformed = f.put("bad.flac", b"fLaC malformed source");
    let label = f.0.join("label.flac");
    let budget = Budget::process();
    let initial = budget.usage().0;
    #[cfg(target_os = "linux")]
    let fd_count = || fs::read_dir("/proc/self/fd").unwrap().count();
    #[cfg(target_os = "linux")]
    let before_fd = fd_count();
    for i in 0..96 {
        match i % 4 {
            0 => {
                drop(TagEngine::read_exact_file(File::open(&actual).unwrap(), &label).unwrap());
            }
            1 => {
                assert!(
                    TagEngine::read_exact_file(File::open(&malformed).unwrap(), &label).is_err()
                );
            }
            2 => {
                assert!(matches!(
                    TagEngine::read_exact_file_with_cancel(
                        File::open(&actual).unwrap(),
                        &label,
                        || true
                    ),
                    Err(ExactReadError::Cancelled)
                ));
            }
            _ => {
                assert!(std::panic::catch_unwind(|| {
                    let _ = TagEngine::read_exact_file_with_cancel(
                        File::open(&actual).unwrap(),
                        &label,
                        || panic!("owned unwind"),
                    );
                })
                .is_err());
            }
        }
        assert_eq!(budget.usage().0, initial);
    }
    #[cfg(target_os = "linux")]
    assert_eq!(fd_count(), before_fd);
    assert_eq!(
        TagEngine::read(&actual).unwrap().tags[&TagField::Title],
        "Held"
    );
    assert!(!label.exists());
}
#[test]
fn foreign_label_changes_at_every_checkpoint_are_not_source_changes() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let f = Fixture::new();
    let actual = f.put("held.flac", &flac("Held"));
    let label = f.put("label.flac", &flac("Foreign"));
    let total = checkpoints(&actual, &label);
    for checkpoint in 1..=total {
        let n = Cell::new(0);
        let read =
            TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
                n.set(n.get() + 1);
                if n.get() == checkpoint {
                    fs::write(&label, flac("Other foreign")).unwrap();
                    assert_eq!(
                        TagEngine::read(&label).unwrap().tags[&TagField::Title],
                        "Other foreign"
                    );
                }
                false
            })
            .unwrap();
        assert_eq!(read.legacy().file.tags[&TagField::Title], "Held");
        assert_eq!(fs::read(&actual).unwrap(), flac("Held"));
    }
}
#[test]
fn held_reader_scopes_are_independent_across_threads_and_nested_same_labels() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let f = Fixture::new();
    let actual = f.put("held.flac", &flac("Outer"));
    let inner = f.put("inner.flac", &flac("Inner"));
    let label = f.put("label.flac", &flac("Public"));
    let total = checkpoints(&actual, &label);
    let n = Cell::new(0);
    let read = TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
        n.set(n.get() + 1);
        if n.get() == total {
            let nested = TagEngine::read_exact_file(File::open(&inner).unwrap(), &label).unwrap();
            assert_eq!(nested.legacy().file.tags[&TagField::Title], "Inner");
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        assert_eq!(
                            TagEngine::read(&label).unwrap().tags[&TagField::Title],
                            "Public"
                        );
                        assert_eq!(
                            TagEngine::read_exact_file(File::open(&inner).unwrap(), &label)
                                .unwrap()
                                .legacy()
                                .file
                                .tags[&TagField::Title],
                            "Inner"
                        );
                    })
                    .join()
                    .unwrap();
            });
        }
        false
    })
    .unwrap();
    assert_eq!(read.legacy().file.tags[&TagField::Title], "Outer");
    assert_eq!(
        TagEngine::read(&label).unwrap().tags[&TagField::Title],
        "Public"
    );
}
#[test]
fn named_reader_also_revalidates_after_the_final_callback() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let f = Fixture::new();
    let actual = f.put("named.flac", &flac("Before"));
    let total = Cell::new(0);
    TagEngine::read_exact_with_cancel(&actual, || {
        total.set(total.get() + 1);
        false
    })
    .unwrap();
    let n = Cell::new(0);
    let result = TagEngine::read_exact_with_cancel(&actual, || {
        n.set(n.get() + 1);
        if n.get() == total.get() {
            fs::write(&actual, flac("After!")).unwrap();
        }
        false
    });
    assert!(matches!(result, Err(ExactReadError::Changed)));
    assert_eq!(fs::read(actual).unwrap(), flac("After!"));
}
