//! Independent exact-read probes using raw, disposable native envelopes.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    model::{
        exact::{ExactReadError, ExactTextState, NativeTextTarget},
        TagEngine, TagField,
    },
    resources::Budget,
};

static CASE: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-exact-adversarial-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn flac(records: &[&str]) -> Vec<u8> {
    let mut file = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    file.extend(info);
    let mut comment = 0u32.to_le_bytes().to_vec();
    comment.extend((records.len() as u32).to_le_bytes());
    for record in records {
        comment.extend((record.len() as u32).to_le_bytes());
        comment.extend(record.as_bytes());
    }
    file.extend([
        0x84,
        (comment.len() >> 16) as u8,
        (comment.len() >> 8) as u8,
        comment.len() as u8,
    ]);
    file.extend(comment);
    file.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    file
}
fn target(name: &str) -> NativeTextTarget {
    NativeTextTarget::Vorbis { name: name.into() }
}

#[test]
#[cfg(unix)]
fn changing_owned_parent_link_cannot_mix_legacy_and_exact_snapshots() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    for (name, title) in [("a", "TITLE=First"), ("b", "TITLE=Other")] {
        std::fs::create_dir(fixture.0.join(name)).unwrap();
        std::fs::write(fixture.0.join(name).join("track.flac"), flac(&[title])).unwrap();
    }
    let link = fixture.0.join("selected");
    std::os::unix::fs::symlink("a", &link).unwrap();
    let path = link.join("track.flac");
    let calls = std::cell::Cell::new(0usize);
    drop(
        TagEngine::read_exact_with_cancel(&path, || {
            calls.set(calls.get() + 1);
            false
        })
        .unwrap(),
    );
    let total = calls.get();
    let calls = std::cell::Cell::new(0usize);
    let result = TagEngine::read_exact_with_cancel(&path, || {
        let count = calls.get() + 1;
        calls.set(count);
        if count == total - 2 {
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink("b", &link).unwrap();
        }
        if count == total - 1 {
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink("a", &link).unwrap();
        }
        false
    });
    if let Ok(read) = result {
        let exact = read.text().state(&target("TITLE")).unwrap();
        let exact = &exact.values().unwrap().as_slice()[0];
        let legacy = read.legacy().file.tags.get(&TagField::Title).unwrap();
        assert_eq!(
            exact, legacy,
            "one returned read must describe the same file in both projections"
        );
    }
}

#[test]
fn repeated_empty_unicode_native_values_and_intent_ownership_survive_snapshot_drop() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let bytes = flac(&[
        "TITLE=Name",
        "CUSTOM=",
        "CUSTOM= 日本語; A/B ",
        "CUSTOM=",
        "CUSTOM= 日本語; A/B ",
    ]);
    let path = fixture.put("track.flac", &bytes);
    let budget = Budget::process();
    let before = budget.usage().0;
    let read = TagEngine::read_exact(&path).unwrap();
    assert_eq!(
        read.text()
            .state(&target("CUSTOM"))
            .unwrap()
            .values()
            .unwrap()
            .as_slice(),
        ["", " 日本語; A/B ", "", " 日本語; A/B "]
    );
    let patch = read
        .text()
        .prepare_patch(
            &target("CUSTOM"),
            ExactTextState::try_from(vec!["".into(), "A; B".into()]).unwrap(),
        )
        .unwrap();
    drop(read);
    assert!(budget.usage().0.retained > before.retained);
    assert_eq!(patch.old().values().unwrap().as_slice().len(), 4);
    assert!(!patch.executable());
    drop(patch);
    assert_eq!(budget.usage().0, before);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn bounded_deterministic_native_mutations_never_panic_or_leak_admission() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let original = flac(&["TITLE=Music", "CUSTOM=A; B", "CUSTOM=", "CUSTOM=尾"]);
    let path = fixture.put("mutation.flac", &original);
    let budget = Budget::process();
    let baseline = budget.usage().0;
    let mut random = 0x5eeda11ce_u64;
    for iteration in 0..2000 {
        let mut bytes = original.clone();
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let offset = (random as usize) % bytes.len();
        match iteration % 4 {
            0 => bytes.truncate(offset),
            1 => bytes[offset] ^= (random >> 32) as u8,
            2 => {
                let stop = (offset + 4).min(bytes.len());
                bytes[offset..stop].fill(0xff);
            }
            _ => bytes.insert(offset, 0),
        }
        std::fs::write(&path, &bytes).unwrap();
        let result = std::panic::catch_unwind(|| TagEngine::read_exact(&path));
        assert!(result.is_ok(), "native mutation {iteration} panicked");
        drop(result);
        assert_eq!(
            budget.usage().0,
            baseline,
            "native mutation {iteration} leaked admission"
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn cancellation_at_every_read_checkpoint_returns_no_partial_owner() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let path = fixture.put("cancel.flac", &flac(&["TITLE=Music", "CUSTOM=A; B"]));
    let budget = Budget::process();
    let before = budget.usage().0;
    let count = std::cell::Cell::new(0usize);
    drop(
        TagEngine::read_exact_with_cancel(&path, || {
            count.set(count.get() + 1);
            false
        })
        .unwrap(),
    );
    for checkpoint in 1..=count.get() {
        let current = std::cell::Cell::new(0usize);
        let result = TagEngine::read_exact_with_cancel(&path, || {
            current.set(current.get() + 1);
            current.get() >= checkpoint
        });
        assert!(
            matches!(result, Err(ExactReadError::Cancelled)),
            "checkpoint {checkpoint}: {result:?}"
        );
        assert_eq!(budget.usage().0, before);
    }
}
