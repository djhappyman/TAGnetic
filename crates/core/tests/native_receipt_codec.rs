//! Durable receipt evidence on disposable candidates; no Executor readiness claim.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    model::{
        exact::{ExactTextState, NativeTextTarget},
        native_patch::{self, Receipt},
        TagEngine,
    },
    resources::{Budget, Charge, Limits},
};
// Functional codec cases share the real process coordinator. Serialize their
// setup/mutation rather than treating legitimate harness contention as an
// unlimited-job guarantee. Explicit refusal tests use independent local budgets.
static CASE: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Fixture(
    PathBuf,
    #[allow(dead_code)] std::sync::MutexGuard<'static, ()>,
);
impl Fixture {
    fn new() -> Self {
        let guard = CASE.lock().unwrap_or_else(|poison| poison.into_inner());
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-receipt-codec-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path, guard)
    }
    fn path(&self) -> PathBuf {
        self.0.join("candidate.flac")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn flac() -> Vec<u8> {
    let mut bytes = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    bytes.extend(info);
    let records = [
        "ARTIST=A; B",
        "TITLE=Saved",
        "GENRE=House",
        "GENRE=house",
        "CUSTOM=",
        "CUSTOM=尾",
    ];
    let mut comments = 4u32.to_le_bytes().to_vec();
    comments.extend(b"test");
    comments.extend((records.len() as u32).to_le_bytes());
    for record in records {
        comments.extend((record.len() as u32).to_le_bytes());
        comments.extend(record.as_bytes());
    }
    let n = comments.len();
    bytes.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    bytes.extend(comments);
    bytes.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    bytes
}
fn file(path: &Path) -> std::fs::File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap()
}
fn create(f: &Fixture) -> Receipt {
    fs::write(f.path(), flac()).unwrap();
    let intent = TagEngine::read_exact(&f.path())
        .unwrap()
        .text()
        .prepare_patch(
            &NativeTextTarget::Vorbis {
                name: "TITLE".into(),
            },
            ExactTextState::try_from(vec!["New 尾".into(), "".into(), "New 尾".into(), "".into()])
                .unwrap(),
        )
        .unwrap();
    native_patch::apply_prepared(&mut file(&f.path()), &intent, &mut ()).unwrap()
}
fn budget() -> Budget {
    Budget::new(Limits {
        retained: 4 << 20,
        transient: 8 << 20,
        total: 12 << 20,
        jobs: 1,
    })
}
fn seal(bytes: &mut [u8]) {
    let digest: [u8; 32] = Sha256::digest(&bytes[44..]).into();
    bytes[12..44].copy_from_slice(&digest);
}
#[test]
fn encoded_receipt_roundtrip_restores_raw_bytes_and_reverse_receipt_redoes() {
    let f = Fixture::new();
    let receipt = create(&f);
    let applied = fs::read(f.path()).unwrap();
    let encoded = receipt.encode_for_journal().unwrap();
    let resource = budget();
    let baseline = resource.usage().0;
    let decoded =
        Receipt::decode_for_journal_with_budget(encoded.bytes(), resource.clone()).unwrap();
    assert_eq!(decoded.old(), receipt.old());
    assert_eq!(decoded.new_state(), receipt.new_state());
    assert_eq!(
        decoded.before_target_digest(),
        receipt.before_target_digest()
    );
    assert_eq!(
        decoded.encode_for_journal().unwrap().bytes(),
        encoded.bytes()
    );
    assert!(resource.usage().0.retained > baseline.retained);
    let reverse = native_patch::restore(&mut file(&f.path()), &decoded, &mut ()).unwrap();
    assert_eq!(fs::read(f.path()).unwrap(), flac());
    let reverse = reverse.encode_for_journal().unwrap();
    let reverse = Receipt::decode_for_journal(reverse.bytes()).unwrap();
    native_patch::restore(&mut file(&f.path()), &reverse, &mut ()).unwrap();
    assert_eq!(fs::read(f.path()).unwrap(), applied);
    drop(decoded);
    assert_eq!(resource.usage().0, baseline);
}
#[test]
fn truncation_and_single_byte_damage_refuse_without_retaining_decoded_owners() {
    let f = Fixture::new();
    let receipt = create(&f);
    let encoded = receipt.encode_for_journal().unwrap();
    let resource = budget();
    let before = fs::read(f.path()).unwrap();
    for end in 0..encoded.bytes().len() {
        assert!(
            Receipt::decode_for_journal_with_budget(&encoded.bytes()[..end], resource.clone())
                .is_err()
        );
    }
    for at in 0..encoded.bytes().len() {
        let mut bad = encoded.bytes().to_vec();
        bad[at] ^= 1;
        assert!(
            Receipt::decode_for_journal_with_budget(&bad, resource.clone()).is_err(),
            "damage accepted at {at}"
        );
        assert_eq!(resource.usage().0, Charge::default());
    }
    assert_eq!(fs::read(f.path()).unwrap(), before);
}
#[test]
fn integrity_valid_unselected_raw_records_cannot_become_restore_authority() {
    let f = Fixture::new();
    let receipt = create(&f);
    let encoded = receipt.encode_for_journal().unwrap();
    let mut bad = encoded.bytes().to_vec();
    let mut changed = 0;
    for at in 44..bad.len().saturating_sub(5) {
        if bad.get(at..at + 6) == Some(b"TITLE=") {
            bad[at..at + 5].copy_from_slice(b"OTHER");
            changed += 1;
        }
    }
    assert!(changed >= 2);
    seal(&mut bad);
    let before = fs::read(f.path()).unwrap();
    let error = Receipt::decode_for_journal(&bad).err().unwrap();
    assert!(error.to_string().contains("does not own target"), "{error}");
    assert_eq!(fs::read(f.path()).unwrap(), before);
}
#[test]
fn checksum_valid_semantic_lie_and_trailing_payload_are_refused() {
    let f = Fixture::new();
    let receipt = create(&f);
    let encoded = receipt.encode_for_journal().unwrap();
    let mut bad = encoded.bytes().to_vec();
    let at = bad.windows(5).position(|s| s == b"Saved").unwrap();
    bad[at..at + 5].copy_from_slice(b"Lying");
    seal(&mut bad);
    assert!(Receipt::decode_for_journal(&bad)
        .err()
        .unwrap()
        .to_string()
        .contains("semantic records"));
    let mut bad = encoded.bytes().to_vec();
    bad.push(0);
    let n = (bad.len() - 44) as u32;
    bad[8..12].copy_from_slice(&n.to_le_bytes());
    seal(&mut bad);
    assert!(Receipt::decode_for_journal(&bad).is_err());
}
#[test]
fn pressure_refusal_preserves_existing_receipt_and_budget_owners() {
    let f = Fixture::new();
    let receipt = create(&f);
    let encoded = receipt.encode_for_journal().unwrap();
    let resource = budget();
    let pressure = resource
        .try_acquire(Charge {
            transient: 8 << 20,
            ..Default::default()
        })
        .unwrap();
    let before = resource.usage().0;
    assert!(Receipt::decode_for_journal_with_budget(encoded.bytes(), resource.clone()).is_err());
    assert_eq!(resource.usage().0, before);
    drop(pressure);
    drop(Receipt::decode_for_journal_with_budget(encoded.bytes(), resource.clone()).unwrap());
    assert_eq!(resource.usage().0, Charge::default());
}
#[test]
fn persistent_receipt_rebinds_identical_semantics_after_a_fresh_process() {
    let name = "persistent_receipt_rebinds_identical_semantics_after_a_fresh_process";
    if let Ok(root) = std::env::var("TAGREX_RECEIPT_REOPEN") {
        let root = PathBuf::from(root);
        tagrex_core::model::set_multi_value_separator(" ~~ ");
        let receipt =
            Receipt::decode_for_journal(&fs::read(root.join("receipt.bin")).unwrap()).unwrap();
        assert_ne!(
            receipt.context().policy_revision,
            tagrex_core::model::read_policy_revision()
        );
        let receipt = receipt.rebind_current_policy().unwrap();
        native_patch::restore(&mut file(&root.join("candidate.flac")), &receipt, &mut ()).unwrap();
        assert_eq!(fs::read(root.join("candidate.flac")).unwrap(), flac());
        return;
    }
    if std::env::var("TAGREX_RECEIPT_ISOLATED").is_err() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env("TAGREX_RECEIPT_ISOLATED", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    tagrex_core::model::set_multi_value_separator(" ~~ ");
    tagrex_core::model::set_multi_value_separator(" other ");
    tagrex_core::model::set_multi_value_separator(" ~~ ");
    let f = Fixture::new();
    let receipt = create(&f);
    let encoded = receipt.encode_for_journal().unwrap();
    fs::write(f.0.join("receipt.bin"), encoded.bytes()).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_RECEIPT_REOPEN", &f.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn checksum_valid_mutation_sweep_never_panics_or_leaks_decoded_owners() {
    let f = Fixture::new();
    let receipt = create(&f);
    let encoded = receipt.encode_for_journal().unwrap();
    let resource = budget();
    let mut accepted = 0usize;
    for at in 44..encoded.bytes().len() {
        for mask in [1, 0xff] {
            let mut bytes = encoded.bytes().to_vec();
            bytes[at] ^= mask;
            seal(&mut bytes);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Receipt::decode_for_journal_with_budget(&bytes, resource.clone())
            }));
            let result = result.expect("bounded decoder panicked on checksum-valid damage");
            if let Ok(receipt) = result {
                accepted += 1;
                let roundtrip = receipt.encode_for_journal().unwrap();
                let again =
                    Receipt::decode_for_journal_with_budget(roundtrip.bytes(), resource.clone())
                        .unwrap();
                assert_eq!(again.old(), receipt.old());
                assert_eq!(again.new_state(), receipt.new_state());
            }
            assert_eq!(resource.usage().0, Charge::default());
        }
    }
    eprintln!(
        "RECEIPT_CODEC checksum_valid_mutations={} structurally_valid={accepted}",
        (encoded.bytes().len() - 44) * 2
    );
}

#[test]
#[ignore = "requires FFmpeg and FFprobe independent packet reader"]
fn actual_mp3_and_m4a_receipts_roundtrip_native_targets_and_encoded_packets() {
    use std::process::Command;
    let f = Fixture::new();
    for (extension, codec) in [("mp3", "libmp3lame"), ("m4a", "aac")] {
        let path = f.0.join(format!("real.{extension}"));
        let mut command = Command::new("ffmpeg");
        command.args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=8000:cl=mono",
            "-t",
            "0.2",
            "-c:a",
            codec,
            "-metadata",
            "title=Saved",
            "-metadata",
            "artist=Literal ; / artist",
        ]);
        if extension == "mp3" {
            command.args(["-id3v2_version", "4", "-write_id3v1", "0"]);
        }
        let result = command.arg(&path).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let before = fs::read(&path).unwrap();
        let packets = |path: &Path| {
            let output = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "a:0",
                    "-show_packets",
                    "-show_entries",
                    "packet=data_hash",
                    "-show_data_hash",
                    "sha256",
                    "-of",
                    "compact=p=0:nk=1",
                ])
                .arg(path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.stdout.is_empty());
            output.stdout
        };
        let audio = packets(&path);
        let read = TagEngine::read_exact(&path).unwrap();
        let target = read
            .text()
            .resolve_field(&tagrex_core::model::TagField::Title)
            .unwrap()
            .unwrap()
            .target
            .clone();
        let new = ExactTextState::try_from(vec![
            "Repeated ; /".into(),
            "".into(),
            "Repeated ; /".into(),
        ])
        .unwrap();
        let intent = read.text().prepare_patch(&target, new.clone()).unwrap();
        let receipt = native_patch::apply_prepared(&mut file(&path), &intent, &mut ()).unwrap();
        let saved = receipt.encode_for_journal().unwrap();
        assert_eq!(packets(&path), audio);
        assert_eq!(
            TagEngine::read_exact(&path)
                .unwrap()
                .text()
                .state(&target)
                .unwrap(),
            new
        );
        let decoded = Receipt::decode_for_journal(saved.bytes()).unwrap();
        let reverse = native_patch::restore(&mut file(&path), &decoded, &mut ()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(packets(&path), audio);
        let reverse = reverse.encode_for_journal().unwrap();
        let reverse = Receipt::decode_for_journal(reverse.bytes()).unwrap();
        native_patch::restore(&mut file(&path), &reverse, &mut ()).unwrap();
        assert_eq!(packets(&path), audio);
        // A native fourcc deletion's reverse is a historical restore, even
        // though a fresh absent-fourcc creation is not an admitted user edit.
        let saved_before_clear = fs::read(&path).unwrap();
        let read = TagEngine::read_exact(&path).unwrap();
        let clear = read
            .text()
            .prepare_patch(&target, ExactTextState::absent())
            .unwrap();
        let clear = native_patch::apply_prepared(&mut file(&path), &clear, &mut ()).unwrap();
        assert!(TagEngine::read_exact(&path)
            .unwrap()
            .text()
            .state(&target)
            .unwrap()
            .values()
            .is_none());
        let encoded = clear.encode_for_journal().unwrap();
        let decoded = Receipt::decode_for_journal(encoded.bytes()).unwrap();
        let reverse = native_patch::restore(&mut file(&path), &decoded, &mut ()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), saved_before_clear);
        let reverse = reverse.encode_for_journal().unwrap();
        let reverse = Receipt::decode_for_journal(reverse.bytes()).unwrap();
        native_patch::restore(&mut file(&path), &reverse, &mut ()).unwrap();
        assert!(TagEngine::read_exact(&path)
            .unwrap()
            .text()
            .state(&target)
            .unwrap()
            .values()
            .is_none());
        assert_eq!(packets(&path), audio);
    }
}
