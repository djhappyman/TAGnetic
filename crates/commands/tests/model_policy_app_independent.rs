#![cfg(target_os = "linux")]
//! Public-App probes with independent raw FLAC readback and disposable profiles.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Barrier,
    },
};
use tagrex_commands::{App, SettingsDto, TagEditDto};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-policy-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn track(&self, name: &str) -> PathBuf {
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
        let p = self.0.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }
    fn app(&self) -> App {
        App::open(&self.0, &self.0.join("history.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn settings(separator: &str) -> SettingsDto {
    SettingsDto {
        multi_value_separator: separator.into(),
        ..SettingsDto::default()
    }
}
fn edit(path: &Path, field: &str, value: &str) -> TagEditDto {
    TagEditDto {
        path: path.to_string_lossy().into_owned(),
        field: field.into(),
        value: Some(value.into()),
    }
}
fn raw_comments(path: &Path) -> Vec<String> {
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
                out.push(String::from_utf8(body[p..p + n].to_vec()).unwrap());
                p += n;
            }
            return out;
        }
        assert_eq!(flag & 128, 0);
    }
}

#[test]
fn parallel_apps_keep_divergent_reads_cache_apply_undo_and_redo_isolated() {
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [" | ", " / "]
        .into_iter()
        .map(|separator| {
            let gate = barrier.clone();
            std::thread::spawn(move || {
                let f = Fixture::new();
                let p = f.track("a.flac");
                let original = fs::read(&p).unwrap();
                let mut app = f.app();
                app.apply_settings(&settings(separator));
                gate.wait();
                for i in 0..3 {
                    assert_eq!(
                        app.read_track(&p).unwrap().tags["artist"],
                        format!("One{separator}Two")
                    );
                    let _ = app.query_snapshot();
                    let desired = format!("Three{separator}Four{i}");
                    let plan = app
                        .preview_tag_edits(&[edit(&p, "artist", &desired)])
                        .unwrap();
                    assert_eq!(fs::read(&p).unwrap(), original);
                    let batch = app.apply(&plan).unwrap();
                    assert_eq!(app.read_track(&p).unwrap().tags["artist"], desired);
                    let raw = raw_comments(&p);
                    assert!(raw.contains(&"ARTIST=Three".into()));
                    assert!(raw.contains(&format!("ARTIST=Four{i}")));
                    assert!(raw.contains(&"CUSTOM=尾".into()));
                    app.undo(batch.id).unwrap();
                    app.redo_with(&mut |_| {}).unwrap().unwrap();
                    let latest = app.history().unwrap()[0].id;
                    app.undo(latest).unwrap();
                    assert_eq!(fs::read(&p).unwrap(), original);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn reopened_journal_rebinds_matching_semantics_but_refuses_changed_policy() {
    let f = Fixture::new();
    let p = f.track("a.flac");
    let before = fs::read(&p).unwrap();
    let id = {
        let mut app = f.app();
        app.apply_settings(&settings(" | "));
        let plan = app
            .preview_tag_edits(&[edit(&p, "artist", "Three | Four")])
            .unwrap();
        app.apply(&plan).unwrap().id
    };
    let written = fs::read(&p).unwrap();
    let mut reopened = f.app();
    reopened.apply_settings(&settings(" / "));
    assert!(reopened.undo(id).is_err());
    assert_eq!(fs::read(&p).unwrap(), written);
    assert_eq!(reopened.history().unwrap().len(), 1);
    reopened.apply_settings(&settings(" | "));
    reopened.undo(id).unwrap();
    assert_eq!(fs::read(&p).unwrap(), before);
}

#[test]
fn standalone_policy_change_after_first_write_cancellation_restores_owned_prefix() {
    const CHILD: &str = "TAGREX_INDEPENDENT_POLICY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "standalone_policy_change_after_first_write_cancellation_restores_owned_prefix",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(result.success());
        return;
    }
    let f = Fixture::new();
    let a = f.track("a.flac");
    let b = f.track("b.flac");
    let before_a = fs::read(&a).unwrap();
    let before_b = fs::read(&b).unwrap();
    let mut app = f.app();
    let plan = app
        .preview_tag_edits(&[edit(&a, "title", "After A"), edit(&b, "title", "After B")])
        .unwrap();
    let cancel = AtomicBool::new(false);
    let mut wrote = false;
    let result = app.apply_with(
        &plan,
        &mut |event| {
            if event.completed.is_some() && !wrote {
                wrote = true;
                tagrex_core::model::set_multi_value_separator("changed");
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(wrote, "probe must reach an owned write");
    assert!(result.is_err());
    assert_eq!(
        fs::read(&a).unwrap(),
        before_a,
        "owned prefix should roll back despite invalidated live scope: {result:?}"
    );
    assert_eq!(fs::read(&b).unwrap(), before_b);
    assert!(
        app.history().unwrap().is_empty(),
        "fully rolled back cancellation should retire history"
    );
}

#[test]
fn changed_owned_core_policy_after_first_write_restores_prefix() {
    use tagrex_core::{
        journal::{SqliteJournal, UndoJournal},
        model::{ModelPolicy, TagField},
        plan::{ChangePlan, Executor, FieldChange, FileChange},
    };
    let f = Fixture::new();
    let a = f.track("a.flac");
    let b = f.track("b.flac");
    let before_a = fs::read(&a).unwrap();
    let before_b = fs::read(&b).unwrap();
    let policy = ModelPolicy::default();
    let mut journal = SqliteJournal::open(&f.0.join("history.sqlite")).unwrap();
    let plan = ChangePlan {
        changes: [&a, &b]
            .into_iter()
            .map(|p| FileChange {
                path: p.clone(),
                tag_changes: vec![FieldChange {
                    field: TagField::Title,
                    old: Some("Before".into()),
                    new: Some("After".into()),
                }],
                ..FileChange::default()
            })
            .collect(),
        ..ChangePlan::default()
    };
    let scope = policy.enter();
    let mut wrote = false;
    let cancel = AtomicBool::new(false);
    let result = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed.is_some() && !wrote {
                wrote = true;
                policy.configure(false, &[], "changed", "");
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(wrote);
    assert!(result.is_err());
    assert_eq!(
        fs::read(&a).unwrap(),
        before_a,
        "owned configuration changed during callback: {result:?}"
    );
    assert_eq!(fs::read(&b).unwrap(), before_b);
    assert!(journal.batches().unwrap().is_empty());
    drop(scope);
    let _new = policy.enter();
    assert_eq!(tagrex_core::model::multi_value_separator(), "changed");
}

#[test]
fn own_settings_changed_inside_write_scope_refuse_until_outer_operation_finishes() {
    let f = Fixture::new();
    let p = f.track("a.flac");
    let before = fs::read(&p).unwrap();
    let mut app = f.app();
    app.with_write_scope(std::slice::from_ref(&f.0), |app| {
        app.apply_settings(&settings(" | "));
        let plan = app.preview_tag_edits(&[edit(&p, "title", "After")])?;
        assert!(
            app.apply(&plan).is_err(),
            "changed owner must not silently refresh an active operation"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(fs::read(&p).unwrap(), before);
    assert!(app.history().unwrap().is_empty());
    let plan = app
        .preview_tag_edits(&[edit(&p, "artist", "Three | Four")])
        .unwrap();
    let batch = app.apply(&plan).unwrap();
    assert!(raw_comments(&p).contains(&"ARTIST=Three".into()));
    app.undo(batch.id).unwrap();
    assert_eq!(fs::read(&p).unwrap(), before);
}

#[test]
fn pinned_rollback_policy_never_authorizes_overwriting_external_owned_field_change() {
    use tagrex_core::{
        journal::{SqliteJournal, UndoJournal},
        model::{ModelPolicy, TagField},
        plan::{ChangePlan, Executor, FieldChange, FileChange},
    };
    let f = Fixture::new();
    let a = f.track("a.flac");
    let b = f.track("b.flac");
    let before_b = fs::read(&b).unwrap();
    let policy = ModelPolicy::default();
    let mut journal = SqliteJournal::open(&f.0.join("history.sqlite")).unwrap();
    let plan = ChangePlan {
        changes: [&a, &b]
            .into_iter()
            .map(|p| FileChange {
                path: p.clone(),
                tag_changes: vec![FieldChange {
                    field: TagField::Title,
                    old: Some("Before".into()),
                    new: Some("After".into()),
                }],
                ..FileChange::default()
            })
            .collect(),
        ..ChangePlan::default()
    };
    let _scope = policy.enter();
    let mut external = None;
    let cancel = AtomicBool::new(false);
    let result = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.completed.is_some() && external.is_none() {
                let mut bytes = fs::read(&a).unwrap();
                let at = bytes
                    .windows(b"TITLE=After".len())
                    .position(|window| window == b"TITLE=After")
                    .unwrap();
                bytes[at..at + b"TITLE=Other".len()].copy_from_slice(b"TITLE=Other");
                fs::write(&a, &bytes).unwrap();
                external = Some(bytes);
                policy.configure(false, &[], "changed", "");
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read(&a).unwrap(),
        external.expect("must reach first owned write")
    );
    assert_eq!(fs::read(&b).unwrap(), before_b);
    assert_eq!(
        journal.batches().unwrap().len(),
        1,
        "unresolved external edit must retain recovery history"
    );
}

#[test]
fn standalone_settings_and_new_app_defaults_are_explicitly_separate() {
    const CHILD: &str = "TAGREX_INDEPENDENT_DEFAULTS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "standalone_settings_and_new_app_defaults_are_explicitly_separate",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(result.success());
        return;
    }
    let f = Fixture::new();
    let path = f.track("a.flac");
    tagrex_core::model::set_multi_value_separator(" / ");
    let app = f.app();
    assert_eq!(app.read_track(&path).unwrap().tags["artist"], "One; Two");
    app.apply_settings(&settings(" | "));
    assert_eq!(app.read_track(&path).unwrap().tags["artist"], "One | Two");
    assert_eq!(tagrex_core::model::multi_value_separator(), " / ");
    assert_eq!(
        tagrex_core::model::TagEngine::read(&path).unwrap().tags
            [&tagrex_core::model::TagField::Artist],
        "One / Two"
    );
}

#[test]
fn read_after_same_owner_settings_change_does_not_admit_pinned_old_cache() {
    let f = Fixture::new();
    let p = f.track("a.flac");
    let mut app = f.app();
    app.with_write_scope(std::slice::from_ref(&f.0), |app| {
        app.apply_settings(&settings(" | "));
        // Old operation keeps its interpretation, or may refuse a read. Either
        // way it cannot make that old interpretation a new-operation cache hit.
        let _ = app.read_track(&p);
        Ok(())
    })
    .unwrap();
    assert_eq!(app.read_track(&p).unwrap().tags["artist"], "One | Two");
}

#[test]
fn query_after_same_owner_settings_change_rebuilds_pinned_old_snapshot() {
    let f = Fixture::new();
    let _ = f.track("a.flac");
    let mut app = f.app();
    let previously_published = app.query_snapshot().unwrap();
    assert!(previously_published.is_current());
    let mut pinned = None;
    app.with_write_scope(std::slice::from_ref(&f.0), |app| {
        app.apply_settings(&settings(" | "));
        assert!(!previously_published.is_current());
        let snapshot = app.query_snapshot();
        assert!(
            snapshot.is_err(),
            "invalidated operation must refuse publishing its pinned old view"
        );
        assert!(app.query_snapshot_id().is_none());
        pinned = Some(snapshot);
        Ok(())
    })
    .unwrap();
    assert!(pinned.as_ref().unwrap().is_err());
    let current = app.query_snapshot().unwrap();
    assert!(current.is_current());
    assert!(pinned.unwrap().is_err());
    assert_eq!(current.rows.len(), 1);
    assert_eq!(current.rows[0].value("artist"), "One | Two");
}

#[test]
fn failed_outer_scope_discards_both_old_display_and_query_cache() {
    let f = Fixture::new();
    let p = f.track("a.flac");
    let before = fs::read(&p).unwrap();
    let mut app = f.app();
    let failure: Result<(), _> = app.with_write_scope(std::slice::from_ref(&f.0), |app| {
        app.apply_settings(&settings(" | "));
        let _ = app.read_track(&p);
        let _ = app.query_snapshot();
        Err(std::io::Error::other("independent outer operation failure").into())
    });
    assert!(failure.is_err());
    assert_eq!(app.read_track(&p).unwrap().tags["artist"], "One | Two");
    assert_eq!(
        app.query_snapshot().unwrap().rows[0].value("artist"),
        "One | Two"
    );
    assert_eq!(fs::read(&p).unwrap(), before);
}

#[test]
fn same_owner_settings_change_and_restore_does_not_restore_write_admission() {
    let f = Fixture::new();
    let p = f.track("a.flac");
    let before = fs::read(&p).unwrap();
    let mut app = f.app();
    app.apply_settings(&settings(" | "));
    app.with_write_scope(std::slice::from_ref(&f.0), |app| {
        app.apply_settings(&settings(" / "));
        let _ = app.read_track(&p);
        let _ = app.query_snapshot();
        app.apply_settings(&settings(" | "));
        let _ = app.read_track(&p);
        let _ = app.query_snapshot();
        let plan = app.preview_tag_edits(&[edit(&p, "title", "After")])?;
        assert!(app.apply(&plan).is_err(), "restore of semantic settings must not undo an active operation's revision invalidation");
        Ok(())
    }).unwrap();
    assert_eq!(app.read_track(&p).unwrap().tags["artist"], "One | Two");
    assert_eq!(
        app.query_snapshot().unwrap().rows[0].value("artist"),
        "One | Two"
    );
    assert_eq!(fs::read(&p).unwrap(), before);
    let plan = app
        .preview_tag_edits(&[edit(&p, "title", "After")])
        .unwrap();
    let batch = app.apply(&plan).unwrap();
    app.undo(batch.id).unwrap();
    assert_eq!(fs::read(&p).unwrap(), before);
}

#[test]
fn unwinding_outer_scope_does_not_keep_stale_policy_cache_or_write_scope() {
    let f = Fixture::new();
    let p = f.track("a.flac");
    let before = fs::read(&p).unwrap();
    let mut app = f.app();
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), _> = app.with_write_scope(std::slice::from_ref(&f.0), |app| {
            app.apply_settings(&settings(" | "));
            let _ = app.read_track(&p);
            let _ = app.query_snapshot();
            panic!("independent public scope unwind");
        });
    }));
    assert!(unwind.is_err());
    assert_eq!(app.read_track(&p).unwrap().tags["artist"], "One | Two");
    assert_eq!(
        app.query_snapshot().unwrap().rows[0].value("artist"),
        "One | Two"
    );
    assert_eq!(fs::read(&p).unwrap(), before);
    let plan = app
        .preview_tag_edits(&[edit(&p, "title", "After")])
        .unwrap();
    let batch = app.apply(&plan).unwrap();
    app.undo(batch.id).unwrap();
    assert_eq!(fs::read(&p).unwrap(), before);
}

mod sealed_recovery {
    use super::*;
    use std::{cell::Cell, rc::Rc};
    use tagrex_core::{
        journal::{
            native::{Entries, Intent, NativeJournal},
            AppliedBatch, BatchId, JournalError, SqliteJournal, UndoJournal, VecJournal,
        },
        media_mutation::{Record, Store},
        model::{read_policy_revision, ModelPolicy, TagField},
        plan::{ChangePlan, Executor, FieldChange, FileChange, UndoMode},
    };

    struct Trap;
    impl UndoJournal for Trap {
        fn record(&mut self, _: &AppliedBatch) -> Result<BatchId, JournalError> {
            panic!("sealed reentry reached legacy record")
        }
        fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
            panic!("sealed reentry read legacy journal")
        }
        fn finish(&mut self, _: BatchId, _: &AppliedBatch) -> Result<(), JournalError> {
            panic!("sealed reentry reached legacy finish")
        }
        fn rollback(&mut self, _: BatchId) -> Result<(), JournalError> {
            panic!("sealed reentry reached legacy rollback")
        }
    }

    struct Observed {
        inner: SqliteJournal,
        armed: Rc<Cell<bool>>,
        probed: Cell<bool>,
        root: PathBuf,
        target: PathBuf,
    }
    impl Observed {
        fn probe(&self) {
            assert_ne!(
                read_policy_revision(),
                u64::MAX,
                "automatic recovery must use its admitted snapshot"
            );
            // A numeric field forces the legacy route if a generic entry guard
            // is missing. Neither plan shape nor journal capability is authority.
            let plan = ChangePlan {
                changes: vec![FileChange {
                    path: self.target.clone(),
                    tag_changes: vec![FieldChange {
                        field: TagField::Year,
                        old: None,
                        new: Some("2026".into()),
                    }],
                    ..FileChange::default()
                }],
                ..ChangePlan::default()
            };
            for nested in [false, true] {
                let peer = ModelPolicy::default();
                let _peer = nested.then(|| peer.enter());
                let mut memory = VecJournal::new();
                let mut trap = Trap;
                for journal in [&mut memory as &mut dyn UndoJournal, &mut trap] {
                    let mut callbacks = 0;
                    let error = Executor::apply_with(
                        &plan,
                        journal,
                        std::slice::from_ref(&self.root),
                        &mut |_| callbacks += 1,
                        &AtomicBool::new(false),
                    )
                    .unwrap_err();
                    assert!(
                        error
                            .to_string()
                            .contains("new user operations are refused during exact rollback"),
                        "{error:?}"
                    );
                    let error = Executor::undo_with_mode(
                        journal,
                        BatchId(999),
                        std::slice::from_ref(&self.root),
                        UndoMode::Overwrite,
                        &mut |_| callbacks += 1,
                    )
                    .unwrap_err();
                    assert!(
                        error
                            .to_string()
                            .contains("new user operations are refused during exact rollback"),
                        "{error:?}"
                    );
                    assert_eq!(callbacks, 0);
                }
                assert!(memory.batches().unwrap().is_empty());
            }
        }
    }
    impl UndoJournal for Observed {
        fn native_tags(&mut self) -> Option<&mut dyn NativeJournal> {
            Some(self)
        }
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
    }
    impl NativeJournal for Observed {
        fn store(&mut self) -> Result<Store, JournalError> {
            NativeJournal::store(&mut self.inner)
        }
        fn record_native(
            &mut self,
            b: &AppliedBatch,
            i: &[Intent],
        ) -> Result<BatchId, JournalError> {
            NativeJournal::record_native(&mut self.inner, b, i)
        }
        fn entries(&self, b: &AppliedBatch) -> Result<Entries, JournalError> {
            if self.armed.get() && !self.probed.replace(true) {
                self.probe();
            }
            NativeJournal::entries(&self.inner, b)
        }
        fn begin(&mut self, b: &AppliedBatch, i: usize, o: u8) -> Result<String, JournalError> {
            NativeJournal::begin(&mut self.inner, b, i, o)
        }
        fn checkpoint(
            &mut self,
            b: &AppliedBatch,
            position: (usize, u8),
            r: &Record,
            expected: &Intent,
            receipts: Option<&[u8]>,
            done: bool,
        ) -> Result<(), JournalError> {
            NativeJournal::checkpoint(&mut self.inner, b, position, r, expected, receipts, done)
        }
        fn confirm(
            &self,
            b: &AppliedBatch,
            position: (usize, u8),
            r: &Record,
            expected: &Intent,
            receipts: &[u8],
        ) -> Result<(), JournalError> {
            NativeJournal::confirm(&self.inner, b, position, r, expected, receipts)
        }
        fn finish_native(&mut self, b: &AppliedBatch) -> Result<(), JournalError> {
            NativeJournal::finish_native(&mut self.inner, b)
        }
        fn retain_native(&mut self, b: &AppliedBatch) -> Result<(), JournalError> {
            NativeJournal::retain_native(&mut self.inner, b)
        }
        fn remove_native(&mut self, id: BatchId) -> Result<(), JournalError> {
            NativeJournal::remove_native(&mut self.inner, id)
        }
    }

    #[test]
    fn public_apply_and_undo_refuse_sealed_rollback_even_through_peer_or_legacy_adapter() {
        let f = Fixture::new();
        let a = f.track("a.flac");
        let b = f.track("b.flac");
        let untouched = f.track("untouched.flac");
        let before_a = fs::read(&a).unwrap();
        let before_b = fs::read(&b).unwrap();
        let before_untouched = fs::read(&untouched).unwrap();
        let armed = Rc::new(Cell::new(false));
        let mut journal = Observed {
            inner: SqliteJournal::open(&f.0.join("history.sqlite")).unwrap(),
            armed: armed.clone(),
            probed: Cell::new(false),
            root: f.0.clone(),
            target: untouched.clone(),
        };
        let policy = ModelPolicy::default();
        let _scope = policy.enter();
        let plan = ChangePlan {
            changes: [&a, &b]
                .into_iter()
                .map(|p| FileChange {
                    path: p.clone(),
                    tag_changes: vec![FieldChange {
                        field: TagField::Title,
                        old: Some("Before".into()),
                        new: Some("After".into()),
                    }],
                    ..FileChange::default()
                })
                .collect(),
            ..ChangePlan::default()
        };
        let cancel = AtomicBool::new(false);
        let result = Executor::apply_with(
            &plan,
            &mut journal,
            std::slice::from_ref(&f.0),
            &mut |event| {
                if event.completed.is_some() && !armed.replace(true) {
                    policy.configure(false, &[], "changed", "");
                    cancel.store(true, Ordering::SeqCst);
                }
            },
            &cancel,
        );
        assert!(result.is_err());
        assert!(
            journal.probed.get(),
            "must exercise the actual sealed recovery callback"
        );
        assert_eq!(
            read_policy_revision(),
            u64::MAX,
            "private recovery must not repair the caller's invalidation"
        );
        assert_eq!(fs::read(&a).unwrap(), before_a);
        assert_eq!(fs::read(&b).unwrap(), before_b);
        assert_eq!(fs::read(&untouched).unwrap(), before_untouched);
        assert!(journal.batches().unwrap().is_empty());
    }
}

#[test]
fn query_workers_keep_the_originating_apps_policy_across_shared_views() {
    let first = Fixture::new();
    let second = Fixture::new();
    first.track("first.flac");
    second.track("second.flac");
    let a = first.app();
    let b = second.app();
    a.apply_settings(&settings(" | "));
    b.apply_settings(&settings(" / "));
    let original = a.query_snapshot().unwrap();
    let peer = b.query_snapshot().unwrap();
    let worker = original.clone();
    std::thread::spawn(move || {
        assert!(worker.is_current());
        assert!(peer.is_current());
        assert_eq!(worker.rows[0].value("artist"), "One | Two");
        assert_eq!(peer.rows[0].value("artist"), "One / Two");
        let fork = worker.fork_snapshot_rows(None).unwrap();
        assert!(fork.is_current());
        assert_eq!(fork.rows[0].value("artist"), "One | Two");
    })
    .join()
    .unwrap();
    b.apply_settings(&settings(" :: "));
    assert!(
        original.is_current(),
        "another App must not invalidate this snapshot"
    );
    a.apply_settings(&settings(" + "));
    assert!(!original.is_current());
}
