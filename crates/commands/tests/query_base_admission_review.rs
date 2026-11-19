//! Independent base-index admission and native-session compatibility probes.
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tagrex_commands::{
    query::{CustomField, QueryIndex, QueryRequest, QueryRow},
    with_preview_cancellation, App, TrackDto,
};
use tagrex_core::resources::{Budget, Charge, Limits};

// Observational allocator: delegates pointers/layouts unchanged to System.
// Only the isolated high-cardinality probe reads these counters.
struct AllocationObserver;
static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PEAK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn grew(bytes: usize) {
    let now = LIVE
        .fetch_add(bytes, Ordering::Relaxed)
        .saturating_add(bytes);
    PEAK.fetch_max(now, Ordering::Relaxed);
}
// SAFETY: every allocation/deallocation is performed by System with the
// original layout; the counters never dereference or modify allocation data.
unsafe impl std::alloc::GlobalAlloc for AllocationObserver {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::GlobalAlloc::alloc_zeroed(&std::alloc::System, layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, p, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        let p = unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, p, layout, size) };
        if !p.is_null() {
            if size >= layout.size() {
                grew(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        p
    }
}
#[global_allocator]
static ALLOCATIONS: AllocationObserver = AllocationObserver;

fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_QUERY_REVIEW_CASE").as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_QUERY_REVIEW_CASE", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-query-admission-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wav(&self, name: &str) -> PathBuf {
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
        bytes.extend([0; 320]);
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
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

fn track(path: &str, tags: BTreeMap<String, String>) -> TrackDto {
    serde_json::from_value(
        serde_json::json!({"path":path,"format":"FLAC","tags":tags,"has_cover":false}),
    )
    .unwrap()
}
fn request(field: &str, value: &str) -> QueryRequest {
    serde_json::from_value(serde_json::json!({"requestId":1,"query":{
        "version":2,"root":{"kind":"condition","id":"condition","field":field,"operator":"equals","values":[value]}
    }})).unwrap()
}

#[test]
fn native_computed_masks_keep_reading_existing_technical_column_values() {
    if isolated("native_computed_masks_keep_reading_existing_technical_column_values") {
        return;
    }
    let fixture = Fixture::new();
    fixture.wav("technical.wav");
    let app = fixture.app();
    let index = app.query_snapshot().unwrap();
    for (field, mask) in [
        ("size", "_filesize_bytes"),
        ("bitrate", "_bitrate"),
        ("samplerate", "_samplerate"),
        ("channels", "_channels"),
    ] {
        let expected = index.rows[0].value(field);
        assert!(!expected.is_empty(), "fixture must have technical {field}");
        let mut query = request("custom:display", expected);
        query.query.custom_fields = vec![CustomField {
            key: "custom:display".into(),
            mask: format!("%{mask}%"),
        }];
        let result = index.query(&query, || false).unwrap();
        assert_eq!(
            result.indices,
            vec![0],
            "technical {field} stopped resolving through the existing custom mask field path"
        );
    }
}

#[test]
fn unicode_high_cardinality_rows_keep_base_leases_across_updates_and_custom_cache() {
    let budget = Budget::new(Limits {
        retained: 128 * 1024 * 1024,
        transient: 1,
        total: 128 * 1024 * 1024,
        jobs: 1,
    });
    let mut rows = Vec::new();
    for i in 0..96 {
        let mut tags = BTreeMap::new();
        tags.insert("artist".into(), format!("İ Artist {i} 尾"));
        for j in 0..128 {
            tags.insert(
                format!("custom:namespace_{j:03}"),
                format!("İ/;\\尾 {i}:{j} ").repeat(48),
            );
        }
        rows.push(
            QueryRow::try_from_track_in(&track(&format!("/fixture/{i}.flac"), tags), &budget)
                .unwrap(),
        );
    }
    let base_charge = budget.usage().0.retained;
    assert!(base_charge > 8 * 1024 * 1024);
    let index = QueryIndex::from_rows(rows);
    let held = index.rows[0].clone();
    let unchanged = index.rows[1].clone();
    let mut tags = BTreeMap::new();
    tags.insert("artist".into(), "Changed artist".into());
    let replacement =
        QueryRow::try_from_track_in(&track("/fixture/0.flac", tags), &budget).unwrap();
    let replacement_charge = budget.usage().0.retained - base_charge;
    let updated = index.with_updates(vec![replacement]);
    assert!(!index.is_current());
    assert!(Arc::ptr_eq(&unchanged, &updated.rows[1]));
    let mut query = request("custom:display", "Changed artist");
    query.query.custom_fields = vec![CustomField {
        key: "custom:display".into(),
        mask: "%artist%".into(),
    }];
    assert_eq!(updated.query(&query, || false).unwrap().indices, vec![0]);
    assert_eq!(updated.query(&query, || false).unwrap().indices, vec![0]);
    assert_eq!(budget.usage().0.retained, base_charge + replacement_charge);
    drop(index);
    drop(unchanged);
    drop(updated);
    assert!(
        budget.usage().0.retained > 0,
        "held source row owns its saved allocation"
    );
    drop(held);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn cancellation_after_partial_native_listing_keeps_prior_generation_and_releases_partial_rows() {
    if isolated("cancellation_after_partial_native_listing_keeps_prior_generation_and_releases_partial_rows") { return; }
    use std::cell::Cell;
    use std::rc::Rc;
    let fixture = Fixture::new();
    for index in 0..32 {
        fixture.wav(&format!("{index:02}.wav"));
    }
    let app = fixture.app();
    let before = app.query_snapshot().unwrap();
    let budget = Budget::process();
    let usage = budget.usage().0;
    let calls = Rc::new(Cell::new(0));
    let probe = calls.clone();
    let outcome = with_preview_cancellation(
        move || {
            let n = probe.get() + 1;
            probe.set(n);
            n >= 20
        },
        || app.list_tracks(),
    );
    assert!(outcome.is_err());
    assert!(calls.get() >= 20);
    assert_eq!(app.query_snapshot_id(), Some(before.id));
    assert!(Arc::ptr_eq(&before, &app.query_snapshot().unwrap()));
    assert!(before.is_current());
    // Existing metadata cache was already warm, so cancellation may not retain
    // any partial query rows or new cache entries.
    assert_eq!(budget.usage().0, usage);
    assert_eq!(app.list_tracks().unwrap().len(), 32);
}

#[test]
fn short_tag_block_labels_cannot_allocate_past_admitted_capacity_before_refusal() {
    if isolated("short_tag_block_labels_cannot_allocate_past_admitted_capacity_before_refusal") {
        return;
    }
    let mut row = track("/fixture/blocks.flac", BTreeMap::new());
    row.tag_blocks = (0..100_000)
        .map(|_| tagrex_commands::TagBlockDto {
            label: String::new(),
            kind: String::new(),
            read_from: true,
        })
        .collect();
    let budget = Budget::new(Limits {
        retained: 64 * 1024,
        transient: 0,
        total: 64 * 1024,
        jobs: 1,
    });
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let outcome = QueryRow::try_from_track_in(&row, &budget);
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    assert!(outcome.is_err(), "shape must exceed this small budget");
    assert_eq!(budget.usage().0, Charge::default());
    assert!(peak <= 64*1024, "constructor allocated {peak} bytes before refusing while shared retained high-water was only {}", budget.usage().1.retained);
}

#[test]
fn simultaneous_row_construction_cannot_overbook_or_leak_a_shared_retained_budget() {
    let budget = Budget::new(Limits {
        retained: 1024 * 1024,
        transient: 0,
        total: 1024 * 1024,
        jobs: 1,
    });
    let gate = Arc::new(std::sync::Barrier::new(33));
    let mut threads = Vec::new();
    for i in 0..32 {
        let budget = budget.clone();
        let gate = gate.clone();
        threads.push(std::thread::spawn(move || {
            let row = track(
                &format!("/fixture/parallel/{i}.flac"),
                [("comment".into(), "İ尾/; ".repeat(3000))].into(),
            );
            let result = QueryRow::try_from_track_in(&row, &budget);
            gate.wait();
            assert!(budget.usage().0.retained <= 1024 * 1024);
            gate.wait();
            result.is_ok()
        }));
    }
    gate.wait();
    let peak = budget.usage().1.retained;
    assert!(peak > 0 && peak <= 1024 * 1024);
    gate.wait();
    let admitted = threads
        .into_iter()
        .map(|thread| usize::from(thread.join().unwrap()))
        .sum::<usize>();
    assert!(
        admitted > 0 && admitted < 32,
        "expected actual admission and actual pressure"
    );
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn admitted_tag_block_boundary_counts_empty_labels_and_refuses_next_member() {
    let budget = Budget::new(Limits {
        retained: 64 * 1024,
        transient: 0,
        total: 64 * 1024,
        jobs: 1,
    });
    let mut input = track("/fixture/boundary.flac", BTreeMap::new());
    input.tag_blocks = (0..256)
        .map(|n| tagrex_commands::TagBlockDto {
            label: String::new(),
            kind: String::new(),
            read_from: n % 2 == 0,
        })
        .collect();
    let row = QueryRow::try_from_track_in(&input, &budget).unwrap();
    assert_eq!(row.value("tagtypes"), " + ".repeat(255));
    drop(row);
    assert_eq!(budget.usage().0, Charge::default());
    input.tag_blocks.push(tagrex_commands::TagBlockDto {
        label: String::new(),
        kind: String::new(),
        read_from: false,
    });
    assert!(matches!(
        QueryRow::try_from_track_in(&input, &budget),
        Err(tagrex_core::resources::AdmissionError::TooLarge)
    ));
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn policy_change_during_native_listing_cannot_publish_mixed_generation_rows() {
    if isolated("policy_change_during_native_listing_cannot_publish_mixed_generation_rows") {
        return;
    }
    use std::cell::Cell;
    use std::rc::Rc;
    let fixture = Fixture::new();
    for n in 0..16 {
        fixture.wav(&format!("{n:02}.wav"));
    }
    let app = fixture.app();
    let before = app.query_snapshot().unwrap();
    let calls = Rc::new(Cell::new(0));
    let probe = calls.clone();
    let outcome = with_preview_cancellation(
        move || {
            let n = probe.get() + 1;
            probe.set(n);
            if n == 20 {
                tagrex_core::model::set_multi_value_separator(" / ");
            }
            false
        },
        || app.list_tracks(),
    );
    assert!(calls.get() >= 20);
    assert!(
        matches!(outcome, Err(tagrex_commands::AppError::Io(ref error)) if error.kind()==std::io::ErrorKind::Interrupted)
    );
    assert_eq!(
        app.query_snapshot_id(),
        Some(before.id),
        "failed mixed-policy listing was published"
    );
    assert!(
        !before.is_current(),
        "old interpretation cannot be queried after policy change"
    );
    let current = app.query_snapshot().unwrap();
    assert_ne!(current.id, before.id);
    assert!(current.is_current());
    assert_eq!(current.rows.len(), 16);
}
