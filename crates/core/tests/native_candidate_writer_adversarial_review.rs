//! Independent public candidate-writer probes. Only disposable media is edited.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};
use tagrex_core::{
    model::{
        exact::{
            ExactTextState, Id3TextEncoding, Mp4TextKey, Mp4TextKind, NativeTextTarget,
            PreparedTextPatch,
        },
        native_patch::{self as patch, Control, Event},
        TagEngine,
    },
    resources::{Budget, Charge, Limits},
};
struct Observed;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn grow(n: usize) {
    let value = LIVE.fetch_add(n, Ordering::Relaxed) + n;
    PEAK.fetch_max(value, Ordering::Relaxed);
}
// SAFETY: this observational allocator delegates every pointer/layout operation
// unchanged to System and only records successful allocation sizes.
unsafe impl GlobalAlloc for Observed {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, n: usize) -> *mut u8 {
        let next = unsafe { System.realloc(p, layout, n) };
        if !next.is_null() {
            if n >= layout.size() {
                grow(n - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - n, Ordering::Relaxed);
            }
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: Observed = Observed;
fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_CANDIDATE_REVIEW").as_deref() == Ok(name) {
        return false;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_CANDIDATE_REVIEW", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-native-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn budget() -> Budget {
    Budget::new(Limits {
        retained: 64 << 20,
        transient: 128 << 20,
        total: 192 << 20,
        jobs: 1,
    })
}
fn rw(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap()
}
fn state(values: &[&str]) -> ExactTextState {
    ExactTextState::try_from(values.iter().map(|v| v.to_string()).collect::<Vec<_>>()).unwrap()
}
fn target(name: &str) -> NativeTextTarget {
    NativeTextTarget::Vorbis { name: name.into() }
}
fn intent(path: &Path, target: &NativeTextTarget, new: ExactTextState) -> PreparedTextPatch {
    TagEngine::read_exact(path)
        .unwrap()
        .text()
        .prepare_patch(target, new)
        .unwrap()
}
fn syncsafe(n: usize) -> [u8; 4] {
    [
        ((n >> 21) & 127) as u8,
        ((n >> 14) & 127) as u8,
        ((n >> 7) & 127) as u8,
        (n & 127) as u8,
    ]
}
fn frame(name: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = name.to_vec();
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
fn flac(records: &[(String, String)], suffix: &[u8]) -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut comments = 0u32.to_le_bytes().to_vec();
    comments.extend((records.len() as u32).to_le_bytes());
    for (k, v) in records {
        let n = k.len() + 1 + v.len();
        comments.extend((n as u32).to_le_bytes());
        comments.extend(k.as_bytes());
        comments.push(b'=');
        comments.extend(v.as_bytes());
    }
    let n = comments.len();
    out.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(comments);
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out.extend(suffix);
    out
}

#[test]
fn duplicate_nul_dense_frames_refuse_before_exceeding_work_reservation() {
    let name = "duplicate_nul_dense_frames_refuse_before_exceeding_work_reservation";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let path = f.put("candidate.mp3", &mp3(&[frame(b"TIT2", b"\x03saved")]));
    let wanted = NativeTextTarget::Id3 {
        frame: "TIT2".into(),
        description: None,
        language: None,
        encoding: Some(Id3TextEncoding::Utf8),
    };
    let prepared = intent(&path, &wanted, state(&["new"]));
    let mut dense = vec![0; 900_001];
    dense[0] = 3;
    let attack = mp3(&(0..6).map(|_| frame(b"TIT2", &dense)).collect::<Vec<_>>());
    fs::write(&path, &attack).unwrap();
    drop(dense);
    let resource = budget();
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let result =
        patch::apply_prepared_with_budget(&mut rw(&path), &prepared, resource.clone(), &mut ());
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    assert!(result.is_err());
    assert_eq!(fs::read(&path).unwrap(), attack);
    assert_eq!(resource.usage().0, Charge::default());
    eprintln!(
        "NATIVE_DUPLICATE_NUL peak_delta={peak} work_reservation={}",
        128usize << 20
    );
    assert!(
        peak <= 128usize << 20,
        "duplicate-frame expansion allocated {peak} bytes outside the 128MiB work reservation"
    );
}

fn inspect_flac(bytes: &[u8]) -> Vec<(String, String)> {
    let mut at = 4;
    loop {
        let flag = bytes[at];
        let length = ((bytes[at + 1] as usize) << 16)
            | ((bytes[at + 2] as usize) << 8)
            | bytes[at + 3] as usize;
        at += 4;
        let body = &bytes[at..at + length];
        at += length;
        if flag & 127 == 4 {
            let u32at =
                |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap()) as usize;
            let mut pos = 4 + u32at(0);
            let count = u32at(pos);
            pos += 4;
            let mut result = Vec::new();
            for _ in 0..count {
                let n = u32at(pos);
                pos += 4;
                let item = std::str::from_utf8(&body[pos..pos + n]).unwrap();
                pos += n;
                let (k, v) = item.split_once('=').unwrap();
                result.push((k.into(), v.into()));
            }
            assert_eq!(pos, body.len());
            return result;
        }
        assert_eq!(flag & 128, 0);
    }
}
#[test]
fn randomized_native_interleaving_undo_and_redo_match_raw_byte_oracle() {
    let name = "randomized_native_interleaving_undo_and_redo_match_raw_byte_oracle";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let suffix: Vec<u8> = (0..98_321).map(|i| ((i * 17 + 3) % 251) as u8).collect();
    let mut random = 0x8ad2_0fb1u32;
    for iteration in 0..160 {
        let mut items = Vec::new();
        for i in 0..(6 + iteration % 15) {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            let name = match random % 4 {
                0 => "ARTIST",
                1 => "GENRE",
                2 => "CUSTOM",
                _ => "TITLE",
            };
            let value = match (random >> 8) % 5 {
                0 => String::new(),
                1 => "A; B".into(),
                2 => "尾 / 日本語".into(),
                3 => "House".into(),
                _ => format!("{i}-{iteration}"),
            };
            items.push((name.into(), value));
        }
        if !items.iter().any(|(k, _)| k == "ARTIST") {
            items.push(("ARTIST".into(), "old".into()));
        }
        let original = flac(&items, &suffix);
        let path = f.put("random.flac", &original);
        let values = match iteration % 4 {
            0 => ExactTextState::absent(),
            1 => state(&[""]),
            2 => state(&["A; B", "", "尾", "A; B"]),
            _ => state(&["long edited value"]),
        };
        let prepared = intent(&path, &target("ARTIST"), values.clone());
        let resource = budget();
        let receipt =
            patch::apply_prepared_with_budget(&mut rw(&path), &prepared, resource.clone(), &mut ())
                .unwrap();
        let applied = fs::read(&path).unwrap();
        let mut expected = Vec::new();
        let mut first = true;
        for item in &items {
            if item.0 == "ARTIST" {
                if first {
                    for value in values.values().map_or(&[][..], |v| v.as_slice()) {
                        expected.push(("ARTIST".into(), value.clone()));
                    }
                    first = false;
                }
            } else {
                expected.push(item.clone());
            }
        }
        assert_eq!(inspect_flac(&applied), expected, "iteration {iteration}");
        let redo = patch::restore_with_budget(&mut rw(&path), &receipt, resource.clone(), &mut ())
            .unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            original,
            "Undo iteration {iteration}"
        );
        let undone =
            patch::restore_with_budget(&mut rw(&path), &redo, resource.clone(), &mut ()).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            applied,
            "Redo iteration {iteration}"
        );
        drop((receipt, redo, undone));
        assert_eq!(resource.usage().0, Charge::default());
    }
}
#[test]
fn selective_restore_keeps_unrelated_expanded_list_and_refuses_ambiguous_interior_anchor() {
    let name =
        "selective_restore_keeps_unrelated_expanded_list_and_refuses_ambiguous_interior_anchor";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let resource = budget();
    for interior in [false, true] {
        let items = if interior {
            vec![
                ("CUSTOM".into(), "first".into()),
                ("TITLE".into(), "old".into()),
                ("CUSTOM".into(), "second".into()),
            ]
        } else {
            vec![
                ("CUSTOM".into(), "first".into()),
                ("CUSTOM".into(), "second".into()),
                ("TITLE".into(), "old".into()),
            ]
        };
        let original = flac(&items, b"independent audio suffix");
        let path = f.put("anchor.flac", &original);
        let title = intent(&path, &target("TITLE"), state(&["new"]));
        let receipt =
            patch::apply_prepared_with_budget(&mut rw(&path), &title, resource.clone(), &mut ())
                .unwrap();
        let custom = intent(
            &path,
            &target("CUSTOM"),
            state(&["updated", "", "別", "fourth"]),
        );
        let current_receipt =
            patch::apply_prepared_with_budget(&mut rw(&path), &custom, resource.clone(), &mut ())
                .unwrap();
        let before = fs::read(&path).unwrap();
        let result =
            patch::restore_with_budget(&mut rw(&path), &receipt, resource.clone(), &mut ());
        if interior {
            assert!(matches!(result, Err(patch::Error::Ambiguous)));
            assert_eq!(fs::read(&path).unwrap(), before);
        } else {
            let restored = result.unwrap();
            let records = inspect_flac(&fs::read(&path).unwrap());
            assert_eq!(
                records
                    .iter()
                    .filter(|(k, _)| k == "CUSTOM")
                    .map(|(_, v)| v.as_str())
                    .collect::<Vec<_>>(),
                ["updated", "", "別", "fourth"]
            );
            assert_eq!(records.last(), Some(&("TITLE".into(), "old".into())));
            drop(restored);
        }
        drop((receipt, current_receipt));
        assert_eq!(resource.usage().0, Charge::default());
    }
}
#[derive(Default)]
struct Events(Vec<Event>);
impl Control for Events {
    fn event(&mut self, event: Event) -> std::io::Result<()> {
        self.0.push(event);
        Ok(())
    }
}
struct Interrupt {
    at: usize,
    seen: usize,
    panic: bool,
}
impl Control for Interrupt {
    fn event(&mut self, _: Event) -> std::io::Result<()> {
        self.seen += 1;
        if self.seen == self.at {
            if self.panic {
                panic!("owned candidate interruption");
            }
            return Err(std::io::Error::other("owned candidate IO interruption"));
        }
        Ok(())
    }
}
#[test]
fn every_observed_event_error_or_unwind_releases_resources_and_never_touches_original() {
    let name = "every_observed_event_error_or_unwind_releases_resources_and_never_touches_original";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let original = flac(
        &[
            ("TITLE".into(), "old".repeat(3000)),
            ("CUSTOM".into(), "opaque literal".into()),
        ],
        &vec![21; 160_000],
    );
    let saved = f.put("original.flac", &original);
    let path = f.put("candidate.flac", &original);
    let prepared = intent(&path, &target("TITLE"), state(&["new"]));
    let resource = budget();
    let mut events = Events::default();
    let receipt =
        patch::apply_prepared_with_budget(&mut rw(&path), &prepared, resource.clone(), &mut events)
            .unwrap();
    drop(receipt);
    assert!(events.0.iter().any(|e| matches!(e, Event::Written(_))));
    let event_count = events.0.len();
    for panic in [false, true] {
        for at in 1..=event_count {
            fs::write(&path, &original).unwrap();
            let mut control = Interrupt { at, seen: 0, panic };
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                patch::apply_prepared_with_budget(
                    &mut rw(&path),
                    &prepared,
                    resource.clone(),
                    &mut control,
                )
            }));
            if panic {
                assert!(outcome.is_err());
            } else {
                assert!(outcome.unwrap().is_err());
            }
            assert_eq!(fs::read(&saved).unwrap(), original);
            assert_eq!(resource.usage().0, Charge::default());
            if !events.0[..at]
                .iter()
                .any(|e| matches!(e, Event::Written(_) | Event::Resized))
            {
                assert_eq!(fs::read(&path).unwrap(), original);
            }
        }
    }
    eprintln!(
        "NATIVE_EVENT_MATRIX {} errors and {} unwinds",
        event_count, event_count
    );
}

#[test]
#[cfg(unix)]
fn kernel_short_write_and_efbig_leave_original_and_resource_owners_intact() {
    let name = "kernel_short_write_and_efbig_leave_original_and_resource_owners_intact";
    if std::env::var("TAGREX_KERNEL_CANDIDATE").as_deref() == Ok("child") {
        let path = PathBuf::from(std::env::var_os("TAGREX_KERNEL_CANDIDATE_PATH").unwrap());
        let prepared = intent(&path, &target("TITLE"), state(&["new"]));
        let resource = budget();
        let mut events = Events::default();
        let result = patch::apply_prepared_with_budget(
            &mut rw(&path),
            &prepared,
            resource.clone(),
            &mut events,
        );
        assert!(
            matches!(result, Err(patch::Error::Io(_))),
            "expected actual kernel write failure"
        );
        let written: Vec<_> = events
            .0
            .iter()
            .filter_map(|e| {
                if let Event::Written(n) = e {
                    Some(*n)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !written.is_empty(),
            "the first write must return a real successful short prefix"
        );
        assert!(
            written[0] > 0 && written[0] < 2048,
            "OS allowance was not the expected short write: {written:?}"
        );
        assert_eq!(resource.usage().0, Charge::default());
        eprintln!("NATIVE_KERNEL_SHORT_WRITE written={written:?}");
        return;
    }
    let f = Fixture::new();
    let original = flac(
        &[("TITLE".into(), "long saved metadata".repeat(400))],
        &vec![0x51; 160_000],
    );
    let saved = f.put("original.flac", &original);
    let path = f.put("candidate.flac", &original);
    let output = Command::new("sh")
        .args([
            "-c",
            "trap '' XFSZ; ulimit -f 1; exec \"$@\"",
            "owned-kernel-probe",
        ])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_KERNEL_CANDIDATE", "child")
        .env("TAGREX_KERNEL_CANDIDATE_PATH", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("NATIVE_KERNEL_SHORT_WRITE"));
    assert_eq!(fs::read(&saved).unwrap(), original);
    assert_ne!(
        fs::read(&path).unwrap(),
        original,
        "failed disposable candidate must actually contain the short prefix"
    );
}
#[test]
fn external_candidate_edit_with_restored_mtime_is_not_adopted_after_callback() {
    let name = "external_candidate_edit_with_restored_mtime_is_not_adopted_after_callback";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let original = flac(
        &[
            ("TITLE".into(), "old".into()),
            ("CUSTOM".into(), "unchanged".into()),
        ],
        &[11; 10_000],
    );
    let path = f.put("candidate.flac", &original);
    let prepared = intent(
        &path,
        &target("TITLE"),
        state(&["new ".repeat(100).as_str()]),
    );
    let mut foreign = original.clone();
    *foreign.last_mut().unwrap() = 99;
    struct Modify {
        path: PathBuf,
        bytes: Vec<u8>,
        at: std::time::SystemTime,
        done: bool,
    }
    impl Control for Modify {
        fn event(&mut self, event: Event) -> std::io::Result<()> {
            if event == Event::BeforeMutation && !self.done {
                self.done = true;
                fs::write(&self.path, &self.bytes)?;
                rw(&self.path).set_times(std::fs::FileTimes::new().set_modified(self.at))?;
            }
            Ok(())
        }
    }
    let at = fs::metadata(&path).unwrap().modified().unwrap();
    let mut control = Modify {
        path: path.clone(),
        bytes: foreign.clone(),
        at,
        done: false,
    };
    let resource = budget();
    assert!(matches!(
        patch::apply_prepared_with_budget(
            &mut rw(&path),
            &prepared,
            resource.clone(),
            &mut control
        ),
        Err(patch::Error::Changed)
    ));
    assert!(control.done);
    assert_eq!(fs::read(&path).unwrap(), foreign);
    assert_eq!(resource.usage().0, Charge::default());
}

fn checked(command: &mut Command) -> std::process::Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn packet_hashes(path: &Path) -> Vec<String> {
    let output = checked(
        Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_packets",
                "-show_data_hash",
                "sha256",
                "-show_entries",
                "packet=data_hash",
                "-select_streams",
                "a:0",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(path),
    );
    let hashes: Vec<_> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("SHA256:"))
        .map(str::to_owned)
        .collect();
    assert!(!hashes.is_empty());
    hashes
}

#[test]
#[ignore = "manual independent FFmpeg/FFprobe/Mutagen readback"]
fn independent_readers_check_qualified_id3_and_namespaced_mp4_lists_and_packets() {
    let f = Fixture::new();
    let python = std::env::var("TAGREX_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let script = r#"
import sys,json
from mutagen.id3 import ID3,TXXX,TALB,UFID
from mutagen.mp4 import MP4,MP4FreeForm,AtomDataType
kind,phase,path=sys.argv[1:]
if kind=='mp3':
    tags=ID3(path)
    if phase=='seed':
        tags.add(TXXX(encoding=3,desc='review-日本',text=['A; B','','old']))
        tags.add(TALB(encoding=1,text=['Untouched UTF16 album']))
        tags.add(UFID(owner='review.native',data=b'private\x00\xff\x11'))
        tags.save(path,v2_version=4,padding=lambda _:0)
    else:
        assert tags['TALB'].text==['Untouched UTF16 album']
        assert tags['UFID:review.native'].data==b'private\x00\xff\x11'
        # Mutagen trims a terminal empty ID3 member. Require the actual raw
        # complete frame payload to retain it, rather than treating its display
        # projection as evidence that the writer dropped or kept native bytes.
        assert tags['TXXX:review-日本'].text==['','日本語','repeat','repeat']
        raw=open(path,'rb').read()
        size=0
        for b in raw[6:10]: size=(size<<7)|b
        at=10
        expected=b'\x03'+'review-日本'.encode()+b'\0'+'\0'.join(['','日本語','repeat','repeat','']).encode()
        matching=[]
        while at<10+size and raw[at]:
            n=0
            for b in raw[at+4:at+8]: n=(n<<7)|b
            body=raw[at+10:at+10+n]
            if raw[at:at+4]==b'TXXX' and body.startswith(b'\x03'+'review-日本'.encode()+b'\0'): matching.append(body)
            at+=10+n
        assert matching==[expected]
        print(json.dumps(['','日本語','repeat','repeat',''],ensure_ascii=False))
else:
    tags=MP4(path)
    if phase=='seed':
        tags['----:review.native:List']=[MP4FreeForm(v,dataformat=AtomDataType.UTF8) for v in [b'A; B',b'',b'old']]
        tags['----:other.namespace:List']=[MP4FreeForm(b'unchanged\x00opaque',dataformat=AtomDataType.UTF8)]
        tags.save()
    else:
        assert [bytes(v) for v in tags['----:other.namespace:List']]==[b'unchanged\x00opaque']
        print(json.dumps([bytes(v).decode('utf8') for v in tags['----:review.native:List']],ensure_ascii=False))
"#;
    for kind in ["mp3", "m4a"] {
        let path = f.0.join(format!("real.{kind}"));
        checked(
            Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=441:duration=0.15",
                    "-c:a",
                    if kind == "mp3" { "libmp3lame" } else { "aac" },
                    "-metadata",
                    "title=Saved",
                ])
                .arg(&path),
        );
        checked(
            Command::new(&python)
                .args(["-c", script, kind, "seed"])
                .arg(&path),
        );
        let original = fs::read(&path).unwrap();
        let packets = packet_hashes(&path);
        let target = if kind == "mp3" {
            NativeTextTarget::Id3 {
                frame: "TXXX".into(),
                description: Some("review-日本".into()),
                language: None,
                encoding: Some(Id3TextEncoding::Utf8),
            }
        } else {
            NativeTextTarget::Mp4 {
                key: Mp4TextKey::Freeform {
                    mean: "review.native".into(),
                    name: "List".into(),
                },
                kind: Mp4TextKind::Utf8,
                locale: 0,
            }
        };
        let prepared = intent(
            &path,
            &target,
            state(&["", "日本語", "repeat", "repeat", ""]),
        );
        let resource = budget();
        let receipt =
            patch::apply_prepared_with_budget(&mut rw(&path), &prepared, resource.clone(), &mut ())
                .unwrap();
        let output = checked(
            Command::new(&python)
                .args(["-c", script, kind, "read"])
                .arg(&path),
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            r#"["", "日本語", "repeat", "repeat", ""]"#
        );
        assert_eq!(packet_hashes(&path), packets);
        let reversed =
            patch::restore_with_budget(&mut rw(&path), &receipt, resource.clone(), &mut ())
                .unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(packet_hashes(&path), packets);
        drop((receipt, reversed));
        assert_eq!(resource.usage().0, Charge::default());
    }
}

#[test]
fn id3_list_member_bound_is_inclusive_and_stale_overflow_refuses_without_writes() {
    let name = "id3_list_member_bound_is_inclusive_and_stale_overflow_refuses_without_writes";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let original = mp3(&[frame(b"TIT2", b"\x03saved")]);
    let path = f.put("boundary.mp3", &original);
    let wanted = NativeTextTarget::Id3 {
        frame: "TIT2".into(),
        description: None,
        language: None,
        encoding: Some(Id3TextEncoding::Utf8),
    };
    let resource = budget();
    for count in [1023, 1024] {
        fs::write(&path, &original).unwrap();
        let values = ExactTextState::try_from(vec![String::new(); count]).unwrap();
        let prepared = intent(&path, &wanted, values.clone());
        let receipt =
            patch::apply_prepared_with_budget(&mut rw(&path), &prepared, resource.clone(), &mut ())
                .unwrap();
        let after = TagEngine::read_exact(&path).unwrap();
        assert_eq!(after.text().state(&wanted).unwrap(), values);
        let reversed =
            patch::restore_with_budget(&mut rw(&path), &receipt, resource.clone(), &mut ())
                .unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        drop((receipt, reversed, after));
        assert_eq!(resource.usage().0, Charge::default());
    }
    fs::write(&path, &original).unwrap();
    let prepared = intent(&path, &wanted, state(&["new"]));
    let mut dense = vec![0; 1025];
    dense[0] = 3;
    let invalid = mp3(&[frame(b"TIT2", &dense)]);
    fs::write(&path, &invalid).unwrap();
    assert!(matches!(
        patch::apply_prepared_with_budget(&mut rw(&path), &prepared, resource.clone(), &mut ()),
        Err(patch::Error::Limit)
    ));
    assert_eq!(fs::read(&path).unwrap(), invalid);
    assert_eq!(resource.usage().0, Charge::default());
}

#[test]
fn maximum_duplicate_frame_count_and_opaque_metadata_fit_admitted_work() {
    let name = "maximum_duplicate_frame_count_and_opaque_metadata_fit_admitted_work";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let path = f.put("many.mp3", &mp3(&[frame(b"TIT2", b"\x03saved")]));
    let wanted = NativeTextTarget::Id3 {
        frame: "TIT2".into(),
        description: None,
        language: None,
        encoding: Some(Id3TextEncoding::Utf8),
    };
    let prepared = intent(&path, &wanted, state(&["new"]));
    let mut empty = vec![0; 1024];
    empty[0] = 3;
    let mut frames: Vec<_> = (0..4095).map(|_| frame(b"TIT2", &empty)).collect();
    let occupied = frames.iter().map(Vec::len).sum::<usize>();
    let opaque = vec![0x52; (16 << 20) - 10 - 16 - occupied - 10];
    frames.push(frame(b"PRIV", &opaque));
    let attack = mp3(&frames);
    drop((frames, opaque, empty));
    fs::write(&path, &attack).unwrap();
    let resource = budget();
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let result =
        patch::apply_prepared_with_budget(&mut rw(&path), &prepared, resource.clone(), &mut ());
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    assert!(result.is_err());
    assert_eq!(fs::read(&path).unwrap(), attack);
    assert_eq!(resource.usage().0, Charge::default());
    eprintln!(
        "NATIVE_MAX_DUPLICATES peak_delta={peak} work_reservation={}",
        128usize << 20
    );
    assert!(
        peak <= 128usize << 20,
        "maximum duplicate/opaque record combination allocated {peak} bytes outside reservation"
    );
}
