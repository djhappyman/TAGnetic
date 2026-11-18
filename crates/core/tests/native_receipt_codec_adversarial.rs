//! Independent public-codec probes over disposable candidates and hostile wire data.
use sha2::{Digest, Sha256};
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
            ExactTextState, Id3TextEncoding, Mp4TextKey, Mp4TextKind, NativeTextContext,
            NativeTextTarget,
        },
        native_patch::{self as patch, Receipt},
        AudioFormat, Id3v2Revision, TagBlockKind, TagEngine,
    },
    resources::{Budget, Charge, Limits},
};
struct Observe;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn allocated(n: usize) {
    let now = LIVE.fetch_add(n, Ordering::Relaxed) + n;
    PEAK.fetch_max(now, Ordering::Relaxed);
}
// SAFETY: pointers/layouts are delegated unchanged to System. Counters observe
// successful allocations only; they never influence allocation or ownership.
unsafe impl GlobalAlloc for Observe {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            allocated(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            allocated(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            if n >= l.size() {
                allocated(n - l.size());
            } else {
                LIVE.fetch_sub(l.size() - n, Ordering::Relaxed);
            }
        }
        q
    }
}
#[global_allocator]
static ALLOC: Observe = Observe;
fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_RECEIPT_ADVERSARIAL_CASE").as_deref() == Ok(name) {
        return false;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_RECEIPT_ADVERSARIAL_CASE", name)
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
        let p = std::env::temp_dir().join(format!(
            "tagrex-codec-independent-{}-{}",
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
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn file(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap()
}
fn flac() -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let record = b"TITLE=A";
    let mut body = 4u32.to_le_bytes().to_vec();
    body.extend(b"test");
    body.extend(1u32.to_le_bytes());
    body.extend((record.len() as u32).to_le_bytes());
    body.extend(record);
    let n = body.len();
    out.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(body);
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out
}
fn number(out: &mut Vec<u8>, n: usize) {
    out.extend((n as u32).to_le_bytes());
}
fn blob(out: &mut Vec<u8>, bytes: &[u8]) {
    number(out, bytes.len());
    out.extend(bytes);
}
fn block(k: TagBlockKind) -> u8 {
    match k {
        TagBlockKind::Id3v1 => 1,
        TagBlockKind::Id3v2 => 2,
        TagBlockKind::VorbisComments => 3,
        TagBlockKind::Ape => 4,
        TagBlockKind::Mp4Ilst => 5,
        TagBlockKind::RiffInfo => 6,
        TagBlockKind::AiffText => 7,
    }
}
fn context_bytes(context: &NativeTextContext) -> Vec<u8> {
    let mut p = vec![
        match context.container {
            AudioFormat::Flac => 1,
            AudioFormat::Mp3 => 2,
            AudioFormat::Aac => 3,
            AudioFormat::M4a => 4,
            _ => panic!("fixture profile"),
        },
        block(context.selected_block.unwrap()),
        match context.id3_revision {
            None => 0,
            Some(Id3v2Revision::V2) => 2,
            Some(Id3v2Revision::V3) => 3,
            Some(Id3v2Revision::V4) => 4,
        },
    ];
    p.extend(context.policy_revision.to_le_bytes());
    p.push(1);
    p.push(1);
    p.extend(context.raw_digest.unwrap());
    p.extend(context.read_policy.revision.to_le_bytes());
    blob(&mut p, context.read_policy.multi_value_separator.as_bytes());
    number(&mut p, context.read_policy.read_priority.len());
    for k in &context.read_policy.read_priority {
        p.push(block(*k));
    }
    if let Some(cp) = &context.read_policy.legacy_code_page {
        p.push(1);
        blob(&mut p, cp.as_bytes());
    } else {
        p.push(0);
    }
    p.push(u8::from(context.read_policy.write_id3v23));
    p
}
fn prefix(context: &NativeTextContext) -> Vec<u8> {
    let mut p = context_bytes(context);
    p.push(1);
    blob(&mut p, b"TITLE");
    for value in [b"A", b"B"] {
        p.push(1);
        number(&mut p, 1);
        blob(&mut p, value);
    }
    p.extend([0; 64]);
    p
}
fn envelope(payload: &[u8]) -> Vec<u8> {
    let mut out = b"TRX-NTP1".to_vec();
    number(&mut out, payload.len());
    out.extend(Sha256::digest(payload));
    out.extend(payload);
    out
}
fn resource() -> Budget {
    Budget::new(Limits {
        retained: 16 << 20,
        transient: 64 << 20,
        total: 80 << 20,
        jobs: 1,
    })
}
#[test]
fn declared_nodes_refuse_without_allocating_past_the_work_reservation() {
    let name = "declared_nodes_refuse_without_allocating_past_the_work_reservation";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let path = f.put("candidate.flac", &flac());
    let read = TagEngine::read_exact(&path).unwrap();
    let mut payload = prefix(read.text().context());
    number(&mut payload, 1);
    payload.extend([0, 0]);
    number(&mut payload, 4096);
    for _ in 0..4096 {
        number(&mut payload, 0);
    }
    number(&mut payload, 0);
    let bytes = envelope(&payload);
    let budget = resource();
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let result = Receipt::decode_for_journal_with_budget(&bytes, budget.clone());
    let observed = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    let admitted = budget.usage().1.transient;
    assert!(result.is_err());
    assert_eq!(budget.usage().0, Charge::default());
    eprintln!(
        "CODEC_NODE_PREFLIGHT wire={} peak={observed} admitted={admitted}",
        bytes.len()
    );
    assert!(
        observed <= admitted,
        "node-vector allocation exceeds admitted work before refusal: {observed}>{admitted}"
    );
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
    let mut body: Vec<_> = frames.iter().flatten().copied().collect();
    body.extend([0; 16]);
    let mut out = b"ID3\x04\0\0".to_vec();
    out.extend(syncsafe(body.len()));
    out.extend(body);
    let mut audio = vec![0xff, 0xfb, 0x90, 0];
    audio.resize(417, 0);
    for _ in 0..4 {
        out.extend(&audio);
    }
    out
}
fn raw_digest(records: &[Vec<u8>]) -> [u8; 32] {
    let mut d = Sha256::new();
    for raw in records {
        d.update((raw.len() as u64).to_le_bytes());
        d.update(raw);
    }
    d.finalize().into()
}
fn id3_wire(
    context: &NativeTextContext,
    old: &[&str],
    new: &[&str],
    before: &[Vec<u8>],
    after: &[Vec<u8>],
) -> Vec<u8> {
    let mut p = context_bytes(context);
    p.push(2);
    blob(&mut p, b"TIT2");
    p.extend([0, 0, 2]);
    for values in [old, new] {
        p.push(1);
        number(&mut p, values.len());
        for v in values {
            blob(&mut p, v.as_bytes());
        }
    }
    p.extend(raw_digest(before));
    p.extend(raw_digest(after));
    number(&mut p, 1);
    p.extend([0, 0]);
    number(&mut p, before.len());
    for raw in before {
        blob(&mut p, raw);
    }
    number(&mut p, after.len());
    for raw in after {
        blob(&mut p, raw);
    }
    envelope(&p)
}
#[test]
fn impossible_duplicate_id3_target_records_refuse_during_decode() {
    let name = "impossible_duplicate_id3_target_records_refuse_during_decode";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let before = mp3(&[frame(b"TIT2", b"\x03A\0B")]);
    let path = f.put("candidate.mp3", &before);
    let read = TagEngine::read_exact(&path).unwrap();
    let context = read.text().context().clone();
    let target = NativeTextTarget::Id3 {
        frame: "TIT2".into(),
        description: None,
        language: None,
        encoding: Some(Id3TextEncoding::Utf8),
    };
    let intent = read
        .text()
        .prepare_patch(&target, ExactTextState::try_from(vec!["C".into()]).unwrap())
        .unwrap();
    patch::apply_prepared(&mut file(&path), &intent, &mut ()).unwrap();
    let saved = fs::read(&path).unwrap();
    let wire = id3_wire(
        &context,
        &["A", "B"],
        &["C"],
        &[frame(b"TIT2", b"\x03A"), frame(b"TIT2", b"\x03B")],
        &[frame(b"TIT2", b"\x03C")],
    );
    let decoded = Receipt::decode_for_journal(&wire);
    let refused = decoded.is_err();
    if let Ok(decoded) = decoded {
        let result = patch::restore(&mut file(&path), &decoded, &mut ());
        eprintln!(
            "IMPOSSIBLE_ID3 receipt accepted; restore_error={:?}; candidate_changed={}",
            result.err(),
            fs::read(&path).unwrap() != saved
        );
    }
    assert!(
        refused,
        "codec accepted two native target frames that the admitted ID3 profile cannot produce"
    );
    assert_eq!(fs::read(&path).unwrap(), saved);
}

fn state(values: &[&str]) -> ExactTextState {
    ExactTextState::try_from(values.iter().map(|v| v.to_string()).collect::<Vec<_>>()).unwrap()
}
fn roundtrip(path: &Path, target: &NativeTextTarget, new: ExactTextState) {
    let before = fs::read(path).unwrap();
    let read = TagEngine::read_exact(path).unwrap();
    let old = read.text().state(target).unwrap();
    let intent = read.text().prepare_patch(target, new.clone()).unwrap();
    let receipt = patch::apply_prepared(&mut file(path), &intent, &mut ()).unwrap();
    let after = fs::read(path).unwrap();
    let bytes = receipt.encode_for_journal().unwrap();
    let r = Receipt::decode_for_journal(bytes.bytes()).unwrap();
    assert_eq!(r.target(), target);
    assert_eq!(r.old(), &old);
    assert_eq!(r.new_state(), &new);
    let reverse = patch::restore(&mut file(path), &r, &mut ()).unwrap();
    assert_eq!(fs::read(path).unwrap(), before);
    let bytes = reverse.encode_for_journal().unwrap();
    let r = Receipt::decode_for_journal(bytes.bytes()).unwrap();
    patch::restore(&mut file(path), &r, &mut ()).unwrap();
    assert_eq!(fs::read(path).unwrap(), after);
}
#[test]
fn qualified_id3_comment_and_absent_custom_receipts_keep_other_native_identities() {
    let name = "qualified_id3_comment_and_absent_custom_receipts_keep_other_native_identities";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let mut english = b"\x03engdescript\0".to_vec();
    english.extend("Old 尾".as_bytes());
    let mut german = b"\x03deudescript\0".to_vec();
    german.extend("Behalten ä".as_bytes());
    let before = mp3(&[
        frame(b"TIT2", b"\x03Track"),
        frame(b"COMM", &english),
        frame(b"PRIV", b"owner\0\xff\x01"),
        frame(b"COMM", &german),
    ]);
    let path = f.put("qualified.mp3", &before);
    let target = NativeTextTarget::Id3 {
        frame: "COMM".into(),
        description: Some("descript".into()),
        language: Some(*b"eng"),
        encoding: Some(Id3TextEncoding::Utf8),
    };
    roundtrip(&path, &target, state(&["New ; / 尾"]));
    let target = NativeTextTarget::Id3 {
        frame: "TXXX".into(),
        description: Some("Native key 日本語".into()),
        language: None,
        encoding: Some(Id3TextEncoding::Utf8),
    };
    roundtrip(&path, &target, state(&["one; two", "", "one; two", ""]));
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
fn freeform(mean: &str, name: &str, kind: u32, locale: u32, values: &[&str]) -> Vec<u8> {
    let mut body = Vec::new();
    for (key, text) in [(b"mean", mean), (b"name", name)] {
        let mut b = vec![0; 4];
        b.extend(text.as_bytes());
        body.extend(atom(key, &b));
    }
    for value in values {
        let bytes = if kind == 2 {
            value
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>()
        } else {
            value.as_bytes().to_vec()
        };
        body.extend(data(kind, locale, &bytes));
    }
    atom(b"----", &body)
}
fn replace_ilst(bytes: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut at = 0;
    let mut result = Vec::new();
    while at < bytes.len() {
        let n = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let kind: &[u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
        let body = &bytes[at + 8..at + n];
        let body = match kind {
            b"moov" | b"udta" => replace_ilst(body, replacement),
            b"meta" => {
                let mut b = body[..4].to_vec();
                b.extend(replace_ilst(&body[4..], replacement));
                b
            }
            b"ilst" => replacement.to_vec(),
            _ => body.to_vec(),
        };
        result.extend(atom(kind, &body));
        at += n;
    }
    result
}
#[test]
fn mp4_freeform_utf16_locale_and_absence_survive_codec_restore_and_redo() {
    let name = "mp4_freeform_utf16_locale_and_absence_survive_codec_restore_and_redo";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let mut ilst = atom(b"\xa9nam", &data(1, 0, b"Untouched"));
    ilst.extend(freeform(
        "com.example.one",
        "same key",
        2,
        0x409,
        &["A; B", "", "尾"],
    ));
    ilst.extend(freeform(
        "com.example.two",
        "same key",
        1,
        0,
        &["Leave this namespace"],
    ));
    let before = replace_ilst(include_bytes!("fixtures/minimal.m4a"), &ilst);
    let path = f.put("qualified.m4a", &before);
    let target = NativeTextTarget::Mp4 {
        key: Mp4TextKey::Freeform {
            mean: "com.example.one".into(),
            name: "same key".into(),
        },
        kind: Mp4TextKind::Utf16,
        locale: 0x409,
    };
    roundtrip(&path, &target, state(&["新しい", "新しい", ""]));
    let target = NativeTextTarget::Mp4 {
        key: Mp4TextKey::Freeform {
            mean: "new namespace".into(),
            name: "same key".into(),
        },
        kind: Mp4TextKind::Utf8,
        locale: 17,
    };
    roundtrip(&path, &target, state(&["Literal ; /", "", ""]));
}

#[test]
fn impossible_duplicate_mp4_target_items_refuse_during_decode() {
    let name = "impossible_duplicate_mp4_target_items_refuse_during_decode";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let mut values = data(1, 0, b"A");
    values.extend(data(1, 0, b"B"));
    let before = replace_ilst(
        include_bytes!("fixtures/minimal.m4a"),
        &atom(b"\xa9nam", &values),
    );
    let path = f.put("duplicate.m4a", &before);
    let target = NativeTextTarget::Mp4 {
        key: Mp4TextKey::Fourcc(*b"\xa9nam"),
        kind: Mp4TextKind::Utf8,
        locale: 0,
    };
    let read = TagEngine::read_exact(&path).unwrap();
    let context = read.text().context().clone();
    let intent = read.text().prepare_patch(&target, state(&["C"])).unwrap();
    patch::apply_prepared(&mut file(&path), &intent, &mut ()).unwrap();
    let saved = fs::read(&path).unwrap();
    let before = [
        atom(b"\xa9nam", &data(1, 0, b"A")),
        atom(b"\xa9nam", &data(1, 0, b"B")),
    ];
    let after = [atom(b"\xa9nam", &data(1, 0, b"C"))];
    let mut p = context_bytes(&context);
    p.extend([3, 0]);
    p.extend(b"\xa9nam");
    p.extend(1u32.to_le_bytes());
    p.extend(0u32.to_le_bytes());
    for values in [&["A", "B"][..], &["C"][..]] {
        p.push(1);
        number(&mut p, values.len());
        for v in values {
            blob(&mut p, v.as_bytes());
        }
    }
    p.extend(raw_digest(&before));
    p.extend(raw_digest(&after));
    number(&mut p, 1);
    p.extend([0, 0]);
    number(&mut p, 2);
    for raw in &before {
        blob(&mut p, raw);
    }
    number(&mut p, 1);
    blob(&mut p, &after[0]);
    let decoded = Receipt::decode_for_journal(&envelope(&p));
    let refused = decoded.is_err();
    if let Ok(decoded) = decoded {
        let result = patch::restore(&mut file(&path), &decoded, &mut ());
        eprintln!(
            "IMPOSSIBLE_MP4 receipt accepted; restore_error={:?}; candidate_changed={}",
            result.err(),
            fs::read(&path).unwrap() != saved
        );
    }
    assert!(
        refused,
        "codec accepted duplicate target items which the admitted MP4 profile cannot produce"
    );
    assert_eq!(fs::read(&path).unwrap(), saved);
}

fn vorbis_raw(value: &str) -> Vec<u8> {
    let s = format!("TITLE={value}");
    let mut b = (s.len() as u32).to_le_bytes().to_vec();
    b.extend(s.as_bytes());
    b
}
fn flac_wire(
    context: &NativeTextContext,
    old: &[&str],
    new: &[&str],
    runs: &[Vec<Vec<u8>>],
    after: &[Vec<u8>],
) -> Vec<u8> {
    let mut p = context_bytes(context);
    p.push(1);
    blob(&mut p, b"TITLE");
    for values in [old, new] {
        p.push(1);
        number(&mut p, values.len());
        for v in values {
            blob(&mut p, v.as_bytes());
        }
    }
    let before: Vec<_> = runs.iter().flatten().cloned().collect();
    p.extend(raw_digest(&before));
    p.extend(raw_digest(after));
    number(&mut p, runs.len());
    for run in runs {
        p.extend([0, 0]);
        number(&mut p, run.len());
        for raw in run {
            blob(&mut p, raw);
        }
    }
    number(&mut p, after.len());
    for raw in after {
        blob(&mut p, raw);
    }
    envelope(&p)
}
#[test]
fn maximum_empty_value_vectors_fit_both_decode_work_and_retained_admission() {
    let name = "maximum_empty_value_vectors_fit_both_decode_work_and_retained_admission";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let path = f.put("candidate.flac", &flac());
    let read = TagEngine::read_exact(&path).unwrap();
    let after = vec![vorbis_raw(""); 1024];
    let values = vec![""; 1024];
    let bytes = flac_wire(
        read.text().context(),
        &["A"],
        &values,
        &[vec![vorbis_raw("A")]],
        &after,
    );
    let budget = resource();
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let receipt = Receipt::decode_for_journal_with_budget(&bytes, budget.clone()).unwrap();
    let observed = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    let retained = LIVE.load(Ordering::Relaxed).saturating_sub(before);
    let usage = budget.usage();
    eprintln!("CODEC_EMPTY_VECTORS wire={} peak={observed} work={} actual_retained={retained} charged_retained={}",bytes.len(),usage.1.transient,usage.0.retained);
    assert!(
        observed <= usage.1.transient,
        "decode work exceeds admission"
    );
    assert!(
        retained <= usage.0.retained,
        "retained allocations exceed their transferred lease: {retained}>{}",
        usage.0.retained
    );
    drop(receipt);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn semantic_policy_changes_refuse_until_an_explicit_compatible_rebind() {
    let name = "semantic_policy_changes_refuse_until_an_explicit_compatible_rebind";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let before = flac();
    let path = f.put("policy.flac", &before);
    let target = NativeTextTarget::Vorbis {
        name: "TITLE".into(),
    };
    let read = TagEngine::read_exact(&path).unwrap();
    let intent = read.text().prepare_patch(&target, state(&["B"])).unwrap();
    let receipt = patch::apply_prepared(&mut file(&path), &intent, &mut ()).unwrap();
    let saved = receipt.encode_for_journal().unwrap();
    let candidate = fs::read(&path).unwrap();
    let old_separator = receipt.context().read_policy.multi_value_separator.clone();
    let resource = resource();
    tagrex_core::model::set_multi_value_separator(" independent changed separator ");
    let decoded = Receipt::decode_for_journal_with_budget(saved.bytes(), resource.clone()).unwrap();
    assert!(decoded.rebind_current_policy().is_err());
    assert_eq!(resource.usage().0, Charge::default());
    assert!(patch::restore(&mut file(&path), &receipt, &mut ()).is_err());
    assert_eq!(fs::read(&path).unwrap(), candidate);
    tagrex_core::model::set_multi_value_separator(&old_separator);
    let decoded = Receipt::decode_for_journal_with_budget(saved.bytes(), resource.clone()).unwrap();
    assert_ne!(
        decoded.context().policy_revision,
        tagrex_core::model::read_policy_revision()
    );
    let decoded = decoded.rebind_current_policy().unwrap();
    patch::restore(&mut file(&path), &decoded, &mut ()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    drop(decoded);
    assert_eq!(resource.usage().0, Charge::default());
}

#[test]
fn checksum_valid_invalid_counts_enums_flags_and_native_records_release_every_owner() {
    let name = "checksum_valid_invalid_counts_enums_flags_and_native_records_release_every_owner";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let path = f.put("candidate.flac", &flac());
    let target = NativeTextTarget::Vorbis {
        name: "TITLE".into(),
    };
    let read = TagEngine::read_exact(&path).unwrap();
    let intent = read.text().prepare_patch(&target, state(&["B"])).unwrap();
    let receipt = patch::apply_prepared(&mut file(&path), &intent, &mut ()).unwrap();
    let encoded = receipt.encode_for_journal().unwrap();
    let original = encoded.bytes();
    let resource = resource();
    let baseline = fs::read(&path).unwrap();
    let mut attempts = 0usize;
    // Native context tags and boolean fields have fixed wire offsets. Every
    // mutation has a recomputed integrity hash so it reaches structural checks.
    for (at, replacement) in [
        (44, 0),
        (44, 255),
        (45, 0),
        (45, 8),
        (46, 2),
        (55, 2),
        (56, 2),
    ] {
        let mut payload = original[44..].to_vec();
        payload[at - 44] = replacement;
        let bytes = envelope(&payload);
        assert!(Receipt::decode_for_journal_with_budget(&bytes, resource.clone()).is_err());
        attempts += 1;
        assert_eq!(resource.usage().0, Charge::default());
    }
    for count in [1025, 4096, u32::MAX as usize] {
        let mut p = prefix(read.text().context());
        number(&mut p, 1);
        p.extend([0, 0]);
        number(&mut p, count);
        p.extend([0; 128]);
        let bytes = envelope(&p);
        assert!(Receipt::decode_for_journal_with_budget(&bytes, resource.clone()).is_err());
        attempts += 1;
    }
    let raw = vorbis_raw("A");
    let good_after = [vorbis_raw("B")];
    for invalid in [vec![], b"\x01\0\0\0X".to_vec(), vorbis_raw("A\0B"), {
        let mut b = raw.clone();
        b[4] = 0;
        b
    }] {
        let wire = flac_wire(
            read.text().context(),
            &["A"],
            &["B"],
            &[vec![invalid]],
            &good_after,
        );
        assert!(Receipt::decode_for_journal_with_budget(&wire, resource.clone()).is_err());
        attempts += 1;
        assert_eq!(resource.usage().0, Charge::default());
    }
    assert_eq!(fs::read(&path).unwrap(), baseline);
    eprintln!("CODEC_STRUCTURAL_REFUSALS {attempts}");
}

#[test]
fn busy_and_too_large_decodes_preserve_existing_owners_and_recover_capacity() {
    let name = "busy_and_too_large_decodes_preserve_existing_owners_and_recover_capacity";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let path = f.put("pressure.flac", &flac());
    let target = NativeTextTarget::Vorbis {
        name: "TITLE".into(),
    };
    let read = TagEngine::read_exact(&path).unwrap();
    let intent = read.text().prepare_patch(&target, state(&["B"])).unwrap();
    let receipt = patch::apply_prepared(&mut file(&path), &intent, &mut ()).unwrap();
    let wire = receipt.encode_for_journal().unwrap();
    let budget = Budget::new(Limits {
        retained: 256 << 10,
        transient: 64 << 20,
        total: 65 << 20,
        jobs: 1,
    });
    let mut owners = Vec::with_capacity(32);
    let work = budget
        .try_acquire(Charge {
            jobs: 1,
            ..Default::default()
        })
        .unwrap();
    let baseline = budget.usage().0;
    assert!(matches!(
        Receipt::decode_for_journal_with_budget(wire.bytes(), budget.clone()),
        Err(patch::Error::Resource(
            tagrex_core::resources::AdmissionError::Busy
        ))
    ));
    assert_eq!(budget.usage().0, baseline);
    drop(work);
    let small = Budget::new(Limits {
        retained: 256 << 10,
        transient: 32 << 10,
        total: 288 << 10,
        jobs: 1,
    });
    assert!(matches!(
        Receipt::decode_for_journal_with_budget(wire.bytes(), small.clone()),
        Err(patch::Error::Resource(
            tagrex_core::resources::AdmissionError::TooLarge
        ))
    ));
    assert_eq!(small.usage().0, Charge::default());
    let mut refused = false;
    for _ in 0..32 {
        match Receipt::decode_for_journal_with_budget(wire.bytes(), budget.clone()) {
            Ok(r) => owners.push(r),
            Err(patch::Error::Resource(tagrex_core::resources::AdmissionError::Busy)) => {
                refused = true;
                break;
            }
            Err(e) => panic!("unexpected refusal: {e}"),
        }
    }
    assert!(refused && !owners.is_empty());
    let usage = budget.usage().0;
    let retained = LIVE.load(Ordering::Relaxed);
    for _ in 0..24 {
        assert!(Receipt::decode_for_journal_with_budget(wire.bytes(), budget.clone()).is_err());
        assert_eq!(budget.usage().0, usage);
        assert_eq!(LIVE.load(Ordering::Relaxed), retained);
    }
    drop(owners.pop().unwrap());
    owners.push(Receipt::decode_for_journal_with_budget(wire.bytes(), budget.clone()).unwrap());
    assert_eq!(budget.usage().0, usage);
    eprintln!(
        "CODEC_PRESSURE live_owners={} stable_refusals=24",
        owners.len()
    );
    drop(owners);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn typed_targets_do_not_enter_through_the_reverse_receipt_exception() {
    let name = "typed_targets_do_not_enter_through_the_reverse_receipt_exception";
    if isolated(name) {
        return;
    }
    let f = Fixture::new();
    let path = f.put("typed.flac", &flac());
    let read = TagEngine::read_exact(&path).unwrap();
    let resource = resource();
    for key in [
        "TRACKNUMBER",
        "TRACKTOTAL",
        "DISCNUMBER",
        "DISCTOTAL",
        "DATE",
        "BPM",
    ] {
        let mut p = context_bytes(read.text().context());
        p.push(1);
        blob(&mut p, key.as_bytes());
        for value in [b"1", b"2"] {
            p.push(1);
            number(&mut p, 1);
            blob(&mut p, value);
        }
        let make = |value: &str| {
            let item = format!("{key}={value}");
            let mut b = (item.len() as u32).to_le_bytes().to_vec();
            b.extend(item.as_bytes());
            b
        };
        let before = [make("1")];
        let after = [make("2")];
        p.extend(raw_digest(&before));
        p.extend(raw_digest(&after));
        number(&mut p, 1);
        p.extend([0, 0]);
        number(&mut p, 1);
        blob(&mut p, &before[0]);
        number(&mut p, 1);
        blob(&mut p, &after[0]);
        let error = Receipt::decode_for_journal_with_budget(&envelope(&p), resource.clone())
            .err()
            .unwrap_or_else(|| panic!("typed native target {key} admitted"));
        assert!(
            error.to_string().contains("text capability"),
            "{key}: {error}"
        );
        assert_eq!(resource.usage().0, Charge::default());
    }
}
