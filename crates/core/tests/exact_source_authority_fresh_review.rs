//! Fresh held-source review through public readers and owned native fixtures.
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    model::{exact::NativeTextTarget, TagEngine, TagField},
    resources::Budget,
};

#[cfg(unix)]
use tagrex_core::model::exact::ExactReadError;

static CASE: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-exact-source-fresh-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn put(&self, name: &str, title: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, flac(title)).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn flac(title: &str) -> Vec<u8> {
    let mut bytes = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    bytes.extend(info);
    let mut comments = 0u32.to_le_bytes().to_vec();
    comments.extend(2u32.to_le_bytes());
    for record in [format!("TITLE={title}"), "CUSTOM=literal ; / 尾".into()] {
        comments.extend((record.len() as u32).to_le_bytes());
        comments.extend(record.as_bytes());
    }
    bytes.extend([
        0x84,
        (comments.len() >> 16) as u8,
        (comments.len() >> 8) as u8,
        comments.len() as u8,
    ]);
    bytes.extend(comments);
    bytes.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    bytes
}
fn checkpoints(path: &Path) -> usize {
    let calls = Cell::new(0usize);
    drop(
        TagEngine::read_exact_with_cancel(path, || {
            calls.set(calls.get() + 1);
            false
        })
        .unwrap(),
    );
    calls.get()
}
fn exact_title(read: &tagrex_core::model::exact::ExactTrackRead) -> String {
    read.text()
        .state(&NativeTextTarget::Vorbis {
            name: "TITLE".into(),
        })
        .unwrap()
        .values()
        .unwrap()
        .as_slice()[0]
        .clone()
}
#[cfg(unix)]
fn select(link: &Path, target: &str) {
    if link.symlink_metadata().is_ok() {
        std::fs::remove_file(link).unwrap();
    }
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(unix)]
#[test]
fn reentrant_public_legacy_read_uses_its_own_current_source() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let first = fixture.put("a/track.flac", "Outer");
    let other = fixture.put("b/track.flac", "Nested");
    let link = fixture.0.join("selected");
    select(&link, "a");
    let path = link.join("track.flac");
    let total = checkpoints(&path);
    let baseline = Budget::process().usage().0;
    let count = Cell::new(0usize);
    let nested_title = std::cell::RefCell::new(None);
    let outer = TagEngine::read_exact_with_cancel(&path, || {
        count.set(count.get() + 1);
        if count.get() == total - 2 {
            select(&link, "b");
            let nested = TagEngine::read(&path).unwrap();
            *nested_title.borrow_mut() = nested.tags.get(&TagField::Title).cloned();
            select(&link, "a");
        }
        false
    })
    .unwrap();
    assert_eq!(exact_title(&outer), "Outer");
    assert_eq!(outer.legacy().file.tags[&TagField::Title], "Outer");
    drop(outer);
    assert_eq!(Budget::process().usage().0, baseline);
    assert_eq!(std::fs::read(first).unwrap(), flac("Outer"));
    assert_eq!(std::fs::read(other).unwrap(), flac("Nested"));
    assert_eq!(
        nested_title.into_inner().as_deref(),
        Some("Nested"),
        "a new public legacy read must not inherit another request's held source"
    );
}

#[cfg(unix)]
#[test]
fn nested_exact_same_logical_path_restores_outer_cursor_and_authority_after_error() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    fixture.put("a/track.flac", "Outer");
    fixture.put("b/track.flac", "Nested");
    let link = fixture.0.join("selected");
    select(&link, "a");
    let path = link.join("track.flac");
    let total = checkpoints(&path);
    let baseline = Budget::process().usage().0;
    for mode in 0..3 {
        let count = Cell::new(0usize);
        let outer = TagEngine::read_exact_with_cancel(&path, || {
            count.set(count.get() + 1);
            if count.get() == total - 2 {
                select(&link, "b");
                let nested_count = Cell::new(0usize);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    TagEngine::read_exact_with_cancel(&path, || {
                        nested_count.set(nested_count.get() + 1);
                        if nested_count.get() == total - 1 {
                            if mode == 2 {
                                panic!("owned nested callback panic");
                            }
                            return mode == 1;
                        }
                        false
                    })
                }));
                match (mode, result) {
                    (0, Ok(Ok(read))) => assert_eq!(exact_title(&read), "Nested"),
                    (1, Ok(Err(ExactReadError::Cancelled))) | (2, Err(_)) => (),
                    other => panic!("unexpected nested outcome: {other:?}"),
                }
                select(&link, "a");
            }
            false
        })
        .unwrap();
        assert_eq!(exact_title(&outer), "Outer");
        assert_eq!(outer.legacy().file.tags[&TagField::Title], "Outer");
        drop(outer);
        assert_eq!(Budget::process().usage().0, baseline);
    }
}

#[cfg(unix)]
#[test]
fn held_source_is_thread_local_while_other_thread_reads_changed_parent() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    fixture.put("a/track.flac", "Outer");
    fixture.put("b/track.flac", "Other thread");
    let link = fixture.0.join("selected");
    select(&link, "a");
    let path = link.join("track.flac");
    let total = checkpoints(&path);
    let baseline = Budget::process().usage().0;
    let count = Cell::new(0usize);
    let outer = TagEngine::read_exact_with_cancel(&path, || {
        count.set(count.get() + 1);
        if count.get() == total - 2 {
            select(&link, "b");
            let path = path.clone();
            let observed = std::thread::spawn(move || {
                let read = TagEngine::read_exact(&path).unwrap();
                exact_title(&read)
            })
            .join()
            .unwrap();
            assert_eq!(observed, "Other thread");
            select(&link, "a");
        }
        false
    })
    .unwrap();
    assert_eq!(exact_title(&outer), "Outer");
    drop(outer);
    assert_eq!(Budget::process().usage().0, baseline);
}

#[cfg(unix)]
#[test]
fn reentrant_public_block_reader_uses_the_new_request_path() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    fixture.put("a/track.flac", "Outer");
    fixture.put("b/track.flac", "Block request");
    let link = fixture.0.join("selected");
    select(&link, "a");
    let path = link.join("track.flac");
    let total = checkpoints(&path);
    let baseline = Budget::process().usage().0;
    let count = Cell::new(0usize);
    let nested_title = std::cell::RefCell::new(None);
    let outer = TagEngine::read_exact_with_cancel(&path, || {
        count.set(count.get() + 1);
        if count.get() == total - 2 {
            select(&link, "b");
            let block =
                TagEngine::read_block(&path, tagrex_core::model::TagBlockKind::VorbisComments)
                    .unwrap()
                    .unwrap();
            *nested_title.borrow_mut() = block.tags.get(&TagField::Title).cloned();
            select(&link, "a");
        }
        false
    })
    .unwrap();
    assert_eq!(exact_title(&outer), "Outer");
    drop(outer);
    assert_eq!(Budget::process().usage().0, baseline);
    assert_eq!(
        nested_title.into_inner().as_deref(),
        Some("Block request"),
        "public block read inherited the outer request's held source"
    );
}

#[cfg(unix)]
#[test]
fn failing_reentrant_legacy_request_restores_the_outer_source_before_parser_runs() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    fixture.put("a/track.flac", "Outer");
    let invalid = fixture.put("b/track.flac", "Malformed");
    std::fs::write(&invalid, b"invalid owned fixture").unwrap();
    fixture.put("c/track.flac", "Other visible source");
    let link = fixture.0.join("selected");
    select(&link, "a");
    let path = link.join("track.flac");
    let total = checkpoints(&path);
    let baseline = Budget::process().usage().0;
    let count = Cell::new(0usize);
    let outer = TagEngine::read_exact_with_cancel(&path, || {
        count.set(count.get() + 1);
        if count.get() == total - 2 {
            select(&link, "b");
            assert!(
                TagEngine::read(&path).is_err(),
                "new malformed request read the old source"
            );
            select(&link, "c");
        }
        if count.get() == total - 1 {
            select(&link, "a");
        }
        false
    })
    .unwrap();
    assert_eq!(exact_title(&outer), "Outer");
    assert_eq!(outer.legacy().file.tags[&TagField::Title], "Outer");
    drop(outer);
    assert_eq!(Budget::process().usage().0, baseline);
}

#[cfg(unix)]
#[test]
fn non_utf8_logical_names_and_repeated_nested_same_source_reads_do_not_share_cursor_state() {
    use std::os::unix::ffi::OsStrExt;
    fn read(
        path: &Path,
        backing: Option<&Path>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<tagrex_core::model::exact::ExactTrackRead, ExactReadError> {
        if let Some(backing) = backing {
            TagEngine::read_exact_file_with_cancel(
                std::fs::File::open(backing).unwrap(),
                path,
                cancelled,
            )
        } else {
            TagEngine::read_exact_with_cancel(path, cancelled)
        }
    }
    fn verify(path: &Path, backing: Option<&Path>) {
        let calls = Cell::new(0usize);
        read(path, backing, &|| {
            calls.set(calls.get() + 1);
            false
        })
        .unwrap();
        let total = calls.get();
        let baseline = Budget::process().usage().0;
        for _ in 0..24 {
            let count = Cell::new(0usize);
            let outer = read(path, backing, &|| {
                count.set(count.get() + 1);
                if count.get() == total - 2 {
                    for _ in 0..4 {
                        let inner = read(path, backing, &|| false).unwrap();
                        assert_eq!(exact_title(&inner), "Exact non-UTF8 path");
                        assert_eq!(
                            TagEngine::read(backing.unwrap_or(path)).unwrap().tags[&TagField::Title],
                            "Exact non-UTF8 path"
                        );
                    }
                }
                false
            })
            .unwrap();
            assert_eq!(exact_title(&outer), "Exact non-UTF8 path");
            assert_eq!(
                outer.legacy().file.tags[&TagField::Title],
                "Exact non-UTF8 path"
            );
            drop(outer);
            assert_eq!(Budget::process().usage().0, baseline);
        }
    }
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let path = fixture
        .0
        .join(std::ffi::OsStr::from_bytes(b"native-\xff.flac"));
    let bytes = flac("Exact non-UTF8 path");
    let backing = fixture.put("valid-backing.flac", "Exact non-UTF8 path");
    // A logical name need not be representable on the backing filesystem.
    // Always retain this capability contract, including on APFS.
    verify(&path, Some(&backing));
    match std::fs::write(&path, &bytes) {
        Ok(()) => {
            verify(&path, None);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        Err(error) if cfg!(target_vendor = "apple") && error.raw_os_error() == Some(92) => {
            // APFS rejects invalid UTF-8 filenames with EILSEQ before any file
            // exists; the held logical-name case above still runs in full.
            assert!(std::fs::symlink_metadata(&path).is_err());
        }
        Err(error) => panic!("could not create native-byte filename: {error}"),
    }
    assert_eq!(std::fs::read(backing).unwrap(), bytes);
}

#[cfg(unix)]
#[test]
fn reentrant_public_property_readers_observe_their_own_pcm_source() {
    fn wave(rate: u32, seconds: u32, title: &str) -> Vec<u8> {
        let payload = [vec![3], title.as_bytes().to_vec()].concat();
        let frame_len = payload.len() as u32;
        let syncsafe = |n: u32| {
            [
                ((n >> 21) & 127) as u8,
                ((n >> 14) & 127) as u8,
                ((n >> 7) & 127) as u8,
                (n & 127) as u8,
            ]
        };
        let mut frame = b"TIT2".to_vec();
        frame.extend(syncsafe(frame_len));
        frame.extend([0, 0]);
        frame.extend(payload);
        let mut id3 = b"ID3\x04\0\0".to_vec();
        id3.extend(syncsafe(frame.len() as u32));
        id3.extend(frame);
        let mut body = b"WAVEfmt ".to_vec();
        body.extend(16u32.to_le_bytes());
        body.extend(1u16.to_le_bytes());
        body.extend(1u16.to_le_bytes());
        body.extend(rate.to_le_bytes());
        body.extend((rate * 2).to_le_bytes());
        body.extend(2u16.to_le_bytes());
        body.extend(16u16.to_le_bytes());
        body.extend(b"data");
        let samples = rate * seconds * 2;
        body.extend(samples.to_le_bytes());
        body.resize(body.len() + samples as usize, 0);
        body.extend(b"id3 ");
        body.extend((id3.len() as u32).to_le_bytes());
        body.extend(&id3);
        if id3.len() % 2 == 1 {
            body.push(0);
        }
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((body.len() as u32).to_le_bytes());
        bytes.extend(body);
        bytes
    }
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    for (dir, rate, seconds, title) in [("a", 8000, 3, "Outer PCM"), ("b", 16000, 4, "Nested PCM")]
    {
        std::fs::create_dir(fixture.0.join(dir)).unwrap();
        std::fs::write(
            fixture.0.join(dir).join("track.wav"),
            wave(rate, seconds, title),
        )
        .unwrap();
    }
    let link = fixture.0.join("selected");
    select(&link, "a");
    let path = link.join("track.wav");
    let total = checkpoints(&path);
    let baseline = Budget::process().usage().0;
    let count = Cell::new(0usize);
    let outer = TagEngine::read_exact_with_cancel(&path, || {
        count.set(count.get() + 1);
        if count.get() == total - 2 {
            select(&link, "b");
            assert_eq!(
                TagEngine::read_audio_props(&path).unwrap().sample_rate_hz,
                Some(16000)
            );
            assert_eq!(TagEngine::read_duration(&path).unwrap().as_secs(), 4);
            let nested = TagEngine::read_with_props(&path).unwrap();
            assert_eq!(nested.props.sample_rate_hz, Some(16000));
            assert_eq!(nested.file.tags[&TagField::Title], "Nested PCM");
            select(&link, "a");
        }
        false
    })
    .unwrap();
    assert_eq!(outer.legacy().file.tags[&TagField::Title], "Outer PCM");
    assert_eq!(outer.legacy().props.sample_rate_hz, Some(8000));
    drop(outer);
    assert_eq!(Budget::process().usage().0, baseline);
}

#[test]
fn portable_nested_public_readers_leave_outer_native_and_legacy_sources_consistent() {
    let _case = CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let path = fixture.put("outer.flac", "Portable outer");
    let nested_path = fixture.put("nested.flac", "Portable nested");
    let total = checkpoints(&path);
    let baseline = Budget::process().usage().0;
    for _ in 0..12 {
        let count = Cell::new(0usize);
        let outer = TagEngine::read_exact_with_cancel(&path, || {
            count.set(count.get() + 1);
            if count.get() == total - 2 {
                let nested = TagEngine::read_exact(&nested_path).unwrap();
                assert_eq!(exact_title(&nested), "Portable nested");
                let same = TagEngine::read_exact(&path).unwrap();
                assert_eq!(exact_title(&same), "Portable outer");
                assert_eq!(
                    TagEngine::read_with_props(&nested_path).unwrap().file.tags[&TagField::Title],
                    "Portable nested"
                );
                assert_eq!(
                    TagEngine::read(&path).unwrap().tags[&TagField::Title],
                    "Portable outer"
                );
                assert!(TagEngine::read(&fixture.0).is_err());
                let other_thread = nested_path.clone();
                let title = std::thread::spawn(move || {
                    TagEngine::read(&other_thread).unwrap().tags[&TagField::Title].clone()
                })
                .join()
                .unwrap();
                assert_eq!(title, "Portable nested");
            }
            false
        })
        .unwrap();
        assert_eq!(exact_title(&outer), "Portable outer");
        assert_eq!(outer.legacy().file.tags[&TagField::Title], "Portable outer");
        drop(outer);
        assert_eq!(Budget::process().usage().0, baseline);
    }
    assert_eq!(std::fs::read(path).unwrap(), flac("Portable outer"));
    assert_eq!(std::fs::read(nested_path).unwrap(), flac("Portable nested"));
}
