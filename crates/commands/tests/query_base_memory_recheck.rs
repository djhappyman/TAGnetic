//! Independent retained-allocation probe for the complete query snapshot owner.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
use tagrex_commands::{query::QueryIndex, TrackDto};
use tagrex_core::resources::Budget;

struct Observed;
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
// SAFETY: every pointer/layout operation delegates unchanged to System. The
// counter observes successful allocation sizes and never changes allocation.
unsafe impl GlobalAlloc for Observed {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            OUTSTANDING.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            OUTSTANDING.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        OUTSTANDING.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let replacement = unsafe { System.realloc(pointer, layout, size) };
        if !replacement.is_null() {
            if size >= layout.size() {
                OUTSTANDING.fetch_add(size - layout.size(), Ordering::Relaxed);
            } else {
                OUTSTANDING.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        replacement
    }
}
#[global_allocator]
static ALLOCATOR: Observed = Observed;

#[test]
fn base_query_rows_retain_charged_ownership_until_snapshot_drop() {
    let name = "base_query_rows_retain_charged_ownership_until_snapshot_drop";
    if std::env::var("TAGREX_BASE_QUERY_MEMORY_CHILD").as_deref() != Ok("1") {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env("TAGREX_BASE_QUERY_MEMORY_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let budget = Budget::process();
    let comment = "Long retained Comment 🎵 ".repeat(3000);
    let tracks:Vec<TrackDto>=(0..64).map(|index|serde_json::from_value(serde_json::json!({"path":format!("/fixture/{index}.flac"),"format":"FLAC","tags":{"title":format!("Track {index}"),"comment":comment},"has_cover":false})).unwrap()).collect();
    // Caller-owned input allocations are excluded. Measure only retained data
    // created by the native searchable snapshot, before any derived facet work.
    let allocated_before = OUTSTANDING.load(Ordering::Relaxed);
    let charged_before = budget.usage().0.retained;
    let index = QueryIndex::try_from_tracks(&tracks).unwrap();
    assert_eq!(index.rows.len(), tracks.len());
    let allocated = OUTSTANDING
        .load(Ordering::Relaxed)
        .saturating_sub(allocated_before);
    let charged = budget.usage().0.retained.saturating_sub(charged_before);
    eprintln!("BASE_QUERY_MEMORY allocated={allocated} charged={charged}");
    assert!(
        allocated.saturating_sub(charged) < 1024 * 1024,
        "base query text/search/folded allocations bypass participating retained admission"
    );
    let row = index.rows[0].clone();
    drop(index);
    assert!(
        budget.usage().0.retained > charged_before,
        "outstanding worker row lost accounting"
    );
    drop(row);
    assert_eq!(budget.usage().0.retained, charged_before);
}

#[test]
fn admission_refusal_keeps_the_previous_session_snapshot_available() {
    let name = "admission_refusal_keeps_the_previous_session_snapshot_available";
    if std::env::var("TAGREX_BASE_QUERY_REFUSAL_CHILD").as_deref() != Ok("1") {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env("TAGREX_BASE_QUERY_REFUSAL_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("tagrex-query-admission-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("track.wav");
    // A complete mono PCM frame; no tag writer participates in fixture creation.
    let mut wave = b"RIFF".to_vec();
    wave.extend(38u32.to_le_bytes());
    wave.extend(b"WAVEfmt ");
    wave.extend(16u32.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(8000u32.to_le_bytes());
    wave.extend(16000u32.to_le_bytes());
    wave.extend(2u16.to_le_bytes());
    wave.extend(16u16.to_le_bytes());
    wave.extend(b"data");
    wave.extend(2u32.to_le_bytes());
    wave.extend([0, 0]);
    std::fs::write(&path, &wave).unwrap();
    let app = tagrex_commands::App::open(&dir, &dir.join("journal.sqlite")).unwrap();
    assert_eq!(app.list_tracks().unwrap().len(), 1);
    let original = app.query_snapshot().unwrap();
    let budget = Budget::process();
    let before = budget.usage().0.retained;
    let pressure = budget
        .try_acquire(tagrex_core::resources::Charge {
            retained: 256 * 1024 * 1024 - before,
            ..Default::default()
        })
        .unwrap();
    std::fs::write(dir.join("added.wav"), wave).unwrap();
    assert!(matches!(
        app.list_tracks(),
        Err(tagrex_commands::AppError::Resource(_))
    ));
    assert_eq!(app.query_snapshot_id(), Some(original.id));
    assert!(original.is_current());
    assert!(std::sync::Arc::ptr_eq(
        &original,
        &app.query_snapshot().unwrap()
    ));
    assert!(matches!(
        app.read_track_rows(&[path]),
        Err(tagrex_commands::AppError::Resource(_))
    ));
    assert!(original.is_current());
    drop(pressure);
    assert_eq!(app.list_tracks().unwrap().len(), 2);
    assert!(!original.is_current());
    drop(original);
    drop(app);
    assert_eq!(budget.usage().0.retained, 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn oversized_and_busy_base_rows_refuse_without_retaining_partial_owners() {
    use tagrex_core::resources::{AdmissionError, Charge, Limits};
    let track: TrackDto = serde_json::from_value(serde_json::json!({
        "path": "/fixture/text.flac", "format": "FLAC", "tags": {
            "title": "İ Literal ; / \\  尾".repeat(1000), "comment": ""}, "has_cover": false
    }))
    .unwrap();
    let too_small = Budget::new(Limits {
        retained: 1,
        transient: 1,
        total: 2,
        jobs: 1,
    });
    assert_eq!(
        tagrex_commands::query::QueryRow::try_from_track_in(&track, &too_small).unwrap_err(),
        AdmissionError::TooLarge
    );
    assert_eq!(too_small.usage().0, Charge::default());
    let budget = Budget::new(Limits {
        retained: 8 * 1024 * 1024,
        transient: 1,
        total: 8 * 1024 * 1024,
        jobs: 1,
    });
    let pressure = budget
        .try_acquire(Charge {
            retained: 8 * 1024 * 1024,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        tagrex_commands::query::QueryRow::try_from_track_in(&track, &budget).unwrap_err(),
        AdmissionError::Busy
    );
    drop(pressure);
    let row = tagrex_commands::query::QueryRow::try_from_track_in(&track, &budget).unwrap();
    assert_eq!(row.value("title"), track.tags["title"]);
    assert!(budget.usage().0.retained > 0);
    drop(row);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn forked_snapshot_rows_share_admission_but_keep_independent_validity() {
    use tagrex_commands::query::{QueryError, QueryRow};
    use tagrex_core::resources::Limits;
    let budget = Budget::new(Limits {
        retained: 8 << 20,
        transient: 8 << 20,
        total: 16 << 20,
        jobs: 1,
    });
    let tracks:Vec<TrackDto>=(0..3).map(|i|serde_json::from_value(serde_json::json!({"path":format!("/fixture/{i}.flac"),"format":"FLAC","tags":{"title":"Shared saved text 尾".repeat(1000)},"has_cover":false})).unwrap()).collect();
    let rows = tracks
        .iter()
        .map(|track| QueryRow::try_from_track_in(track, &budget).unwrap())
        .collect();
    let base = QueryIndex::from_rows(rows);
    let before = budget.usage().0;
    let process_budget = Budget::process();
    let process_before = process_budget.usage().0;
    let pressure = process_budget
        .try_acquire(tagrex_core::resources::Charge {
            retained: (256 << 20) - process_before.retained,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(
        base.fork_snapshot_rows(None),
        Err(QueryError::Resource(
            tagrex_core::resources::AdmissionError::Busy
        ))
    ));
    assert!(base.is_current());
    drop(pressure);
    let first = base.fork_snapshot_rows(Some(&[0, 2])).unwrap();
    let second = base.fork_snapshot_rows(None).unwrap();
    assert_ne!(base.id, first.id);
    assert_ne!(first.id, second.id);
    assert_eq!(budget.usage().0, before);
    assert!(process_budget.usage().0.retained > process_before.retained);
    assert_eq!(
        first
            .rows
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        vec!["/fixture/0.flac", "/fixture/2.flac"]
    );
    for selection in [&[2, 0][..], &[0, 0][..], &[3][..]] {
        assert!(matches!(
            base.fork_snapshot_rows(Some(selection)),
            Err(QueryError::Invalid(_))
        ));
    }
    base.invalidate();
    assert!(!base.is_current());
    assert!(first.is_current());
    assert!(second.is_current());
    assert!(matches!(
        base.fork_snapshot_rows(None),
        Err(QueryError::Cancelled)
    ));
    first.invalidate();
    assert!(second.is_current());
    drop(base);
    drop(second);
    assert!(budget.usage().0.retained > 0);
    drop(first);
    assert_eq!(budget.usage().0, Default::default());
    assert_eq!(process_budget.usage().0, process_before);
}
