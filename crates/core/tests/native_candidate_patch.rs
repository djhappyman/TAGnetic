//! Independent raw fixtures; only disposable candidate files are mutated.
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
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
    resources::{Budget, Limits},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-native-candidate-{}-{}",
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
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn budget() -> Budget {
    Budget::new(Limits {
        retained: 32 << 20,
        transient: 128 << 20,
        total: 160 << 20,
        jobs: 2,
    })
}
fn file(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap()
}
fn state(values: &[&str]) -> ExactTextState {
    ExactTextState::try_from(values.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
}
fn vorbis(name: &str) -> NativeTextTarget {
    NativeTextTarget::Vorbis { name: name.into() }
}
fn id3_target(frame: &str, encoding: Id3TextEncoding) -> NativeTextTarget {
    NativeTextTarget::Id3 {
        frame: frame.into(),
        description: None,
        language: None,
        encoding: Some(encoding),
    }
}
fn intent(path: &Path, target: &NativeTextTarget, new: ExactTextState) -> PreparedTextPatch {
    static READ: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _capacity = READ.lock().unwrap_or_else(|p| p.into_inner());
    TagEngine::read_exact(path)
        .unwrap()
        .text()
        .prepare_patch(target, new)
        .unwrap()
}
fn flac(items: &[(&str, &str)], frames: usize) -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut comments = 4u32.to_le_bytes().to_vec();
    comments.extend(b"test");
    comments.extend((items.len() as u32).to_le_bytes());
    for (k, v) in items {
        let item = format!("{k}={v}");
        comments.extend((item.len() as u32).to_le_bytes());
        comments.extend(item.as_bytes());
    }
    let n = comments.len();
    out.extend([4, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(comments);
    out.extend([0x82, 0, 0, 8]);
    out.extend(b"TEST\0\xff\x01\x02");
    for _ in 0..frames {
        out.extend([
            0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
        ]);
    }
    out
}
fn flac_inspect(bytes: &[u8]) -> (Vec<(String, String)>, Vec<u8>, Vec<u8>) {
    assert_eq!(&bytes[..4], b"fLaC");
    let mut at = 4;
    let mut records = Vec::new();
    let mut opaque = Vec::new();
    loop {
        let flag = bytes[at];
        let n = ((bytes[at + 1] as usize) << 16)
            | ((bytes[at + 2] as usize) << 8)
            | bytes[at + 3] as usize;
        let body = &bytes[at + 4..at + 4 + n];
        if flag & 127 == 4 {
            let num = |i: usize| u32::from_le_bytes(body[i..i + 4].try_into().unwrap()) as usize;
            let mut pos = 4 + num(0);
            let count = num(pos);
            pos += 4;
            for _ in 0..count {
                let n = num(pos);
                pos += 4;
                let item = std::str::from_utf8(&body[pos..pos + n]).unwrap();
                let (k, v) = item.split_once('=').unwrap();
                records.push((k.into(), v.into()));
                pos += n;
            }
        } else {
            opaque.extend(&bytes[at..at + 4 + n]);
        }
        at += 4 + n;
        if flag & 128 != 0 {
            break;
        }
    }
    (records, opaque, bytes[at..].to_vec())
}
fn syncsafe(n: usize) -> [u8; 4] {
    [
        ((n >> 21) & 127) as u8,
        ((n >> 14) & 127) as u8,
        ((n >> 7) & 127) as u8,
        (n & 127) as u8,
    ]
}
fn frame(id: &[u8; 4], body: &[u8], version: u8) -> Vec<u8> {
    let mut raw = id.to_vec();
    raw.extend(if version == 4 {
        syncsafe(body.len())
    } else {
        (body.len() as u32).to_be_bytes()
    });
    raw.extend([0, 0]);
    raw.extend(body);
    raw
}
fn mp3(frames: &[Vec<u8>], version: u8, audio_frames: usize) -> Vec<u8> {
    let mut body: Vec<u8> = frames.iter().flatten().copied().collect();
    body.extend([0; 16]);
    let mut bytes = vec![b'I', b'D', b'3', version, 0, 0];
    bytes.extend(syncsafe(body.len()));
    bytes.extend(body);
    let mut audio = vec![0xff, 0xfb, 0x90, 0];
    audio.resize(417, 0);
    for _ in 0..audio_frames {
        bytes.extend(&audio);
    }
    bytes
}
fn id3_inspect(bytes: &[u8]) -> (Vec<Vec<u8>>, Vec<u8>) {
    let n = bytes[6..10]
        .iter()
        .fold(0usize, |n, b| (n << 7) | *b as usize);
    let mut at = 10;
    let mut records = Vec::new();
    while at < 10 + n && bytes[at] != 0 {
        let len = if bytes[3] == 4 {
            bytes[at + 4..at + 8]
                .iter()
                .fold(0usize, |n, b| (n << 7) | *b as usize)
        } else {
            u32::from_be_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize
        };
        records.push(bytes[at..at + 10 + len].to_vec());
        at += 10 + len;
    }
    (records, bytes[10 + n..].to_vec())
}

#[test]
fn flac_title_only_candidate_edit_preserves_every_untouched_native_record_and_exact_receipt_restore(
) {
    let f = Fixture::new();
    let before = flac(
        &[
            ("TITLE", "Saved"),
            ("ARTIST", "A; B"),
            ("GENRE", "House"),
            ("GENRE", "house"),
            ("CUSTOM", ""),
            ("CUSTOM", "尾"),
        ],
        10_000,
    );
    let path = f.put("candidate.flac", &before);
    let intent = intent(&path, &vorbis("TITLE"), state(&["Imported 日本語"]));
    assert!(!intent.executable());
    let resource = budget();
    let baseline = resource.usage().0;
    let receipt =
        patch::apply_prepared_with_budget(&mut file(&path), &intent, resource.clone(), &mut ())
            .unwrap();
    let after = fs::read(&path).unwrap();
    let (mut expected, opaque, audio) = flac_inspect(&before);
    let (mut actual, opaque_after, audio_after) = flac_inspect(&after);
    expected.retain(|(k, _)| k != "TITLE");
    assert_eq!(actual.remove(0), ("TITLE".into(), "Imported 日本語".into()));
    assert_eq!(actual, expected);
    assert_eq!(opaque_after, opaque);
    assert_eq!(audio_after, audio);
    assert_ne!(
        receipt.before_target_digest(),
        receipt.after_target_digest()
    );
    assert!(resource.usage().0.retained > baseline.retained);
    let redo =
        patch::restore_with_budget(&mut file(&path), &receipt, resource.clone(), &mut ()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    drop(redo);
    drop(receipt);
    assert_eq!(resource.usage().0, baseline);
}

#[test]
fn receipt_restoration_preserves_unrelated_new_native_values_and_order() {
    let f = Fixture::new();
    let before = flac(
        &[
            ("TITLE", "Saved"),
            ("ARTIST", "A; B"),
            ("GENRE", "House"),
            ("GENRE", "house"),
            ("CUSTOM", ""),
            ("CUSTOM", "尾"),
        ],
        1,
    );
    let path = f.put("candidate.flac", &before);
    let title = intent(&path, &vorbis("TITLE"), state(&["Imported"]));
    let resource = budget();
    let title_receipt =
        patch::apply_prepared_with_budget(&mut file(&path), &title, resource.clone(), &mut ())
            .unwrap();
    let artist = intent(
        &path,
        &vorbis("ARTIST"),
        state(&[" New; literal ", "", "別"]),
    );
    let artist_receipt =
        patch::apply_prepared_with_budget(&mut file(&path), &artist, resource.clone(), &mut ())
            .unwrap();
    let redo =
        patch::restore_with_budget(&mut file(&path), &title_receipt, resource.clone(), &mut ())
            .unwrap();
    let (items, _, _) = flac_inspect(&fs::read(&path).unwrap());
    assert_eq!(items[0], ("TITLE".into(), "Saved".into()));
    assert_eq!(
        &items[1..4],
        &[
            ("ARTIST".into(), " New; literal ".into()),
            ("ARTIST".into(), "".into()),
            ("ARTIST".into(), "別".into())
        ]
    );
    patch::restore_with_budget(&mut file(&path), &artist_receipt, resource.clone(), &mut ())
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    patch::restore_with_budget(&mut file(&path), &redo, resource, &mut ()).unwrap();
    assert_eq!(
        flac_inspect(&fs::read(&path).unwrap()).0[0],
        ("TITLE".into(), "Imported".into())
    );
}

#[test]
fn repeated_empty_and_absent_values_are_distinct_and_raw_interleaving_restores() {
    let f = Fixture::new();
    let before = flac(
        &[
            ("TITLE", "Name"),
            ("ARTIST", "A; B"),
            ("GENRE", "House"),
            ("ARTIST", ""),
            ("GENRE", "house"),
            ("ARTIST", "A; B"),
        ],
        1,
    );
    for new in [
        state(&["", "日本語", "", "日本語"]),
        state(&[""]),
        ExactTextState::absent(),
    ] {
        let path = f.put("candidate.flac", &before);
        let change = intent(&path, &vorbis("ARTIST"), new.clone());
        let receipt =
            patch::apply_prepared_with_budget(&mut file(&path), &change, budget(), &mut ())
                .unwrap();
        let values: Vec<_> = flac_inspect(&fs::read(&path).unwrap())
            .0
            .into_iter()
            .filter(|(k, _)| k == "ARTIST")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(
            values,
            new.values().map_or(Vec::new(), |v| v.as_slice().to_vec())
        );
        patch::restore_with_budget(&mut file(&path), &receipt, budget(), &mut ()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    let path = f.put("candidate.flac", &before);
    let new = intent(&path, &vorbis("NEW"), state(&["", "literal; value"]));
    let receipt =
        patch::apply_prepared_with_budget(&mut file(&path), &new, budget(), &mut ()).unwrap();
    patch::restore_with_budget(&mut file(&path), &receipt, budget(), &mut ()).unwrap();
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn id3_versions_keep_untouched_empty_lists_encodings_private_frames_and_audio() {
    let f = Fixture::new();
    for version in [3, 4] {
        let enc = if version == 3 {
            Id3TextEncoding::Latin1
        } else {
            Id3TextEncoding::Utf8
        };
        let mut title = vec![if version == 3 { 0 } else { 3 }];
        title.extend(b"Saved");
        let before = mp3(
            &[
                frame(b"TIT2", &title, version),
                frame(
                    b"TPE1",
                    if version == 3 {
                        b"\0A; B"
                    } else {
                        b"\x03A; B\0House\0house\0\0"
                    },
                    version,
                ),
                frame(b"TPE2", b"\x01\xff\xfeA\0\0\0", version),
                frame(b"PRIV", b"owner\0\xff\x01\x02", version),
            ],
            version,
            1000,
        );
        let path = f.put("candidate.mp3", &before);
        let target = id3_target("TIT2", enc);
        let change = intent(&path, &target, state(&["Café"]));
        let receipt =
            patch::apply_prepared_with_budget(&mut file(&path), &change, budget(), &mut ())
                .unwrap();
        let after = fs::read(&path).unwrap();
        let (old, audio) = id3_inspect(&before);
        let (new, audio_after) = id3_inspect(&after);
        assert_eq!(&new[1..], &old[1..]);
        assert_eq!(audio_after, audio);
        assert_eq!(after[3], version);
        patch::restore_with_budget(&mut file(&path), &receipt, budget(), &mut ()).unwrap();
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn id3_structural_lists_and_qualified_comment_edit_do_not_touch_other_qualifiers() {
    let f = Fixture::new();
    let before = mp3(
        &[
            frame(b"TIT2", b"\x03Saved", 4),
            frame(b"TPE1", b"\x03A; B\0C\0C\0\0", 4),
            frame(b"COMM", b"\x03engnotes\0One", 4),
            frame(b"COMM", b"\x03deunotes\0Andere", 4),
            frame(b"TXXX", b"\x03CUSTOM\0\0tail\0", 4),
        ],
        4,
        5,
    );
    let path = f.put("candidate.mp3", &before);
    let artist = intent(
        &path,
        &id3_target("TPE1", Id3TextEncoding::Utf8),
        state(&["", "尾", "尾", ""]),
    );
    let receipt =
        patch::apply_prepared_with_budget(&mut file(&path), &artist, budget(), &mut ()).unwrap();
    let (new, _) = id3_inspect(&fs::read(&path).unwrap());
    assert_eq!(&new[1][11..], "\0尾\0尾\0".as_bytes());
    patch::restore_with_budget(&mut file(&path), &receipt, budget(), &mut ()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    let target = NativeTextTarget::Id3 {
        frame: "COMM".into(),
        description: Some("notes".into()),
        language: Some(*b"eng"),
        encoding: Some(Id3TextEncoding::Utf8),
    };
    let comment = intent(&path, &target, state(&[" Changed\n日本語 "]));
    let receipt =
        patch::apply_prepared_with_budget(&mut file(&path), &comment, budget(), &mut ()).unwrap();
    let (old, _) = id3_inspect(&before);
    let (new, _) = id3_inspect(&fs::read(&path).unwrap());
    assert_eq!(new[0], old[0]);
    assert_eq!(new[1], old[1]);
    assert_eq!(&new[3..], &old[3..]);
    patch::restore_with_budget(&mut file(&path), &receipt, budget(), &mut ()).unwrap();
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn stale_or_unsupported_preflight_leaves_the_candidate_unchanged() {
    let f = Fixture::new();
    let original = mp3(&[frame(b"TIT2", b"\x03Saved", 4)], 4, 5);
    let path = f.put("candidate.mp3", &original);
    let change = intent(
        &path,
        &id3_target("TIT2", Id3TextEncoding::Utf8),
        state(&["Changed"]),
    );
    for (offset, value) in [(5, 0x80), (18, 0x10), (6, 0xff), (21, b'X')] {
        let mut bytes = original.clone();
        bytes[offset] = value;
        fs::write(&path, &bytes).unwrap();
        assert!(
            patch::apply_prepared_with_budget(&mut file(&path), &change, budget(), &mut ())
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let original = flac(&[("TITLE", "Saved")], 1);
    fs::write(&path, &original).unwrap();
    assert!(
        patch::apply_prepared_with_budget(&mut file(&path), &change, budget(), &mut ()).is_err()
    );
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn resource_pressure_and_prewrite_cancellation_do_not_mutate_or_leak_receipts() {
    let f = Fixture::new();
    let bytes = flac(&[("TITLE", "Saved")], 10000);
    let path = f.put("candidate.flac", &bytes);
    let change = intent(&path, &vorbis("TITLE"), state(&["Changed"]));
    let resource = budget();
    let lease = resource
        .try_acquire(tagrex_core::resources::Charge {
            transient: 128 << 20,
            jobs: 1,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(
        patch::apply_prepared_with_budget(&mut file(&path), &change, resource.clone(), &mut ()),
        Err(patch::Error::Resource(_))
    ));
    drop(lease);
    struct Cancel;
    impl Control for Cancel {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if event == Event::BeforeMutation {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            Ok(())
        }
    }
    assert!(patch::apply_prepared_with_budget(
        &mut file(&path),
        &change,
        resource.clone(),
        &mut Cancel
    )
    .is_err());
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(resource.usage().0, Default::default());
}

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend(kind);
    out.extend(body);
    out
}
fn data(kind: u32, locale: u32, value: &[u8]) -> Vec<u8> {
    let mut body = kind.to_be_bytes().to_vec();
    body.extend(locale.to_be_bytes());
    body.extend(value);
    atom(b"data", &body)
}
fn freeform(mean: &str, name: &str, kind: u32, locale: u32, values: &[&[u8]]) -> Vec<u8> {
    let mut body = Vec::new();
    for (kind, value) in [(b"mean", mean), (b"name", name)] {
        let mut bytes = vec![0; 4];
        bytes.extend(value.as_bytes());
        body.extend(atom(kind, &bytes));
    }
    for value in values {
        body.extend(data(kind, locale, value));
    }
    atom(b"----", &body)
}
fn boxes(bytes: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut at = 0;
    let mut out = Vec::new();
    while at < bytes.len() {
        let n = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert!(n >= 8 && at + n <= bytes.len());
        out.push((&bytes[at + 4..at + 8], &bytes[at..at + n]));
        at += n;
    }
    out
}
fn replace_ilst(bytes: &[u8], ilst: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (kind, raw) in boxes(bytes) {
        let body = match kind {
            b"ilst" => ilst.to_vec(),
            b"moov" | b"udta" => replace_ilst(&raw[8..], ilst),
            b"meta" => {
                let mut v = raw[8..12].to_vec();
                v.extend(replace_ilst(&raw[12..], ilst));
                v
            }
            _ => raw[8..].to_vec(),
        };
        out.extend(atom(kind.try_into().unwrap(), &body));
    }
    out
}
fn mp4(ilst: &[u8]) -> Vec<u8> {
    replace_ilst(include_bytes!("fixtures/minimal.m4a"), ilst)
}
fn find_ilst(bytes: &[u8]) -> Vec<u8> {
    for (kind, raw) in boxes(bytes) {
        match kind {
            b"ilst" => return raw[8..].to_vec(),
            b"moov" | b"udta" => {
                let found = find_ilst(&raw[8..]);
                if !found.is_empty() {
                    return found;
                }
            }
            b"meta" => {
                let found = find_ilst(&raw[12..]);
                if !found.is_empty() {
                    return found;
                }
            }
            _ => {}
        }
    }
    Vec::new()
}
fn mdat(bytes: &[u8]) -> Vec<u8> {
    boxes(bytes)
        .into_iter()
        .filter(|(kind, _)| *kind == b"mdat")
        .flat_map(|(_, raw)| raw[8..].iter().copied())
        .collect()
}
fn mp4_target(mean: &str, name: &str, kind: Mp4TextKind, locale: u32) -> NativeTextTarget {
    NativeTextTarget::Mp4 {
        key: Mp4TextKey::Freeform {
            mean: mean.into(),
            name: name.into(),
        },
        kind,
        locale,
    }
}

#[test]
fn mp4_eof_metadata_preserves_full_namespaces_utf16_locales_empties_and_opaque_items() {
    let f = Fixture::new();
    let utf16 = |s: &str| {
        s.encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>()
    };
    let first = utf16(" A; B ");
    let last = utf16("尾");
    let selected = freeform("org.one", "CUE", 2, 0x409, &[&first, b"", &last, &last]);
    let other = freeform("org.two", "CUE", 1, 0x411, &[b"private;literal", b""]);
    let opaque = freeform("org.private", "BINARY", 0, 0, &[b"\0\xff\x01\x02"]);
    let mut ilst = atom(b"\xa9nam", &data(1, 0, b"Saved"));
    ilst.extend(&selected);
    ilst.extend(&other);
    ilst.extend(&opaque);
    let before = mp4(&ilst);
    let path = f.put("candidate.m4a", &before);
    let target = mp4_target("org.one", "CUE", Mp4TextKind::Utf16, 0x409);
    let change = intent(&path, &target, state(&["日本語 🦀", "", " literal; / "]));
    let receipt =
        patch::apply_prepared_with_budget(&mut file(&path), &change, budget(), &mut ()).unwrap();
    let after = fs::read(&path).unwrap();
    assert_eq!(mdat(&after), mdat(&before));
    let raw = find_ilst(&after);
    let items = boxes(&raw);
    assert_eq!(items[0].1, boxes(&ilst)[0].1);
    assert_eq!(items[2].1, other);
    assert_eq!(items[3].1, opaque);
    let children = boxes(&items[1].1[8..]);
    assert_eq!(children[0].1, boxes(&selected[8..])[0].1);
    assert_eq!(children[1].1, boxes(&selected[8..])[1].1);
    let values: Vec<_> = children
        .iter()
        .filter(|(kind, _)| *kind == b"data")
        .map(|(_, raw)| {
            assert_eq!(&raw[8..16], &[0, 0, 0, 2, 0, 0, 4, 9]);
            let words = raw[16..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|w| u16::from_be_bytes([w[0], w[1]]));
            char::decode_utf16(words)
                .collect::<std::result::Result<String, _>>()
                .unwrap()
        })
        .collect();
    assert_eq!(values, ["日本語 🦀", "", " literal; / "]);
    patch::restore_with_budget(&mut file(&path), &receipt, budget(), &mut ()).unwrap();
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn mp4_absent_freeform_receipt_restores_without_rewriting_unrelated_atoms() {
    let f = Fixture::new();
    let before = mp4(&atom(b"\xa9nam", &data(1, 0, b"Saved")));
    let path = f.put("candidate.m4a", &before);
    let new = mp4_target("org.new", "CUSTOM", Mp4TextKind::Utf8, 0x1234);
    let change = intent(&path, &new, state(&["", "A; B", "A; B"]));
    let receipt =
        patch::apply_prepared_with_budget(&mut file(&path), &change, budget(), &mut ()).unwrap();
    let after = fs::read(&path).unwrap();
    assert_eq!(mdat(&after), mdat(&before));
    let ilst = find_ilst(&after);
    let entries = boxes(&ilst);
    assert_eq!(
        entries[1].1,
        freeform("org.new", "CUSTOM", 1, 0x1234, &[b"", b"A; B", b"A; B"])
    );
    patch::restore_with_budget(&mut file(&path), &receipt, budget(), &mut ()).unwrap();
    assert_eq!(fs::read(path).unwrap(), before);
}

fn add_chunk_offset(bytes: &[u8], delta: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for (kind, raw) in boxes(bytes) {
        let body = match kind {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => add_chunk_offset(&raw[8..], delta),
            b"stco" => {
                let mut body = raw[8..].to_vec();
                let count = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                for i in 0..count {
                    let at = 8 + i * 4;
                    let old = u32::from_be_bytes(body[at..at + 4].try_into().unwrap());
                    body[at..at + 4].copy_from_slice(&(old + delta).to_be_bytes());
                }
                body
            }
            _ => raw[8..].to_vec(),
        };
        out.extend(atom(kind.try_into().unwrap(), &body));
    }
    out
}
fn front_moov(bytes: &[u8]) -> Vec<u8> {
    let root = boxes(bytes);
    let moov = root.iter().find(|(k, _)| *k == b"moov").unwrap().1;
    let moov = add_chunk_offset(moov, moov.len() as u32);
    let mut out = Vec::new();
    for (kind, raw) in &root {
        if *kind == b"ftyp" {
            out.extend(*raw);
        }
    }
    out.extend(moov);
    for (kind, raw) in root {
        if kind != b"moov" && kind != b"ftyp" {
            out.extend(raw);
        }
    }
    out
}

#[test]
fn mp4_front_moov_supports_fixed_length_and_refuses_relocation_before_changes() {
    let f = Fixture::new();
    let before = front_moov(&mp4(&atom(b"\xa9nam", &data(1, 0, b"Saved"))));
    let path = f.put("candidate.m4a", &before);
    let target = NativeTextTarget::Mp4 {
        key: Mp4TextKey::Fourcc(*b"\xa9nam"),
        kind: Mp4TextKind::Utf8,
        locale: 0,
    };
    let growing = intent(&path, &target, state(&["Longer title"]));
    assert!(matches!(
        patch::apply_prepared_with_budget(&mut file(&path), &growing, budget(), &mut ()),
        Err(patch::Error::Unsupported(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    let same = intent(&path, &target, state(&["Other"]));
    let receipt =
        patch::apply_prepared_with_budget(&mut file(&path), &same, budget(), &mut ()).unwrap();
    assert_eq!(mdat(&fs::read(&path).unwrap()), mdat(&before));
    patch::restore_with_budget(&mut file(&path), &receipt, budget(), &mut ()).unwrap();
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn mp4_mixed_target_qualifiers_and_damaged_tables_refuse_before_mutation() {
    let f = Fixture::new();
    let mut body = data(1, 0, b"Saved");
    body.extend(data(1, 0x409, b"Other"));
    let before = mp4(&atom(b"\xa9nam", &body));
    let path = f.put("candidate.m4a", &before);
    let target = NativeTextTarget::Mp4 {
        key: Mp4TextKey::Fourcc(*b"\xa9nam"),
        kind: Mp4TextKind::Utf8,
        locale: 0,
    };
    let change = intent(&path, &target, state(&["Changed"]));
    assert!(matches!(
        patch::apply_prepared_with_budget(&mut file(&path), &change, budget(), &mut ()),
        Err(patch::Error::Unsupported(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    let valid = mp4(&atom(b"\xa9nam", &data(1, 0, b"Saved")));
    fs::write(&path, &valid).unwrap();
    let change = intent(&path, &target, state(&["Changed"]));
    let mut corrupt = valid;
    let at = corrupt.windows(4).position(|b| b == b"stco").unwrap();
    corrupt[at + 8..at + 12].copy_from_slice(&u32::MAX.to_be_bytes());
    fs::write(&path, &corrupt).unwrap();
    assert!(
        patch::apply_prepared_with_budget(&mut file(&path), &change, budget(), &mut ()).is_err()
    );
    assert_eq!(fs::read(path).unwrap(), corrupt);
}

#[test]
fn cancellation_at_each_observed_checkpoint_and_short_write_faults_never_return_a_receipt() {
    struct Trace(usize);
    impl Control for Trace {
        fn event(&mut self, _: Event) -> io::Result<()> {
            self.0 += 1;
            Ok(())
        }
    }
    struct Cancel {
        at: usize,
        count: usize,
    }
    impl Control for Cancel {
        fn event(&mut self, _: Event) -> io::Result<()> {
            self.count += 1;
            Ok(())
        }
        fn cancelled(&mut self) -> bool {
            self.count == self.at
        }
    }
    let f = Fixture::new();
    let bytes = flac(
        &[("TITLE", "Saved"), ("ARTIST", "A; B"), ("CUSTOM", "")],
        15_000,
    );
    let original = f.put("source.flac", &bytes);
    let candidate = f.put("candidate.flac", &bytes);
    let intent = intent(&original, &vorbis("TITLE"), state(&["Longer 日本語 title"]));
    let resource = budget();
    let mut trace = Trace(0);
    drop(
        patch::apply_prepared_with_budget(
            &mut file(&candidate),
            &intent,
            resource.clone(),
            &mut trace,
        )
        .unwrap(),
    );
    assert!(trace.0 > 20);
    for at in 1..=trace.0 {
        fs::write(&candidate, &bytes).unwrap();
        assert!(matches!(
            patch::apply_prepared_with_budget(
                &mut file(&candidate),
                &intent,
                resource.clone(),
                &mut Cancel { at, count: 0 }
            ),
            Err(patch::Error::Cancelled)
        ));
        assert_eq!(resource.usage().0, Default::default());
        assert_eq!(fs::read(&original).unwrap(), bytes);
    }
    struct Short {
        max: usize,
        fail: Option<usize>,
        calls: usize,
    }
    impl Control for Short {
        fn write_limit(&mut self, n: usize) -> io::Result<usize> {
            self.calls += 1;
            if self.fail == Some(self.calls) {
                return Err(io::Error::from_raw_os_error(28));
            }
            Ok(n.min(self.max))
        }
    }
    for (max, fail) in [(7, None), (0, None), (7, Some(20))] {
        fs::write(&candidate, &bytes).unwrap();
        let result = patch::apply_prepared_with_budget(
            &mut file(&candidate),
            &intent,
            resource.clone(),
            &mut Short {
                max,
                fail,
                calls: 0,
            },
        );
        if max > 0 && fail.is_none() {
            let receipt = result.unwrap();
            patch::restore_with_budget(&mut file(&candidate), &receipt, resource.clone(), &mut ())
                .unwrap();
            assert_eq!(fs::read(&candidate).unwrap(), bytes);
        } else {
            assert!(result.is_err());
        }
        assert_eq!(fs::read(&original).unwrap(), bytes);
        assert_eq!(resource.usage().0, Default::default());
    }
}

#[test]
fn deterministic_malformed_metadata_never_panics_or_leaks_and_preserves_unselected_payload_on_success(
) {
    let f = Fixture::new();
    let bytes = flac(&[("TITLE", "Saved"), ("ARTIST", "A; B"), ("CUSTOM", "")], 1);
    let path = f.put("candidate.flac", &bytes);
    let intent = intent(&path, &vorbis("TITLE"), state(&["Changed"]));
    let resource = budget();
    let mut random = 0x1deafacade_u64;
    for iteration in 0..1000 {
        let mut input = bytes.clone();
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let index = random as usize % input.len();
        match iteration % 4 {
            0 => input.truncate(index),
            1 => input[index] ^= (random >> 24) as u8,
            2 => {
                let end = (index + 4).min(input.len());
                input[index..end].fill(0xff);
            }
            _ => input.insert(index, 0),
        }
        fs::write(&path, &input).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            patch::apply_prepared_with_budget(&mut file(&path), &intent, resource.clone(), &mut ())
        }));
        assert!(result.is_ok(), "mutation {iteration} panicked");
        match result.unwrap() {
            Ok(receipt) => {
                let before = flac_inspect(&input);
                let after = flac_inspect(&fs::read(&path).unwrap());
                assert_eq!(&before.0[1..], &after.0[1..]);
                assert_eq!(before.1, after.1);
                assert_eq!(before.2, after.2);
                drop(receipt);
            }
            Err(_) => assert_eq!(fs::read(&path).unwrap(), input),
        }
        assert_eq!(resource.usage().0, Default::default());
    }
}

#[test]
fn changed_candidate_in_a_callback_is_refused_before_any_more_candidate_writes() {
    let f = Fixture::new();
    let bytes = flac(&[("TITLE", "Saved")], 10_000);
    let path = f.put("candidate.flac", &bytes);
    let intent = intent(&path, &vorbis("TITLE"), state(&["Changed"]));
    struct Change {
        path: PathBuf,
        at: Event,
    }
    impl Control for Change {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if event == self.at {
                fs::write(&self.path, b"foreign candidate")?;
            }
            Ok(())
        }
    }
    for at in [Event::BeforeMutation, Event::Verified] {
        fs::write(&path, &bytes).unwrap();
        assert!(patch::apply_prepared_with_budget(
            &mut file(&path),
            &intent,
            budget(),
            &mut Change {
                path: path.clone(),
                at
            }
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), b"foreign candidate");
    }
}

#[test]
fn native_policy_change_child() {
    let Ok(root) = std::env::var("TAGREX_NATIVE_POLICY_ROOT") else {
        return;
    };
    let f = Path::new(&root);
    let bytes = flac(&[("TITLE", "Saved")], 1);
    let path = f.join("policy.flac");
    fs::write(&path, &bytes).unwrap();
    let intent = intent(&path, &vorbis("TITLE"), state(&["Changed"]));
    struct Change;
    impl Control for Change {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if event == Event::BeforeMutation {
                tagrex_core::model::set_multi_value_separator(" / ");
            }
            Ok(())
        }
    }
    assert!(matches!(
        patch::apply_prepared_with_budget(&mut file(&path), &intent, budget(), &mut Change),
        Err(patch::Error::Changed)
    ));
    assert_eq!(fs::read(&path).unwrap(), bytes);
}
#[test]
fn policy_changes_are_revalidated_before_editing_without_affecting_other_test_settings() {
    let f = Fixture::new();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "native_policy_change_child", "--nocapture"])
        .env("TAGREX_NATIVE_POLICY_ROOT", &f.0)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
#[ignore = "manual independent Mutagen/ffprobe Apply-like and receipt restore on owned FLAC/MP3/AAC/M4A candidates"]
fn independent_native_readback_and_encoded_packet_hashes() {
    let f = Fixture::new();
    let flac = f.put(
        "candidate.flac",
        &flac(&[("TITLE", "Saved"), ("ARTIST", "A; B"), ("CUSTOM", "")], 1),
    );
    let mp3 = f.put(
        "candidate.mp3",
        &mp3(
            &[
                frame(b"TIT2", b"\x03Saved", 4),
                frame(b"PRIV", b"owner\0\xff\x01", 4),
            ],
            4,
            5,
        ),
    );
    let m4a = f.put(
        "candidate.m4a",
        &mp4(&atom(b"\xa9nam", &data(1, 0, b"Saved"))),
    );
    let aac = f.0.join("encoded.aac");
    let encoded = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=44100:cl=mono",
            "-t",
            "0.05",
            "-c:a",
            "aac",
            "-f",
            "adts",
        ])
        .arg(&aac)
        .output()
        .unwrap();
    assert!(
        encoded.status.success(),
        "{}",
        String::from_utf8_lossy(&encoded.stderr)
    );
    let mut aac_bytes = mp3_tag_only();
    aac_bytes.extend(fs::read(&aac).unwrap());
    fs::write(&aac, &aac_bytes).unwrap();
    let targets = [
        vorbis("TITLE"),
        id3_target("TIT2", Id3TextEncoding::Utf8),
        id3_target("TIT2", Id3TextEncoding::Utf8),
        NativeTextTarget::Mp4 {
            key: Mp4TextKey::Fourcc(*b"\xa9nam"),
            kind: Mp4TextKind::Utf8,
            locale: 0,
        },
    ];
    let paths = [flac, mp3, aac, m4a];
    let packets = |path: &Path| {
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "a",
                "-show_packets",
                "-show_data_hash",
                "sha256",
                "-show_entries",
                "packet=data_hash,size,duration",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.windows(9).any(|part| part == b"data_hash"),
            "no encoded packets"
        );
        output.stdout
    };
    let before: Vec<_> = paths
        .iter()
        .map(|path| (fs::read(path).unwrap(), packets(path)))
        .collect();
    let mut receipts = Vec::new();
    for (path, target) in paths.iter().zip(&targets) {
        let intent = intent(path, target, state(&["日本語 Title"]));
        receipts.push(
            patch::apply_prepared_with_budget(&mut file(path), &intent, budget(), &mut ()).unwrap(),
        );
    }
    let script = r#"import sys,mutagen
from mutagen.flac import FLAC
from mutagen.id3 import ID3
from mutagen.mp4 import MP4
a,b,c,d=sys.argv[1:]
assert FLAC(a)['title']==['日本語 Title']
assert ID3(b)['TIT2'].text==['日本語 Title']
assert ID3(c)['TIT2'].text==['日本語 Title']
assert MP4(d)['\xa9nam']==['日本語 Title']
print('independent Mutagen',mutagen.version_string,'title readback passed')
"#;
    let readback = std::process::Command::new(
        std::env::var_os("TAGREX_TEST_PYTHON").unwrap_or_else(|| "python3".into()),
    )
    .arg("-c")
    .arg(script)
    .args(&paths)
    .output()
    .unwrap();
    assert!(
        readback.status.success(),
        "{}",
        String::from_utf8_lossy(&readback.stderr)
    );
    print!("{}", String::from_utf8_lossy(&readback.stdout));
    for (index, path) in paths.iter().enumerate() {
        assert_eq!(packets(path), before[index].1);
        patch::restore_with_budget(&mut file(path), &receipts[index], budget(), &mut ()).unwrap();
        assert_eq!(fs::read(path).unwrap(), before[index].0);
        assert_eq!(packets(path), before[index].1);
    }
}
fn mp3_tag_only() -> Vec<u8> {
    mp3(&[frame(b"TIT2", b"\x03Saved", 4)], 4, 0)
}

#[test]
fn concurrent_candidate_admission_refuses_before_mutating_and_releases_after_success() {
    struct Hold {
        ready: std::sync::mpsc::Sender<()>,
        resume: std::sync::mpsc::Receiver<()>,
    }
    impl Control for Hold {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if event == Event::BeforeMutation {
                self.ready.send(()).unwrap();
                self.resume.recv().unwrap();
            }
            Ok(())
        }
    }
    let f = Fixture::new();
    let bytes = flac(&[("TITLE", "Saved"), ("ARTIST", "A; B")], 100);
    let first = f.put("first.flac", &bytes);
    let second = f.put("second.flac", &bytes);
    let first_intent = intent(&first, &vorbis("TITLE"), state(&["Changed"]));
    let second_intent = intent(&second, &vorbis("TITLE"), state(&["Changed"]));
    let resource = budget();
    let (ready, waiting) = std::sync::mpsc::channel();
    let (resume, held) = std::sync::mpsc::channel();
    let other = resource.clone();
    let thread = std::thread::spawn(move || {
        patch::apply_prepared_with_budget(
            &mut file(&first),
            &first_intent,
            other,
            &mut Hold {
                ready,
                resume: held,
            },
        )
        .unwrap()
    });
    waiting
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(matches!(
        patch::apply_prepared_with_budget(
            &mut file(&second),
            &second_intent,
            resource.clone(),
            &mut ()
        ),
        Err(patch::Error::Resource(_))
    ));
    assert_eq!(fs::read(&second).unwrap(), bytes);
    resume.send(()).unwrap();
    drop(thread.join().unwrap());
    assert_eq!(resource.usage().0, Default::default());
    drop(
        patch::apply_prepared_with_budget(
            &mut file(&second),
            &second_intent,
            resource.clone(),
            &mut (),
        )
        .unwrap(),
    );
    assert_eq!(resource.usage().0, Default::default());
}

#[test]
#[ignore = "manual 64MiB streaming candidate growth/restore cost and bounded-write probe"]
fn native_candidate_streaming_cost_probe() {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom, Write};
    struct Writes {
        bytes: u64,
        largest: u64,
    }
    impl Control for Writes {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if let Event::Written(n) = event {
                self.bytes += n;
                self.largest = self.largest.max(n);
            }
            Ok(())
        }
    }
    fn digest(file: &mut File) -> [u8; 32] {
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut hash = Sha256::new();
        let mut block = [0; 64 * 1024];
        loop {
            let n = file.read(&mut block).unwrap();
            if n == 0 {
                break;
            }
            hash.update(&block[..n]);
        }
        hash.finalize().into()
    }
    let f = Fixture::new();
    let path = f.put(
        "large.flac",
        &flac(&[("TITLE", "Saved"), ("ARTIST", "A; B")], 1),
    );
    let change = intent(&path, &vorbis("TITLE"), state(&["Longer 日本語 title"]));
    let mut candidate = file(&path);
    candidate.seek(SeekFrom::End(0)).unwrap();
    let block = [0x5a; 64 * 1024];
    for _ in 0..1024 {
        candidate.write_all(&block).unwrap();
    }
    let before = digest(&mut candidate);
    let resource = budget();
    let mut writes = Writes {
        bytes: 0,
        largest: 0,
    };
    let start = std::time::Instant::now();
    let receipt =
        patch::apply_prepared_with_budget(&mut candidate, &change, resource.clone(), &mut writes)
            .unwrap();
    let apply_time = start.elapsed();
    let restore_start = std::time::Instant::now();
    drop(
        patch::restore_with_budget(&mut candidate, &receipt, resource.clone(), &mut writes)
            .unwrap(),
    );
    let restore_time = restore_start.elapsed();
    drop(receipt);
    assert_eq!(digest(&mut candidate), before);
    assert!(writes.largest <= 64 * 1024);
    assert!(writes.bytes > 128 * 1024 * 1024);
    assert_eq!(resource.usage().0, Default::default());
    eprintln!(
        "64MiB suffix: apply={apply_time:?} restore={restore_time:?} written={} max_write={}",
        writes.bytes, writes.largest
    );
}
