//! Independent mutation-boundary probes using only disposable PCM fixtures.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tagrex_core::{
    journal::{SqliteJournal, UndoJournal},
    plan::{ChangePlan, Executor, FileChange},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-authority-final-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wav(sample: u8) -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend(356u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(8000u32.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(320u32.to_le_bytes());
    bytes.extend([sample; 320]);
    bytes
}

fn source_replacement_during_apply(copy: bool, same_directory: bool) {
    let fixture = Fixture::new("apply-leaf");
    let input = fixture.0.join("input");
    let output = fixture.0.join("output");
    std::fs::create_dir(&input).unwrap();
    std::fs::create_dir(&output).unwrap();
    let source = input.join("source.wav");
    let target = if same_directory {
        input.join("target.wav")
    } else {
        output.join("target.wav")
    };
    let preserved = input.join("approved-source.wav");
    let original = wav(0);
    let replacement = wav(1);
    std::fs::write(&source, &original).unwrap();
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            rename_to: Some(target.clone()),
            copy,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut changed = false;
    let outcome = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if !changed
                && progress.done > 0
                && progress.completed.is_none()
                && progress.path == source
            {
                std::fs::rename(&source, &preserved).unwrap();
                std::fs::write(&source, &replacement).unwrap();
                changed = true;
            }
        },
        &AtomicBool::new(false),
    );
    assert!(changed, "must reach mutation boundary");
    assert_eq!(std::fs::read(&preserved).unwrap(), original);
    assert_eq!(
        std::fs::read(&source).ok().as_deref(),
        Some(replacement.as_slice()),
        "unapproved replacement was moved; outcome {outcome:?}"
    );
    assert!(
        !target.exists(),
        "unapproved source must never be published at target; outcome {outcome:?}"
    );
    assert!(
        outcome.is_err(),
        "replacement must refuse instead of transferring a new source: {outcome:?}"
    );
}
#[test]
fn cross_directory_apply_move_refuses_replaced_leaf() {
    source_replacement_during_apply(false, false);
}
#[test]
fn same_directory_apply_move_refuses_replaced_leaf() {
    source_replacement_during_apply(false, true);
}
#[test]
fn cross_directory_apply_copy_refuses_replaced_leaf() {
    source_replacement_during_apply(true, false);
}

#[test]
fn cross_directory_undo_move_refuses_replaced_leaf_and_retains_history() {
    let fixture = Fixture::new("undo-leaf");
    let input = fixture.0.join("input");
    let output = fixture.0.join("output");
    std::fs::create_dir(&input).unwrap();
    std::fs::create_dir(&output).unwrap();
    let source = input.join("source.wav");
    let target = output.join("target.wav");
    let preserved = output.join("approved-target.wav");
    let original = wav(0);
    let replacement = wav(2);
    std::fs::write(&source, &original).unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            rename_to: Some(target.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap();
    drop(journal);
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let mut changed = false;
    let outcome = Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if !changed && progress.completed.is_none() && progress.path == target {
                std::fs::rename(&target, &preserved).unwrap();
                std::fs::write(&target, &replacement).unwrap();
                changed = true;
            }
        },
    );
    assert!(changed);
    assert_eq!(std::fs::read(&preserved).unwrap(), original);
    assert_eq!(
        std::fs::read(&target).ok().as_deref(),
        Some(replacement.as_slice()),
        "Undo moved an unapproved replacement: {outcome:?}"
    );
    assert!(
        !source.exists(),
        "Undo must not install an unapproved replacement at original path"
    );
    assert!(outcome.is_err(), "replacement must refuse: {outcome:?}");
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[test]
fn reopened_move_undo_does_not_claim_a_replacement_as_its_own_transfer() {
    let fixture = Fixture::new("durable-move-leaf");
    let source = fixture.0.join("source.wav");
    let target = fixture.0.join("target.wav");
    let preserved = fixture.0.join("approved-target.wav");
    let original = wav(0);
    let replacement = wav(3);
    std::fs::write(&source, &original).unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            rename_to: Some(target.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap();
    drop(journal);
    // A later process replaces the entry while the app is closed. The moved
    // fixture's original inode survives under a different owned fixture name.
    std::fs::rename(&target, &preserved).unwrap();
    std::fs::write(&target, &replacement).unwrap();
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let outcome = Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0));
    assert_eq!(std::fs::read(&preserved).unwrap(), original);
    assert_eq!(
        std::fs::read(&target).ok().as_deref(),
        Some(replacement.as_slice()),
        "reopened Undo claimed an unrelated replacement as its own move: {outcome:?}"
    );
    assert!(
        !source.exists(),
        "an unproved replacement must not be installed at the original path"
    );
    assert!(
        outcome.is_err(),
        "replaced durable target requires explicit resolution"
    );
    assert!(journal.batch(batch.id).unwrap().is_some());
}

#[test]
fn failed_apply_rollback_does_not_relocate_a_later_move_target_replacement() {
    let fixture = Fixture::new("rollback-move-leaf");
    let source = fixture.0.join("first.wav");
    let target = fixture.0.join("first-moved.wav");
    let second = fixture.0.join("second.wav");
    let blocked = fixture.0.join("second-moved.wav");
    let preserved = fixture.0.join("approved-target.wav");
    let original = wav(0);
    let replacement = wav(4);
    std::fs::write(&source, &original).unwrap();
    std::fs::write(&second, wav(1)).unwrap();
    let plan = ChangePlan {
        changes: vec![
            FileChange {
                path: source.clone(),
                rename_to: Some(target.clone()),
                ..Default::default()
            },
            FileChange {
                path: second.clone(),
                rename_to: Some(blocked.clone()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let mut changed = false;
    let outcome = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if !changed && progress.path == source && progress.completed.is_some() {
                std::fs::rename(&target, &preserved).unwrap();
                std::fs::write(&target, &replacement).unwrap();
                std::fs::write(&blocked, b"unrelated destination").unwrap();
                changed = true;
            }
        },
        &AtomicBool::new(false),
    );
    assert!(changed);
    assert!(outcome.is_err());
    assert_eq!(std::fs::read(&preserved).unwrap(), original);
    assert_eq!(std::fs::read(&blocked).unwrap(), b"unrelated destination");
    assert_eq!(
        std::fs::read(&target).ok().as_deref(),
        Some(replacement.as_slice()),
        "rollback moved a later external replacement: {outcome:?}"
    );
    assert!(
        !source.exists(),
        "rollback must preserve an unproved transfer for explicit resolution"
    );
    assert!(
        !journal.batches().unwrap().is_empty(),
        "failed rollback needs retained recovery history"
    );
}
