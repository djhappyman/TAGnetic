//! Independent Unix typed-identity recovery checks; all writes use owned fixtures.
#![cfg(unix)]
use std::{
    collections::BTreeMap,
    os::unix::fs::{symlink, MetadataExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tagrex_core::{
    journal::{AppliedBatch, BatchId, JournalError, MoveStage, SqliteJournal, UndoJournal},
    plan::{ChangePlan, Executor, FileChange},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "tagrex-u2-fresh-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn db(&self) -> PathBuf {
        self.0.join("history.sqlite")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wav(n: u8) -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend(356u32.to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(320u32.to_le_bytes());
    b.extend([n; 320]);
    b
}
fn plan(root: &Path, links: bool, copy: bool) -> ChangePlan {
    let mut changes = (0..3)
        .map(|n| FileChange {
            path: root.join(format!("source-{n}.wav")),
            rename_to: Some(root.join(format!("target-{n}.wav"))),
            copy: copy && n == 2,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    if links {
        changes[0]
            .sidecar_renames
            .push((root.join("source.cue"), root.join("target.cue")));
    }
    ChangePlan {
        changes,
        ..Default::default()
    }
}
struct ExitAfterCapture {
    journal: SqliteJournal,
    calls: usize,
    link: bool,
}
impl UndoJournal for ExitAfterCapture {
    fn supports_copy_checkpoints(&self) -> bool {
        self.journal.supports_copy_checkpoints()
    }
    fn checkpoint_copy(
        &mut self,
        id: BatchId,
        stamp: &tagrex_core::journal::CopyStamp,
        previous: Option<&tagrex_core::journal::CopyStamp>,
    ) -> Result<(), tagrex_core::journal::JournalError> {
        self.journal.checkpoint_copy(id, stamp, previous)
    }

    fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
        self.journal.record(b)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.journal.batches()
    }
    fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
        self.journal.batch(id)
    }
    fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
        self.journal.finish(id, b)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.journal.rollback(id)
    }
    fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
        self.journal.move_stages(id)
    }
    fn prepare_move_stage(&mut self, id: BatchId, s: &MoveStage) -> Result<(), JournalError> {
        self.journal.prepare_move_stage(id, s)?;
        self.calls += 1;
        if (!self.link && self.calls == 3)
            || (self.link && s.target.extension().is_some_and(|e| e == "cue"))
        {
            std::fs::rename(&s.from, &s.path).unwrap();
            std::process::exit(86);
        }
        Ok(())
    }
}
#[test]
fn interrupted_fixture_child() {
    let Some(root) = std::env::var_os("TAGREX_U2_FRESH_FIXTURE") else {
        return;
    };
    let root = PathBuf::from(root);
    let link = std::env::var_os("TAGREX_U2_FRESH_LINK").is_some();
    let mut j = ExitAfterCapture {
        journal: SqliteJournal::open(&root.join("history.sqlite")).unwrap(),
        calls: 0,
        link,
    };
    let copy = std::env::var_os("TAGREX_U2_FRESH_COPY").is_some();
    Executor::apply(
        &plan(&root, link, copy),
        &mut j,
        std::slice::from_ref(&root),
    )
    .unwrap();
    panic!("fixture never reached capture");
}
fn seed(link: bool, interrupted: bool) -> (Fixture, BatchId, Vec<MoveStage>) {
    seed_with_copy(link, interrupted, link && !interrupted)
}
fn seed_with_copy(link: bool, interrupted: bool, copy: bool) -> (Fixture, BatchId, Vec<MoveStage>) {
    let f = Fixture::new();
    for n in 0..3 {
        std::fs::write(f.0.join(format!("source-{n}.wav")), wav(n)).unwrap();
    }
    std::fs::write(f.0.join("foreign-sibling"), b"must remain byte-identical").unwrap();
    if link {
        std::fs::write(f.0.join("referent"), b"never mutate referent").unwrap();
        symlink("referent", f.0.join("source.cue")).unwrap();
    }
    if interrupted {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args(["--exact", "interrupted_fixture_child", "--nocapture"])
            .env("TAGREX_U2_FRESH_FIXTURE", &f.0)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if copy {
            c.env("TAGREX_U2_FRESH_COPY", "1");
        } else {
            c.env_remove("TAGREX_U2_FRESH_COPY");
        }
        if link {
            c.env("TAGREX_U2_FRESH_LINK", "1");
        } else {
            c.env_remove("TAGREX_U2_FRESH_LINK");
        }
        let mut c = c.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = c.try_wait().unwrap() {
                assert_eq!(status.code(), Some(86));
                break;
            }
            if Instant::now() > deadline {
                c.kill().unwrap();
                c.wait().unwrap();
                panic!("owned fixture child timeout");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    } else {
        let mut j = SqliteJournal::open(&f.db()).unwrap();
        Executor::apply(&plan(&f.0, link, copy), &mut j, std::slice::from_ref(&f.0)).unwrap();
    }
    let j = SqliteJournal::open(&f.db()).unwrap();
    let b = j.batches().unwrap().remove(0);
    let s = j.move_stages(b.id).unwrap();
    (f, b.id, s)
}
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    dev: u64,
    ino: u64,
    mode: u32,
    bytes: Vec<u8>,
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn visit(root: &Path, p: &Path, out: &mut BTreeMap<PathBuf, Entry>) {
        for e in std::fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("history.sqlite")
            {
                continue;
            }
            let m = std::fs::symlink_metadata(&p).unwrap();
            let bytes = if m.is_file() {
                std::fs::read(&p).unwrap()
            } else if m.file_type().is_symlink() {
                use std::os::unix::ffi::OsStrExt;
                std::fs::read_link(&p)
                    .unwrap()
                    .as_os_str()
                    .as_bytes()
                    .to_vec()
            } else {
                vec![]
            };
            out.insert(
                p.strip_prefix(root).unwrap().to_path_buf(),
                Entry {
                    dev: m.dev(),
                    ino: m.ino(),
                    mode: m.mode(),
                    bytes,
                },
            );
            if m.is_dir() {
                visit(root, &p, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn refuse_unchanged(f: &Fixture, id: BatchId) {
    let before = tree(&f.0);
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let saved_batch = format!("{:?}", j.batch(id).unwrap());
    let saved_stages = j.move_stages(id).unwrap();
    assert!(
        j.batch(id).unwrap().is_some(),
        "history must remain readable"
    );
    assert!(Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).is_err());
    assert_eq!(
        tree(&f.0),
        before,
        "invalid collection caused filesystem effects"
    );
    assert_eq!(format!("{:?}", j.batch(id).unwrap()), saved_batch);
    assert_eq!(j.move_stages(id).unwrap(), saved_stages);
}
fn alter(f: &Fixture, id: BatchId, s: &MoveStage, identity: &[u8], directory: bool) {
    let sql = rusqlite::Connection::open(f.db()).unwrap();
    if directory {
        sql.execute(
            "UPDATE move_stages SET directory_identity=?1 WHERE batch_id=?2 AND path=?3",
            rusqlite::params![identity, id.0, s.path.to_str().unwrap()],
        )
        .unwrap();
    } else {
        sql.execute(
            "UPDATE copy_stamps SET identity=?1 WHERE batch_id=?2 AND path=?3",
            rusqlite::params![identity, id.0, s.target.to_str().unwrap()],
        )
        .unwrap();
        sql.execute(
            "UPDATE move_stages SET identity=?1 WHERE batch_id=?2 AND path=?3",
            rusqlite::params![identity, id.0, s.path.to_str().unwrap()],
        )
        .unwrap();
    }
}
#[test]
fn invalid_typed_audio_proofs_refuse_every_position_before_any_effect() {
    let mut cases = 0;
    for row in 0..3 {
        for mutation in 0..13 {
            let (f, id, s) = seed(false, true);
            let mut proof = s[row].identity.clone();
            assert_eq!(&proof[..6], b"TRX-U2");
            match mutation {
                0 => proof[38] = 1,
                1 => proof[38] = 2,
                2 => proof[38] = 3,
                3 => proof[38] = 255,
                4 => proof[5] = b'1',
                5 => proof[5] = b'3',
                6 => {
                    proof.pop();
                }
                7 => proof.push(0),
                8 => proof[14..22].fill(0),
                9 => proof[22..38].fill(255),
                10 => proof.clear(),
                11 => {
                    let mut windows = b"TRX-W2".to_vec();
                    windows.extend(&proof[6..14]);
                    windows.extend([1u8; 16]);
                    windows.extend(&proof[22..38]);
                    windows.push(0);
                    proof = windows;
                }
                12 => {
                    proof[5] = b'1';
                    proof[38] = 1;
                }
                _ => unreachable!(),
            }
            // -1ns birth is representable, so use i128::MAX rather than an arbitrary valid old date.
            if mutation == 9 {
                proof[22..38].copy_from_slice(&i128::MAX.to_le_bytes());
            }
            alter(&f, id, &s[row], &proof, false);
            refuse_unchanged(&f, id);
            cases += 1;
        }
    }
    eprintln!("typed audio proof refusal cases={cases}");
}
#[test]
fn invalid_directory_seals_refuse_empty_completed_and_occupied_pending_reservations() {
    let mut cases = 0;
    for row in 0..3 {
        for mutation in 0..8 {
            // A well-formed, wrong native seal cannot be checked after its directory is gone.
            // Test that mismatch against the occupied pending reservation; malformed roles
            // remain invalid even for already-cleaned completed reservations.
            if mutation == 5 && row < 2 {
                continue;
            }
            let (f, id, s) = seed(false, true);
            let mut proof = s[row].directory_identity.clone();
            assert_eq!(proof[38], 2);
            match mutation {
                0 => proof[38] = 0,
                1 => proof[38] = 1,
                2 => proof[38] = 255,
                3 => {
                    proof[5] = b'1';
                    proof[38] = 0;
                }
                4 => {
                    proof[5] = b'1';
                    proof[38] = 1;
                }
                5 => {
                    proof = s[row].identity.clone();
                    proof[38] = 2;
                }
                6 => proof[14..22].fill(0),
                7 => proof[22..38].copy_from_slice(&i128::MAX.to_le_bytes()),
                _ => unreachable!(),
            }
            alter(&f, id, &s[row], &proof, true);
            refuse_unchanged(&f, id);
            cases += 1;
        }
    }
    eprintln!("directory seal refusal cases={cases}");
}
#[test]
fn u1_nonlink_proofs_do_not_gain_authority_when_stage_collection_is_empty() {
    for row in 0..3 {
        let (f, id, s) = seed(false, false);
        assert_eq!(s.len(), 3);
        let mut proof = s[row].identity.clone();
        proof[5] = b'1';
        proof[38] = 0;
        alter(&f, id, &s[row], &proof, false);
        let sql = rusqlite::Connection::open(f.db()).unwrap();
        sql.execute("DELETE FROM move_stages WHERE batch_id=?1", [id.0])
            .unwrap();
        drop(sql);
        refuse_unchanged(&f, id);
    }
}
#[test]
fn swapped_genuine_file_and_directory_proofs_refuse_without_any_recovery_effect() {
    for row in 0..3 {
        let (f, id, s) = seed(false, true);
        alter(&f, id, &s[row], &s[row].directory_identity, false);
        refuse_unchanged(&f, id);
    }
}
#[test]
fn explicit_u1_and_u2_link_entries_survive_missing_referent_and_mixed_copy_restart() {
    for interrupted in [false, true] {
        for legacy in [false, true] {
            let (f, id, s) = seed(true, interrupted);
            if legacy {
                let stage = s
                    .iter()
                    .find(|s| s.target.extension().is_some_and(|e| e == "cue"))
                    .unwrap();
                let mut proof = stage.identity.clone();
                assert_eq!(proof[38], 1);
                proof[5] = b'1';
                alter(&f, id, stage, &proof, false);
            }
            // A sidecar link is an entry, even when it no longer resolves.
            std::fs::remove_file(f.0.join("referent")).unwrap();
            let hard = f.0.join("independent.wav");
            std::fs::hard_link(f.0.join("target-0.wav"), &hard).unwrap();
            std::fs::write(&hard, wav(9)).unwrap();
            let mut j = SqliteJournal::open(&f.db()).unwrap();
            Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).unwrap();
            assert_eq!(
                std::fs::read_link(f.0.join("source.cue")).unwrap(),
                Path::new("referent")
            );
            assert!(std::fs::symlink_metadata(f.0.join("target.cue")).is_err());
            assert_eq!(std::fs::read(f.0.join("source-0.wav")).unwrap(), wav(9));
            assert_eq!(std::fs::read(&hard).unwrap(), wav(9));
            assert_eq!(
                std::fs::metadata(f.0.join("source-0.wav")).unwrap().ino(),
                std::fs::metadata(hard).unwrap().ino()
            );
            for n in 1..3 {
                assert_eq!(
                    std::fs::read(f.0.join(format!("source-{n}.wav"))).unwrap(),
                    wav(n)
                );
            }
            for n in 0..3 {
                assert!(!f.0.join(format!("target-{n}.wav")).exists());
            }
            assert_eq!(
                std::fs::read(f.0.join("foreign-sibling")).unwrap(),
                b"must remain byte-identical"
            );
            assert!(j.batches().unwrap().is_empty());
            for stage in s {
                assert!(!stage.path.parent().unwrap().exists());
            }
        }
    }
}

#[test]
fn interrupted_unfinished_copy_refuses_deletion_and_keeps_all_payloads_and_history() {
    let (f, id, stages) = seed_with_copy(true, true, true);
    // Preserve this legacy unknown-proof canary explicitly. Modern executors
    // checkpoint the completed copy before the later sidecar capture/exit.
    let conn = rusqlite::Connection::open(f.db()).unwrap();
    conn.execute("DELETE FROM copy_stamps WHERE moved=0", [])
        .unwrap();
    drop(conn);
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let batch = j.batch(id).unwrap().unwrap();
    assert!(batch.interrupted);
    assert!(!batch
        .copy_stamps
        .iter()
        .any(|s| s.path == f.0.join("target-2.wav")));
    assert!(Executor::undo(&mut j, id, std::slice::from_ref(&f.0)).is_err());
    // Pending move capture is restored; an unproved copy remains for inspection.
    for n in 0..2 {
        assert_eq!(
            std::fs::read(f.0.join(format!("target-{n}.wav"))).unwrap(),
            wav(n)
        );
    }
    assert_eq!(std::fs::read(f.0.join("source-2.wav")).unwrap(), wav(2));
    assert_eq!(std::fs::read(f.0.join("target-2.wav")).unwrap(), wav(2));
    assert_eq!(
        std::fs::read_link(f.0.join("source.cue")).unwrap(),
        Path::new("referent")
    );
    assert_eq!(
        std::fs::read(f.0.join("referent")).unwrap(),
        b"never mutate referent"
    );
    assert_eq!(
        std::fs::read(f.0.join("foreign-sibling")).unwrap(),
        b"must remain byte-identical"
    );
    assert!(j.batch(id).unwrap().is_some());
    for s in stages {
        assert!(!s.path.exists());
    }
}
