#![cfg(target_os = "linux")]
//! Independent public-Executor probes; raw FLAC inspection does not use TagEngine.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};
use tagrex_core::{
    journal::{SqliteJournal, UndoJournal},
    model::TagField,
    plan::{ChangePlan, Executor, FieldChange, FileChange, UndoMode},
};
static CASE: Mutex<()> = Mutex::new(());
struct Fixture(PathBuf, MutexGuard<'static, ()>);
impl Fixture {
    fn new() -> Self {
        let guard = CASE.lock().unwrap_or_else(|e| e.into_inner());
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "tagrex-native-flac-independent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p, guard)
    }
    fn file(&self, name: &str) -> PathBuf {
        let p = self.0.join(name);
        fs::write(
            &p,
            flac(&[
                ("TITLE", "Before"),
                ("ARTIST", "A; B"),
                ("CUSTOM", ""),
                ("CUSTOM", "尾"),
            ]),
        )
        .unwrap();
        p
    }
    fn journal(&self) -> SqliteJournal {
        SqliteJournal::open(&self.0.join("history.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = &self.1;
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn flac(items: &[(&str, &str)]) -> Vec<u8> {
    let mut b = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    b.extend(info);
    let mut c = 3u32.to_le_bytes().to_vec();
    c.extend(b"own");
    c.extend((items.len() as u32).to_le_bytes());
    for (k, v) in items {
        let s = format!("{k}={v}");
        c.extend((s.len() as u32).to_le_bytes());
        c.extend(s.as_bytes());
    }
    let n = c.len();
    b.extend([4, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    b.extend(c);
    b.extend([0x82, 0, 0, 8]);
    b.extend(b"OWN!\0\xff\x01\x02");
    b.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    b
}
fn tags(path: &Path) -> Vec<(String, String)> {
    let b = fs::read(path).unwrap();
    let mut at = 4;
    loop {
        let flag = b[at];
        let n = ((b[at + 1] as usize) << 16) | ((b[at + 2] as usize) << 8) | b[at + 3] as usize;
        at += 4;
        let body = &b[at..at + n];
        at += n;
        if flag & 127 == 4 {
            let u = |i| u32::from_le_bytes(body[i..i + 4].try_into().unwrap()) as usize;
            let mut p = 4 + u(0);
            let count = u(p);
            p += 4;
            let mut out = Vec::new();
            for _ in 0..count {
                let n = u(p);
                p += 4;
                let (k, v) = std::str::from_utf8(&body[p..p + n])
                    .unwrap()
                    .split_once('=')
                    .unwrap();
                out.push((k.into(), v.into()));
                p += n;
            }
            return out;
        }
        assert_eq!(flag & 128, 0, "missing comments");
    }
}
fn media_parts(path: &Path) -> (Vec<u8>, Vec<u8>) {
    let bytes = fs::read(path).unwrap();
    let mut at = 4;
    let mut opaque = Vec::new();
    loop {
        let flag = bytes[at];
        let length = ((bytes[at + 1] as usize) << 16)
            | ((bytes[at + 2] as usize) << 8)
            | bytes[at + 3] as usize;
        at += 4;
        if flag & 127 == 2 {
            opaque.extend_from_slice(&bytes[at..at + length]);
        }
        at += length;
        if flag & 128 != 0 {
            return (opaque, bytes[at..].to_vec());
        }
    }
}
fn set(path: &Path, key: &str, values: &[&str]) {
    let mut all = tags(path);
    all.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
    all.extend(values.iter().map(|v| (key.to_string(), v.to_string())));
    let refs: Vec<_> = all.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    fs::write(path, flac(&refs)).unwrap();
}
fn change(path: &Path, new: &str) -> FileChange {
    FileChange {
        path: path.into(),
        tag_changes: vec![FieldChange {
            field: TagField::Title,
            old: Some("Before".into()),
            new: Some(new.into()),
        }],
        ..Default::default()
    }
}
#[test]
fn duplicate_native_inode_aliases_refuse_before_publication_or_history() {
    let f = Fixture::new();
    let a = f.file("first.flac");
    let b = f.0.join("alias.flac");
    fs::hard_link(&a, &b).unwrap();
    let before = fs::read(&a).unwrap();
    let mut j = f.journal();
    let p = ChangePlan {
        changes: vec![change(&a, "First desired"), change(&b, "Second desired")],
        ..Default::default()
    };
    assert!(Executor::apply(&p, &mut j, std::slice::from_ref(&f.0)).is_err());
    assert_eq!(
        fs::read(&a).unwrap(),
        before,
        "known duplicate native inode must refuse before editing its first alias"
    );
    assert_eq!(fs::read(&b).unwrap(), before);
    assert!(j.batches().unwrap().is_empty());
}
#[test]
fn failed_apply_rolls_back_owned_earlier_file_when_unreached_target_was_edited() {
    let f = Fixture::new();
    let a = f.file("first.flac");
    let b = f.file("second.flac");
    let before = tags(&a);
    let mut j = f.journal();
    let cancel = AtomicBool::new(false);
    let mut external = None;
    let p = ChangePlan {
        changes: vec![change(&a, "First desired"), change(&b, "Second desired")],
        ..Default::default()
    };
    let result = Executor::apply_with(
        &p,
        &mut j,
        std::slice::from_ref(&f.0),
        &mut |e| {
            if e.completed.is_some() && external.is_none() {
                set(&b, "TITLE", &["External later"]);
                external = Some(fs::read(&b).unwrap());
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(
        matches!(
            result,
            Err(tagrex_core::plan::PlanError::RollbackIncomplete { .. })
        ),
        "{result:?}"
    );
    assert!(external.is_some());
    assert_eq!(
        tags(&a),
        before,
        "an unreached external field must not prevent restoring an earlier owned write"
    );
    assert_eq!(fs::read(&b).unwrap(), external.unwrap());
    assert_eq!(
        j.batches().unwrap().len(),
        1,
        "incomplete external recovery must retain history"
    );
}

#[test]
fn unrelated_changes_through_a_hardlink_survive_composed_reopened_undo() {
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        use std::os::unix::fs::MetadataExt;
        let f = Fixture::new();
        let a = f.file("original.flac");
        let alias = f.0.join("outside-plan.flac");
        fs::hard_link(&a, &alias).unwrap();
        let ino = fs::metadata(&a).unwrap().ino();
        let initial_media = media_parts(&a);
        let mut j = f.journal();
        let p = ChangePlan {
            changes: vec![FileChange {
                path: a.clone(),
                tag_changes: vec![
                    FieldChange {
                        field: TagField::Title,
                        old: Some("Before".into()),
                        new: Some("New 🎵".into()),
                    },
                    FieldChange {
                        field: TagField::Artist,
                        old: Some("A; B".into()),
                        new: Some("One; Two".into()),
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let b = Executor::apply(&p, &mut j, std::slice::from_ref(&f.0)).unwrap();
        assert!(b.native_tags);
        assert_eq!(media_parts(&a), initial_media);
        assert_eq!(fs::metadata(&a).unwrap().ino(), ino);
        set(&alias, "EXTERNAL", &["repeat", "repeat", ""]);
        drop(j);
        let mut j = f.journal();
        let report =
            Executor::undo_with_mode(&mut j, b.id, std::slice::from_ref(&f.0), mode, &mut |_| {})
                .unwrap();
        assert!(report.kept.is_empty());
        let all = tags(&a);
        assert!(all.contains(&("TITLE".into(), "Before".into())));
        assert!(all.contains(&("ARTIST".into(), "A; B".into())));
        assert_eq!(
            all.iter()
                .filter(|(k, _)| k == "EXTERNAL")
                .map(|(_, v)| v.as_str())
                .collect::<Vec<_>>(),
            ["repeat", "repeat", ""]
        );
        assert_eq!(fs::read(&a).unwrap(), fs::read(&alias).unwrap());
        assert_eq!(media_parts(&a), initial_media);
        assert_eq!(fs::metadata(&a).unwrap().ino(), ino);
        assert!(j.batches().unwrap().is_empty());
    }
}
#[test]
fn last_preflight_policy_change_refuses_before_any_native_intent() {
    let f = Fixture::new();
    let a = f.file("first.flac");
    let b = f.file("second.flac");
    let before_a = fs::read(&a).unwrap();
    let before_b = fs::read(&b).unwrap();
    let mut j = f.journal();
    let p = ChangePlan {
        changes: vec![change(&a, "New"), change(&b, "New")],
        ..Default::default()
    };
    let old = tagrex_core::model::multi_value_separator();
    let mut changed = false;
    let result = Executor::apply_with(
        &p,
        &mut j,
        std::slice::from_ref(&f.0),
        &mut |e| {
            if !changed && e.path == b {
                tagrex_core::model::set_multi_value_separator(" ↔ ");
                changed = true;
            }
        },
        &AtomicBool::new(false),
    );
    tagrex_core::model::set_multi_value_separator(&old);
    assert!(changed);
    assert!(result.is_err());
    assert_eq!(fs::read(&a).unwrap(), before_a);
    assert_eq!(fs::read(&b).unwrap(), before_b);
    assert!(j.batches().unwrap().is_empty());
    assert!(!f.0.join("history.sqlite.native-media").exists());
}
#[test]
fn callback_panic_after_first_publication_rolls_back_and_releases_for_retry() {
    let f = Fixture::new();
    let a = f.file("first.flac");
    let b = f.file("second.flac");
    let before_a = tags(&a);
    let before_b = tags(&b);
    let mut j = f.journal();
    let p = ChangePlan {
        changes: vec![change(&a, "New"), change(&b, "New")],
        ..Default::default()
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Executor::apply_with(
            &p,
            &mut j,
            std::slice::from_ref(&f.0),
            &mut |e| {
                if e.completed.is_some() {
                    panic!("owned completion callback panic");
                }
            },
            &AtomicBool::new(false),
        )
    }));
    assert!(result.is_err());
    assert_eq!(tags(&a), before_a);
    assert_eq!(tags(&b), before_b);
    assert!(j.batches().unwrap().is_empty());
    let applied = Executor::apply(&p, &mut j, std::slice::from_ref(&f.0)).unwrap();
    Executor::undo(&mut j, applied.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(tags(&a), before_a);
    assert_eq!(tags(&b), before_b);
}
#[test]
fn byte_identical_later_replacement_refuses_whole_undo_in_every_mode() {
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        let f = Fixture::new();
        let a = f.file("first.flac");
        let b = f.file("second.flac");
        let mut j = f.journal();
        let p = ChangePlan {
            changes: vec![change(&a, "New"), change(&b, "New")],
            ..Default::default()
        };
        let batch = Executor::apply(&p, &mut j, std::slice::from_ref(&f.0)).unwrap();
        let after_a = fs::read(&a).unwrap();
        let after_b = fs::read(&b).unwrap();
        fs::rename(&b, f.0.join("saved-object.flac")).unwrap();
        fs::write(&b, &after_b).unwrap();
        drop(j);
        let mut j = f.journal();
        let mut calls = 0;
        assert!(Executor::undo_with_mode(
            &mut j,
            batch.id,
            std::slice::from_ref(&f.0),
            mode,
            &mut |_| calls += 1
        )
        .is_err());
        assert_eq!(calls, 0);
        assert_eq!(fs::read(&a).unwrap(), after_a);
        assert_eq!(fs::read(&b).unwrap(), after_b);
        assert!(j.batch(batch.id).unwrap().is_some());
    }
}
#[test]
fn late_external_edit_is_not_overwritten_by_undo_rollback() {
    let f = Fixture::new();
    let a = f.file("first.flac");
    let b = f.file("second.flac");
    let mut j = f.journal();
    let p = ChangePlan {
        changes: vec![change(&a, "New"), change(&b, "New")],
        ..Default::default()
    };
    let batch = Executor::apply(&p, &mut j, std::slice::from_ref(&f.0)).unwrap();
    let mut external = None;
    let result = Executor::undo_with_mode(
        &mut j,
        batch.id,
        std::slice::from_ref(&f.0),
        UndoMode::Strict,
        &mut |e| {
            if e.path == b && external.is_none() {
                set(&a, "TITLE", &["Changed after first Undo"]);
                external = Some(fs::read(&a).unwrap());
                fs::rename(&b, f.0.join("second-saved.flac")).unwrap();
                fs::write(&b, flac(&[("TITLE", "Foreign")])).unwrap();
            }
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(&a).unwrap(), external.unwrap());
    assert!(j.batch(batch.id).unwrap().is_some());
    assert!(tags(&b).contains(&("TITLE".into(), "Foreign".into())));
}

use tagrex_core::{
    journal::{
        native::{Entries, Intent, NativeJournal},
        AppliedBatch, BatchId, JournalError,
    },
    media_mutation::{Record, Store},
};
struct BeginFailure {
    inner: SqliteJournal,
    fail_remove: bool,
    fail_begin: bool,
}
impl UndoJournal for BeginFailure {
    fn native_tags(&mut self) -> Option<&mut dyn NativeJournal> {
        Some(self)
    }
    fn record(&mut self, b: &AppliedBatch) -> Result<BatchId, JournalError> {
        self.inner.record(b)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        self.inner.batches()
    }
    fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
        self.inner.batch(id)
    }
    fn finish(&mut self, id: BatchId, b: &AppliedBatch) -> Result<(), JournalError> {
        self.inner.finish(id, b)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.inner.rollback(id)
    }
}
impl NativeJournal for BeginFailure {
    fn store(&mut self) -> Result<Store, JournalError> {
        NativeJournal::store(&mut self.inner)
    }
    fn record_native(&mut self, b: &AppliedBatch, i: &[Intent]) -> Result<BatchId, JournalError> {
        NativeJournal::record_native(&mut self.inner, b, i)
    }
    fn entries(&self, b: &AppliedBatch) -> Result<Entries, JournalError> {
        NativeJournal::entries(&self.inner, b)
    }
    fn begin(&mut self, b: &AppliedBatch, i: usize, o: u8) -> Result<String, JournalError> {
        let id = NativeJournal::begin(&mut self.inner, b, i, o)?;
        if o == 2 && self.fail_begin {
            self.fail_begin = false;
            return Err(JournalError::Storage(
                "owned error after durable rollback begin".into(),
            ));
        }
        Ok(id)
    }
    fn checkpoint(
        &mut self,
        b: &AppliedBatch,
        p: (usize, u8),
        r: &Record,
        e: &Intent,
        receipts: Option<&[u8]>,
        done: bool,
    ) -> Result<(), JournalError> {
        NativeJournal::checkpoint(&mut self.inner, b, p, r, e, receipts, done)
    }
    fn confirm(
        &self,
        b: &AppliedBatch,
        p: (usize, u8),
        r: &Record,
        e: &Intent,
        receipts: &[u8],
    ) -> Result<(), JournalError> {
        NativeJournal::confirm(&self.inner, b, p, r, e, receipts)
    }
    fn finish_native(&mut self, b: &AppliedBatch) -> Result<(), JournalError> {
        NativeJournal::finish_native(&mut self.inner, b)
    }
    fn retain_native(&mut self, b: &AppliedBatch) -> Result<(), JournalError> {
        NativeJournal::retain_native(&mut self.inner, b)
    }
    fn remove_native(&mut self, id: BatchId) -> Result<(), JournalError> {
        if self.fail_remove {
            self.fail_remove = false;
            return Err(JournalError::Storage(
                "owned error before history removal".into(),
            ));
        }
        NativeJournal::remove_native(&mut self.inner, id)
    }
}
#[test]
fn persisted_rollback_begin_before_prepared_keeps_reopened_undo_usable() {
    let f = Fixture::new();
    let a = f.file("original.flac");
    let p = ChangePlan {
        changes: vec![change(&a, "New")],
        ..Default::default()
    };
    let mut j = BeginFailure {
        inner: f.journal(),
        fail_remove: true,
        fail_begin: true,
    };
    let batch = Executor::apply(&p, &mut j, std::slice::from_ref(&f.0)).unwrap();
    set(&a, "TITLE", &["External before Undo"]);
    assert!(Executor::undo_with_mode(
        &mut j,
        batch.id,
        std::slice::from_ref(&f.0),
        UndoMode::Overwrite,
        &mut |_| {}
    )
    .is_err());
    assert!(
        !j.fail_remove && !j.fail_begin,
        "both intended durable fault boundaries must be reached"
    );
    assert!(j.batch(batch.id).unwrap().is_some());
    drop(j);
    let mut j = f.journal();
    Executor::undo_with_mode(&mut j,batch.id,std::slice::from_ref(&f.0),UndoMode::Overwrite,&mut |_|{}).expect("a durable op2 begin before any new preparation must not make retained op1 evidence unreadable");
    assert!(tags(&a).contains(&("TITLE".into(), "Before".into())));
    assert!(j.batches().unwrap().is_empty());
}

#[test]
fn inaccessible_or_replaced_unreached_sources_do_not_block_owned_rollback() {
    let mut failures = Vec::new();
    for scenario in ["deleted", "identical-replacement", "malformed"] {
        let f = Fixture::new();
        let a = f.file("first.flac");
        let b = f.file("second.flac");
        let before = tags(&a);
        let original_b = fs::read(&b).unwrap();
        let mut j = f.journal();
        let cancel = AtomicBool::new(false);
        let mut fired = false;
        let p = ChangePlan {
            changes: vec![change(&a, "First desired"), change(&b, "Second desired")],
            ..Default::default()
        };
        let result = Executor::apply_with(
            &p,
            &mut j,
            std::slice::from_ref(&f.0),
            &mut |e| {
                if e.completed.is_some() && !fired {
                    match scenario {
                        "deleted" => fs::remove_file(&b).unwrap(),
                        "identical-replacement" => {
                            fs::rename(&b, f.0.join("saved-second.flac")).unwrap();
                            fs::write(&b, &original_b).unwrap();
                        }
                        "malformed" => fs::write(&b, b"unrelated broken bytes").unwrap(),
                        _ => unreachable!(),
                    }
                    fired = true;
                    cancel.store(true, Ordering::SeqCst);
                }
            },
            &cancel,
        );
        assert!(fired);
        assert!(result.is_err());
        if tags(&a) != before {
            failures.push(format!(
                "scenario={scenario} result={result:?}: earlier owned write was not restored"
            ));
        }
        assert_eq!(j.batches().unwrap().len(), 1);
        match scenario {
            "deleted" => assert!(!b.exists()),
            "identical-replacement" => assert_eq!(fs::read(&b).unwrap(), original_b),
            "malformed" => assert_eq!(fs::read(&b).unwrap(), b"unrelated broken bytes"),
            _ => unreachable!(),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn strict_undo_rechecks_a_skipped_target_after_its_public_callback() {
    let f = Fixture::new();
    let a = f.file("original.flac");
    let mut j = f.journal();
    let p = ChangePlan {
        changes: vec![change(&a, "Applied")],
        ..Default::default()
    };
    let batch = Executor::apply(&p, &mut j, std::slice::from_ref(&f.0)).unwrap();
    // A prior retry/external restoration makes this target a no-op at preflight.
    set(&a, "TITLE", &["Before"]);
    drop(j);
    let mut j = f.journal();
    let mut external = None;
    let result = Executor::undo_with_mode(
        &mut j,
        batch.id,
        std::slice::from_ref(&f.0),
        UndoMode::Strict,
        &mut |_| {
            if external.is_none() {
                set(&a, "TITLE", &["Late external"]);
                external = Some(fs::read(&a).unwrap());
            }
        },
    );
    assert!(external.is_some());
    assert_eq!(fs::read(&a).unwrap(), external.unwrap());
    let retained = j.batch(batch.id).unwrap().is_some();
    assert!(
        result.is_err() && retained,
        "Strict accepted stale no-op selection: result={result:?}, history_retained={retained}"
    );
}
