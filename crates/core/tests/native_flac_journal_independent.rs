#![cfg(target_os = "linux")]
//! Independent durable native-journal corruption and authority probes.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tagrex_core::{
    journal::{AppliedBatch, SqliteJournal, UndoJournal, VecJournal},
    model::{TagEngine, TagField},
    plan::{ChangePlan, Executor, FieldChange, FileChange, UndoMode},
};
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct Fixture {
    root: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-native-journal-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        for i in 0..2 {
            fs::write(root.join(format!("{i}.flac")), flac(&format!("Saved {i}"))).unwrap();
        }
        Self {
            root,
            _guard: guard,
        }
    }
    fn db(&self) -> PathBuf {
        self.root.join("journal.sqlite")
    }
    fn media(&self, i: usize) -> PathBuf {
        self.root.join(format!("{i}.flac"))
    }
    fn store(&self) -> PathBuf {
        self.root.join("journal.sqlite.native-media")
    }
    fn apply(&self) -> AppliedBatch {
        let p = ChangePlan {
            changes: (0..2)
                .map(|i| FileChange {
                    path: self.media(i),
                    tag_changes: vec![FieldChange {
                        field: TagField::Title,
                        old: Some(format!("Saved {i}")),
                        new: Some(format!("Applied {i}")),
                    }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut j = SqliteJournal::open(&self.db()).unwrap();
        let b = Executor::apply(&p, &mut j, std::slice::from_ref(&self.root)).unwrap();
        assert!(b.native_tags);
        b
    }
    fn connection(&self) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(self.db()).unwrap();
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn
    }
    fn bytes(&self) -> [Vec<u8>; 2] {
        [
            fs::read(self.media(0)).unwrap(),
            fs::read(self.media(1)).unwrap(),
        ]
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn flac(title: &str) -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0u8; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut c = 4u32.to_le_bytes().to_vec();
    c.extend(b"test");
    let items = [
        format!("TITLE={title}"),
        "ARTIST=A; B".into(),
        "ARTIST=".into(),
        "GENRE=House".into(),
        "GENRE=house".into(),
        "CUSTOM=尾".into(),
    ];
    c.extend((items.len() as u32).to_le_bytes());
    for item in items {
        c.extend((item.len() as u32).to_le_bytes());
        c.extend(item.as_bytes());
    }
    let n = c.len();
    out.extend([4, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(c);
    out.extend([0x82, 0, 0, 8]);
    out.extend(b"TEST\0\xff\x01\x02");
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out
}
fn inventory(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, at: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(at).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                visit(root, &p, out)
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(p).unwrap(),
                ))
            }
        }
    }
    if !root.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
fn rows(conn: &rusqlite::Connection) -> Vec<String> {
    let mut names = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let names: Vec<String> = names
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut out = Vec::new();
    for name in names {
        let mut s = conn
            .prepare(&format!(
                "SELECT * FROM \"{}\" ORDER BY rowid",
                name.replace('"', "\"\"")
            ))
            .unwrap();
        let n = s.column_count();
        let mut rs = s.query([]).unwrap();
        while let Some(r) = rs.next().unwrap() {
            let values: Vec<String> = (0..n)
                .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                .collect();
            out.push(format!("{name}:{values:?}"));
        }
    }
    out
}
fn assert_no_effect_undo(f: &Fixture, b: &AppliedBatch, mode: UndoMode) {
    let media = f.bytes();
    let artifacts = inventory(&f.store());
    let before = rows(&f.connection());
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let mut callbacks = 0;
    let result = Executor::undo_with_mode(
        &mut j,
        b.id,
        std::slice::from_ref(&f.root),
        mode,
        &mut |_| callbacks += 1,
    );
    assert!(
        result.is_err(),
        "unexpected accepted malformed native journal"
    );
    assert_eq!(callbacks, 0);
    assert_eq!(f.bytes(), media);
    assert_eq!(inventory(&f.store()), artifacts);
    assert_eq!(rows(&f.connection()), before);
}
#[test]
fn later_native_row_corruptions_refuse_before_any_media_history_or_store_effect() {
    let cases=[
        "UPDATE native_tag_files SET identity=zeroblob(32) WHERE ordinal=1",
        "UPDATE native_tag_files SET roots_digest=zeroblob(32) WHERE ordinal=1",
        "UPDATE native_tag_files SET digest=zeroblob(32) WHERE ordinal=1",
        "UPDATE native_tag_files SET operation=3 WHERE ordinal=1",
        "UPDATE native_tag_files SET operation=256 WHERE ordinal=1",
        "UPDATE native_tag_files SET ordinal=-1 WHERE ordinal=1",
        "UPDATE native_tag_files SET operation_id='m-not-valid' WHERE ordinal=1",
        "UPDATE native_tag_files SET record=zeroblob(8193) WHERE ordinal=1",
        "UPDATE native_tag_files SET receipts=NULL WHERE ordinal=1",
        "UPDATE native_tag_files SET intents=(SELECT intents FROM native_tag_files WHERE ordinal=0) WHERE ordinal=1",
        "UPDATE native_tag_files SET receipts=(SELECT receipts FROM native_tag_files WHERE ordinal=0) WHERE ordinal=1",
        "UPDATE native_tag_files SET recovery=(SELECT receipts FROM native_tag_files WHERE ordinal=0) WHERE ordinal=1",
    ];
    for sql in cases {
        for mode in [UndoMode::Strict, UndoMode::KeepChanged, UndoMode::Overwrite] {
            let f = Fixture::new();
            let b = f.apply();
            f.connection().execute_batch(sql).unwrap();
            eprintln!("CORRUPTION {sql} mode={mode:?}");
            assert_no_effect_undo(&f, &b, mode);
        }
    }
}
#[test]
fn native_marker_and_owner_joins_refuse_mixed_missing_and_cross_file_collections() {
    let cases=[
        "UPDATE file_changes SET path=substr(path,17) WHERE id=(SELECT max(id) FROM file_changes)",
        "DELETE FROM native_tag_files WHERE ordinal=1",
        "UPDATE native_tag_files SET batch_id=999 WHERE ordinal=1",
        "UPDATE native_tag_files SET file_change_id=999 WHERE ordinal=1",
        "UPDATE file_changes SET copied=1 WHERE id=(SELECT max(id) FROM file_changes)",
        "UPDATE file_changes SET rename_to='unexpected.flac' WHERE id=(SELECT max(id) FROM file_changes)",
        "UPDATE file_changes SET path=CAST(path AS TEXT) WHERE id=(SELECT max(id) FROM file_changes)",
        "UPDATE file_changes SET path=x'5452582d4e41544956452d5041544802' WHERE id=(SELECT max(id) FROM file_changes)",
    ];
    for sql in cases {
        let f = Fixture::new();
        let b = f.apply();
        f.connection().execute_batch(sql).unwrap();
        assert_no_effect_undo(&f, &b, UndoMode::Overwrite);
    }
}
#[test]
fn future_and_unrecognized_schema_open_is_read_only_and_never_quarantines_history() {
    for sql in [
        "PRAGMA user_version=3",
        "UPDATE native_tag_schema SET signature='future-native-v2'",
        "DROP TABLE native_tag_schema",
    ] {
        let f = Fixture::new();
        let _b = f.apply();
        f.connection().execute_batch(sql).unwrap();
        let before = fs::read(f.db()).unwrap();
        let media = f.bytes();
        let artifacts = inventory(&f.store());
        assert!(SqliteJournal::open_or_recover(&f.db()).is_err());
        assert_eq!(fs::read(f.db()).unwrap(), before);
        assert_eq!(f.bytes(), media);
        assert_eq!(inventory(&f.store()), artifacts);
        assert!(!fs::read_dir(&f.root).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("corrupt")));
    }
}
#[test]
fn marker_path_boundaries_reject_traversal_nul_relative_and_future_encoding() {
    for suffix in [
        b"relative.flac".as_slice(),
        b"/tmp/../escape.flac",
        b"/tmp/name\0.flac",
        b"",
    ] {
        let f = Fixture::new();
        let b = f.apply();
        let mut path = b"TRX-NATIVE-PATH\x01".to_vec();
        path.extend(suffix);
        f.connection()
            .execute(
                "UPDATE file_changes SET path=?1 WHERE id=(SELECT max(id) FROM file_changes)",
                [path],
            )
            .unwrap();
        let mut j = SqliteJournal::open(&f.db()).unwrap();
        assert!(j.headers(std::slice::from_ref(&f.root)).is_err());
        let before = f.bytes();
        assert!(Executor::undo(&mut j, b.id, std::slice::from_ref(&f.root)).is_err());
        assert_eq!(f.bytes(), before);
    }
}
#[test]
fn memory_adapter_cannot_reinterpret_loaded_native_history_as_scalar_undo() {
    let f = Fixture::new();
    let b = f.apply();
    let mut mem = VecJournal::default();
    let id = mem.record(&b).unwrap();
    let before = f.bytes();
    assert!(Executor::undo(&mut mem, id, std::slice::from_ref(&f.root)).is_err());
    assert_eq!(f.bytes(), before);
    assert!(mem.batch(id).unwrap().is_some());
}
#[test]
fn legacy_scalar_history_stays_version_one_and_native_history_survives_reopen() {
    let f = Fixture::new();
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    assert_eq!(
        f.connection()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let p = ChangePlan {
        changes: vec![FileChange {
            path: f.media(0),
            tag_changes: vec![FieldChange {
                field: TagField::Year,
                old: None,
                new: Some("1999".into()),
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let b = Executor::apply(&p, &mut j, std::slice::from_ref(&f.root)).unwrap();
    assert!(!b.native_tags);
    assert_eq!(
        f.connection()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    Executor::undo(&mut j, b.id, std::slice::from_ref(&f.root)).unwrap();
    drop(j);
    let before = f.bytes();
    let b = f.apply();
    assert_eq!(
        f.connection()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    assert_eq!(
        j.headers(std::slice::from_ref(&f.root)).unwrap()[0]
            .paths
            .len(),
        2
    );
    Executor::undo(&mut j, b.id, std::slice::from_ref(&f.root)).unwrap();
    assert_eq!(f.bytes(), before);
    assert_eq!(
        TagEngine::read(&f.media(1)).unwrap().tags[&TagField::Title],
        "Saved 1"
    );
}
#[test]
fn final_completion_callback_cannot_publish_success_with_known_broken_undo_receipts() {
    let f = Fixture::new();
    let p = ChangePlan {
        changes: (0..2)
            .map(|i| FileChange {
                path: f.media(i),
                tag_changes: vec![FieldChange {
                    field: TagField::Title,
                    old: Some(format!("Saved {i}")),
                    new: Some(format!("Applied {i}")),
                }],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let mut j = SqliteJournal::open(&f.db()).unwrap();
    let mut changed = false;
    let result = Executor::apply_with(
        &p,
        &mut j,
        std::slice::from_ref(&f.root),
        &mut |progress| {
            if progress.completed == Some(tagrex_core::plan::ApplyStep::Tags)
                && progress.path == f.media(1)
            {
                changed = true;
                f.connection()
                    .execute(
                        "UPDATE native_tag_files SET receipts=NULL WHERE ordinal=1",
                        [],
                    )
                    .unwrap();
            }
        },
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert!(changed, "the final completion boundary must actually occur");
    assert!(
        result.is_err(),
        "Apply reported success after final callback removed durable Undo proof"
    );
    let interrupted: i64 = f
        .connection()
        .query_row(
            "SELECT interrupted FROM batches ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        interrupted, 1,
        "known uncertain completion must retain interrupted history"
    );
}
#[test]
fn legacy_version_cannot_bypass_unknown_native_schema_with_existing_native_history() {
    for version in [0, 1] {
        let f = Fixture::new();
        let _b = f.apply();
        f.connection().execute_batch(&format!("UPDATE native_tag_schema SET signature='future-native-format'; PRAGMA user_version={version};")).unwrap();
        let db = fs::read(f.db()).unwrap();
        let media = f.bytes();
        let artifacts = inventory(&f.store());
        assert!(
            SqliteJournal::open(&f.db()).is_err(),
            "legacy version {version} bypassed unknown native schema"
        );
        assert_eq!(fs::read(f.db()).unwrap(), db);
        assert_eq!(f.bytes(), media);
        assert_eq!(inventory(&f.store()), artifacts);
    }
}
#[test]
fn missing_last_plan_owner_cannot_shorten_the_native_batch_and_complete_partial_undo() {
    let f = Fixture::new();
    let b = f.apply();
    let conn = f.connection();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    conn.execute("DELETE FROM file_changes WHERE batch_id=?1 AND id=(SELECT max(id) FROM file_changes WHERE batch_id=?1)",[b.id.0]).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM native_tag_files WHERE batch_id=?1",
            [b.id.0],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let media = f.bytes();
    let artifacts = inventory(&f.store());
    let rows_before = rows(&conn);
    let mut journal = SqliteJournal::open(&f.db()).unwrap();
    let mut callbacks = 0;
    let result = Executor::undo_with_mode(
        &mut journal,
        b.id,
        std::slice::from_ref(&f.root),
        UndoMode::Strict,
        &mut |_| callbacks += 1,
    );
    if result.is_ok() {
        let after = f.bytes();
        let count: i64 = f
            .connection()
            .query_row("SELECT count(*) FROM batches WHERE id=?1", [b.id.0], |r| {
                r.get(0)
            })
            .unwrap();
        eprintln!(
            "MISSING_OWNER first_restored={} final_still_applied={} history_rows={count}",
            after[0]
                .windows(b"TITLE=Saved 0".len())
                .any(|w| w == b"TITLE=Saved 0"),
            after[1]
                .windows(b"TITLE=Applied 1".len())
                .any(|w| w == b"TITLE=Applied 1")
        );
    }
    assert!(
        result.is_err(),
        "missing final owner was accepted as a shorter complete native plan"
    );
    assert_eq!(callbacks, 0);
    assert_eq!(f.bytes(), media);
    assert_eq!(inventory(&f.store()), artifacts);
    assert_eq!(rows(&f.connection()), rows_before);
}
