//! Independent public exact-read expansion probes; all inputs are disposable.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use tagrex_core::{model::TagEngine, resources::Budget};
struct Observed;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn grow(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}
// SAFETY: all pointer/layout work delegates unchanged to System. Counters only
// observe successful allocations and do not alter allocator behavior.
unsafe impl GlobalAlloc for Observed {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grow(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            grow(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(ptr, layout, size) };
        if !next.is_null() {
            if size >= layout.size() {
                grow(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: Observed = Observed;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-exact-expansion-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn isolated(name: &str) -> Option<Fixture> {
    if std::env::var("TAGREX_EXACT_EXPANSION_CASE").as_deref() == Ok(name) {
        #[cfg(target_os = "linux")]
        {
            use rustix::process::{getrlimit, setrlimit, Resource, Rlimit};
            let old = getrlimit(Resource::As);
            setrlimit(
                Resource::As,
                Rlimit {
                    current: Some(old.current.unwrap_or(u64::MAX).min(1 << 30)),
                    maximum: old.maximum,
                },
            )
            .unwrap();
        }
        return Some(Fixture(PathBuf::from(
            std::env::var_os("TAGREX_EXACT_EXPANSION_ROOT").unwrap(),
        )));
    }
    let fixture = Fixture::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_EXACT_EXPANSION_CASE", name)
        .env("TAGREX_EXACT_EXPANSION_ROOT", &fixture.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("owned exact-read probe exceeded45s");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
    None
}
fn syncsafe(n: usize) -> [u8; 4] {
    [
        ((n >> 21) & 127) as u8,
        ((n >> 14) & 127) as u8,
        ((n >> 7) & 127) as u8,
        (n & 127) as u8,
    ]
}
fn frame(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend(syncsafe(body.len()));
    out.extend([0, 0]);
    out.extend(body);
    out
}
fn mp3(frames: &[Vec<u8>]) -> Vec<u8> {
    let len = frames.iter().map(Vec::len).sum::<usize>() + 16;
    let mut out = b"ID3\x04\0\0".to_vec();
    out.extend(syncsafe(len));
    for frame in frames {
        out.extend(frame);
    }
    out.extend([0; 16]);
    for _ in 0..3 {
        out.extend([0xff, 0xfb, 0x90, 0]);
        out.extend([0; 413]);
    }
    out
}
#[derive(Clone, Copy)]
enum Expect {
    Refused,
    Unavailable,
    Readable,
}
fn measured_result(path: &Path, label: &str, expect: Expect) {
    let before = fs::read(path).unwrap();
    let resource = Budget::process();
    let baseline = resource.usage().0;
    let allocated = LIVE.load(Ordering::Relaxed);
    PEAK.store(allocated, Ordering::Relaxed);
    let result = TagEngine::read_exact(path);
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(allocated);
    let high = resource.usage().1;
    let admitted = high.transient + high.retained - baseline.transient - baseline.retained;
    // These bounded probes must fit the transient reservation alone. The
    // process high water is monotonic across subcases, so do not allow a prior
    // retained result to raise the actual-allocation ceiling of a later read.
    let ceiling = admitted.min(64 * 1024 * 1024);
    println!(
        "EXACT_EXPANSION {label} bytes={} peak={peak} budget_high_water={admitted} ceiling={ceiling} refused={}",
        before.len(),
        result.is_err()
    );
    if matches!(expect, Expect::Refused) {
        assert!(
            result.is_err(),
            "oversized editable native state must refuse"
        );
    } else if matches!(expect, Expect::Unavailable) {
        if let Ok(read) = &result {
            assert!(
                read.text().unavailable().is_some()
                    || read
                        .text()
                        .entries()
                        .iter()
                        .any(|entry| entry.state.is_err()),
                "unsupported large native state must not claim exact editability"
            );
        }
    } else {
        let read = result
            .as_ref()
            .expect("admitted native text stays readable");
        assert!(read.text().unavailable().is_none());
        assert!(read
            .text()
            .entries()
            .iter()
            .all(|entry| entry.state.is_ok()));
    }
    drop(result);
    assert_eq!(resource.usage().0, baseline);
    assert_eq!(fs::read(path).unwrap(), before);
    assert!(
        peak <= ceiling,
        "exact compatibility expansion peak {peak} exceeded ceiling {ceiling} for {label}"
    );
}
#[test]
fn nul_dense_duplicate_text_frames_refuse_within_exact_read_admission() {
    let name = "nul_dense_duplicate_text_frames_refuse_within_exact_read_admission";
    let Some(f) = isolated(name) else {
        return;
    };
    let mut body = vec![0; 900_001];
    body[0] = 3;
    body[900_000] = b'X';
    let bytes = mp3(&(0..6).map(|_| frame(b"TIT2", &body)).collect::<Vec<_>>());
    let path = f.0.join("duplicate.mp3");
    fs::write(&path, bytes).unwrap();
    measured_result(&path, "duplicateTIT2", Expect::Refused);
}
#[test]
fn nul_dense_distinct_text_frames_refuse_within_exact_read_admission() {
    let name = "nul_dense_distinct_text_frames_refuse_within_exact_read_admission";
    let Some(f) = isolated(name) else {
        return;
    };
    let mut body = vec![0; 900_001];
    body[0] = 3;
    body[900_000] = b'X';
    let frames = [b"TIT2", b"TPE1", b"TALB", b"TPE2", b"TCON", b"TCOM"]
        .iter()
        .map(|id| frame(id, &body))
        .collect::<Vec<_>>();
    let path = f.0.join("distinct.mp3");
    fs::write(&path, mp3(&frames)).unwrap();
    measured_result(&path, "distinctText", Expect::Refused);
}

#[test]
fn utf16_nul_dense_text_refuses_before_legacy_expansion() {
    let name = "utf16_nul_dense_text_refuses_before_legacy_expansion";
    let Some(f) = isolated(name) else {
        return;
    };
    let mut body = vec![1, 0xff, 0xfe];
    body.extend(std::iter::repeat_n(0, 600_000));
    body.extend([b'X', 0]);
    let frames = (0..6).map(|_| frame(b"TIT2", &body)).collect::<Vec<_>>();
    let path = f.0.join("utf16.mp3");
    fs::write(&path, mp3(&frames)).unwrap();
    measured_result(&path, "UTF16duplicateTIT2", Expect::Unavailable);
}
#[test]
fn qualified_custom_text_refuses_before_legacy_expansion() {
    let name = "qualified_custom_text_refuses_before_legacy_expansion";
    let Some(f) = isolated(name) else {
        return;
    };
    let frames = (0..6)
        .map(|i| {
            let mut body = vec![3];
            body.extend(format!("description-{i}").bytes());
            body.push(0);
            body.extend(std::iter::repeat_n(0, 200_000));
            body.push(b'X');
            frame(b"TXXX", &body)
        })
        .collect::<Vec<_>>();
    let path = f.0.join("qualified.mp3");
    fs::write(&path, mp3(&frames)).unwrap();
    measured_result(&path, "qualifiedTXXX", Expect::Refused);
}
#[test]
fn admitted_text_value_boundaries_and_unavailable_utf16_stay_readable() {
    use tagrex_core::model::exact::{NativeTextTarget, UnavailableReason};
    let name = "admitted_text_value_boundaries_and_unavailable_utf16_stay_readable";
    let Some(f) = isolated(name) else {
        return;
    };
    for count in [1, 1023, 1024] {
        let mut title = vec![3];
        title.extend(std::iter::repeat_n(0, count - 1));
        title.push(b'X');
        let mut custom = vec![3];
        custom.extend(b"desc\0");
        custom.extend(std::iter::repeat_n(0, count - 1));
        custom.push(b'Y');
        let bytes = mp3(&[frame(b"TIT2", &title), frame(b"TXXX", &custom)]);
        let path = f.0.join(format!("valid-{count}.mp3"));
        fs::write(&path, &bytes).unwrap();
        let budget = Budget::process();
        let baseline = budget.usage().0;
        let read = TagEngine::read_exact(&path).unwrap();
        for (name, last) in [("TIT2", "X"), ("TXXX", "Y")] {
            let entry = read
                .text()
                .entries()
                .iter()
                .find(
                    |entry| matches!(&entry.target,NativeTextTarget::Id3 {frame,..} if frame==name),
                )
                .unwrap();
            let values = entry.state.as_ref().unwrap().values().unwrap().as_slice();
            assert_eq!(values.len(), count);
            assert_eq!(values.last().unwrap(), last);
            assert!(values[..count - 1].iter().all(String::is_empty));
        }
        drop(read);
        assert_eq!(budget.usage().0, baseline);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    let bytes = mp3(&[frame(b"TIT2", b"\x01\xff\xfeW\0i\0d\0e\0")]);
    let path = f.0.join("small-utf16.mp3");
    fs::write(&path, &bytes).unwrap();
    let read = TagEngine::read_exact(&path).unwrap();
    assert!(read
        .text()
        .entries()
        .iter()
        .any(|entry| entry.state == Err(UnavailableReason::UnsupportedEncoding)));
    assert_eq!(fs::read(path).unwrap(), bytes);
}

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut raw = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    raw.extend(kind);
    raw.extend(body);
    raw
}
fn replace_ilst(bytes: &[u8], ilst: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && at + size <= bytes.len());
        let kind: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
        let payload = &bytes[at + 8..at + size];
        let body = match &kind {
            b"ilst" => ilst.to_vec(),
            b"moov" | b"udta" => replace_ilst(payload, ilst),
            b"meta" => {
                let mut body = payload[..4].to_vec();
                body.extend(replace_ilst(&payload[4..], ilst));
                body
            }
            _ => payload.to_vec(),
        };
        result.extend(atom(&kind, &body));
        at += size;
    }
    result
}
#[test]
fn oversized_mp4_text_refuses_within_exact_read_admission() {
    let name = "oversized_mp4_text_refuses_within_exact_read_admission";
    let Some(f) = isolated(name) else {
        return;
    };
    for kind in [1u32, 2] {
        let value = if kind == 1 {
            vec![b'x'; 15 * 1024 * 1024]
        } else {
            [8, 0].repeat(6 * 1024 * 1024)
        };
        let mut data = kind.to_be_bytes().to_vec();
        data.extend([0; 4]);
        data.extend(value);
        let ilst = atom(b"\xa9nam", &atom(b"data", &data));
        let bytes = replace_ilst(include_bytes!("fixtures/minimal.m4a"), &ilst);
        let path = f.0.join(format!("oversized-{kind}.m4a"));
        fs::write(&path, bytes).unwrap();
        measured_result(
            &path,
            if kind == 1 { "MP4UTF8" } else { "MP4UTF16" },
            Expect::Refused,
        );
    }
}
#[test]
fn flac_many_small_native_members_refuse_without_large_compatibility_expansion() {
    let name = "flac_many_small_native_members_refuse_without_large_compatibility_expansion";
    let Some(f) = isolated(name) else {
        return;
    };
    let mut bytes = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    bytes.extend(info);
    let mut comments = 0u32.to_le_bytes().to_vec();
    comments.extend(4096u32.to_le_bytes());
    for i in 0..4096 {
        let record = format!("CUSTOM={i}-{}", "x".repeat(900));
        comments.extend((record.len() as u32).to_le_bytes());
        comments.extend(record.bytes());
    }
    let n = comments.len();
    bytes.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    bytes.extend(comments);
    bytes.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    let path = f.0.join("many.flac");
    fs::write(&path, bytes).unwrap();
    measured_result(&path, "FLAC4096values", Expect::Refused);
}

#[test]
fn aligned_utf16_ascii_and_surrogates_stay_bounded_and_diagnostic() {
    let name = "aligned_utf16_ascii_and_surrogates_stay_bounded_and_diagnostic";
    let Some(f) = isolated(name) else {
        return;
    };
    for (encoding, prefix, units) in [
        (1, vec![0xff, 0xfe], [b'A', 0].repeat(100_000)),
        (2, vec![], [0, b'A'].repeat(100_000)),
        (2, vec![], [0xd8, 0x3d, 0xde, 0x00].repeat(50_000)),
    ] {
        let mut body = vec![encoding];
        body.extend(prefix);
        body.extend(units);
        let path = f.0.join(format!("aligned-{encoding}-{}.mp3", body.len()));
        fs::write(&path, mp3(&[frame(b"TIT2", &body)])).unwrap();
        measured_result(&path, "alignedUTF16", Expect::Unavailable);
        let read = TagEngine::read_exact(&path).expect("aligned UTF16 has native diagnostic");
        assert!(read
            .text()
            .entries()
            .iter()
            .any(|entry| entry.state.is_err()));
    }
}
#[test]
fn per_frame_and_aggregate_text_member_boundaries() {
    let name = "per_frame_and_aggregate_text_member_boundaries";
    let Some(f) = isolated(name) else {
        return;
    };
    for count in [1023usize, 1024, 1025, 16_383, 16_384, 16_385] {
        let mut frames = Vec::new();
        let mut remaining = count;
        while remaining > 0 {
            // The 1025 case isolates a per-frame overflow. Larger cases use
            // individually valid frames to isolate the aggregate admission.
            let members = if count == 1025 {
                remaining
            } else {
                remaining.min(1024)
            };
            let mut body = vec![3];
            body.extend(format!("description-{}", frames.len()).bytes());
            body.push(0);
            body.extend(std::iter::repeat_n(0, members - 1));
            body.push(b'X');
            frames.push(frame(b"TXXX", &body));
            remaining -= members;
        }
        let path = f.0.join(format!("members-{count}.mp3"));
        fs::write(&path, mp3(&frames)).unwrap();
        measured_result(
            &path,
            &format!("members{count}"),
            if count == 1025 || count > 16_384 {
                Expect::Refused
            } else {
                Expect::Readable
            },
        );
    }
}
#[test]
fn near_native_snapshot_text_and_large_opaque_controls_stay_bounded() {
    let name = "near_native_snapshot_text_and_large_opaque_controls_stay_bounded";
    let Some(f) = isolated(name) else {
        return;
    };
    let mut body = vec![3];
    body.extend(std::iter::repeat_n(b'x', 1024 * 1024 - 4096));
    let frames = [
        b"TIT2", b"TPE1", b"TALB", b"TPE2", b"TCON", b"TCOM", b"TCOP", b"TPUB",
    ]
    .iter()
    .map(|id| frame(id, &body))
    .collect::<Vec<_>>();
    let path = f.0.join("near-snapshot.mp3");
    fs::write(&path, mp3(&frames)).unwrap();
    measured_result(&path, "near8MiBnativeText", Expect::Readable);
    let read = TagEngine::read_exact(&path).unwrap();
    assert_eq!(read.text().entries().len(), 8);
    for entry in read.text().entries() {
        let values = entry.state.as_ref().unwrap().values().unwrap().as_slice();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].len(), 1024 * 1024 - 4096);
    }
    drop(read);
    let opaque = vec![b'x'; 14 * 1024 * 1024];
    let path = f.0.join("large-opaque.mp3");
    fs::write(
        &path,
        mp3(&[frame(b"PRIV", &opaque), frame(b"TIT2", b"\x03Title")]),
    )
    .unwrap();
    measured_result(&path, "14MiBopaque", Expect::Unavailable);
    let read = TagEngine::read_exact(&path).expect("opaque data does not hide supported title");
    assert!(read
        .text()
        .entries()
        .iter()
        .any(|entry| entry.state.as_ref().is_ok_and(|state| state
            .values()
            .is_some_and(|values| values.as_slice() == ["Title"]))));
}

#[test]
fn mp4_native_text_byte_boundaries_remain_readable_or_refuse_explicitly() {
    let name = "mp4_native_text_byte_boundaries_remain_readable_or_refuse_explicitly";
    let Some(f) = isolated(name) else {
        return;
    };
    for (label, kind, value, expected_len) in [
        (
            "utf8-native-max",
            1u32,
            vec![b'x'; 1024 * 1024],
            Some(1024 * 1024),
        ),
        ("utf8-native-over", 1u32, vec![b'x'; 1024 * 1024 + 1], None),
        (
            "utf16-native-near",
            2u32,
            [8, 0].repeat(349_525),
            Some(1_048_575),
        ),
        ("utf16-native-over", 2u32, [8, 0].repeat(349_526), None),
    ] {
        let mut data = kind.to_be_bytes().to_vec();
        data.extend([0; 4]);
        data.extend(value);
        let ilst = atom(b"\xa9nam", &atom(b"data", &data));
        let path = f.0.join(format!("{label}.m4a"));
        fs::write(
            &path,
            replace_ilst(include_bytes!("fixtures/minimal.m4a"), &ilst),
        )
        .unwrap();
        measured_result(
            &path,
            label,
            if expected_len.is_some() {
                Expect::Readable
            } else {
                Expect::Refused
            },
        );
        if let Some(len) = expected_len {
            let read = TagEngine::read_exact(&path).unwrap();
            let entries = read.text().entries();
            assert_eq!(entries.len(), 1);
            let values = entries[0]
                .state
                .as_ref()
                .unwrap()
                .values()
                .unwrap()
                .as_slice();
            assert_eq!(values.len(), 1);
            assert_eq!(values[0].len(), len);
        }
    }
}

#[test]
fn oversized_latin1_text_refuses_before_utf8_expansion() {
    let name = "oversized_latin1_text_refuses_before_utf8_expansion";
    let Some(f) = isolated(name) else {
        return;
    };
    let mut body = vec![0];
    body.extend(std::iter::repeat_n(0xff, 15 * 1024 * 1024));
    let path = f.0.join("latin1.mp3");
    fs::write(&path, mp3(&[frame(b"TIT2", &body)])).unwrap();
    measured_result(&path, "15MiBLatin1", Expect::Refused);
}

#[test]
fn latin1_decoded_native_field_boundary_preserves_text() {
    let name = "latin1_decoded_native_field_boundary_preserves_text";
    let Some(f) = isolated(name) else {
        return;
    };
    for bytes in [512 * 1024, 512 * 1024 + 1] {
        let mut body = vec![0];
        body.extend(std::iter::repeat_n(0xff, bytes));
        let path = f.0.join(format!("latin1-boundary-{bytes}.mp3"));
        fs::write(&path, mp3(&[frame(b"TIT2", &body)])).unwrap();
        measured_result(
            &path,
            "Latin1nativeBoundary",
            if bytes == 512 * 1024 {
                Expect::Readable
            } else {
                Expect::Refused
            },
        );
        if bytes == 512 * 1024 {
            let read = TagEngine::read_exact(&path).unwrap();
            let values = read.text().entries()[0]
                .state
                .as_ref()
                .unwrap()
                .values()
                .unwrap()
                .as_slice();
            assert_eq!(values.len(), 1);
            assert_eq!(values[0].len(), 1024 * 1024);
            assert!(values[0].chars().all(|ch| ch == '\u{ff}'));
        }
    }
}

#[test]
fn configured_codepage_incremental_preflight_keeps_native_semantics() {
    use tagrex_core::model::set_legacy_code_page;
    let name = "configured_codepage_incremental_preflight_keeps_native_semantics";
    let Some(f) = isolated(name) else {
        return;
    };
    for (label, payload, expected) in [
        ("windows-1252", vec![0x80; 349_525], Some((1_048_575, '€'))),
        ("windows-1252", vec![0x80; 349_526], None),
        (
            "shift_jis",
            [0x82, 0xa0].repeat(349_525),
            Some((1_048_575, 'あ')),
        ),
        ("shift_jis", vec![0x82], None),
    ] {
        set_legacy_code_page(label);
        let mut body = vec![0];
        body.extend(payload);
        let path = f.0.join(format!("codepage-{label}-{}.mp3", body.len()));
        fs::write(&path, mp3(&[frame(b"TIT2", &body)])).unwrap();
        measured_result(
            &path,
            label,
            if expected.is_some() {
                Expect::Readable
            } else {
                Expect::Refused
            },
        );
        if let Some((bytes, character)) = expected {
            let read = TagEngine::read_exact(&path).unwrap();
            let values = read.text().entries()[0]
                .state
                .as_ref()
                .unwrap()
                .values()
                .unwrap()
                .as_slice();
            assert_eq!(values.len(), 1);
            assert_eq!(values[0].len(), bytes);
            assert!(values[0].chars().all(|ch| ch == character));
        }
    }
    set_legacy_code_page("");
}
