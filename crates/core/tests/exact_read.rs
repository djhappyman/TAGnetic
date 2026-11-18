//! Native exact reads against owned FLAC, ID3/WAV/MP3 and real AAC-in-MP4 fixtures.
//! Fixture construction uses raw envelopes so backend writers cannot normalize
//! the values before the reader sees them. No production mutation is invoked.
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    model::{
        exact::{
            ExactReadError, ExactTextState, Id3TextEncoding, Mp4TextKey, Mp4TextKind,
            NativeTextTarget, UnavailableReason,
        },
        TagBlockKind, TagEngine, TagField,
    },
    values::FieldValues,
};

// Successful single-operation cases have exclusive capacity. A separate test
// deliberately saturates the real shared coordinator with concurrent reads.
static READ_CASE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-exact-read-{}-{}",
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
fn values(state: &ExactTextState) -> Vec<&str> {
    state
        .values()
        .unwrap()
        .as_slice()
        .iter()
        .map(String::as_str)
        .collect()
}
fn syncsafe(value: usize) -> [u8; 4] {
    [
        ((value >> 21) & 127) as u8,
        ((value >> 14) & 127) as u8,
        ((value >> 7) & 127) as u8,
        (value & 127) as u8,
    ]
}
fn id3_frame(id: &[u8; 4], content: &[u8], revision: u8) -> Vec<u8> {
    let mut frame = id.to_vec();
    frame.extend(if revision == 4 {
        syncsafe(content.len())
    } else {
        (content.len() as u32).to_be_bytes()
    });
    frame.extend([0, 0]);
    frame.extend(content);
    frame
}
fn id3(frames: &[Vec<u8>], revision: u8) -> Vec<u8> {
    let body: Vec<_> = frames.iter().flatten().copied().collect();
    let mut tag = vec![b'I', b'D', b'3', revision, 0, 0];
    tag.extend(syncsafe(body.len()));
    tag.extend(body);
    tag
}
fn wav(tag: &[u8]) -> Vec<u8> {
    let mut body = b"WAVEfmt ".to_vec();
    body.extend(16u32.to_le_bytes());
    body.extend(1u16.to_le_bytes());
    body.extend(1u16.to_le_bytes());
    body.extend(8000u32.to_le_bytes());
    body.extend(16000u32.to_le_bytes());
    body.extend(2u16.to_le_bytes());
    body.extend(16u16.to_le_bytes());
    body.extend(b"data");
    body.extend(64u32.to_le_bytes());
    body.extend([0; 64]);
    body.extend(b"id3 ");
    body.extend((tag.len() as u32).to_le_bytes());
    body.extend(tag);
    if !tag.len().is_multiple_of(2) {
        body.push(0);
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}
fn flac(items: &[(&str, &str)]) -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = vec![0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut packet = Vec::new();
    packet.extend(4u32.to_le_bytes());
    packet.extend(b"test");
    packet.extend((items.len() as u32).to_le_bytes());
    for (name, value) in items {
        let item = format!("{name}={value}");
        packet.extend((item.len() as u32).to_le_bytes());
        packet.extend(item.as_bytes());
    }
    let length = packet.len() as u32;
    out.extend([4, (length >> 16) as u8, (length >> 8) as u8, length as u8]);
    out.extend(packet);
    // Unknown/private application metadata is deliberately outside text state.
    out.extend([0x82, 0, 0, 8]);
    out.extend(b"TEST\0\xff\x01\x02");
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out
}
fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend(kind);
    out.extend(payload);
    out
}
fn data(kind: u32, locale: u32, bytes: &[u8]) -> Vec<u8> {
    let mut out = kind.to_be_bytes().to_vec();
    out.extend(locale.to_be_bytes());
    out.extend(bytes);
    atom(b"data", &out)
}
fn freeform(mean: &str, name: &str, values: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut m = vec![0; 4];
    m.extend(mean.as_bytes());
    let mut n = vec![0; 4];
    n.extend(name.as_bytes());
    let mut out = atom(b"mean", &m);
    out.extend(atom(b"name", &n));
    for (kind, locale, value) in values {
        out.extend(data(*kind, *locale, value));
    }
    atom(b"----", &out)
}
fn replace_ilst(bytes: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let kind: &[u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
        let body = &bytes[at + 8..at + size];
        let changed = match kind {
            b"ilst" => replacement.to_vec(),
            b"moov" | b"udta" => replace_ilst(body, replacement),
            b"meta" => {
                let mut b = body[..4].to_vec();
                b.extend(replace_ilst(&body[4..], replacement));
                b
            }
            _ => body.to_vec(),
        };
        out.extend(atom(kind, &changed));
        at += size;
    }
    out
}
fn mp4(payload: &[u8]) -> Vec<u8> {
    replace_ilst(include_bytes!("fixtures/minimal.m4a"), payload)
}
fn vorbis(name: &str) -> NativeTextTarget {
    NativeTextTarget::Vorbis { name: name.into() }
}
fn id3_target(
    frame: &str,
    description: Option<&str>,
    language: Option<[u8; 3]>,
) -> NativeTextTarget {
    NativeTextTarget::Id3 {
        frame: frame.into(),
        description: description.map(str::to_owned),
        language,
        encoding: Some(Id3TextEncoding::Utf8),
    }
}
fn assert_read_only(path: &Path, before: &[u8]) {
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn flac_native_values_preserve_punctuation_empties_order_duplicates_and_case() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let bytes = flac(&[
        ("ARTIST", "AC/DC"),
        ("ARTIST", "Hello; Goodbye"),
        ("ARTIST", ""),
        ("ARTIST", " Björk "),
        ("ARTIST", "AC/DC"),
        ("GENRE", "House"),
        ("GENRE", "house"),
        ("VENUE", "日本語 🦀"),
    ]);
    let path = f.put("native.flac", &bytes);
    let legacy = TagEngine::read_with_props(&path).unwrap();
    let exact = TagEngine::read_exact(&path).unwrap();
    assert_eq!(exact.legacy().file.tags, legacy.file.tags);
    assert_eq!(exact.legacy().blocks, legacy.blocks);
    assert_eq!(
        values(&exact.text().state(&vorbis("ARTIST")).unwrap()),
        ["AC/DC", "Hello; Goodbye", "", " Björk ", "AC/DC"]
    );
    assert_eq!(
        values(&exact.text().state(&vorbis("artist")).unwrap()),
        ["AC/DC", "Hello; Goodbye", "", " Björk ", "AC/DC"]
    );
    assert_eq!(
        values(&exact.text().state(&vorbis("GENRE")).unwrap()),
        ["House", "house"]
    );
    assert_eq!(
        exact.text().state(&vorbis("MISSING")).unwrap(),
        ExactTextState::absent()
    );
    let empty = ExactTextState::try_from(vec![String::new(), String::new()]).unwrap();
    let patch = exact
        .text()
        .prepare_patch(&vorbis("artist"), empty.clone())
        .unwrap();
    assert_eq!(patch.new_state(), &empty);
    assert_eq!(patch.target(), &vorbis("ARTIST"));
    assert!(!patch.executable());
    assert_read_only(&path, &bytes);
}
#[test]
fn vorbis_native_case_collisions_are_ambiguous_not_silently_joined() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let path = f.put(
        "case.flac",
        &flac(&[("ARTIST", "First"), ("artist", "Second")]),
    );
    let read = TagEngine::read_exact(&path).unwrap();
    assert!(matches!(
        read.text().resolve_field(&TagField::Artist),
        Err(UnavailableReason::AmbiguousIdentity)
    ));
    assert!(matches!(
        read.text().state(&vorbis("ARTIST")),
        Err(ExactReadError::Unavailable(
            UnavailableReason::AmbiguousIdentity
        ))
    ));
}
#[test]
fn raw_id3_reads_trailing_empty_members_and_separates_qualified_comments() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let tag = id3(
        &[
            id3_frame(b"TPE1", b"\x03A; B\0C\0C\0\0", 4),
            id3_frame(b"TIT2", "\x03日本語 🦀".as_bytes(), 4),
            id3_frame(b"TXXX", b"\x03VENUE\0 First \0\0", 4),
            id3_frame(b"COMM", b"\x03eng\0One\nTwo", 4),
            id3_frame(b"COMM", b"\x03deuNotes\0Andere", 4),
            id3_frame(b"PRIV", b"owner\0\xff\x01\x02", 4),
        ],
        4,
    );
    let bytes = wav(&tag);
    let path = f.put("native.wav", &bytes);
    let read = TagEngine::read_exact(&path).unwrap();
    assert_eq!(
        values(&read.text().state(&id3_target("TPE1", None, None)).unwrap()),
        ["A; B", "C", "C", "", ""]
    );
    assert_eq!(
        values(
            &read
                .text()
                .state(&id3_target("TXXX", Some("VENUE"), None))
                .unwrap()
        ),
        [" First ", "", ""]
    );
    assert_eq!(
        values(
            &read
                .text()
                .state(&id3_target("COMM", Some(""), Some(*b"eng")))
                .unwrap()
        ),
        ["One\nTwo"]
    );
    assert!(matches!(
        read.text().resolve_field(&TagField::Comment),
        Err(UnavailableReason::AmbiguousIdentity)
    ));
    assert!(read
        .text()
        .entries()
        .iter()
        .any(|entry| matches!(entry.state, Err(UnavailableReason::NonText))));
    assert_read_only(&path, &bytes);
    let mut mp3 = tag;
    let mut frame = vec![0xff, 0xfb, 0x90, 0];
    frame.resize(417, 0);
    mp3.extend(frame.repeat(5));
    let path = f.put("native.mp3", &mp3);
    assert_eq!(
        values(
            &TagEngine::read_exact(&path)
                .unwrap()
                .text()
                .state(&id3_target("TPE1", None, None))
                .unwrap()
        ),
        ["A; B", "C", "C", "", ""]
    );
    assert_read_only(&path, &mp3);
}
#[test]
fn mp4_native_namespaces_data_kinds_locales_and_empty_values_are_retained() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let utf16: Vec<_> = "日本語 🦀"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    let mut payload = freeform(
        "org.first",
        "NAME",
        &[
            (1, 0, b"A; B".to_vec()),
            (1, 7, b"Localized".to_vec()),
            (1, 0, Vec::new()),
            (1, 0, b"A; B".to_vec()),
        ],
    );
    payload.extend(freeform("org.second", "NAME", &[(2, 7, utf16)]));
    payload.extend(atom(b"covr", &data(13, 0, b"private picture bytes")));
    let bytes = mp4(&payload);
    let path = f.put("native.m4a", &bytes);
    let read = TagEngine::read_exact(&path).unwrap();
    let target = NativeTextTarget::Mp4 {
        key: Mp4TextKey::Freeform {
            mean: "org.first".into(),
            name: "NAME".into(),
        },
        kind: Mp4TextKind::Utf8,
        locale: 0,
    };
    assert_eq!(
        values(&read.text().state(&target).unwrap()),
        ["A; B", "", "A; B"]
    );
    let localized = NativeTextTarget::Mp4 {
        key: Mp4TextKey::Freeform {
            mean: "org.first".into(),
            name: "NAME".into(),
        },
        kind: Mp4TextKind::Utf8,
        locale: 7,
    };
    assert_eq!(
        values(&read.text().state(&localized).unwrap()),
        ["Localized"]
    );
    let prepared = read
        .text()
        .prepare_patch(
            &localized,
            ExactTextState::try_from(vec!["New locale value".into()]).unwrap(),
        )
        .unwrap();
    assert_eq!(prepared.target(), &localized);
    let other = NativeTextTarget::Mp4 {
        key: Mp4TextKey::Freeform {
            mean: "org.second".into(),
            name: "NAME".into(),
        },
        kind: Mp4TextKind::Utf16,
        locale: 7,
    };
    assert_eq!(values(&read.text().state(&other).unwrap()), ["日本語 🦀"]);
    assert!(matches!(
        read.text().resolve_field(&TagField::Custom("NAME".into())),
        Err(UnavailableReason::AmbiguousIdentity)
    ));
    let patch = read
        .text()
        .prepare_patch(
            &other,
            ExactTextState::try_from(vec!["Changed".into()]).unwrap(),
        )
        .unwrap();
    assert_eq!(patch.target(), &other);
    assert_read_only(&path, &bytes);
}
#[test]
fn exact_state_validation_and_typed_patch_capabilities_do_not_infer_lists() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(ExactTextState::present(FieldValues::new(Vec::new()).unwrap()).is_err());
    assert!(ExactTextState::try_from(vec!["bad\0member".into()]).is_err());
    assert_ne!(
        ExactTextState::absent(),
        ExactTextState::try_from(vec![String::new()]).unwrap()
    );
    let f = Fixture::new();
    let bytes = wav(&id3(&[id3_frame(b"TRCK", b"\x031/2", 4)], 4));
    let path = f.put("typed.wav", &bytes);
    let read = TagEngine::read_exact(&path).unwrap();
    assert!(matches!(
        read.text().prepare_patch(
            &id3_target("TRCK", None, None),
            ExactTextState::try_from(vec!["1".into(), "2".into()]).unwrap()
        ),
        Err(ExactReadError::Unavailable(UnavailableReason::TypedField))
    ));
    let missing_private = NativeTextTarget::Id3 {
        frame: "PRIV".into(),
        description: None,
        language: None,
        encoding: None,
    };
    assert!(read
        .text()
        .prepare_patch(
            &missing_private,
            ExactTextState::try_from(vec!["text".into()]).unwrap()
        )
        .is_err());
}
#[test]
fn unsupported_id3_encodings_and_duplicate_frames_never_certify_exact_absence() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let path = f.put(
        "encoding.wav",
        &wav(&id3(&[id3_frame(b"TPE1", b"\x01\xff\xfeA\0\0\0", 4)], 4)),
    );
    let read = TagEngine::read_exact(&path).unwrap();
    assert!(read.text().state(&id3_target("TPE1", None, None)).is_err());
    let path = f.put(
        "duplicate.wav",
        &wav(&id3(
            &[
                id3_frame(b"TPE1", b"\x03One", 4),
                id3_frame(b"TPE1", b"\x03Two", 4),
            ],
            4,
        )),
    );
    let read = TagEngine::read_exact(&path).unwrap();
    assert!(matches!(
        read.text().state(&id3_target("TPE1", None, None)),
        Err(ExactReadError::Unavailable(
            UnavailableReason::AmbiguousIdentity
        ))
    ));
}
#[test]
fn malformed_and_oversized_metadata_fail_before_a_result_is_published() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let path = f.put("sparse.mp3", &[b'I', b'D', b'3', 4, 0, 0, 0x09, 0, 0, 0]); // 18 MiB metadata.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(18 * 1024 * 1024 + 10)
        .unwrap();
    assert!(matches!(
        TagEngine::read_exact(&path),
        Err(ExactReadError::Limit)
    ));
    let mut bad = flac(&[("ARTIST", "A")]);
    bad[43..46].copy_from_slice(&[0xff, 0xff, 0xff]);
    let path = f.put("bad.flac", &bad);
    assert!(TagEngine::read_exact(&path).is_err());
    let mut flags = id3(&[id3_frame(b"TPE1", b"\x03One", 4)], 4);
    flags[5] = 0x80;
    let path = f.put("flagged.mp3", &flags);
    assert!(matches!(
        TagEngine::read_exact(&path),
        Err(ExactReadError::Unavailable(
            UnavailableReason::UnsupportedLayout
        ))
    ));
    let payload = freeform("org.test", "VALUE", &[(2, 0, vec![0xd8, 0x00])]);
    let path = f.put("bad.m4a", &mp4(&payload));
    assert!(TagEngine::read_exact(&path).is_err());
}
#[test]
fn repeated_values_and_snapshot_limits_fail_without_partial_native_inventory() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let items = vec![("ARTIST", "A"); 1025];
    let path = f.put("too-many.flac", &flac(&items));
    assert!(matches!(
        TagEngine::read_exact(&path),
        Err(ExactReadError::Limit)
    ));
    let large = "X".repeat(1024 * 1024);
    let owned: Vec<_> = (0..8)
        .map(|i| (format!("FIELD{i}"), large.as_str()))
        .collect();
    let items: Vec<_> = owned
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect();
    let path = f.put("snapshot.flac", &flac(&items));
    assert!(matches!(
        TagEngine::read_exact(&path),
        Err(ExactReadError::Limit)
    ));
}

#[test]
fn selected_block_fallback_and_unsupported_layout_are_reported_honestly() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let mut bytes = wav(&id3(&[id3_frame(b"PRIV", b"owner\0private", 4)], 4));
    let mut info = b"INFOINAM".to_vec();
    let name = b"RIFF title\0";
    info.extend((name.len() as u32).to_le_bytes());
    info.extend(name);
    if !name.len().is_multiple_of(2) {
        info.push(0);
    }
    bytes.extend(b"LIST");
    bytes.extend((info.len() as u32).to_le_bytes());
    bytes.extend(info);
    let length = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    let path = f.put("fallback.wav", &bytes);
    let read = TagEngine::read_exact(&path).unwrap();
    assert_eq!(
        read.legacy()
            .file
            .tags
            .get(&TagField::Title)
            .map(String::as_str),
        Some("RIFF title")
    );
    assert_eq!(
        read.text().context().selected_block,
        Some(TagBlockKind::RiffInfo)
    );
    assert_eq!(
        read.text().unavailable(),
        Some(UnavailableReason::UnsupportedLayout)
    );
    assert!(read.text().state(&id3_target("TIT2", None, None)).is_err());
    assert_read_only(&path, &bytes);
}

#[test]
fn policy_changes_cancellation_and_busy_reads_release_all_ownership() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    const ENV: &str = "TAGREX_EXACT_POLICY_CHILD";
    if std::env::var_os(ENV).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "policy_changes_cancellation_and_busy_reads_release_all_ownership",
                "--nocapture",
            ])
            .env(ENV, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    use tagrex_core::resources::{Budget, Charge};
    let f = Fixture::new();
    let bytes = flac(&[("ARTIST", "A"), ("ARTIST", "B")]);
    let path = f.put("policy.flac", &bytes);
    let budget = Budget::process();
    let baseline = budget.usage().0;
    assert!(matches!(
        TagEngine::read_exact_with_cancel(&path, || true),
        Err(ExactReadError::Cancelled)
    ));
    let pressure = budget
        .try_acquire(Charge {
            jobs: 8,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(
        TagEngine::read_exact(&path),
        Err(ExactReadError::Resource(_))
    ));
    drop(pressure);
    let probes = std::cell::Cell::new(0);
    let result = TagEngine::read_exact_with_cancel(&path, || {
        let n = probes.get() + 1;
        probes.set(n);
        if n == 2 {
            tagrex_core::model::set_multi_value_separator(" | ");
        }
        false
    });
    assert!(matches!(result, Err(ExactReadError::Changed)));
    assert_eq!(budget.usage().0, baseline);
    let read = TagEngine::read_exact(&path).unwrap();
    assert_eq!(
        values(&read.text().state(&vorbis("ARTIST")).unwrap()),
        ["A", "B"]
    );
    tagrex_core::model::set_multi_value_separator(" / ");
    assert!(matches!(
        read.text()
            .prepare_patch(&vorbis("ARTIST"), ExactTextState::absent()),
        Err(ExactReadError::Changed)
    ));
    drop(read);
    assert_eq!(budget.usage().0, baseline);
    assert_read_only(&path, &bytes);
}

#[test]
fn mp4_box_overflow_depth_and_iteration_limits_refuse_without_large_allocation() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let mut overflow = atom(b"ftyp", b"M4A \0\0\0\0M4A isom");
    overflow.extend(1u32.to_be_bytes());
    overflow.extend(b"moov");
    overflow.extend(u64::MAX.to_be_bytes());
    let path = f.put("overflow.m4a", &overflow);
    assert!(TagEngine::read_exact(&path).is_err());
    let mut deep = atom(b"ilst", &[]);
    for _ in 0..10 {
        deep = atom(b"moov", &deep);
    }
    let mut bytes = atom(b"ftyp", b"M4A \0\0\0\0M4A isom");
    bytes.extend(deep);
    let path = f.put("depth.m4a", &bytes);
    assert!(matches!(
        TagEngine::read_exact(&path),
        Err(ExactReadError::Limit)
    ));
    let bytes = atom(b"free", &[]).repeat(65_537);
    let path = f.put("count.m4a", &bytes);
    assert!(matches!(
        TagEngine::read_exact(&path),
        Err(ExactReadError::Limit)
    ));
    for name in [b"stco", b"stsz"] {
        let mut bytes = include_bytes!("fixtures/minimal.m4a").to_vec();
        let at = bytes.windows(4).position(|bytes| bytes == name).unwrap();
        let count = if name == b"stsz" {
            bytes[at + 8..at + 12].fill(0);
            at + 12
        } else {
            at + 8
        };
        bytes[count..count + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        let path = f.put(
            &format!("bad-table-{}.m4a", String::from_utf8_lossy(name)),
            &bytes,
        );
        assert!(matches!(
            TagEngine::read_exact(&path),
            Err(ExactReadError::Invalid(_))
        ));
    }
}

#[test]
#[ignore = "manual independent Mutagen/ffprobe readback on owned synthetic native metadata"]
fn independent_native_readback_preserves_files_and_encoded_packets() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let flac_path = f.put(
        "independent.flac",
        &flac(&[
            ("ARTIST", "A; B"),
            ("ARTIST", "C"),
            ("ARTIST", "C"),
            ("ARTIST", ""),
        ]),
    );
    let mut mp3 = id3(&[id3_frame(b"TPE1", b"\x03A; B\0C\0C\0\0", 4)], 4);
    let mut frame = vec![0xff, 0xfb, 0x90, 0];
    frame.resize(417, 0);
    mp3.extend(frame.repeat(5));
    let mp3_path = f.put("independent.mp3", &mp3);
    let m4a_path = f.put(
        "independent.m4a",
        &mp4(&freeform(
            "org.exact.test",
            "NAME",
            &[
                (1, 0, b"A; B".to_vec()),
                (1, 0, Vec::new()),
                (1, 0, b"A; B".to_vec()),
            ],
        )),
    );
    let script = r#"
import sys, mutagen
from mutagen.flac import FLAC
from mutagen.id3 import ID3
from mutagen.mp4 import MP4
flac,mp3,mp4=sys.argv[1:]
assert FLAC(flac)['artist']==['A; B','C','C','']
# Mutagen can apply its own ID3 terminator interpretation. Assert common text,
# then independently verify every terminal empty directly in the raw envelope.
assert ID3(mp3)['TPE1'].text[:3]==['A; B','C','C']
b=open(mp3,'rb').read();at=b.index(b'TPE1');s=b[at+4:at+8];n=0
for v in s:n=(n<<7)|v
assert b[at+11:at+10+n].decode('utf-8').split('\x00')==['A; B','C','C','','']
assert [bytes(v) for v in MP4(mp4)['----:org.exact.test:NAME']]==[b'A; B',b'',b'A; B']
print('independent Mutagen',mutagen.version_string,'native text/namespace/raw-empty checks passed')
"#;
    let result = std::process::Command::new(
        std::env::var_os("TAGREX_TEST_PYTHON").unwrap_or_else(|| "python3".into()),
    )
    .arg("-c")
    .arg(script)
    .args([&flac_path, &mp3_path, &m4a_path])
    .output()
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    print!("{}", String::from_utf8_lossy(&result.stdout));
    for path in [&flac_path, &mp3_path, &m4a_path] {
        let before = std::fs::read(path).unwrap();
        let packet = || {
            let result = std::process::Command::new("ffprobe")
                .args(["-v", "error", "-show_packets", "-show_data_hash", "sha256"])
                .arg(path)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            result.stdout
        };
        let packets = packet();
        assert!(
            packets.windows(8).any(|bytes| bytes == b"[PACKET]"),
            "fixture has no independently readable encoded packets"
        );
        let read = TagEngine::read_exact(path).unwrap();
        assert!(!read.text().entries().is_empty());
        assert_eq!(packets, packet());
        assert_read_only(path, &before);
    }
}

#[test]
fn concurrent_exact_read_pressure_refuses_cleanly_and_retry_preserves_native_results() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use tagrex_core::resources::Budget;
    let f = Fixture::new();
    let bytes = flac(&[("ARTIST", "A; B"), ("ARTIST", ""), ("ARTIST", "A; B")]);
    let path = f.put("pressure.flac", &bytes);
    let budget = Budget::process();
    let baseline = budget.usage().0;
    let released = Arc::new((Mutex::new(false), Condvar::new()));
    let (sent, received) = mpsc::channel();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..12 {
            let released = released.clone();
            let sent = sent.clone();
            let path = &path;
            handles.push(scope.spawn(move || {
                let probes = std::cell::Cell::new(0);
                let result = TagEngine::read_exact_with_cancel(path, || {
                    let n = probes.get() + 1;
                    probes.set(n);
                    if n == 2 {
                        sent.send(true).unwrap();
                        let (lock, condition) = &*released;
                        let mut ready = lock.lock().unwrap();
                        while !*ready {
                            let (next, timed) = condition
                                .wait_timeout(ready, std::time::Duration::from_secs(10))
                                .unwrap();
                            ready = next;
                            assert!(!timed.timed_out(), "admitted reader was not released");
                        }
                    }
                    false
                });
                match result {
                    Ok(read) => {
                        assert_eq!(
                            values(&read.text().state(&vorbis("ARTIST")).unwrap()),
                            ["A; B", "", "A; B"]
                        );
                        true
                    }
                    Err(ExactReadError::Resource(tagrex_core::resources::AdmissionError::Busy)) => {
                        sent.send(false).unwrap();
                        false
                    }
                    other => panic!("unexpected pressure result {other:?}"),
                }
            }));
        }
        let admitted = (0..12)
            .map(|_| {
                received
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap()
            })
            .filter(|admitted| *admitted)
            .count();
        assert!(
            admitted > 0 && admitted < 12,
            "concurrent pressure must be explicitly admitted or refused"
        );
        let used = budget.usage().0;
        assert!(used.jobs <= 8 && used.transient <= 384 * 1024 * 1024);
        let (lock, condition) = &*released;
        *lock.lock().unwrap() = true;
        condition.notify_all();
        assert_eq!(
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .filter(|admitted| *admitted)
                .count(),
            admitted
        );
    });
    assert_eq!(budget.usage().0, baseline);
    for _ in 0..12 {
        let read = TagEngine::read_exact(&path).unwrap();
        assert_eq!(
            values(&read.text().state(&vorbis("ARTIST")).unwrap()),
            ["A; B", "", "A; B"]
        );
    }
    assert_eq!(budget.usage().0, baseline);
    assert_read_only(&path, &bytes);
}

#[test]
fn file_replacement_during_native_capture_is_refused_without_overwriting_the_replacement() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let before = flac(&[("TITLE", "Before")]);
    let after = flac(&[("TITLE", "External replacement")]);
    let path = f.put("before.flac", &before);
    let replacement = f.put("replacement.flac", &after);
    let probes = std::cell::Cell::new(0);
    let result = TagEngine::read_exact_with_cancel(&path, || {
        let n = probes.get() + 1;
        probes.set(n);
        if n == 2 {
            std::fs::rename(&replacement, &path).unwrap();
        }
        false
    });
    assert!(matches!(result, Err(ExactReadError::Changed)));
    assert_read_only(&path, &after);
}

#[cfg(unix)]
#[test]
fn nonregular_files_and_unpinned_final_links_are_refused() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let bytes = flac(&[("ARTIST", "A")]);
    let path = f.put("original.flac", &bytes);
    let link = f.0.join("link.flac");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(TagEngine::read_exact(&link).is_err());
    assert!(TagEngine::read_exact(&f.0).is_err());
    let socket = f.0.join("socket.flac");
    let _server = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    assert!(TagEngine::read_exact(&socket).is_err());
    assert_read_only(&path, &bytes);
}

#[cfg(unix)]
#[test]
fn held_source_survives_parent_swaps_across_every_read_checkpoint_and_native_container() {
    let _case = READ_CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let title = |value: &str| atom(b"\xa9nam", &data(1, 0, value.as_bytes()));
    let samples = [
        (
            "flac",
            flac(&[("TITLE", "First")]),
            flac(&[("TITLE", "Other")]),
            vorbis("TITLE"),
        ),
        (
            "wav",
            wav(&id3(&[id3_frame(b"TIT2", b"\x03First", 4)], 4)),
            wav(&id3(&[id3_frame(b"TIT2", b"\x03Other", 4)], 4)),
            id3_target("TIT2", None, None),
        ),
        (
            "m4a",
            mp4(&title("First")),
            mp4(&title("Other")),
            NativeTextTarget::Mp4 {
                key: Mp4TextKey::Fourcc(*b"\xa9nam"),
                kind: Mp4TextKind::Utf8,
                locale: 0,
            },
        ),
    ];
    let budget = tagrex_core::resources::Budget::process();
    let baseline = budget.usage().0;
    for (extension, first, other, target) in samples {
        let folder = fixture.0.join(extension);
        for (name, bytes) in [("a", &first), ("b", &other)] {
            std::fs::create_dir_all(folder.join(name)).unwrap();
            std::fs::write(folder.join(name).join(format!("track.{extension}")), bytes).unwrap();
        }
        let link = folder.join("selected");
        std::os::unix::fs::symlink("a", &link).unwrap();
        let path = link.join(format!("track.{extension}"));
        let calls = std::cell::Cell::new(0usize);
        drop(
            TagEngine::read_exact_with_cancel(&path, || {
                calls.set(calls.get() + 1);
                false
            })
            .unwrap(),
        );
        let total = calls.get();
        let mut accepted = 0;
        for swap_at in 1..total - 1 {
            let calls = std::cell::Cell::new(0usize);
            let result = TagEngine::read_exact_with_cancel(&path, || {
                let count = calls.get() + 1;
                calls.set(count);
                if count == swap_at {
                    std::fs::remove_file(&link).unwrap();
                    std::os::unix::fs::symlink("b", &link).unwrap();
                }
                if count == total - 1 {
                    std::fs::remove_file(&link).unwrap();
                    std::os::unix::fs::symlink("a", &link).unwrap();
                }
                false
            });
            match result {
                Ok(read) => {
                    let native = read.text().state(&target).unwrap();
                    let native = &native.values().unwrap().as_slice()[0];
                    assert_eq!(
                        Some(native),
                        read.legacy().file.tags.get(&TagField::Title),
                        "{extension}: mixed sources after checkpoint {swap_at}"
                    );
                    accepted += 1;
                }
                Err(ExactReadError::Changed) => {}
                Err(error) => panic!("{extension}: unexpected refusal at {swap_at}: {error}"),
            }
            // Early refusal can precede the restoring callback.
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink("a", &link).unwrap();
            assert_eq!(budget.usage().0, baseline);
        }
        assert!(
            accepted > 0,
            "held source must permit a consistent {extension} read"
        );
        assert_read_only(&folder.join("a").join(format!("track.{extension}")), &first);
        assert_read_only(&folder.join("b").join(format!("track.{extension}")), &other);
    }
}

#[test]
fn nested_reads_and_unwinding_restore_the_outer_held_source() {
    let _case = READ_CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let first = flac(&[("TITLE", "Outer")]);
    let other = flac(&[("TITLE", "Nested")]);
    let path = fixture.put("outer.flac", &first);
    let nested = fixture.put("nested.flac", &other);
    let budget = tagrex_core::resources::Budget::process();
    let baseline = budget.usage().0;
    let checkpoints = |path: &Path| {
        let count = std::cell::Cell::new(0usize);
        drop(
            TagEngine::read_exact_with_cancel(path, || {
                count.set(count.get() + 1);
                false
            })
            .unwrap(),
        );
        count.get()
    };
    let outer_total = checkpoints(&path);
    let nested_total = checkpoints(&nested);
    for mode in 0..3 {
        let count = std::cell::Cell::new(0usize);
        let read = TagEngine::read_exact_with_cancel(&path, || {
            count.set(count.get() + 1);
            if count.get() == outer_total - 2 {
                let entered = std::cell::Cell::new(0usize);
                let nested_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    TagEngine::read_exact_with_cancel(&nested, || {
                        entered.set(entered.get() + 1);
                        if entered.get() == nested_total - 2 {
                            if mode == 2 {
                                panic!("owned callback unwind probe");
                            }
                            return mode == 1;
                        }
                        false
                    })
                }));
                match (mode, nested_result) {
                    (0, Ok(Ok(read))) => assert_eq!(
                        read.legacy()
                            .file
                            .tags
                            .get(&TagField::Title)
                            .map(String::as_str),
                        Some("Nested")
                    ),
                    (1, Ok(Err(ExactReadError::Cancelled))) | (2, Err(_)) => {}
                    _ => panic!("unexpected nested result in mode {mode}"),
                }
            }
            false
        })
        .unwrap();
        assert_eq!(
            values(&read.text().state(&vorbis("TITLE")).unwrap()),
            ["Outer"]
        );
        assert_eq!(
            read.legacy()
                .file
                .tags
                .get(&TagField::Title)
                .map(String::as_str),
            Some("Outer")
        );
        drop(read);
        assert_eq!(budget.usage().0, baseline);
        assert_eq!(
            TagEngine::read(&nested)
                .unwrap()
                .tags
                .get(&TagField::Title)
                .map(String::as_str),
            Some("Nested")
        );
    }
    assert_read_only(&path, &first);
    assert_read_only(&nested, &other);
}

#[test]
fn held_source_keeps_the_original_extension_directed_parser_fallback() {
    let _case = READ_CASE.lock().unwrap_or_else(|p| p.into_inner());
    let fixture = Fixture::new();
    let mut bytes = id3(&[id3_frame(b"TIT2", b"\x00Weekend Lover", 3)], 3);
    let mut junk = vec![0; 1035];
    junk[400..402].copy_from_slice(&[0xff, 0xf1]);
    bytes.extend(junk);
    let mut silence = vec![0xff, 0xfb, 0x90, 0];
    silence.resize(417, 0);
    for _ in 0..5 {
        bytes.extend(&silence);
    }
    let path = fixture.put("fallback.mp3", &bytes);
    let legacy = TagEngine::read_with_props(&path).unwrap();
    assert_eq!(legacy.file.format, tagrex_core::model::AudioFormat::Mp3);
    let exact = TagEngine::read_exact(&path).unwrap();
    assert_eq!(exact.legacy().file.format, legacy.file.format);
    assert_eq!(exact.legacy().file.tags, legacy.file.tags);
    let target = NativeTextTarget::Id3 {
        frame: "TIT2".into(),
        description: None,
        language: None,
        encoding: Some(Id3TextEncoding::Latin1),
    };
    assert_eq!(
        values(&exact.text().state(&target).unwrap()),
        ["Weekend Lover"]
    );
    assert_read_only(&path, &bytes);
}

#[test]
#[ignore = "manual native exact/legacy parsing scale probe on 1000 disposable files per format"]
fn native_exact_read_scale_probe() {
    let _case = READ_CASE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let f = Fixture::new();
    let budget = tagrex_core::resources::Budget::process();
    let baseline = budget.usage().0;
    let samples = [
        (
            "flac",
            flac(&[("ARTIST", "A; B"), ("ARTIST", ""), ("ARTIST", "A; B")]),
            vorbis("ARTIST"),
        ),
        (
            "wav",
            wav(&id3(&[id3_frame(b"TPE1", b"\x03A; B\0\0A; B", 4)], 4)),
            id3_target("TPE1", None, None),
        ),
        (
            "m4a",
            mp4(&freeform(
                "org.probe",
                "NAME",
                &[
                    (1, 0, b"A; B".to_vec()),
                    (1, 0, Vec::new()),
                    (1, 0, b"A; B".to_vec()),
                ],
            )),
            NativeTextTarget::Mp4 {
                key: Mp4TextKey::Freeform {
                    mean: "org.probe".into(),
                    name: "NAME".into(),
                },
                kind: Mp4TextKind::Utf8,
                locale: 0,
            },
        ),
    ];
    for (extension, bytes, target) in samples {
        let paths: Vec<_> = (0..1000)
            .map(|i| f.put(&format!("{extension}-{i}.{extension}"), &bytes))
            .collect();
        for count in [100, 1000] {
            let mut legacy = Vec::new();
            let mut exact = Vec::new();
            for _ in 0..3 {
                let started = std::time::Instant::now();
                for path in &paths[..count] {
                    assert!(!TagEngine::read_with_props(path)
                        .unwrap()
                        .file
                        .tags
                        .is_empty());
                }
                legacy.push(started.elapsed().as_secs_f64() * 1000.0);
                let started = std::time::Instant::now();
                for path in &paths[..count] {
                    let read = TagEngine::read_exact(path).unwrap();
                    assert_eq!(
                        values(&read.text().state(&target).unwrap()),
                        ["A; B", "", "A; B"]
                    );
                }
                exact.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            println!("EXACT_READ_SCALE format={extension} files={count} legacy_ms={legacy:?} exact_ms={exact:?}");
            assert_eq!(budget.usage().0, baseline);
        }
    }
}
