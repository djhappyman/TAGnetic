//! Persistence tests for the SQLite journal: the whole point of #5 is that a
//! recorded batch survives an application restart. "Restart" here is dropping
//! the `SqliteJournal` (closing the connection) and reopening the same file.

use std::path::{Path, PathBuf};

use tagrex_core::journal::{
    AppliedBatch, AppliedValue, BatchId, CopyStamp, FileRevision, SqliteJournal, UndoJournal,
};
use tagrex_core::model::{Id3v2Revision, TagEngine, TagField};
use tagrex_core::plan::{ChangePlan, Executor, FieldChange, FileChange};

const MINIMAL_FLAC: [u8; 76] = [
    0x66, 0x4c, 0x61, 0x43, 0x00, 0x00, 0x00, 0x22, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x0a, 0xc4, 0x42, 0xf0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x81, 0x00, 0x00, 0x10, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // One audio frame (#416): 4096 samples of stereo silence, two CONSTANT
    // subframes, with their CRCs. A file is tags followed by audio, and a
    // tag write is only ever tested against that shape.
    0xff, 0xf8, 0xc9, 0x18, 0x00, 0xc2, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb8, 0xee,
];

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tagrex-sqlite-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn flac(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, MINIMAL_FLAC).unwrap();
        path
    }

    fn db(&self) -> PathBuf {
        self.0.join("journal.sqlite")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// The allowed-root slice the executor takes (#153). Every test here works
/// inside one directory; the multi-root cases build their own.
fn roots(dir: &Path) -> Vec<PathBuf> {
    vec![dir.to_path_buf()]
}

fn set_artist(path: &Path, old: Option<&str>, new: Option<&str>) -> ChangePlan {
    ChangePlan {
        description: "set artist".to_string(),
        changes: vec![FileChange {
            path: path.to_path_buf(),
            tag_changes: vec![FieldChange {
                field: TagField::Artist,
                old: old.map(str::to_string),
                new: new.map(str::to_string),
            }],
            cover_change: None,
            rename_to: None,
            sidecar_renames: Vec::new(),
            ..FileChange::default()
        }],
        ..ChangePlan::default()
    }
}

#[test]
fn batch_survives_reopening_the_database() {
    let dir = TempDir::new("persist");
    let track = dir.flac("track.flac");

    let batch_id = {
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        let plan = set_artist(&track, None, Some("Persisted Artist"));
        Executor::apply(&plan, &mut journal, &roots(dir.path()))
            .unwrap()
            .id
        // journal dropped here -> connection closed, simulating app exit
    };

    // Reopen the same database file, as a fresh process would.
    let journal = SqliteJournal::open(&dir.db()).unwrap();
    let batches = journal.batches().unwrap();

    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.id, batch_id);
    assert_eq!(batch.description, "set artist");
    assert_eq!(batch.plan.changes.len(), 1);
    let change = &batch.plan.changes[0];
    assert_eq!(change.path, track);
    assert_eq!(change.tag_changes[0].field, TagField::Artist);
    assert_eq!(change.tag_changes[0].old, None);
    assert_eq!(
        change.tag_changes[0].new.as_deref(),
        Some("Persisted Artist")
    );
}

#[test]
fn undo_works_against_a_reopened_journal() {
    let dir = TempDir::new("undo-restart");
    let track = dir.flac("track.flac");

    let batch_id = {
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        let plan = set_artist(&track, None, Some("Temporary"));
        Executor::apply(&plan, &mut journal, &roots(dir.path()))
            .unwrap()
            .id
    };
    assert!(TagEngine::read(&track)
        .unwrap()
        .tags
        .contains_key(&TagField::Artist));

    // Reopen and roll back -- the morning-after scenario.
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    Executor::undo(&mut journal, batch_id, &roots(dir.path())).unwrap();

    assert!(!TagEngine::read(&track)
        .unwrap()
        .tags
        .contains_key(&TagField::Artist));
    assert!(journal.batches().unwrap().is_empty());
}

#[test]
fn custom_field_round_trips_through_storage() {
    let dir = TempDir::new("custom");
    let track = dir.flac("track.flac");

    let plan = ChangePlan {
        description: "set custom".to_string(),
        changes: vec![FileChange {
            path: track.clone(),
            tag_changes: vec![FieldChange {
                field: TagField::Custom("MOOD".to_string()),
                old: None,
                new: Some("Energetic".to_string()),
            }],
            cover_change: None,
            rename_to: None,
            sidecar_renames: Vec::new(),
            ..FileChange::default()
        }],
        ..ChangePlan::default()
    };

    {
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        Executor::apply(&plan, &mut journal, &roots(dir.path())).unwrap();
    }

    let journal = SqliteJournal::open(&dir.db()).unwrap();
    let batches = journal.batches().unwrap();
    assert_eq!(
        batches[0].plan.changes[0].tag_changes[0].field,
        TagField::Custom("MOOD".to_string())
    );
}

/// #47: the snapshot of a stripped block is the only copy of it left, so it has
/// to survive a restart — otherwise the "realized in the morning" scenario this
/// journal exists for loses the block instead of restoring it.
#[test]
fn a_stripped_block_can_still_be_restored_after_a_restart() {
    use lofty::config::WriteOptions;
    use lofty::id3::v1::Id3v1Tag;
    use lofty::prelude::TagExt;
    use tagrex_core::model::TagBlockKind;
    use tagrex_core::plan::BlockChange;

    let dir = TempDir::new("block-restart");
    let track = dir.path().join("track.mp3");
    let mut frame = vec![0xFF, 0xFB, 0x90, 0x00];
    frame.resize(417, 0);
    std::fs::write(&track, frame.repeat(5)).unwrap();
    let legacy = Id3v1Tag {
        artist: Some("Stale Artist".to_string()),
        title: Some("Stale Title".to_string()),
        ..Default::default()
    };
    legacy
        .save_to_path(&track, WriteOptions::default())
        .unwrap();

    let removed = TagEngine::read_block(&track, TagBlockKind::Id3v1)
        .unwrap()
        .expect("the file carries an ID3v1 block");
    let plan = ChangePlan {
        description: "Remove ID3v1 tag".to_string(),
        changes: vec![FileChange {
            path: track.clone(),
            block_changes: vec![BlockChange::removal(TagBlockKind::Id3v1, removed.clone())],
            ..FileChange::default()
        }],
        ..ChangePlan::default()
    };

    let batch_id = {
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        Executor::apply(&plan, &mut journal, &roots(dir.path()))
            .unwrap()
            .id
    };
    assert!(TagEngine::read_block(&track, TagBlockKind::Id3v1)
        .unwrap()
        .is_none());

    // Reopen, as a fresh process would, and roll back.
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    assert_eq!(
        journal.batches().unwrap()[0].plan.changes[0].block_changes,
        vec![BlockChange::removal(TagBlockKind::Id3v1, removed.clone())],
        "the snapshot did not survive the reopen"
    );
    Executor::undo(&mut journal, batch_id, &roots(dir.path())).unwrap();

    assert_eq!(
        TagEngine::read_block(&track, TagBlockKind::Id3v1).unwrap(),
        Some(removed)
    );
}

/// #206: the bytes of a destroyed ID3v2 block are the only copy of its binary
/// frames, so they have to survive a restart along with everything else.
#[test]
fn the_bytes_of_a_destroyed_id3v2_block_survive_a_restart() {
    use lofty::config::WriteOptions;
    use lofty::prelude::{Accessor, TagExt};
    use tagrex_core::model::{TagBlockKind, TagEngine as Engine};
    use tagrex_core::plan::BlockChange;

    let dir = TempDir::new("block-bytes-restart");
    let track = dir.path().join("track.mp3");
    let mut frame = vec![0xFF, 0xFB, 0x90, 0x00];
    frame.resize(417, 0);
    std::fs::write(&track, frame.repeat(5)).unwrap();
    let mut tag = lofty::id3::v2::Id3v2Tag::new();
    tag.set_artist("Journaled".to_string());
    tag.save_to_path(&track, WriteOptions::default()).unwrap();

    let content = Engine::read_block(&track, TagBlockKind::Id3v2)
        .unwrap()
        .unwrap();
    let bytes = Engine::dump_id3v2(&track).unwrap().expect("bytes");
    let plan = ChangePlan {
        description: "Remove ID3v2 tag".to_string(),
        changes: vec![FileChange {
            path: track.clone(),
            block_changes: vec![BlockChange {
                kind: TagBlockKind::Id3v2,
                revision: None,
                old_revision: None,
                old_bytes: Some(bytes.clone()),
                old: Some(content),
                new: None,
                text_to_unicode: false,
            }],
            ..FileChange::default()
        }],
        ..ChangePlan::default()
    };

    {
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        Executor::apply(&plan, &mut journal, &roots(dir.path())).unwrap();
    }

    let journal = SqliteJournal::open(&dir.db()).unwrap();
    assert_eq!(
        journal.batches().unwrap()[0].plan.changes[0].block_changes[0].old_bytes,
        Some(bytes),
        "the block's bytes did not survive the reopen"
    );
}

#[test]
fn ids_keep_climbing_across_reopens() {
    let dir = TempDir::new("ids");
    let track = dir.flac("track.flac");

    let first = {
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        Executor::apply(
            &set_artist(&track, None, Some("A")),
            &mut journal,
            &roots(dir.path()),
        )
        .unwrap()
        .id
    };

    // A fresh connection must not restart ids from 1 and collide.
    let second = {
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        Executor::apply(
            &set_artist(&track, Some("A"), Some("B")),
            &mut journal,
            &roots(dir.path()),
        )
        .unwrap()
        .id
    };

    assert_ne!(first, second);
    assert!(second.0 > first.0);
}

#[test]
fn one_batch_can_be_fetched_without_loading_the_others() {
    let dir = TempDir::new("one-batch");
    let first = dir.flac("first.flac");
    let second = dir.flac("second.flac");
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    let a = Executor::apply(
        &set_artist(&first, None, Some("First")),
        &mut journal,
        &roots(dir.path()),
    )
    .unwrap();
    let b = Executor::apply(
        &set_artist(&second, None, Some("Second")),
        &mut journal,
        &roots(dir.path()),
    )
    .unwrap();

    let fetched = journal.batch(a.id).unwrap().expect("the batch is there");
    assert_eq!(fetched.id, a.id);
    assert_eq!(fetched.plan.changes.len(), 1);
    assert_eq!(fetched.plan.changes[0].path, first);
    assert_eq!(
        journal.batch(b.id).unwrap().unwrap().plan.changes[0].path,
        second
    );
    assert!(journal.batch(BatchId(9999)).unwrap().is_none());
}

/// Every child table is read and deleted by its parent's id; without an index
/// each of those lookups scans the whole table, and a journal with a few large
/// batches took minutes to load or roll back (#449).
#[test]
fn the_journal_tables_are_indexed_by_their_parent() {
    let dir = TempDir::new("indexes");
    drop(SqliteJournal::open(&dir.db()).unwrap());
    let conn = rusqlite::Connection::open(dir.db()).unwrap();
    for query in [
        "SELECT * FROM file_changes WHERE batch_id = 1",
        "SELECT * FROM field_changes WHERE file_change_id = 1",
        "SELECT * FROM cover_images WHERE file_change_id = 1",
        "SELECT * FROM sidecar_renames WHERE file_change_id = 1",
        "SELECT * FROM block_changes WHERE file_change_id = 1",
        "SELECT * FROM block_change_fields WHERE block_change_id = 1",
        "SELECT * FROM allowed_roots WHERE batch_id = 1",
        "SELECT * FROM copy_stamps WHERE batch_id = 1",
        "SELECT * FROM applied_values WHERE batch_id = 1",
    ] {
        let plan: String = conn
            .query_row(&format!("EXPLAIN QUERY PLAN {query}"), [], |row| row.get(3))
            .unwrap();
        assert!(plan.contains("USING INDEX"), "{query}: {plan}");
    }
}

#[test]
fn the_stamp_of_a_copy_survives_a_restart() {
    let dir = TempDir::new("copy-stamp");
    let track = dir.flac("track.flac");
    let target = dir.path().join("copy.flac");
    let plan = ChangePlan {
        description: "copy".to_string(),
        changes: vec![FileChange {
            path: track,
            rename_to: Some(target.clone()),
            copy: true,
            ..FileChange::default()
        }],
        ..ChangePlan::default()
    };
    let id = {
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        Executor::apply(&plan, &mut journal, &roots(dir.path()))
            .unwrap()
            .id
    };

    let journal = SqliteJournal::open(&dir.db()).unwrap();
    let batch = journal.batch(id).unwrap().unwrap();
    assert_eq!(batch.copy_stamps, vec![CopyStamp::of(&target).unwrap()]);
}

/// What a batch left each field as (#658) is what undo checks the file
/// against, so it has to be there after a restart.
#[test]
fn what_a_batch_left_survives_a_restart() {
    let dir = TempDir::new("applied-values");
    let track = dir.flac("track.flac");
    let plan = ChangePlan {
        description: "set".to_string(),
        changes: vec![FileChange {
            path: track.clone(),
            tag_changes: vec![
                FieldChange {
                    field: TagField::Artist,
                    old: None,
                    new: Some("Y".to_string()),
                },
                FieldChange {
                    field: TagField::Album,
                    old: None,
                    new: None,
                },
            ],
            ..FileChange::default()
        }],
        ..ChangePlan::default()
    };
    let id = {
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        Executor::apply(&plan, &mut journal, &roots(dir.path()))
            .unwrap()
            .id
    };

    let journal = SqliteJournal::open(&dir.db()).unwrap();
    let batch = journal.batch(id).unwrap().unwrap();
    assert_eq!(
        batch.applied_values,
        vec![
            AppliedValue {
                path: track.clone(),
                field: TagField::Artist,
                value: Some("Y".to_string()),
            },
            AppliedValue {
                path: track,
                field: TagField::Album,
                value: None,
            },
        ]
    );
}

/// #659: the batch is in the journal, marked interrupted, before the first file
/// is written, and loses the mark once every file is.
#[test]
fn a_batch_is_journaled_before_its_first_write() {
    use std::sync::atomic::AtomicBool;

    let dir = TempDir::new("write-ahead");
    let tracks = [dir.flac("a.flac"), dir.flac("b.flac")];
    let plan = ChangePlan {
        description: "set".to_string(),
        changes: tracks
            .iter()
            .map(|path| FileChange {
                path: path.clone(),
                tag_changes: vec![FieldChange {
                    field: TagField::Artist,
                    old: None,
                    new: Some("Y".to_string()),
                }],
                ..FileChange::default()
            })
            .collect(),
        ..ChangePlan::default()
    };
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    let mut seen = Vec::new();
    let batch = Executor::apply_with(
        &plan,
        &mut journal,
        &roots(dir.path()),
        &mut |progress| {
            // Preflight reports before a batch exists and writes nothing.
            if progress.done < plan.changes.len() {
                return;
            }
            // What another process would find, were this one to stop here.
            let other = SqliteJournal::open(&dir.db()).unwrap();
            seen.push(
                other
                    .batches()
                    .unwrap()
                    .iter()
                    .map(|batch| batch.interrupted)
                    .collect::<Vec<_>>(),
            );
        },
        &AtomicBool::new(false),
    )
    .unwrap();

    assert!(!seen.is_empty());
    assert!(seen.iter().all(|batches| batches == &[true]), "{seen:?}");
    assert!(!batch.interrupted);
    let reopened = SqliteJournal::open(&dir.db()).unwrap();
    let stored = reopened.batch(batch.id).unwrap().unwrap();
    assert!(!stored.interrupted);
    assert_eq!(stored.applied_values.len(), 2);
}

/// #659: a batch the process never finished is still there at the next start,
/// marked, and undoing it puts back what it reached and skips what it did not.
#[test]
fn legacy_interrupted_moves_without_identity_refuse_automatic_recovery() {
    let dir = TempDir::new("interrupted");
    let reached = dir.flac("reached.flac");
    let moved = dir.path().join("moved.flac");
    let unreached = dir.flac("unreached.flac");
    let never_moved = dir.path().join("never-moved.flac");
    let change = |path: &PathBuf, rename_to: &PathBuf| FileChange {
        path: path.clone(),
        tag_changes: vec![FieldChange {
            field: TagField::Artist,
            old: None,
            new: Some("Y".to_string()),
        }],
        rename_to: Some(rename_to.clone()),
        ..FileChange::default()
    };
    let plan = ChangePlan {
        description: "set and move".to_string(),
        changes: vec![change(&reached, &moved), change(&unreached, &never_moved)],
        ..ChangePlan::default()
    };
    // What the process got done before it stopped: the first file written and
    // moved, the second not touched.
    let mut file = TagEngine::read(&reached).unwrap();
    file.tags.insert(TagField::Artist, "Y".to_string());
    TagEngine::write(&file).unwrap();
    std::fs::rename(&reached, &moved).unwrap();
    SqliteJournal::open(&dir.db())
        .unwrap()
        .record(&AppliedBatch {
            native_tags: false,
            id: BatchId(0),
            description: plan.description.clone(),
            message: None,
            applied_at: 0,
            plan,
            created_dirs: Vec::new(),
            removed_dirs: Vec::new(),
            allowed_roots: roots(dir.path()),
            copy_stamps: Vec::new(),
            applied_values: Vec::new(),
            block_fingerprints: Vec::new(),
            interrupted: true,
            id3v2_revisions: Vec::new(),
        })
        .unwrap();

    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    let batches = journal.batches().unwrap();
    assert_eq!(batches.len(), 1);
    assert!(batches[0].interrupted);
    // Legacy intent lacks native move ownership. Even manually restoring its
    // field cannot prove that a named output is the original transferred inode.
    for restored in [false, true] {
        if restored {
            let mut file = TagEngine::read(&moved).unwrap();
            file.tags.remove(&TagField::Artist);
            TagEngine::write(&file).unwrap();
        }
        let before = std::fs::read(&moved).unwrap();
        assert!(matches!(
            Executor::undo(&mut journal, batches[0].id, &roots(dir.path())),
            Err(tagrex_core::plan::PlanError::Stale(_))
        ));
        assert_eq!(std::fs::read(&moved).unwrap(), before);
        assert!(moved.exists() && !reached.exists());
        assert!(unreached.exists() && !never_moved.exists());
        assert_eq!(journal.batches().unwrap().len(), 1);
    }
}

/// #666: the revision each file was in survives a restart, recorded once even
/// though the batch is written to the journal twice.
#[test]
fn the_id3v2_revisions_survive_a_restart() {
    let dir = TempDir::new("revisions");
    let revisions = vec![
        FileRevision {
            path: dir.path().join("a.mp3"),
            revision: Id3v2Revision::V3,
        },
        FileRevision {
            path: dir.path().join("b.mp3"),
            revision: Id3v2Revision::V4,
        },
    ];
    let mut batch = AppliedBatch {
        native_tags: false,
        id: BatchId(0),
        description: "edit".to_string(),
        message: None,
        applied_at: 0,
        plan: ChangePlan::default(),
        created_dirs: Vec::new(),
        removed_dirs: Vec::new(),
        allowed_roots: Vec::new(),
        copy_stamps: Vec::new(),
        applied_values: Vec::new(),
        block_fingerprints: Vec::new(),
        interrupted: true,
        id3v2_revisions: revisions.clone(),
    };
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    batch.id = journal.record(&batch).unwrap();
    batch.interrupted = false;
    journal.finish(batch.id, &batch).unwrap();
    drop(journal);

    let reopened = SqliteJournal::open(&dir.db()).unwrap();
    let stored = reopened.batch(batch.id).unwrap().unwrap();
    assert_eq!(stored.id3v2_revisions, revisions);
}

/// A damaged journal made every library refuse to open. It is set aside, byte
/// for byte, and a fresh journal takes its place.
#[test]
fn a_damaged_journal_is_set_aside_and_started_afresh() {
    let dir = TempDir::new("damaged");
    let db = dir.db();
    {
        let mut journal = SqliteJournal::open(&db).unwrap();
        let track = dir.flac("a.flac");
        let plan = ChangePlan {
            description: "seed".to_string(),
            changes: vec![FileChange {
                path: track,
                tag_changes: vec![FieldChange {
                    field: TagField::Artist,
                    old: None,
                    new: Some("A".to_string()),
                }],
                ..FileChange::default()
            }],
            ..ChangePlan::default()
        };
        Executor::apply(&plan, &mut journal, &[dir.path().to_path_buf()]).unwrap();
    }
    let healthy = std::fs::read(&db).unwrap();
    let mut header_corrupt = healthy.clone();
    for byte in &mut header_corrupt[16..100] {
        *byte = 0xFF;
    }
    let cases: [(&str, Vec<u8>); 3] = [
        ("garbage", b"this is not a database".repeat(100)),
        ("truncated", healthy[..100].to_vec()),
        ("header", header_corrupt),
    ];
    for (name, bytes) in cases {
        std::fs::write(&db, &bytes).unwrap();
        assert!(
            SqliteJournal::open(&db).is_err(),
            "{name}: fixture must be damaged"
        );

        let (journal, aside) = SqliteJournal::open_or_recover(&db).unwrap();
        let aside = aside.unwrap_or_else(|| panic!("{name}: nothing set aside"));
        assert!(journal.batches().unwrap().is_empty(), "{name}");
        assert_eq!(
            std::fs::read(&aside).unwrap(),
            bytes,
            "{name}: kept as it was"
        );
        std::fs::remove_file(&aside).ok();
        drop(journal);
        std::fs::remove_file(&db).ok();
    }
}

/// Only damage sets a journal aside: a healthy one opens as it is, and one that
/// cannot be opened for another reason is an error with nothing moved.
#[test]
fn a_healthy_or_unavailable_journal_is_never_set_aside() {
    let dir = TempDir::new("not-damaged");
    let db = dir.db();
    drop(SqliteJournal::open(&db).unwrap());
    let (_, aside) = SqliteJournal::open_or_recover(&db).unwrap();
    assert_eq!(aside, None);

    // A folder where the file should be cannot be opened, and is no database
    // to recover either.
    let folder = dir.path().join("journal-folder.sqlite");
    std::fs::create_dir_all(&folder).unwrap();
    assert!(SqliteJournal::open_or_recover(&folder).is_err());
    assert!(folder.is_dir(), "an unavailable journal is not moved");
}

#[test]
fn shared_image_snapshots_are_stored_once_and_legacy_inline_rows_still_load() {
    use tagrex_core::model::{CoverArt, CoverKind};
    use tagrex_core::plan::CoverChange;
    let dir = TempDir::new("image-pool");
    let a = dir.flac("a.flac");
    let b = dir.flac("b.flac");
    let art = CoverArt {
        mime: "image/jpeg".into(),
        data: vec![7; 65536].into(),
        kind: CoverKind::Back,
        description: "Sleeve".into(),
    };
    let plan = ChangePlan {
        changes: [a, b]
            .iter()
            .map(|p| FileChange {
                path: p.clone(),
                cover_change: Some(CoverChange {
                    old: Vec::new(),
                    new: vec![art.clone()],
                }),
                ..FileChange::default()
            })
            .collect(),
        ..ChangePlan::default()
    };
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    let batch = Executor::apply(&plan, &mut journal, &roots(dir.path())).unwrap();
    drop(journal);
    let connection = rusqlite::Connection::open(dir.db()).unwrap();
    let (count, bytes): (i64, i64) = connection
        .query_row(
            "SELECT count(*), sum(length(data)) FROM journal_image_blobs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((count, bytes), (1, 65536));
    // A reverted binary's original query must fail closed, never silently
    // restore empty image bytes from a new pooled row.
    assert!(connection
        .query_row("SELECT data FROM cover_images LIMIT 1", [], |row| row
            .get::<_, Vec<u8>>(0))
        .is_err());
    {
        let mut reopened = SqliteJournal::open(&dir.db()).unwrap();
        let loaded = reopened.batch(batch.id).unwrap().unwrap();
        let bytes = &loaded.plan.changes[0].cover_change.as_ref().unwrap().new[0].data;
        assert_eq!(
            bytes.as_ptr(),
            loaded.plan.changes[1].cover_change.as_ref().unwrap().new[0]
                .data
                .as_ptr()
        );
        assert_eq!(
            bytes.as_ptr(),
            loaded.clone().plan.changes[0]
                .cover_change
                .as_ref()
                .unwrap()
                .new[0]
                .data
                .as_ptr()
        );
        let mut with_block = batch.clone();
        with_block.plan.changes[0]
            .block_changes
            .push(tagrex_core::plan::BlockChange::removal(
                tagrex_core::model::TagBlockKind::VorbisComments,
                tagrex_core::model::TagBlockContent {
                    covers: vec![art.clone()],
                    ..Default::default()
                },
            ));
        let id = reopened.record(&with_block).unwrap();
        let cross_table = reopened.batch(id).unwrap().unwrap();
        assert_eq!(
            cross_table.plan.changes[0]
                .cover_change
                .as_ref()
                .unwrap()
                .new[0]
                .data
                .as_ptr(),
            cross_table.plan.changes[0].block_changes[0]
                .old
                .as_ref()
                .unwrap()
                .covers[0]
                .data
                .as_ptr()
        );
        reopened.rollback(id).unwrap();
    }
    // Simulate a pre-pool inline snapshot in the same migrated database.
    connection.execute("UPDATE cover_images SET data = ?1, blob_id = NULL WHERE id = (SELECT min(id) FROM cover_images)",rusqlite::params![art.data]).unwrap();
    drop(connection);
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    assert_eq!(
        journal.batch(batch.id).unwrap().unwrap().plan.changes[0]
            .cover_change
            .as_ref()
            .unwrap()
            .new,
        [art]
    );
    Executor::undo(&mut journal, batch.id, &roots(dir.path())).unwrap();
    let connection = rusqlite::Connection::open(dir.db()).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM journal_image_blobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn history_headers_never_hydrate_snapshot_tables() {
    let dir = TempDir::new("headers-only");
    let track = dir.flac("track.flac");
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    let batch = Executor::apply(
        &set_artist(&track, None, Some("A")),
        &mut journal,
        &roots(dir.path()),
    )
    .unwrap();
    let conn = rusqlite::Connection::open(dir.db()).unwrap();
    // A header query must not need even the text-snapshot table, let alone
    // images. Damage is deliberate and confined to this disposable journal.
    conn.execute("DROP TABLE field_changes", []).unwrap();
    assert!(journal.batch(batch.id).is_err());
    let headers = journal.headers(&roots(dir.path())).unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].id, batch.id);
    assert!(journal
        .headers(&[dir.path().join("other-library")])
        .unwrap()
        .is_empty());
}

#[test]
fn image_pool_checks_bytes_on_hash_collision_and_keeps_other_batch_references() {
    use std::hash::{Hash, Hasher};
    use tagrex_core::model::{CoverArt, CoverKind};
    use tagrex_core::plan::CoverChange;
    let dir = TempDir::new("image-collision");
    let a = dir.flac("a.flac");
    let b = dir.flac("b.flac");
    let art = CoverArt {
        mime: "image/jpeg".into(),
        data: vec![7; 64].into(),
        kind: CoverKind::Back,
        description: "Back".into(),
    };
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    let conn = rusqlite::Connection::open(dir.db()).unwrap();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    art.data.hash(&mut hash);
    // A deliberately colliding index candidate is not a matching image.
    conn.execute(
        "INSERT INTO journal_image_blobs (hash,size,data) VALUES (?1,?2,?3)",
        rusqlite::params![hash.finish() as i64, 64, vec![9u8; 64]],
    )
    .unwrap();
    let make_plan = |path: PathBuf| ChangePlan {
        changes: vec![FileChange {
            path,
            cover_change: Some(CoverChange {
                old: Vec::new(),
                new: vec![art.clone()],
            }),
            ..FileChange::default()
        }],
        ..ChangePlan::default()
    };
    let first = Executor::apply(&make_plan(a), &mut journal, &roots(dir.path())).unwrap();
    let second = Executor::apply(&make_plan(b), &mut journal, &roots(dir.path())).unwrap();
    assert_eq!(
        journal.batch(first.id).unwrap().unwrap().plan.changes[0]
            .cover_change
            .as_ref()
            .unwrap()
            .new,
        [art]
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM journal_image_blobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    Executor::undo(&mut journal, second.id, &roots(dir.path())).unwrap();
    // One blob is still referenced; only the synthetic collision orphan went.
    assert_eq!(
        conn.query_row("SELECT count(*) FROM journal_image_blobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    Executor::undo(&mut journal, first.id, &roots(dir.path())).unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM journal_image_blobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

/// Stop at the transaction boundary without running Apply's error rollback.
/// Exit the disposable child without unwinding, so no automatic rollback runs.
/// Reopening SQLite reads the actual durable write-ahead record.
struct StopBeforeFinish(SqliteJournal);

impl UndoJournal for StopBeforeFinish {
    fn supports_copy_checkpoints(&self) -> bool {
        self.0.supports_copy_checkpoints()
    }
    fn checkpoint_copy(
        &mut self,
        id: BatchId,
        stamp: &tagrex_core::journal::CopyStamp,
        previous: Option<&tagrex_core::journal::CopyStamp>,
    ) -> Result<(), tagrex_core::journal::JournalError> {
        self.0.checkpoint_copy(id, stamp, previous)
    }

    fn prepare_move_stage(
        &mut self,
        id: BatchId,
        stage: &tagrex_core::journal::MoveStage,
    ) -> Result<(), tagrex_core::journal::JournalError> {
        self.0.prepare_move_stage(id, stage)
    }
    fn move_stages(
        &self,
        id: BatchId,
    ) -> Result<Vec<tagrex_core::journal::MoveStage>, tagrex_core::journal::JournalError> {
        self.0.move_stages(id)
    }

    fn record(
        &mut self,
        batch: &AppliedBatch,
    ) -> Result<BatchId, tagrex_core::journal::JournalError> {
        self.0.record(batch)
    }
    fn batches(&self) -> Result<Vec<AppliedBatch>, tagrex_core::journal::JournalError> {
        self.0.batches()
    }
    fn finish(
        &mut self,
        _: BatchId,
        _: &AppliedBatch,
    ) -> Result<(), tagrex_core::journal::JournalError> {
        std::process::exit(73)
    }
    fn rollback(&mut self, id: BatchId) -> Result<(), tagrex_core::journal::JournalError> {
        self.0.rollback(id)
    }
}

// A separate disposable SQLite database transports plan changes using the real
// journal codec, including cover/block bytes. Pruning is Apply-only and therefore
// travels separately. The seed database is never execution history.
fn exit_child_at(dir: &TempDir, plan: &ChangePlan, progress_done: Option<usize>) {
    let seed_dir = dir.path().join("crash-plan-seed");
    std::fs::create_dir(&seed_dir).unwrap();
    let mut seed = SqliteJournal::open(&seed_dir.join("plan.sqlite")).unwrap();
    seed.record(&AppliedBatch {
        native_tags: false,
        id: BatchId(0),
        description: plan.description.clone(),
        message: plan.message.clone(),
        applied_at: 0,
        plan: plan.clone(),
        allowed_roots: roots(dir.path()),
        created_dirs: Vec::new(),
        removed_dirs: Vec::new(),
        copy_stamps: Vec::new(),
        applied_values: Vec::new(),
        block_fingerprints: Vec::new(),
        interrupted: true,
        id3v2_revisions: Vec::new(),
    })
    .unwrap();
    drop(seed);
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "sqlite_interruption_process_child",
            "--nocapture",
        ])
        .env("TAGREX_SQLITE_CRASH_ROOT", dir.path())
        .env(
            "TAGREX_SQLITE_CRASH_PRUNE",
            plan.prune_empty_dirs.to_string(),
        )
        .env(
            "TAGREX_SQLITE_CRASH_DONE",
            progress_done.map_or_else(|| "finish".into(), |done| done.to_string()),
        );
    let log_path = seed_dir.join("child.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = command
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    drop(command); // release parent log handles before Windows fixture cleanup
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(30) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => {
                let _ = child.kill();
                let reaped = child.wait();
                panic!(
                    "child timed out or wait failed: {result:?}; reaped={reaped:?}\n{}",
                    String::from_utf8_lossy(&std::fs::read(&log_path).unwrap())
                );
            }
        }
    };
    assert_eq!(
        status.code(),
        Some(73),
        "child missed the requested durable boundary: {}",
        String::from_utf8_lossy(&std::fs::read(&log_path).unwrap())
    );
    std::fs::remove_dir_all(seed_dir).unwrap();
}

/// Dispatch only: the parent tests assert exit73 and the reopened recovery state.
#[test]
fn sqlite_interruption_process_child() {
    let Some(root) = std::env::var_os("TAGREX_SQLITE_CRASH_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let seed = SqliteJournal::open(&root.join("crash-plan-seed/plan.sqlite")).unwrap();
    let mut plan = seed.batches().unwrap().remove(0).plan;
    plan.prune_empty_dirs = std::env::var("TAGREX_SQLITE_CRASH_PRUNE")
        .unwrap()
        .parse()
        .unwrap();
    drop(seed);
    let boundary = std::env::var("TAGREX_SQLITE_CRASH_DONE").unwrap();
    let mut journal = SqliteJournal::open(&root.join("journal.sqlite")).unwrap();
    if boundary == "finish" {
        Executor::apply(&plan, &mut StopBeforeFinish(journal), &roots(&root)).unwrap();
    } else {
        let done: usize = boundary.parse().unwrap();
        Executor::apply_with(
            &plan,
            &mut journal,
            &roots(&root),
            &mut |progress| {
                if progress.done == done {
                    std::process::exit(73);
                }
            },
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
    }
    panic!("child did not reach its requested interruption boundary");
}

fn interrupt_before_finish(dir: &TempDir, plan: &ChangePlan) -> AppliedBatch {
    exit_child_at(dir, plan, None);
    // Explicit historical/missing-proof fixture: new execution checkpoints
    // completed copies before finish, unlike the older journal being tested.
    let conn = rusqlite::Connection::open(dir.db()).unwrap();
    conn.execute("DELETE FROM copy_stamps WHERE moved=0", [])
        .unwrap();
    drop(conn);
    let reopened = SqliteJournal::open(&dir.db()).unwrap();
    let batch = reopened.batches().unwrap().remove(0);
    assert!(batch.interrupted);
    // This historical fixture keeps move identities, but deliberately drops
    // completed-copy outcomes to exercise unproved older journal history.
    assert!(batch
        .copy_stamps
        .iter()
        .all(|stamp| stamp.moved && stamp.identity.is_some() && stamp.sha256.is_none()));
    batch
}

#[test]
fn interrupted_unstamped_copies_and_sidecars_are_never_deleted() {
    use tagrex_core::plan::{PlanError, UndoMode};
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        for sidecar_only in [false, true] {
            let dir = TempDir::new("unstamped-copy");
            let source = dir.flac("source.flac");
            let target = dir.path().join("target.flac");
            let sidecar = dir.path().join("source.cue");
            let sidecar_target = dir.path().join("target.cue");
            std::fs::write(&sidecar, b"original cue").unwrap();
            let source_bytes = std::fs::read(&source).unwrap();
            let plan = ChangePlan {
                changes: vec![FileChange {
                    path: source.clone(),
                    rename_to: Some(target.clone()),
                    copy: true,
                    sidecar_renames: vec![(sidecar.clone(), sidecar_target.clone())],
                    ..FileChange::default()
                }],
                ..ChangePlan::default()
            };
            let batch = interrupt_before_finish(&dir, &plan);
            if sidecar_only {
                std::fs::remove_file(&target).unwrap();
            }
            let replacement = if sidecar_only {
                &sidecar_target
            } else {
                &target
            };
            std::fs::write(replacement, b"later unrelated replacement").unwrap();
            let before = std::fs::read(&sidecar_target).unwrap();
            let mut journal = SqliteJournal::open(&dir.db()).unwrap();
            assert!(
                matches!(Executor::undo_with_mode(&mut journal, batch.id, &roots(dir.path()), mode, &mut |_| {}), Err(PlanError::Stale(path)) if path == *replacement)
            );
            assert_eq!(
                std::fs::read(replacement).unwrap(),
                b"later unrelated replacement"
            );
            assert_eq!(std::fs::read(&sidecar_target).unwrap(), before);
            assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
            assert_eq!(std::fs::read(&sidecar).unwrap(), b"original cue");
            assert!(journal.batch(batch.id).unwrap().unwrap().interrupted);
            // Once both uncertain destinations are absent, Undo is satisfied.
            if target.exists() {
                std::fs::remove_file(&target).unwrap();
            }
            std::fs::remove_file(&sidecar_target).unwrap();
            Executor::undo_with_mode(
                &mut journal,
                batch.id,
                &roots(dir.path()),
                mode,
                &mut |_| {},
            )
            .unwrap();
            assert!(journal.batches().unwrap().is_empty());
        }
    }
}

#[test]
fn completed_legacy_unstamped_copies_refuse_unproved_deletion() {
    for weak_stamp in [false, true] {
        let dir = TempDir::new(if weak_stamp {
            "legacy-weak-copy"
        } else {
            "legacy-copy"
        });
        let source = dir.flac("source.flac");
        let target = dir.path().join("target.flac");
        let plan = ChangePlan {
            changes: vec![FileChange {
                path: source.clone(),
                rename_to: Some(target.clone()),
                copy: true,
                ..FileChange::default()
            }],
            ..ChangePlan::default()
        };
        let mut batch = interrupt_before_finish(&dir, &plan);
        batch.interrupted = false;
        if weak_stamp {
            let mut stamp = CopyStamp::of(&target).unwrap();
            stamp.sha256 = None;
            batch.copy_stamps.push(stamp);
        }
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        journal.finish(batch.id, &batch).unwrap();
        drop(journal);
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        assert!(matches!(
            Executor::undo(&mut journal, batch.id, &roots(dir.path())),
            Err(tagrex_core::plan::PlanError::Stale(_))
        ));
        assert!(source.exists() && target.exists());
        assert!(journal.batch(batch.id).unwrap().is_some());
    }
}

#[test]
fn interrupted_unchanged_copy_requires_observed_stamp() {
    use tagrex_core::plan::{PlanError, UndoMode};
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        for stamp_known in [false, true] {
            let dir = TempDir::new("copy-evidence");
            let source = dir.flac("source.flac");
            let target = dir.path().join("target.flac");
            let plan = ChangePlan {
                changes: vec![FileChange {
                    path: source.clone(),
                    rename_to: Some(target.clone()),
                    copy: true,
                    ..FileChange::default()
                }],
                ..ChangePlan::default()
            };
            let mut batch = interrupt_before_finish(&dir, &plan);
            let mut journal = SqliteJournal::open(&dir.db()).unwrap();
            if stamp_known {
                batch.copy_stamps.push(CopyStamp::of(&target).unwrap());
                journal.retain_interrupted(batch.id, &batch).unwrap();
            }
            drop(journal);
            let mut journal = SqliteJournal::open(&dir.db()).unwrap();
            let result = Executor::undo_with_mode(
                &mut journal,
                batch.id,
                &roots(dir.path()),
                mode,
                &mut |_| {},
            );
            if stamp_known {
                result.unwrap();
            } else {
                assert!(matches!(result, Err(PlanError::Stale(path)) if path == target));
                assert_eq!(std::fs::read(&target).unwrap(), MINIMAL_FLAC);
                assert!(journal.batch(batch.id).unwrap().is_some());
                std::fs::remove_file(&target).unwrap();
                Executor::undo(&mut journal, batch.id, &roots(dir.path())).unwrap();
            }
            assert_eq!(std::fs::read(&source).unwrap(), MINIMAL_FLAC);
            assert!(!target.exists());
            assert!(journal.batches().unwrap().is_empty());
        }
    }
}

#[test]
fn interrupted_missing_field_outcomes_refuse_later_edits_in_every_mode() {
    use tagrex_core::plan::{PlanError, UndoMode};
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        for partial_evidence in [false, true] {
            let dir = TempDir::new("unknown-fields");
            let track = dir.flac("track.flac");
            let untouched = dir.flac("untouched.flac");
            let before_untouched = std::fs::read(&untouched).unwrap();
            let plan = ChangePlan {
                changes: vec![FileChange {
                    path: track.clone(),
                    tag_changes: vec![
                        FieldChange {
                            field: TagField::Artist,
                            old: None,
                            new: Some("Applied".to_string()),
                        },
                        FieldChange {
                            field: TagField::Title,
                            old: None,
                            new: Some("Applied title".to_string()),
                        },
                    ],
                    ..FileChange::default()
                }],
                ..ChangePlan::default()
            };
            let mut batch = interrupt_before_finish(&dir, &plan);
            // Include an unreached entry; refusal must leave it byte-for-byte.
            batch.plan.changes.push(
                set_artist(&untouched, None, Some("Never reached"))
                    .changes
                    .remove(0),
            );
            if partial_evidence {
                batch.applied_values.push(AppliedValue {
                    path: track.clone(),
                    field: TagField::Title,
                    value: Some("Applied title".to_string()),
                });
            }
            let mut file = TagEngine::read(&track).unwrap();
            file.tags
                .insert(TagField::Artist, "Later external edit".to_string());
            TagEngine::write(&file).unwrap();
            let before = std::fs::read(&track).unwrap();
            let mut journal = SqliteJournal::open(&dir.db()).unwrap();
            // record a fixture representing a partially retained execution outcome
            journal.rollback(batch.id).unwrap();
            let id = journal.record(&batch).unwrap();
            drop(journal);
            let mut journal = SqliteJournal::open(&dir.db()).unwrap();
            assert!(
                matches!(Executor::undo_with_mode(&mut journal, id, &roots(dir.path()), mode, &mut |_| {}), Err(PlanError::ChangedSinceApplied(path)) if path == track)
            );
            assert_eq!(std::fs::read(&track).unwrap(), before);
            assert_eq!(std::fs::read(&untouched).unwrap(), before_untouched);
            assert!(journal.batch(id).unwrap().unwrap().interrupted);
            // Restore uncertain fields explicitly, then retry known and unreached fields.
            let mut file = TagEngine::read(&track).unwrap();
            file.tags.remove(&TagField::Artist);
            if !partial_evidence {
                file.tags.remove(&TagField::Title);
            }
            TagEngine::write(&file).unwrap();
            Executor::undo_with_mode(&mut journal, id, &roots(dir.path()), mode, &mut |_| {})
                .unwrap();
            assert!(!TagEngine::read(&track)
                .unwrap()
                .tags
                .contains_key(&TagField::Title));
            assert_eq!(std::fs::read(&untouched).unwrap(), before_untouched);
            assert!(journal.batches().unwrap().is_empty());
        }
    }
}

#[test]
fn interrupted_pruning_restores_authorized_source_parents_and_sidecars() {
    for stop_before_transfer in [false, true] {
        let dir = TempDir::new("prune-recovery");
        let parent = dir.path().join("artist/album");
        let scans = parent.join("Scans");
        std::fs::create_dir_all(&scans).unwrap();
        let source = parent.join("track.flac");
        std::fs::write(&source, MINIMAL_FLAC).unwrap();
        let sidecar = scans.join("cover.jpg");
        std::fs::write(&sidecar, b"disposable scan").unwrap();
        let target = dir.path().join("new/track.flac");
        let sidecar_target = dir.path().join("new/cover.jpg");
        let plan = ChangePlan {
            prune_empty_dirs: true,
            changes: vec![FileChange {
                path: source.clone(),
                rename_to: Some(target.clone()),
                sidecar_renames: vec![(sidecar.clone(), sidecar_target.clone())],
                ..FileChange::default()
            }],
            ..ChangePlan::default()
        };
        let batch = if stop_before_transfer {
            exit_child_at(&dir, &plan, Some(1));
            SqliteJournal::open(&dir.db())
                .unwrap()
                .batches()
                .unwrap()
                .remove(0)
        } else {
            let batch = interrupt_before_finish(&dir, &plan);
            assert!(
                !parent.exists(),
                "pruning really ran before simulated termination"
            );
            batch
        };
        assert!(batch.removed_dirs.contains(&parent));
        assert!(batch.removed_dirs.contains(&scans));
        assert!(!batch.removed_dirs.contains(&dir.path().to_path_buf()));
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        Executor::undo(&mut journal, batch.id, &roots(dir.path())).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), MINIMAL_FLAC);
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"disposable scan");
        assert!(!target.exists() && !sidecar_target.exists());
        assert!(journal.batches().unwrap().is_empty());
    }
}

fn recovery_cover(byte: u8, description: &str) -> tagrex_core::model::CoverArt {
    tagrex_core::model::CoverArt {
        mime: "image/png".to_string(),
        data: vec![0x89, 0x50, 0x4e, 0x47, byte].into(),
        description: description.to_string(),
        ..tagrex_core::model::CoverArt::default()
    }
}

#[test]
fn later_cover_sets_survive_undo_refusal_and_allow_retry() {
    use tagrex_core::model::{CoverArt, CoverKind};
    use tagrex_core::plan::{CoverChange, PlanError, UndoMode};
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        for interrupted in [false, true] {
            for edit in 0..4 {
                let dir = TempDir::new("cover-conflict");
                let parent = dir.path().join("old");
                std::fs::create_dir_all(&parent).unwrap();
                let track = parent.join("track.flac");
                std::fs::write(&track, MINIMAL_FLAC).unwrap();
                let moved = dir.path().join("moved.flac");
                let old = vec![recovery_cover(1, "Original")];
                let new = vec![
                    recovery_cover(2, "Front"),
                    CoverArt {
                        kind: CoverKind::Back,
                        ..recovery_cover(3, "Back")
                    },
                ];
                TagEngine::write_covers(&track, &old).unwrap();
                let plan = ChangePlan {
                    prune_empty_dirs: true,
                    changes: vec![FileChange {
                        path: track.clone(),
                        rename_to: Some(moved.clone()),
                        cover_change: Some(CoverChange {
                            old: old.clone(),
                            new: new.clone(),
                        }),
                        ..FileChange::default()
                    }],
                    ..ChangePlan::default()
                };
                let batch = if interrupted {
                    interrupt_before_finish(&dir, &plan)
                } else {
                    Executor::apply(
                        &plan,
                        &mut SqliteJournal::open(&dir.db()).unwrap(),
                        &roots(dir.path()),
                    )
                    .unwrap()
                };
                let mut later = new.clone();
                match edit {
                    0 => later[0] = recovery_cover(9, "Front"),
                    1 => later[0].description = "Later description".to_string(),
                    2 => later[0].kind = CoverKind::Back,
                    _ => later.reverse(),
                }
                TagEngine::write_covers(&moved, &later).unwrap();
                let before = std::fs::read(&moved).unwrap();
                let mut journal = SqliteJournal::open(&dir.db()).unwrap();
                assert!(
                    matches!(Executor::undo_with_mode(&mut journal, batch.id, &roots(dir.path()), mode, &mut |_| {}), Err(PlanError::ChangedSinceApplied(path)) if path == moved)
                );
                assert_eq!(std::fs::read(&moved).unwrap(), before);
                assert_eq!(TagEngine::read_covers(&moved).unwrap(), later);
                assert!(!track.exists());
                assert!(
                    !parent.exists(),
                    "refusal must not recreate pruned directories"
                );
                assert!(journal.batch(batch.id).unwrap().is_some());
                // A completed batch accepts its applied state; unknown execution
                // accepts only an explicitly restored state.
                TagEngine::write_covers(&moved, if interrupted { &old } else { &new }).unwrap();
                Executor::undo_with_mode(
                    &mut journal,
                    batch.id,
                    &roots(dir.path()),
                    mode,
                    &mut |_| {},
                )
                .unwrap();
                assert_eq!(TagEngine::read_covers(&track).unwrap(), old);
                assert!(!moved.exists());
                assert!(journal.batches().unwrap().is_empty());
            }
        }
    }
}

#[test]
fn interrupted_field_recovery_at_each_tag_write_boundary() {
    use tagrex_core::plan::PlanError;
    for completed_files in 0..=2 {
        let dir = TempDir::new("tag-write-boundary");
        let tracks = [dir.flac("a.flac"), dir.flac("b.flac")];
        let before: Vec<_> = tracks
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect();
        let plan = ChangePlan {
            changes: tracks
                .iter()
                .map(|path| set_artist(path, None, Some("Applied")).changes.remove(0))
                .collect(),
            ..ChangePlan::default()
        };
        exit_child_at(&dir, &plan, Some(2 + completed_files));
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        let id = journal.batches().unwrap()[0].id;
        let after: Vec<_> = tracks
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect();
        let result = Executor::undo(&mut journal, id, &roots(dir.path()));
        if completed_files == 0 {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(PlanError::ChangedSinceApplied(_))));
            for (index, path) in tracks.iter().enumerate() {
                assert_eq!(std::fs::read(path).unwrap(), after[index]);
            }
            assert!(journal.batch(id).unwrap().is_some());
            for path in tracks.iter().take(completed_files) {
                let mut file = TagEngine::read(path).unwrap();
                file.tags.remove(&TagField::Artist);
                TagEngine::write(&file).unwrap();
            }
            Executor::undo(&mut journal, id, &roots(dir.path())).unwrap();
        }
        for (index, path) in tracks.iter().enumerate().skip(completed_files) {
            assert_eq!(
                std::fs::read(path).unwrap(),
                before[index],
                "unreached file was not rewritten"
            );
        }
        assert!(journal.batches().unwrap().is_empty());
    }
}

#[test]
fn interrupted_move_recovers_after_each_pruned_parent() {
    for pruned in 0..=3 {
        let dir = TempDir::new("pruning-boundary");
        let parents = [
            dir.path().join("artist/album/Scans"),
            dir.path().join("artist/album"),
            dir.path().join("artist"),
        ];
        std::fs::create_dir_all(&parents[0]).unwrap();
        let source = parents[1].join("track.flac");
        let sidecar = parents[0].join("cover.jpg");
        std::fs::write(&source, MINIMAL_FLAC).unwrap();
        std::fs::write(&sidecar, b"disposable scan").unwrap();
        let target = dir.path().join("new/track.flac");
        let sidecar_target = dir.path().join("new/cover.jpg");
        let plan = ChangePlan {
            prune_empty_dirs: true,
            changes: vec![FileChange {
                path: source.clone(),
                rename_to: Some(target.clone()),
                sidecar_renames: vec![(sidecar.clone(), sidecar_target.clone())],
                ..FileChange::default()
            }],
            ..ChangePlan::default()
        };
        exit_child_at(&dir, &plan, Some(1));
        // Deterministic filesystem states at the three deletion boundaries,
        // backed by the real production write-ahead record.
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::rename(&source, &target).unwrap();
        std::fs::rename(&sidecar, &sidecar_target).unwrap();
        for parent in parents.iter().take(pruned) {
            std::fs::remove_dir(parent).unwrap();
        }
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        let id = journal.batches().unwrap()[0].id;
        Executor::undo(&mut journal, id, &roots(dir.path())).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), MINIMAL_FLAC);
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"disposable scan");
        assert!(!target.exists() && !sidecar_target.exists());
        assert!(journal.batches().unwrap().is_empty());
    }
}

#[test]
fn failed_apply_does_not_invent_outcomes_for_unreached_external_edits() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tagrex_core::plan::{PlanError, UndoMode};
    let dir = TempDir::new("unreached-external-edit");
    let reached = dir.flac("reached.flac");
    let saved = dir.path().join("saved.flac");
    let untouched = dir.flac("untouched.flac");
    let plan = ChangePlan {
        changes: vec![
            set_artist(&reached, None, Some("Applied"))
                .changes
                .remove(0),
            set_artist(&untouched, None, Some("Never reached"))
                .changes
                .remove(0),
        ],
        ..ChangePlan::default()
    };
    let cancel = AtomicBool::new(false);
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    let error = Executor::apply_with(
        &plan,
        &mut journal,
        &roots(dir.path()),
        &mut |progress| {
            if progress.completed.is_some() {
                // Block rollback of the reached file, edit the unreached one, then
                // cancel before its write begins. Everything is a disposable fixture.
                std::fs::rename(&reached, &saved).unwrap();
                std::fs::create_dir(&reached).unwrap();
                let mut file = TagEngine::read(&untouched).unwrap();
                file.tags
                    .insert(TagField::Artist, "Later external edit".to_string());
                TagEngine::write(&file).unwrap();
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert!(matches!(error, PlanError::RollbackIncomplete { .. }));
    drop(journal);
    std::fs::remove_dir(&reached).unwrap();
    std::fs::rename(&saved, &reached).unwrap();
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    let batch = journal.batches().unwrap().remove(0);
    assert!(batch.interrupted);
    assert!(batch
        .applied_values
        .iter()
        .any(|value| value.path == reached));
    assert!(batch
        .applied_values
        .iter()
        .all(|value| value.path != untouched));
    let reached_bytes = std::fs::read(&reached).unwrap();
    let untouched_bytes = std::fs::read(&untouched).unwrap();
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        assert!(
            matches!(Executor::undo_with_mode(&mut journal, batch.id, &roots(dir.path()), mode, &mut |_| {}),
            Err(PlanError::ChangedSinceApplied(path)) if path == untouched)
        );
        assert_eq!(std::fs::read(&reached).unwrap(), reached_bytes);
        assert_eq!(std::fs::read(&untouched).unwrap(), untouched_bytes);
        assert!(journal.batch(batch.id).unwrap().is_some());
    }
    let mut file = TagEngine::read(&untouched).unwrap();
    file.tags.remove(&TagField::Artist);
    TagEngine::write(&file).unwrap();
    let restored_bytes = std::fs::read(&untouched).unwrap();
    Executor::undo(&mut journal, batch.id, &roots(dir.path())).unwrap();
    assert!(!TagEngine::read(&reached)
        .unwrap()
        .tags
        .contains_key(&TagField::Artist));
    assert_eq!(std::fs::read(&untouched).unwrap(), restored_bytes);
    assert!(journal.batches().unwrap().is_empty());
}

fn whole_block_plan(path: &Path) -> ChangePlan {
    use tagrex_core::model::TagBlockKind;
    let kind = TagBlockKind::VorbisComments;
    let mut block = tagrex_core::plan::BlockChange::removal(
        kind,
        TagEngine::read_block(path, kind).unwrap().unwrap(),
    );
    block.old_bytes = TagEngine::dump_vorbis(path).unwrap();
    ChangePlan {
        changes: vec![FileChange {
            path: path.to_path_buf(),
            block_changes: vec![block],
            ..FileChange::default()
        }],
        ..ChangePlan::default()
    }
}

#[test]
fn whole_block_undo_fingerprints_survive_restart_and_protect_later_metadata() {
    use tagrex_core::plan::{PlanError, UndoMode};
    for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
        let dir = TempDir::new("block-fingerprint");
        let path = dir.flac("track.flac");
        let mut file = TagEngine::read(&path).unwrap();
        file.tags.insert(TagField::Artist, "Before".into());
        TagEngine::write(&file).unwrap();
        let plan = whole_block_plan(&path);
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        let batch = Executor::apply(&plan, &mut journal, &roots(dir.path())).unwrap();
        assert_eq!(batch.block_fingerprints.len(), 1);
        assert_eq!(batch.block_fingerprints[0].before.len(), 32);
        assert_eq!(
            batch.block_fingerprints[0].after.as_ref().unwrap().len(),
            32
        );
        let applied_bytes = std::fs::read(&path).unwrap();
        drop(journal);
        let mut journal = SqliteJournal::open(&dir.db()).unwrap();
        assert_eq!(
            journal.batch(batch.id).unwrap().unwrap().block_fingerprints,
            batch.block_fingerprints
        );
        let mut later = TagEngine::read(&path).unwrap();
        later
            .tags
            .insert(TagField::Custom("EXTERNAL_CUE".into()), "9,9,9".into());
        TagEngine::write(&later).unwrap();
        let protected_bytes = std::fs::read(&path).unwrap();
        assert!(matches!(
            Executor::undo_with_mode(
                &mut journal,
                batch.id,
                &roots(dir.path()),
                mode,
                &mut |_| {}
            ),
            Err(PlanError::ChangedSinceApplied(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), protected_bytes);
        assert!(journal.batch(batch.id).unwrap().is_some());
        std::fs::write(&path, &applied_bytes).unwrap();
        Executor::undo(&mut journal, batch.id, &roots(dir.path())).unwrap();
        assert_eq!(
            TagEngine::read(&path)
                .unwrap()
                .tags
                .get(&TagField::Artist)
                .map(String::as_str),
            Some("Before")
        );
    }
}

#[test]
fn interrupted_whole_block_write_refuses_unknown_outcome_and_preserves_exact_restored_bytes() {
    use tagrex_core::plan::PlanError;
    let dir = TempDir::new("block-unknown-outcome");
    let path = dir.flac("track.flac");
    let mut file = TagEngine::read(&path).unwrap();
    file.tags.insert(TagField::Artist, "Before".into());
    TagEngine::write(&file).unwrap();
    let original = std::fs::read(&path).unwrap();
    let batch = interrupt_before_finish(&dir, &whole_block_plan(&path));
    assert!(batch.block_fingerprints[0].after.is_none());
    let applied = std::fs::read(&path).unwrap();
    let mut journal = SqliteJournal::open(&dir.db()).unwrap();
    assert!(matches!(
        Executor::undo(&mut journal, batch.id, &roots(dir.path())),
        Err(PlanError::ChangedSinceApplied(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), applied);
    assert!(journal.batch(batch.id).unwrap().is_some());
    std::fs::write(&path, &original).unwrap();
    Executor::undo(&mut journal, batch.id, &roots(dir.path())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
