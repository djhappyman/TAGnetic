//! Portable public-Executor policy probes. VecJournal deliberately selects the
//! legacy FLAC path even on Linux; fixtures and raw readback own no live media.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tagrex_core::{
    journal::{AppliedBatch, BatchId, JournalError, UndoJournal, VecJournal},
    model::{read_policy_revision, ModelPolicy, TagField},
    plan::{ChangePlan, Executor, FieldChange, FileChange},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-portable-policy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str) -> PathBuf {
        let mut bytes = b"fLaC\0\0\0\x22".to_vec();
        let mut info = [0; 34];
        info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
        info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
        bytes.extend(info);
        let items = ["TITLE=Before", "ARTIST=One", "ARTIST=Two", "CUSTOM=尾"];
        let mut comments = 3u32.to_le_bytes().to_vec();
        comments.extend(b"own");
        comments.extend((items.len() as u32).to_le_bytes());
        for item in items {
            comments.extend((item.len() as u32).to_le_bytes());
            comments.extend(item.as_bytes());
        }
        let n = comments.len();
        bytes.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
        bytes.extend(comments);
        bytes.extend([
            0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
        ]);
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Raw {
    comments: BTreeMap<String, Vec<String>>,
    audio: Vec<u8>,
}
fn raw(path: &Path) -> Raw {
    let bytes = fs::read(path).unwrap();
    let mut at = 4;
    let mut comments = BTreeMap::<String, Vec<String>>::new();
    loop {
        let flag = bytes[at];
        let len = ((bytes[at + 1] as usize) << 16)
            | ((bytes[at + 2] as usize) << 8)
            | bytes[at + 3] as usize;
        at += 4;
        let body = &bytes[at..at + len];
        at += len;
        if flag & 127 == 4 {
            let u = |p| u32::from_le_bytes(body[p..p + 4].try_into().unwrap()) as usize;
            let mut p = 4 + u(0);
            let count = u(p);
            p += 4;
            for _ in 0..count {
                let n = u(p);
                p += 4;
                let (key, value) = std::str::from_utf8(&body[p..p + n])
                    .unwrap()
                    .split_once('=')
                    .unwrap();
                p += n;
                comments
                    .entry(key.to_uppercase())
                    .or_default()
                    .push(value.into());
            }
        }
        if flag & 128 != 0 {
            return Raw {
                comments,
                audio: bytes[at..].into(),
            };
        }
    }
}
fn plan(paths: &[&Path]) -> ChangePlan {
    ChangePlan {
        changes: paths
            .iter()
            .map(|path| FileChange {
                path: path.to_path_buf(),
                tag_changes: vec![FieldChange {
                    field: TagField::Title,
                    old: Some("Before".into()),
                    new: Some("After A".into()),
                }],
                ..FileChange::default()
            })
            .collect(),
        ..ChangePlan::default()
    }
}

#[test]
fn owned_change_before_first_legacy_effect_refuses_without_media_or_history_changes() {
    for aba in [false, true] {
        let f = Fixture::new();
        let path = f.file("a.flac");
        let before = fs::read(&path).unwrap();
        let policy = ModelPolicy::default();
        let scope = policy.enter();
        let mut journal = VecJournal::new();
        let mut changed = false;
        let result = Executor::apply_with(
            &plan(&[&path]),
            &mut journal,
            std::slice::from_ref(&f.0),
            &mut |_| {
                if !changed {
                    changed = true;
                    policy.configure(false, &[], " | ", "");
                    if aba {
                        policy.configure(false, &[], "; ", "");
                    }
                }
            },
            &AtomicBool::new(false),
        );
        assert!(
            result.is_err(),
            "legacy path accepted changed owner, ABA={aba}: {result:?}"
        );
        assert_eq!(read_policy_revision(), u64::MAX);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(journal.batches().unwrap().is_empty());
        drop(scope);
        let _fresh = policy.enter();
        let batch =
            Executor::apply(&plan(&[&path]), &mut journal, std::slice::from_ref(&f.0)).unwrap();
        Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
        assert_eq!(raw(&path).comments["TITLE"], ["Before"]);
    }
}

#[test]
fn isolated_global_setter_before_first_legacy_effect_refuses() {
    const CHILD: &str = "TAGREX_PORTABLE_POLICY_GLOBAL_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "isolated_global_setter_before_first_legacy_effect_refuses",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(result.success());
        return;
    }
    let f = Fixture::new();
    let path = f.file("a.flac");
    let before = fs::read(&path).unwrap();
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let mut journal = VecJournal::new();
    let result = Executor::apply_with(
        &plan(&[&path]),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |_| tagrex_core::model::set_multi_value_separator(" | "),
        &AtomicBool::new(false),
    );
    assert!(
        result.is_err(),
        "legacy accepted standalone setter: {result:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn change_after_completed_legacy_prefix_rolls_back_with_or_without_cancel() {
    for cancellation in [false, true] {
        let f = Fixture::new();
        let a = f.file("a.flac");
        let b = f.file("b.flac");
        let before_a = raw(&a);
        let before_b = raw(&b);
        let policy = ModelPolicy::default();
        let _scope = policy.enter();
        let mut journal = VecJournal::new();
        let cancel = AtomicBool::new(false);
        let mut changed = false;
        let result = Executor::apply_with(
            &plan(&[&a, &b]),
            &mut journal,
            std::slice::from_ref(&f.0),
            &mut |event| {
                if event.completed.is_some() && !changed {
                    changed = true;
                    policy.configure(false, &[], " | ", "");
                    if cancellation {
                        cancel.store(true, Ordering::SeqCst);
                    }
                }
            },
            &cancel,
        );
        assert!(changed);
        assert!(
            result.is_err(),
            "legacy continued after completed prefix, cancel={cancellation}: {result:?}"
        );
        assert_eq!(raw(&a), before_a);
        assert_eq!(raw(&b), before_b);
        assert!(journal.batches().unwrap().is_empty());
        assert_eq!(read_policy_revision(), u64::MAX);
    }
}

#[test]
fn final_completion_policy_change_rolls_back_before_success_acknowledgement() {
    let f = Fixture::new();
    let path = f.file("a.flac");
    let before = raw(&path);
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let mut journal = VecJournal::new();
    let mut final_seen = false;
    let result = Executor::apply_with(
        &plan(&[&path]),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed.is_some() && event.done == event.total {
                final_seen = true;
                policy.configure(false, &[], " | ", "");
            }
        },
        &AtomicBool::new(false),
    );
    assert!(final_seen);
    assert!(
        result.is_err(),
        "final callback change was silently committed: {result:?}"
    );
    assert_eq!(raw(&path), before);
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn panic_after_completed_legacy_prefix_restores_owned_values_and_audio() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let b = f.file("b.flac");
    let before_a = raw(&a);
    let before_b = raw(&b);
    let policy = ModelPolicy::default();
    let scope = policy.enter();
    let mut journal = VecJournal::new();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = Executor::apply_with(
            &plan(&[&a, &b]),
            &mut journal,
            std::slice::from_ref(&f.0),
            &mut |event| {
                if event.completed.is_some() {
                    policy.configure(false, &[], " | ", "");
                    panic!("independent legacy progress panic");
                }
            },
            &AtomicBool::new(false),
        );
    }));
    assert!(panic.is_err());
    assert_eq!(raw(&a), before_a);
    assert_eq!(raw(&b), before_b);
    assert!(journal.batches().unwrap().is_empty());
    drop(scope);
    let _fresh = policy.enter();
    assert_ne!(read_policy_revision(), u64::MAX);
}

#[test]
fn legacy_undo_policy_change_before_effect_keeps_applied_media_and_history() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let policy = ModelPolicy::default();
    let scope = policy.enter();
    let mut journal = VecJournal::new();
    let batch = Executor::apply(&plan(&[&a]), &mut journal, std::slice::from_ref(&f.0)).unwrap();
    let applied = raw(&a);
    let result = Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&f.0),
        &mut |_| policy.configure(false, &[], " | ", ""),
    );
    assert!(
        result.is_err(),
        "legacy undo accepted callback policy change: {result:?}"
    );
    assert_eq!(raw(&a), applied);
    assert_eq!(journal.batches().unwrap().len(), 1);
    drop(scope);
    let _fresh = policy.enter();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(raw(&a).comments["TITLE"], ["Before"]);
}

#[test]
fn legacy_undo_after_prefix_stops_and_retains_resumable_history() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let b = f.file("b.flac");
    let before_a = raw(&a);
    let before_b = raw(&b);
    let policy = ModelPolicy::default();
    let scope = policy.enter();
    let mut journal = VecJournal::new();
    let batch =
        Executor::apply(&plan(&[&a, &b]), &mut journal, std::slice::from_ref(&f.0)).unwrap();
    let mut changed = false;
    let result = Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.done > 0 && raw(&a) == before_a && !changed {
                changed = true;
                policy.configure(false, &[], " | ", "");
            }
        },
    );
    assert!(changed);
    assert!(
        result.is_err(),
        "dirty incremental Undo must not report full completion: {result:?}"
    );
    assert_eq!(journal.batches().unwrap().len(), 1);
    assert!(
        raw(&a) == before_a || raw(&b) == before_b,
        "must actually reach a restore"
    );
    assert!(
        raw(&a) != before_a || raw(&b) != before_b,
        "must stop before restoring the other file"
    );
    drop(scope);
    let _fresh = policy.enter();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(raw(&a), before_a);
    assert_eq!(raw(&b), before_b);
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn peer_owner_changes_never_invalidate_a_legacy_operation() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let before = raw(&a);
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let peer = ModelPolicy::default();
    let mut journal = VecJournal::new();
    let batch = Executor::apply_with(
        &plan(&[&a]),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |_| {
            let _peer = peer.enter();
            peer.configure(false, &[], " / ", "windows-1251");
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_ne!(read_policy_revision(), u64::MAX);
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(raw(&a), before);
}

struct CallbackJournal {
    inner: VecJournal,
    policy: ModelPolicy,
    at_finish: bool,
    fired: bool,
}
impl UndoJournal for CallbackJournal {
    fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
        let id = self.inner.record(b)?;
        if !self.at_finish && !self.fired {
            self.fired = true;
            self.policy.configure(false, &[], " | ", "");
        }
        Ok(id)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.inner.batches()
    }
    fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
        self.inner.finish(id, b)?;
        if self.at_finish && !self.fired {
            self.fired = true;
            self.policy.configure(false, &[], " | ", "");
        }
        Ok(())
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.inner.rollback(id)
    }
}

#[test]
fn foreign_journal_callback_policy_changes_cannot_publish_legacy_effects() {
    for at_finish in [false, true] {
        let f = Fixture::new();
        let a = f.file("a.flac");
        let before = raw(&a);
        let policy = ModelPolicy::default();
        let _scope = policy.enter();
        let mut journal = CallbackJournal {
            inner: VecJournal::new(),
            policy: policy.clone(),
            at_finish,
            fired: false,
        };
        let result = Executor::apply(&plan(&[&a]), &mut journal, std::slice::from_ref(&f.0));
        assert!(journal.fired);
        assert!(
            result.is_err(),
            "journal callback changed policy at_finish={at_finish}: {result:?}"
        );
        assert_eq!(raw(&a), before);
        assert!(journal.batches().unwrap().is_empty());
    }
}

#[test]
fn foreign_title_written_after_owned_prefix_is_never_overwritten_by_policy_rollback() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let b = f.file("b.flac");
    let before_b = raw(&b);
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let mut journal = VecJournal::new();
    let cancel = AtomicBool::new(false);
    let mut foreign = None;
    let result = Executor::apply_with(
        &plan(&[&a, &b]),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed.is_some() && foreign.is_none() {
                let mut bytes = fs::read(&a).unwrap();
                let at = bytes
                    .windows(b"TITLE=After A".len())
                    .position(|w| w == b"TITLE=After A")
                    .unwrap();
                bytes[at..at + b"TITLE=Foreign".len()].copy_from_slice(b"TITLE=Foreign");
                fs::write(&a, &bytes).unwrap();
                foreign = Some(bytes);
                policy.configure(false, &[], " | ", "");
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read(&a).unwrap(),
        foreign.expect("must reach first completed write")
    );
    assert_eq!(raw(&b), before_b);
    assert_eq!(
        journal.batches().unwrap().len(),
        1,
        "unrestorable foreign field requires retained recovery history"
    );
}

#[test]
fn owned_prefix_cancellation_restores_raw_metadata_and_audio_under_invalid_owner() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let b = f.file("b.flac");
    let before_a = raw(&a);
    let before_b = raw(&b);
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let mut journal = VecJournal::new();
    let cancel = AtomicBool::new(false);
    let mut changed = false;
    let result = Executor::apply_with(
        &plan(&[&a, &b]),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed.is_some() && !changed {
                changed = true;
                policy.configure(false, &[], " | ", "");
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(changed);
    assert!(result.is_err());
    assert_eq!(raw(&a), before_a);
    assert_eq!(raw(&b), before_b);
    assert!(journal.batches().unwrap().is_empty());
    assert_eq!(read_policy_revision(), u64::MAX);
}

#[test]
fn unrelated_foreign_comment_survives_owned_prefix_policy_rollback() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let b = f.file("b.flac");
    let before_a = raw(&a);
    let before_b = raw(&b);
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let mut journal = VecJournal::new();
    let cancel = AtomicBool::new(false);
    let mut changed = false;
    let result = Executor::apply_with(
        &plan(&[&a, &b]),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed.is_some() && !changed {
                changed = true;
                let mut bytes = fs::read(&a).unwrap();
                let needle = "CUSTOM=尾".as_bytes();
                let at = bytes
                    .windows(needle.len())
                    .position(|w| w == needle)
                    .unwrap();
                bytes[at..at + needle.len()].copy_from_slice("CUSTOM=外".as_bytes());
                fs::write(&a, &bytes).unwrap();
                policy.configure(false, &[], " | ", "");
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(changed);
    assert!(result.is_err());
    let current = raw(&a);
    assert_eq!(current.comments["TITLE"], before_a.comments["TITLE"]);
    assert_eq!(current.comments["CUSTOM"], ["外"]);
    assert_eq!(current.comments["ARTIST"], before_a.comments["ARTIST"]);
    assert_eq!(current.audio, before_a.audio);
    assert_eq!(raw(&b), before_b);
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn invalid_owner_at_entry_refuses_before_legacy_progress_or_journal() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let before = fs::read(&a).unwrap();
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    policy.configure(false, &[], " | ", "");
    let mut journal = VecJournal::new();
    let mut callbacks = 0;
    let result = Executor::apply_with(
        &plan(&[&a]),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |_| callbacks += 1,
        &AtomicBool::new(false),
    );
    assert!(result.is_err());
    assert_eq!(callbacks, 0);
    assert!(journal.batches().unwrap().is_empty());
    assert_eq!(fs::read(&a).unwrap(), before);
}

#[test]
fn undo_journal_load_callback_change_refuses_before_progress_or_restore() {
    use std::cell::Cell;
    struct LoadCallback {
        inner: VecJournal,
        policy: ModelPolicy,
        fired: Cell<bool>,
    }
    impl UndoJournal for LoadCallback {
        fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
            self.inner.record(b)
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            if !self.fired.replace(true) {
                self.policy.configure(false, &[], " | ", "");
            }
            self.inner.batches()
        }
        fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
            self.inner.finish(id, b)
        }
        fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
            self.inner.rollback(id)
        }
    }
    let f = Fixture::new();
    let a = f.file("a.flac");
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let mut inner = VecJournal::new();
    let batch = Executor::apply(&plan(&[&a]), &mut inner, std::slice::from_ref(&f.0)).unwrap();
    let applied = raw(&a);
    let mut journal = LoadCallback {
        inner,
        policy: policy.clone(),
        fired: Cell::new(false),
    };
    let mut callbacks = 0;
    let result = Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&f.0),
        &mut |_| callbacks += 1,
    );
    assert!(journal.fired.get());
    assert!(result.is_err());
    assert_eq!(callbacks, 0);
    assert_eq!(raw(&a), applied);
    assert_eq!(journal.inner.batches().unwrap().len(), 1);
}

#[test]
fn automatic_legacy_rollback_seals_public_executor_reentry_from_journal_callback() {
    use std::cell::Cell;
    struct Trap(Cell<usize>);
    impl UndoJournal for Trap {
        fn record(&mut self, _: &AppliedBatch) -> Result<BatchId, JournalError> {
            self.0.set(self.0.get() + 1);
            Err(JournalError::Storage("unexpected legacy record".into()))
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            self.0.set(self.0.get() + 1);
            Err(JournalError::Storage("unexpected legacy load".into()))
        }
        fn finish(&mut self, _: BatchId, _: &AppliedBatch) -> Result<(), JournalError> {
            self.0.set(self.0.get() + 1);
            Err(JournalError::Storage("unexpected legacy finish".into()))
        }
        fn rollback(&mut self, _: BatchId) -> Result<(), JournalError> {
            self.0.set(self.0.get() + 1);
            Err(JournalError::Storage("unexpected legacy retirement".into()))
        }
    }
    struct Reentrant {
        inner: VecJournal,
        root: PathBuf,
        target: PathBuf,
        probed: bool,
    }
    impl UndoJournal for Reentrant {
        fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
            self.inner.record(b)
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            self.inner.batches()
        }
        fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
            self.inner.finish(id, b)
        }
        fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
            self.inner.rollback(id)
        }
        fn retain_interrupted(&mut self, _: BatchId, _: &AppliedBatch) -> Result<(), JournalError> {
            if !self.probed {
                self.probed = true;
                for peer in [false, true] {
                    let policy = ModelPolicy::default();
                    let _peer = peer.then(|| policy.enter());
                    let mut trap = Trap(Cell::new(0));
                    let mut callbacks = 0;
                    assert!(Executor::apply_with(
                        &plan(&[&self.target]),
                        &mut trap,
                        std::slice::from_ref(&self.root),
                        &mut |_| callbacks += 1,
                        &AtomicBool::new(false)
                    )
                    .is_err());
                    assert!(Executor::undo_with(
                        &mut trap,
                        BatchId(999),
                        std::slice::from_ref(&self.root),
                        &mut |_| callbacks += 1
                    )
                    .is_err());
                    assert_eq!(
                        callbacks, 0,
                        "recovery ancestor must refuse reentrant user work before progress"
                    );
                    assert_eq!(
                        trap.0.get(),
                        0,
                        "recovery ancestor must refuse before legacy journal access"
                    );
                }
            }
            Ok(())
        }
    }
    let f = Fixture::new();
    let a = f.file("a.flac");
    let b = f.file("b.flac");
    let target = f.file("untouched.flac");
    let before_a = raw(&a);
    let before_b = raw(&b);
    let before_target = fs::read(&target).unwrap();
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let mut journal = Reentrant {
        inner: VecJournal::new(),
        root: f.0.clone(),
        target: target.clone(),
        probed: false,
    };
    let cancel = AtomicBool::new(false);
    let result = Executor::apply_with(
        &plan(&[&a, &b]),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed.is_some() {
                policy.configure(false, &[], " | ", "");
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(result.is_err());
    assert!(journal.probed);
    assert_eq!(read_policy_revision(), u64::MAX);
    assert_eq!(raw(&a), before_a);
    assert_eq!(raw(&b), before_b);
    assert_eq!(fs::read(&target).unwrap(), before_target);
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn successful_terminal_history_removal_is_legacy_undo_commit_boundary() {
    struct RetireCallback {
        inner: VecJournal,
        policy: ModelPolicy,
        fired: bool,
    }
    impl UndoJournal for RetireCallback {
        fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
            self.inner.record(b)
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            self.inner.batches()
        }
        fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
            self.inner.finish(id, b)
        }
        fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
            self.inner.rollback(id)?;
            self.fired = true;
            self.policy.configure(false, &[], " | ", "");
            Ok(())
        }
    }
    let f = Fixture::new();
    let a = f.file("a.flac");
    let before = raw(&a);
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let mut inner = VecJournal::new();
    let batch = Executor::apply(&plan(&[&a]), &mut inner, std::slice::from_ref(&f.0)).unwrap();
    let mut journal = RetireCallback {
        inner,
        policy: policy.clone(),
        fired: false,
    };
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).expect("successful terminal deletion is committed; a post-call policy mutation must not masquerade as failed Undo with recoverable history");
    assert!(journal.fired);
    assert_eq!(read_policy_revision(), u64::MAX);
    assert_eq!(raw(&a), before);
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn standalone_executor_pins_multivalue_policy_for_cancelled_owned_prefix() {
    const CHILD: &str = "TAGREX_PORTABLE_STANDALONE_PREFIX_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "standalone_executor_pins_multivalue_policy_for_cancelled_owned_prefix",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(result.success());
        return;
    }
    tagrex_core::model::set_multi_value_separator("");
    let f = Fixture::new();
    let a = f.file("a.flac");
    let b = f.file("b.flac");
    let before_a = raw(&a);
    let before_b = raw(&b);
    let mut journal = VecJournal::new();
    let plan = ChangePlan {
        changes: [&a, &b]
            .into_iter()
            .map(|path| FileChange {
                path: path.clone(),
                tag_changes: vec![FieldChange {
                    field: TagField::Artist,
                    old: Some("One; Two".into()),
                    new: Some("Three; Four".into()),
                }],
                ..FileChange::default()
            })
            .collect(),
        ..ChangePlan::default()
    };
    let cancel = AtomicBool::new(false);
    let mut changed = false;
    let result = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed.is_some() && !changed {
                changed = true;
                tagrex_core::model::set_multi_value_separator(" | ");
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(changed);
    assert!(result.is_err());
    assert_eq!(
        raw(&a),
        before_a,
        "direct Executor rollback must use its admitted interpretation: {result:?}"
    );
    assert_eq!(raw(&b), before_b);
    assert!(journal.batches().unwrap().is_empty());
    assert_eq!(
        tagrex_core::model::multi_value_separator(),
        " | ",
        "operation pin must not discard the standalone configuration update"
    );
}

#[test]
fn sqlite_final_completion_callback_cannot_bypass_common_policy_boundary() {
    let f = Fixture::new();
    let a = f.file("a.flac");
    let before = raw(&a);
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let mut journal =
        tagrex_core::journal::SqliteJournal::open(&f.0.join("history.sqlite")).unwrap();
    let mut changed = false;
    let result = Executor::apply_with(
        &plan(&[&a]),
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed.is_some() && event.done == event.total {
                changed = true;
                policy.configure(false, &[], " | ", "");
            }
        },
        &AtomicBool::new(false),
    );
    assert!(changed);
    assert!(
        result.is_err(),
        "SQLite/native dispatch bypassed final callback validation: {result:?}"
    );
    assert_eq!(raw(&a), before);
    assert!(journal.batches().unwrap().is_empty());
}
