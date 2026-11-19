//! Fresh recovery review probes. These use owned PCM files and real journal I/O.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tagrex_core::journal::{SqliteJournal, UndoJournal};
use tagrex_core::plan::{ChangePlan, Executor, FileChange};

#[test]
fn apply_preserves_a_field_changed_after_its_preflight() {
    use tagrex_core::model::{TagEngine, TagField};
    use tagrex_core::plan::FieldChange;
    let fixture = Fixture::new("late-apply-field");
    let source = fixture.0.join("track.wav");
    std::fs::write(&source, wav(0)).unwrap();
    let mut track = TagEngine::read(&source).unwrap();
    track.tags.insert(TagField::Title, "Saved".into());
    TagEngine::write(&track).unwrap();
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            tag_changes: vec![FieldChange {
                field: TagField::Title,
                old: Some("Saved".into()),
                new: Some("Proposed".into()),
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut changed = false;
    let result = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if !changed
                && progress.done > 0
                && progress.completed.is_none()
                && progress.path == source
            {
                let mut external = TagEngine::read(&source).unwrap();
                external
                    .tags
                    .insert(TagField::Title, "External edit".into());
                TagEngine::write(&external).unwrap();
                changed = true;
            }
        },
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert!(changed, "the post-preflight mutation hook must run");
    assert!(
        result.is_err(),
        "a same-field external change must not be overwritten: {result:?}"
    );
    assert_eq!(
        TagEngine::read(&source).unwrap().tags[&TagField::Title],
        "External edit"
    );
}

#[test]
fn undo_preserves_a_field_changed_after_its_preflight() {
    use tagrex_core::model::{TagEngine, TagField};
    use tagrex_core::plan::FieldChange;
    let fixture = Fixture::new("late-undo-field");
    let source = fixture.0.join("track.wav");
    std::fs::write(&source, wav(0)).unwrap();
    let mut track = TagEngine::read(&source).unwrap();
    track.tags.insert(TagField::Title, "Saved".into());
    TagEngine::write(&track).unwrap();
    let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: source.clone(),
            tag_changes: vec![FieldChange {
                field: TagField::Title,
                old: Some("Saved".into()),
                new: Some("Applied".into()),
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&fixture.0)).unwrap();
    let mut changed = false;
    let result = Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&fixture.0),
        &mut |progress| {
            if !changed && progress.path == source && progress.completed.is_none() {
                let mut external = TagEngine::read(&source).unwrap();
                external
                    .tags
                    .insert(TagField::Title, "External edit".into());
                TagEngine::write(&external).unwrap();
                changed = true;
            }
        },
    );
    assert!(changed, "the post-preflight mutation hook must run");
    assert!(
        result.is_err(),
        "Undo must refuse a later field change: {result:?}"
    );
    assert_eq!(
        TagEngine::read(&source).unwrap().tags[&TagField::Title],
        "External edit"
    );
    assert!(journal.batch(batch.id).unwrap().is_some());
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-recovery-durable-review-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
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

#[cfg(unix)]
fn ancestor_redirect_at_undo_mutation(copy: bool) {
    let fixture = Fixture::new(if copy {
        "copy-ancestor"
    } else {
        "move-ancestor"
    });
    let allowed = fixture.0.join("allowed");
    let sub = allowed.join("sub");
    let outside = fixture.0.join("outside");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let source = allowed.join("source.wav");
    let target = sub.join("target.wav");
    std::fs::write(&source, wav(0)).unwrap();
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
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&allowed)).unwrap();
    let original = std::fs::read(&target).unwrap();
    let outside_target = outside.join("target.wav");
    let outside_bytes = if copy { original.clone() } else { wav(1) };
    std::fs::write(&outside_target, &outside_bytes).unwrap();
    // For the copy case, even the strong content evidence intentionally agrees.
    // Authorization still must not follow a newly redirected ancestor.
    std::fs::File::options()
        .write(true)
        .open(&outside_target)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::fs::metadata(&target).unwrap().modified().unwrap()),
        )
        .unwrap();
    let backup = allowed.join("sub-before-redirect");
    let mut redirected = false;
    let result = Executor::undo_with(
        &mut journal,
        batch.id,
        std::slice::from_ref(&allowed),
        &mut |progress| {
            if !redirected && progress.path == target {
                std::fs::rename(&sub, &backup).unwrap();
                std::os::unix::fs::symlink(&outside, &sub).unwrap();
                redirected = true;
            }
        },
    );
    assert!(redirected, "the mutation-boundary callback must have run");
    let outside_after = std::fs::read(&outside_target).ok();
    assert_eq!(
        outside_after.as_deref(),
        Some(outside_bytes.as_slice()),
        "outside file was removed or moved through a replaced ancestor; outcome: {result:?}"
    );
    assert!(
        result.is_err(),
        "Undo followed a replaced ancestor outside its approved root: {result:?}"
    );
    assert_eq!(
        std::fs::read(&outside_target).unwrap(),
        outside_bytes,
        "outside file must remain untouched"
    );
    assert_eq!(std::fs::read(backup.join("target.wav")).unwrap(), original);
    assert!(
        journal.batch(batch.id).unwrap().is_some(),
        "refused Undo must retain recovery history"
    );
}

#[cfg(unix)]
#[test]
fn copy_undo_never_deletes_through_an_ancestor_replaced_after_preflight() {
    ancestor_redirect_at_undo_mutation(true);
}

#[cfg(unix)]
#[test]
fn move_undo_never_transfers_an_outside_file_after_ancestor_replacement() {
    ancestor_redirect_at_undo_mutation(false);
}

#[test]
fn future_schema_refusal_preserves_even_unrecognized_payload_bytes() {
    let fixture = Fixture::new("future-payload");
    let db = fixture.0.join("journal.sqlite");
    drop(SqliteJournal::open(&db).unwrap());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("PRAGMA user_version=2; CREATE TABLE future_bytes(payload BLOB); INSERT INTO future_bytes VALUES(x'000102FF');").unwrap();
    drop(connection);
    let before = std::fs::read(&db).unwrap();
    assert!(SqliteJournal::open_or_recover(&db).is_err());
    assert_eq!(std::fs::read(&db).unwrap(), before);
    assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 1);
}

#[test]
fn copy_undo_after_restart_retains_source_and_removes_only_the_recorded_copy() {
    let fixture = Fixture::new("restart-good");
    let source = fixture.0.join("source.wav");
    let target = fixture.0.join("copy.wav");
    let bytes = wav(4);
    std::fs::write(&source, &bytes).unwrap();
    let db = fixture.0.join("journal.sqlite");
    let mut journal = SqliteJournal::open(&db).unwrap();
    let batch = Executor::apply(
        &ChangePlan {
            changes: vec![FileChange {
                path: source.clone(),
                rename_to: Some(target.clone()),
                copy: true,
                ..Default::default()
            }],
            ..Default::default()
        },
        &mut journal,
        std::slice::from_ref(&fixture.0),
    )
    .unwrap();
    drop(journal);
    let mut journal = SqliteJournal::open(&db).unwrap();
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&fixture.0)).unwrap();
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    assert!(!target.exists());
    assert!(journal.batches().unwrap().is_empty());
    assert!(!std::fs::read_dir(&fixture.0).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".tagrex-undo-copy-")));
}
