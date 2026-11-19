//! Common Executor policy boundaries on the portable WAV + SQLite route.
//! Fixtures are disposable; native FLAC admission cannot hide legacy behavior.
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tagrex_core::{
    journal::{SqliteJournal, UndoJournal},
    model::{ModelPolicy, TagEngine, TagField},
    plan::{ApplyStep, ChangePlan, Executor, FieldChange, FileChange},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-common-policy-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn file(&self, name: &str) -> PathBuf {
        let p = self.0.join(name);
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
        b.extend([0x31; 320]);
        std::fs::write(&p, b).unwrap();
        let mut track = TagEngine::read(&p).unwrap();
        track.tags.insert(TagField::Title, "Before".into());
        TagEngine::write(&track).unwrap();
        p
    }
    fn journal(&self) -> SqliteJournal {
        SqliteJournal::open(&self.0.join("journal.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn plan(paths: &[PathBuf]) -> ChangePlan {
    ChangePlan {
        changes: paths
            .iter()
            .map(|p| FileChange {
                path: p.clone(),
                tag_changes: vec![FieldChange {
                    field: TagField::Title,
                    old: Some("Before".into()),
                    new: Some("After".into()),
                }],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}
fn title(p: &Path) -> String {
    TagEngine::read(p).unwrap().tags[&TagField::Title].clone()
}
#[test]
fn legacy_wav_preflight_change_refuses_before_media_and_history() {
    let f = Fixture::new();
    let p = f.file("a.wav");
    let bytes = std::fs::read(&p).unwrap();
    let mut j = f.journal();
    let policy = ModelPolicy::default();
    let _scope = policy.enter();
    let result = Executor::apply_with(
        &plan(std::slice::from_ref(&p)),
        &mut j,
        std::slice::from_ref(&f.0),
        &mut |_| policy.configure(false, &[], "changed", ""),
        &AtomicBool::new(false),
    );
    assert!(result.is_err(), "legacy route accepted changed policy");
    assert_eq!(std::fs::read(&p).unwrap(), bytes);
    assert!(j.batches().unwrap().is_empty());
}
#[test]
fn legacy_wav_prefix_and_final_changes_restore_under_admitted_policy() {
    for final_only in [false, true] {
        for cancel_too in [false, true] {
            let f = Fixture::new();
            let a = f.file("a.wav");
            let b = f.file("b.wav");
            let mut j = f.journal();
            let policy = ModelPolicy::default();
            let _scope = policy.enter();
            let cancel = AtomicBool::new(false);
            let mut fired = false;
            let result = Executor::apply_with(
                &plan(&[a.clone(), b.clone()]),
                &mut j,
                std::slice::from_ref(&f.0),
                &mut |e| {
                    if !fired
                        && e.completed == Some(ApplyStep::Tags)
                        && (!final_only || e.path == b)
                    {
                        fired = true;
                        policy.configure(false, &[], "changed", "");
                        if cancel_too {
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                },
                &cancel,
            );
            assert!(fired);
            assert!(
                result.is_err(),
                "final={final_only} cancel={cancel_too} accepted dirty operation"
            );
            assert_eq!(title(&a), "Before", "{result:?}");
            assert_eq!(title(&b), "Before");
            assert!(j.batches().unwrap().is_empty());
        }
    }
}
#[test]
fn legacy_wav_callback_unwind_restores_owned_prefix() {
    let f = Fixture::new();
    let a = f.file("a.wav");
    let b = f.file("b.wav");
    let mut j = f.journal();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Executor::apply_with(
            &plan(&[a.clone(), b.clone()]),
            &mut j,
            std::slice::from_ref(&f.0),
            &mut |e| {
                if e.completed == Some(ApplyStep::Tags) {
                    panic!("owned callback panic")
                }
            },
            &AtomicBool::new(false),
        )
    }));
    assert!(result.is_err());
    assert_eq!(title(&a), "Before");
    assert_eq!(title(&b), "Before");
    assert!(j.batches().unwrap().is_empty());
}
