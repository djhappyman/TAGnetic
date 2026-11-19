//! Owned-file exact reads must not reopen their logical label or leak its scope.
use std::{
    cell::Cell,
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    model::{
        exact::{ExactReadError, ExactTrackRead, NativeTextTarget},
        TagEngine, TagField,
    },
    resources::{Budget, Charge},
};
static CASE: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Fixture(PathBuf, std::sync::MutexGuard<'static, ()>);
impl Fixture {
    fn new() -> Self {
        let guard = CASE.lock().unwrap_or_else(|p| p.into_inner());
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-held-exact-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path, guard)
    }
    fn put(&self, name: &str, title: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, flac(title)).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = &self.1;
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn flac(title: &str) -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut comments = 0u32.to_le_bytes().to_vec();
    comments.extend(4u32.to_le_bytes());
    for item in [
        format!("TITLE={title}"),
        "ARTIST=A; B".into(),
        "CUSTOM=".into(),
        "CUSTOM=尾".into(),
    ] {
        comments.extend((item.len() as u32).to_le_bytes());
        comments.extend(item.as_bytes());
    }
    let n = comments.len();
    out.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(comments);
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out
}
fn verify(read: &ExactTrackRead, title: &str, label: &Path) {
    assert_eq!(
        read.legacy()
            .file
            .tags
            .get(&TagField::Title)
            .map(String::as_str),
        Some(title)
    );
    assert_eq!(read.legacy().file.path, label);
    assert_eq!(
        read.text()
            .state(&NativeTextTarget::Vorbis {
                name: "TITLE".into()
            })
            .unwrap()
            .values()
            .unwrap()
            .as_slice(),
        [title]
    );
    assert_eq!(
        read.text()
            .state(&NativeTextTarget::Vorbis {
                name: "CUSTOM".into()
            })
            .unwrap()
            .values()
            .unwrap()
            .as_slice(),
        ["", "尾"]
    );
}
#[test]
fn nonexistent_logical_label_is_never_opened_and_retained_ownership_releases() {
    let f = Fixture::new();
    let actual = f.put("actual.bin", "Held 🎵");
    let label = f.0.join("missing.flac");
    let before = fs::read(&actual).unwrap();
    let resource = Budget::process();
    let baseline = resource.usage().0;
    let read = TagEngine::read_exact_file(File::open(&actual).unwrap(), &label).unwrap();
    verify(&read, "Held 🎵", &label);
    assert!(!label.exists());
    assert_eq!(fs::read(&actual).unwrap(), before);
    assert!(resource.usage().0.retained > baseline.retained);
    drop(read);
    assert_eq!(resource.usage().0, baseline);
}
#[test]
fn existing_foreign_label_does_not_supply_any_legacy_or_native_values() {
    let f = Fixture::new();
    let actual = f.put("actual.flac", "Held");
    let label = f.put("foreign.flac", "Foreign");
    let before_actual = fs::read(&actual).unwrap();
    let before_foreign = fs::read(&label).unwrap();
    verify(
        &TagEngine::read_exact_file(File::open(&actual).unwrap(), &label).unwrap(),
        "Held",
        &label,
    );
    assert_eq!(
        TagEngine::read(&label).unwrap().tags[&TagField::Title],
        "Foreign"
    );
    assert_eq!(fs::read(&actual).unwrap(), before_actual);
    assert_eq!(fs::read(&label).unwrap(), before_foreign);
}
#[test]
fn nested_owned_reads_and_public_legacy_requests_keep_independent_authority() {
    let f = Fixture::new();
    let actual = f.put("actual.flac", "Outer");
    let inner = f.put("inner.flac", "Inner");
    let label = f.put("label.flac", "Public");
    let calls = Cell::new(0usize);
    TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
        calls.set(calls.get() + 1);
        false
    })
    .unwrap();
    let total = calls.get();
    let calls = Cell::new(0usize);
    let nested = Cell::new(false);
    let outer =
        TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
            calls.set(calls.get() + 1);
            if calls.get() == total - 1 {
                verify(
                    &TagEngine::read_exact_file(File::open(&inner).unwrap(), &label).unwrap(),
                    "Inner",
                    &label,
                );
                assert_eq!(
                    TagEngine::read(&label).unwrap().tags[&TagField::Title],
                    "Public"
                );
                nested.set(true);
            }
            false
        })
        .unwrap();
    assert!(nested.get());
    verify(&outer, "Outer", &label);
    assert_eq!(
        TagEngine::read(&label).unwrap().tags[&TagField::Title],
        "Public"
    );
}
#[test]
fn cancellation_and_unwind_drop_bound_source_and_all_work_owners() {
    let f = Fixture::new();
    let actual = f.put("actual.flac", "Held");
    let label = f.put("label.flac", "Public");
    let resource = Budget::process();
    let baseline = resource.usage().0;
    let calls = Cell::new(0usize);
    TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
        calls.set(calls.get() + 1);
        false
    })
    .unwrap();
    let total = calls.get();
    for stop in 1..=total {
        let calls = Cell::new(0usize);
        let result =
            TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
                calls.set(calls.get() + 1);
                calls.get() == stop
            });
        assert!(
            matches!(result, Err(ExactReadError::Cancelled)),
            "cancel checkpoint {stop}"
        );
        assert_eq!(resource.usage().0, baseline);
    }
    let calls = Cell::new(0usize);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
            calls.set(calls.get() + 1);
            if calls.get() == total - 1 {
                panic!("owned held-reader fault");
            }
            false
        })
        .unwrap();
    }));
    assert!(panic.is_err());
    assert_eq!(resource.usage().0, baseline);
    assert_eq!(
        TagEngine::read(&label).unwrap().tags[&TagField::Title],
        "Public"
    );
}
#[test]
fn admission_refusal_precedes_source_io_and_releases_the_consumed_capability() {
    let f = Fixture::new();
    let actual = f.put("actual.flac", "Held");
    let label = f.0.join("missing.flac");
    let mut monitor = File::open(&actual).unwrap();
    monitor.seek(SeekFrom::Start(7)).unwrap();
    let resource = Budget::process();
    let baseline = resource.usage().0;
    let pressure = resource
        .try_acquire(Charge {
            jobs: 8 - baseline.jobs,
            ..Default::default()
        })
        .unwrap();
    let result = TagEngine::read_exact_file(monitor.try_clone().unwrap(), &label);
    assert!(matches!(result, Err(ExactReadError::Resource(_))));
    assert_eq!(monitor.stream_position().unwrap(), 7);
    drop(pressure);
    assert_eq!(resource.usage().0, baseline);
    verify(
        &TagEngine::read_exact_file(monitor, &label).unwrap(),
        "Held",
        &label,
    );
}
#[test]
fn changed_held_source_is_not_returned_as_a_mixed_snapshot() {
    let f = Fixture::new();
    let actual = f.put("actual.flac", "First");
    let label = f.0.join("label.flac");
    let calls = Cell::new(0usize);
    TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
        calls.set(calls.get() + 1);
        false
    })
    .unwrap();
    let total = calls.get();
    let calls = Cell::new(0usize);
    let result =
        TagEngine::read_exact_file_with_cancel(File::open(&actual).unwrap(), &label, || {
            calls.set(calls.get() + 1);
            if calls.get() == total - 1 {
                fs::write(&actual, flac("Other")).unwrap();
            }
            false
        });
    assert!(matches!(result, Err(ExactReadError::Changed)));
    assert_eq!(fs::read(&actual).unwrap(), flac("Other"));
}
#[test]
fn directories_write_only_handles_and_oversized_labels_refuse_without_writes() {
    let f = Fixture::new();
    let actual = f.put("actual.flac", "Held");
    let before = fs::read(&actual).unwrap();
    #[cfg(unix)]
    assert!(
        TagEngine::read_exact_file(File::open(&f.0).unwrap(), Path::new("directory.flac")).is_err()
    );
    let write_only = OpenOptions::new().write(true).open(&actual).unwrap();
    assert!(TagEngine::read_exact_file(write_only, Path::new("write-only.flac")).is_err());
    assert!(matches!(
        TagEngine::read_exact_file(File::open(&actual).unwrap(), Path::new(&"x".repeat(4097))),
        Err(ExactReadError::Limit)
    ));
    assert_eq!(fs::read(&actual).unwrap(), before);
}
