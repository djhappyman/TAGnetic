//! Fresh review canaries using public native entry points and disposable media.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
#[path = "../src/bounded_cache.rs"]
mod derived_cache;
use tagrex_commands::{
    with_preview_cancellation, App, ColumnMapping, EmptyCell, ImportColumn, TabularImportError,
    TabularImportOptions, TextImportMode,
};
use tagrex_core::{
    model::{TagEngine, TagField},
    resources::{AdmissionError, Budget, Charge},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-fresh-resource-import-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn wav(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend(100u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(16000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(64u32.to_le_bytes());
        bytes.extend([0; 64]);
        std::fs::write(&path, bytes).unwrap();
        let mut track = TagEngine::read(&path).unwrap();
        track.tags.insert(TagField::Title, "Saved title".into());
        track.tags.insert(TagField::Artist, "Saved artist".into());
        TagEngine::write(&track).unwrap();
        path
    }
    fn app(&self) -> App {
        App::open(&self.0, &self.0.join("journal.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn mapping(name: &str, field: &str, empty: EmptyCell) -> ColumnMapping {
    ColumnMapping {
        column: ImportColumn::Header { name: name.into() },
        field: field.into(),
        empty,
    }
}
fn table() -> TabularImportOptions {
    TabularImportOptions {
        mappings: vec![mapping("Title", "title", EmptyCell::Skip)],
        ..Default::default()
    }
}
fn isolated(name: &str, job: impl FnOnce()) {
    if std::env::var("TAGREX_FRESH_RECHECK_CHILD").as_deref() == Ok(name) {
        job();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_FRESH_RECHECK_CHILD", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cancelled_rename_preview_cannot_publish_a_shortened_native_plan() {
    let f = Fixture::new("rename");
    let path = f.wav("before.wav");
    let app = f.app();
    let before = std::fs::read(&path).unwrap();
    let result = with_preview_cancellation(
        || true,
        || app.preview_rename("After", std::slice::from_ref(&path)),
    );
    assert!(
        result.is_err(),
        "rename swallowed cancelled metadata reads and returned an accepted plan"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(app
        .preview_rename("After", std::slice::from_ref(&path))
        .is_ok());
}

#[test]
fn cancelled_move_preview_cannot_publish_a_shortened_native_plan() {
    let f = Fixture::new("move");
    let path = f.wav("before.wav");
    let app = f.app();
    let destination = f.0.join("destination");
    std::fs::create_dir(&destination).unwrap();
    let before = std::fs::read(&path).unwrap();
    let result = with_preview_cancellation(
        || true,
        || {
            app.preview_move(
                "After",
                std::slice::from_ref(&path),
                Some(&destination),
                false,
                false,
            )
        },
    );
    assert!(
        result.is_err(),
        "move swallowed cancelled metadata reads and returned an accepted plan"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(std::fs::read_dir(destination).unwrap().next().is_none());
}

#[test]
fn tabular_busy_refusal_releases_all_transient_admission_and_retry_keeps_values() {
    isolated(
        "tabular_busy_refusal_releases_all_transient_admission_and_retry_keeps_values",
        || {
            let f = Fixture::new("busy");
            let path = f.wav("before.wav");
            let app = f.app();
            let budget = Budget::process();
            let baseline = budget.usage().0;
            let pressure = budget
                .try_acquire(Charge {
                    jobs: 8,
                    ..Default::default()
                })
                .unwrap();
            let error = app
                .preview_tabular_import(
                    std::slice::from_ref(&path),
                    b"Title\nNew title",
                    &table(),
                    None,
                )
                .unwrap_err();
            assert!(matches!(
                &error,
                TabularImportError::Resource(AdmissionError::Busy)
            ));
            let dto: tagrex_commands::ErrorDto = error.into();
            assert_eq!(dto.message.code, "error.resourceBusy");
            assert_eq!(budget.usage().0.transient, baseline.transient);
            drop(pressure);
            let preview = app
                .preview_tabular_import(
                    std::slice::from_ref(&path),
                    b"Title\nNew title",
                    &table(),
                    None,
                )
                .unwrap();
            assert_eq!(
                preview.plan.changes[0].tag_changes[0].new.as_deref(),
                Some("New title")
            );
            assert_eq!(budget.usage().0.transient, baseline.transient);
            assert_eq!(budget.usage().0.jobs, baseline.jobs);
            assert_eq!(
                TagEngine::read(&path).unwrap().tags[&TagField::Title],
                "Saved title"
            );
            drop(app);
            assert_eq!(budget.usage().0, baseline);
        },
    );
}

#[test]
fn alternate_dialect_and_locked_clear_keep_other_explicit_values() {
    let f = Fixture::new("dialect");
    let path = f.wav("before.wav");
    let app = f.app();
    app.set_locked_fields(&["artist".into()]);
    let options = TabularImportOptions {
        delimiter: ';',
        quote: '\'',
        mappings: vec![
            mapping("Title", "title", EmptyCell::Skip),
            mapping("Artist", "artist", EmptyCell::Clear),
        ],
        ..Default::default()
    };
    let preview = app
        .preview_tabular_import(
            std::slice::from_ref(&path),
            b"Title;Artist\n'Title; with ''quotes''';''",
            &options,
            None,
        )
        .unwrap();
    let changes = &preview.plan.changes[0].tag_changes;
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].field, "title");
    assert_eq!(changes[0].new.as_deref(), Some("Title; with 'quotes'"));
    assert!(!preview.plan.locked_skipped.is_empty());
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Artist],
        "Saved artist"
    );
}

#[test]
fn mapped_header_only_bad_index_is_refused_without_media_reads() {
    let f = Fixture::new("index");
    let app = f.app();
    let options = TabularImportOptions {
        mappings: vec![ColumnMapping {
            column: ImportColumn::Index { index: 100 },
            field: "title".into(),
            empty: EmptyCell::Skip,
        }],
        ..Default::default()
    };
    assert!(app
        .preview_tabular_import(&[], b"Title", &options, None)
        .is_err());
}

#[test]
fn duplicate_filename_matching_rejects_alias_ambiguity_before_reading() {
    let f = Fixture::new("ambiguous");
    let app = f.app();
    let options = TabularImportOptions {
        mode: TextImportMode::Filename,
        match_column: Some(ImportColumn::Header {
            name: "File".into(),
        }),
        ..table()
    };
    let selected = [f.0.join("a/track.wav"), f.0.join("b/track.wav")];
    assert!(app
        .preview_tabular_import(
            &selected,
            b"File,Title\ntrack.wav,New title",
            &options,
            None
        )
        .is_err());
}

#[test]
fn cancellation_scope_never_survives_a_panicking_worker_job() {
    let f = Fixture::new("unwind");
    let path = f.wav("before.wav");
    let app = f.app();
    let result = std::panic::catch_unwind(|| {
        with_preview_cancellation(|| true, || panic!("fresh unwind canary"))
    });
    assert!(result.is_err());
    assert!(app.read_track_rows(std::slice::from_ref(&path)).is_ok());
    let handle = std::thread::spawn(move || {
        let isolated = Fixture::new("thread");
        let other = isolated.wav("other.wav");
        isolated
            .app()
            .read_track_rows(std::slice::from_ref(&other))
            .is_ok()
    });
    assert!(with_preview_cancellation(
        || true,
        || handle.join().unwrap()
    ));
}

#[test]
fn rejected_global_cache_admission_preserves_existing_hot_entry() {
    isolated(
        "rejected_global_cache_admission_preserves_existing_hot_entry",
        || {
            let budget = Budget::process();
            let baseline = budget.usage().0;
            assert_eq!(baseline.retained, 0);
            let mut cache = derived_cache::Cache::new(2, 10);
            assert_eq!(cache.limits(), (2, 10));
            cache.insert(1, "hot", 8);
            assert_eq!(cache.get(&1), Some(&"hot"));
            // Fill the global retained allowance with another owner. Local LRU
            // eviction could free eight bytes, but the nine-byte newcomer still
            // cannot fit globally; a failed optional admission must retain useful
            // existing data rather than evict it for no replacement.
            let pressure = budget
                .try_acquire(Charge {
                    retained: 256 * 1024 * 1024 - 8,
                    ..Default::default()
                })
                .unwrap();
            cache.insert(2, "cannot fit", 9);
            assert!(cache.get(&2).is_none());
            assert_eq!(
                cache.get(&1),
                Some(&"hot"),
                "global refusal discarded the useful old entry before admission was secured"
            );
            drop(pressure);
            drop(cache);
            assert_eq!(budget.usage().0, baseline);
        },
    );
}
